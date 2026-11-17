//! Command-line surface.
//!
//! Every configuration key has a flag here (INV-2); the merge itself lives in
//! [`crate::config::Config::resolve`].

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::config::{Overrides, TargetSpec};

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

    /// Target as [method:]address[,key=value]… — `1.1.1.1`, `tcp:1.1.1.1:443`,
    /// `dns:example.com@9.9.9.9`, `http:https://example.com/`; repeatable, replaces the
    /// file list
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

    /// Page compression on the wire: auto gzips the HTML when the client accepts it, off
    /// sends plain bytes for a proxy that compresses
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t = Compression::Auto,
        value_name = "MODE"
    )]
    pub compression: Compression,

    /// Print the effective configuration and exit
    #[arg(long, global = true)]
    pub show_config: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Probe the configured targets and record state transitions
    Monitor {
        /// Also serve the web page on this address and port
        #[arg(long, value_name = "ADDRESS:PORT")]
        listen: Option<String>,

        /// Start with this candidate configuration and promote it once it probes;
        /// an absent file is a quiet no-op, so the flag can stay in a start script
        #[arg(long = "try", value_name = "PATH")]
        candidate: Option<PathBuf>,

        /// Start from the active snapshot when the configuration cannot be used
        #[arg(long)]
        fallback_to_active: bool,
    },

    /// Verify a configuration without starting the monitor
    Check {
        /// Also verify this address and port can be bound
        #[arg(long, value_name = "ADDRESS:PORT")]
        listen: Option<String>,

        /// Perform one real probe per target
        #[arg(long)]
        probe: bool,
    },

    /// Create a minimal, runnable setup in a new folder; configuration flags are rejected
    Init {
        /// Folder to create; it must not exist or must be empty
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,
    },

    /// Print the full user guide as Markdown
    #[command(visible_alias = "man")]
    Manual,

    /// Serve the read-only web page over the event database, without probing
    Serve {
        /// Address and port to listen on
        #[arg(long, default_value = "127.0.0.1:8642", value_name = "ADDRESS:PORT")]
        listen: String,
    },

    /// Serve the page over a simulated history, without probing and without a database
    Simulate {
        /// Address and port to listen on
        #[arg(long, value_name = "ADDRESS:PORT")]
        listen: String,

        /// How much past to simulate, as `30d` or `30`, from 1 to 3660 days
        #[arg(long, value_name = "DAYS", default_value = "30d")]
        history: HistorySpan,

        /// Seed of the simulation: the same seed replays the same history
        #[arg(long, value_name = "SEED", default_value_t = 0)]
        seed: u64,

        /// Faults per day of a target that fails now and then: 0 for a quiet board, 20 for a
        /// board that flaps
        #[arg(long, value_name = "FAULTS", default_value = "3")]
        fault_rate: FaultRate,
    },

    /// Import a history exported from the web page
    Import {
        /// File path, http:// URL, or - for standard input
        #[arg(value_name = "PATH|URL|-", allow_hyphen_values = true)]
        source: String,

        /// Replace every event inside the window the archive covers; events before and
        /// after it are kept
        #[arg(long)]
        replace_window: bool,
    },

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

/// How far back a simulated history goes.
///
/// Written either with the unit it is measured in (`30d`) or as a bare number of days
/// (`30`), because that is how the rest of the tool states a duration and a reader should
/// not have to remember which command wants which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistorySpan(i64);

impl HistorySpan {
    /// The shortest and the longest past the simulation will generate.
    pub const MIN_DAYS: i64 = 1;
    pub const MAX_DAYS: i64 = 3_660;

    pub fn days(self) -> i64 {
        self.0
    }
}

impl std::str::FromStr for HistorySpan {
    type Err = String;

    fn from_str(value: &str) -> Result<HistorySpan, String> {
        let trimmed = value.trim();
        let number = trimmed.strip_suffix(['d', 'D']).unwrap_or(trimmed).trim();
        let days: i64 = number
            .parse()
            .map_err(|_| format!("'{value}' is not a number of days; write 30d or 30"))?;
        if !(HistorySpan::MIN_DAYS..=HistorySpan::MAX_DAYS).contains(&days) {
            return Err(format!(
                "'{value}' is out of range: the simulated history covers {} to {} days",
                HistorySpan::MIN_DAYS,
                HistorySpan::MAX_DAYS
            ));
        }
        Ok(HistorySpan(days))
    }
}

/// How many faults a simulated day holds.
///
/// The number is the faults a day of a target that *fails now and then* holds; a target drawn
/// as one that flaps gets about eight times as many, and one in five never fails at all. `0`
/// therefore means a board where nothing goes down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaultRate(f64);

impl FaultRate {
    pub const MIN: f64 = 0.0;
    /// A ceiling that keeps a request finite: a day of 1 000 faults still fits in it.
    pub const MAX: f64 = 1_000.0;

    pub fn per_day(self) -> f64 {
        self.0
    }
}

impl std::str::FromStr for FaultRate {
    type Err = String;

    fn from_str(value: &str) -> Result<FaultRate, String> {
        let trimmed = value.trim();
        let rate: f64 = trimmed
            .parse()
            .map_err(|_| format!("'{value}' is not a number of faults; write 3 or 2.5"))?;
        if !rate.is_finite() || !(FaultRate::MIN..=FaultRate::MAX).contains(&rate) {
            return Err(format!(
                "'{value}' is out of range: the fault rate is {} to {} faults per day",
                FaultRate::MIN,
                FaultRate::MAX
            ));
        }
        Ok(FaultRate(rate))
    }
}

/// How the page travels over HTTP.
///
/// `auto` follows the client: the page is gzipped when the request advertises gzip, which
/// every browser does, and sent as it is otherwise. `off` never compresses, for a
/// deployment whose proxy in front does the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Compression {
    Auto,
    Off,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Table,
    Json,
}

impl Cli {
    /// Whether the invocation carries a configuration flag that `init` and `manual` reject.
    pub fn uses_configuration(&self) -> bool {
        self.config.is_some()
            || self.show_config
            || self.interval_ms.is_some()
            || self.timeout_ms.is_some()
            || self.database.is_some()
            || self.retention_days.is_some()
            || !self.targets.is_empty()
    }

    /// Turn the flags into configuration overrides.
    pub fn overrides(&self) -> Result<Overrides, String> {
        let targets = if self.targets.is_empty() {
            None
        } else {
            let mut parsed = Vec::with_capacity(self.targets.len());
            for spec in &self.targets {
                parsed.push(
                    TargetSpec::parse_spec(spec)
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
    use std::str::FromStr;

    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["network-monitor"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).expect("arguments are valid")
    }

    #[test]
    fn parses_the_monitor_subcommand() {
        let cli = parse(&["monitor"]);
        match cli.command {
            Some(Command::Monitor { listen, .. }) => assert!(listen.is_none()),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn the_monitor_can_serve_the_web_page() {
        let cli = parse(&["monitor", "--listen", "0.0.0.0:8642"]);
        match cli.command {
            Some(Command::Monitor { listen, .. }) => {
                assert_eq!(listen.as_deref(), Some("0.0.0.0:8642"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn the_monitor_takes_a_candidate_configuration() {
        let cli = parse(&["monitor", "--config", "main.toml", "--try", "next.toml"]);
        match cli.command {
            Some(Command::Monitor { candidate, .. }) => {
                assert_eq!(candidate, Some(PathBuf::from("next.toml")));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_the_check_subcommand() {
        let cli = parse(&["check", "--listen", "0.0.0.0:8642", "--probe"]);
        match cli.command {
            Some(Command::Check { listen, probe }) => {
                assert_eq!(listen.as_deref(), Some("0.0.0.0:8642"));
                assert!(probe);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = parse(&["check"]);
        match cli.command {
            Some(Command::Check { listen, probe }) => {
                assert!(listen.is_none());
                assert!(!probe);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_the_init_subcommand() {
        let cli = parse(&["init", "monitor"]);
        match cli.command {
            Some(Command::Init { folder }) => assert_eq!(folder, PathBuf::from("monitor")),
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(
            Cli::try_parse_from(["network-monitor", "init"]).is_err(),
            "the folder is required"
        );
    }

    #[test]
    fn parses_the_manual_subcommand_and_its_man_alias() {
        for name in ["manual", "man"] {
            let cli = parse(&[name]);
            assert!(matches!(cli.command, Some(Command::Manual)));
        }
        assert!(
            Cli::try_parse_from(["network-monitor", "manual", "extra"]).is_err(),
            "the command takes no arguments"
        );
    }

    #[test]
    fn the_help_lists_init_manual_and_the_man_alias() {
        use clap::CommandFactory;

        let mut command = Cli::command();
        let help = command.render_help().to_string();
        assert!(help.contains("init"), "help: {help}");
        assert!(help.contains("manual"), "help: {help}");
        assert!(help.contains("man"), "help: {help}");
    }

    #[test]
    fn uses_configuration_names_every_configuration_flag() {
        assert!(!parse(&["manual"]).uses_configuration());
        for args in [
            vec!["--config", "x.toml", "manual"],
            vec!["--show-config", "manual"],
            vec!["--interval-ms", "1", "manual"],
            vec!["--timeout-ms", "1", "manual"],
            vec!["--database", "x.db", "manual"],
            vec!["--retention-days", "1", "manual"],
            vec!["--target", "1.1.1.1", "manual"],
        ] {
            assert!(parse(&args).uses_configuration(), "args: {args:?}");
        }
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
    fn parses_the_simulate_subcommand_and_its_defaults() {
        let cli = parse(&["simulate", "--listen", "127.0.0.1:8660"]);
        match cli.command {
            Some(Command::Simulate {
                listen,
                history,
                seed,
                fault_rate,
            }) => {
                assert_eq!(listen, "127.0.0.1:8660");
                assert_eq!(fault_rate.per_day(), 3.0);
                assert_eq!(history.days(), 30);
                assert_eq!(seed, 0);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn the_simulate_history_takes_days_with_or_without_its_unit() {
        for (value, days) in [("7d", 7), ("7", 7), ("1", 1), ("3660d", 3_660)] {
            let cli = parse(&["simulate", "--listen", ":0", "--history", value]);
            match cli.command {
                Some(Command::Simulate { history, .. }) => assert_eq!(history.days(), days),
                other => panic!("unexpected command: {other:?}"),
            }
        }
        assert!(HistorySpan::from_str("0").is_err(), "a day at least");
        assert!(HistorySpan::from_str("4000d").is_err(), "and 3660 at most");
        assert!(
            HistorySpan::from_str("30x").is_err(),
            "a unit is d or nothing"
        );
        assert!(HistorySpan::from_str("-3").is_err(), "not a count");
    }

    #[test]
    fn the_simulate_fault_rate_defaults_to_three_and_takes_a_positive_number() {
        let cli = parse(&["simulate", "--listen", ":0"]);
        match cli.command {
            Some(Command::Simulate { fault_rate, .. }) => {
                assert_eq!(fault_rate.per_day(), 3.0);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        for (value, rate) in [("0", 0.0), ("2.5", 2.5), ("20", 20.0)] {
            let cli = parse(&["simulate", "--listen", ":0", "--fault-rate", value]);
            match cli.command {
                Some(Command::Simulate { fault_rate, .. }) => {
                    assert_eq!(fault_rate.per_day(), rate);
                }
                other => panic!("unexpected command: {other:?}"),
            }
        }
        assert!(FaultRate::from_str("-1").is_err(), "no negative rate");
        assert!(FaultRate::from_str("1001").is_err(), "no runaway rate");
        assert!(FaultRate::from_str("NaN").is_err(), "a rate is a number");
        assert!(FaultRate::from_str("lots").is_err(), "a rate is a number");
    }

    #[test]
    fn simulate_requires_a_listen_address_and_a_numeric_seed() {
        assert!(
            Cli::try_parse_from(["network-monitor", "simulate"]).is_err(),
            "--listen is required"
        );
        let cli = parse(&["simulate", "--listen", ":0", "--seed", "42"]);
        match cli.command {
            Some(Command::Simulate { seed, .. }) => assert_eq!(seed, 42),
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(Cli::try_parse_from([
            "network-monitor",
            "simulate",
            "--listen",
            ":0",
            "--seed",
            "x"
        ])
        .is_err());
    }

    #[test]
    fn rejects_an_unknown_log_level() {
        let result = Cli::try_parse_from(["network-monitor", "--log-level", "loud"]);
        assert!(result.is_err());
    }
}
