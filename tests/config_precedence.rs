//! INV-2: every configuration key has a flag, and the flag wins over the file.

use clap::Parser;

use network_monitor::cli::Cli;
use network_monitor::config::{Config, FileConfig, FileTarget, Overrides};

fn parse(arguments: &[&str]) -> Cli {
    let mut full = vec!["network-monitor"];
    full.extend_from_slice(arguments);
    Cli::try_parse_from(full).expect("the arguments are valid")
}

fn file_config() -> FileConfig {
    FileConfig {
        interval_ms: Some(1_000),
        timeout_ms: Some(500),
        database: Some("/tmp/from-file.db".to_string()),
        retention_days: Some(3),
        targets: Some(vec![FileTarget {
            ip: Some("1.1.1.1".to_string()),
            interface: Some("eth0".to_string()),
            label: Some("from-file".to_string()),
            ..FileTarget::default()
        }]),
        ..FileConfig::default()
    }
}

#[test]
fn the_file_is_used_when_no_flag_is_given() {
    let cli = parse(&[]);
    let config = Config::resolve(Some(file_config()), cli.overrides().unwrap()).unwrap();

    assert_eq!(config.interval.as_millis(), 1_000);
    assert_eq!(config.timeout.as_millis(), 500);
    assert_eq!(config.database.to_string_lossy(), "/tmp/from-file.db");
    assert_eq!(config.retention_days, 3);
    assert_eq!(config.targets.len(), 1);
    assert_eq!(config.targets[0].label.as_deref(), Some("from-file"));
}

#[test]
fn every_key_has_a_flag_that_wins_over_the_file() {
    let cli = parse(&[
        "--interval-ms",
        "2500",
        "--timeout-ms",
        "750",
        "--database",
        "/tmp/from-flag.db",
        "--retention-days",
        "9",
        "--target",
        "8.8.8.8,interface=eth1,label=from-flag",
    ]);
    let config = Config::resolve(Some(file_config()), cli.overrides().unwrap()).unwrap();

    assert_eq!(config.interval.as_millis(), 2_500);
    assert_eq!(config.timeout.as_millis(), 750);
    assert_eq!(config.database.to_string_lossy(), "/tmp/from-flag.db");
    assert_eq!(config.retention_days, 9);

    // --target replaces the file list instead of adding to it.
    assert_eq!(config.targets.len(), 1);
    assert_eq!(config.targets[0].key(), "icmp://8.8.8.8");
    assert_eq!(config.targets[0].interface.as_deref(), Some("eth1"));
    assert_eq!(config.targets[0].label.as_deref(), Some("from-flag"));
}

#[test]
fn built_in_defaults_apply_below_the_file() {
    let cli = parse(&[]);
    let config = Config::resolve(
        Some(FileConfig {
            targets: Some(vec![FileTarget {
                ip: Some("1.1.1.1".to_string()),
                ..FileTarget::default()
            }]),
            ..FileConfig::default()
        }),
        cli.overrides().unwrap(),
    )
    .unwrap();

    assert_eq!(config.interval.as_millis(), 5_000);
    assert_eq!(config.timeout.as_millis(), 2_000);
    assert_eq!(config.database.to_string_lossy(), "network-monitor.db");
    assert_eq!(config.retention_days, 0);
}

#[test]
fn a_partial_file_keeps_the_defaults_for_the_missing_keys() {
    let cli = parse(&[]);
    let config = Config::resolve(
        Some(FileConfig {
            interval_ms: Some(10_000),
            ..FileConfig::default()
        }),
        cli.overrides().unwrap(),
    )
    .unwrap();

    assert_eq!(config.interval.as_millis(), 10_000);
    assert_eq!(config.timeout.as_millis(), 2_000);
}

#[test]
fn the_merge_is_the_same_without_a_file() {
    let cli = parse(&["--interval-ms", "1234"]);
    let config = Config::resolve(None, cli.overrides().unwrap()).unwrap();
    assert_eq!(config.interval.as_millis(), 1_234);
    assert!(config.targets.is_empty());
}

#[test]
fn overrides_without_values_do_not_change_anything() {
    let cli = parse(&[]);
    let overrides: Overrides = cli.overrides().unwrap();
    assert!(overrides.interval_ms.is_none());
    assert!(overrides.targets.is_none());
}

/// The per-target keys exist as flags too, and they carry the method with them.
#[test]
fn the_target_keys_of_every_method_are_reachable_from_the_flag() {
    let cli = parse(&[
        "--target",
        "tcp:1.1.1.1:443,interface=eth0,label=from-flag,timeout_ms=1500,interval_ms=900",
        "--target",
        "dns:example.com@9.9.9.9,record=AAAA",
        "--target",
        "http:https://example.com/health,expect=204",
        "--target",
        "icmp:1.0.0.1",
    ]);
    let config = Config::resolve(None, cli.overrides().unwrap()).unwrap();

    assert_eq!(
        config
            .targets
            .iter()
            .map(|target| target.key())
            .collect::<Vec<_>>(),
        vec![
            "tcp://1.1.1.1:443",
            "dns://example.com@9.9.9.9",
            "https://example.com/health",
            "icmp://1.0.0.1",
        ]
    );
    assert_eq!(config.targets[0].interface.as_deref(), Some("eth0"));
    assert_eq!(config.targets[0].label.as_deref(), Some("from-flag"));
    assert_eq!(config.targets[0].timeout.as_millis(), 1_500);
    assert_eq!(config.targets[0].interval.as_millis(), 900);
    assert_eq!(config.timeout.as_millis(), 2_000, "the global default");
}
