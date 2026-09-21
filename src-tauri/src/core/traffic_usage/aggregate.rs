//! In-memory aggregation of connection traffic deltas.
//!
//! Every mihomo snapshot lists the live connections together with their
//! cumulative byte counters. We remember the last counters per connection id,
//! add the difference to the `(hour bucket, process, host, proxy)` accumulator
//! and forget connections that disappeared. The first snapshot after
//! (re)subscribing only registers baselines so bytes that were already counted
//! before a reconnect are never counted twice.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Dimension value used when mihomo did not report a process / host / proxy.
pub const UNKNOWN: &str = "unknown";

const HOUR_SECS: i64 = 3600;

/// Minimal projection of a mihomo `/connections` snapshot.
///
/// Only the fields the collector needs are declared and every one of them
/// defaults when missing, so a core version that adds or removes metadata
/// fields never breaks ingestion.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default)]
    pub connections: Option<Vec<SnapshotConnection>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotConnection {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    /// `chains[0]` is the outbound node, the last element the top-level group.
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub metadata: SnapshotMetadata,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMetadata {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub sniff_host: String,
    #[serde(default, rename = "destinationIP")]
    pub destination_ip: String,
    #[serde(default)]
    pub remote_destination: String,
    #[serde(default)]
    pub process: String,
    #[serde(default)]
    pub process_path: String,
}

/// Unique key of one aggregated row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AggKey {
    /// Start of the hour, unix seconds (UTC).
    pub bucket_ts: i64,
    pub process: String,
    pub host: String,
    pub proxy: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bytes {
    pub upload: u64,
    pub download: u64,
}

impl Bytes {
    const fn accumulate(&mut self, other: Self) {
        self.upload = self.upload.saturating_add(other.upload);
        self.download = self.download.saturating_add(other.download);
    }

    const fn is_zero(self) -> bool {
        self.upload == 0 && self.download == 0
    }
}

/// Dimensions of a connection, resolved once when it is first seen.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Dims {
    process: String,
    host: String,
    proxy: String,
}

impl Dims {
    fn of(conn: &SnapshotConnection) -> Self {
        Self {
            process: process_name(&conn.metadata),
            host: host_name(&conn.metadata),
            proxy: proxy_name(&conn.chains),
        }
    }

    fn key(&self, bucket_ts: i64) -> AggKey {
        AggKey {
            bucket_ts,
            process: self.process.clone(),
            host: self.host.clone(),
            proxy: self.proxy.clone(),
        }
    }
}

#[derive(Debug)]
struct LastSeen {
    counters: Bytes,
    dims: Dims,
}

#[derive(Debug, Default)]
pub struct Accumulator {
    pending: HashMap<AggKey, Bytes>,
    last_seen: HashMap<String, LastSeen>,
}

impl Accumulator {
    /// Fold one snapshot into the accumulator.
    ///
    /// `is_baseline` marks the first snapshot after (re)subscribing: it only
    /// records the current counters. Later snapshots add the delta since the
    /// previous observation. A connection that shows up for the first time in
    /// a non-baseline snapshot started after the previous push, so its whole
    /// counters are new traffic and are counted as well.
    pub fn apply_snapshot(&mut self, snapshot: &Snapshot, now_ts: i64, is_baseline: bool) {
        let bucket_ts = bucket_of(now_ts);
        let connections = snapshot.connections.as_deref().unwrap_or_default();
        let mut alive: HashSet<&str> = HashSet::with_capacity(connections.len());

        for conn in connections.iter().filter(|conn| !conn.id.is_empty()) {
            alive.insert(conn.id.as_str());
            let counters = Bytes {
                upload: conn.upload,
                download: conn.download,
            };

            match self.last_seen.get_mut(&conn.id) {
                Some(last) => {
                    let delta = Bytes {
                        upload: counters.upload.saturating_sub(last.counters.upload),
                        download: counters.download.saturating_sub(last.counters.download),
                    };
                    last.counters = counters;
                    if !is_baseline {
                        push(&mut self.pending, last.dims.key(bucket_ts), delta);
                    }
                }
                None => {
                    let dims = Dims::of(conn);
                    if !is_baseline {
                        push(&mut self.pending, dims.key(bucket_ts), counters);
                    }
                    self.last_seen.insert(conn.id.clone(), LastSeen { counters, dims });
                }
            }
        }

        self.last_seen.retain(|id, _| alive.contains(id.as_str()));
    }

    /// Forget every observed connection so the next snapshot acts as baseline.
    pub fn reset_observations(&mut self) {
        self.last_seen.clear();
    }

    /// Take everything accumulated since the last drain.
    pub fn drain(&mut self) -> Vec<(AggKey, Bytes)> {
        self.pending.drain().collect()
    }

    /// Put rows back after a failed flush so they are retried next time.
    pub fn restore(&mut self, rows: Vec<(AggKey, Bytes)>) {
        for (key, bytes) in rows {
            push(&mut self.pending, key, bytes);
        }
    }

    /// Drop pending rows without touching observations ("clear history").
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }
}

fn push(pending: &mut HashMap<AggKey, Bytes>, key: AggKey, bytes: Bytes) {
    if bytes.is_zero() {
        return;
    }
    pending.entry(key).or_default().accumulate(bytes);
}

/// Floor a unix timestamp to the start of its hour.
pub const fn bucket_of(ts: i64) -> i64 {
    ts - ts.rem_euclid(HOUR_SECS)
}

fn process_name(meta: &SnapshotMetadata) -> String {
    let process = meta.process.trim();
    if !process.is_empty() {
        return process.to_owned();
    }
    file_name(&meta.process_path).unwrap_or(UNKNOWN).to_owned()
}

/// Last non-empty path segment, accepting both `/` and `\` separators.
fn file_name(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\'])
        .map(str::trim)
        .find(|segment| !segment.is_empty())
}

fn host_name(meta: &SnapshotMetadata) -> String {
    [
        &meta.host,
        &meta.sniff_host,
        &meta.destination_ip,
        &meta.remote_destination,
    ]
    .into_iter()
    .map(|value| value.trim())
    .find(|value| !value.is_empty())
    .unwrap_or(UNKNOWN)
    .to_owned()
}

fn proxy_name(chains: &[String]) -> String {
    chains
        .first()
        .map(|node| node.trim())
        .filter(|node| !node.is_empty())
        .unwrap_or(UNKNOWN)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 39_540; // 10:59:00 on day 0 (UTC)

    fn conn(id: &str, upload: u64, download: u64) -> SnapshotConnection {
        SnapshotConnection {
            id: id.to_owned(),
            upload,
            download,
            chains: vec!["HK-01".to_owned(), "Auto".to_owned(), "Proxy".to_owned()],
            metadata: SnapshotMetadata {
                host: "example.com".to_owned(),
                process: "chrome.exe".to_owned(),
                ..SnapshotMetadata::default()
            },
        }
    }

    const fn snapshot(connections: Vec<SnapshotConnection>) -> Snapshot {
        Snapshot {
            connections: Some(connections),
        }
    }

    fn totals(acc: &Accumulator) -> Bytes {
        let mut sum = Bytes::default();
        for bytes in acc.pending.values() {
            sum.accumulate(*bytes);
        }
        sum
    }

    #[test]
    fn baseline_snapshot_only_registers() {
        let mut acc = Accumulator::default();
        acc.apply_snapshot(&snapshot(vec![conn("a", 100, 100)]), T0, true);
        assert!(acc.pending.is_empty());
        assert_eq!(acc.last_seen.len(), 1);
    }

    #[test]
    fn deltas_are_accumulated_after_baseline() {
        let mut acc = Accumulator::default();
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 100)]), T0, true);
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 350)]), T0 + 1, false);
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 900)]), T0 + 2, false);
        assert_eq!(
            totals(&acc),
            Bytes {
                upload: 0,
                download: 800
            }
        );
    }

    #[test]
    fn reconnect_baseline_does_not_recount() {
        let mut acc = Accumulator::default();
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 500)]), T0, true);
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 1000)]), T0 + 1, false);
        assert_eq!(totals(&acc).download, 500);

        acc.reset_observations();
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 1200)]), T0 + 5, true);
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 1500)]), T0 + 6, false);
        assert_eq!(totals(&acc).download, 800);
    }

    #[test]
    fn new_connection_after_baseline_counts_initial_bytes() {
        let mut acc = Accumulator::default();
        acc.apply_snapshot(&snapshot(Vec::new()), T0, true);
        acc.apply_snapshot(&snapshot(vec![conn("short", 40, 5000)]), T0 + 1, false);
        assert_eq!(
            totals(&acc),
            Bytes {
                upload: 40,
                download: 5000
            }
        );
    }

    #[test]
    fn vanished_connection_is_forgotten_without_extra_bytes() {
        let mut acc = Accumulator::default();
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 100), conn("b", 0, 100)]), T0, true);
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 150)]), T0 + 1, false);
        assert_eq!(acc.last_seen.len(), 1);
        assert!(acc.last_seen.contains_key("a"));
        assert_eq!(totals(&acc).download, 50);
    }

    #[test]
    fn deltas_land_in_the_bucket_of_their_snapshot() {
        let mut acc = Accumulator::default();
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 0)]), T0, true);
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 100)]), T0 + 30, false); // 10:59:30
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 300)]), T0 + 120, false); // 11:01:00

        let mut buckets: Vec<(i64, u64)> = acc
            .pending
            .iter()
            .map(|(key, bytes)| (key.bucket_ts, bytes.download))
            .collect();
        buckets.sort_unstable();
        assert_eq!(buckets, vec![(36_000, 100), (39_600, 200)]);
    }

    #[test]
    fn drain_and_restore_round_trip() {
        let mut acc = Accumulator::default();
        acc.apply_snapshot(&snapshot(vec![conn("a", 0, 0)]), T0, true);
        acc.apply_snapshot(&snapshot(vec![conn("a", 10, 20)]), T0 + 1, false);
        let rows = acc.drain();
        assert!(acc.pending.is_empty());
        acc.restore(rows);
        assert_eq!(
            totals(&acc),
            Bytes {
                upload: 10,
                download: 20
            }
        );
    }

    #[test]
    fn process_falls_back_to_path_file_name_then_unknown() {
        let mut meta = SnapshotMetadata {
            process_path: r"C:\Apps\foo\bar.exe".to_owned(),
            ..SnapshotMetadata::default()
        };
        assert_eq!(process_name(&meta), "bar.exe");

        meta.process_path = "/usr/bin/curl".to_owned();
        assert_eq!(process_name(&meta), "curl");

        meta.process_path = String::new();
        assert_eq!(process_name(&meta), UNKNOWN);

        meta.process = " firefox ".to_owned();
        assert_eq!(process_name(&meta), "firefox");
    }

    #[test]
    fn host_falls_back_through_sniff_host_and_destination_ip() {
        let mut meta = SnapshotMetadata {
            destination_ip: "1.2.3.4".to_owned(),
            ..SnapshotMetadata::default()
        };
        assert_eq!(host_name(&meta), "1.2.3.4");

        meta.sniff_host = "cdn.example.com".to_owned();
        assert_eq!(host_name(&meta), "cdn.example.com");

        meta.host = "example.com".to_owned();
        assert_eq!(host_name(&meta), "example.com");

        assert_eq!(host_name(&SnapshotMetadata::default()), UNKNOWN);
    }

    #[test]
    fn proxy_is_the_outbound_node() {
        assert_eq!(proxy_name(&["HK-01".to_owned(), "Auto".to_owned()]), "HK-01");
        assert_eq!(proxy_name(&[]), UNKNOWN);
    }

    #[test]
    fn bucket_of_floors_to_the_hour() {
        assert_eq!(bucket_of(T0), 36_000);
        assert_eq!(bucket_of(39_600), 39_600);
        assert_eq!(bucket_of(-1), -3600);
    }
}
