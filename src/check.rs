//! The `check` component: verify a configuration without starting the monitor.
//!
//! The pipeline follows the order the process starts in, so the first `fail` line is the
//! condition that would have stopped it: the target list, the interfaces, the event
//! database, the listening address, the notification channels, and — only when asked —
//! one real probe per target. It owns no state: it opens a read-only store, opens and
//! closes each channel, and drops the listener it bound.

use std::io::ErrorKind;
use std::path::PathBuf;

use crate::config::{ChannelSettings, Config};
use crate::notify;
use crate::probe::{interface, Probe, Prober, Verdict};
use crate::snapshot;
use crate::store::Store;
use crate::web::{self, ServeError};

/// What `check` was asked to verify beyond a plain start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// The address the monitor would listen on, when one is given.
    pub listen: Option<String>,
    /// Perform one real probe per target.
    pub probe: bool,
    /// The active snapshot to report, when there is a configuration file.
    pub active: Option<PathBuf>,
}

/// The severity of one line of the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineLevel {
    Ok,
    Warn,
    Fail,
}

impl LineLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LineLevel::Ok => "ok",
            LineLevel::Warn => "warn",
            LineLevel::Fail => "fail",
        }
    }
}

/// One check and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub level: LineLevel,
    pub subject: String,
    pub detail: String,
}

impl Line {
    fn ok(subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Line {
            level: LineLevel::Ok,
            subject: subject.into(),
            detail: detail.into(),
        }
    }

    fn warn(subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Line {
            level: LineLevel::Warn,
            subject: subject.into(),
            detail: detail.into(),
        }
    }

    fn fail(subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Line {
            level: LineLevel::Fail,
            subject: subject.into(),
            detail: detail.into(),
        }
    }
}

/// The verification of one configuration: one line per check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub lines: Vec<Line>,
}

impl Report {
    /// Whether the configuration would not start.
    pub fn failed(&self) -> bool {
        self.lines.iter().any(|line| line.level == LineLevel::Fail)
    }

    /// One aligned line per check, the form the command prints.
    pub fn render(&self) -> String {
        let mut text = String::new();
        for line in &self.lines {
            text.push_str(&format!(
                "{:<5} {:<18} {}\n",
                line.level.as_str(),
                line.subject,
                line.detail
            ));
        }
        text
    }
}

/// Verify a configuration over the real prober and the real channels.
pub fn run(config: &Config, options: Options) -> Report {
    run_with(config, options, &Prober::new())
}

/// [`run`] over a caller-supplied prober, so the probe verdicts are testable.
fn run_with(config: &Config, options: Options, prober: &dyn Probe) -> Report {
    let mut lines = Vec::new();

    check_targets(config, &mut lines);
    check_interfaces(config, &mut lines);
    check_database(config, &mut lines);
    if let Some(address) = options.listen.as_deref() {
        check_listen(address, &mut lines);
    }
    check_channels(config, &mut lines);
    check_active(options.active.as_deref(), &mut lines);
    if options.probe {
        check_probes(config, prober, &mut lines);
    }

    Report { lines }
}

fn check_targets(config: &Config, lines: &mut Vec<Line>) {
    match config.require_targets() {
        Ok(()) => lines.push(Line::ok(
            "targets",
            format!("{} target(s)", config.targets.len()),
        )),
        Err(error) => lines.push(Line::fail("targets", error.to_string())),
    }
}

fn check_interfaces(config: &Config, lines: &mut Vec<Line>) {
    let mut checked = 0;
    for target in &config.targets {
        let Some(name) = &target.interface else {
            continue;
        };
        checked += 1;
        match interface::validate(name) {
            Ok(()) => lines.push(Line::ok("interface", format!("{} {name}", target.key()))),
            Err(reason) => lines.push(Line::fail(
                "interface",
                format!("{} {name}: {reason}", target.key()),
            )),
        }
    }
    if checked == 0 {
        lines.push(Line::ok("interface", "none configured"));
    }
}

fn check_database(config: &Config, lines: &mut Vec<Line>) {
    let path = &config.database;
    if !path.exists() {
        lines.push(Line::warn(
            "database",
            format!(
                "{} does not exist yet; the next start creates it",
                path.display()
            ),
        ));
        return;
    }
    match Store::open_ro(path) {
        Ok(_) => lines.push(Line::ok(
            "database",
            format!("{} (readable)", path.display()),
        )),
        Err(error) => lines.push(Line::fail("database", error.to_string())),
    }
}

fn check_listen(address: &str, lines: &mut Vec<Line>) {
    match web::bind(address) {
        Ok(listener) => {
            drop(listener);
            lines.push(Line::ok("listen", format!("{address} is available")));
        }
        Err(ServeError::Bind { source, .. }) if source.kind() == ErrorKind::AddrInUse => {
            lines.push(Line::warn(
                "listen",
                format!("{address} is in use — the running monitor?"),
            ));
        }
        Err(error) => lines.push(Line::fail("listen", error.to_string())),
    }
}

fn check_channels(config: &Config, lines: &mut Vec<Line>) {
    let declared = config.notify.channels.len();
    if declared == 0 {
        lines.push(Line::ok("channel", "none declared"));
        return;
    }
    match notify::verify(&config.notify) {
        Ok(()) => {
            lines.push(Line::ok(
                "channel",
                format!("{declared} channel(s) verified"),
            ));
            for channel in &config.notify.channels {
                if let ChannelSettings::LogFile(settings) = channel {
                    if settings.max_bytes == 0 {
                        lines.push(Line::warn(
                            format!("channel {}", settings.name),
                            format!(
                                "{} is appended without rotation; set max_bytes to bound it",
                                settings.filename.display()
                            ),
                        ));
                    }
                }
            }
        }
        Err(error) => lines.push(Line::fail(
            format!("channel {}", error.channel),
            error.reason,
        )),
    }
}

fn check_active(active: Option<&std::path::Path>, lines: &mut Vec<Line>) {
    match active.and_then(snapshot::describe) {
        Some(description) => lines.push(Line::ok("active", description)),
        None => lines.push(Line::warn("active", "no snapshot yet")),
    }
}

fn check_probes(config: &Config, prober: &dyn Probe, lines: &mut Vec<Line>) {
    for target in &config.targets {
        let key = target.key();
        match prober.probe(
            &target.endpoint,
            target.interface.as_deref(),
            target.timeout,
        ) {
            Ok(Verdict::Up { rtt }) => lines.push(Line::ok(
                "probe",
                format!("{key} up (rtt {:.3} ms)", rtt.as_secs_f64() * 1_000.0),
            )),
            Ok(Verdict::Down(failure)) => lines.push(Line::warn(
                "probe",
                format!("{key} down ({} — {})", failure.code(), failure.message()),
            )),
            Err(error) => lines.push(Line::fail("probe", format!("{key}: {error}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::path::Path;

    use super::*;
    use crate::config::{Config, FileConfig, FileNotification, FileTarget, Overrides};
    use crate::probe::fake::{down_with, up, FakeProbe};
    use crate::probe::{Code, Details, ProbeError};

    fn config(directory: &Path, targets: Option<Vec<FileTarget>>) -> Config {
        Config::resolve(
            Some(FileConfig {
                database: Some(directory.join("events.db").display().to_string()),
                targets,
                ..FileConfig::default()
            }),
            Overrides::default(),
        )
        .expect("a valid configuration")
    }

    fn icmp_target(ip: &str) -> FileTarget {
        FileTarget {
            ip: Some(ip.to_string()),
            ..FileTarget::default()
        }
    }

    fn report(config: &Config, options: Options) -> Report {
        run_with(config, options, &FakeProbe::always(up()))
    }

    fn find<'a>(report: &'a Report, subject: &str) -> &'a Line {
        report
            .lines
            .iter()
            .find(|line| line.subject == subject)
            .unwrap_or_else(|| panic!("no line for subject {subject}: {report:?}"))
    }

    #[test]
    fn a_logfile_without_rotation_is_a_warning_and_a_bounded_one_is_not() {
        let directory = tempfile::tempdir().unwrap();
        let unbounded = Config::resolve(
            Some(FileConfig {
                database: Some(directory.path().join("events.db").display().to_string()),
                notification: Some(vec![FileNotification {
                    name: Some("audit".to_string()),
                    kind: Some("logfile".to_string()),
                    filename: Some(directory.path().join("audit.log").display().to_string()),
                    ..FileNotification::default()
                }]),
                targets: Some(vec![icmp_target("127.0.0.1")]),
                ..FileConfig::default()
            }),
            Overrides::default(),
        )
        .expect("a valid configuration");

        let unbounded_report = report(&unbounded, Options::default());
        let line = find(&unbounded_report, "channel audit");
        assert_eq!(line.level, LineLevel::Warn, "{line:?}");
        assert!(line.detail.contains("max_bytes"), "{line:?}");
        assert!(
            !unbounded_report.failed(),
            "a warning never fails the check"
        );

        let bounded = Config::resolve(
            Some(FileConfig {
                database: Some(directory.path().join("events.db").display().to_string()),
                notification: Some(vec![FileNotification {
                    name: Some("audit".to_string()),
                    kind: Some("logfile".to_string()),
                    filename: Some(directory.path().join("bounded.log").display().to_string()),
                    max_bytes: Some(4096),
                    ..FileNotification::default()
                }]),
                targets: Some(vec![icmp_target("127.0.0.1")]),
                ..FileConfig::default()
            }),
            Overrides::default(),
        )
        .expect("a valid configuration");

        let bounded_report = report(&bounded, Options::default());
        assert!(
            bounded_report
                .lines
                .iter()
                .all(|line| !line.subject.starts_with("channel audit")),
            "a bounded file is not warned about: {bounded_report:?}"
        );
        assert!(!bounded_report.failed());
    }

    #[test]
    fn a_target_less_configuration_fails_on_the_target_list() {
        let directory = tempfile::tempdir().unwrap();
        let report = report(&config(directory.path(), None), Options::default());

        let line = find(&report, "targets");
        assert_eq!(line.level, LineLevel::Fail);
        assert!(line.detail.contains("no targets configured"));
        assert!(report.failed());
    }

    #[test]
    fn an_unknown_interface_fails_and_names_it() {
        let directory = tempfile::tempdir().unwrap();
        let settings = config(
            directory.path(),
            Some(vec![FileTarget {
                interface: Some("does-not-exist0".to_string()),
                ..icmp_target("127.0.0.1")
            }]),
        );

        let report = report(&settings, Options::default());
        let line = find(&report, "interface");
        assert_eq!(line.level, LineLevel::Fail);
        assert!(line.detail.contains("does-not-exist0"), "{line:?}");
        assert!(report.failed());
    }

    #[test]
    fn a_missing_database_is_only_a_warning() {
        let directory = tempfile::tempdir().unwrap();
        let report = report(
            &config(directory.path(), Some(vec![icmp_target("127.0.0.1")])),
            Options::default(),
        );

        let line = find(&report, "database");
        assert_eq!(line.level, LineLevel::Warn);
        assert!(line.detail.contains("does not exist yet"));
        assert!(!report.failed());
    }

    #[test]
    fn a_readable_database_is_ok() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("events.db");
        Store::open_rw(&database).expect("the test creates the database");
        let settings = config(directory.path(), Some(vec![icmp_target("127.0.0.1")]));

        let report = report(&settings, Options::default());
        let line = find(&report, "database");
        assert_eq!(line.level, LineLevel::Ok, "{line:?}");
        assert!(!report.failed());
    }

    #[test]
    fn a_bad_listen_address_fails() {
        let directory = tempfile::tempdir().unwrap();
        let report = report(
            &config(directory.path(), Some(vec![icmp_target("127.0.0.1")])),
            Options {
                listen: Some("not-an-address".to_string()),
                ..Options::default()
            },
        );

        let line = find(&report, "listen");
        assert_eq!(line.level, LineLevel::Fail);
        assert!(line.detail.contains("invalid listen address"));
    }

    #[test]
    fn an_address_in_use_is_only_a_warning() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();

        let report = report(
            &config(directory.path(), Some(vec![icmp_target("127.0.0.1")])),
            Options {
                listen: Some(address),
                ..Options::default()
            },
        );

        let line = find(&report, "listen");
        assert_eq!(line.level, LineLevel::Warn, "{line:?}");
        assert!(!report.failed());
    }

    #[test]
    fn a_channel_that_cannot_open_fails_and_names_it() {
        let directory = tempfile::tempdir().unwrap();
        let settings = Config::resolve(
            Some(FileConfig {
                database: Some(directory.path().join("events.db").display().to_string()),
                notification: Some(vec![FileNotification {
                    name: Some("audit".to_string()),
                    kind: Some("logfile".to_string()),
                    filename: Some("missing/audit.log".to_string()),
                    ..FileNotification::default()
                }]),
                targets: Some(vec![icmp_target("127.0.0.1")]),
                ..FileConfig::default()
            }),
            Overrides::default(),
        )
        .expect("a valid configuration");

        let report = report(&settings, Options::default());
        let line = find(&report, "channel audit");
        assert_eq!(line.level, LineLevel::Fail);
        assert!(line.detail.contains("cannot open"), "{line:?}");
    }

    #[test]
    fn a_report_of_ok_and_warn_lines_is_not_failed() {
        let directory = tempfile::tempdir().unwrap();
        let report = report(
            &config(directory.path(), Some(vec![icmp_target("127.0.0.1")])),
            Options::default(),
        );

        assert!(!report.failed());
        assert!(
            report
                .lines
                .iter()
                .all(|line| line.level != LineLevel::Fail),
            "{report:?}"
        );
    }

    #[test]
    fn a_probe_reports_up_as_ok_and_down_as_a_warning() {
        let directory = tempfile::tempdir().unwrap();
        let settings = config(directory.path(), Some(vec![icmp_target("127.0.0.1")]));
        let report = run_with(
            &settings,
            Options {
                probe: true,
                ..Options::default()
            },
            &FakeProbe::always(down_with(Code::IcmpNoReply)),
        );

        let line = find(&report, "probe");
        assert_eq!(
            line.level,
            LineLevel::Warn,
            "a down verdict is not a failure"
        );
        assert!(line.detail.contains("ICMP.NO_REPLY"));
        assert!(!report.failed());
    }

    #[test]
    fn a_probe_that_cannot_run_is_a_failure() {
        let directory = tempfile::tempdir().unwrap();
        let settings = config(directory.path(), Some(vec![icmp_target("127.0.0.1")]));
        let error = ProbeError::of(
            Code::IcmpSocket,
            &Details {
                reason: Some("operation not permitted"),
                ..Details::default()
            },
        );
        let report = run_with(
            &settings,
            Options {
                probe: true,
                ..Options::default()
            },
            &FakeProbe::scripted(vec![Err(error)]),
        );

        let line = find(&report, "probe");
        assert_eq!(line.level, LineLevel::Fail, "{line:?}");
        assert!(report.failed());
    }

    #[test]
    fn a_missing_snapshot_is_a_warning_with_no_failure() {
        let directory = tempfile::tempdir().unwrap();
        let active = directory.path().join("network-monitor.toml.active");
        let report = run_with(
            &config(directory.path(), Some(vec![icmp_target("127.0.0.1")])),
            Options {
                active: Some(active),
                ..Options::default()
            },
            &FakeProbe::always(up()),
        );

        let line = find(&report, "active");
        assert_eq!(line.level, LineLevel::Warn, "{line:?}");
        assert_eq!(line.detail, "no snapshot yet");
        assert!(!report.failed());
    }

    #[test]
    fn an_existing_snapshot_is_ok_with_its_path_and_age() {
        let directory = tempfile::tempdir().unwrap();
        let active = directory.path().join("network-monitor.toml.active");
        crate::snapshot::write(&active, b"interval_ms = 1000\n").unwrap();
        let report = run_with(
            &config(directory.path(), Some(vec![icmp_target("127.0.0.1")])),
            Options {
                active: Some(active.clone()),
                ..Options::default()
            },
            &FakeProbe::always(up()),
        );

        let line = find(&report, "active");
        assert_eq!(line.level, LineLevel::Ok, "{line:?}");
        assert!(
            line.detail.starts_with(&active.display().to_string()),
            "{line:?}"
        );
        assert!(line.detail.contains("ago"), "{line:?}");
    }

    #[test]
    fn the_rendered_line_is_aligned_and_one_line_per_check() {
        let report = Report {
            lines: vec![
                Line::ok("targets", "7 target(s)"),
                Line::fail("channel telegram", "Unauthorized"),
            ],
        };

        assert_eq!(
            report.render(),
            "ok    targets            7 target(s)\n\
             fail  channel telegram   Unauthorized\n"
        );
    }
}
