//! Background collector that turns mihomo connection snapshots into hourly
//! per-process / host / proxy usage rows.
//!
//! Lifecycle:
//! - `init` opens the database, loads settings and starts the flush / prune
//!   workers.
//! - `CoreManager` calls `notify_core_state` whenever the core starts or
//!   stops; a running core (re)subscribes, a stopped core flushes.
//! - `feat::patch_verge` calls `refresh_settings` when the user toggles the
//!   feature or changes the retention period.

use super::{
    aggregate::{Accumulator, Snapshot},
    store::{DB_FILE, GroupBy, Store, StoreStatus, UsageFilter, UsageRange, UsageRow},
};
use crate::{
    config::{Config, IVerge},
    core::{
        handle::Handle,
        manager::{CoreManager, RunningMode},
    },
    process::AsyncHandler,
    singleton,
    utils::dirs,
};
use anyhow::{Result, anyhow};
use arc_swap::{ArcSwap, ArcSwapOption};
use clash_verge_logging::{Type, logging};
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tauri_plugin_mihomo::models::WebSocketMessage;
use tokio::{
    sync::{mpsc, watch},
    time::MissedTickBehavior,
};

const FLUSH_INTERVAL: Duration = Duration::from_secs(30);
const PRUNE_INTERVAL: Duration = Duration::from_secs(3600);
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const DISCONNECT_FORCE_TIMEOUT_MS: u64 = 1000;
const DEFAULT_RETENTION_DAYS: u32 = 30;
const MIN_RETENTION_DAYS: u32 = 1;
const MAX_RETENTION_DAYS: u32 = 365;
const SECS_PER_DAY: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Settings {
    enabled: bool,
    retention_days: u32,
}

impl Settings {
    fn from_verge(verge: &IVerge) -> Self {
        Self {
            enabled: verge.enable_traffic_usage.unwrap_or(true),
            retention_days: verge
                .traffic_usage_retention_days
                .unwrap_or(DEFAULT_RETENTION_DAYS)
                .clamp(MIN_RETENTION_DAYS, MAX_RETENTION_DAYS),
        }
    }
}

impl Default for Settings {
    /// Disabled until `init` has read the real configuration.
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatus {
    pub enabled: bool,
    pub collecting: bool,
    pub db_size_bytes: u64,
    pub oldest_bucket_ts: Option<i64>,
}

struct Subscription {
    stop: watch::Sender<bool>,
    /// Incremented for every subscription; a task only reports its state while
    /// it is still the current generation, so a task that is being replaced can
    /// never clobber the flags of its successor.
    generation: u64,
}

pub struct TrafficUsageCollector {
    accumulator: Mutex<Accumulator>,
    store: ArcSwapOption<Store>,
    settings: ArcSwap<Settings>,
    subscription: Mutex<Option<Subscription>>,
    collecting: AtomicBool,
    workers_started: AtomicBool,
    generation: AtomicU64,
}

singleton!(TrafficUsageCollector, TRAFFIC_USAGE_COLLECTOR);

impl TrafficUsageCollector {
    fn new() -> Self {
        Self {
            accumulator: Mutex::new(Accumulator::default()),
            store: ArcSwapOption::new(None),
            settings: ArcSwap::from_pointee(Settings::default()),
            subscription: Mutex::new(None),
            collecting: AtomicBool::new(false),
            workers_started: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }

    /// Open the database, load settings and start the background workers.
    ///
    /// The subscription itself is started by `notify_core_state` once the core
    /// is running.
    pub async fn init(&self) -> Result<()> {
        self.reload_settings().await;
        let path = dirs::app_home_dir()?.join(DB_FILE);
        match AsyncHandler::spawn_blocking(move || Store::open(path)).await {
            Ok(Ok(store)) => {
                self.store.store(Some(Arc::new(store)));
                logging!(info, Type::Core, "Traffic usage database ready");
            }
            Ok(Err(err)) => logging!(
                error,
                Type::Core,
                "Traffic usage database unavailable, usage will not be collected: {err:#}"
            ),
            Err(err) => logging!(error, Type::Core, "Traffic usage database open task failed: {err}"),
        }
        self.start_workers();
        // The core may already be running when we get here (init order is
        // not guaranteed), in which case nobody else will kick off the
        // subscription for us.
        if self.settings.load().enabled && Self::core_running() {
            self.restart_subscription();
        }
        Ok(())
    }

    /// Re-read the verge settings and start / stop collection accordingly.
    pub async fn refresh_settings(&self) {
        let before = **self.settings.load();
        self.reload_settings().await;
        let after = **self.settings.load();

        if before.enabled != after.enabled {
            if !after.enabled {
                self.stop_subscription();
                self.flush_now().await;
            } else if Self::core_running() {
                self.restart_subscription();
            }
        }
        if before.retention_days != after.retention_days {
            self.prune().await;
        }
    }

    /// Called by `CoreManager` after the core process state changed.
    ///
    /// A stopped core only drops the subscription; `CoreManager::stop_core`
    /// flushes before the process goes away.
    pub fn notify_core_state(&self, running: bool) {
        if running {
            if self.settings.load().enabled {
                self.restart_subscription();
            }
        } else {
            self.stop_subscription();
        }
    }

    /// Drop the current subscription (if any) and start a fresh one.
    pub fn restart_subscription(&self) {
        // Bumping the generation inside the critical section makes the swap
        // atomic: the replaced task sees a stale generation and stays quiet.
        let mut slot = self.subscription.lock();
        if let Some(previous) = slot.take() {
            let _ = previous.stop.send(true);
        }
        let generation = self.next_generation();
        let (stop_tx, stop_rx) = watch::channel(false);
        *slot = Some(Subscription {
            stop: stop_tx,
            generation,
        });
        drop(slot);
        self.collecting.store(false, Ordering::Release);

        if self.store.load().is_none() {
            logging!(warn, Type::Core, "Traffic usage database missing, not subscribing");
            return;
        }
        AsyncHandler::spawn(move || async move {
            Self::global().run_subscription(stop_rx, generation).await;
        });
    }

    pub fn stop_subscription(&self) {
        let previous = self.subscription.lock().take();
        if let Some(subscription) = previous {
            let _ = subscription.stop.send(true);
        }
        self.collecting.store(false, Ordering::Release);
    }

    /// Next subscription generation. Only ever called while holding the
    /// subscription lock.
    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    /// Whether `generation` is the subscription that is still installed.
    fn is_current_generation(&self, generation: u64) -> bool {
        self.subscription
            .lock()
            .as_ref()
            .is_some_and(|current| current.generation == generation)
    }

    /// Write everything accumulated so far to the database.
    pub async fn flush_now(&self) {
        let rows = self.accumulator.lock().drain();
        if rows.is_empty() {
            return;
        }
        let Some(store) = self.store.load_full() else {
            // Without a database there is nothing useful to do with the rows;
            // dropping them keeps memory bounded.
            return;
        };
        let result = AsyncHandler::spawn_blocking(move || store.upsert_batch(&rows).map_err(|err| (err, rows))).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err((err, rows))) => {
                // Only transient failures are worth retrying; a corrupt or
                // read-only database would otherwise make every flush fail
                // forever while the pending map grows without bound.
                if is_retryable(&err) {
                    logging!(
                        warn,
                        Type::Core,
                        "Failed to persist {} traffic usage rows, retrying later: {err:#}",
                        rows.len()
                    );
                    self.accumulator.lock().restore(rows);
                } else {
                    logging!(
                        error,
                        Type::Core,
                        "Dropping {} traffic usage rows, database is not writable: {err:#}",
                        rows.len()
                    );
                }
            }
            Err(err) => logging!(error, Type::Core, "Traffic usage flush task failed: {err}"),
        }
    }

    /// Aggregated usage in `range`, biggest consumer first.
    pub async fn query(&self, range: UsageRange, group_by: GroupBy, filter: UsageFilter) -> Result<Vec<UsageRow>> {
        if range.since_ts > range.until_ts {
            return Err(anyhow!("invalid range: sinceTs is after untilTs"));
        }
        let Some(store) = self.store.load_full() else {
            return Ok(Vec::new());
        };
        // Make the newest pending bytes visible to the query.
        self.flush_now().await;
        AsyncHandler::spawn_blocking(move || store.query(range, group_by, &filter)).await?
    }

    /// Delete all persisted history and pending rows; observations are kept so
    /// new traffic keeps being counted from zero.
    pub async fn clear(&self) -> Result<()> {
        self.accumulator.lock().clear_pending();
        let Some(store) = self.store.load_full() else {
            return Ok(());
        };
        AsyncHandler::spawn_blocking(move || store.clear()).await?
    }

    pub async fn status(&self) -> Result<UsageStatus> {
        let settings = **self.settings.load();
        let collecting = self.collecting.load(Ordering::Acquire);
        let store_status = match self.store.load_full() {
            Some(store) => AsyncHandler::spawn_blocking(move || store.status()).await??,
            None => StoreStatus::default(),
        };
        Ok(UsageStatus {
            enabled: settings.enabled,
            collecting,
            db_size_bytes: store_status.db_size_bytes,
            oldest_bucket_ts: store_status.oldest_bucket_ts,
        })
    }

    async fn reload_settings(&self) {
        let verge = Config::verge().await.latest_arc();
        self.settings.store(Arc::new(Settings::from_verge(&verge)));
    }

    fn core_running() -> bool {
        !matches!(*CoreManager::global().get_running_mode(), RunningMode::NotRunning)
    }

    fn start_workers(&self) {
        if self.workers_started.swap(true, Ordering::AcqRel) {
            return;
        }
        AsyncHandler::spawn(|| async {
            let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                Self::global().flush_now().await;
            }
        });
        AsyncHandler::spawn(|| async {
            let mut ticker = tokio::time::interval(PRUNE_INTERVAL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                Self::global().prune().await;
            }
        });
    }

    async fn prune(&self) {
        let Some(store) = self.store.load_full() else {
            return;
        };
        let retention_days = self.settings.load().retention_days;
        let cutoff_ts = chrono::Utc::now().timestamp() - i64::from(retention_days) * SECS_PER_DAY;
        match AsyncHandler::spawn_blocking(move || store.prune_before(cutoff_ts)).await {
            Ok(Ok(0)) => {}
            Ok(Ok(deleted)) => logging!(
                info,
                Type::Core,
                "Pruned {deleted} traffic usage rows older than {retention_days} days"
            ),
            Ok(Err(err)) => logging!(warn, Type::Core, "Failed to prune traffic usage: {err:#}"),
            Err(err) => logging!(error, Type::Core, "Traffic usage prune task failed: {err}"),
        }
    }

    async fn run_subscription(&self, mut stop: watch::Receiver<bool>, generation: u64) {
        let mut failures: u32 = 0;
        while !Self::should_stop(&stop) {
            match self.subscribe_once(&mut stop, generation).await {
                Ok(()) => failures = 0,
                Err(err) => {
                    // The core is usually just not up yet; keep the log quiet
                    // after the first attempt.
                    failures = failures.saturating_add(1);
                    if failures == 1 {
                        logging!(warn, Type::Core, "Traffic usage subscription failed: {err:#}");
                    } else {
                        logging!(
                            debug,
                            Type::Core,
                            "Traffic usage subscription retry {failures} failed: {err:#}"
                        );
                    }
                }
            }
            self.mark_not_collecting(generation);
            if Self::should_stop(&stop) {
                break;
            }
            tokio::select! {
                () = tokio::time::sleep(RECONNECT_DELAY) => {}
                _ = stop.changed() => {}
            }
        }
        logging!(debug, Type::Core, "Traffic usage subscription loop exited");
    }

    /// Subscribe and pump snapshots until the stream closes or `stop` fires.
    async fn subscribe_once(&self, stop: &mut watch::Receiver<bool>, generation: u64) -> Result<()> {
        self.accumulator.lock().reset_observations();
        let baseline = AtomicBool::new(true);
        let (closed_tx, mut closed_rx) = mpsc::channel::<()>(1);

        let callback = move |value: serde_json::Value| Self::on_message(value, &baseline, &closed_tx);
        let ws_id = Handle::mihomo()
            .await
            .ws_connections(callback)
            .await
            .map_err(|err| anyhow!("{err}"))?;
        logging!(info, Type::Core, "Traffic usage collector subscribed (ws {ws_id})");
        self.mark_collecting(generation);

        tokio::select! {
            _ = closed_rx.recv() => {
                logging!(info, Type::Core, "Connections stream closed (ws {ws_id})");
            }
            _ = stop.changed() => {}
        }
        self.mark_not_collecting(generation);

        // Close our end and make the plugin forget the socket.
        if let Err(err) = Handle::mihomo()
            .await
            .disconnect(ws_id, Some(DISCONNECT_FORCE_TIMEOUT_MS))
            .await
        {
            logging!(debug, Type::Core, "Disconnect ws {ws_id}: {err}");
        }
        Ok(())
    }

    /// Report the subscription as live, unless a newer one already took over.
    fn mark_collecting(&self, generation: u64) {
        if self.is_current_generation(generation) {
            self.collecting.store(true, Ordering::Release);
        }
    }

    fn mark_not_collecting(&self, generation: u64) {
        if self.is_current_generation(generation) {
            self.collecting.store(false, Ordering::Release);
        }
    }

    fn should_stop(stop: &watch::Receiver<bool>) -> bool {
        *stop.borrow() || stop.has_changed().is_err()
    }

    fn on_message(value: serde_json::Value, baseline: &AtomicBool, closed: &mpsc::Sender<()>) {
        match serde_json::from_value::<WebSocketMessage>(value) {
            Ok(WebSocketMessage::Text(text)) => {
                if text.starts_with('{') {
                    match serde_json::from_str::<Snapshot>(&text) {
                        Ok(snapshot) => {
                            let is_baseline = baseline.swap(false, Ordering::AcqRel);
                            Self::global().ingest(&snapshot, is_baseline);
                        }
                        Err(err) => logging!(debug, Type::Core, "Ignoring unparsable connections snapshot: {err}"),
                    }
                } else {
                    // The plugin reports transport errors as plain text.
                    logging!(warn, Type::Core, "Connections stream error: {text}");
                    let _ = closed.try_send(());
                }
            }
            Ok(WebSocketMessage::Close(_)) => {
                let _ = closed.try_send(());
            }
            Ok(_) => {}
            Err(err) => logging!(debug, Type::Core, "Ignoring unknown websocket payload: {err}"),
        }
    }

    fn ingest(&self, snapshot: &Snapshot, is_baseline: bool) {
        let now_ts = chrono::Utc::now().timestamp();
        self.accumulator.lock().apply_snapshot(snapshot, now_ts, is_baseline);
    }
}

/// Whether a failed write is worth retrying with the same rows.
///
/// Busy / timeout / locked errors clear on their own; anything else (disk
/// full, corruption, read-only file, …) would keep failing, so the rows are
/// dropped instead of piling up in memory forever.
fn is_retryable(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(inner, _))
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseBusy
                    | rusqlite::ErrorCode::DatabaseLocked
                    | rusqlite::ErrorCode::OperationInterrupted
            )
    )
}
