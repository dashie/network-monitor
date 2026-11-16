//! Line-based diagnostics on stderr.
//!
//! stdout carries data only: query results must stay pipeable. Every diagnostic is
//! one line, `<RFC 3339> <LEVEL> [<logger>] [<target>] <message>`.

use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::time::{format_rfc3339, now_ms};

/// The logger of the process itself.
pub const MONITOR: &str = "monitor";

/// The logger of the notification diagnostics.
pub const NOTIFICATION: &str = "notification";

/// Diagnostic levels, ordered from the quietest to the noisiest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

impl Level {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "error" => Some(Level::Error),
            "warn" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }

    /// Whether a line of this level passes a configured level.
    ///
    /// The configured level is the most verbose one admitted, so `debug` admits
    /// `error`, `warn`, `info`, and `debug`, and `error` admits only `error`.
    pub fn allows(self, configured: Level) -> bool {
        (self as u8) <= (configured as u8)
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn enabled(level: Level) -> bool {
    (level as u8) <= LEVEL.load(Ordering::Relaxed)
}

/// One diagnostic line, `<timestamp> <LEVEL> <logger> [<target>] <message>`.
///
/// The target bracket is omitted when no target applies.
pub fn format_line_at(
    at_ms: i64,
    level: Level,
    logger: &str,
    target: Option<&str>,
    message: &str,
) -> String {
    let timestamp = format_rfc3339(at_ms);
    match target {
        Some(target) => format!(
            "{timestamp} {:<5} {logger} [{target}] {message}",
            level.label()
        ),
        None => format!("{timestamp} {:<5} {logger} {message}", level.label()),
    }
}

pub fn write(level: Level, target: Option<&str>, message: &str) {
    if !enabled(level) {
        return;
    }
    let line = format_line_at(now_ms(), level, MONITOR, target, message);
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = writeln!(handle, "{line}");
}

/// A notification diagnostic, written under the `notification` logger.
pub fn notification(level: Level, target: Option<&str>, message: &str) {
    if !enabled(level) {
        return;
    }
    let line = format_line_at(now_ms(), level, NOTIFICATION, target, message);
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = writeln!(handle, "{line}");
}

pub fn error(target: Option<&str>, message: &str) {
    write(Level::Error, target, message);
}

pub fn warn(target: Option<&str>, message: &str) {
    write(Level::Warn, target, message);
}

pub fn info(target: Option<&str>, message: &str) {
    write(Level::Info, target, message);
}

pub fn debug(target: Option<&str>, message: &str) {
    write(Level::Debug, target, message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_names_the_logger_and_the_target() {
        assert_eq!(
            format_line_at(
                0,
                Level::Info,
                MONITOR,
                Some("icmp://1.1.1.1"),
                "initialized target"
            ),
            "1970-01-01T00:00:00.000Z INFO  monitor [icmp://1.1.1.1] initialized target"
        );
    }

    #[test]
    fn a_line_without_a_target_has_no_bracket() {
        assert_eq!(
            format_line_at(0, Level::Debug, MONITOR, None, "no configuration file"),
            "1970-01-01T00:00:00.000Z DEBUG monitor no configuration file"
        );
    }

    #[test]
    fn a_notification_line_carries_its_own_logger() {
        assert_eq!(
            format_line_at(0, Level::Warn, NOTIFICATION, Some("icmp://1.1.1.1"), "lost"),
            "1970-01-01T00:00:00.000Z WARN  notification [icmp://1.1.1.1] lost"
        );
    }

    #[test]
    fn the_configured_level_is_the_most_verbose_one_admitted() {
        assert!(Level::Error.allows(Level::Error));
        assert!(!Level::Warn.allows(Level::Error));
        assert!(Level::Info.allows(Level::Debug));
        assert!(Level::Debug.allows(Level::Debug));
        assert!(!Level::Debug.allows(Level::Info));
    }
}
