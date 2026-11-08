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
