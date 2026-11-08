//! The `query` subcommand: read the database without touching the monitor.

pub mod render;

use thiserror::Error;

use crate::cli::QueryCommand;
use crate::config::Config;
use crate::store::{EventFilter, Store, StoreError};
use crate::time;

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("{0}")]
    Store(#[from] StoreError),
    #[error("invalid --since value '{value}': {reason}")]
    Since { value: String, reason: String },
}

pub fn run(config: &Config, what: &QueryCommand) -> Result<(), QueryError> {
    let store = Store::open_ro(&config.database)?;

    match what {
        QueryCommand::State { format } => {
            let rows = store.states()?;
            println!("{}", render::states(&rows, *format));
        }
        QueryCommand::Events {
            target,
            since,
            limit,
            format,
        } => {
            let since_ms = match since {
                Some(value) => {
                    Some(time::parse_since(value, time::now_ms()).map_err(|reason| {
                        QueryError::Since {
                            value: value.clone(),
                            reason,
                        }
                    })?)
                }
                None => None,
            };

            // Accept a label as well as an address, so `--target cloudflare-dns` works.
            let resolved = match target {
                Some(needle) => store.resolve_target(needle)?.or(Some(needle.clone())),
                None => None,
            };

            let filter = EventFilter {
                target: resolved,
                since_ms,
                limit: Some(*limit),
            };
            let rows = store.events(&filter)?;
            println!("{}", render::events(&rows, *format));
        }
    }

    Ok(())
}
