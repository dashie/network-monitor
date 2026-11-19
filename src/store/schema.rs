//! Database schema and its version.
//!
//! The schema is owned by the store module: no other component issues DDL (see
//! "Data ownership" in docs/ARCHITECTURE.md).

/// Schema version written to `schema_meta` by this binary.
pub const SCHEMA_VERSION: i64 = 5;

/// The schema version that added the single-row `monitor_liveness` table.
pub const LIVENESS_SCHEMA: i64 = 4;

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
    rtt_us         INTEGER,
    cause          TEXT,
    cause_message  TEXT
);

CREATE INDEX IF NOT EXISTS events_target_time
    ON events (target, occurred_at_ms);

CREATE TABLE IF NOT EXISTS targets (
    target            TEXT PRIMARY KEY,
    interface         TEXT,
    label             TEXT,
    priority          INTEGER,
    current_state     TEXT    NOT NULL,
    first_seen_at_ms  INTEGER NOT NULL,
    last_change_at_ms INTEGER NOT NULL,
    last_rtt_us       INTEGER
);

CREATE TABLE IF NOT EXISTS monitor_liveness (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    started_at_ms INTEGER NOT NULL,
    last_seen_ms  INTEGER NOT NULL,
    gap_after_ms  INTEGER NOT NULL,
    stopped_at_ms INTEGER
);
";
