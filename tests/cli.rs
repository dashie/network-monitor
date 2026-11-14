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

#[test]
fn an_invalid_listen_address_fails_before_probing() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args([
            "monitor",
            "--target",
            "127.0.0.1",
            "--database",
            "events.db",
            "--listen",
            "not-an-address",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid listen address"));
}

#[test]
fn serve_refuses_an_invalid_listen_address() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["serve", "--listen", "nope", "--database", "events.db"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid listen address"));
}

#[test]
fn show_config_renders_one_shape_per_method() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args([
            "--show-config",
            "--target",
            "tcp:1.1.1.1:443,label=https",
            "--target",
            "dns:example.com@9.9.9.9,record=AAAA",
            "--target",
            "http:https://example.com/health,expect=204",
            "--target",
            "1.1.1.1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("method = \"tcp\""))
        .stdout(predicate::str::contains("host = \"1.1.1.1\""))
        .stdout(predicate::str::contains("port = 443"))
        .stdout(predicate::str::contains("method = \"dns\""))
        .stdout(predicate::str::contains("resolver = \"9.9.9.9\""))
        .stdout(predicate::str::contains("record = \"AAAA\""))
        .stdout(predicate::str::contains("method = \"http\""))
        .stdout(predicate::str::contains(
            "url = \"https://example.com/health\"",
        ))
        .stdout(predicate::str::contains("expect_status"))
        .stdout(predicate::str::contains("method = \"icmp\""))
        .stdout(predicate::str::contains("ip = \"1.1.1.1\""));
}

#[test]
fn an_incomplete_target_spec_names_the_problem() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["--show-config", "--target", "tcp:1.1.1.1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("host:port"));
}

#[test]
fn a_target_key_belonging_to_another_method_is_refused() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["--show-config", "--target", "tcp:1.1.1.1:443,record=A"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("record"));
}
