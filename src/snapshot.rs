//! The active configuration snapshot: `<config>.active` beside the configuration.
//!
//! It holds the exact bytes the process started from, is written once per successful
//! start through a private sibling and one `rename`, and is never read by the monitor
//! itself: it is recovery material, not configuration.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a start wants published in the snapshot: where, and the bytes it started from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// The snapshot path of a configuration file: the same name plus `.active`.
pub fn path_for(config: &Path) -> PathBuf {
    let name = config
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    config.with_file_name(format!("{name}.active"))
}

/// Publish `bytes` through a private sibling and one `rename`.
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = temporary_path(path);
    if let Err(error) = write_private(&temporary, bytes) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

/// `<path> (<age>)` from the modification time, for a log or a check line.
pub fn describe(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let elapsed = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        .as_millis() as i64;
    Some(format!("{} ({})", path.display(), age(elapsed)))
}

fn age(ms: i64) -> String {
    let seconds = ms.max(0) / 1_000;
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{name}.{}.tmp", std::process::id()))
}

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

    #[test]
    fn the_snapshot_sits_beside_the_configuration() {
        assert_eq!(
            path_for(Path::new("/etc/uptime-monitor.toml")),
            PathBuf::from("/etc/uptime-monitor.toml.active")
        );
        assert_eq!(
            path_for(Path::new("uptime-monitor.toml")),
            PathBuf::from("uptime-monitor.toml.active")
        );
    }

    #[test]
    fn a_write_publishes_the_bytes_and_leaves_no_temporary() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("uptime-monitor.toml.active");

        write(&path, b"interval_ms = 1000\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"interval_ms = 1000\n");
        let temporaries: Vec<String> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(temporaries.is_empty(), "{temporaries:?}");
    }

    #[test]
    fn a_second_write_overwrites_the_first() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("uptime-monitor.toml.active");

        write(&path, b"first\n").unwrap();
        write(&path, b"second\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second\n");
    }

    #[cfg(unix)]
    #[test]
    fn the_snapshot_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("uptime-monitor.toml.active");
        write(&path, b"secret = \"token\"\n").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_missing_parent_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing/uptime-monitor.toml.active");
        assert!(write(&path, b"x").is_err());
    }

    #[test]
    fn describe_names_the_path_and_an_age() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("uptime-monitor.toml.active");
        write(&path, b"x").unwrap();

        let description = describe(&path).expect("the file exists");
        assert!(
            description.starts_with(&path.display().to_string()),
            "{description}"
        );
        assert!(description.contains("ago"), "{description}");
        assert!(describe(&directory.path().join("absent.active")).is_none());
    }
}
