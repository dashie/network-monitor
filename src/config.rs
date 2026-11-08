//! Configuration: defaults, TOML file, command-line overrides.
//!
//! The merge order is fixed by BR-3 and INV-2: command line, then file, then the
//! built-in default. [`Config::resolve`] is the only function that knows that order.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_CONFIG_FILE: &str = "network-monitor.toml";
pub const DEFAULT_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 2_000;
pub const DEFAULT_DATABASE: &str = "network-monitor.db";
pub const DEFAULT_RETENTION_DAYS: u64 = 0;

/// The configuration file as written on disk. Every key is optional so that the
/// same struct can express a partial file.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub interval_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub database: Option<String>,
    pub retention_days: Option<u64>,
    pub targets: Option<Vec<FileTarget>>,
}

/// One `[[targets]]` entry of the configuration file.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileTarget {
    pub ip: String,
    pub interface: Option<String>,
    pub label: Option<String>,
}

/// A validated target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub ip: Ipv4Addr,
    pub interface: Option<String>,
    pub label: Option<String>,
}

impl Target {
    /// Parse the command-line form `IP[,interface=NAME][,label=NAME]`.
    pub fn parse_spec(spec: &str) -> Result<Target, String> {
        let mut parts = spec.split(',');
        let address = parts.next().unwrap_or("").trim();
        if address.is_empty() {
            return Err("the target address is missing".to_string());
        }
        let ip = parse_ipv4(address)?;

        let mut interface = None;
        let mut label = None;
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| format!("expected key=value in target attribute '{part}'"))?;
            let value = value.trim();
            if value.is_empty() {
                return Err(format!("attribute '{}' has an empty value", key.trim()));
            }
            match key.trim() {
                "interface" => interface = Some(value.to_string()),
                "label" => label = Some(value.to_string()),
                other => {
                    return Err(format!(
                        "unknown target attribute '{other}' (expected interface or label)"
                    ))
                }
            }
        }

        Ok(Target {
            ip,
            interface,
            label,
        })
    }

    /// Name used in logs and query output: the label when present, the address otherwise.
    pub fn display(&self) -> String {
        match &self.label {
            Some(label) => label.clone(),
            None => self.ip.to_string(),
        }
    }

    fn from_file(target: FileTarget, index: usize) -> Result<Target, ConfigError> {
        let ip = parse_ipv4(&target.ip).map_err(|reason| ConfigError::Invalid {
            key: format!("targets[{index}].ip"),
            reason,
        })?;
        let interface = target.interface.filter(|value| !value.trim().is_empty());
        let label = target.label.filter(|value| !value.trim().is_empty());
        Ok(Target {
            ip,
            interface,
            label,
        })
    }
}

/// The effective configuration, after the three layers have been merged.
#[derive(Debug, Clone)]
pub struct Config {
    pub interval: Duration,
    pub timeout: Duration,
    pub database: PathBuf,
    pub retention_days: u64,
    pub targets: Vec<Target>,
}

/// Command-line values that override the file. `None` means "not given".
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub interval_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub database: Option<PathBuf>,
    pub retention_days: Option<u64>,
    pub targets: Option<Vec<Target>>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read the configuration file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse the configuration file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("invalid configuration: {key}: {reason}")]
    Invalid { key: String, reason: String },
    #[error("no targets configured; add [[targets]] entries or use --target")]
    NoTargets,
}

/// Load the configuration file.
///
/// An explicit path must exist. Without an explicit path, `network-monitor.toml` is
/// read when present and skipped otherwise, so a fully command-line invocation works
/// without a file.
pub fn load(explicit: Option<&Path>) -> Result<Option<FileConfig>, ConfigError> {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => {
            let default = PathBuf::from(DEFAULT_CONFIG_FILE);
            if !default.exists() {
                return Ok(None);
            }
            default
        }
    };

    let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        path: path.clone(),
        source,
    })?;
    let parsed = toml::from_str::<FileConfig>(&text).map_err(|source| ConfigError::Parse {
        path,
        source: Box::new(source),
    })?;
    Ok(Some(parsed))
}

impl Config {
    /// Merge the file and the overrides into the effective configuration.
    pub fn resolve(file: Option<FileConfig>, overrides: Overrides) -> Result<Config, ConfigError> {
        let file = file.unwrap_or_default();

        let interval_ms = overrides
            .interval_ms
            .or(file.interval_ms)
            .unwrap_or(DEFAULT_INTERVAL_MS);
        if interval_ms == 0 {
            return Err(ConfigError::Invalid {
                key: "interval_ms".to_string(),
                reason: "must be greater than zero".to_string(),
            });
        }

        let timeout_ms = overrides
            .timeout_ms
            .or(file.timeout_ms)
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        if timeout_ms == 0 {
            return Err(ConfigError::Invalid {
                key: "timeout_ms".to_string(),
                reason: "must be greater than zero".to_string(),
            });
        }

        let database = overrides
            .database
            .or_else(|| file.database.map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DATABASE));

        let retention_days = overrides
            .retention_days
            .or(file.retention_days)
            .unwrap_or(DEFAULT_RETENTION_DAYS);

        let targets = match overrides.targets {
            Some(targets) => targets,
            None => file
                .targets
                .map(|targets| {
                    targets
                        .into_iter()
                        .enumerate()
                        .map(|(index, target)| Target::from_file(target, index))
                        .collect()
                })
                .transpose()?
                .unwrap_or_default(),
        };

        ensure_unique(&targets)?;

        Ok(Config {
            interval: Duration::from_millis(interval_ms),
            timeout: Duration::from_millis(timeout_ms),
            database,
            retention_days,
            targets,
        })
    }

    /// Fail when a command that needs targets is started without any.
    pub fn require_targets(&self) -> Result<(), ConfigError> {
        if self.targets.is_empty() {
            Err(ConfigError::NoTargets)
        } else {
            Ok(())
        }
    }

    /// Render the effective configuration as TOML, for `--show-config`.
    pub fn to_toml(&self) -> String {
        #[derive(Serialize)]
        struct SerializableConfig {
            interval_ms: u64,
            timeout_ms: u64,
            database: String,
            retention_days: u64,
            targets: Vec<SerializableTarget>,
        }

        #[derive(Serialize)]
        struct SerializableTarget {
            ip: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            interface: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            label: Option<String>,
        }

        let serializable = SerializableConfig {
            interval_ms: self.interval.as_millis() as u64,
            timeout_ms: self.timeout.as_millis() as u64,
            database: self.database.to_string_lossy().into_owned(),
            retention_days: self.retention_days,
            targets: self
                .targets
                .iter()
                .map(|target| SerializableTarget {
                    ip: target.ip.to_string(),
                    interface: target.interface.clone(),
                    label: target.label.clone(),
                })
                .collect(),
        };

        toml::to_string_pretty(&serializable).expect("the effective configuration is serializable")
    }
}

fn parse_ipv4(text: &str) -> Result<Ipv4Addr, String> {
    match text.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => Ok(ip),
        Ok(IpAddr::V6(_)) => Err(format!(
            "'{text}' is an IPv6 address; only IPv4 targets are supported in this version"
        )),
        Err(_) => Err(format!("'{text}' is not an IP address")),
    }
}

/// Two entries for the same address would share one row in the event database.
fn ensure_unique(targets: &[Target]) -> Result<(), ConfigError> {
    for (index, target) in targets.iter().enumerate() {
        if targets[..index].iter().any(|other| other.ip == target.ip) {
            return Err(ConfigError::Invalid {
                key: format!("targets[{index}].ip"),
                reason: format!("duplicate target {}", target.ip),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides() -> Overrides {
        Overrides::default()
    }

    #[test]
    fn defaults_apply_without_any_file() {
        let config = Config::resolve(None, overrides()).unwrap();
        assert_eq!(config.interval, Duration::from_millis(DEFAULT_INTERVAL_MS));
        assert_eq!(config.timeout, Duration::from_millis(DEFAULT_TIMEOUT_MS));
        assert_eq!(config.database, PathBuf::from(DEFAULT_DATABASE));
        assert_eq!(config.retention_days, DEFAULT_RETENTION_DAYS);
        assert!(config.targets.is_empty());
    }

    #[test]
    fn parses_target_specs() {
        let target = Target::parse_spec("1.1.1.1,interface=eth1,label=cloudflare").unwrap();
        assert_eq!(target.ip, Ipv4Addr::new(1, 1, 1, 1));
        assert_eq!(target.interface.as_deref(), Some("eth1"));
        assert_eq!(target.label.as_deref(), Some("cloudflare"));
    }

    #[test]
    fn rejects_unknown_target_attributes() {
        assert!(Target::parse_spec("1.1.1.1,color=red").is_err());
        assert!(Target::parse_spec("1.1.1.1,interface=").is_err());
        assert!(Target::parse_spec("").is_err());
    }

    #[test]
    fn rejects_ipv6_targets() {
        assert!(Target::parse_spec("2001:db8::1").is_err());
    }

    #[test]
    fn rejects_zero_interval() {
        let result = Config::resolve(
            None,
            Overrides {
                interval_ms: Some(0),
                ..Overrides::default()
            },
        );
        assert!(matches!(result, Err(ConfigError::Invalid { .. })));
    }

    #[test]
    fn rejects_duplicate_targets() {
        let target = || FileTarget {
            ip: "1.1.1.1".to_string(),
            interface: None,
            label: None,
        };
        let result = Config::resolve(
            Some(FileConfig {
                targets: Some(vec![target(), target()]),
                ..FileConfig::default()
            }),
            overrides(),
        );
        assert!(matches!(result, Err(ConfigError::Invalid { .. })));
    }

    #[test]
    fn serializes_the_effective_configuration() {
        let config = Config::resolve(
            Some(FileConfig {
                targets: Some(vec![FileTarget {
                    ip: "1.1.1.1".to_string(),
                    interface: Some("eth1".to_string()),
                    label: None,
                }]),
                ..FileConfig::default()
            }),
            overrides(),
        )
        .unwrap();
        let text = config.to_toml();
        assert!(text.contains("interval_ms = 5000"));
        assert!(text.contains("[[targets]]"));
        assert!(text.contains("interface = \"eth1\""));
    }

    #[test]
    fn unknown_file_keys_are_rejected() {
        let parsed = toml::from_str::<FileConfig>("interval_ms = 1000\nunknown = 1\n");
        assert!(parsed.is_err());
    }
}
