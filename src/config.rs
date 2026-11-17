//! Configuration: defaults, TOML file, command-line overrides.
//!
//! The merge order is fixed by BR-3 and INV-2: command line, then file, then the
//! built-in default. [`Config::resolve`] is the only function that knows that order.
//!
//! A target is a [`Method`](crate::probe::Method) plus its parameters, and every common
//! property — `interface`, `label`, `timeout_ms`, `interval_ms` — is optional per target.
//! [`TargetSpec`] is what the file or the command line states; [`Target`] is the same
//! target with the global defaults applied, and its key is its identity in the database.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::logging::Level;
use crate::notify::message::Kind;
use crate::probe::http::{parse_url, UrlParts};
use crate::probe::{Endpoint, Method, RecordType, Scheme, StatusMatch};

pub const DEFAULT_CONFIG_FILE: &str = "network-monitor.toml";
pub const DEFAULT_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 2_000;
pub const DEFAULT_DATABASE: &str = "network-monitor.db";
pub const DEFAULT_RETENTION_DAYS: u64 = 0;
pub const DEFAULT_STABILIZE_SECONDS: u64 = 60;

/// The default of a `logfile` channel: append for ever, as before rotation existed.
pub const DEFAULT_MAX_BYTES: u64 = 0;
/// How many rotated files a rotating `logfile` channel keeps by default.
pub const DEFAULT_MAX_FILES: u64 = 5;
/// The bounds of a size that enables rotation: below 1 KiB is useless, above 1 GiB is
/// a file no monitor should hold.
pub const MAX_BYTES_RANGE: std::ops::RangeInclusive<u64> = 1024..=1_073_741_824;
/// The bounds of the number of rotated files kept.
pub const MAX_FILES_RANGE: std::ops::RangeInclusive<u64> = 1..=100;

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
    /// The channels enabled for global events and for every target that does not
    /// state its own list.
    pub notify: Option<Vec<String>>,
    pub notification: Option<Vec<FileNotification>>,
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

/// One `[[notification]]` declaration: a channel, declared once and enabled by name.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileNotification {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub stabilize_seconds: Option<u64>,
    pub bot_token: Option<String>,
    pub chat_id: Option<String>,
    pub url: Option<String>,
    pub token: Option<String>,
    pub filename: Option<String>,
    pub logger: Option<String>,
    pub level: Option<String>,
    pub format: Option<String>,
    /// Rotate the file once it would grow past this many bytes; `0` never rotates.
    pub max_bytes: Option<u64>,
    /// How many rotated files are kept, `<filename>.1` … `<filename>.<max_files>`.
    pub max_files: Option<u64>,
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
    /// The channels this target enables instead of the root list.
    pub notify: Option<Vec<String>>,
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
    pub notify: Option<Vec<String>>,
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
            notify: None,
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
            notify: clean_list(target.notify),
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
    pub notify: NotificationConfig,
    pub targets: Vec<Target>,
}

/// The notification settings: the declared channels and the routing lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationConfig {
    pub channels: Vec<ChannelSettings>,
    /// The channels enabled for global events and as the default for every target.
    pub global: Vec<String>,
    /// The effective list of every resolved target, keyed by its target key.
    pub by_target: HashMap<String, Vec<String>>,
}

impl NotificationConfig {
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    pub fn channel(&self, name: &str) -> Option<&ChannelSettings> {
        self.channels.iter().find(|channel| channel.name() == name)
    }
}

/// The configuration of one declared channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelSettings {
    Telegram(Telegram),
    Webhook(Webhook),
    LogFile(LogFile),
}

impl ChannelSettings {
    pub fn name(&self) -> &str {
        match self {
            ChannelSettings::Telegram(settings) => &settings.name,
            ChannelSettings::Webhook(settings) => &settings.name,
            ChannelSettings::LogFile(settings) => &settings.name,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            ChannelSettings::Telegram(_) => Kind::Telegram,
            ChannelSettings::Webhook(_) => Kind::Webhook,
            ChannelSettings::LogFile(_) => Kind::LogFile,
        }
    }

    pub fn stabilize(&self) -> Duration {
        match self {
            ChannelSettings::Telegram(settings) => settings.stabilize,
            ChannelSettings::Webhook(settings) => settings.stabilize,
            ChannelSettings::LogFile(settings) => settings.stabilize,
        }
    }

    /// The real output destination, which makes a channel unique (INV-14).
    pub fn destination(&self) -> Destination {
        match self {
            ChannelSettings::Telegram(settings) => {
                Destination::TelegramChat(settings.chat_id.clone())
            }
            ChannelSettings::Webhook(settings) => Destination::Url(canonical_url(&settings.url)),
            ChannelSettings::LogFile(settings) => Destination::File(settings.filename.clone()),
        }
    }
}

/// Where a channel writes. Two channels may not share one.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Destination {
    TelegramChat(String),
    /// The canonical URL, including the path and the query.
    Url(String),
    File(PathBuf),
}

impl std::fmt::Display for Destination {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Destination::TelegramChat(chat) => write!(formatter, "chat {chat}"),
            Destination::Url(url) => formatter.write_str(host_only(url)),
            Destination::File(path) => write!(formatter, "{}", path.display()),
        }
    }
}

impl std::fmt::Debug for Destination {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// The effective Telegram settings: where to post, and how long `up` must hold.
#[derive(Clone, PartialEq, Eq)]
pub struct Telegram {
    pub name: String,
    pub bot_token: String,
    pub chat_id: String,
    pub stabilize: Duration,
}

/// The token is a secret: it must never reach a log line through a derived `Debug`.
impl std::fmt::Debug for Telegram {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Telegram")
            .field("name", &self.name)
            .field("bot_token", &REDACTED_TOKEN)
            .field("chat_id", &self.chat_id)
            .field("stabilize", &self.stabilize)
            .finish()
    }
}

/// The effective webhook settings: the endpoint, an optional bearer token, and the window.
#[derive(Clone, PartialEq, Eq)]
pub struct Webhook {
    pub name: String,
    pub url: UrlParts,
    pub token: Option<String>,
    pub stabilize: Duration,
}

impl std::fmt::Debug for Webhook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Webhook")
            .field("name", &self.name)
            .field("url", &host_only(&canonical_url(&self.url)))
            .field(
                "token",
                &if self.token.is_some() {
                    REDACTED_TOKEN
                } else {
                    "none"
                },
            )
            .field("stabilize", &self.stabilize)
            .finish()
    }
}

/// The effective log file settings: where to write, under which logger, at which level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    pub name: String,
    pub filename: PathBuf,
    pub logger: String,
    pub level: Level,
    pub format: LogFormat,
    pub stabilize: Duration,
    /// `0` disables rotation.
    pub max_bytes: u64,
    pub max_files: u64,
}

/// How a log file renders one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Text,
    Jsonl,
}

impl LogFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            LogFormat::Text => "text",
            LogFormat::Jsonl => "jsonl",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "text" => Some(LogFormat::Text),
            "jsonl" => Some(LogFormat::Jsonl),
            _ => None,
        }
    }
}

/// The canonical identity of a webhook URL: lower-case scheme and host, the resolved
/// port, the path and the query.
fn canonical_url(parts: &UrlParts) -> String {
    format!(
        "{}://{}:{}{}",
        parts.scheme.as_str(),
        parts.host.to_ascii_lowercase(),
        parts.port,
        parts.target
    )
}

/// The scheme, host and port of a URL, without the path a secret may live in.
fn host_only(url: &str) -> &str {
    match url.find("://") {
        Some(scheme_end) => {
            let rest = &url[scheme_end + 3..];
            match rest.find('/') {
                Some(path_start) => &url[..scheme_end + 3 + path_start],
                None => url,
            }
        }
        None => url,
    }
}

/// A URL as `--show-config` writes it: the default port is omitted.
fn url_text(parts: &UrlParts) -> String {
    if parts.port == parts.scheme.default_port() {
        format!("{}://{}{}", parts.scheme.as_str(), parts.host, parts.target)
    } else {
        format!(
            "{}://{}:{}{}",
            parts.scheme.as_str(),
            parts.host,
            parts.port,
            parts.target
        )
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

        let file_targets = file.targets;
        let specs = match overrides.targets {
            Some(specs) => specs,
            None => file_targets
                .as_ref()
                .map(|targets| {
                    targets
                        .iter()
                        .cloned()
                        .enumerate()
                        .map(|(index, target)| TargetSpec::from_file(target, index))
                        .collect()
                })
                .transpose()?
                .unwrap_or_default(),
        };
        let spec_notify: Vec<(String, Option<Vec<String>>)> = specs
            .iter()
            .map(|spec| (spec.endpoint.to_string(), spec.notify.clone()))
            .collect();

        let interval = Duration::from_millis(interval_ms);
        let timeout = Duration::from_millis(timeout_ms);
        let targets = specs
            .into_iter()
            .enumerate()
            .map(|(index, spec)| Target::resolve(spec, index, timeout, interval))
            .collect::<Result<Vec<_>, _>>()?;
        ensure_unique(&targets)?;
        let notify = resolve_notifications(
            file.notification.as_deref(),
            file.telegram,
            file.notify,
            file_targets.as_deref(),
            &spec_notify,
            &targets,
        )?;

        Ok(Config {
            interval,
            timeout,
            database,
            retention_days,
            notify,
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
            notify: Vec<String>,
            notification: Vec<SerializableNotification>,
            targets: Vec<SerializableTarget>,
        }

        #[derive(Serialize, Default)]
        struct SerializableNotification {
            name: String,
            #[serde(rename = "type")]
            kind: String,
            stabilize_seconds: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            bot_token: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            chat_id: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            url: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            token: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            filename: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            logger: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            level: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            format: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_bytes: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_files: Option<u64>,
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
            #[serde(skip_serializing_if = "Option::is_none")]
            notify: Option<Vec<String>>,
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
                if let Some(effective) = self.notify.by_target.get(&target.key()) {
                    if *effective != self.notify.global {
                        serializable.notify = Some(effective.clone());
                    }
                }
                serializable
            })
            .collect();

        let notification = self
            .notify
            .channels
            .iter()
            .map(|channel| match channel {
                ChannelSettings::Telegram(settings) => SerializableNotification {
                    name: settings.name.clone(),
                    kind: "telegram".to_string(),
                    stabilize_seconds: settings.stabilize.as_secs(),
                    bot_token: Some(REDACTED_TOKEN.to_string()),
                    chat_id: Some(settings.chat_id.clone()),
                    ..SerializableNotification::default()
                },
                ChannelSettings::Webhook(settings) => SerializableNotification {
                    name: settings.name.clone(),
                    kind: "webhook".to_string(),
                    stabilize_seconds: settings.stabilize.as_secs(),
                    url: Some(url_text(&settings.url)),
                    token: settings.token.as_ref().map(|_| REDACTED_TOKEN.to_string()),
                    ..SerializableNotification::default()
                },
                ChannelSettings::LogFile(settings) => SerializableNotification {
                    name: settings.name.clone(),
                    kind: "logfile".to_string(),
                    stabilize_seconds: settings.stabilize.as_secs(),
                    filename: Some(settings.filename.to_string_lossy().into_owned()),
                    logger: Some(settings.logger.clone()),
                    level: Some(settings.level.label().to_ascii_lowercase()),
                    format: Some(settings.format.as_str().to_string()),
                    // A channel that does not rotate keeps printing exactly what it
                    // printed before the keys existed.
                    max_bytes: (settings.max_bytes > 0).then_some(settings.max_bytes),
                    max_files: (settings.max_bytes > 0).then_some(settings.max_files),
                    ..SerializableNotification::default()
                },
            })
            .collect();

        let serializable = SerializableConfig {
            interval_ms: self.interval.as_millis() as u64,
            timeout_ms: self.timeout.as_millis() as u64,
            database: self.database.to_string_lossy().into_owned(),
            retention_days: self.retention_days,
            notify: self.notify.global.clone(),
            notification,
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
fn resolve_telegram(file: FileTelegram) -> Result<Telegram, ConfigError> {
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

    Ok(Telegram {
        name: "telegram".to_string(),
        bot_token: bot_token.to_string(),
        chat_id: chat_id.to_string(),
        stabilize: Duration::from_secs(file.stabilize_seconds.unwrap_or(DEFAULT_STABILIZE_SECONDS)),
    })
}

/// Resolve the declared channels and the routing lists into one effective configuration.
///
/// No entry declares a scope: a channel is declared once and enabled by name, globally
/// or per target. Two channels may not share an output destination, because then one
/// event would reach that destination twice (INV-14).
fn resolve_notifications(
    declarations: Option<&[FileNotification]>,
    legacy: Option<FileTelegram>,
    root: Option<Vec<String>>,
    file_targets: Option<&[FileTarget]>,
    spec_notify: &[(String, Option<Vec<String>>)],
    targets: &[Target],
) -> Result<NotificationConfig, ConfigError> {
    let mut channels: Vec<ChannelSettings> = Vec::new();
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut destinations: HashMap<Destination, String> = HashMap::new();

    for (index, declaration) in declarations.unwrap_or(&[]).iter().enumerate() {
        let channel = resolve_channel(declaration, index)?;
        let name = channel.name().to_string();
        if let Some(previous) = names.insert(name.clone(), index) {
            return Err(invalid(
                format!("notification[{index}].name"),
                format!("duplicate channel name '{name}' (already declared by notification[{previous}])"),
            ));
        }
        let destination = channel.destination();
        if let Some(previous) = destinations.insert(destination.clone(), name.clone()) {
            return Err(invalid(
                format!("notification[{index}]"),
                format!("channels '{previous}' and '{name}' both write to {destination}"),
            ));
        }
        channels.push(channel);
    }

    let mut legacy_name = None;
    if let Some(legacy) = legacy {
        let channel = ChannelSettings::Telegram(resolve_telegram(legacy)?);
        let name = channel.name().to_string();
        if names.contains_key(&name) {
            return Err(invalid(
                "telegram",
                format!("the channel name '{name}' is already declared in [[notification]]"),
            ));
        }
        let destination = channel.destination();
        if let Some(previous) = destinations.insert(destination.clone(), name.clone()) {
            return Err(invalid(
                "telegram.chat_id",
                format!("channels '{previous}' and '{name}' both write to {destination}"),
            ));
        }
        names.insert(name.clone(), channels.len());
        legacy_name = Some(name);
        channels.push(channel);
    }

    let mut global = clean_list(root).unwrap_or_default();
    if let Some(name) = &legacy_name {
        if !global.contains(name) {
            global.push(name.clone());
        }
    }
    dedupe(&mut global);

    for name in &global {
        if !names.contains_key(name) {
            return Err(invalid(
                "notify",
                format!("unknown notification channel '{name}'"),
            ));
        }
    }

    for (index, target) in file_targets.unwrap_or(&[]).iter().enumerate() {
        for name in clean_list(target.notify.clone()).unwrap_or_default() {
            if !names.contains_key(&name) {
                return Err(invalid(
                    format!("targets[{index}].notify"),
                    format!("unknown notification channel '{name}'"),
                ));
            }
        }
    }

    // A declared channel that nothing enables is legal: it starts, states `CHANNEL` on
    // itself, and receives nothing. The caller reports it as a `WARN`, never as a reason
    // to refuse the configuration.

    let mut by_target = HashMap::new();
    for target in targets {
        let key = target.key();
        let override_list = spec_notify
            .iter()
            .find(|(spec_key, _)| *spec_key == key)
            .and_then(|(_, list)| list.clone());
        let mut effective = clean_list(override_list).unwrap_or_else(|| global.clone());
        dedupe(&mut effective);
        for name in &effective {
            if !names.contains_key(name) {
                return Err(invalid(
                    format!("targets[{key}].notify"),
                    format!("unknown notification channel '{name}'"),
                ));
            }
        }
        by_target.insert(key, effective);
    }

    Ok(NotificationConfig {
        channels,
        global,
        by_target,
    })
}

/// Resolve one `[[notification]]` entry into the settings of its type.
fn resolve_channel(file: &FileNotification, index: usize) -> Result<ChannelSettings, ConfigError> {
    let key = |name: &str| format!("notification[{index}].{name}");

    let name = required(file.name.as_deref(), &key("name"))?.to_string();
    if !valid_channel_name(&name) {
        return Err(invalid(
            key("name"),
            "may contain only ASCII letters, digits, '.', '_' and '-'",
        ));
    }
    let kind = required(file.kind.as_deref(), &key("type"))?;
    let stabilize =
        Duration::from_secs(file.stabilize_seconds.unwrap_or(DEFAULT_STABILIZE_SECONDS));

    match kind {
        "telegram" => {
            reject_foreign(file, index, &["bot_token", "chat_id"])?;
            let bot_token = required(file.bot_token.as_deref(), &key("bot_token"))?;
            if bot_token
                .chars()
                .any(|character| character.is_whitespace() || matches!(character, '/' | '?' | '#'))
            {
                return Err(invalid(
                    key("bot_token"),
                    "must not contain whitespace, '/', '?' or '#'",
                ));
            }
            let chat_id = required(file.chat_id.as_deref(), &key("chat_id"))?;
            Ok(ChannelSettings::Telegram(Telegram {
                name,
                bot_token: bot_token.to_string(),
                chat_id: chat_id.to_string(),
                stabilize,
            }))
        }
        "webhook" => {
            reject_foreign(file, index, &["url", "token"])?;
            let url = required(file.url.as_deref(), &key("url"))?;
            let parts = parse_url(url).map_err(|reason| invalid(key("url"), reason))?;
            let token = clean(file.token.clone());
            if token.is_some() && parts.scheme == Scheme::Http {
                return Err(invalid(
                    key("token"),
                    "requires an https URL, so the token is not sent in clear",
                ));
            }
            Ok(ChannelSettings::Webhook(Webhook {
                name,
                url: parts,
                token,
                stabilize,
            }))
        }
        "logfile" => {
            reject_foreign(
                file,
                index,
                &[
                    "filename",
                    "logger",
                    "level",
                    "format",
                    "max_bytes",
                    "max_files",
                ],
            )?;
            let filename = required(file.filename.as_deref(), &key("filename"))?;
            let filename = normalise_path(Path::new(filename));
            let logger = clean(file.logger.clone()).unwrap_or_else(|| "notification".to_string());
            if !valid_channel_name(&logger) {
                return Err(invalid(
                    key("logger"),
                    "may contain only ASCII letters, digits, '.', '_' and '-'",
                ));
            }
            let level = match clean(file.level.clone()) {
                None => Level::Info,
                Some(text) => Level::parse(&text)
                    .ok_or_else(|| invalid(key("level"), "must be error, warn, info or debug"))?,
            };
            let format = match clean(file.format.clone()) {
                None => LogFormat::Text,
                Some(text) => LogFormat::parse(&text)
                    .ok_or_else(|| invalid(key("format"), "must be text or jsonl"))?,
            };
            let max_bytes = file.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
            if max_bytes != 0 && !MAX_BYTES_RANGE.contains(&max_bytes) {
                return Err(invalid(
                    key("max_bytes"),
                    "must be 0, or between 1024 and 1073741824",
                ));
            }
            let max_files = file.max_files.unwrap_or(DEFAULT_MAX_FILES);
            if !MAX_FILES_RANGE.contains(&max_files) {
                return Err(invalid(key("max_files"), "must be between 1 and 100"));
            }
            Ok(ChannelSettings::LogFile(LogFile {
                name,
                filename,
                logger,
                level,
                format,
                stabilize,
                max_bytes,
                max_files,
            }))
        }
        other => Err(invalid(
            key("type"),
            format!("must be telegram, webhook or logfile, not '{other}'"),
        )),
    }
}

/// A key of another channel type is refused rather than ignored.
fn reject_foreign(
    file: &FileNotification,
    index: usize,
    allowed: &[&str],
) -> Result<(), ConfigError> {
    for (name, present) in [
        ("bot_token", file.bot_token.is_some()),
        ("chat_id", file.chat_id.is_some()),
        ("url", file.url.is_some()),
        ("token", file.token.is_some()),
        ("filename", file.filename.is_some()),
        ("logger", file.logger.is_some()),
        ("level", file.level.is_some()),
        ("format", file.format.is_some()),
        ("max_bytes", file.max_bytes.is_some()),
        ("max_files", file.max_files.is_some()),
    ] {
        if present && !allowed.contains(&name) {
            return Err(invalid(
                format!("notification[{index}].{name}"),
                "does not apply to this channel type",
            ));
        }
    }
    Ok(())
}

fn valid_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

/// Trim every entry and drop the empty ones; `Some([])` stays `Some([])`, because an
/// empty target list silences the target.
fn clean_list(values: Option<Vec<String>>) -> Option<Vec<String>> {
    values.map(|values| {
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    })
}

/// Keep the first occurrence of every name, preserving the order.
fn dedupe(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

/// Make a filename absolute against the working directory and fold `.` and `..`
/// lexically. Symlinks are not resolved: that would depend on the filesystem.
fn normalise_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut normal = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other.as_os_str()),
        }
    }
    normal
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
        assert!(text.contains("[[notification]]"), "unexpected: {text}");
        assert!(text.contains("chat_id = \"-100200\""), "unexpected: {text}");

        // The round trip keeps everything except the secret, which reads the placeholder.
        let parsed = parse(&text, Path::new("roundtrip.toml")).unwrap();
        let declaration = &parsed.notification.as_ref().unwrap()[0];
        assert_eq!(declaration.bot_token.as_deref(), Some(REDACTED_TOKEN));
        assert_eq!(declaration.chat_id.as_deref(), Some("-100200"));
    }

    #[test]
    fn a_telegram_section_becomes_a_declared_channel() {
        let config = resolve(Some(with_telegram("123456:abc-def", "-100200", Some(15))));
        let channel = config
            .notify
            .channel("telegram")
            .expect("the channel exists");
        match channel {
            ChannelSettings::Telegram(telegram) => {
                assert_eq!(telegram.bot_token, "123456:abc-def");
                assert_eq!(telegram.chat_id, "-100200");
                assert_eq!(telegram.stabilize, Duration::from_secs(15));
            }
            other => panic!("not a telegram channel: {other:?}"),
        }
        assert_eq!(config.notify.global, vec!["telegram".to_string()]);
    }

    #[test]
    fn a_telegram_section_defaults_to_a_minute_and_accepts_zero() {
        let default = resolve(Some(with_telegram("123456:abc-def", "-100200", None)));
        match default.notify.channel("telegram").unwrap() {
            ChannelSettings::Telegram(telegram) => {
                assert_eq!(
                    telegram.stabilize,
                    Duration::from_secs(DEFAULT_STABILIZE_SECONDS)
                )
            }
            other => panic!("not a telegram channel: {other:?}"),
        }

        let immediate = resolve(Some(with_telegram("123456:abc-def", "-100200", Some(0))));
        match immediate.notify.channel("telegram").unwrap() {
            ChannelSettings::Telegram(telegram) => {
                assert_eq!(telegram.stabilize, Duration::ZERO)
            }
            other => panic!("not a telegram channel: {other:?}"),
        }
    }

    #[test]
    fn an_absent_telegram_section_configures_nothing() {
        let config = resolve(None);
        assert!(config.notify.channels.is_empty());
        assert!(config.notify.global.is_empty());
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
    fn the_debug_form_of_the_channel_hides_the_token() {
        let config = resolve(Some(with_telegram("123456:secret", "-100200", None)));
        let rendered = format!("{:?}", config.notify.channel("telegram").unwrap());
        assert!(!rendered.contains("secret"), "the token leaked: {rendered}");
        assert!(rendered.contains(REDACTED_TOKEN), "unexpected: {rendered}");
    }

    fn declaration(name: &str, kind: &str) -> FileNotification {
        FileNotification {
            name: Some(name.to_string()),
            kind: Some(kind.to_string()),
            ..FileNotification::default()
        }
    }

    fn logfile(name: &str, filename: &str, format: Option<&str>) -> FileNotification {
        FileNotification {
            name: Some(name.to_string()),
            kind: Some("logfile".to_string()),
            filename: Some(filename.to_string()),
            format: format.map(str::to_string),
            ..FileNotification::default()
        }
    }

    fn target_with(ip: &str, notify: Option<Vec<&str>>) -> FileTarget {
        FileTarget {
            ip: Some(ip.to_string()),
            notify: notify.map(|names| names.into_iter().map(str::to_string).collect()),
            ..FileTarget::default()
        }
    }

    #[test]
    fn a_channel_is_enabled_by_name_globally_and_per_target() {
        let file = FileConfig {
            notification: Some(vec![
                logfile("audit", "/tmp/audit.log", None),
                logfile("debug", "/tmp/debug.log", Some("jsonl")),
            ]),
            notify: Some(vec!["audit".to_string()]),
            targets: Some(vec![
                target_with("1.1.1.1", None),
                target_with("8.8.8.8", Some(vec!["audit", "debug"])),
                target_with("9.9.9.9", Some(vec![])),
            ]),
            ..FileConfig::default()
        };

        let config = resolve(Some(file));
        assert_eq!(config.notify.global, vec!["audit".to_string()]);
        assert_eq!(config.notify.by_target["icmp://1.1.1.1"], vec!["audit"]);
        assert_eq!(
            config.notify.by_target["icmp://8.8.8.8"],
            vec!["audit", "debug"]
        );
        assert!(config.notify.by_target["icmp://9.9.9.9"].is_empty());
    }

    #[test]
    fn an_unknown_name_is_refused() {
        let file = FileConfig {
            notification: Some(vec![logfile("audit", "/tmp/a.log", None)]),
            notify: Some(vec!["missing".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        };
        let error = Config::resolve(Some(file), overrides()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unknown notification channel 'missing'"),
            "{error}"
        );
    }

    #[test]
    fn a_declared_channel_that_nothing_enables_is_accepted() {
        let file = FileConfig {
            notification: Some(vec![logfile("audit", "/tmp/a.log", None)]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        };
        let config = Config::resolve(Some(file), overrides()).expect("the channel is only unused");
        assert_eq!(config.notify.channels.len(), 1);
        assert!(config.notify.global.is_empty());
        assert!(config.notify.by_target["icmp://1.1.1.1"].is_empty());
    }

    #[test]
    fn two_channels_may_not_share_a_destination() {
        for (one, two) in [("/tmp/a.log", "/tmp/a.log"), ("./tmp/a.log", "tmp/a.log")] {
            let file = FileConfig {
                notification: Some(vec![logfile("one", one, None), logfile("two", two, None)]),
                notify: Some(vec!["one".to_string(), "two".to_string()]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            };
            let error = Config::resolve(Some(file), overrides()).unwrap_err();
            assert!(error.to_string().contains("both write to"), "{error}");
        }
    }

    #[test]
    fn a_duplicate_name_is_refused() {
        let file = FileConfig {
            notification: Some(vec![
                logfile("audit", "/tmp/a.log", None),
                logfile("audit", "/tmp/b.log", None),
            ]),
            notify: Some(vec!["audit".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        };
        let error = Config::resolve(Some(file), overrides()).unwrap_err();
        assert!(
            error.to_string().contains("duplicate channel name"),
            "{error}"
        );
    }

    #[test]
    fn a_webhook_token_requires_https() {
        let file = FileConfig {
            notification: Some(vec![FileNotification {
                url: Some("http://example.test/hook".to_string()),
                token: Some("secret".to_string()),
                ..declaration("hook", "webhook")
            }]),
            notify: Some(vec!["hook".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        };
        let error = Config::resolve(Some(file), overrides()).unwrap_err();
        assert!(
            error.to_string().contains("requires an https URL"),
            "{error}"
        );
    }

    #[test]
    fn a_logfile_carries_its_logger_level_and_format() {
        let file = FileConfig {
            notification: Some(vec![FileNotification {
                filename: Some("/tmp/a.log".to_string()),
                logger: Some("audit-log".to_string()),
                level: Some("debug".to_string()),
                format: Some("jsonl".to_string()),
                ..declaration("audit", "logfile")
            }]),
            notify: Some(vec!["audit".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        };
        let config = resolve(Some(file));
        match config.notify.channel("audit").unwrap() {
            ChannelSettings::LogFile(logfile) => {
                assert_eq!(logfile.logger, "audit-log");
                assert_eq!(logfile.level, Level::Debug);
                assert_eq!(logfile.format, LogFormat::Jsonl);
                assert_eq!(logfile.max_bytes, DEFAULT_MAX_BYTES);
                assert_eq!(logfile.max_files, DEFAULT_MAX_FILES);
            }
            other => panic!("not a log file: {other:?}"),
        }
    }

    fn logfile_with(
        name: &str,
        max_bytes: Option<u64>,
        max_files: Option<u64>,
    ) -> FileNotification {
        FileNotification {
            max_bytes,
            max_files,
            ..logfile(name, "/tmp/a.log", None)
        }
    }

    fn resolve_logfile(declaration: FileNotification) -> Result<Config, ConfigError> {
        Config::resolve(
            Some(FileConfig {
                notification: Some(vec![declaration]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            }),
            overrides(),
        )
    }

    fn max_bytes_of(config: &Config) -> u64 {
        match config.notify.channel("audit").unwrap() {
            ChannelSettings::LogFile(logfile) => logfile.max_bytes,
            other => panic!("not a log file: {other:?}"),
        }
    }

    fn max_files_of(config: &Config) -> u64 {
        match config.notify.channel("audit").unwrap() {
            ChannelSettings::LogFile(logfile) => logfile.max_files,
            other => panic!("not a log file: {other:?}"),
        }
    }

    #[test]
    fn the_rotation_keys_accept_zero_and_both_bounds() {
        let off = resolve_logfile(logfile_with("audit", Some(0), Some(1))).unwrap();
        assert_eq!(max_bytes_of(&off), 0);
        assert_eq!(max_files_of(&off), 1);

        for bytes in [1024, 1_073_741_824] {
            let config = resolve_logfile(logfile_with("audit", Some(bytes), Some(100))).unwrap();
            assert_eq!(max_bytes_of(&config), bytes);
            assert_eq!(max_files_of(&config), 100);
        }
    }

    #[test]
    fn an_out_of_range_rotation_key_is_refused_with_its_name() {
        for bytes in [1, 1023, 1_073_741_825] {
            let error = resolve_logfile(logfile_with("audit", Some(bytes), None)).unwrap_err();
            assert!(
                error.to_string().contains("notification[0].max_bytes"),
                "{bytes}: {error}"
            );
        }

        for files in [0, 101] {
            let error =
                resolve_logfile(logfile_with("audit", Some(4096), Some(files))).unwrap_err();
            assert!(
                error.to_string().contains("notification[0].max_files"),
                "{files}: {error}"
            );
        }
    }

    #[test]
    fn the_rotation_keys_belong_to_the_log_file_only() {
        let error = Config::resolve(
            Some(FileConfig {
                notification: Some(vec![FileNotification {
                    max_bytes: Some(4096),
                    ..declaration("audit", "webhook")
                }]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            }),
            overrides(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("notification[0].max_bytes: does not apply to this channel type"),
            "{error}"
        );
    }

    #[test]
    fn show_config_prints_the_rotation_keys_only_when_it_rotates() {
        let off = resolve(Some(FileConfig {
            notification: Some(vec![logfile("audit", "/tmp/a.log", None)]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        }));
        let text = off.to_toml();
        assert!(!text.contains("max_bytes"), "{text}");
        assert!(!text.contains("max_files"), "{text}");

        let on = resolve(Some(FileConfig {
            notification: Some(vec![logfile_with("audit", Some(4096), Some(3))]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        }));
        let text = on.to_toml();
        assert!(text.contains("max_bytes = 4096"), "{text}");
        assert!(text.contains("max_files = 3"), "{text}");

        let again = resolve(Some(parse(&text, Path::new("roundtrip.toml")).unwrap()));
        assert_eq!(max_bytes_of(&again), 4096);
        assert_eq!(max_files_of(&again), 3);
    }

    fn webhook(name: &str, url: &str, token: Option<&str>) -> FileNotification {
        FileNotification {
            name: Some(name.to_string()),
            kind: Some("webhook".to_string()),
            url: Some(url.to_string()),
            token: token.map(str::to_string),
            ..FileNotification::default()
        }
    }

    #[test]
    fn a_webhook_carries_its_url_token_and_window() {
        let file = FileConfig {
            notification: Some(vec![FileNotification {
                stabilize_seconds: Some(15),
                ..webhook(
                    "hook",
                    "https://hooks.example.test/notify?room=ops",
                    Some("s3cret"),
                )
            }]),
            notify: Some(vec!["hook".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        };
        let config = resolve(Some(file));
        let channel = config.notify.channel("hook").unwrap();
        match channel {
            ChannelSettings::Webhook(webhook) => {
                assert_eq!(webhook.name, "hook");
                assert_eq!(webhook.token.as_deref(), Some("s3cret"));
                assert_eq!(webhook.url.host, "hooks.example.test");
                assert_eq!(webhook.url.port, 443);
                assert_eq!(webhook.url.target, "/notify?room=ops");
                assert_eq!(webhook.stabilize, Duration::from_secs(15));
            }
            other => panic!("not a webhook: {other:?}"),
        }
        assert_eq!(channel.kind(), Kind::Webhook);

        let without_optional = resolve(Some(FileConfig {
            notification: Some(vec![webhook("hook", "https://hooks.example.test", None)]),
            notify: Some(vec!["hook".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        }));
        match without_optional.notify.channel("hook").unwrap() {
            ChannelSettings::Webhook(webhook) => {
                assert_eq!(webhook.token, None);
                assert_eq!(
                    webhook.stabilize,
                    Duration::from_secs(DEFAULT_STABILIZE_SECONDS)
                );
            }
            other => panic!("not a webhook: {other:?}"),
        }
    }

    #[test]
    fn a_webhook_destination_names_the_host_and_never_the_path() {
        let config = resolve(Some(FileConfig {
            notification: Some(vec![webhook(
                "hook",
                "https://hooks.example.test/secret-path?room=ops",
                None,
            )]),
            notify: Some(vec!["hook".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        }));
        let destination = config.notify.channel("hook").unwrap().destination();
        assert!(
            !destination.to_string().contains("secret-path"),
            "the path leaked: {destination}"
        );
        assert!(destination
            .to_string()
            .starts_with("https://hooks.example.test"));
    }

    #[test]
    fn two_channels_may_not_share_a_url() {
        for (one, two) in [
            (
                "https://hooks.example.test/notify",
                "https://HOOKS.example.test:443/notify",
            ),
            (
                "http://hooks.example.test/room",
                "http://hooks.example.test:80/room",
            ),
        ] {
            let file = FileConfig {
                notification: Some(vec![webhook("one", one, None), webhook("two", two, None)]),
                notify: Some(vec!["one".to_string(), "two".to_string()]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            };
            let error = Config::resolve(Some(file), overrides()).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("both write to"), "{message}");
            assert!(
                !message.contains("/notify") && !message.contains("/room"),
                "the path leaked: {message}"
            );
        }
    }

    #[test]
    fn the_rendered_webhook_omits_the_default_port_and_round_trips() {
        let config = resolve(Some(FileConfig {
            notification: Some(vec![
                webhook(
                    "hook",
                    "https://hooks.example.test/notify?room=ops",
                    Some("s3cret"),
                ),
                webhook("bespoke", "https://hooks.example.test:8443/hook", None),
            ]),
            notify: Some(vec!["hook".to_string(), "bespoke".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        }));
        let text = config.to_toml();
        assert!(
            text.contains("url = \"https://hooks.example.test/notify?room=ops\""),
            "{text}"
        );
        assert!(
            text.contains("url = \"https://hooks.example.test:8443/hook\""),
            "{text}"
        );
        assert!(!text.contains("s3cret"), "the token leaked: {text}");

        // The token is printed as `<redacted>`, so the printed file is not the same
        // secret back: the identity that must survive the round trip is the destination.
        let again = resolve(Some(parse(&text, Path::new("roundtrip.toml")).unwrap()));
        let destinations = |config: &Config| -> Vec<Destination> {
            config
                .notify
                .channels
                .iter()
                .map(ChannelSettings::destination)
                .collect()
        };
        assert_eq!(destinations(&again), destinations(&config));
        assert_eq!(again.notify.global, config.notify.global);
        assert_eq!(again.notify.by_target, config.notify.by_target);
    }

    #[test]
    fn a_key_of_another_channel_type_is_refused() {
        let file = FileConfig {
            notification: Some(vec![FileNotification {
                bot_token: Some("123456:abc-def".to_string()),
                chat_id: Some("-100200".to_string()),
                filename: Some("/tmp/a.log".to_string()),
                ..declaration("mixed", "telegram")
            }]),
            notify: Some(vec!["mixed".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        };
        let error = Config::resolve(Some(file), overrides()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("notification[0].filename: does not apply to this channel type"),
            "{error}"
        );
    }

    #[test]
    fn an_invalid_channel_name_or_logger_is_refused() {
        let bad_name = Config::resolve(
            Some(FileConfig {
                notification: Some(vec![logfile("bad name", "/tmp/a.log", None)]),
                notify: Some(vec!["bad name".to_string()]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            }),
            overrides(),
        )
        .unwrap_err();
        assert!(
            bad_name.to_string().contains("notification[0].name")
                && bad_name.to_string().contains("may contain only ASCII"),
            "{bad_name}"
        );

        let bad_logger = Config::resolve(
            Some(FileConfig {
                notification: Some(vec![FileNotification {
                    logger: Some("bad/logger".to_string()),
                    ..logfile("audit", "/tmp/a.log", None)
                }]),
                notify: Some(vec!["audit".to_string()]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            }),
            overrides(),
        )
        .unwrap_err();
        assert!(
            bad_logger.to_string().contains("notification[0].logger")
                && bad_logger.to_string().contains("may contain only ASCII"),
            "{bad_logger}"
        );
    }

    #[test]
    fn an_unknown_channel_type_is_refused() {
        let file = FileConfig {
            notification: Some(vec![declaration("mail", "email")]),
            notify: Some(vec!["mail".to_string()]),
            targets: Some(vec![icmp("1.1.1.1")]),
            ..FileConfig::default()
        };
        let error = Config::resolve(Some(file), overrides()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must be telegram, webhook or logfile, not 'email'"),
            "{error}"
        );
    }

    #[test]
    fn an_invalid_level_or_format_is_refused() {
        let bad_level = Config::resolve(
            Some(FileConfig {
                notification: Some(vec![FileNotification {
                    level: Some("verbose".to_string()),
                    ..logfile("audit", "/tmp/a.log", None)
                }]),
                notify: Some(vec!["audit".to_string()]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            }),
            overrides(),
        )
        .unwrap_err();
        assert!(
            bad_level
                .to_string()
                .contains("notification[0].level: must be error, warn, info or debug"),
            "{bad_level}"
        );

        let bad_format = Config::resolve(
            Some(FileConfig {
                notification: Some(vec![logfile("audit", "/tmp/a.log", Some("xml"))]),
                notify: Some(vec!["audit".to_string()]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            }),
            overrides(),
        )
        .unwrap_err();
        assert!(
            bad_format
                .to_string()
                .contains("notification[0].format: must be text or jsonl"),
            "{bad_format}"
        );
    }

    #[test]
    fn a_telegram_section_may_not_repeat_a_channel_name_or_a_chat() {
        let repeated_name = Config::resolve(
            Some(FileConfig {
                telegram: Some(FileTelegram {
                    bot_token: Some("123456:abc-def".to_string()),
                    chat_id: Some("-100200".to_string()),
                    stabilize_seconds: None,
                }),
                notification: Some(vec![logfile("telegram", "/tmp/a.log", None)]),
                notify: Some(vec!["telegram".to_string()]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            }),
            overrides(),
        )
        .unwrap_err();
        assert!(
            repeated_name
                .to_string()
                .contains("the channel name 'telegram' is already declared"),
            "{repeated_name}"
        );

        let repeated_chat = Config::resolve(
            Some(FileConfig {
                telegram: Some(FileTelegram {
                    bot_token: Some("123456:abc-def".to_string()),
                    chat_id: Some("-100200".to_string()),
                    stabilize_seconds: None,
                }),
                notification: Some(vec![FileNotification {
                    bot_token: Some("999:xyz".to_string()),
                    chat_id: Some("-100200".to_string()),
                    ..declaration("second", "telegram")
                }]),
                notify: Some(vec!["second".to_string()]),
                targets: Some(vec![icmp("1.1.1.1")]),
                ..FileConfig::default()
            }),
            overrides(),
        )
        .unwrap_err();
        assert!(
            repeated_chat
                .to_string()
                .contains("both write to chat -100200"),
            "{repeated_chat}"
        );
    }

    #[test]
    fn the_rendered_routing_round_trips() {
        let file = FileConfig {
            notification: Some(vec![logfile("audit", "/tmp/a.log", None)]),
            notify: Some(vec!["audit".to_string()]),
            targets: Some(vec![
                target_with("1.1.1.1", None),
                target_with("8.8.8.8", Some(vec![])),
            ]),
            ..FileConfig::default()
        };
        let config = resolve(Some(file));
        let text = config.to_toml();
        let parsed = parse(&text, Path::new("roundtrip.toml")).unwrap();
        let again = resolve(Some(parsed));

        assert_eq!(again.notify.global, config.notify.global);
        assert_eq!(again.notify.by_target, config.notify.by_target);
        assert_eq!(again.notify.channels.len(), config.notify.channels.len());
    }

    #[test]
    fn unknown_file_keys_are_rejected() {
        let parsed = toml::from_str::<FileConfig>("interval_ms = 1000\nunknown = 1\n");
        assert!(parsed.is_err());
    }
}
