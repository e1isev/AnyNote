//! Identifiers and timestamps.
//!
//! Every notebook, section, page, block and ink stroke gets a UUIDv7 at
//! creation. v7 IDs sort by creation time, which keeps SQLite indexes and
//! exported folders in a stable, meaningful order.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default()
}
