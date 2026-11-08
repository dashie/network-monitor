//! Configuration file watcher.
//!
//! The file is small and local, so the monitor compares its content once per second
//! instead of watching inode events: no dependency, no platform difference, and an
//! editor that saves through a rename is handled like any other write.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How often the configuration file is compared with the last accepted content.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// What the watcher noticed on this poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The file content differs from the last accepted content.
    Content(Vec<u8>),
    /// The file cannot be read right now (deleted, permissions, mid-save).
    ReadError(String),
}

/// Compares the content of one file with the last content it accepted.
pub struct Watcher {
    path: PathBuf,
    last: Option<Vec<u8>>,
    failing: bool,
}

impl Watcher {
    /// Start from the content that is already active, so the first poll is quiet.
    pub fn new(path: &Path) -> Watcher {
        Watcher {
            path: path.to_path_buf(),
            last: fs::read(path).ok(),
            failing: false,
        }
    }

    pub fn poll(&mut self) -> Option<Change> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                self.failing = false;
                if self.last.as_deref() == Some(bytes.as_slice()) {
                    None
                } else {
                    self.last = Some(bytes.clone());
                    Some(Change::Content(bytes))
                }
            }
            Err(error) => {
                if self.failing {
                    // Report a broken file once, not on every poll.
                    None
                } else {
                    self.failing = true;
                    Some(Change::ReadError(error.to_string()))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        fs::write(path, content).expect("the test can write the file");
    }

    #[test]
    fn the_initial_content_is_not_a_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        write(&path, "interval_ms = 1000\n");

        let mut watcher = Watcher::new(&path);
        assert_eq!(watcher.poll(), None);
    }

    #[test]
    fn a_rewritten_file_is_a_change_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        write(&path, "interval_ms = 1000\n");

        let mut watcher = Watcher::new(&path);
        assert_eq!(watcher.poll(), None);

        write(&path, "interval_ms = 2000\n");
        assert_eq!(
            watcher.poll(),
            Some(Change::Content(b"interval_ms = 2000\n".to_vec()))
        );
        assert_eq!(watcher.poll(), None);
    }

    #[test]
    fn identical_content_is_not_a_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        write(&path, "interval_ms = 1000\n");

        let mut watcher = Watcher::new(&path);
        write(&path, "interval_ms = 1000\n");
        assert_eq!(watcher.poll(), None);
    }

    #[test]
    fn a_missing_file_is_reported_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        write(&path, "interval_ms = 1000\n");

        let mut watcher = Watcher::new(&path);
        fs::remove_file(&path).unwrap();

        assert!(matches!(watcher.poll(), Some(Change::ReadError(_))));
        assert_eq!(watcher.poll(), None);

        // A restored, different file is a change again.
        write(&path, "interval_ms = 3000\n");
        assert_eq!(
            watcher.poll(),
            Some(Change::Content(b"interval_ms = 3000\n".to_vec()))
        );
    }

    #[test]
    fn a_file_that_did_not_exist_at_start_is_detected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");

        let mut watcher = Watcher::new(&path);
        write(&path, "interval_ms = 1000\n");
        assert!(matches!(watcher.poll(), Some(Change::Content(_))));
    }
}
