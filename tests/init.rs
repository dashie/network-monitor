//! The `init` subcommand end to end: the folder it creates, the `check` it enables, and the
//! refusals that keep an operator's files safe.

use assert_cmd::Command;
use predicates::prelude::*;

fn monitor(directory: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("network-monitor").unwrap();
    command.current_dir(directory);
    command
}

#[test]
fn init_creates_a_folder_that_check_accepts() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("monitor");

    monitor(root.path())
        .arg("init")
        .arg(&folder)
        .assert()
        .success()
        .stdout(predicate::str::contains("network-monitor.toml"))
        .stdout(predicate::str::contains("data/"))
        .stdout(predicate::str::contains("logs/"));

    assert!(folder.join("network-monitor.toml").is_file());
    assert!(folder.join("data").is_dir());
    assert!(folder.join("logs").is_dir());
    assert!(!folder.join("data/events.db").exists());

    monitor(&folder)
        .args(["check", "--config", "network-monitor.toml"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 target(s)"))
        .stdout(predicate::str::contains("1 channel(s) verified"));

    assert!(folder.join("logs/events.log").is_file());
    assert!(!folder.join("data/events.db").exists());
}

#[test]
fn init_refuses_a_non_empty_folder() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("monitor");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("keep.txt"), "keep").unwrap();

    monitor(root.path())
        .arg("init")
        .arg(&folder)
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not empty"))
        .stderr(predicate::str::contains("choose a new or empty folder"));

    assert!(!folder.join("network-monitor.toml").exists());
    assert_eq!(
        std::fs::read_to_string(folder.join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn init_refuses_a_file() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("not-a-folder");
    std::fs::write(&file, "x").unwrap();

    monitor(root.path())
        .arg("init")
        .arg(&file)
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a directory"));
}

#[test]
fn init_refuses_a_second_run() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("monitor");

    monitor(root.path())
        .arg("init")
        .arg(&folder)
        .assert()
        .success();
    let first = std::fs::read(folder.join("network-monitor.toml")).unwrap();

    monitor(root.path())
        .arg("init")
        .arg(&folder)
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not empty"));

    assert_eq!(
        std::fs::read(folder.join("network-monitor.toml")).unwrap(),
        first
    );
}

#[test]
fn init_does_not_read_the_default_configuration() {
    let root = tempfile::tempdir().unwrap();
    let invalid = root.path().join("network-monitor.toml");
    std::fs::write(&invalid, "not = [valid").unwrap();
    let folder = root.path().join("fresh");

    monitor(root.path())
        .arg("init")
        .arg(&folder)
        .assert()
        .success();

    assert_eq!(std::fs::read_to_string(&invalid).unwrap(), "not = [valid");
}

#[test]
fn init_rejects_configuration_flags() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("monitor");
    let cases = [
        ("--config", Some("other.toml")),
        ("--target", Some("1.1.1.1")),
        ("--database", Some("other.db")),
        ("--show-config", None),
    ];

    for (flag, value) in cases {
        let mut command = monitor(root.path());
        command.arg("init").arg(&folder).arg(flag);
        if let Some(value) = value {
            command.arg(value);
        }
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains("init takes only a folder"));
    }

    assert!(!folder.exists());
}

#[test]
fn init_accepts_an_empty_existing_folder() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("monitor");
    std::fs::create_dir(&folder).unwrap();

    monitor(root.path())
        .arg("init")
        .arg(&folder)
        .assert()
        .success();

    assert!(folder.join("network-monitor.toml").is_file());
}

#[test]
fn init_accepts_a_path_with_spaces() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("my monitor");

    monitor(root.path())
        .arg("init")
        .arg(&folder)
        .assert()
        .success();

    assert!(folder.join("network-monitor.toml").is_file());
}

#[test]
fn the_scaffolded_folder_is_checked_from_another_directory() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("monitor");
    monitor(root.path())
        .arg("init")
        .arg(&folder)
        .assert()
        .success();

    let config = folder.join("network-monitor.toml");
    monitor(root.path())
        .args(["check", "--config", config.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("ok    channel"));

    assert!(
        folder.join("logs/events.log").is_file(),
        "the event log belongs beside the configuration"
    );
    assert!(
        !root.path().join("logs").exists(),
        "nothing is created in the working directory"
    );
    assert!(!root.path().join("events.db").exists());
}
