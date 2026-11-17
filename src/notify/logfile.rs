//! The log file channel: one line per event, as text or as a JSON document.
//!
//! The file is append-only and created with mode `0600` on Unix: it carries target
//! addresses and causes. Its own transport failures go to stderr, never into the file,
//! so a failing sink cannot feed itself.

use super::channel::{Channel, Payload, SendError};
use super::message::{Kind, Message};
use crate::config::{LogFile as Settings, LogFormat};
use crate::logging::{format_line_at, Level};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// The log file transport of one channel.
#[derive(Debug)]
pub struct LogFile {
    name: String,
    filename: PathBuf,
    logger: String,
    level: Level,
    format: LogFormat,
    file: Option<File>,
}

impl LogFile {
    pub fn new(settings: &Settings) -> Self {
        LogFile {
            name: settings.name.clone(),
            filename: settings.filename.clone(),
            logger: settings.logger.clone(),
            level: settings.level,
            format: settings.format,
            file: None,
        }
    }

    fn line(&self, message: &Message) -> String {
        match self.format {
            LogFormat::Text => format_line_at(
                message.occurred_at_ms,
                message.severity(),
                &self.logger,
                message.target_key(),
                &message.summary(),
            ),
            LogFormat::Jsonl => message.json(Some(&self.logger)),
        }
    }

    /// Open the file for append, creating it with mode `0600` on Unix.
    fn open_file(&self) -> Result<File, String> {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&self.filename).map_err(|error| {
            format!(
                "cannot open {} for append: {error}",
                self.filename.display()
            )
        })
    }
}

impl Channel for LogFile {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> Kind {
        Kind::LogFile
    }

    fn open(&mut self) -> Result<(), String> {
        self.file = Some(self.open_file()?);
        Ok(())
    }

    fn verify(&mut self) -> Result<(), String> {
        self.open_file().map(|_| ())
    }

    fn payload(&self, message: &Message) -> Option<Payload> {
        if !message.severity().allows(self.level) {
            return None;
        }
        let (body, media_type) = match self.format {
            LogFormat::Text => (self.line(message), "text/plain; charset=utf-8"),
            LogFormat::Jsonl => (self.line(message), "application/x-ndjson"),
        };
        Some(Payload { body, media_type })
    }

    fn send(&mut self, payload: &Payload) -> Result<(), SendError> {
        let Some(file) = &mut self.file else {
            return Err(SendError {
                retryable: false,
                reason: format!("{} is not open", self.filename.display()),
            });
        };
        writeln!(file, "{}", payload.body).map_err(|error| SendError {
            retryable: false,
            reason: format!("cannot write {}: {error}", self.filename.display()),
        })?;
        file.flush().map_err(|error| SendError {
            retryable: false,
            reason: format!("cannot flush {}: {error}", self.filename.display()),
        })
    }

    fn close(&mut self) {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::message::{Cause, TargetRef};
    use std::time::Duration;

    fn settings(directory: &tempfile::TempDir, level: Level, format: LogFormat) -> Settings {
        Settings {
            name: "audit".to_string(),
            filename: directory.path().join("audit.log"),
            logger: "notification".to_string(),
            level,
            format,
            stabilize: Duration::from_secs(60),
        }
    }

    fn down() -> Message {
        Message::target_down(
            TargetRef::new("icmp://1.1.1.1", Some("cloudflare-dns".to_string())),
            Cause::new("ICMP.NO_REPLY", "no reply within 2000 ms"),
            1_764_000_000_000,
        )
    }

    fn up() -> Message {
        Message::target_up(TargetRef::new("icmp://1.1.1.1", None), 1_764_000_000_000)
    }

    #[test]
    fn a_text_line_carries_the_timestamp_the_level_and_the_logger() {
        let directory = tempfile::tempdir().unwrap();
        let channel = LogFile::new(&settings(&directory, Level::Info, LogFormat::Text));
        let payload = channel.payload(&down()).expect("a warning passes info");
        assert_eq!(payload.media_type, "text/plain; charset=utf-8");
        assert_eq!(
            payload.body,
            concat!(
                "2025-11-24T16:00:00.000Z WARN  notification [icmp://1.1.1.1] ",
                "DOWN cloudflare-dns (icmp://1.1.1.1) ",
                "cause=ICMP.NO_REPLY: no reply within 2000 ms"
            )
        );
    }

    #[test]
    fn a_json_line_is_the_document_plus_the_logger() {
        let directory = tempfile::tempdir().unwrap();
        let channel = LogFile::new(&settings(&directory, Level::Info, LogFormat::Jsonl));
        let payload = channel.payload(&down()).expect("a warning passes info");
        assert_eq!(payload.media_type, "application/x-ndjson");
        assert_eq!(payload.body, down().json(Some("notification")));
    }

    #[test]
    fn the_level_filters_what_reaches_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let channel = LogFile::new(&settings(&directory, Level::Error, LogFormat::Text));
        assert!(
            channel.payload(&down()).is_none(),
            "a warning is below error"
        );
        assert!(
            channel.payload(&up()).is_none(),
            "information is below error"
        );
    }

    #[test]
    fn the_file_is_appended_and_survives_a_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let settings = settings(&directory, Level::Info, LogFormat::Text);

        let mut first = LogFile::new(&settings);
        first.open().unwrap();
        if let Some(payload) = first.payload(&down()) {
            first.send(&payload).unwrap();
        }
        first.close();

        let mut second = LogFile::new(&settings);
        second.open().unwrap();
        if let Some(payload) = second.payload(&up()) {
            second.send(&payload).unwrap();
        }
        second.close();

        let text = std::fs::read_to_string(&settings.filename).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().next().unwrap().contains("DOWN cloudflare-dns"));
        assert!(text.lines().nth(1).unwrap().contains("UP icmp://1.1.1.1"));
    }

    #[test]
    fn verifying_opens_the_file_and_leaves_it_empty() {
        let directory = tempfile::tempdir().unwrap();
        let settings = settings(&directory, Level::Info, LogFormat::Text);
        let mut channel = LogFile::new(&settings);

        channel.verify().expect("the file is writable");

        assert!(settings.filename.exists(), "verification opens the file");
        assert_eq!(
            std::fs::read_to_string(&settings.filename).unwrap(),
            "",
            "and writes nothing into it"
        );
    }

    #[test]
    fn verifying_an_unwritable_parent_reports_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.filename = directory.path().join("missing/audit.log");
        let mut channel = LogFile::new(&settings);

        let reason = channel.verify().unwrap_err();
        assert!(reason.contains("missing/audit.log"), "unexpected: {reason}");
    }

    #[test]
    fn the_channel_states_its_name_and_kind() {
        let directory = tempfile::tempdir().unwrap();
        let channel = LogFile::new(&settings(&directory, Level::Info, LogFormat::Text));
        assert_eq!(channel.name(), "audit");
        assert_eq!(channel.kind(), Kind::LogFile);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_created_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let settings = settings(&directory, Level::Info, LogFormat::Text);
        let mut channel = LogFile::new(&settings);
        channel.open().unwrap();

        let mode = std::fs::metadata(&settings.filename)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn an_unwritable_parent_is_a_start_error() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.filename = directory.path().join("missing/audit.log");
        let mut channel = LogFile::new(&settings);
        assert!(channel.open().is_err());
    }
}
