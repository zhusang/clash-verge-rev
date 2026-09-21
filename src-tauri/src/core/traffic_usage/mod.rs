//! Background traffic usage history.
//!
//! The collector subscribes to mihomo's `/connections` stream, turns the
//! per-connection byte counters into hourly deltas grouped by process, host
//! and proxy node, and persists them to a small SQLite database so the user
//! can later answer "who used the traffic" even for connections that are long
//! gone.

mod aggregate;
mod collector;
mod store;

pub use collector::{TrafficUsageCollector, UsageStatus};
pub use store::{GroupBy, UsageFilter, UsageRange, UsageRow};
