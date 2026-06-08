//! Persistent activity log.
//!
//! Every association event and every DIMSE message that flows through
//! `core::dimse` is recorded here and re-broadcast as an `activity`
//! Tauri event for the live UI. The log is capped at `CAP_ROWS` rows
//! and trimmed in-line every `TRIM_INTERVAL` inserts so the SQLite file
//! does not grow without bound during long-running sessions.
//!
//! The activity table lives in the same `store.sqlite` file as the SOP
//! Instance index but opens its own `Connection` so its mutex does not
//! contend with the store-index mutex.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use super::dimse::ActivityEvent;
use super::error::AppError;

/// Hard upper bound on persisted rows. Older rows are trimmed when this
/// is exceeded.
pub const CAP_ROWS: i64 = 50_000;

/// We only check the row count every Nth insert — amortises the trim
/// cost so a busy session does not run `COUNT(*)` on every message.
const TRIM_INTERVAL: u64 = 500;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS activity_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp_ms    INTEGER NOT NULL,
    direction       TEXT    NOT NULL,
    peer_ae_title   TEXT,
    peer_host       TEXT,
    command         TEXT,
    status          TEXT    NOT NULL,
    message         TEXT    NOT NULL,
    association_id  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_activity_ts    ON activity_events(timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_activity_assoc ON activity_events(association_id);
";

/// A persisted row: the inbound `ActivityEvent` plus its database id.
///
/// `#[serde(flatten)]` produces a flat JSON shape (`{id, timestamp_ms,
/// direction, …}`) so the frontend code does not need a wrapper type.
#[derive(Debug, Clone, Serialize)]
pub struct PersistedActivityEvent {
    pub id: i64,
    #[serde(flatten)]
    pub event: ActivityEvent,
}

/// Free-text + categorical filter applied to `list`. Every field is
/// optional; omitted fields do not narrow the result set. Strings are
/// matched case-insensitively.
#[derive(Debug, Default, Clone, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ActivityFilter {
    pub direction: Option<String>,
    pub status: Option<String>,
    pub peer_ae_title: Option<String>,
    pub command: Option<String>,
    pub association_id: Option<String>,
    /// Free-text substring search over `message` (and `peer_ae_title`
    /// for convenience).
    pub search: Option<String>,
    /// Lower bound on `timestamp_ms`; results return events newer than
    /// this. Useful for incremental polling.
    pub since_ms: Option<i64>,
    /// Page size. Capped at `MAX_LIMIT` even when callers ask for more.
    pub limit: Option<i64>,
    /// Zero-based row offset from the start of the (filtered, newest-
    /// first) result set. Combined with `limit` to drive page-based
    /// pagination on the UI.
    pub offset: Option<i64>,
}

/// A page of activity events plus the total matching count, so the UI
/// can render `Page X of Y` without a separate round trip.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityPage {
    pub events: Vec<PersistedActivityEvent>,
    pub total: i64,
}

const DEFAULT_LIMIT: i64 = 500;
const MAX_LIMIT: i64 = 5000;

/// Persistent activity log backed by SQLite.
pub struct ActivityLog {
    conn: Mutex<Connection>,
    insert_count: AtomicU64,
}

impl ActivityLog {
    /// Opens (or creates) the activity tables in the given SQLite file.
    /// Idempotent.
    pub fn open(path: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
            insert_count: AtomicU64::new(0),
        })
    }

    /// Persists one event and returns the row with its assigned id.
    pub fn record(&self, event: ActivityEvent) -> Result<PersistedActivityEvent, AppError> {
        let conn = self.lock()?;

        conn.execute(
            "INSERT INTO activity_events
                (timestamp_ms, direction, peer_ae_title, peer_host,
                 command, status, message, association_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.timestamp_ms,
                serde_to_token(&event.direction),
                event.peer_ae_title,
                event.peer_host,
                event.command,
                serde_to_token(&event.status),
                event.message,
                event.association_id,
            ],
        )?;
        let id = conn.last_insert_rowid();

        drop(conn);

        // Periodic trim, off the hot path.
        let n = self.insert_count.fetch_add(1, Ordering::Relaxed);
        if n % TRIM_INTERVAL == 0 {
            // Lock again briefly — keeps the trim sequential with
            // other inserts but avoids holding the lock across the
            // `insert_count` update.
            if let Err(err) = self.trim_if_needed() {
                tracing::warn!(error = %err, "activity trim failed");
            }
        }

        Ok(PersistedActivityEvent { id, event })
    }

    /// Returns a page of matching rows (newest first) plus the total
    /// matching count, so the UI can render `Page X of Y` without a
    /// second round trip.
    pub fn list(&self, filter: ActivityFilter) -> Result<ActivityPage, AppError> {
        let conn = self.lock()?;

        let limit = filter.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let offset = filter.offset.unwrap_or(0).max(0);

        // Build the WHERE clause dynamically. Each clause appends to
        // `where_parts` and pushes a value into `bound`. We reuse the
        // bindings for both the SELECT and the COUNT — that is why
        // they are collected first.
        let mut where_parts: Vec<&'static str> = Vec::new();
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(v) = filter.direction.as_deref().filter(|s| !s.is_empty()) {
            where_parts.push("direction = ?");
            bound.push(Box::new(v.to_string()));
        }
        if let Some(v) = filter.status.as_deref().filter(|s| !s.is_empty()) {
            where_parts.push("status = ?");
            bound.push(Box::new(v.to_string()));
        }
        if let Some(v) = filter.peer_ae_title.as_deref().filter(|s| !s.is_empty()) {
            where_parts.push("peer_ae_title = ?");
            bound.push(Box::new(v.to_string()));
        }
        if let Some(v) = filter.command.as_deref().filter(|s| !s.is_empty()) {
            where_parts.push("command = ?");
            bound.push(Box::new(v.to_string()));
        }
        if let Some(v) = filter.association_id.as_deref().filter(|s| !s.is_empty()) {
            where_parts.push("association_id = ?");
            bound.push(Box::new(v.to_string()));
        }
        if let Some(v) = filter.search.as_deref().filter(|s| !s.is_empty()) {
            where_parts.push("(message LIKE ? OR COALESCE(peer_ae_title,'') LIKE ?)");
            let pattern = format!("%{v}%");
            bound.push(Box::new(pattern.clone()));
            bound.push(Box::new(pattern));
        }
        if let Some(since) = filter.since_ms {
            where_parts.push("timestamp_ms > ?");
            bound.push(Box::new(since));
        }

        let where_sql = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };

        let count_params: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM activity_events {where_sql}"),
            count_params.as_slice(),
            |r| r.get(0),
        )?;

        let sql = format!(
            "SELECT id, timestamp_ms, direction, peer_ae_title, peer_host,
                    command, status, message, association_id
             FROM activity_events
             {where_sql}
             ORDER BY id DESC
             LIMIT {limit} OFFSET {offset}"
        );

        let mut stmt = conn.prepare(&sql)?;
        let select_params: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let events = stmt
            .query_map(select_params.as_slice(), map_persisted_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ActivityPage { events, total })
    }

    /// Deletes every row. The UI exposes this behind a "Clear log"
    /// button; it does not affect the SOP Instance index.
    pub fn clear(&self) -> Result<(), AppError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM activity_events", [])?;
        Ok(())
    }

    /// Returns the current row count. Used by the UI header.
    pub fn count(&self) -> Result<i64, AppError> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM activity_events", [], |r| r.get(0))?;
        Ok(n)
    }

    /// If the table is over `CAP_ROWS`, deletes the oldest rows. Called
    /// from `record` every `TRIM_INTERVAL` inserts.
    fn trim_if_needed(&self) -> Result<(), AppError> {
        let conn = self.lock()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM activity_events", [], |r| r.get(0))?;
        if count > CAP_ROWS {
            let to_delete = count - CAP_ROWS;
            // Delete the oldest `to_delete` rows.
            conn.execute(
                "DELETE FROM activity_events
                 WHERE id IN (
                    SELECT id FROM activity_events ORDER BY id ASC LIMIT ?1
                 )",
                params![to_delete],
            )?;
            tracing::info!(deleted = to_delete, "activity log trimmed");
        }
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, AppError> {
        self.conn
            .lock()
            .map_err(|_| AppError::Internal("activity log mutex poisoned".to_string()))
    }
}

fn map_persisted_row(row: &Row<'_>) -> rusqlite::Result<PersistedActivityEvent> {
    let direction: String = row.get(2)?;
    let status: String = row.get(6)?;
    let event = ActivityEvent {
        timestamp_ms: row.get(1)?,
        direction: token_to_direction(&direction),
        peer_ae_title: row.get(3)?,
        peer_host: row.get(4)?,
        command: row.get(5)?,
        status: token_to_status(&status),
        message: row.get(7)?,
        association_id: row.get(8)?,
    };
    Ok(PersistedActivityEvent {
        id: row.get(0)?,
        event,
    })
}

// ----- Direction / Status token helpers ----- //
//
// We persist the lowercase JSON discriminator (`inbound`, `outbound`,
// `info`, `success`, `warning`, `error`) rather than the Rust enum
// integer so the SQLite file is readable from `sqlite3` without any
// decoding, and so future enum-variant additions don't break old rows.

use super::dimse::{Direction, Status};

fn serde_to_token<T: Serialize>(value: &T) -> String {
    // `serde_json::to_value` is the cheapest way to extract the
    // discriminator we set on the enum (`#[serde(rename_all = "lowercase")]`).
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "unknown".to_string(),
    }
}

fn token_to_direction(token: &str) -> Direction {
    match token {
        "inbound" => Direction::Inbound,
        "outbound" => Direction::Outbound,
        _ => Direction::Info,
    }
}

fn token_to_status(token: &str) -> Status {
    match token {
        "success" => Status::Success,
        "warning" => Status::Warning,
        "error" => Status::Error,
        _ => Status::Info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log() -> (tempfile::TempDir, ActivityLog) {
        // `tempfile::TempDir` guarantees a unique directory per call
        // even when tests run in parallel — a previous nanos-based
        // scheme could collide on the same nanosecond and leak rows
        // between tests, breaking the count assertions.
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("activity.sqlite");
        let log = ActivityLog::open(&path).expect("open");
        (dir, log)
    }

    fn sample_event(message: &str) -> ActivityEvent {
        ActivityEvent {
            timestamp_ms: 1_700_000_000_000,
            direction: Direction::Inbound,
            peer_ae_title: Some("TESTSCU".to_string()),
            peer_host: Some("127.0.0.1:54321".to_string()),
            command: Some("C-ECHO-RQ".to_string()),
            status: Status::Info,
            message: message.to_string(),
            association_id: "a-1".to_string(),
        }
    }

    #[test]
    fn record_then_list_returns_event() {
        let (_path, log) = temp_log();
        let persisted = log.record(sample_event("hello")).expect("record");
        assert!(persisted.id > 0);

        let page = log.list(ActivityFilter::default()).expect("list");
        assert_eq!(page.total, 1);
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].event.message, "hello");
    }

    #[test]
    fn filter_by_search_matches_substring() {
        let (_path, log) = temp_log();
        log.record(sample_event("association accepted")).unwrap();
        log.record(sample_event("C-ECHO success")).unwrap();
        log.record(sample_event("release acknowledged")).unwrap();

        let filter = ActivityFilter {
            search: Some("echo".to_string()),
            ..Default::default()
        };
        let page = log.list(filter).expect("list");
        assert_eq!(page.total, 1);
        assert_eq!(page.events.len(), 1);
        assert!(page.events[0].event.message.contains("ECHO"));
    }

    #[test]
    fn clear_removes_every_row() {
        let (_path, log) = temp_log();
        log.record(sample_event("x")).unwrap();
        log.record(sample_event("y")).unwrap();
        assert_eq!(log.count().unwrap(), 2);
        log.clear().unwrap();
        assert_eq!(log.count().unwrap(), 0);
    }

    #[test]
    fn newer_rows_come_first() {
        let (_path, log) = temp_log();
        log.record(sample_event("first")).unwrap();
        log.record(sample_event("second")).unwrap();
        log.record(sample_event("third")).unwrap();

        let page = log.list(ActivityFilter::default()).expect("list");
        assert_eq!(page.events[0].event.message, "third");
        assert_eq!(page.events[2].event.message, "first");
    }

    #[test]
    fn limit_caps_returned_rows() {
        let (_path, log) = temp_log();
        for i in 0..10 {
            log.record(sample_event(&format!("msg-{i}"))).unwrap();
        }
        let page = log
            .list(ActivityFilter {
                limit: Some(3),
                ..Default::default()
            })
            .expect("list");
        assert_eq!(page.events.len(), 3);
        assert_eq!(page.total, 10);
    }

    #[test]
    fn offset_skips_rows() {
        let (_path, log) = temp_log();
        for i in 0..10 {
            log.record(sample_event(&format!("msg-{i}"))).unwrap();
        }
        // Newest first means msg-9 .. msg-0. Offset 5 with limit 3
        // should return msg-4, msg-3, msg-2.
        let page = log
            .list(ActivityFilter {
                limit: Some(3),
                offset: Some(5),
                ..Default::default()
            })
            .expect("list");
        assert_eq!(page.events.len(), 3);
        assert_eq!(page.events[0].event.message, "msg-4");
        assert_eq!(page.events[2].event.message, "msg-2");
        assert_eq!(page.total, 10);
    }
}
