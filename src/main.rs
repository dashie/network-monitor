//! Entry point: parse the command line, resolve the configuration, dispatch.

use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use clap::Parser;
use thiserror::Error;

use network_monitor::cli::{Cli, Command};
use network_monitor::config::{self, Config, ConfigError};
use network_monitor::logging::{self, Level};
use network_monitor::monitor::{self, RunError};
use network_monitor::probe::interface;
use network_monitor::probe::{IcmpProbe, Probe};
use network_monitor::query::{self, QueryError};
use network_monitor::shutdown;
use network_monitor::store::{Store, StoreError};

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
    let reload = loaded.as_ref().map(|loaded| monitor::Reload {
        path: loaded.path.clone(),
        overrides: overrides.clone(),
    });
    let settings = Config::resolve(loaded.map(|loaded| loaded.config), overrides)?;

    if cli.show_config {
        print!("{}", settings.to_toml());
        return Ok(ExitCode::SUCCESS);
    }

    match cli.command {
        Some(Command::Monitor) => {
            settings.require_targets()?;
            validate_interfaces(&settings)?;
            run_monitor(&settings, reload)?;
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

fn validate_interfaces(settings: &Config) -> Result<(), AppError> {
    for target in &settings.targets {
        if let Some(name) = &target.interface {
            interface::validate(name).map_err(|reason| {
                AppError::Invalid(format!("interface '{name}' for {}: {reason}", target.ip))
            })?;
        }
    }
    Ok(())
}

fn run_monitor(settings: &Config, reload: Option<monitor::Reload>) -> Result<(), AppError> {
    let prober: Arc<dyn Probe> = Arc::new(IcmpProbe::new(settings.timeout));
    let store = Arc::new(Mutex::new(Store::open_rw(&settings.database)?));
    let stop = shutdown::Shutdown::new();

    shutdown::install();
    monitor::run(settings, prober, store, &stop, reload)?;
    Ok(())
}
