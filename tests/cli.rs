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
fn the_startup_log_states_the_defaults_and_every_target() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args([
            "monitor",
            "--target",
            "1.1.1.1",
            "--target",
            "tcp:1.1.1.1:443",
            "--database",
            "events.db",
            "--listen",
            "not-an-address",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("INFO  monitor initializing"))
        .stderr(predicate::str::contains(
            "INFO  monitor no configuration file; using the command line and the built-in defaults",
        ))
        .stderr(predicate::str::contains(
            "INFO  monitor notification defaults: none",
        ))
        .stderr(predicate::str::contains(
            "INFO  monitor initialized target icmp://1.1.1.1",
        ))
        .stderr(predicate::str::contains(
            "INFO  monitor initialized target tcp://1.1.1.1:443",
        ));
}

#[test]
fn the_startup_log_names_the_configuration_file() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        "[[targets]]\nip = \"9.9.9.9\"\n",
    )
    .unwrap();

    monitor(directory.path())
        .args([
            "monitor",
            "--database",
            "events.db",
            "--listen",
            "not-an-address",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "INFO  monitor reading configuration from network-monitor.toml",
        ));
}

#[test]
fn the_startup_log_names_the_configured_chat() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        "[[targets]]\nip = \"9.9.9.9\"\n\n[telegram]\nbot_token = \"123456:abc-def\"\nchat_id = \"-100200\"\n",
    )
    .unwrap();

    monitor(directory.path())
        .args([
            "monitor",
            "--database",
            "events.db",
            "--listen",
            "not-an-address",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "INFO  monitor notification channel telegram (telegram) configured for chat -100200",
        ))
        .stderr(predicate::str::contains("123456:abc-def").not());
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

#[test]
fn simulate_requires_a_listen_address() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["simulate", "--target", "1.1.1.1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--listen"));
}

#[test]
fn simulate_refuses_a_history_outside_the_simulated_range() {
    let directory = tempfile::tempdir().unwrap();

    for value in ["0", "4000d"] {
        monitor(directory.path())
            .args(["simulate", "--listen", ":0", "--history", value])
            .assert()
            .failure()
            .stderr(predicate::str::contains("1 to 3660 days"));
    }

    monitor(directory.path())
        .args(["simulate", "--listen", ":0", "--history", "30x"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a number of days"));
}

#[test]
fn simulate_refuses_a_fault_rate_outside_the_accepted_range() {
    let directory = tempfile::tempdir().unwrap();

    // A negative value is refused by clap as an unknown flag, before the parser sees it; the
    // range of the number itself is asserted by the unit test of `FaultRate`.
    for value in ["1001", "NaN", "lots"] {
        monitor(directory.path())
            .args(["simulate", "--listen", ":0", "--fault-rate", value])
            .assert()
            .failure()
            .stderr(predicate::str::contains("--fault-rate"));
    }
}

#[test]
fn simulate_without_targets_fails() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["simulate", "--listen", ":0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no targets configured"));
}

#[test]
fn simulate_refuses_an_invalid_listen_address() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["simulate", "--listen", "nope", "--target", "1.1.1.1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid listen address"));
}

#[test]
fn the_startup_log_states_the_simulation_and_that_no_database_is_used() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args([
            "simulate",
            "--listen",
            "nope",
            "--target",
            "1.1.1.1",
            "--history",
            "7d",
            "--seed",
            "42",
            "--fault-rate",
            "0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "simulating 1 target(s) over the last 7 day(s), seed 42, 0 fault(s) per day: no \
             probe runs and no database is read or written",
        ))
        .stderr(predicate::str::contains("simulated target icmp://1.1.1.1"));
}

#[test]
fn a_channel_is_declared_once_and_printed_redacted() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        concat!(
            "notify = [\"telegram\"]\n\n",
            "[[notification]]\n",
            "name = \"telegram\"\n",
            "type = \"telegram\"\n",
            "bot_token = \"123456:secret\"\n",
            "chat_id = \"-100200\"\n\n",
            "[[notification]]\n",
            "name = \"audit\"\n",
            "type = \"logfile\"\n",
            "filename = \"audit.log\"\n",
            "format = \"jsonl\"\n",
            "level = \"info\"\n\n",
            "[[targets]]\n",
            "ip = \"9.9.9.9\"\n",
            "notify = [\"telegram\", \"audit\"]\n",
        ),
    )
    .unwrap();

    let output = monitor(directory.path())
        .arg("--show-config")
        .assert()
        .success()
        .stdout(predicate::str::contains("[[notification]]"))
        .stdout(predicate::str::contains("name = \"audit\""))
        .stdout(predicate::str::contains("format = \"jsonl\""))
        .stdout(predicate::str::contains("<redacted>"))
        .stdout(predicate::str::contains("123456:secret").not())
        .get_output()
        .stdout
        .clone();

    // The printed configuration parses back into the same routing.
    std::fs::write(directory.path().join("roundtrip.toml"), output).unwrap();
    monitor(directory.path())
        .args(["--config", "roundtrip.toml", "--show-config"])
        .assert()
        .success();
}

#[test]
fn show_config_prints_a_webhook_without_its_token() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        concat!(
            "notify = [\"hook\"]\n\n",
            "[[notification]]\n",
            "name = \"hook\"\n",
            "type = \"webhook\"\n",
            "url = \"https://hooks.example.test/notify?room=ops\"\n",
            "token = \"s3cret-bearer\"\n\n",
            "[[targets]]\n",
            "ip = \"9.9.9.9\"\n",
        ),
    )
    .unwrap();

    monitor(directory.path())
        .arg("--show-config")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "url = \"https://hooks.example.test/notify?room=ops\"",
        ))
        .stdout(predicate::str::contains("token = \"<redacted>\""))
        .stdout(predicate::str::contains("s3cret-bearer").not());
}

#[test]
fn the_startup_log_names_a_webhook_by_host_only() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        concat!(
            "notify = [\"hook\"]\n\n",
            "[[notification]]\n",
            "name = \"hook\"\n",
            "type = \"webhook\"\n",
            "url = \"http://127.0.0.1:1/secret-path\"\n\n",
            "[[targets]]\n",
            "ip = \"9.9.9.9\"\n",
        ),
    )
    .unwrap();

    monitor(directory.path())
        .args([
            "monitor",
            "--database",
            "events.db",
            "--listen",
            "not-an-address",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "INFO  monitor notification channel hook (webhook) configured for http://127.0.0.1:1",
        ))
        .stderr(predicate::str::contains("secret-path").not());
}

#[test]
fn an_unknown_notify_name_fails_before_the_monitor_starts() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        concat!(
            "notify = [\"missing\"]\n\n",
            "[[notification]]\n",
            "name = \"audit\"\n",
            "type = \"logfile\"\n",
            "filename = \"audit.log\"\n\n",
            "[[targets]]\n",
            "ip = \"9.9.9.9\"\n",
        ),
    )
    .unwrap();

    monitor(directory.path())
        .args(["monitor", "--database", "events.db"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unknown notification channel 'missing'",
        ));
}

#[test]
fn a_channel_that_nothing_enables_is_only_a_warning() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        concat!(
            "[[notification]]\n",
            "name = \"audit\"\n",
            "type = \"logfile\"\n",
            "filename = \"audit.log\"\n\n",
            "[[targets]]\n",
            "ip = \"9.9.9.9\"\n",
        ),
    )
    .unwrap();

    monitor(directory.path())
        .args([
            "monitor",
            "--database",
            "events.db",
            "--listen",
            "not-an-address",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "WARN  notification notification channel audit (logfile) is declared but nothing enables it",
        ))
        .stderr(predicate::str::contains("invalid listen address"));
}

#[test]
fn two_channels_on_one_destination_fail_before_the_monitor_starts() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("network-monitor.toml"),
        concat!(
            "notify = [\"one\", \"two\"]\n\n",
            "[[notification]]\n",
            "name = \"one\"\n",
            "type = \"logfile\"\n",
            "filename = \"audit.log\"\n\n",
            "[[notification]]\n",
            "name = \"two\"\n",
            "type = \"logfile\"\n",
            "filename = \"./audit.log\"\n\n",
            "[[targets]]\n",
            "ip = \"9.9.9.9\"\n",
        ),
    )
    .unwrap();

    monitor(directory.path())
        .args(["monitor", "--database", "events.db"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("both write to"));
}
