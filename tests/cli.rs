//! The command-line contract: exit codes, effective configuration, query output.

use assert_cmd::Command;
use predicates::prelude::*;

fn monitor(directory: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("uptime-monitor").unwrap();
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
        .stdout(predicate::str::contains("database = \"uptime-monitor.db\""))
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
        directory.path().join("uptime-monitor.toml"),
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
fn a_file_path_is_resolved_beside_the_configuration_file() {
    let root = tempfile::tempdir().unwrap();
    let conf = root.path().join("conf");
    std::fs::create_dir(&conf).unwrap();
    std::fs::write(
        conf.join("uptime-monitor.toml"),
        concat!(
            "notify = [\"audit\"]\n",
            "database = \"events.db\"\n\n",
            "[[notification]]\n",
            "name = \"audit\"\n",
            "type = \"logfile\"\n",
            "filename = \"logs/audit.log\"\n\n",
            "[[targets]]\n",
            "ip = \"127.0.0.1\"\n",
        ),
    )
    .unwrap();

    monitor(root.path())
        .args(["--config", "conf/uptime-monitor.toml", "--show-config"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "database = \"{}\"",
            conf.join("events.db").display()
        )))
        .stdout(predicate::str::contains(format!(
            "filename = \"{}\"",
            conf.join("logs/audit.log").display()
        )));
}

#[test]
fn check_creates_the_log_beside_the_configuration_file() {
    let root = tempfile::tempdir().unwrap();
    let conf = root.path().join("conf");
    std::fs::create_dir_all(conf.join("logs")).unwrap();
    std::fs::write(
        conf.join("uptime-monitor.toml"),
        concat!(
            "notify = [\"audit\"]\n",
            "database = \"events.db\"\n\n",
            "[[notification]]\n",
            "name = \"audit\"\n",
            "type = \"logfile\"\n",
            "filename = \"logs/audit.log\"\n\n",
            "[[targets]]\n",
            "ip = \"127.0.0.1\"\n",
        ),
    )
    .unwrap();

    monitor(root.path())
        .args(["check", "--config", "conf/uptime-monitor.toml"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ok    channel"));

    assert!(
        conf.join("logs/audit.log").is_file(),
        "the log belongs beside the configuration"
    );
    assert!(
        !root.path().join("logs/audit.log").exists(),
        "nothing is created in the working directory"
    );
    assert!(!root.path().join("events.db").exists());
}

#[test]
fn an_unknown_key_in_the_file_fails() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
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
        directory.path().join("uptime-monitor.toml"),
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
            "INFO  monitor reading configuration from uptime-monitor.toml",
        ));
}

#[test]
fn the_startup_log_names_the_configured_chat() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
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
        directory.path().join("uptime-monitor.toml"),
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
        directory.path().join("uptime-monitor.toml"),
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
        directory.path().join("uptime-monitor.toml"),
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
        directory.path().join("uptime-monitor.toml"),
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
        directory.path().join("uptime-monitor.toml"),
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
        directory.path().join("uptime-monitor.toml"),
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

#[test]
fn check_reports_ok_and_warn_lines_and_exits_zero() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        "database = \"events.db\"\n\n[[targets]]\nip = \"127.0.0.1\"\n",
    )
    .unwrap();

    monitor(directory.path())
        .args(["check", "--config", "uptime-monitor.toml"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ok    targets"))
        .stdout(predicate::str::contains("warn  database"))
        .stdout(predicate::str::contains("ok    channel"))
        .stdout(predicate::str::contains("fail ").not());

    assert!(
        !directory.path().join("events.db").exists(),
        "check never creates the event database"
    );
}

#[test]
fn check_names_an_unknown_interface_and_exits_one() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        "database = \"events.db\"\n\n[[targets]]\nip = \"127.0.0.1\"\ninterface = \"does-not-exist0\"\n",
    )
    .unwrap();

    monitor(directory.path())
        .args(["check", "--config", "uptime-monitor.toml"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("fail  interface"))
        .stdout(predicate::str::contains("does-not-exist0"));
}

#[test]
fn check_reports_a_bad_listen_address_on_stdout() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        "database = \"events.db\"\n\n[[targets]]\nip = \"127.0.0.1\"\n",
    )
    .unwrap();

    monitor(directory.path())
        .args([
            "check",
            "--config",
            "uptime-monitor.toml",
            "--listen",
            "not-an-address",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("fail  listen"))
        .stdout(predicate::str::contains("invalid listen address"));
}

#[test]
fn the_configured_listen_address_is_checked() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        "listen = \"127.0.0.1:0\"\ndatabase = \"events.db\"\n\n[[targets]]\nip = \"127.0.0.1\"\n",
    )
    .unwrap();

    monitor(directory.path())
        .args(["check", "--config", "uptime-monitor.toml"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "ok    listen             127.0.0.1:0 is available",
        ));
}

#[test]
fn the_monitor_listen_flag_overrides_the_configured_address() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        "listen = \"127.0.0.1:1\"\ndatabase = \"events.db\"\n\n[[targets]]\nip = \"127.0.0.1\"\n",
    )
    .unwrap();

    monitor(directory.path())
        .args([
            "check",
            "--config",
            "uptime-monitor.toml",
            "--listen",
            "127.0.0.1:0",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("127.0.0.1:0 is available"))
        .stdout(predicate::str::contains("127.0.0.1:1 is available").not());
}

#[test]
fn show_config_prints_the_merged_listen_address() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args([
            "monitor",
            "--listen",
            "127.0.0.1:8642",
            "--show-config",
            "--target",
            "1.1.1.1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("listen = \"127.0.0.1:8642\""));

    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        "listen = \"127.0.0.1:8643\"\n\n[[targets]]\nip = \"1.1.1.1\"\n",
    )
    .unwrap();
    monitor(directory.path())
        .args(["--show-config"])
        .assert()
        .success()
        .stdout(predicate::str::contains("listen = \"127.0.0.1:8643\""));
}

// --- `monitor --try`: the candidate decision table -------------------------------

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command as StdCommand, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use uptime_monitor::store::{EventFilter, Store};

/// A spawned monitor that never outlives the test, even when an assertion fails.
struct Running {
    child: Option<Child>,
}

impl Running {
    fn spawn(directory: &Path, args: &[&str], log: &Path) -> Running {
        let child = StdCommand::new(env!("CARGO_BIN_EXE_uptime-monitor"))
            .current_dir(directory)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                std::fs::File::create(log).expect("the test creates the log file"),
            ))
            .spawn()
            .expect("the binary starts");
        Running { child: Some(child) }
    }

    fn stop(mut self) -> ExitStatus {
        #[cfg(unix)]
        unsafe {
            libc::kill(
                self.child.as_ref().unwrap().id() as libc::pid_t,
                libc::SIGTERM,
            );
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.as_mut().unwrap().kill();
        }
        let status = self
            .child
            .as_mut()
            .unwrap()
            .wait()
            .expect("the monitor exits");
        self.child = None;
        status
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn write_main_config(path: &Path, port: u16) {
    std::fs::write(
        path,
        format!(
            "interval_ms = 50\ntimeout_ms = 500\ndatabase = \"events.db\"\n\
             retention_days = 0\n\n[[targets]]\nmethod = \"tcp\"\n\
             host = \"127.0.0.1\"\nport = {port}\nlabel = \"main\"\n"
        ),
    )
    .unwrap();
}

fn write_candidate(path: &Path, port: u16, extra: &str) {
    std::fs::write(
        path,
        format!(
            "interval_ms = 50\ntimeout_ms = 500\ndatabase = \"events.db\"\n\
             retention_days = 0\n\n[[targets]]\nmethod = \"tcp\"\n\
             host = \"127.0.0.1\"\nport = {port}\nlabel = \"candidate\"\n{extra}"
        ),
    )
    .unwrap();
}

fn wait_for_events(database: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(store) = Store::open_ro(database) {
            if let Ok(events) = store.events(&EventFilter::default()) {
                if !events.is_empty() {
                    return;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "no event in {} within {timeout:?}",
            database.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn loopback_listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").expect("the test binds a loopback port")
}

#[test]
fn the_monitor_serves_the_page_on_the_configured_address() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        format!(
            "interval_ms = 100\ntimeout_ms = 200\ndatabase = \"events.db\"\n\
             listen = \"127.0.0.1:{port}\"\n\n[[targets]]\nmethod = \"tcp\"\n\
             host = \"127.0.0.1\"\nport = 9\nlabel = \"closed\"\n"
        ),
    )
    .unwrap();

    let log = directory.path().join("monitor.log");
    let running = Running::spawn(directory.path(), &["monitor"], &log);
    let status = wait_for_page(port, Duration::from_secs(5));
    running.stop();

    assert!(
        status.contains("200") || status.contains("503"),
        "status: {status}"
    );
    assert!(
        log_text(&log).contains(&format!("web interface ready on http://127.0.0.1:{port}/")),
        "log: {}",
        log_text(&log)
    );
}

fn write_listen_config(path: &Path, port: u16) {
    std::fs::write(
        path,
        format!(
            "interval_ms = 100\ntimeout_ms = 200\ndatabase = \"events.db\"\n\
             listen = \"127.0.0.1:{port}\"\n\n[[targets]]\nmethod = \"tcp\"\n\
             host = \"127.0.0.1\"\nport = 9\nlabel = \"closed\"\n"
        ),
    )
    .unwrap();
}

#[test]
fn a_changed_listen_address_needs_a_restart() {
    let directory = tempfile::tempdir().unwrap();
    let first = loopback_listener();
    let first_port = first.local_addr().unwrap().port();
    drop(first);
    let second = loopback_listener();
    let second_port = second.local_addr().unwrap().port();
    drop(second);
    assert_ne!(first_port, second_port);

    let config = directory.path().join("uptime-monitor.toml");
    write_listen_config(&config, first_port);
    let log = directory.path().join("monitor.log");
    let running = Running::spawn(directory.path(), &["monitor"], &log);
    wait_for_page(first_port, Duration::from_secs(5));

    write_listen_config(&config, second_port);
    let expected =
        format!("listen changed to 127.0.0.1:{second_port}; restart the monitor to use it");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !log_text(&log).contains(&expected) {
        assert!(
            Instant::now() < deadline,
            "no reload warning in: {}",
            log_text(&log)
        );
        thread::sleep(Duration::from_millis(50));
    }

    let status = wait_for_page(first_port, Duration::from_secs(2));
    assert!(
        status.contains("200") || status.contains("503"),
        "status: {status}"
    );
    assert!(running.stop().success());
}

/// Wait until the page answers on `port`, then return the status line.
fn wait_for_page(port: u16, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            stream
                .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .expect("the request is written");
            let mut answer = String::new();
            stream
                .read_to_string(&mut answer)
                .expect("the answer is read");
            if let Some(status) = answer.lines().next() {
                if status.starts_with("HTTP/1.") {
                    return status.to_string();
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "no page on 127.0.0.1:{port} within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn log_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn backup_files(directory: &Path) -> Vec<String> {
    std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".bak"))
        .collect()
}

#[test]
fn try_without_a_config_is_a_usage_error() {
    let directory = tempfile::tempdir().unwrap();

    monitor(directory.path())
        .args(["monitor", "--try", "next.toml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--try requires --config"));
}

#[test]
fn try_naming_the_main_file_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    write_main_config(&directory.path().join("main.toml"), port);

    monitor(directory.path())
        .args(["monitor", "--config", "./main.toml", "--try", "main.toml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--try must name a different file from --config",
        ));
}

#[test]
fn an_absent_candidate_is_a_quiet_no_op() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    write_main_config(&directory.path().join("main.toml"), port);
    let log = directory.path().join("stderr.log");

    let running = Running::spawn(
        directory.path(),
        &["monitor", "--config", "main.toml", "--try", "next.toml"],
        &log,
    );
    wait_for_events(&directory.path().join("events.db"), Duration::from_secs(10));
    let status = running.stop();

    assert!(status.success(), "a quiet no-op exits 0: {status:?}");
    assert!(
        log_text(&log).contains("no candidate configuration at next.toml; using main.toml"),
        "unexpected log: {}",
        log_text(&log)
    );
}

#[test]
fn an_unreadable_candidate_logs_and_monitors_the_main_file() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    write_main_config(&directory.path().join("main.toml"), port);
    // A directory is unreadable as a file for every user, root included.
    std::fs::create_dir(directory.path().join("next.toml")).unwrap();
    let log = directory.path().join("stderr.log");

    let running = Running::spawn(
        directory.path(),
        &["monitor", "--config", "main.toml", "--try", "next.toml"],
        &log,
    );
    wait_for_events(&directory.path().join("events.db"), Duration::from_secs(10));
    let status = running.stop();

    assert!(status.success(), "the monitor keeps running: {status:?}");
    let text = log_text(&log);
    assert!(
        text.contains("cannot read the candidate configuration next.toml"),
        "unexpected log: {text}"
    );
    assert!(text.contains("main.toml stays in effect"), "{text}");
}

#[test]
fn a_candidate_that_fails_the_check_is_reported_and_main_keeps_running() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    let main = directory.path().join("main.toml");
    write_main_config(&main, port);
    let original = std::fs::read(&main).unwrap();
    write_candidate(
        &directory.path().join("next.toml"),
        port,
        "interface = \"does-not-exist0\"\n",
    );
    let log = directory.path().join("stderr.log");

    let running = Running::spawn(
        directory.path(),
        &["monitor", "--config", "main.toml", "--try", "next.toml"],
        &log,
    );
    wait_for_events(&directory.path().join("events.db"), Duration::from_secs(10));
    let status = running.stop();

    assert!(status.success(), "the monitor keeps running: {status:?}");
    let text = log_text(&log);
    assert!(
        text.contains("candidate configuration next.toml: interface:"),
        "unexpected log: {text}"
    );
    assert!(text.contains("main.toml stays in effect"), "{text}");
    assert_eq!(
        std::fs::read(&main).unwrap(),
        original,
        "a rejected candidate never touches the main file"
    );
}

#[test]
fn an_identical_candidate_promotes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    let main = directory.path().join("main.toml");
    write_main_config(&main, port);
    std::fs::copy(&main, directory.path().join("next.toml")).unwrap();
    let log = directory.path().join("stderr.log");

    let running = Running::spawn(
        directory.path(),
        &["monitor", "--config", "main.toml", "--try", "next.toml"],
        &log,
    );
    wait_for_events(&directory.path().join("events.db"), Duration::from_secs(10));
    let status = running.stop();

    assert!(status.success(), "the monitor keeps running: {status:?}");
    assert!(
        log_text(&log).contains("matches main.toml; nothing to promote"),
        "unexpected log: {}",
        log_text(&log)
    );
    assert!(
        backup_files(directory.path()).is_empty(),
        "no backup for a no-op: {:?}",
        backup_files(directory.path())
    );
}

#[test]
fn fallback_to_active_starts_from_the_snapshot_when_main_is_broken() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        "interval_ms = 30\nbroken = [\n",
    )
    .unwrap();
    write_main_config(&directory.path().join("uptime-monitor.toml.active"), port);
    let log = directory.path().join("stderr.log");

    let running = Running::spawn(
        directory.path(),
        &[
            "monitor",
            "--config",
            "uptime-monitor.toml",
            "--fallback-to-active",
        ],
        &log,
    );
    wait_for_events(&directory.path().join("events.db"), Duration::from_secs(10));
    let status = running.stop();

    assert!(
        status.success(),
        "the snapshot carries the start: {status:?}"
    );
    let text = log_text(&log);
    assert!(
        text.contains("starting from the active snapshot"),
        "unexpected log: {text}"
    );
    assert!(text.contains("uptime-monitor.toml.active"), "{text}");
}

#[test]
fn a_broken_main_without_the_flag_names_the_snapshot_and_fails() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        directory.path().join("uptime-monitor.toml"),
        "interval_ms = 30\nbroken = [\n",
    )
    .unwrap();
    write_main_config(&directory.path().join("uptime-monitor.toml.active"), port);

    monitor(directory.path())
        .args(["monitor", "--config", "uptime-monitor.toml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the last configuration that worked is",
        ))
        .stderr(predicate::str::contains("uptime-monitor.toml.active"));
}

#[test]
fn fallback_to_active_uses_the_snapshot_when_main_fails_preflight() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        directory.path().join("main.toml"),
        format!(
            "interval_ms = 50\ntimeout_ms = 500\ndatabase = \"events.db\"\n\n\
             [[targets]]\nmethod = \"tcp\"\nhost = \"127.0.0.1\"\nport = {port}\n\
             interface = \"does-not-exist0\"\n"
        ),
    )
    .unwrap();
    write_main_config(&directory.path().join("main.toml.active"), port);
    let log = directory.path().join("stderr.log");

    let running = Running::spawn(
        directory.path(),
        &["monitor", "--config", "main.toml", "--fallback-to-active"],
        &log,
    );
    wait_for_events(&directory.path().join("events.db"), Duration::from_secs(10));
    let status = running.stop();

    assert!(
        status.success(),
        "the snapshot carries the start: {status:?}"
    );
    let text = log_text(&log);
    assert!(
        text.contains("starting from the active snapshot"),
        "unexpected log: {text}"
    );
    assert!(text.contains("does-not-exist0"), "{text}");
    assert!(text.contains("main.toml.active"), "{text}");
}

#[test]
fn fallback_to_active_uses_the_snapshot_when_main_has_no_targets() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        directory.path().join("main.toml"),
        "interval_ms = 50\ntimeout_ms = 500\ndatabase = \"events.db\"\n",
    )
    .unwrap();
    write_main_config(&directory.path().join("main.toml.active"), port);
    let log = directory.path().join("stderr.log");

    let running = Running::spawn(
        directory.path(),
        &["monitor", "--config", "main.toml", "--fallback-to-active"],
        &log,
    );
    wait_for_events(&directory.path().join("events.db"), Duration::from_secs(10));
    let status = running.stop();

    assert!(
        status.success(),
        "the snapshot carries the start: {status:?}"
    );
    let text = log_text(&log);
    assert!(
        text.contains("cannot use main.toml: targets: no targets configured"),
        "unexpected log: {text}"
    );
    assert!(text.contains("starting from the active snapshot"), "{text}");
}

#[test]
fn fallback_to_active_finds_a_snapshot_without_a_main_file() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    write_main_config(&directory.path().join("uptime-monitor.toml.active"), port);
    let log = directory.path().join("stderr.log");

    let running = Running::spawn(directory.path(), &["monitor", "--fallback-to-active"], &log);
    wait_for_events(&directory.path().join("events.db"), Duration::from_secs(10));
    let status = running.stop();

    assert!(
        status.success(),
        "the snapshot carries the start: {status:?}"
    );
    assert!(
        log_text(&log).contains("starting from the active snapshot"),
        "unexpected log: {}",
        log_text(&log)
    );
}

#[test]
fn a_failure_with_only_a_snapshot_names_it_and_its_age() {
    let directory = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let port = listener.local_addr().unwrap().port();
    write_main_config(&directory.path().join("uptime-monitor.toml.active"), port);

    monitor(directory.path())
        .arg("monitor")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the last configuration that worked is",
        ))
        .stderr(predicate::str::contains("uptime-monitor.toml.active"));
}

#[test]
fn the_binary_reports_its_name() {
    Command::cargo_bin("uptime-monitor")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("uptime-monitor "));

    Command::cargo_bin("uptime-monitor")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: uptime-monitor"));
}
