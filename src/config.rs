//! Configuration: defaults, TOML file, command-line overrides.
//!
//! The merge order is fixed by BR-3 and INV-2: command line, then file, then the
//! built-in default. [`Config::resolve`] is the only function that knows that order.
//!
//! A target is a [`Method`](crate::probe::Method) plus its parameters, and every common
//! property — `interface`, `label`, `timeout_ms`, `interval_ms` — is optional per target.
//! [`TargetSpec`] is what the file or the command line states; [`Target`] is the same
//! target with the global defaults applied, and its key is its identity in the database.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::probe::http::parse_url;
use crate::probe::{Endpoint, Method, RecordType, StatusMatch};

pub const DEFAULT_CONFIG_FILE: &str = "network-monitor.toml";
pub const DEFAULT_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 2_000;
pub const DEFAULT_DATABASE: &str = "network-monitor.db";
pub const DEFAULT_RETENTION_DAYS: u64 = 0;
pub const DEFAULT_STABILIZE_SECONDS: u64 = 60;

/// What every rendering of the bot token shows instead of the secret.
pub const REDACTED_TOKEN: &str = "<redacted>";

/// The ways a target can be checked.
const METHODS: [&str; 4] = ["icmp", "tcp", "dns", "http"];

/// The configuration file as written on disk. Every key is optional so that the
/// same struct can express a partial file.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub interval_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub database: Option<String>,
    pub retention_days: Option<u64>,
    pub telegram: Option<FileTelegram>,
    pub targets: Option<Vec<FileTarget>>,
}

/// The `[telegram]` section as written on disk.
///
/// The section is the only source of these settings: there is no flag and no
/// environment variable, because the bot token is a secret (ADR-0026).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileTelegram {
    /// The secret of the bot; it travels in the request path, so it may not contain
    /// whitespace or a path separator.
    pub bot_token: Option<String>,
    /// The chat or channel the messages are posted to.
    pub chat_id: Option<String>,
    /// How long `up` must hold before a recovery is reported; `0` reports it at once.
    pub stabilize_seconds: Option<u64>,
}

/// One `[[targets]]` entry of the configuration file.
///
/// `method` selects which of the remaining keys apply (INV-2: every key has a flag too);
/// a key that belongs to another method is refused rather than ignored.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileTarget {
    /// `icmp` (the default), `tcp`, `dns`, or `http`.
    pub method: Option<String>,
    /// Address of an `icmp` target.
    pub ip: Option<String>,
    /// Host of a `tcp` target: an address or a name.
    pub host: Option<String>,
    /// Port of a `tcp` target.
    pub port: Option<u16>,
    /// Name a `dns` target must resolve.
    pub name: Option<String>,
    /// Server a `dns` target asks; the system resolver is used when absent.
    pub resolver: Option<String>,
    /// `A` (the default) or `AAAA`.
    pub record: Option<String>,
    /// URL of an `http` target.
    pub url: Option<String>,
    /// Statuses that count as `up`: a class such as `2xx`, or exact codes.
    pub expect_status: Option<Vec<String>>,
    pub interface: Option<String>,
    pub label: Option<String>,
    /// Where the lane of this target sits on the page: lower first, absent = after them.
    pub priority: Option<i64>,
    pub timeout_ms: Option<u64>,
    pub interval_ms: Option<u64>,
}

/// A target as the file or the command line states it, before the global defaults apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSpec {
    pub endpoint: Endpoint,
    pub interface: Option<String>,
    pub label: Option<String>,
    pub priority: Option<i64>,
    pub timeout_ms: Option<u64>,
    pub interval_ms: Option<u64>,
}

/// A validated target: the endpoint plus the effective timeout and interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub endpoint: Endpoint,
    pub interface: Option<String>,
    pub label: Option<String>,
    pub priority: Option<i64>,
    pub timeout: Duration,
    pub interval: Duration,
}

impl Target {
    /// The identity of the target in the database and on the page.
    pub fn key(&self) -> String {
        self.endpoint.to_string()
    }

    /// Name used in logs: the label when present, the key otherwise.
    pub fn display(&self) -> String {
        match &self.label {
            Some(label) => label.clone(),
            None => self.key(),
        }
    }

    pub fn method(&self) -> Method {
        self.endpoint.method()
    }

    /// Apply the global defaults and refuse what the method cannot do.
    fn resolve(
        spec: TargetSpec,
        index: usize,
        timeout: Duration,
        interval: Duration,
    ) -> Result<Target, ConfigError> {
        if spec.interface.is_some() && !matches!(spec.endpoint.method(), Method::Icmp | Method::Tcp)
        {
            return Err(invalid(
                format!("targets[{index}].interface"),
                format!("is not supported by the {} method", spec.endpoint.method()),
            ));
        }

        Ok(Target {
            endpoint: spec.endpoint,
            interface: spec.interface,
            label: spec.label,
            priority: spec.priority,
            timeout: millis(spec.timeout_ms, format!("targets[{index}].timeout_ms"))?
                .unwrap_or(timeout),
            interval: millis(spec.interval_ms, format!("targets[{index}].interval_ms"))?
                .unwrap_or(interval),
        })
    }
}

impl TargetSpec {
    /// Parse the command-line form `[method:]address[,key=value]…`.
    pub fn parse_spec(spec: &str) -> Result<TargetSpec, String> {
        let mut parts = spec.split(',');
        let head = parts.next().unwrap_or("").trim();
        if head.is_empty() {
            return Err("the target is missing".to_string());
        }
        let (method, address) = split_method(head)?;

        let mut interface = None;
        let mut label = None;
        let mut priority = None;
        let mut timeout_ms = None;
        let mut interval_ms = None;
        let mut record = None;
        let mut expect: Vec<String> = Vec::new();

        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (name, value) = part
                .split_once('=')
                .ok_or_else(|| format!("expected key=value in target attribute '{part}'"))?;
            let name = name.trim();
            let value = value.trim();
            if value.is_empty() {
                return Err(format!("attribute '{name}' has an empty value"));
            }
            match name {
                "interface" => interface = Some(value.to_string()),
                "label" => label = Some(value.to_string()),
                "priority" => priority = Some(parse_priority(value)?),
                "timeout_ms" => timeout_ms = Some(parse_millis(value)?),
                "interval_ms" => interval_ms = Some(parse_millis(value)?),
                "record" => record = Some(value.to_string()),
                "expect" => expect.push(value.to_string()),
                other => {
                    return Err(format!(
                        "unknown target attribute '{other}' (expected interface, label, \
                         priority, timeout_ms, interval_ms, record or expect)"
                    ))
                }
            }
        }

        let endpoint = match method {
            Method::Icmp => {
                reject_attributes(method, record.is_some(), !expect.is_empty())?;
                Endpoint::Icmp {
                    address: parse_ipv4(address)?,
                }
            }
            Method::Tcp => {
                reject_attributes(method, record.is_some(), !expect.is_empty())?;
                let (host, port) = address
                    .rsplit_once(':')
                    .ok_or_else(|| "a tcp target is host:port".to_string())?;
                Endpoint::Tcp {
                    host: parse_host(host)?,
                    port: parse_port(port)?,
                }
            }
            Method::Dns => {
                reject_attributes(method, record.is_some() && false, !expect.is_empty())?;
                let (name, resolver) = match address.split_once('@') {
                    Some((name, resolver)) => (name, Some(parse_ipv4_resolver(resolver)?)),
                    None => (address, None),
                };
                Endpoint::Dns {
                    name: parse_host(name)?,
                    resolver,
                    record: match record.as_deref() {
                        None => RecordType::A,
                        Some(text) => parse_record(text)?,
                    },
                }
            }
            Method::Http => {
                if record.is_some() {
                    return Err("'record' does not apply to the http method".to_string());
                }
                let parts = parse_url(address)?;
                let expect = if expect.is_empty() {
                    StatusMatch::default()
                } else {
                    StatusMatch::parse(&expect)?
                };
                Endpoint::Http {
                    scheme: parts.scheme,
                    host: parts.host,
                    port: parts.port,
                    target: parts.target,
                    expect,
                }
            }
        };

        Ok(TargetSpec {
            endpoint,
            interface,
            label,
            priority,
            timeout_ms,
            interval_ms,
        })
    }

    fn from_file(target: FileTarget, index: usize) -> Result<TargetSpec, ConfigError> {
        let key = |name: &str| format!("targets[{index}].{name}");

        let method = match clean(target.method).as_deref() {
            None => Method::Icmp,
            Some(name) => parse_method(name).map_err(|reason| invalid(key("method"), reason))?,
        };

        let reject = |name: &str, present: bool| -> Result<(), ConfigError> {
            if present {
                Err(invalid(
                    key(name),
                    format!("does not apply to the {method} method"),
                ))
            } else {
                Ok(())
            }
        };

        let endpoint = match method {
            Method::Icmp => {
                reject("host", target.host.is_some())?;
                reject("port", target.port.is_some())?;
                reject("name", target.name.is_some())?;
                reject("resolver", target.resolver.is_some())?;
                reject("record", target.record.is_some())?;
                reject("url", target.url.is_some())?;
                reject("expect_status", target.expect_status.is_some())?;
                let ip = required(target.ip.as_deref(), &key("ip"))?;
                Endpoint::Icmp {
                    address: parse_ipv4(ip).map_err(|reason| invalid(key("ip"), reason))?,
                }
            }
            Method::Tcp => {
                reject("ip", target.ip.is_some())?;
                reject("name", target.name.is_some())?;
                reject("resolver", target.resolver.is_some())?;
                reject("record", target.record.is_some())?;
                reject("url", target.url.is_some())?;
                reject("expect_status", target.expect_status.is_some())?;
                let host = required(target.host.as_deref(), &key("host"))?;
                let port = target
                    .port
                    .ok_or_else(|| invalid(key("port"), "is required"))?;
                Endpoint::Tcp {
                    host: parse_host(host).map_err(|reason| invalid(key("host"), reason))?,
                    port: if port == 0 {
                        return Err(invalid(key("port"), "must be between 1 and 65535"));
                    } else {
                        port
                    },
                }
            }
            Method::Dns => {
                reject("ip", target.ip.is_some())?;
                reject("host", target.host.is_some())?;
                reject("port", target.port.is_some())?;
                reject("url", target.url.is_some())?;
                reject("expect_status", target.expect_status.is_some())?;
                let name = required(target.name.as_deref(), &key("name"))?;
                let resolver = match clean(target.resolver) {
                    Some(text) => Some(
                        parse_ipv4_resolver(&text)
                            .map_err(|reason| invalid(key("resolver"), reason))?,
                    ),
                    None => None,
                };
                let record = match clean(target.record) {
                    Some(text) => {
                        parse_record(&text).map_err(|reason| invalid(key("record"), reason))?
                    }
                    None => RecordType::A,
                };
                Endpoint::Dns {
                    name: parse_host(name).map_err(|reason| invalid(key("name"), reason))?,
                    resolver,
                    record,
                }
            }
            Method::Http => {
                reject("ip", target.ip.is_some())?;
                reject("host", target.host.is_some())?;
                reject("port", target.port.is_some())?;
                reject("name", target.name.is_some())?;
                reject("resolver", target.resolver.is_some())?;
                reject("record", target.record.is_some())?;
                let url = required(target.url.as_deref(), &key("url"))?;
                let parts = parse_url(url).map_err(|reason| invalid(key("url"), reason))?;
                let expect = match &target.expect_status {
                    Some(parts) => StatusMatch::parse(parts)
                        .map_err(|reason| invalid(key("expect_status"), reason))?,
                    None => StatusMatch::default(),
                };
                Endpoint::Http {
                    scheme: parts.scheme,
                    host: parts.host,
                    port: parts.port,
                    target: parts.target,
                    expect,
                }
            }
        };

        Ok(TargetSpec {
            endpoint,
            interface: clean(target.interface),
            label: clean(target.label),
            priority: target.priority,
            timeout_ms: target.timeout_ms,
            interval_ms: target.interval_ms,
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
    pub telegram: Option<Telegram>,
    pub targets: Vec<Target>,
}

/// The effective notification settings: where to post, and how long `up` must hold.
#[derive(Clone, PartialEq, Eq)]
pub struct Telegram {
    pub bot_token: String,
    pub chat_id: String,
    pub stabilize: Duration,
}

/// The token is a secret: it must never reach a log line through a derived `Debug`.
impl std::fmt::Debug for Telegram {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Telegram")
            .field("bot_token", &REDACTED_TOKEN)
            .field("chat_id", &self.chat_id)
            .field("stabilize", &self.stabilize)
            .finish()
    }
}

/// Command-line values that override the file. `None` means "not given".
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub interval_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub database: Option<PathBuf>,
    pub retention_days: Option<u64>,
    pub targets: Option<Vec<TargetSpec>>,
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

/// A configuration file that was read, together with its path.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub path: PathBuf,
    pub config: FileConfig,
}

/// Load the configuration file.
///
/// An explicit path must exist. Without an explicit path, `network-monitor.toml` is
/// read when present and skipped otherwise, so a fully command-line invocation works
/// without a file.
pub fn load(explicit: Option<&Path>) -> Result<Option<Loaded>, ConfigError> {
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
    let config = parse(&text, &path)?;
    Ok(Some(Loaded { path, config }))
}

/// Parse the content of a configuration file that was already read.
pub fn parse(text: &str, path: &Path) -> Result<FileConfig, ConfigError> {
    toml::from_str::<FileConfig>(text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
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
            return Err(invalid("interval_ms", "must be greater than zero"));
        }

        let timeout_ms = overrides
            .timeout_ms
            .or(file.timeout_ms)
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        if timeout_ms == 0 {
            return Err(invalid("timeout_ms", "must be greater than zero"));
        }

        let database = overrides
            .database
            .or_else(|| file.database.map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DATABASE));

        let retention_days = overrides
            .retention_days
            .or(file.retention_days)
            .unwrap_or(DEFAULT_RETENTION_DAYS);

        let specs = match overrides.targets {
            Some(specs) => specs,
            None => file
                .targets
                .map(|targets| {
                    targets
                        .into_iter()
                        .enumerate()
                        .map(|(index, target)| TargetSpec::from_file(target, index))
                        .collect()
                })
                .transpose()?
                .unwrap_or_default(),
        };

        let interval = Duration::from_millis(interval_ms);
        let timeout = Duration::from_millis(timeout_ms);
        let targets = specs
            .into_iter()
            .enumerate()
            .map(|(index, spec)| Target::resolve(spec, index, timeout, interval))
            .collect::<Result<Vec<_>, _>>()?;
        ensure_unique(&targets)?;
        let telegram = resolve_telegram(file.telegram)?;

        Ok(Config {
            interval,
            timeout,
            database,
            retention_days,
            telegram,
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
    ///
    /// The result parses back into the same targets: a method prints the keys it uses and
    /// nothing else, and a timeout or interval is stated only when it differs from the
    /// global one.
    pub fn to_toml(&self) -> String {
        #[derive(Serialize, Default)]
        struct SerializableConfig {
            interval_ms: u64,
            timeout_ms: u64,
            database: String,
            retention_days: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            telegram: Option<SerializableTelegram>,
            targets: Vec<SerializableTarget>,
        }

        #[derive(Serialize, Default)]
        struct SerializableTelegram {
            bot_token: String,
            chat_id: String,
            stabilize_seconds: u64,
        }

        #[derive(Serialize, Default)]
        struct SerializableTarget {
            method: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            ip: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            host: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            port: Option<u16>,
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            resolver: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            record: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            url: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            expect_status: Option<Vec<String>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            interface: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            label: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            priority: Option<i64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            timeout_ms: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            interval_ms: Option<u64>,
        }

        let targets = self
            .targets
            .iter()
            .map(|target| {
                let mut serializable = SerializableTarget {
                    method: target.method().as_str().to_string(),
                    ..SerializableTarget::default()
                };
                match &target.endpoint {
                    Endpoint::Icmp { address } => serializable.ip = Some(address.to_string()),
                    Endpoint::Tcp { host, port } => {
                        serializable.host = Some(host.clone());
                        serializable.port = Some(*port);
                    }
                    Endpoint::Dns {
                        name,
                        resolver,
                        record,
                    } => {
                        serializable.name = Some(name.clone());
                        serializable.resolver = resolver.map(|address| address.to_string());
                        if *record != RecordType::A {
                            serializable.record = Some(record.as_str().to_string());
                        }
                    }
                    Endpoint::Http { expect, .. } => {
                        serializable.url = Some(target.endpoint.to_string());
                        if *expect != StatusMatch::default() {
                            serializable.expect_status = Some(expect.specs());
                        }
                    }
                }
                serializable.interface = target.interface.clone();
                serializable.label = target.label.clone();
                serializable.priority = target.priority;
                if target.timeout != self.timeout {
                    serializable.timeout_ms = Some(target.timeout.as_millis() as u64);
                }
                if target.interval != self.interval {
                    serializable.interval_ms = Some(target.interval.as_millis() as u64);
                }
                serializable
            })
            .collect();

        let serializable = SerializableConfig {
            interval_ms: self.interval.as_millis() as u64,
            timeout_ms: self.timeout.as_millis() as u64,
            database: self.database.to_string_lossy().into_owned(),
            retention_days: self.retention_days,
            telegram: self.telegram.as_ref().map(|telegram| SerializableTelegram {
                bot_token: REDACTED_TOKEN.to_string(),
                chat_id: telegram.chat_id.clone(),
                stabilize_seconds: telegram.stabilize.as_secs(),
            }),
            targets,
        };

        toml::to_string_pretty(&serializable).expect("the effective configuration is serializable")
    }
}

fn invalid(key: impl Into<String>, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        key: key.into(),
        reason: reason.into(),
    }
}

/// The text of an optional key, trimmed, with an empty value read as absent.
fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn required<'a>(value: Option<&'a str>, key: &str) -> Result<&'a str, ConfigError> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Ok(value),
        None => Err(invalid(key, "is required")),
    }
}

fn millis(value: Option<u64>, key: String) -> Result<Option<Duration>, ConfigError> {
    match value {
        None => Ok(None),
        Some(0) => Err(invalid(key, "must be greater than zero")),
        Some(ms) => Ok(Some(Duration::from_millis(ms))),
    }
}

/// The priority of a target: any integer, lower first. Absent leaves it unordered.
fn parse_priority(text: &str) -> Result<i64, String> {
    text.parse::<i64>()
        .map_err(|_| format!("'{text}' is not a whole number"))
}

fn parse_millis(text: &str) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(0) | Err(_) => Err(format!("'{text}' is not a positive number of milliseconds")),
        Ok(ms) => Ok(ms),
    }
}

fn parse_method(name: &str) -> Result<Method, String> {
    match name {
        "icmp" => Ok(Method::Icmp),
        "tcp" => Ok(Method::Tcp),
        "dns" => Ok(Method::Dns),
        "http" => Ok(Method::Http),
        other => Err(format!(
            "unknown method '{other}' (expected icmp, tcp, dns or http)"
        )),
    }
}

/// Split `[method:]address`; a head without a known method is an ICMP address.
fn split_method(head: &str) -> Result<(Method, &str), String> {
    if let Some((name, rest)) = head.split_once(':') {
        if METHODS.contains(&name) {
            return Ok((parse_method(name)?, rest.trim()));
        }
    }
    Ok((Method::Icmp, head))
}

fn reject_attributes(method: Method, record: bool, expect: bool) -> Result<(), String> {
    if record {
        return Err(format!("'record' does not apply to the {method} method"));
    }
    if expect {
        return Err(format!("'expect' does not apply to the {method} method"));
    }
    Ok(())
}

fn parse_host(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the host is missing".to_string());
    }
    if !text.is_ascii() {
        return Err("the host must be written in ASCII".to_string());
    }
    if text.contains([':', '/', '@']) || text.chars().any(char::is_whitespace) {
        return Err(format!("'{text}' is not a host name or address"));
    }
    Ok(text.to_string())
}

fn parse_port(text: &str) -> Result<u16, String> {
    match text.trim().parse::<u16>() {
        Ok(0) | Err(_) => Err(format!("'{text}' is not a port between 1 and 65535")),
        Ok(port) => Ok(port),
    }
}

fn parse_record(text: &str) -> Result<RecordType, String> {
    match text.trim().to_ascii_uppercase().as_str() {
        "A" => Ok(RecordType::A),
        "AAAA" => Ok(RecordType::Aaaa),
        other => Err(format!(
            "'{other}' is not a record type (expected A or AAAA)"
        )),
    }
}

fn parse_ipv4(text: &str) -> Result<Ipv4Addr, String> {
    match text.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => Ok(ip),
        Ok(IpAddr::V6(_)) => Err(format!(
            "'{text}' is an IPv6 address; only IPv4 targets are supported in this version"
        )),
        Err(_) => Err(format!("'{text}' is not an IP address")),
    }
}

fn parse_ipv4_resolver(text: &str) -> Result<IpAddr, String> {
    match text.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => Ok(IpAddr::V4(ip)),
        Ok(IpAddr::V6(_)) => Err(format!(
            "'{text}' is an IPv6 address; only IPv4 resolvers are supported in this version"
        )),
        Err(_) => Err(format!("'{text}' is not an IP address")),
    }
}

/// Two entries with the same key would share one row in the event database.
fn ensure_unique(targets: &[Target]) -> Result<(), ConfigError> {
    for (index, target) in targets.iter().enumerate() {
        if targets[..index]
            .iter()
            .any(|other| other.key() == target.key())
        {
            return Err(invalid(
                format!("targets[{index}]"),
                format!("duplicate target {}", target.key()),
            ));
        }
    }
    Ok(())
}

/// Turn the raw section into the effective settings, or `None` when it is absent.
fn resolve_telegram(file: Option<FileTelegram>) -> Result<Option<Telegram>, ConfigError> {
    let Some(file) = file else {
        return Ok(None);
    };

    // The token travels in the request path, so a separator would change the request.
    let bot_token = required(file.bot_token.as_deref(), "telegram.bot_token")?;
    if bot_token
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '/' | '?' | '#'))
    {
        return Err(invalid(
            "telegram.bot_token",
            "must not contain whitespace, '/', '?' or '#'",
        ));
    }
    let chat_id = required(file.chat_id.as_deref(), "telegram.chat_id")?;

    Ok(Some(Telegram {
        bot_token: bot_token.to_string(),
        chat_id: chat_id.to_string(),
        stabilize: Duration::from_secs(file.stabilize_seconds.unwrap_or(DEFAULT_STABILIZE_SECONDS)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Scheme;

    fn overrides() -> Overrides {
        Overrides::default()
    }

    fn file(targets: Vec<FileTarget>) -> FileConfig {
        FileConfig {
            targets: Some(targets),
            ..FileConfig::default()
        }
    }

    fn file_with_telegram(telegram: FileTelegram) -> FileConfig {
        FileConfig {
            telegram: Some(telegram),
            ..FileConfig::default()
        }
    }

    fn with_telegram(token: &str, chat_id: &str, stabilize_seconds: Option<u64>) -> FileConfig {
        file_with_telegram(FileTelegram {
            bot_token: Some(token.to_string()),
            chat_id: Some(chat_id.to_string()),
            stabilize_seconds,
        })
    }

    fn icmp(ip: &str) -> FileTarget {
        FileTarget {
            ip: Some(ip.to_string()),
            ..FileTarget::default()
        }
    }

    fn resolve(file: Option<FileConfig>) -> Config {
        Config::resolve(file, overrides()).unwrap()
    }

    fn keys(config: &Config) -> Vec<String> {
        config.targets.iter().map(Target::key).collect()
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
    fn an_icmp_target_is_the_default_method() {
        let config = resolve(Some(file(vec![icmp("1.1.1.1")])));
        assert_eq!(config.targets[0].method(), Method::Icmp);
        assert_eq!(config.targets[0].key(), "icmp://1.1.1.1");
        assert_eq!(
            config.targets[0].timeout,
            Duration::from_millis(DEFAULT_TIMEOUT_MS)
        );
    }

    #[test]
    fn every_method_is_read_from_the_file() {
        let config = resolve(Some(file(vec![
            FileTarget {
                method: Some("tcp".to_string()),
                host: Some("1.1.1.1".to_string()),
                port: Some(443),
                label: Some("https".to_string()),
                ..FileTarget::default()
            },
            FileTarget {
                method: Some("dns".to_string()),
                name: Some("example.com".to_string()),
                resolver: Some("9.9.9.9".to_string()),
                record: Some("AAAA".to_string()),
                ..FileTarget::default()
            },
            FileTarget {
                method: Some("http".to_string()),
                url: Some("https://example.com/health".to_string()),
                expect_status: Some(vec!["204".to_string()]),
                ..FileTarget::default()
            },
            icmp("1.0.0.1"),
        ])));

        assert_eq!(
            keys(&config),
            vec![
                "tcp://1.1.1.1:443",
                "dns://example.com@9.9.9.9",
                "https://example.com/health",
                "icmp://1.0.0.1"
            ]
        );
        assert_eq!(
            config.targets[1].endpoint,
            Endpoint::Dns {
                name: "example.com".to_string(),
                resolver: Some("9.9.9.9".parse().unwrap()),
                record: RecordType::Aaaa,
            }
        );
        assert_eq!(
            config.targets[2].endpoint,
            Endpoint::Http {
                scheme: Scheme::Https,
                host: "example.com".to_string(),
                port: 443,
                target: "/health".to_string(),
                expect: StatusMatch {
                    classes: Vec::new(),
                    codes: vec![204],
                },
            }
        );
    }

    #[test]
    fn a_key_of_another_method_is_refused() {
        let error = Config::resolve(
            Some(file(vec![FileTarget {
                method: Some("tcp".to_string()),
                ip: Some("1.1.1.1".to_string()),
                port: Some(443),
                ..FileTarget::default()
            }])),
            overrides(),
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("targets[0].ip"), "unexpected: {message}");
        assert!(message.contains("does not apply"), "unexpected: {message}");
    }

    #[test]
    fn a_missing_key_is_named() {
        let error = Config::resolve(
            Some(file(vec![FileTarget {
                method: Some("tcp".to_string()),
                host: Some("1.1.1.1".to_string()),
                ..FileTarget::default()
            }])),
            overrides(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("targets[0].port"));
    }

    #[test]
    fn per_target_timeout_and_interval_override_the_globals() {
        let config = resolve(Some(file(vec![FileTarget {
            ip: Some("1.1.1.1".to_string()),
            timeout_ms: Some(1_500),
            interval_ms: Some(1_000),
            ..FileTarget::default()
        }])));
        assert_eq!(config.targets[0].timeout, Duration::from_millis(1_500));
        assert_eq!(config.targets[0].interval, Duration::from_millis(1_000));
        assert_eq!(config.timeout, Duration::from_millis(DEFAULT_TIMEOUT_MS));
    }

    #[test]
    fn a_zero_per_target_timeout_is_refused() {
        let error = Config::resolve(
            Some(file(vec![FileTarget {
                ip: Some("1.1.1.1".to_string()),
                timeout_ms: Some(0),
                ..FileTarget::default()
            }])),
            overrides(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("targets[0].timeout_ms"));
    }

    #[test]
    fn interface_is_refused_for_dns_and_http() {
        for target in [
            FileTarget {
                method: Some("dns".to_string()),
                name: Some("example.com".to_string()),
                interface: Some("eth1".to_string()),
                ..FileTarget::default()
            },
            FileTarget {
                method: Some("http".to_string()),
                url: Some("http://example.com/".to_string()),
                interface: Some("eth1".to_string()),
                ..FileTarget::default()
            },
        ] {
            let error = Config::resolve(Some(file(vec![target])), overrides()).unwrap_err();
            assert!(
                error.to_string().contains("interface"),
                "unexpected: {error}"
            );
        }
    }

    #[test]
    fn duplicate_keys_are_refused() {
        // The same endpoint written twice, and the same URL with the default port stated.
        let error = Config::resolve(
            Some(file(vec![
                FileTarget {
                    method: Some("http".to_string()),
                    url: Some("https://example.com/".to_string()),
                    ..FileTarget::default()
                },
                FileTarget {
                    method: Some("http".to_string()),
                    url: Some("https://example.com:443/".to_string()),
                    ..FileTarget::default()
                },
            ])),
            overrides(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate target"));
    }

    #[test]
    fn parses_the_command_line_grammar() {
        let specs = [
            "icmp:1.1.1.1,interface=eth1,label=dns,timeout_ms=1500,interval_ms=1000",
            "tcp:1.1.1.1:443,label=https",
            "dns:example.com@9.9.9.9,record=AAAA",
            "http:https://example.com/health,expect=204,expect=3xx",
        ];
        let config = Config::resolve(
            None,
            Overrides {
                targets: Some(
                    specs
                        .iter()
                        .map(|spec| TargetSpec::parse_spec(spec).unwrap())
                        .collect(),
                ),
                ..Overrides::default()
            },
        )
        .unwrap();

        assert_eq!(
            keys(&config),
            vec![
                "icmp://1.1.1.1",
                "tcp://1.1.1.1:443",
                "dns://example.com@9.9.9.9",
                "https://example.com/health"
            ]
        );
        assert_eq!(config.targets[0].interface.as_deref(), Some("eth1"));
        assert_eq!(config.targets[0].timeout, Duration::from_millis(1_500));
        assert_eq!(config.targets[0].interval, Duration::from_millis(1_000));
    }

    #[test]
    fn a_bare_address_is_an_icmp_target() {
        let spec = TargetSpec::parse_spec("1.1.1.1,label=cloudflare").unwrap();
        assert_eq!(spec.endpoint.method(), Method::Icmp);
        assert_eq!(spec.endpoint.to_string(), "icmp://1.1.1.1");
        assert_eq!(spec.label.as_deref(), Some("cloudflare"));
    }

    /// A target states where its lane sits on the page, in the file and on the command
    /// line alike; a whole number, negative ones included.
    #[test]
    fn a_target_carries_its_priority() {
        let spec = TargetSpec::parse_spec("1.1.1.1,priority=3").unwrap();
        assert_eq!(spec.priority, Some(3));
        let spec = TargetSpec::parse_spec("1.1.1.1,priority=-2,label=cloudflare").unwrap();
        assert_eq!(spec.priority, Some(-2));
        assert_eq!(spec.label.as_deref(), Some("cloudflare"));
        assert!(TargetSpec::parse_spec("1.1.1.1,priority=high").is_err());
        assert!(TargetSpec::parse_spec("1.1.1.1,priority=1.5").is_err());
        assert!(
            TargetSpec::parse_spec("1.1.1.1,priority=").is_err(),
            "an empty value is refused like every other attribute"
        );

        let file = parse(
            "[[targets]]\nip = \"1.1.1.1\"\npriority = 7\n",
            std::path::Path::new("test.toml"),
        )
        .unwrap();
        let config = Config::resolve(Some(file), Overrides::default()).unwrap();
        assert_eq!(config.targets[0].priority, Some(7));
    }

    /// `--show-config` states the priority the process resolved, and states none when the
    /// target has none.
    #[test]
    fn the_effective_configuration_states_the_priority() {
        let file = parse(
            "[[targets]]\nip = \"1.1.1.1\"\npriority = 4\n\n[[targets]]\nip = \"8.8.8.8\"\n",
            std::path::Path::new("test.toml"),
        )
        .unwrap();
        let config = Config::resolve(Some(file), Overrides::default()).unwrap();
        let rendered = config.to_toml();
        assert!(rendered.contains("priority = 4"), "got {rendered}");
        assert_eq!(
            rendered.matches("priority").count(),
            1,
            "a target without a priority states none: {rendered}"
        );
    }

    #[test]
    fn rejects_unknown_target_attributes() {
        assert!(TargetSpec::parse_spec("1.1.1.1,color=red").is_err());
        assert!(TargetSpec::parse_spec("1.1.1.1,interface=").is_err());
        assert!(TargetSpec::parse_spec("").is_err());
        assert!(TargetSpec::parse_spec("tcp:1.1.1.1:443,record=A").is_err());
        assert!(TargetSpec::parse_spec("ftp:1.1.1.1").is_err());
    }

    #[test]
    fn rejects_ipv6_targets_and_resolvers() {
        assert!(TargetSpec::parse_spec("2001:db8::1").is_err());
        assert!(TargetSpec::parse_spec("dns:example.com@2001:db8::1").is_err());
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
    fn serializes_the_effective_configuration_so_it_parses_back() {
        let config = resolve(Some(file(vec![
            FileTarget {
                ip: Some("1.1.1.1".to_string()),
                interface: Some("eth1".to_string()),
                timeout_ms: Some(1_500),
                ..FileTarget::default()
            },
            FileTarget {
                method: Some("dns".to_string()),
                name: Some("example.com".to_string()),
                resolver: Some("9.9.9.9".to_string()),
                record: Some("AAAA".to_string()),
                ..FileTarget::default()
            },
            FileTarget {
                method: Some("http".to_string()),
                url: Some("https://example.com/health?deep=1".to_string()),
                expect_status: Some(vec!["204".to_string(), "3xx".to_string()]),
                label: Some("site".to_string()),
                ..FileTarget::default()
            },
        ])));

        let text = config.to_toml();
        assert!(text.contains("[[targets]]"));
        assert!(text.contains("method = \"dns\""));
        assert!(text.contains("timeout_ms = 1500"));

        let parsed = parse(&text, Path::new("roundtrip.toml")).unwrap();
        let again = resolve(Some(parsed));
        assert_eq!(keys(&again), keys(&config));
        assert_eq!(again.targets[0].timeout, Duration::from_millis(1_500));
        assert_eq!(again.targets[1].endpoint, config.targets[1].endpoint);
        assert_eq!(again.targets[2].endpoint, config.targets[2].endpoint);
    }

    #[test]
    fn the_rendered_configuration_redacts_the_token() {
        let config = resolve(Some(with_telegram("123456:secret", "-100200", None)));

        let text = config.to_toml();
        assert!(!text.contains("secret"), "the token leaked: {text}");
        assert!(text.contains(REDACTED_TOKEN), "unexpected: {text}");
        assert!(text.contains("chat_id = \"-100200\""), "unexpected: {text}");

        // The round trip keeps everything except the secret, which reads the placeholder.
        let parsed = parse(&text, Path::new("roundtrip.toml")).unwrap();
        assert_eq!(
            parsed.telegram.as_ref().unwrap().bot_token.as_deref(),
            Some(REDACTED_TOKEN)
        );
        assert_eq!(
            parsed.telegram.as_ref().unwrap().chat_id.as_deref(),
            Some("-100200")
        );
    }

    #[test]
    fn a_telegram_section_is_resolved() {
        let config = resolve(Some(with_telegram("123456:abc-def", "-100200", Some(15))));
        let telegram = config.telegram.expect("the section is present");
        assert_eq!(telegram.bot_token, "123456:abc-def");
        assert_eq!(telegram.chat_id, "-100200");
        assert_eq!(telegram.stabilize, Duration::from_secs(15));
    }

    #[test]
    fn a_telegram_section_defaults_to_a_minute_and_accepts_zero() {
        let default = resolve(Some(with_telegram("123456:abc-def", "-100200", None)));
        assert_eq!(
            default.telegram.unwrap().stabilize,
            Duration::from_secs(DEFAULT_STABILIZE_SECONDS)
        );

        let immediate = resolve(Some(with_telegram("123456:abc-def", "-100200", Some(0))));
        assert_eq!(immediate.telegram.unwrap().stabilize, Duration::ZERO);
    }

    #[test]
    fn an_absent_telegram_section_configures_nothing() {
        assert!(resolve(None).telegram.is_none());
    }

    #[test]
    fn a_telegram_section_missing_a_key_is_named() {
        let no_token = Config::resolve(
            Some(file_with_telegram(FileTelegram {
                chat_id: Some("-100200".to_string()),
                ..FileTelegram::default()
            })),
            overrides(),
        )
        .unwrap_err();
        assert!(no_token.to_string().contains("telegram.bot_token"));

        let no_chat = Config::resolve(
            Some(file_with_telegram(FileTelegram {
                bot_token: Some("123456:abc-def".to_string()),
                ..FileTelegram::default()
            })),
            overrides(),
        )
        .unwrap_err();
        assert!(no_chat.to_string().contains("telegram.chat_id"));
    }

    #[test]
    fn a_token_with_a_path_separator_is_refused() {
        for token in ["123/456", "123?456", "123#456", "123 456"] {
            let error = Config::resolve(Some(with_telegram(token, "-100200", None)), overrides())
                .unwrap_err();
            assert!(
                error.to_string().contains("telegram.bot_token"),
                "{token}: {error}"
            );
        }
    }

    #[test]
    fn the_debug_form_of_the_section_hides_the_token() {
        let telegram = resolve(Some(with_telegram("123456:secret", "-100200", None)))
            .telegram
            .unwrap();
        let rendered = format!("{telegram:?}");
        assert!(!rendered.contains("secret"), "the token leaked: {rendered}");
        assert!(rendered.contains(REDACTED_TOKEN), "unexpected: {rendered}");
    }

    #[test]
    fn unknown_file_keys_are_rejected() {
        let parsed = toml::from_str::<FileConfig>("interval_ms = 1000\nunknown = 1\n");
        assert!(parsed.is_err());
    }
}
