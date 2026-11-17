//! Candidate promotion: the backup name and the atomic replace.
//!
//! A candidate is read and verified once at startup and carried as bytes. Promotion
//! never re-reads the candidate path: a file edited or deleted afterwards can never
//! promote content that was not verified. The main file is replaced through a sibling
//! temporary file and one `rename`, so a reader never sees a truncated configuration.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::time::format_compact_utc;

/// A verified candidate waiting for the first probe round.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The candidate file the bytes were read from, for the promotion log line.
    pub source: PathBuf,
    /// The main configuration the candidate replaces.
    pub main: PathBuf,
    /// The bytes read and verified at startup.
    pub bytes: Vec<u8>,
    /// The start instant, used in the backup name.
    pub stamp: i64,
}

#[derive(Debug, Error)]
pub enum PromoteError {
    #[error("cannot read the main configuration {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot write the backup {path}: {source}")]
    Backup {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot write the promoted configuration {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot replace the main configuration {path}: {source}")]
    Replace {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl Candidate {
    /// Keep the main file as `<name>.<UTC stamp>.bak` and replace it with the candidate.
    ///
    /// Returns the backup path on success, `None` when the bytes already equal the main
    /// file, and an error without touching the main file otherwise.
    pub fn commit(&self) -> Result<Option<PathBuf>, PromoteError> {
        let current = fs::read(&self.main).map_err(|source| PromoteError::Read {
            path: self.main.clone(),
            source,
        })?;
        if current == self.bytes {
            return Ok(None);
        }

        let backup = backup_path(&self.main, self.stamp);
        fs::copy(&self.main, &backup).map_err(|source| PromoteError::Backup {
            path: backup.clone(),
            source,
        })?;

        let temporary = temporary_path(&self.main, self.stamp);
        if let Err(error) = write_private(&temporary, &self.bytes) {
            let _ = fs::remove_file(&temporary);
            return Err(PromoteError::Write {
                path: temporary,
                source: error,
            });
        }
        if let Err(source) = fs::rename(&temporary, &self.main) {
            let _ = fs::remove_file(&temporary);
            return Err(PromoteError::Replace {
                path: self.main.clone(),
                source,
            });
        }

        Ok(Some(backup))
    }
}

/// The backup of `main` for the instant `stamp`, in the same directory.
pub fn backup_path(main: &Path, stamp: i64) -> PathBuf {
    with_suffix(main, &format!("{}.bak", format_compact_utc(stamp)))
}

/// The sibling temporary file a promotion publishes through.
fn temporary_path(main: &Path, stamp: i64) -> PathBuf {
    with_suffix(
        main,
        &format!("{}.{}.tmp", format_compact_utc(stamp), std::process::id()),
    )
}

fn with_suffix(main: &Path, suffix: &str) -> PathBuf {
    let name = main
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    main.with_file_name(format!("{name}.{suffix}"))
}

/// Write a sibling temporary file with mode `0600` on Unix, so the promoted
/// configuration is never readable by another user.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, Candidate) {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("uptime-monitor.toml");
        fs::write(&main, b"interval_ms = 1000\n").unwrap();
        let candidate = Candidate {
            source: directory.path().join("next.toml"),
            main: main.clone(),
            bytes: b"interval_ms = 2000\n".to_vec(),
            stamp: 1_794_096_000_000,
        };
        (directory, main, candidate)
    }

    #[test]
    fn the_backup_carries_the_timestamp_and_the_previous_bytes() {
        let (_directory, main, candidate) = setup();

        let backup = candidate.commit().unwrap().expect("the main file differed");

        assert_eq!(
            backup.file_name().unwrap().to_string_lossy(),
            "uptime-monitor.toml.20261108T000000Z.bak"
        );
        assert_eq!(fs::read(&backup).unwrap(), b"interval_ms = 1000\n");
        assert_eq!(fs::read(&main).unwrap(), b"interval_ms = 2000\n");
    }

    #[test]
    fn no_temporary_file_survives_a_promotion() {
        let (directory, _main, candidate) = setup();
        candidate.commit().unwrap();

        let temporaries: Vec<String> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(temporaries.is_empty(), "{temporaries:?}");
    }

    #[test]
    fn a_candidate_equal_to_the_main_file_writes_nothing() {
        let (_directory, main, mut candidate) = setup();
        candidate.bytes = fs::read(&main).unwrap();

        assert_eq!(candidate.commit().unwrap(), None);
        assert!(!backup_path(&main, candidate.stamp).exists());
    }

    #[test]
    fn a_failed_backup_leaves_the_main_file_untouched() {
        let (_directory, main, candidate) = setup();
        // A directory on the backup path makes the copy fail for every user.
        fs::create_dir(backup_path(&main, candidate.stamp)).unwrap();

        assert!(candidate.commit().is_err());
        assert_eq!(fs::read(&main).unwrap(), b"interval_ms = 1000\n");
    }

    #[cfg(unix)]
    #[test]
    fn the_promoted_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let (_directory, main, candidate) = setup();
        fs::set_permissions(&main, fs::Permissions::from_mode(0o644)).unwrap();

        candidate.commit().unwrap();

        let mode = fs::metadata(&main).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
