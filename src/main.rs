//! Entry point: parse the command line, resolve the configuration, dispatch.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use clap::Parser;
use thiserror::Error;

use network_monitor::check;
use network_monitor::cli::{Cli, Command, Compression};
use network_monitor::config::{self, ChannelSettings, Config, ConfigError, Overrides};
use network_monitor::history::HistorySource;
use network_monitor::import::{self, ImportError};
use network_monitor::init;
use network_monitor::logging::{self, Level};
use network_monitor::manual;
use network_monitor::monitor::{self, RunError};
use network_monitor::probe::interface;
use network_monitor::probe::{Probe, Prober};
use network_monitor::promote::Candidate;
use network_monitor::query::{self, QueryError};
use network_monitor::shutdown;
use network_monitor::sim::history::SimulatedSource;
use network_monitor::snapshot;
use network_monitor::store::{DatabaseSource, Store, StoreError};
use network_monitor::time::{format_rfc3339, now_ms};
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
    Init(#[from] init::InitError),
    #[error("{0}")]
    Manual(#[from] manual::ManualError),
    #[error("{0}")]
    Invalid(String),
}

impl AppError {
    /// Name the active snapshot and its age after a start that stops.
    fn with_recovery_hint(self, config: &Path) -> AppError {
        match snapshot::describe(&snapshot::path_for(config)) {
            Some(description) => AppError::Invalid(format!(
                "{self}; the last configuration that worked is {description}"
            )),
            None => self,
        }
    }
}

fn run() -> Result<ExitCode, AppError> {
    let cli = Cli::parse();

    if let Some(name) = cli.log_level.as_deref() {
        let level = Level::parse(name)
            .ok_or_else(|| AppError::Invalid(format!("unknown log level '{name}'")))?;
        logging::set_level(level);
    }

    if let Some(Command::Init { folder }) = &cli.command {
        if cli.uses_configuration() {
            return Err(AppError::Invalid(
                "init takes only a folder; it does not read a configuration".to_string(),
            ));
        }
        let report = init::run(folder)?;
        print!("{}", report.render());
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(Command::Manual) = &cli.command {
        if cli.uses_configuration() {
            return Err(AppError::Invalid(
                "manual takes no arguments; it does not read a configuration".to_string(),
            ));
        }
        manual::run()?;
        return Ok(ExitCode::SUCCESS);
    }

    let overrides = cli.overrides().map_err(AppError::Invalid)?;
    let compression = cli.compression;

    if cli.show_config {
        let (_, settings) = load_settings(cli.config.as_deref(), &overrides)?;
        print!("{}", settings.to_toml());
        return Ok(ExitCode::SUCCESS);
    }

    match cli.command {
        Some(Command::Monitor {
            listen,
            candidate,
            fallback_to_active,
        }) => {
            if candidate.is_some() && cli.config.is_none() {
                return Err(AppError::Invalid(
                    "--try requires --config <PATH>, the file a promotion replaces".to_string(),
                ));
            }
            let hint = main_config_path(cli.config.as_deref());
            monitor_command(
                cli.config.as_deref(),
                &overrides,
                listen,
                candidate.as_deref(),
                fallback_to_active,
                compression,
            )
            .map_err(|error| error.with_recovery_hint(&hint))?;
        }
        other => {
            let (source, settings) = load_settings(cli.config.as_deref(), &overrides)?;
            match other {
                Some(Command::Check { listen, probe }) => {
                    let report = check::run(
                        &settings,
                        check::Options {
                            listen,
                            probe,
                            active: source.as_deref().map(snapshot::path_for),
                        },
                    );
                    print!("{}", report.render());
                    if report.failed() {
                        return Ok(ExitCode::FAILURE);
                    }
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
                Some(Command::Monitor { .. }) => unreachable!("the monitor is handled above"),
                Some(Command::Init { .. }) | Some(Command::Manual) => {
                    unreachable!("init and manual are handled before the configuration is loaded")
                }
                None => {
                    println!("error: a subcommand is required; run --help for the list\n");
                    return Ok(ExitCode::from(2));
                }
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Resolve the configuration file and its command-line overrides.
fn load_settings(
    explicit: Option<&Path>,
    overrides: &Overrides,
) -> Result<(Option<PathBuf>, Config), AppError> {
    let loaded = config::load(explicit)?;
    let source = loaded.as_ref().map(|loaded| loaded.path.clone());
    let settings = Config::resolve(loaded.map(|loaded| loaded.config), overrides.clone())?;
    Ok((source, settings))
}

/// The configuration a start begins from, and the snapshot it should publish.
struct StartConfig {
    source: Option<PathBuf>,
    config: Config,
    snapshot: Option<snapshot::Snapshot>,
}

/// Resolve the start configuration, falling back to the active snapshot when asked.
///
/// The flag covers every way the main file can be unusable, including the checks a
/// plain start runs: the target list, the interfaces, the database, and the channels.
fn load_start(
    explicit: Option<&Path>,
    overrides: &Overrides,
    fallback_to_active: bool,
) -> Result<StartConfig, AppError> {
    match load_settings(explicit, overrides) {
        Ok((source, config)) => {
            if fallback_to_active {
                if let Some(failure) = preflight_failure(&config) {
                    if let Some(start) = snapshot_start(explicit, overrides, &failure) {
                        return Ok(start);
                    }
                    return Err(AppError::Invalid(failure));
                }
            }
            let snapshot = source.as_deref().and_then(snapshot_plan_for);
            Ok(StartConfig {
                source,
                config,
                snapshot,
            })
        }
        Err(error) if fallback_to_active => {
            match snapshot_start(explicit, overrides, &error.to_string()) {
                Some(start) => Ok(start),
                None => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

/// The first check that would stop a start of this configuration, without `listen` or
/// `probe`: exactly the pipeline the candidate decision uses.
fn preflight_failure(config: &Config) -> Option<String> {
    let report = check::run(config, check::Options::default());
    report
        .lines
        .iter()
        .find(|line| line.level == check::LineLevel::Fail)
        .map(|line| format!("{}: {}", line.subject, line.detail))
}

/// Start from the active snapshot, when it exists and passes the same checks.
fn snapshot_start(
    explicit: Option<&Path>,
    overrides: &Overrides,
    failure: &str,
) -> Option<StartConfig> {
    let main = main_config_path(explicit);
    let snapshot_path = snapshot::path_for(&main);
    let config = load_snapshot(&snapshot_path, overrides)?;
    let described =
        snapshot::describe(&snapshot_path).unwrap_or_else(|| snapshot_path.display().to_string());
    logging::warn(
        None,
        &format!(
            "cannot use {}: {failure}; starting from the active snapshot {described}",
            main.display()
        ),
    );
    let snapshot = snapshot::Snapshot {
        path: snapshot_path.clone(),
        bytes: std::fs::read(&snapshot_path).unwrap_or_default(),
    };
    Some(StartConfig {
        source: Some(main),
        config,
        snapshot: Some(snapshot),
    })
}

/// The bytes of the configuration that started, and where its snapshot lives.
fn snapshot_plan_for(path: &Path) -> Option<snapshot::Snapshot> {
    let bytes = std::fs::read(path).ok()?;
    Some(snapshot::Snapshot {
        path: snapshot::path_for(path),
        bytes,
    })
}

/// Read the snapshot as configuration through the same parse/resolve/check pipeline.
fn load_snapshot(path: &Path, overrides: &Overrides) -> Option<Config> {
    let text = std::fs::read_to_string(path).ok()?;
    let file = config::parse(&text, path).ok()?;
    let config = Config::resolve(Some(file), overrides.clone()).ok()?;
    let report = check::run(
        &config,
        check::Options {
            active: Some(path.to_path_buf()),
            ..Default::default()
        },
    );
    if report.failed() {
        None
    } else {
        Some(config)
    }
}

/// The main configuration path: the explicit file or the default name.
///
/// The default is returned even when the file is absent, so its snapshot is still found
/// and a failed start can name it.
fn main_config_path(explicit: Option<&Path>) -> PathBuf {
    explicit
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(config::DEFAULT_CONFIG_FILE))
}

/// Run the monitor from the configuration the candidate table and the snapshot choose.
fn monitor_command(
    explicit: Option<&Path>,
    overrides: &Overrides,
    listen: Option<String>,
    candidate_path: Option<&Path>,
    fallback_to_active: bool,
    compression: Compression,
) -> Result<(), AppError> {
    let StartConfig {
        source,
        config: main,
        snapshot: start_snapshot,
    } = load_start(explicit, overrides, fallback_to_active)?;
    log_initializing(source.as_deref());

    let (effective, promotion) =
        choose_configuration(&main, source.as_deref(), candidate_path, overrides)?;
    let fallback = promotion.as_ref().map(|_| main.clone());
    let snapshot = match (&promotion, source.as_deref()) {
        (Some(candidate), Some(path)) => Some(snapshot::Snapshot {
            path: snapshot::path_for(path),
            bytes: candidate.bytes.clone(),
        }),
        _ => start_snapshot,
    };

    effective.require_targets()?;
    validate_interfaces(&effective)?;
    log_notifications(&effective);
    warn_unused_channels(&effective);
    log_targets(&effective);
    let reload = source.as_ref().map(|path| monitor::Reload {
        path: path.clone(),
        overrides: overrides.clone(),
    });
    run_monitor(
        &effective,
        source.as_deref(),
        reload,
        listen,
        compression,
        monitor::Start {
            startup: None,
            promotion,
            fallback,
            snapshot,
        },
    )
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

/// State every declared channel that no `notify` list enables.
///
/// An unused channel is not an error: it starts and receives nothing, so the operator
/// is warned and the monitor runs.
fn warn_unused_channels(settings: &Config) {
    let mut enabled: HashSet<&str> = settings.notify.global.iter().map(String::as_str).collect();
    for names in settings.notify.by_target.values() {
        enabled.extend(names.iter().map(String::as_str));
    }
    for channel in &settings.notify.channels {
        if !enabled.contains(channel.name()) {
            logging::notification(
                Level::Warn,
                None,
                &format!(
                    "notification channel {} ({}) is declared but nothing enables it; it receives no event",
                    channel.name(),
                    channel.kind().as_str()
                ),
            );
        }
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

/// Pick the configuration the monitor runs with and the candidate it may promote.
///
/// The candidate table is deliberately quiet on an absent file: `--try` can stay in a
/// start script forever. A candidate that exists but cannot be used is one `ERROR` and
/// one `WARN`, and the monitor runs the main configuration exactly as a plain start.
fn choose_configuration(
    main: &Config,
    main_path: Option<&Path>,
    candidate_path: Option<&Path>,
    overrides: &Overrides,
) -> Result<(Config, Option<Candidate>), AppError> {
    let Some(candidate_path) = candidate_path else {
        return Ok((main.clone(), None));
    };
    let main_path = main_path.ok_or_else(|| {
        AppError::Invalid(
            "--try requires --config <PATH>, the file a promotion replaces".to_string(),
        )
    })?;
    if same_file(main_path, candidate_path) {
        return Err(AppError::Invalid(format!(
            "--try must name a different file from --config ({})",
            main_path.display()
        )));
    }

    let bytes = match std::fs::read(candidate_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            logging::info(
                None,
                &format!(
                    "no candidate configuration at {}; using {}",
                    candidate_path.display(),
                    main_path.display()
                ),
            );
            return Ok((main.clone(), None));
        }
        Err(error) => {
            logging::error(
                None,
                &format!(
                    "cannot read the candidate configuration {}: {error}",
                    candidate_path.display()
                ),
            );
            logging::warn(None, &format!("{} stays in effect", main_path.display()));
            return Ok((main.clone(), None));
        }
    };

    let candidate = match resolve_candidate(&bytes, candidate_path, overrides) {
        Ok(candidate) => candidate,
        Err(reason) => {
            logging::error(
                None,
                &format!(
                    "candidate configuration {} is invalid: {reason}",
                    candidate_path.display()
                ),
            );
            logging::warn(None, &format!("{} stays in effect", main_path.display()));
            return Ok((main.clone(), None));
        }
    };

    let report = check::run(&candidate, check::Options::default());
    if let Some(line) = report
        .lines
        .iter()
        .find(|line| line.level == check::LineLevel::Fail)
    {
        logging::error(
            None,
            &format!(
                "candidate configuration {}: {}: {}",
                candidate_path.display(),
                line.subject,
                line.detail
            ),
        );
        logging::warn(None, &format!("{} stays in effect", main_path.display()));
        return Ok((main.clone(), None));
    }

    let main_bytes = std::fs::read(main_path).unwrap_or_default();
    if bytes == main_bytes {
        logging::info(
            None,
            &format!(
                "candidate {} matches {}; nothing to promote",
                candidate_path.display(),
                main_path.display()
            ),
        );
        return Ok((candidate, None));
    }

    let promotion = Candidate {
        source: candidate_path.to_path_buf(),
        main: main_path.to_path_buf(),
        bytes,
        stamp: now_ms(),
    };
    Ok((candidate, Some(promotion)))
}

fn resolve_candidate(bytes: &[u8], path: &Path, overrides: &Overrides) -> Result<Config, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("it is not valid UTF-8: {error}"))?;
    let file = config::parse(text, path).map_err(|error| error.to_string())?;
    Config::resolve(Some(file), overrides.clone()).map_err(|error| error.to_string())
}

/// Whether two paths name the same file, following a symlink or a relative spelling.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn run_monitor(
    settings: &Config,
    source: Option<&Path>,
    reload: Option<monitor::Reload>,
    listen: Option<String>,
    compression: Compression,
    mut start: monitor::Start,
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

    start.startup = Some(monitor::startup_event(settings, source, serving.as_deref()));
    let outcome = monitor::run(settings, prober, store, &stop, reload, start);

    if let Some(thread) = web_thread {
        stop.request();
        let _ = thread.join();
    }

    outcome?;
    Ok(())
}
