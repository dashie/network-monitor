//! Command-line surface.
//!
//! Every configuration key has a flag here (INV-2); the merge itself lives in
//! [`crate::config::Config::resolve`].

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::config::{Overrides, Target};

/// Event-based ICMP reachability monitor.
#[derive(Debug, Parser)]
#[command(
    name = "network-monitor",
    version,
    about = "Probe IP addresses with ICMP and record only state transitions",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Configuration file (default: ./network-monitor.toml when it exists)
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Probe interval in milliseconds
    #[arg(long, global = true, value_name = "MILLISECONDS")]
    pub interval_ms: Option<u64>,

    /// Probe timeout in milliseconds
    #[arg(long, global = true, value_name = "MILLISECONDS")]
    pub timeout_ms: Option<u64>,

    /// Event database file
    #[arg(long, global = true, value_name = "PATH")]
    pub database: Option<PathBuf>,

    /// Delete events older than this many days (0 keeps every event)
    #[arg(long, global = true, value_name = "DAYS")]
    pub retention_days: Option<u64>,

    /// Target as IP[,interface=NAME][,label=NAME]; repeatable, replaces the file list
    #[arg(long = "target", global = true, value_name = "SPEC")]
    pub targets: Vec<String>,

    /// Diagnostic level: error, warn, info, debug
    #[arg(
        long,
        global = true,
        value_name = "LEVEL",
        value_parser = ["error", "warn", "info", "debug"]
    )]
    pub log_level: Option<String>,

    /// Print the effective configuration and exit
    #[arg(long, global = true)]
    pub show_config: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Probe the configured targets and record state transitions
    Monitor,

    /// Read the recorded state and history
    Query {
        #[command(subcommand)]
        what: QueryCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum QueryCommand {
    /// Current state of every target
    State {
        /// Output format
        #[arg(long, value_enum, default_value_t = Format::Table)]
        format: Format,
    },

    /// Recorded transitions, newest first
    Events {
        /// Restrict to one target address or label
        #[arg(value_name = "ADDRESS|LABEL")]
        target: Option<String>,

        /// Only events at or after this moment: 30s, 15m, 24h, 7d, or epoch milliseconds
        #[arg(long, value_name = "DURATION|EPOCH_MS")]
        since: Option<String>,

        /// Maximum number of rows
        #[arg(long, default_value_t = 100, value_name = "ROWS")]
        limit: u32,

        /// Output format
        #[arg(long, value_enum, default_value_t = Format::Table)]
        format: Format,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Table,
    Json,
}

impl Cli {
    /// Turn the flags into configuration overrides.
    pub fn overrides(&self) -> Result<Overrides, String> {
        let targets = if self.targets.is_empty() {
            None
        } else {
            let mut parsed = Vec::with_capacity(self.targets.len());
            for spec in &self.targets {
                parsed.push(
                    Target::parse_spec(spec)
                        .map_err(|reason| format!("invalid --target '{spec}': {reason}"))?,
                );
            }
            Some(parsed)
        };

        Ok(Overrides {
            interval_ms: self.interval_ms,
            timeout_ms: self.timeout_ms,
            database: self.database.clone(),
            retention_days: self.retention_days,
            targets,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["network-monitor"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).expect("arguments are valid")
    }

    #[test]
    fn parses_the_monitor_subcommand() {
        let cli = parse(&["monitor"]);
        assert!(matches!(cli.command, Some(Command::Monitor)));
    }

    #[test]
    fn parses_query_events() {
        let cli = parse(&["query", "events", "--since", "24h", "--format", "json"]);
        match cli.command {
            Some(Command::Query {
                what:
                    QueryCommand::Events {
                        since,
                        format,
                        limit,
                        ..
                    },
            }) => {
                assert_eq!(since.as_deref(), Some("24h"));
                assert_eq!(format, Format::Json);
                assert_eq!(limit, 100);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn turns_flags_into_overrides() {
        let cli = parse(&[
            "--interval-ms",
            "1000",
            "--timeout-ms",
            "500",
            "--database",
            "/tmp/x.db",
            "--retention-days",
            "7",
            "--target",
            "1.1.1.1,interface=eth1",
        ]);
        let overrides = cli.overrides().unwrap();
        assert_eq!(overrides.interval_ms, Some(1_000));
        assert_eq!(overrides.timeout_ms, Some(500));
        assert_eq!(overrides.database, Some(PathBuf::from("/tmp/x.db")));
        assert_eq!(overrides.retention_days, Some(7));
        let targets = overrides.targets.unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].interface.as_deref(), Some("eth1"));
    }

    #[test]
    fn reports_a_bad_target_spec() {
        let cli = parse(&["--target", "not-an-address"]);
        assert!(cli.overrides().is_err());
    }

    #[test]
    fn rejects_an_unknown_log_level() {
        let result = Cli::try_parse_from(["network-monitor", "--log-level", "loud"]);
        assert!(result.is_err());
    }
}
