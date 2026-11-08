//! The command-line contract: exit codes, effective configuration, query output.

use assert_cmd::Command;
use predicates::prelude::*;

fn monitor(directory: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("network-monitor").unwrap();
    command.current_dir(directory);
    command
}

#[test]
fn show_config_prints_the_effective_configuration_without_a_file() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .arg("--show-config")
        .assert()
        .success()
        .stdout(predicate::str::contains("interval_ms = 5000"))
        .stdout(predicate::str::contains("timeout_ms = 2000"))
        .stdout(predicate::str::contains(
            "database = \"network-monitor.db\"",
        ))
        .stdout(predicate::str::contains("retention_days = 0"));
}

#[test]
fn show_config_reflects_the_flags() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args([
            "--show-config",
            "--interval-ms",
            "1000",
            "--target",
            "1.1.1.1,interface=eth1,label=cloudflare",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("interval_ms = 1000"))
        .stdout(predicate::str::contains("[[targets]]"))
        .stdout(predicate::str::contains("interface = \"eth1\""))
        .stdout(predicate::str::contains("label = \"cloudflare\""));
}

#[test]
fn an_explicit_missing_config_file_fails() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["--config", "missing.toml", "--show-config"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot read the configuration file",
        ));
}

#[test]
fn an_invalid_target_spec_fails() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["--show-config", "--target", "not-an-address"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid --target"));
}

#[test]
fn monitor_without_targets_fails() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["monitor", "--database", "events.db"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no targets configured"));
}

#[test]
fn query_on_a_missing_database_fails() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["query", "state", "--database", "missing.db"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn a_configuration_file_is_read_from_the_default_path() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        "interval_ms = 15000\ntimeout_ms = 3000\ndatabase = \"from-default.toml.db\"\n\n[[targets]]\nip = \"9.9.9.9\"\n",
    )
    .unwrap();

    monitor(directory.path())
        .arg("--show-config")
        .assert()
        .success()
        .stdout(predicate::str::contains("interval_ms = 15000"))
        .stdout(predicate::str::contains("from-default.toml.db"))
        .stdout(predicate::str::contains("ip = \"9.9.9.9\""));
}

#[test]
fn an_unknown_key_in_the_file_fails() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        "interval_ms = 1000\nunknown_key = true\n",
    )
    .unwrap();

    monitor(directory.path())
        .arg("--show-config")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot parse the configuration file",
        ));
}

#[test]
fn the_query_help_documents_the_subcommands() {
    monitor(&std::env::temp_dir())
        .args(["query", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("state"))
        .stdout(predicate::str::contains("events"));
}
