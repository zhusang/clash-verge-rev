//! SQLite persistence for hourly traffic usage buckets.
//!
//! Every method here blocks on the database and must be called from a
//! blocking task (`AsyncHandler::spawn_blocking`), never directly from async
//! code.

use super::aggregate::{AggKey, Bytes};
use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use rusqlite::{Connection, params, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub const DB_FILE: &str = "traffic_usage.db";

const SCHEMA_VERSION: i32 = 1;
/// Upper bound on rows returned by one query; keeps the IPC payload small.
const MAX_ROWS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    Process,
    Host,
    Proxy,
}

impl GroupBy {
    const fn column(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Host => "host",
            Self::Proxy => "proxy",
        }
    }
}

/// Half-open time range `[since_ts, until_ts)` in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRange {
    pub since_ts: i64,
    pub until_ts: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageFilter {
    #[serde(default)]
    pub process: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub key: String,
    pub upload: u64,
    pub download: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoreStatus {
    pub db_size_bytes: u64,
    pub oldest_bucket_ts: Option<i64>,
}

pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Store {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        Self::from_connection(conn, path)
    }

    #[cfg(test)]
    fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, PathBuf::new())
    }

    fn from_connection(conn: Connection, path: PathBuf) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    /// Add the given deltas to their buckets inside a single transaction.
    pub fn upsert_batch(&self, rows: &[(AggKey, Bytes)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut guard = self.conn.lock();
        let conn = &mut *guard;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO usage_hourly (bucket_ts, process, host, proxy, upload, download)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (bucket_ts, process, host, proxy)
                 DO UPDATE SET upload = upload + excluded.upload,
                               download = download + excluded.download",
            )?;
            for (key, bytes) in rows {
                stmt.execute(params![
                    key.bucket_ts,
                    key.process,
                    key.host,
                    key.proxy,
                    to_i64(bytes.upload),
                    to_i64(bytes.download),
                ])?;
            }
        }
        tx.commit()?;
        drop(guard);
        Ok(())
    }

    /// Sum the buckets in `range` grouped by one dimension, biggest first.
    pub fn query(&self, range: UsageRange, group_by: GroupBy, filter: &UsageFilter) -> Result<Vec<UsageRow>> {
        let column = group_by.column();
        let mut sql = format!(
            "SELECT {column} AS k, SUM(upload) AS up, SUM(download) AS down, SUM(upload + download) AS total
             FROM usage_hourly WHERE bucket_ts >= ?1 AND bucket_ts < ?2"
        );
        let mut args: Vec<Value> = vec![Value::Integer(range.since_ts), Value::Integer(range.until_ts)];
        for (name, value) in [
            ("process", &filter.process),
            ("host", &filter.host),
            ("proxy", &filter.proxy),
        ] {
            if let Some(value) = value {
                args.push(Value::Text(value.clone()));
                sql.push_str(&format!(" AND {name} = ?{}", args.len()));
            }
        }
        sql.push_str(&format!(" GROUP BY k ORDER BY total DESC, k ASC LIMIT {MAX_ROWS}"));

        let guard = self.conn.lock();
        let conn: &Connection = &guard;
        let mut stmt = conn.prepare(&sql)?;
        let mapped = stmt.query_map(params_from_iter(args), |row| {
            Ok(UsageRow {
                key: row.get(0)?,
                upload: from_i64(row.get(1)?),
                download: from_i64(row.get(2)?),
                total: from_i64(row.get(3)?),
            })
        })?;
        let rows = mapped.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        drop(guard);
        Ok(rows)
    }

    pub fn clear(&self) -> Result<()> {
        self.conn.lock().execute("DELETE FROM usage_hourly", [])?;
        Ok(())
    }

    /// Delete buckets that start before `cutoff_ts`; returns the row count.
    pub fn prune_before(&self, cutoff_ts: i64) -> Result<usize> {
        let deleted = self
            .conn
            .lock()
            .execute("DELETE FROM usage_hourly WHERE bucket_ts < ?1", params![cutoff_ts])?;
        Ok(deleted)
    }

    pub fn status(&self) -> Result<StoreStatus> {
        let oldest_bucket_ts: Option<i64> =
            self.conn
                .lock()
                .query_row("SELECT MIN(bucket_ts) FROM usage_hourly", [], |row| row.get(0))?;
        Ok(StoreStatus {
            db_size_bytes: file_size(&self.path) + file_size(&wal_path(&self.path)),
            oldest_bucket_ts,
        })
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < SCHEMA_VERSION {
        // The composite primary key already covers `bucket_ts` range scans, so
        // no separate index is needed.
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS usage_hourly (
                bucket_ts INTEGER NOT NULL,
                process   TEXT    NOT NULL,
                host      TEXT    NOT NULL,
                proxy     TEXT    NOT NULL,
                upload    INTEGER NOT NULL DEFAULT 0,
                download  INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (bucket_ts, process, host, proxy)
            ) WITHOUT ROWID;
            PRAGMA user_version = {SCHEMA_VERSION};"
        ))?;
    }
    Ok(())
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn wal_path(path: &Path) -> PathBuf {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    PathBuf::from(wal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(bucket_ts: i64, process: &str, host: &str, proxy: &str) -> AggKey {
        AggKey {
            bucket_ts,
            process: process.to_owned(),
            host: host.to_owned(),
            proxy: proxy.to_owned(),
        }
    }

    fn seeded() -> Result<Store> {
        let store = Store::open_in_memory()?;
        store.upsert_batch(&[
            (
                key(3600, "chrome.exe", "a.com", "HK-01"),
                Bytes {
                    upload: 10,
                    download: 100,
                },
            ),
            (
                key(3600, "chrome.exe", "b.com", "HK-01"),
                Bytes {
                    upload: 20,
                    download: 200,
                },
            ),
            (
                key(7200, "curl", "a.com", "US-01"),
                Bytes {
                    upload: 30,
                    download: 250,
                },
            ),
        ])?;
        Ok(store)
    }

    #[test]
    fn upsert_accumulates_on_conflict() -> Result<()> {
        let store = seeded()?;
        store.upsert_batch(&[(
            key(3600, "chrome.exe", "a.com", "HK-01"),
            Bytes { upload: 1, download: 1 },
        )])?;
        let rows = store.query(
            UsageRange {
                since_ts: 0,
                until_ts: 10_000,
            },
            GroupBy::Host,
            &UsageFilter {
                process: Some("chrome.exe".to_owned()),
                ..UsageFilter::default()
            },
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "b.com");
        assert_eq!(rows[1].key, "a.com");
        assert_eq!(rows[1].upload, 11);
        assert_eq!(rows[1].download, 101);
        assert_eq!(rows[1].total, 112);
        Ok(())
    }

    #[test]
    fn query_groups_by_dimension_and_respects_range() -> Result<()> {
        let store = seeded()?;
        let rows = store.query(
            UsageRange {
                since_ts: 0,
                until_ts: 10_000,
            },
            GroupBy::Process,
            &UsageFilter::default(),
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].key.as_str(), rows[0].total), ("chrome.exe", 330));
        assert_eq!((rows[1].key.as_str(), rows[1].total), ("curl", 280));

        let rows = store.query(
            UsageRange {
                since_ts: 7200,
                until_ts: 10_000,
            },
            GroupBy::Proxy,
            &UsageFilter::default(),
        )?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "US-01");

        let rows = store.query(
            UsageRange {
                since_ts: 20_000,
                until_ts: 30_000,
            },
            GroupBy::Proxy,
            &UsageFilter::default(),
        )?;
        assert!(rows.is_empty());
        Ok(())
    }

    #[test]
    fn prune_clear_and_status() -> Result<()> {
        let store = seeded()?;
        assert_eq!(store.status()?.oldest_bucket_ts, Some(3600));
        assert_eq!(store.prune_before(7200)?, 2);
        assert_eq!(store.status()?.oldest_bucket_ts, Some(7200));
        store.clear()?;
        assert_eq!(store.status()?.oldest_bucket_ts, None);
        Ok(())
    }
}
