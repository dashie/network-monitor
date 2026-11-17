//! The log file channel: one line per event, as text or as a JSON document.
//!
//! The file is append-only and created with mode `0600` on Unix: it carries target
//! addresses and causes. Its own transport failures go to stderr, never into the file,
//! so a failing sink cannot feed itself.

use super::channel::{Channel, Payload, SendError};
use super::message::{Kind, Message};
use crate::config::{LogFile as Settings, LogFormat};
use crate::logging::{self, format_line_at, Level};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
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
    /// `0` disables rotation.
    max_bytes: u64,
    max_files: u64,
    /// What the current file holds, so a write knows whether it would cross the limit.
    size: u64,
    /// The file the rotations rename: the configured path with a symbolic link resolved,
    /// so the link keeps pointing at the file that is being written.
    real: PathBuf,
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
            max_bytes: settings.max_bytes,
            max_files: settings.max_files,
            size: 0,
            real: settings.filename.clone(),
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

    /// The name of one rotated file: the real name plus `.<index>`.
    fn rotated(&self, index: u64) -> PathBuf {
        let mut name = self.real.clone().into_os_string();
        name.push(format!(".{index}"));
        PathBuf::from(name)
    }

    /// Rename the current file to `.1`, shift the older ones up, and drop what falls
    /// past `max_files`. A missing rotated file is not an error.
    fn rotate(&mut self) -> Result<(), String> {
        match std::fs::remove_file(self.rotated(self.max_files)) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot remove {}: {error}",
                    self.rotated(self.max_files).display()
                ));
            }
        }

        for index in (1..self.max_files).rev() {
            let from = self.rotated(index);
            let to = self.rotated(index + 1);
            match std::fs::rename(&from, &to) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("cannot rename {}: {error}", from.display()));
                }
            }
        }

        std::fs::rename(&self.real, self.rotated(1))
            .map_err(|error| format!("cannot rename {}: {error}", self.real.display()))?;
        let file = self.open_file()?;
        self.file = Some(file);
        self.size = 0;
        Ok(())
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
        let file = self.open_file()?;
        self.size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        self.real = std::fs::canonicalize(&self.filename).unwrap_or_else(|_| self.filename.clone());
        self.file = Some(file);
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
        if self.file.is_none() {
            return Err(SendError {
                retryable: false,
                reason: format!("{} is not open", self.filename.display()),
            });
        }

        // The newline is part of the line, so the limit bounds the file that is read.
        let line_bytes = payload.body.len() as u64 + 1;
        if self.max_bytes > 0 && self.size > 0 && self.size + line_bytes > self.max_bytes {
            if let Err(reason) = self.rotate() {
                logging::warn(
                    None,
                    &format!("cannot rotate {}: {reason}", self.filename.display()),
                );
            }
        }

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
        })?;
        self.size += line_bytes;
        Ok(())
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
            max_bytes: 0,
            max_files: 5,
        }
    }

    fn payload(text: &str) -> Payload {
        Payload {
            body: text.to_string(),
            media_type: "text/plain; charset=utf-8",
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

    fn rotated_path(settings: &Settings, index: u64) -> PathBuf {
        let mut name = settings.filename.clone().into_os_string();
        name.push(format!(".{index}"));
        PathBuf::from(name)
    }

    #[test]
    fn max_bytes_zero_appends_for_ever() {
        let directory = tempfile::tempdir().unwrap();
        let settings = settings(&directory, Level::Info, LogFormat::Text);
        let mut channel = LogFile::new(&settings);
        channel.open().unwrap();

        for _ in 0..5 {
            channel.send(&payload(&"x".repeat(200))).unwrap();
        }

        assert!(
            std::fs::metadata(&settings.filename).unwrap().len() > 1000,
            "the file grows without a bound"
        );
        assert!(!rotated_path(&settings, 1).exists(), "nothing was rotated");
    }

    #[test]
    fn a_line_that_would_cross_the_limit_rotates_first() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.max_bytes = 10;
        let mut channel = LogFile::new(&settings);
        channel.open().unwrap();

        channel.send(&payload("aaaa")).unwrap();
        channel.send(&payload("bbbb")).unwrap();
        assert!(
            !rotated_path(&settings, 1).exists(),
            "exactly at the limit nothing rotates"
        );

        channel.send(&payload("c")).unwrap();

        assert_eq!(
            std::fs::read_to_string(rotated_path(&settings, 1)).unwrap(),
            "aaaa\nbbbb\n"
        );
        assert_eq!(std::fs::read_to_string(&settings.filename).unwrap(), "c\n");
    }

    #[test]
    fn a_line_larger_than_the_limit_is_written_whole() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.max_bytes = 5;
        let mut channel = LogFile::new(&settings);
        channel.open().unwrap();

        channel.send(&payload("123456789")).unwrap();
        channel.send(&payload("x")).unwrap();

        assert_eq!(
            std::fs::read_to_string(rotated_path(&settings, 1)).unwrap(),
            "123456789\n",
            "the oversized line stays in one piece"
        );
        assert_eq!(std::fs::read_to_string(&settings.filename).unwrap(), "x\n");
    }

    #[test]
    fn the_shift_keeps_the_last_max_files_and_deletes_the_oldest() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.max_bytes = 4;
        settings.max_files = 3;
        let mut channel = LogFile::new(&settings);
        channel.open().unwrap();

        for line in ["a", "b", "c", "d", "e", "f", "g", "h", "i"] {
            channel.send(&payload(line)).unwrap();
        }

        assert_eq!(std::fs::read_to_string(&settings.filename).unwrap(), "i\n");
        assert_eq!(
            std::fs::read_to_string(rotated_path(&settings, 1)).unwrap(),
            "g\nh\n"
        );
        assert_eq!(
            std::fs::read_to_string(rotated_path(&settings, 2)).unwrap(),
            "e\nf\n"
        );
        assert_eq!(
            std::fs::read_to_string(rotated_path(&settings, 3)).unwrap(),
            "c\nd\n"
        );
        assert!(
            !rotated_path(&settings, 4).exists(),
            "nothing is kept past max_files"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_rotation_keeps_the_line_and_does_not_stop_the_channel() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.max_bytes = 4;
        let mut channel = LogFile::new(&settings);
        channel.open().unwrap();
        channel.send(&payload("aaaa")).unwrap();

        // The file is already open, so the write must still reach it when the
        // directory refuses the rename.
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = channel.send(&payload("b"));
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_ok(), "the line must still be written");
        assert_eq!(
            std::fs::read_to_string(&settings.filename).unwrap(),
            "aaaa\nb\n"
        );
        assert!(!rotated_path(&settings, 1).exists(), "nothing was renamed");
    }

    #[cfg(unix)]
    #[test]
    fn a_rotated_file_keeps_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.max_bytes = 4;
        let mut channel = LogFile::new(&settings);
        channel.open().unwrap();
        channel.send(&payload("aaaa")).unwrap();
        channel.send(&payload("b")).unwrap();

        for path in [settings.filename.clone(), rotated_path(&settings, 1)] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} has mode {mode:o}", path.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_followed_and_the_target_rotates() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("real.log");
        std::fs::write(&target, "").unwrap();
        let link = directory.path().join("audit.log");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.filename = link.clone();
        settings.max_bytes = 10;
        let mut channel = LogFile::new(&settings);
        channel.open().unwrap();

        channel.send(&payload("aaaa")).unwrap();
        channel.send(&payload("bbbb")).unwrap();
        channel.send(&payload("c")).unwrap();

        assert_eq!(
            std::fs::read_to_string(&link).unwrap(),
            "c\n",
            "the link still reaches the current file"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("real.log.1")).unwrap(),
            "aaaa\nbbbb\n"
        );
        assert!(
            !directory.path().join("audit.log.1").exists(),
            "the link itself is not renamed"
        );
    }

    #[test]
    fn verification_never_rotates() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(&directory, Level::Info, LogFormat::Text);
        settings.max_bytes = 4;
        std::fs::write(&settings.filename, "aaaa\n").unwrap();
        let mut channel = LogFile::new(&settings);

        channel.verify().expect("the file is writable");

        assert_eq!(
            std::fs::read_to_string(&settings.filename).unwrap(),
            "aaaa\n",
            "verification leaves the file untouched"
        );
        assert!(!rotated_path(&settings, 1).exists());
    }
}
