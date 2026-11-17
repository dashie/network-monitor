//! The `manual` subcommand end to end: the exact embedded guide, the alias, and the
//! guarantee that it reads no configuration and writes no files.

use std::process::Command as ProcessCommand;

use assert_cmd::Command;
use predicates::prelude::*;

fn output(arg: &str) -> std::process::Output {
    ProcessCommand::new(assert_cmd::cargo::cargo_bin!("network-monitor"))
        .arg(arg)
        .output()
        .unwrap()
}

#[test]
fn manual_prints_the_embedded_guide() {
    let output = output("manual");

    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        std::fs::read("docs/USER-GUIDE.md").expect("the guide is on disk")
    );
    assert!(output.stdout.starts_with(b"# User Guide"));
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn man_is_an_alias() {
    let manual = output("manual");
    let man = output("man");

    assert!(manual.status.success());
    assert!(man.status.success());
    assert_eq!(manual.stdout, man.stdout);
}

#[test]
fn manual_reads_no_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("network-monitor.toml");
    std::fs::write(&invalid, "not = [valid").unwrap();

    ProcessCommand::new(assert_cmd::cargo::cargo_bin!("network-monitor"))
        .current_dir(directory.path())
        .arg("manual")
        .output()
        .map(|output| assert!(output.status.success()))
        .unwrap();

    assert_eq!(std::fs::read_to_string(&invalid).unwrap(), "not = [valid");
}

#[test]
fn manual_rejects_configuration_flags() {
    let directory = tempfile::tempdir().unwrap();
    let cases = [
        ("--config", Some("missing.toml")),
        ("--target", Some("1.1.1.1")),
        ("--database", Some("missing.db")),
        ("--show-config", None),
    ];

    for (flag, value) in cases {
        let mut command = Command::cargo_bin("network-monitor").unwrap();
        command
            .current_dir(directory.path())
            .arg("manual")
            .arg(flag);
        if let Some(value) = value {
            command.arg(value);
        }
        command.assert().failure().stderr(predicate::str::contains(
            "manual takes no arguments; it does not read a configuration",
        ));
    }
}

#[test]
fn manual_creates_no_files() {
    let directory = tempfile::tempdir().unwrap();

    ProcessCommand::new(assert_cmd::cargo::cargo_bin!("network-monitor"))
        .current_dir(directory.path())
        .arg("manual")
        .output()
        .map(|output| assert!(output.status.success()))
        .unwrap();

    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none(),
        "manual created a file"
    );
}

#[test]
fn manual_help_lists_the_command_and_alias() {
    Command::cargo_bin("network-monitor")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("manual"))
        .stdout(predicate::str::contains("alias: man"));

    for name in ["manual", "man"] {
        Command::cargo_bin("network-monitor")
            .unwrap()
            .args([name, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "Print the full user guide as Markdown",
            ));
    }
}

#[cfg(unix)]
#[test]
fn manual_survives_a_closed_pipe() {
    use std::io::Read;
    use std::process::Stdio;

    let mut child = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("network-monitor"))
        .arg("manual")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut buffer = [0u8; 64];
    stdout.read_exact(&mut buffer).unwrap();
    drop(stdout);

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("panicked"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
