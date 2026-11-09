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
}

/// One disconnection: the moment a target went down and, if it came back, the moment
/// it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownEpisode {
    /// Id of the `down` event, used to page through the history.
    pub event_id: i64,
    pub target: String,
    pub started_at_ms: i64,
    /// `None` while the target is still down.
    pub ended_at_ms: Option<i64>,
}

const DAY_MS: i64 = 86_400_000;

/// The columns of one disconnection: event id, target, start, end. The end is `NULL`
/// while the target is still down. Shared by the page and the export.
const DOWNS_SELECT: &str = "
SELECT e.id, e.target, e.occurred_at_ms,
       (SELECT MIN(n.occurred_at_ms) FROM events n
         WHERE n.target = e.target AND n.occurred_at_ms > e.occurred_at_ms)
FROM events e
WHERE e.state = 'down'";

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
        let mut store = Store { connection };
        store.connection.execute_batch(PRAGMAS)?;
        store.migrate()?;
        Ok(store)
    }

    /// Open the database read-only. The file must already exist and carry a schema.
    pub fn open_ro(path: &Path) -> Result<Store, StoreError> {
        if !path.exists() {
            return Err(StoreError::Missing {
                path: path.to_path_buf(),
            });
        }

        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        connection.busy_timeout(Duration::from_secs(5))?;

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

        Ok(Store { connection })
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
            Some(_) => {
                // Version 1 has no migration steps yet; keep the table creation
                // idempotent so a partially created database heals on open.
                self.connection.execute_batch(TABLES)?;
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
                (target, interface, previous_state, state, occurred_at_ms, rtt_us)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.target,
                event.interface,
                event.previous_state.as_str(),
                event.state.as_str(),
                event.occurred_at_ms,
                event.rtt_us,
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
                    (target, interface, previous_state, state, occurred_at_ms, rtt_us)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6
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

    /// Delete every event of every target inside `[from_ms, to_ms]`, inclusive.
    pub fn delete_events_between(&self, from_ms: i64, to_ms: i64) -> Result<usize, StoreError> {
        let deleted = self.connection.execute(
            "DELETE FROM events WHERE occurred_at_ms BETWEEN ?1 AND ?2",
            params![from_ms, to_ms],
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

    /// Recorded transitions, newest first.
    pub fn events(&self, filter: &EventFilter) -> Result<Vec<EventRow>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, target, interface, previous_state, state, occurred_at_ms, rtt_us
             FROM events
             WHERE (?1 IS NULL OR target = ?1)
               AND (?2 IS NULL OR occurred_at_ms >= ?2)
             ORDER BY occurred_at_ms DESC, id DESC
             LIMIT COALESCE(?3, -1)",
        )?;
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
                })
            },
        )?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Resolve a target address or label to the target address it identifies.
    pub fn resolve_target(&self, needle: &str) -> Result<Option<String>, StoreError> {
        let target = self
            .connection
            .query_row(
                "SELECT target FROM targets WHERE target = ?1 OR label = ?1 LIMIT 1",
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
                 WHERE target = ?1 OR label = ?1
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

    /// Number of transitions to `down` per UTC day, one entry per day in the window.
    pub fn daily_drops(
        &self,
        target: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<DailyDrops>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT occurred_at_ms FROM events
             WHERE target = ?1 AND state = 'down'
               AND occurred_at_ms >= ?2 AND occurred_at_ms < ?3",
        )?;

        let mut buckets: HashMap<i64, u32> = HashMap::new();
        for row in
            statement.query_map(params![target, from_ms, to_ms], |row| row.get::<_, i64>(0))?
        {
            let occurred_at_ms = row?;
            let day = occurred_at_ms.div_euclid(DAY_MS) * DAY_MS;
            *buckets.entry(day).or_insert(0) += 1;
        }

        let mut days = Vec::new();
        let mut day = from_ms.div_euclid(DAY_MS) * DAY_MS;
        while day < to_ms {
            days.push(DailyDrops {
                day_start_ms: day,
                drops: buckets.get(&day).copied().unwrap_or(0),
            });
            day += DAY_MS;
        }
        Ok(days)
    }

    /// One page of the whole history in insertion order, for a streamed export.
    ///
    /// Paging keeps every read transaction short, so a slow download never holds the
    /// WAL checkpoints of the writer.
    pub fn events_after(
        &self,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<ExportedEvent>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT e.id, e.target, t.label, e.interface, e.state, e.occurred_at_ms, e.rtt_us
               FROM events e
               LEFT JOIN targets t ON t.target = e.target
              WHERE e.id > ?1
              ORDER BY e.id
              LIMIT ?2",
        )?;

        let rows = statement
            .query_map(params![after_id, limit], |row| {
                let state: String = row.get(4)?;
                Ok(ExportedEvent {
                    id: row.get(0)?,
                    target: row.get(1)?,
                    label: row.get(2)?,
                    interface: row.get(3)?,
                    state: TargetState::parse(&state).unwrap_or(TargetState::Unknown),
                    occurred_at_ms: row.get(5)?,
                    rtt_us: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The most recent disconnections of every target, newest first.
    pub fn recent_downs(&self, limit: u32) -> Result<Vec<DownEpisode>, StoreError> {
        let query = format!("{DOWNS_SELECT} ORDER BY e.occurred_at_ms DESC, e.id DESC LIMIT ?1");
        let mut statement = self.connection.prepare(&query)?;

        let rows = statement
            .query_map(params![limit], |row| {
                Ok(DownEpisode {
                    event_id: row.get(0)?,
                    target: row.get(1)?,
                    started_at_ms: row.get(2)?,
                    ended_at_ms: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// One page of disconnections in chronological order, for a streamed export.
    ///
    /// Paging keeps every read transaction short, so a slow download never holds the
    /// WAL checkpoints of the writer.
    pub fn downs_after(&self, after_id: i64, limit: u32) -> Result<Vec<DownEpisode>, StoreError> {
        let query = format!("{DOWNS_SELECT} AND e.id > ?1 ORDER BY e.id LIMIT ?2");
        let mut statement = self.connection.prepare(&query)?;

        let rows = statement
            .query_map(params![after_id, limit], |row| {
                Ok(DownEpisode {
                    event_id: row.get(0)?,
                    target: row.get(1)?,
                    started_at_ms: row.get(2)?,
                    ended_at_ms: row.get(3)?,
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
    }

    #[test]
    fn recent_downs_are_newest_first_and_keep_the_open_one() {
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

        let downs = store.recent_downs(10).unwrap();
        assert_eq!(downs.len(), 2);
        assert_eq!(downs[0].started_at_ms, 9_000);
        assert_eq!(downs[0].ended_at_ms, None, "the target is still down");
        assert_eq!(downs[1].started_at_ms, 2_000);
        assert_eq!(downs[1].ended_at_ms, Some(5_000));

        assert_eq!(store.recent_downs(1).unwrap().len(), 1);
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
