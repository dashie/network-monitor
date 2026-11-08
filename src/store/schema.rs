//! Database schema and its version.
//!
//! The schema is owned by the store module: no other component issues DDL (see
//! "Data ownership" in docs/ARCHITECTURE.md).

/// Schema version written to `schema_meta` by this binary.
pub const SCHEMA_VERSION: i64 = 1;

/// Connection pragmas: WAL for concurrent readers, `NORMAL` sync for throughput,
/// and a busy timeout so a reader never fails on a short write lock.
pub const PRAGMAS: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
";

pub const SCHEMA_META: &str = "
CREATE TABLE IF NOT EXISTS schema_meta (
    version INTEGER NOT NULL
);
";

pub const TABLES: &str = "
CREATE TABLE IF NOT EXISTS events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    target         TEXT    NOT NULL,
    interface      TEXT,
    previous_state TEXT    NOT NULL,
    state          TEXT    NOT NULL,
    occurred_at_ms INTEGER NOT NULL,
    rtt_us         INTEGER
);

CREATE INDEX IF NOT EXISTS events_target_time
    ON events (target, occurred_at_ms);

CREATE TABLE IF NOT EXISTS targets (
    target            TEXT PRIMARY KEY,
    interface         TEXT,
    label             TEXT,
    current_state     TEXT    NOT NULL,
    first_seen_at_ms  INTEGER NOT NULL,
    last_change_at_ms INTEGER NOT NULL,
    last_rtt_us       INTEGER
);
";
