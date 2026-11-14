//! Event database.
//!
//! Two tables: `events`, append-only, and `targets`, the current state of each
//! target. A transition writes both in one transaction, so a reader never sees a
//! state that contradicts the latest event (INV-7).

pub mod schema;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use thiserror::Error;

use schema::{PRAGMAS, SCHEMA_META, SCHEMA_VERSION, TABLES};

/// The state of a target. `Unknown` before its first verdict (BR-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetState {
    Unknown,
    Up,
    Down,
}

impl TargetState {
    pub fn as_str(self) -> &'static str {
        match self {
            TargetState::Unknown => "unknown",
            TargetState::Up => "up",
            TargetState::Down => "down",
        }
    }

    pub fn parse(value: &str) -> Option<TargetState> {
        match value {
            "unknown" => Some(TargetState::Unknown),
            "up" => Some(TargetState::Up),
            "down" => Some(TargetState::Down),
            _ => None,
        }
    }
}

/// A transition to persist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub target: String,
    pub interface: Option<String>,
    pub label: Option<String>,
    pub previous_state: TargetState,
    pub state: TargetState,
    pub occurred_at_ms: i64,
    pub rtt_us: Option<i64>,
    /// The code of a disconnection, when the event is one.
    pub cause: Option<String>,
    /// The standard message of that cause, as it was rendered when it happened.
    pub cause_message: Option<String>,
}

/// A transition read back from the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub id: i64,
    pub target: String,
    pub interface: Option<String>,
    pub previous_state: TargetState,
    pub state: TargetState,
    pub occurred_at_ms: i64,
    pub rtt_us: Option<i64>,
    /// The code of a disconnection, when the event is one.
    pub cause: Option<String>,
    /// The standard message of that cause, as it was rendered.
    pub cause_message: Option<String>,
}

/// The current state of a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetStateRow {
    pub target: String,
    pub interface: Option<String>,
    pub label: Option<String>,
    pub state: TargetState,
    pub first_seen_at_ms: i64,
    pub last_change_at_ms: i64,
    pub last_rtt_us: Option<i64>,
}

/// Filters for the event history.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub target: Option<String>,
    pub since_ms: Option<i64>,
    pub limit: Option<u32>,
}

/// One interval of constant state on a timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineSegment {
    pub state: TargetState,
    pub from_ms: i64,
    pub to_ms: i64,
}

/// One target's state over a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetTimeline {
    pub target: String,
    pub label: Option<String>,
    pub segments: Vec<TimelineSegment>,
}

/// A state change inside a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionPoint {
    pub occurred_at_ms: i64,
    pub state: TargetState,
}

/// One day of the disconnection histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyDrops {
    /// Start of the UTC day, in epoch milliseconds.
    pub day_start_ms: i64,
    pub drops: u32,
    /// Time spent in `down` inside the day, clipped to the queried window.
    pub down_ms: i64,
}

/// One event as the export writes it: the label comes from the target row, because the
/// events table stores the identity of the target, not its current name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedEvent {
    pub id: i64,
    pub target: String,
    pub label: Option<String>,
    pub interface: Option<String>,
    pub state: TargetState,
    pub occurred_at_ms: i64,
    pub rtt_us: Option<i64>,
    pub cause: Option<String>,
    pub cause_message: Option<String>,
}

/// One fault: a target that went down, why, and — if it came back — when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// Id of the `down` event, used to page through the history.
    pub event_id: i64,
    pub target: String,
    /// The label of the target, so the list names it the way the chart does.
    pub label: Option<String>,
    pub started_at_ms: i64,
    /// `None` while the target is still down.
    pub ended_at_ms: Option<i64>,
    /// The code of the failure that started it; `None` on a row written before causes.
    pub cause: Option<String>,
    /// The standard message of that cause, as it was rendered.
    pub cause_message: Option<String>,
}

const DAY_MS: i64 = 86_400_000;

/// The columns of one fault: event id, target, label, start, end, and the two of the
/// cause. The end is `NULL` while the target is still down. Shared by both pages.
/// The schema version that added `events.cause` and `events.cause_message`. A read-only
/// open of an older file selects `NULL` in their place.
const CAUSE_COLUMNS: i64 = 2;

const FAULTS_SELECT: &str = "
SELECT e.id, e.target, t.label, e.occurred_at_ms,
       (SELECT MIN(n.occurred_at_ms) FROM events n
         WHERE n.target = e.target AND n.occurred_at_ms > e.occurred_at_ms),
       {cause}
FROM events e
LEFT JOIN targets t ON t.target = e.target
WHERE e.state = 'down'";

/// Split the part of `[start_ms, end_ms)` that falls inside `[from_ms, to_ms)` into
/// per-UTC-day slices: the start of the day and the milliseconds spent inside it.
fn day_slices(start_ms: i64, end_ms: i64, from_ms: i64, to_ms: i64) -> Vec<(i64, i64)> {
    let mut slices = Vec::new();
    let mut cursor = start_ms.max(from_ms);
    let end = end_ms.min(to_ms);
    while cursor < end {
        let day = cursor.div_euclid(DAY_MS) * DAY_MS;
        let next = (day + DAY_MS).min(end);
        slices.push((day, next - cursor));
        cursor = next;
    }
    slices
}

/// Turn a starting state and the transitions of a window into segments that cover
/// the window exactly. Pure, so the rendering and the SQL share the same rule.
pub fn timeline_segments(
    state_before: TargetState,
    transitions: &[TransitionPoint],
    from_ms: i64,
    to_ms: i64,
) -> Vec<TimelineSegment> {
    if to_ms <= from_ms {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut cursor = from_ms;
    let mut state = state_before;

    for point in transitions {
        let at = point.occurred_at_ms.clamp(from_ms, to_ms);
        if at > cursor {
            segments.push(TimelineSegment {
                state,
                from_ms: cursor,
                to_ms: at,
            });
            cursor = at;
        }
        state = point.state;
    }

    if cursor < to_ms {
        segments.push(TimelineSegment {
            state,
            from_ms: cursor,
            to_ms,
        });
    }

    segments
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("cannot open the event database {path}: {source}")]
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("cannot create the directory {path}: {source}")]
    Directory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the event database {path} does not exist; start the monitor first")]
    Missing { path: PathBuf },
    #[error("the event database {path} was not initialized by network-monitor")]
    Uninitialized { path: PathBuf },
    #[error(
        "the event database uses schema version {found}, newer than the supported version \
         {supported}; upgrade the binary or restore a backup"
    )]
    NewerSchema { found: i64, supported: i64 },
}

/// The event database of one process.
pub struct Store {
    connection: Connection,
    /// Whether the file carries the two cause columns. A read-only open of a database
    /// written before them reads `NULL` in their place instead of migrating the file.
    has_cause: bool,
}

/// A connection that cannot write, for every read-only entry point.
fn read_only_connection(path: &Path) -> Result<Connection, StoreError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| StoreError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(connection)
}

/// The schema version of an open database, refusing one this binary cannot read.
fn checked_version(connection: &Connection, path: &Path) -> Result<i64, StoreError> {
    let meta_exists: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'schema_meta'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if meta_exists.is_none() {
        return Err(StoreError::Uninitialized {
            path: path.to_path_buf(),
        });
    }

    let version: Option<i64> = connection
        .query_row("SELECT version FROM schema_meta LIMIT 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    let Some(version) = version else {
        return Err(StoreError::Uninitialized {
            path: path.to_path_buf(),
        });
    };
    if version > SCHEMA_VERSION {
        return Err(StoreError::NewerSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(version)
}

impl Store {
    /// Open the database for writing, creating the file, the schema, and the
    /// directories when they are absent.
    pub fn open_rw(path: &Path) -> Result<Store, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|source| StoreError::Directory {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }

        let connection = Connection::open(path).map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        let mut store = Store {
            connection,
            has_cause: true,
        };
        store.connection.execute_batch(PRAGMAS)?;
        store.migrate()?;
        store.rewrite_legacy_target_keys()?;
        Ok(store)
    }

    /// Open the database read-only. The file must already exist and carry a schema.
    pub fn open_ro(path: &Path) -> Result<Store, StoreError> {
        if !path.exists() {
            return Err(StoreError::Missing {
                path: path.to_path_buf(),
            });
        }

        let connection = read_only_connection(path)?;
        // A database one version behind is read as it is: the two cause columns it lacks
        // are selected as `NULL`, so a read-only process never writes to the file, and a
        // page can run against a snapshot or a volume mounted read-only (INV-10).
        let has_cause = checked_version(&connection, path)? >= CAUSE_COLUMNS;
        Ok(Store {
            connection,
            has_cause,
        })
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        self.connection.execute_batch(SCHEMA_META)?;
        let version: Option<i64> = self
            .connection
            .query_row("SELECT version FROM schema_meta LIMIT 1", [], |row| {
                row.get(0)
            })
            .optional()?;

        match version {
            None => {
                self.connection.execute_batch(TABLES)?;
                self.connection.execute(
                    "INSERT INTO schema_meta (version) VALUES (?1)",
                    params![SCHEMA_VERSION],
                )?;
            }
            Some(found) if found > SCHEMA_VERSION => {
                return Err(StoreError::NewerSchema {
                    found,
                    supported: SCHEMA_VERSION,
                })
            }
            Some(found) => {
                // Keep the table creation idempotent so a partially created database
                // heals on open, then apply the steps the version still misses.
                self.connection.execute_batch(TABLES)?;
                if found < 2 {
                    self.add_cause_columns()?;
                }
                if found < SCHEMA_VERSION {
                    self.connection.execute(
                        "UPDATE schema_meta SET version = ?1",
                        params![SCHEMA_VERSION],
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Add the two cause columns to a version 1 database, keeping every event.
    ///
    /// `CREATE TABLE IF NOT EXISTS` cannot add a column to a table that exists, so the
    /// step is written here: the rows that predate it read `NULL`.
    fn add_cause_columns(&mut self) -> Result<(), StoreError> {
        for column in ["cause", "cause_message"] {
            let present: Option<i64> = self
                .connection
                .query_row(
                    "SELECT 1 FROM pragma_table_info('events') WHERE name = ?1",
                    params![column],
                    |row| row.get(0),
                )
                .optional()?;
            if present.is_none() {
                self.connection
                    .execute_batch(&format!("ALTER TABLE events ADD COLUMN {column} TEXT"))?;
            }
        }
        Ok(())
    }

    /// Current state of every known target, keyed by address.
    pub fn load_states(&self) -> Result<HashMap<String, TargetState>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT target, current_state FROM targets")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut states = HashMap::new();
        for row in rows {
            let (target, state) = row?;
            let state = TargetState::parse(&state).unwrap_or(TargetState::Unknown);
            states.insert(target, state);
        }
        Ok(states)
    }

    /// Persist one transition and the resulting current state, atomically.
    pub fn record_transition(&mut self, event: &Event) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;

        transaction.execute(
            "INSERT INTO events
                (target, interface, previous_state, state, occurred_at_ms, rtt_us,
                 cause, cause_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.target,
                event.interface,
                event.previous_state.as_str(),
                event.state.as_str(),
                event.occurred_at_ms,
                event.rtt_us,
                event.cause,
                event.cause_message,
            ],
        )?;

        transaction.execute(
            "INSERT INTO targets
                (target, interface, label, current_state,
                 first_seen_at_ms, last_change_at_ms, last_rtt_us)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)
             ON CONFLICT(target) DO UPDATE SET
                interface         = excluded.interface,
                label             = excluded.label,
                current_state     = excluded.current_state,
                last_change_at_ms = excluded.last_change_at_ms,
                last_rtt_us       = excluded.last_rtt_us",
            params![
                event.target,
                event.interface,
                event.label,
                event.state.as_str(),
                event.occurred_at_ms,
                event.rtt_us,
            ],
        )?;

        transaction.commit()?;
        Ok(())
    }

    /// Insert imported events that are not in the database yet, in one transaction,
    /// and keep the target projection consistent with the newest event.
    ///
    /// An event already present — same target, state, and instant — is left alone, so
    /// importing the same archive twice changes nothing. The projection never moves
    /// backwards: `first_seen_at_ms` takes the minimum, `last_change_at_ms` the
    /// maximum, and the current state follows the newest event.
    pub fn import_events(&mut self, events: &[Event]) -> Result<u32, StoreError> {
        let transaction = self.connection.transaction()?;
        let mut inserted = 0;

        {
            let mut insert = transaction.prepare(
                "INSERT INTO events
                    (target, interface, previous_state, state, occurred_at_ms, rtt_us,
                     cause, cause_message)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
                  WHERE NOT EXISTS (
                     SELECT 1 FROM events
                      WHERE target = ?1 AND state = ?4 AND occurred_at_ms = ?5
                  )",
            )?;
            let mut project = transaction.prepare(
                "INSERT INTO targets
                    (target, interface, label, current_state,
                     first_seen_at_ms, last_change_at_ms, last_rtt_us)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)
                 ON CONFLICT(target) DO UPDATE SET
                    first_seen_at_ms  = MIN(targets.first_seen_at_ms, excluded.first_seen_at_ms),
                    current_state     = CASE
                        WHEN excluded.last_change_at_ms >= targets.last_change_at_ms
                        THEN excluded.current_state ELSE targets.current_state END,
                    last_change_at_ms = MAX(targets.last_change_at_ms, excluded.last_change_at_ms),
                    interface         = COALESCE(targets.interface, excluded.interface),
                    label             = COALESCE(targets.label, excluded.label),
                    last_rtt_us       = COALESCE(excluded.last_rtt_us, targets.last_rtt_us)",
            )?;

            for event in events {
                inserted += insert.execute(params![
                    event.target,
                    event.interface,
                    event.previous_state.as_str(),
                    event.state.as_str(),
                    event.occurred_at_ms,
                    event.rtt_us,
                    event.cause,
                    event.cause_message,
                ])? as u32;
                project.execute(params![
                    event.target,
                    event.interface,
                    event.label,
                    event.state.as_str(),
                    event.occurred_at_ms,
                    event.rtt_us,
                ])?;
            }
        }

        transaction.commit()?;
        Ok(inserted)
    }

    /// Delete every event of one target inside `[from_ms, to_ms]`, inclusive.
    pub fn delete_events_for_target_between(
        &self,
        target: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<usize, StoreError> {
        let deleted = self.connection.execute(
            "DELETE FROM events
              WHERE target = ?1 AND occurred_at_ms BETWEEN ?2 AND ?3",
            params![target, from_ms, to_ms],
        )?;
        Ok(deleted)
    }

    /// Recompute every target row from the events that are left, and drop the targets
    /// that have none. Used after events have been deleted: the projection must never
    /// describe a history that is no longer there.
    pub fn rebuild_from_events(&self) -> Result<(), StoreError> {
        self.connection.execute_batch(
            "DELETE FROM targets
              WHERE NOT EXISTS (SELECT 1 FROM events WHERE events.target = targets.target);
             UPDATE targets SET
                first_seen_at_ms  = (SELECT MIN(e.occurred_at_ms) FROM events e
                                      WHERE e.target = targets.target),
                last_change_at_ms = (SELECT MAX(e.occurred_at_ms) FROM events e
                                      WHERE e.target = targets.target),
                current_state     = (SELECT e.state FROM events e
                                      WHERE e.target = targets.target
                                      ORDER BY e.occurred_at_ms DESC, e.id DESC LIMIT 1),
                last_rtt_us       = (SELECT e.rtt_us FROM events e
                                      WHERE e.target = targets.target
                                      ORDER BY e.occurred_at_ms DESC, e.id DESC LIMIT 1)",
        )?;
        Ok(())
    }

    /// Make the stored chain consistent whatever order the events arrived in.
    ///
    /// Two repairs, both driven by the timestamps rather than by insertion order:
    /// an event that repeats the state of the one before it is dropped (BR-1), and every
    /// event is told which state it really replaced. Returns how many events were dropped.
    pub fn normalize_history(&self) -> Result<u32, StoreError> {
        let dropped = self.connection.execute(
            "DELETE FROM events WHERE id IN (
                 SELECT id FROM (
                     SELECT id,
                            state AS current,
                            LAG(state) OVER (
                                PARTITION BY target ORDER BY occurred_at_ms, id
                            ) AS previous_state
                     FROM events
                 )
                 WHERE previous_state = current
             )",
            [],
        )? as u32;

        self.connection.execute(
            "UPDATE events SET previous_state = COALESCE((
                 SELECT earlier.state FROM events earlier
                  WHERE earlier.target = events.target
                    AND (earlier.occurred_at_ms < events.occurred_at_ms
                         OR (earlier.occurred_at_ms = events.occurred_at_ms
                             AND earlier.id < events.id))
                  ORDER BY earlier.occurred_at_ms DESC, earlier.id DESC
                  LIMIT 1), 'unknown')",
            [],
        )?;

        Ok(dropped)
    }

    /// Delete events older than `before_ms`; the `targets` rows are kept (FR-10).
    pub fn prune(&self, before_ms: i64) -> Result<usize, StoreError> {
        let deleted = self.connection.execute(
            "DELETE FROM events WHERE occurred_at_ms < ?1",
            params![before_ms],
        )?;
        Ok(deleted)
    }

    /// Current state of every target, ordered by address.
    pub fn states(&self) -> Result<Vec<TargetStateRow>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT target, interface, label, current_state,
                    first_seen_at_ms, last_change_at_ms, last_rtt_us
             FROM targets
             ORDER BY target",
        )?;
        let rows = statement.query_map([], |row| {
            let state: String = row.get(3)?;
            Ok(TargetStateRow {
                target: row.get(0)?,
                interface: row.get(1)?,
                label: row.get(2)?,
                state: TargetState::parse(&state).unwrap_or(TargetState::Unknown),
                first_seen_at_ms: row.get(4)?,
                last_change_at_ms: row.get(5)?,
                last_rtt_us: row.get(6)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The faults query, with the cause columns it can read from this file.
    fn faults_select(&self) -> String {
        FAULTS_SELECT.replace("{cause}", self.cause_columns())
    }

    /// The cause columns of a read, or `NULL` in their place on a file that predates them.
    fn cause_columns(&self) -> &'static str {
        if self.has_cause {
            "cause, cause_message"
        } else {
            "NULL, NULL"
        }
    }

    /// Recorded transitions, newest first.
    pub fn events(&self, filter: &EventFilter) -> Result<Vec<EventRow>, StoreError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, target, interface, previous_state, state, occurred_at_ms, rtt_us,
                    {cause}
             FROM events
             WHERE (?1 IS NULL OR target = ?1)
               AND (?2 IS NULL OR occurred_at_ms >= ?2)
             ORDER BY occurred_at_ms DESC, id DESC
             LIMIT COALESCE(?3, -1)",
            cause = self.cause_columns(),
        ))?;
        let rows = statement.query_map(
            params![filter.target, filter.since_ms, filter.limit],
            |row| {
                let previous_state: String = row.get(3)?;
                let state: String = row.get(4)?;
                Ok(EventRow {
                    id: row.get(0)?,
                    target: row.get(1)?,
                    interface: row.get(2)?,
                    previous_state: TargetState::parse(&previous_state)
                        .unwrap_or(TargetState::Unknown),
                    state: TargetState::parse(&state).unwrap_or(TargetState::Unknown),
                    occurred_at_ms: row.get(5)?,
                    rtt_us: row.get(6)?,
                    cause: row.get(7)?,
                    cause_message: row.get(8)?,
                })
            },
        )?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Give the targets recorded before methods existed their canonical key.
    ///
    /// A bare `1.1.1.1` becomes `icmp://1.1.1.1`, so an older history keeps its identity
    /// and its lanes. Idempotent: a key that already carries a scheme is left alone.
    pub fn rewrite_legacy_target_keys(&self) -> Result<usize, StoreError> {
        let events = self.connection.execute(
            "UPDATE events SET target = 'icmp://' || target WHERE instr(target, '://') = 0",
            [],
        )?;
        let targets = self.connection.execute(
            "UPDATE targets SET target = 'icmp://' || target WHERE instr(target, '://') = 0",
            [],
        )?;
        Ok(events + targets)
    }

    /// Resolve a target key or label to the key it identifies.
    ///
    /// A bare address is accepted for an ICMP target, because that is how the tool was
    /// used before methods existed.
    pub fn resolve_target(&self, needle: &str) -> Result<Option<String>, StoreError> {
        let target = self
            .connection
            .query_row(
                "SELECT target FROM targets
                 WHERE target = ?1 OR label = ?1 OR target = 'icmp://' || ?1
                 LIMIT 1",
                params![needle],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(target)
    }

    /// One entry per target, chosen by address or label.
    pub fn find_target(&self, needle: &str) -> Result<Option<TargetStateRow>, StoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT target, interface, label, current_state,
                        first_seen_at_ms, last_change_at_ms, last_rtt_us
                 FROM targets
                 WHERE target = ?1 OR label = ?1 OR target = 'icmp://' || ?1
                 LIMIT 1",
                params![needle],
                |row| {
                    let state: String = row.get(3)?;
                    Ok(TargetStateRow {
                        target: row.get(0)?,
                        interface: row.get(1)?,
                        label: row.get(2)?,
                        state: TargetState::parse(&state).unwrap_or(TargetState::Unknown),
                        first_seen_at_ms: row.get(4)?,
                        last_change_at_ms: row.get(5)?,
                        last_rtt_us: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Number of transitions to `down` and time spent down per UTC day, one entry per
    /// day in the window.
    ///
    /// A disconnection that opened before the window is not counted as a drop, but the
    /// part of it that falls inside the window is counted as down time. An open
    /// disconnection ends at `to_ms`, the end of the window.
    pub fn daily_drops(
        &self,
        target: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<DailyDrops>, StoreError> {
        let query = format!(
            "{} AND e.target = ?1 AND e.occurred_at_ms < ?2",
            self.faults_select()
        );
        let mut statement = self.connection.prepare(&query)?;

        let mut buckets: HashMap<i64, DailyDrops> = HashMap::new();
        let mut day = from_ms.div_euclid(DAY_MS) * DAY_MS;
        while day < to_ms {
            buckets.insert(
                day,
                DailyDrops {
                    day_start_ms: day,
                    drops: 0,
                    down_ms: 0,
                },
            );
            day += DAY_MS;
        }

        let episodes = statement.query_map(params![target, to_ms], |row| {
            Ok((row.get::<_, i64>(3)?, row.get::<_, Option<i64>>(4)?))
        })?;
        for episode in episodes {
            let (started_at_ms, ended_at_ms) = episode?;
            let ended_at_ms = ended_at_ms.unwrap_or(to_ms);
            if started_at_ms >= from_ms && started_at_ms < to_ms {
                if let Some(bucket) = buckets.get_mut(&(started_at_ms.div_euclid(DAY_MS) * DAY_MS))
                {
                    bucket.drops += 1;
                }
            }
            for (day, down_ms) in day_slices(started_at_ms, ended_at_ms, from_ms, to_ms) {
                if let Some(bucket) = buckets.get_mut(&day) {
                    bucket.down_ms += down_ms;
                }
            }
        }

        let mut days = Vec::new();
        let mut day = from_ms.div_euclid(DAY_MS) * DAY_MS;
        while day < to_ms {
            if let Some(bucket) = buckets.remove(&day) {
                days.push(bucket);
            }
            day += DAY_MS;
        }
        Ok(days)
    }

    /// One page of the history in insertion order, for a streamed export.
    ///
    /// Paging keeps every read transaction short, so a slow download never holds the
    /// WAL checkpoints of the writer. `target` narrows the page to one target, which is
    /// what the export of a detail page covers.
    pub fn events_after(
        &self,
        target: Option<&str>,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<ExportedEvent>, StoreError> {
        // The filter goes in the statement rather than in a `?3 IS NULL OR` clause, so the
        // index on the target still narrows the scan.
        let filter = if target.is_some() {
            "AND e.target = ?3"
        } else {
            ""
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT e.id, e.target, t.label, e.interface, e.state, e.occurred_at_ms, e.rtt_us,
                    {cause}
               FROM events e
               LEFT JOIN targets t ON t.target = e.target
              WHERE e.id > ?1 {filter}
              ORDER BY e.id
              LIMIT ?2",
            cause = self.cause_columns(),
        ))?;

        let event = |row: &rusqlite::Row| -> rusqlite::Result<ExportedEvent> {
            let state: String = row.get(4)?;
            Ok(ExportedEvent {
                id: row.get(0)?,
                target: row.get(1)?,
                label: row.get(2)?,
                interface: row.get(3)?,
                state: TargetState::parse(&state).unwrap_or(TargetState::Unknown),
                occurred_at_ms: row.get(5)?,
                rtt_us: row.get(6)?,
                cause: row.get(7)?,
                cause_message: row.get(8)?,
            })
        };
        let rows = match target {
            Some(target) => statement
                .query_map(params![after_id, limit, target], event)?
                .collect::<Result<Vec<_>, _>>()?,
            None => statement
                .query_map(params![after_id, limit], event)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    /// The most recent faults of every target, newest first.
    pub fn recent_faults(&self, limit: u32) -> Result<Vec<Fault>, StoreError> {
        let query = format!(
            "{} ORDER BY e.occurred_at_ms DESC, e.id DESC LIMIT ?1",
            self.faults_select()
        );
        let mut statement = self.connection.prepare(&query)?;

        let rows = statement
            .query_map(params![limit], |row| {
                Ok(Fault {
                    event_id: row.get(0)?,
                    target: row.get(1)?,
                    label: row.get(2)?,
                    started_at_ms: row.get(3)?,
                    ended_at_ms: row.get(4)?,
                    cause: row.get(5)?,
                    cause_message: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The most recent faults of one target, newest first.
    pub fn recent_faults_for(&self, target: &str, limit: u32) -> Result<Vec<Fault>, StoreError> {
        let query = format!(
            "{} AND e.target = ?1 ORDER BY e.occurred_at_ms DESC, e.id DESC LIMIT ?2",
            self.faults_select()
        );
        let mut statement = self.connection.prepare(&query)?;

        let rows = statement
            .query_map(params![target, limit], |row| {
                Ok(Fault {
                    event_id: row.get(0)?,
                    target: row.get(1)?,
                    label: row.get(2)?,
                    started_at_ms: row.get(3)?,
                    ended_at_ms: row.get(4)?,
                    cause: row.get(5)?,
                    cause_message: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// One page of faults in chronological order, for the streamed export.
    ///
    /// Paging keeps every read transaction short, so a slow download never holds the
    /// WAL checkpoints of the writer.
    pub fn faults_after(&self, after_id: i64, limit: u32) -> Result<Vec<Fault>, StoreError> {
        let query = format!(
            "{} AND e.id > ?1 ORDER BY e.id LIMIT ?2",
            self.faults_select()
        );
        let mut statement = self.connection.prepare(&query)?;

        let rows = statement
            .query_map(params![after_id, limit], |row| {
                Ok(Fault {
                    event_id: row.get(0)?,
                    target: row.get(1)?,
                    label: row.get(2)?,
                    started_at_ms: row.get(3)?,
                    ended_at_ms: row.get(4)?,
                    cause: row.get(5)?,
                    cause_message: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// State of every target over the window, as segments that cover it exactly.
    pub fn timeline(&self, from_ms: i64, to_ms: i64) -> Result<Vec<TargetTimeline>, StoreError> {
        if to_ms <= from_ms {
            return Ok(Vec::new());
        }

        let mut targets_statement = self
            .connection
            .prepare("SELECT target, label FROM targets ORDER BY target")?;
        let targets = targets_statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut before_statement = self.connection.prepare(
            "SELECT target, state FROM (
                 SELECT target, state,
                        ROW_NUMBER() OVER (
                            PARTITION BY target ORDER BY occurred_at_ms DESC, id DESC
                        ) AS position
                 FROM events
                 WHERE occurred_at_ms < ?1
             )
             WHERE position = 1",
        )?;
        let mut states_before = HashMap::new();
        for row in before_statement.query_map(params![from_ms], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (target, state) = row?;
            states_before.insert(
                target,
                TargetState::parse(&state).unwrap_or(TargetState::Unknown),
            );
        }

        let mut events_statement = self.connection.prepare(
            "SELECT target, state, occurred_at_ms FROM events
             WHERE occurred_at_ms >= ?1 AND occurred_at_ms < ?2
             ORDER BY target, occurred_at_ms, id",
        )?;
        let mut transitions: HashMap<String, Vec<TransitionPoint>> = HashMap::new();
        for row in events_statement.query_map(params![from_ms, to_ms], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })? {
            let (target, state, occurred_at_ms) = row?;
            transitions
                .entry(target)
                .or_default()
                .push(TransitionPoint {
                    occurred_at_ms,
                    state: TargetState::parse(&state).unwrap_or(TargetState::Unknown),
                });
        }

        Ok(targets
            .into_iter()
            .map(|(target, label)| {
                let state_before = states_before
                    .get(&target)
                    .copied()
                    .unwrap_or(TargetState::Unknown);
                let points = transitions.get(&target).map(Vec::as_slice).unwrap_or(&[]);
                TargetTimeline {
                    segments: timeline_segments(state_before, points, from_ms, to_ms),
                    target,
                    label,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn event(target: &str, previous: TargetState, state: TargetState, at: i64) -> Event {
        Event {
            target: target.to_string(),
            interface: None,
            label: None,
            previous_state: previous,
            state,
            occurred_at_ms: at,
            rtt_us: None,
            cause: None,
            cause_message: None,
        }
    }

    #[test]
    fn writes_events_and_the_current_state_together() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();

        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                1_000,
            ))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 2_000))
            .unwrap();

        let states = store.states().unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].state, TargetState::Down);
        assert_eq!(states[0].first_seen_at_ms, 1_000);
        assert_eq!(states[0].last_change_at_ms, 2_000);

        let events = store.events(&EventFilter::default()).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].state, TargetState::Down);
        assert_eq!(events[1].state, TargetState::Up);
    }

    #[test]
    fn loads_the_last_known_state() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Unknown, TargetState::Up, 10))
            .unwrap();

        let states = store.load_states().unwrap();
        assert_eq!(states.get("1.1.1.1"), Some(&TargetState::Up));
    }

    /// The export of a detail page covers one target and nothing else.
    #[test]
    fn a_paged_export_names_the_target_it_reads() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        for (target, state, at) in [
            ("1.1.1.1", TargetState::Up, 1_000),
            ("8.8.8.8", TargetState::Up, 2_000),
            ("1.1.1.1", TargetState::Down, 3_000),
        ] {
            store
                .record_transition(&event(target, TargetState::Unknown, state, at))
                .unwrap();
        }

        let every = store.events_after(None, i64::MIN, 10).unwrap();
        assert_eq!(every.len(), 3, "the whole history when no target is named");

        let only = store.events_after(Some("1.1.1.1"), i64::MIN, 10).unwrap();
        assert_eq!(only.len(), 2);
        assert!(only.iter().all(|event| event.target == "1.1.1.1"));

        let after = store.events_after(Some("1.1.1.1"), only[0].id, 10).unwrap();
        assert_eq!(after.len(), 1, "the page still starts after the last id");
    }

    #[test]
    fn filters_events_by_target_and_time() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                1_000,
            ))
            .unwrap();
        store
            .record_transition(&event(
                "8.8.8.8",
                TargetState::Unknown,
                TargetState::Down,
                2_000,
            ))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 3_000))
            .unwrap();

        let filtered = store
            .events(&EventFilter {
                target: Some("1.1.1.1".to_string()),
                since_ms: Some(1_500),
                limit: None,
            })
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].occurred_at_ms, 3_000);

        let limited = store
            .events(&EventFilter {
                limit: Some(1),
                ..EventFilter::default()
            })
            .unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].occurred_at_ms, 3_000);
    }

    #[test]
    fn builds_segments_that_cover_the_window() {
        use TargetState::{Down, Unknown, Up};

        let segments = timeline_segments(
            Unknown,
            &[
                TransitionPoint {
                    occurred_at_ms: 120,
                    state: Up,
                },
                TransitionPoint {
                    occurred_at_ms: 180,
                    state: Down,
                },
            ],
            100,
            200,
        );

        assert_eq!(
            segments,
            vec![
                TimelineSegment {
                    state: Unknown,
                    from_ms: 100,
                    to_ms: 120,
                },
                TimelineSegment {
                    state: Up,
                    from_ms: 120,
                    to_ms: 180,
                },
                TimelineSegment {
                    state: Down,
                    from_ms: 180,
                    to_ms: 200,
                },
            ]
        );
    }

    #[test]
    fn a_window_without_transitions_is_one_segment() {
        let segments = timeline_segments(TargetState::Up, &[], 0, 100);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].state, TargetState::Up);
        assert_eq!((segments[0].from_ms, segments[0].to_ms), (0, 100));
    }

    #[test]
    fn the_timeline_starts_from_the_state_before_the_window() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                1_000,
            ))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 5_000))
            .unwrap();

        let timelines = store.timeline(4_000, 6_000).unwrap();
        assert_eq!(timelines.len(), 1);
        let segments = &timelines[0].segments;
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].state, TargetState::Up);
        assert_eq!((segments[0].from_ms, segments[0].to_ms), (4_000, 5_000));
        assert_eq!(segments[1].state, TargetState::Down);
        assert_eq!((segments[1].from_ms, segments[1].to_ms), (5_000, 6_000));
    }

    #[test]
    fn the_timeline_is_unknown_before_the_first_event() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                1_000,
            ))
            .unwrap();

        let timelines = store.timeline(500, 2_000).unwrap();
        let segments = &timelines[0].segments;
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].state, TargetState::Unknown);
        assert_eq!(segments[1].state, TargetState::Up);
    }

    #[test]
    fn daily_drops_has_one_entry_per_day() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Down,
                1_000,
            ))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Down, TargetState::Up, 2_000))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 3_000))
            .unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Down,
                TargetState::Up,
                DAY_MS + 1_000,
            ))
            .unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Up,
                TargetState::Down,
                DAY_MS + 2_000,
            ))
            .unwrap();

        let days = store.daily_drops("1.1.1.1", 0, 3 * DAY_MS).unwrap();
        assert_eq!(days.len(), 3);
        assert_eq!(days[0].drops, 2);
        assert_eq!(days[1].drops, 1);
        assert_eq!(days[2].drops, 0);
        assert_eq!(days[0].down_ms, DAY_MS - 2_000);
        assert_eq!(days[1].down_ms, DAY_MS - 1_000);
        assert_eq!(days[2].down_ms, DAY_MS, "the open one ends at the window");
    }

    #[test]
    fn a_disconnection_open_before_the_window_is_down_time_but_not_a_drop() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Down,
                1_000,
            ))
            .unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Down,
                TargetState::Up,
                5 * DAY_MS,
            ))
            .unwrap();

        let days = store
            .daily_drops("1.1.1.1", 2 * DAY_MS, 4 * DAY_MS)
            .unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].drops, 0);
        assert_eq!(days[0].down_ms, DAY_MS);
        assert_eq!(days[1].drops, 0);
        assert_eq!(days[1].down_ms, DAY_MS);
    }

    #[test]
    fn recent_faults_are_newest_first_and_keep_the_open_one() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                1_000,
            ))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 2_000))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Down, TargetState::Up, 5_000))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 9_000))
            .unwrap();

        let downs = store.recent_faults(10).unwrap();
        assert_eq!(downs.len(), 2);
        assert_eq!(downs[0].started_at_ms, 9_000);
        assert_eq!(downs[0].ended_at_ms, None, "the target is still down");
        assert_eq!(downs[1].started_at_ms, 2_000);
        assert_eq!(downs[1].ended_at_ms, Some(5_000));
        assert_eq!(downs[1].label, None, "the target carries no label");

        let mut labelled = event("9.9.9.9", TargetState::Unknown, TargetState::Down, 12_000);
        labelled.label = Some("quad9".to_string());
        labelled.cause = Some("TCP.REFUSED".to_string());
        labelled.cause_message = Some("the connection to 9.9.9.9:443 was refused".to_string());
        store.record_transition(&labelled).unwrap();

        let downs = store.recent_faults(10).unwrap();
        assert_eq!(downs[0].target, "9.9.9.9");
        assert_eq!(downs[0].label.as_deref(), Some("quad9"));
        assert_eq!(downs[0].cause.as_deref(), Some("TCP.REFUSED"));
        assert_eq!(
            downs[0].cause_message.as_deref(),
            Some("the connection to 9.9.9.9:443 was refused")
        );
        assert_eq!(downs[1].cause, None, "the earlier rows carry no cause");

        assert_eq!(store.recent_faults(1).unwrap().len(), 1);
    }

    #[test]
    fn the_downs_of_one_target_leave_the_others_out() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                1_000,
            ))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 2_000))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Down, TargetState::Up, 5_000))
            .unwrap();
        store
            .record_transition(&event(
                "9.9.9.9",
                TargetState::Unknown,
                TargetState::Down,
                9_000,
            ))
            .unwrap();

        let downs = store.recent_faults_for("1.1.1.1", 10).unwrap();
        assert_eq!(downs.len(), 1);
        assert_eq!(downs[0].target, "1.1.1.1");
        assert_eq!(downs[0].started_at_ms, 2_000);
        assert_eq!(downs[0].ended_at_ms, Some(5_000));

        let downs = store.recent_faults_for("9.9.9.9", 10).unwrap();
        assert_eq!(downs.len(), 1);
        assert_eq!(downs[0].started_at_ms, 9_000);
        assert_eq!(downs[0].ended_at_ms, None, "the target is still down");

        assert!(
            store.recent_faults_for("8.8.8.8", 10).unwrap().is_empty(),
            "a target without a drop lists none"
        );
    }

    #[test]
    fn a_target_can_be_found_by_address_or_label() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        let mut first = event("1.1.1.1", TargetState::Unknown, TargetState::Up, 1_000);
        first.label = Some("cloudflare-dns".to_string());
        store.record_transition(&first).unwrap();

        assert_eq!(
            store.find_target("1.1.1.1").unwrap().map(|row| row.target),
            Some("1.1.1.1".to_string())
        );
        assert_eq!(
            store
                .find_target("cloudflare-dns")
                .unwrap()
                .map(|row| row.target),
            Some("1.1.1.1".to_string())
        );
        assert!(store.find_target("8.8.8.8").unwrap().is_none());
    }

    #[test]
    fn prunes_old_events_but_keeps_targets() {
        let directory = tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                1_000,
            ))
            .unwrap();
        store
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 5_000))
            .unwrap();

        assert_eq!(store.prune(4_000).unwrap(), 1);
        assert_eq!(store.events(&EventFilter::default()).unwrap().len(), 1);
        assert_eq!(store.states().unwrap().len(), 1);
    }

    #[test]
    fn reads_one_database_while_it_is_written() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("events.db");
        let mut writer = Store::open_rw(&path).unwrap();
        writer
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                1_000,
            ))
            .unwrap();

        let reader = Store::open_ro(&path).unwrap();
        assert_eq!(reader.states().unwrap().len(), 1);

        writer
            .record_transition(&event("1.1.1.1", TargetState::Up, TargetState::Down, 2_000))
            .unwrap();
        assert_eq!(reader.states().unwrap()[0].state, TargetState::Down);
    }

    /// A database written by version 0.2: version 1, and no cause column.
    fn version_one_database(path: &std::path::Path) {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_meta (version INTEGER NOT NULL);
                 INSERT INTO schema_meta (version) VALUES (1);
                 CREATE TABLE events (
                     id             INTEGER PRIMARY KEY AUTOINCREMENT,
                     target         TEXT    NOT NULL,
                     interface      TEXT,
                     previous_state TEXT    NOT NULL,
                     state          TEXT    NOT NULL,
                     occurred_at_ms INTEGER NOT NULL,
                     rtt_us         INTEGER
                 );
                 CREATE TABLE targets (
                     target            TEXT PRIMARY KEY,
                     interface         TEXT,
                     label             TEXT,
                     current_state     TEXT    NOT NULL,
                     first_seen_at_ms  INTEGER NOT NULL,
                     last_change_at_ms INTEGER NOT NULL,
                     last_rtt_us       INTEGER
                 );
                 INSERT INTO events
                     (target, previous_state, state, occurred_at_ms)
                     VALUES ('icmp://1.1.1.1', 'unknown', 'down', 2000);",
            )
            .unwrap();
    }

    #[test]
    fn a_version_one_database_gains_the_cause_columns_and_keeps_its_events() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("events.db");
        version_one_database(&path);

        let store = Store::open_rw(&path).unwrap();
        let version: i64 = store
            .connection
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2, "the migration stamps the new version");

        let faults = store.recent_faults(10).unwrap();
        assert_eq!(faults.len(), 1, "the event written by 0.2 is still there");
        assert_eq!(faults[0].target, "icmp://1.1.1.1");
        assert_eq!(
            faults[0].cause, None,
            "a row that predates the column has none"
        );
        assert_eq!(faults[0].cause_message, None);

        // Opening it again must not try to add the columns twice.
        drop(store);
        assert!(Store::open_rw(&path).is_ok());
    }

    #[test]
    fn a_read_only_open_reads_a_database_without_the_cause_columns() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("events.db");
        version_one_database(&path);

        // `serve` and `query` open the database this way: every query reads the file as
        // it is, `NULL` in place of the cause columns it lacks.
        let store = Store::open_ro(&path).expect("an older database is read as it is");
        let faults = store.recent_faults(10).unwrap();
        assert_eq!(faults.len(), 1, "the event written by 0.2 is still there");
        assert_eq!(faults[0].cause, None);
        assert_eq!(faults[0].cause_message, None);

        let events = store.events(&EventFilter::default()).unwrap();
        assert_eq!(events.len(), 1, "the events query reads it too");
        assert_eq!(events[0].cause, None);

        let exported = store.events_after(None, 0, 10).unwrap();
        assert_eq!(exported.len(), 1, "so does the streamed export");
        assert_eq!(exported[0].cause, None);

        let version: i64 = store
            .connection
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1, "a read-only open writes nothing at all");
    }

    #[test]
    fn a_read_only_open_of_a_migrated_database_reads_the_cause() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("events.db");
        version_one_database(&path);
        Store::open_rw(&path).unwrap();

        let store = Store::open_ro(&path).unwrap();
        let events = store.events(&EventFilter::default()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].cause, None,
            "the row written before the columns has none"
        );
        let version: i64 = store
            .connection
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn a_read_only_open_leaves_the_history_as_it_is() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("events.db");
        version_one_database(&path);
        {
            // A key of the version before methods existed, which a writable open rewrites.
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO events (target, previous_state, state, occurred_at_ms)
                     VALUES ('9.9.9.9', 'unknown', 'down', 3000);",
                )
                .unwrap();
        }

        let store = Store::open_ro(&path).unwrap();
        let keys = store
            .connection
            .prepare("SELECT target FROM events ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            keys,
            ["icmp://1.1.1.1", "9.9.9.9"],
            "a read-only open never rewrites a target key"
        );
        let version: i64 = store
            .connection
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1, "and it does not touch the schema either");

        // The first writable open is what converts the history.
        drop(store);
        Store::open_rw(&path).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        let keys: Vec<String> = connection
            .prepare("SELECT target FROM events ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(keys, ["icmp://1.1.1.1", "icmp://9.9.9.9"]);
    }

    #[test]
    fn a_read_only_open_still_refuses_a_database_from_the_future() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("events.db");
        {
            let store = Store::open_rw(&path).unwrap();
            store
                .connection
                .execute("UPDATE schema_meta SET version = 99", [])
                .unwrap();
        }

        assert!(matches!(
            Store::open_ro(&path),
            Err(StoreError::NewerSchema { found: 99, .. })
        ));
    }

    #[test]
    fn refuses_a_newer_schema() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("events.db");
        {
            let store = Store::open_rw(&path).unwrap();
            store
                .connection
                .execute("UPDATE schema_meta SET version = 99", [])
                .unwrap();
        }

        let opened = Store::open_ro(&path);
        assert!(matches!(
            opened,
            Err(StoreError::NewerSchema { found: 99, .. })
        ));
    }

    #[test]
    fn refuses_a_missing_or_uninitialized_database() {
        let directory = tempdir().unwrap();
        let missing = directory.path().join("missing.db");
        assert!(matches!(
            Store::open_ro(&missing),
            Err(StoreError::Missing { .. })
        ));

        let empty = directory.path().join("empty.db");
        std::fs::write(&empty, b"").unwrap();
        assert!(matches!(
            Store::open_ro(&empty),
            Err(StoreError::Uninitialized { .. })
        ));
    }
}
