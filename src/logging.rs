//! Line-based diagnostics on stderr.
//!
//! stdout carries data only: query results must stay pipeable. Every diagnostic is
//! one line, `<RFC 3339> <LEVEL> [<target>] <message>`.

use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::time::{format_rfc3339, now_ms};

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
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn enabled(level: Level) -> bool {
    (level as u8) <= LEVEL.load(Ordering::Relaxed)
}

pub fn write(level: Level, target: Option<&str>, message: &str) {
    if !enabled(level) {
        return;
    }
    let timestamp = format_rfc3339(now_ms());
    let line = match target {
        Some(target) => format!("{timestamp} {:<5} [{target}] {message}", level.label()),
        None => format!("{timestamp} {:<5} {message}", level.label()),
    };
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
