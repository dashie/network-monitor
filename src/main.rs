//! Entry point: parse the command line, resolve the configuration, dispatch.

use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use clap::Parser;
use thiserror::Error;

use network_monitor::cli::{Cli, Command, Compression};
use network_monitor::config::{self, ChannelSettings, Config, ConfigError};
use network_monitor::history::HistorySource;
use network_monitor::import::{self, ImportError};
use network_monitor::logging::{self, Level};
use network_monitor::monitor::{self, RunError};
use network_monitor::probe::interface;
use network_monitor::probe::{Probe, Prober};
use network_monitor::query::{self, QueryError};
use network_monitor::shutdown;
use network_monitor::sim::history::SimulatedSource;
use network_monitor::store::{DatabaseSource, Store, StoreError};
use network_monitor::time::format_rfc3339;
use network_monitor::web::{self, ServeError};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, Error)]
enum AppError {
    #[error("{0}")]
    Config(#[from] ConfigError),
    #[error("{0}")]
    Store(#[from] StoreError),
    #[error("{0}")]
    Query(#[from] QueryError),
    #[error("{0}")]
    Monitor(#[from] RunError),
    #[error("{0}")]
    Serve(#[from] ServeError),
    #[error("{0}")]
    Import(#[from] ImportError),
    #[error("{0}")]
    Invalid(String),
}

fn run() -> Result<ExitCode, AppError> {
    let cli = Cli::parse();

    if let Some(name) = cli.log_level.as_deref() {
        let level = Level::parse(name)
            .ok_or_else(|| AppError::Invalid(format!("unknown log level '{name}'")))?;
        logging::set_level(level);
    }

    let overrides = cli.overrides().map_err(AppError::Invalid)?;
    let loaded = config::load(cli.config.as_deref())?;
    let source = loaded.as_ref().map(|loaded| loaded.path.clone());
    let reload = loaded.as_ref().map(|loaded| monitor::Reload {
        path: loaded.path.clone(),
        overrides: overrides.clone(),
    });
    let settings = Config::resolve(loaded.map(|loaded| loaded.config), overrides)?;
    let compression = cli.compression;

    if cli.show_config {
        print!("{}", settings.to_toml());
        return Ok(ExitCode::SUCCESS);
    }

    match cli.command {
        Some(Command::Monitor { listen }) => {
            log_initializing(source.as_deref());
            settings.require_targets()?;
            validate_interfaces(&settings)?;
            log_notifications(&settings);
            log_targets(&settings);
            run_monitor(&settings, source.as_deref(), reload, listen, compression)?;
        }
        Some(Command::Serve { listen }) => {
            log_initializing(source.as_deref());
            let stop = shutdown::Shutdown::new();
            shutdown::install();
            let source: Arc<dyn HistorySource> =
                Arc::new(DatabaseSource::new(settings.database.clone()));
            web::run(source, &listen, &stop, compression)?;
        }
        Some(Command::Simulate {
            listen,
            history,
            seed,
            fault_rate,
        }) => {
            log_initializing(source.as_deref());
            settings.require_targets()?;
            log_simulation(&settings, seed, history.days(), fault_rate.per_day());
            let stop = shutdown::Shutdown::new();
            shutdown::install();
            let simulated: Arc<dyn HistorySource> = Arc::new(SimulatedSource::new(
                settings.targets.clone(),
                seed,
                history.days(),
                fault_rate.per_day(),
            ));
            web::run(simulated, &listen, &stop, compression)?;
        }
        Some(Command::Import {
            source,
            replace_window,
        }) => {
            let report = import::run(
                &settings.database,
                &source,
                import::Options { replace_window },
            )?;
            let dropped = if report.dropped > 0 {
                format!(
                    ", {} events dropped as repeats of the state before them",
                    report.dropped
                )
            } else {
                String::new()
            };
            if let Some((from, to)) = report.window {
                println!(
                    "replaced {} events inside the window of each target ({} to {}), then \
                     imported {} events for {} target(s) from {} ({} rows skipped{dropped}, {} \
                     format)",
                    report.deleted,
                    format_rfc3339(from),
                    format_rfc3339(to),
                    report.events,
                    report.targets,
                    report.source,
                    report.skipped,
                    if report.gzipped { "gzip" } else { "plain csv" }
                );
            } else {
                println!(
                    "imported {} events for {} target(s) from {} ({} rows skipped{dropped}, {} \
                     format)",
                    report.events,
                    report.targets,
                    report.source,
                    report.skipped,
                    if report.gzipped { "gzip" } else { "plain csv" }
                );
            }
        }
        Some(Command::Query { what }) => {
            query::run(&settings, &what)?;
        }
        None => {
            println!("error: a subcommand is required (monitor, query); try --help\n");
            return Ok(ExitCode::from(2));
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// The opening `INFO` lines: the process states that it starts and names what it read.
fn log_initializing(source: Option<&Path>) {
    logging::info(None, "initializing");
    match source {
        Some(path) => logging::info(
            None,
            &format!("reading configuration from {}", path.display()),
        ),
        None => logging::info(
            None,
            "no configuration file; using the command line and the built-in defaults",
        ),
    }
}

fn log_notifications(settings: &Config) {
    for channel in &settings.notify.channels {
        let destination = match channel {
            ChannelSettings::LogFile(logfile) => format!(
                "{} (logger {}, level {}, format {})",
                logfile.filename.display(),
                logfile.logger,
                logfile.level.label().to_ascii_lowercase(),
                logfile.format.as_str()
            ),
            other => other.destination().to_string(),
        };
        logging::info(
            None,
            &format!(
                "notification channel {} ({}) configured for {}",
                channel.name(),
                channel.kind().as_str(),
                destination
            ),
        );
    }
    if settings.notify.global.is_empty() {
        logging::info(None, "notification defaults: none");
    } else {
        logging::info(
            None,
            &format!(
                "notification defaults: {}",
                settings.notify.global.join(", ")
            ),
        );
    }
}

/// The one line a simulated run states about itself: what it shows, from when, and that
/// nothing is probed and nothing is stored.
fn log_simulation(settings: &Config, seed: u64, days: i64, fault_rate: f64) {
    logging::info(
        None,
        &format!(
            "simulating {} target(s) over the last {} day(s), seed {}, {} fault(s) per day: no probe runs and no database is read or written",
            settings.targets.len(),
            days,
            seed,
            fault_rate
        ),
    );
    for target in &settings.targets {
        logging::info(None, &format!("simulated target {}", target.key()));
    }
}

fn log_targets(settings: &Config) {
    for target in &settings.targets {
        logging::info(None, &format!("initialized target {}", target.key()));
    }
}

fn validate_interfaces(settings: &Config) -> Result<(), AppError> {
    for target in &settings.targets {
        if let Some(name) = &target.interface {
            interface::validate(name).map_err(|reason| {
                AppError::Invalid(format!("interface '{name}' for {}: {reason}", target.key()))
            })?;
        }
    }
    Ok(())
}

fn run_monitor(
    settings: &Config,
    source: Option<&Path>,
    reload: Option<monitor::Reload>,
    listen: Option<String>,
    compression: Compression,
) -> Result<(), AppError> {
    let prober: Arc<dyn Probe> = Arc::new(Prober::new());
    let store = Arc::new(Mutex::new(Store::open_rw(&settings.database)?));
    let stop = shutdown::Shutdown::new();

    shutdown::install();

    let mut serving = None;
    // Bind before the first probe: an unusable address must fail at once, not after
    // the monitor has been running for a while.
    let web_thread = match listen {
        Some(address) => {
            let listener = web::bind(&address)?;
            serving = Some(
                listener
                    .local_addr()
                    .map(|bound| bound.to_string())
                    .unwrap_or(address),
            );
            let source: Arc<dyn HistorySource> =
                Arc::new(DatabaseSource::new(settings.database.clone()));
            let web_stop = stop.clone();
            Some(std::thread::spawn(move || {
                if let Err(error) = web::serve(listener, source, &web_stop, compression) {
                    logging::error(None, &format!("web interface failed: {error}"));
                }
            }))
        }
        None => None,
    };

    let startup = monitor::startup_event(settings, source, serving.as_deref());
    let outcome = monitor::run(settings, prober, store, &stop, reload, Some(startup));

    if let Some(thread) = web_thread {
        stop.request();
        let _ = thread.join();
    }

    outcome?;
    Ok(())
}
