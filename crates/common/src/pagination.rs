//! Keyset (cursor) pagination. We page on `(created_at, id)` rather than OFFSET
//! so list latency stays flat as tables grow — OFFSET re-scans skipped rows.
//!
//! The cursor is an opaque, URL-safe string: hex of `"<rfc3339>|<uuid>"`. It is
//! deliberately opaque so clients treat it as a token, not a queryable field.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

pub const DEFAULT_LIMIT: i64 = 25;
pub const MAX_LIMIT: i64 = 100;

/// Clamp a client-supplied limit into the allowed range.
pub fn clamp_limit(requested: Option<i64>) -> i64 {
    requested.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Encode a `(created_at, id)` pair into an opaque cursor.
pub fn encode_cursor(created_at: DateTime<Utc>, id: Uuid) -> String {
    hex::encode(format!("{}|{}", created_at.to_rfc3339(), id))
}

/// Decode a cursor back into its `(created_at, id)` pair, if well-formed.
pub fn decode_cursor(cursor: &str) -> Option<(DateTime<Utc>, Uuid)> {
    let bytes = hex::decode(cursor).ok()?;
    let raw = String::from_utf8(bytes).ok()?;
    let (ts, id) = raw.split_once('|')?;
    let created_at = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&Utc);
    let id = Uuid::parse_str(id).ok()?;
    Some((created_at, id))
}

/// A standard list envelope: `{ "data": [...], "next_cursor": "..." | null }`.
#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    pub fn new(data: Vec<T>, next_cursor: Option<String>) -> Self {
        Self { data, next_cursor }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips() {
        let id = Uuid::now_v7();
        let now = Utc::now();
        let c = encode_cursor(now, id);
        let (ts, got) = decode_cursor(&c).unwrap();
        assert_eq!(got, id);
        assert_eq!(ts.timestamp_micros(), now.timestamp_micros());
    }

    #[test]
    fn bad_cursor_is_none() {
        assert!(decode_cursor("not-hex-zzz").is_none());
        assert!(decode_cursor(&hex::encode("garbage-no-pipe")).is_none());
    }
}
