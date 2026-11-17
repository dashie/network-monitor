//! The monitoring loop: one interval loop per target, one shared store.

pub mod state;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::config::{Config, Overrides, Target};
use crate::logging;
use crate::notify::{self, Cause, Message, Notifier, TargetRef};
use crate::probe::{Probe, ProbeError, Verdict};
use crate::promote::Candidate;
use crate::reload::{self, Change, Watcher};
use crate::shutdown::{self, Shutdown};
use crate::store::{Event, Store, StoreError, TargetState};
use crate::time::now_ms;

use state::Transition;

#[derive(Debug, Error)]
pub enum RunError {
    #[error("no targets configured; add [[targets]] entries or use --target")]
    NoTargets,
    #[error("{0}")]
    Store(#[from] StoreError),
    #[error("{0}")]
    Notification(#[from] notify::StartError),
    #[error("the event store is unavailable")]
    Lock,
    #[error("a monitor thread stopped unexpectedly")]
    Panic,
}

/// Where the configuration is re-read from while monitoring runs.
#[derive(Debug, Clone)]
pub struct Reload {
    pub path: PathBuf,
    pub overrides: Overrides,
}

/// What a start carries beyond the configuration: the opening announcement and an
/// optional candidate waiting for the first probe round.
#[derive(Debug, Clone, Default)]
pub struct Start {
    pub startup: Option<Message>,
    pub promotion: Option<Candidate>,
    /// The main configuration, kept so a candidate that cannot probe is swapped back.
    pub fallback: Option<Config>,
    /// The active snapshot to publish once the first probe round is clean.
    pub snapshot: Option<crate::snapshot::Snapshot>,
}

struct Worker {
    stop: Shutdown,
    handle: thread::JoinHandle<Result<(), RunError>>,
}

/// The first outcome of every target of a start, reported once per target.
///
/// The gate that arms the snapshot and the promotion: it is complete when every target
/// of the starting configuration has probed once, and it records whether any of those
/// first probes was a local failure.
#[derive(Debug)]
struct FirstRound {
    expected: usize,
    reported: Mutex<HashSet<String>>,
    local_failure: AtomicBool,
    first_failure: Mutex<Option<String>>,
}

impl FirstRound {
    fn new(expected: usize) -> Self {
        FirstRound {
            expected,
            reported: Mutex::new(HashSet::new()),
            local_failure: AtomicBool::new(false),
            first_failure: Mutex::new(None),
        }
    }

    fn record(&self, target: &str, outcome: &Result<Verdict, ProbeError>) {
        if let Err(error) = outcome {
            self.local_failure.store(true, Ordering::SeqCst);
            let mut first = self
                .first_failure
                .lock()
                .expect("the first-failure lock is not poisoned");
            if first.is_none() {
                *first = Some(format!("{target}: {error}"));
            }
        }
        self.reported
            .lock()
            .expect("the first-round lock is not poisoned")
            .insert(target.to_string());
    }

    fn complete(&self) -> bool {
        self.reported
            .lock()
            .expect("the first-round lock is not poisoned")
            .len()
            >= self.expected
    }

    fn local_failure(&self) -> Option<String> {
        self.first_failure
            .lock()
            .expect("the first-failure lock is not poisoned")
            .clone()
    }
}

/// Owns the current configuration, the workers that apply it, and the reload logic.
struct Supervisor {
    config: Config,
    overrides: Overrides,
    prober: Arc<dyn Probe>,
    store: Arc<Mutex<Store>>,
    notifier: Arc<dyn Notifier>,
    retention_days: Arc<AtomicU64>,
    first_round: Arc<FirstRound>,
    workers: Vec<Worker>,
}

impl Supervisor {
    fn start_workers(&mut self) -> Result<(), RunError> {
        let initial_states = {
            let mut store = self.store.lock().map_err(|_| RunError::Lock)?;
            // The page orders the lanes by the priority of this configuration. A target
            // seen before is updated here, so a change takes effect on the page at once
            // instead of on its next transition; a target seen for the first time carries
            // its own on the event that creates its row.
            let priorities: Vec<(String, Option<i64>)> = self
                .config
                .targets
                .iter()
                .map(|target| (target.key(), target.priority))
                .collect();
            store.set_priorities(&priorities)?;
            store.load_states()?
        };

        self.workers.clear();
        for target in &self.config.targets {
            let target = target.clone();
            let prober = Arc::clone(&self.prober);
            let store = Arc::clone(&self.store);
            let notifier = Arc::clone(&self.notifier);
            let stop = Shutdown::new();
            let worker_stop = stop.clone();
            let previous = initial_states
                .get(&target.key())
                .copied()
                .unwrap_or(TargetState::Unknown);
            let first_round = Arc::clone(&self.first_round);

            let handle = thread::spawn(move || {
                monitor_target(
                    target,
                    prober,
                    store,
                    notifier,
                    previous,
                    worker_stop,
                    first_round,
                )
            });
            self.workers.push(Worker { stop, handle });
        }
        Ok(())
    }

    fn stop_workers(&mut self) {
        for worker in &self.workers {
            worker.stop.request();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.handle.join();
        }
    }

    /// Report a worker that exited on its own, which only happens on a store failure.
    fn reap(&mut self) -> Option<RunError> {
        let mut failure = None;
        let mut index = 0;
        while index < self.workers.len() {
            if self.workers[index].handle.is_finished() {
                let worker = self.workers.remove(index);
                match worker.handle.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => failure = failure.or(Some(error)),
                    Err(_) => failure = failure.or(Some(RunError::Panic)),
                }
            } else {
                index += 1;
            }
        }
        failure
    }

    /// Adopt `next`: stop the workers, keep the new configuration, start them again.
    fn swap(&mut self, next: Config, detail: &str) -> Result<(), RunError> {
        self.stop_workers();
        self.config = next;
        self.retention_days
            .store(self.config.retention_days, Ordering::Relaxed);
        self.start_workers()?;
        logging::info(None, detail);
        Ok(())
    }

    /// The recovery suffix a rejected reload states: the snapshot and its age.
    fn recovery_hint(path: &Path) -> String {
        crate::snapshot::describe(&crate::snapshot::path_for(path))
            .map(|description| format!("; the last configuration that worked is {description}"))
            .unwrap_or_default()
    }

    /// Validate a change and, when it is usable, apply it without a restart.
    fn apply(&mut self, change: Change, path: &Path) -> Result<(), RunError> {
        let bytes = match change {
            Change::Content(bytes) => bytes,
            Change::ReadError(reason) => {
                logging::warn(
                    None,
                    &format!(
                        "cannot read the configuration file {}: {reason}; keeping the current configuration{}",
                        path.display(),
                        Supervisor::recovery_hint(path)
                    ),
                );
                return Ok(());
            }
        };

        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error) => {
                logging::warn(
                    None,
                    &format!(
                        "configuration file {} is not valid UTF-8: {error}; keeping the current configuration{}",
                        path.display(),
                        Supervisor::recovery_hint(path)
                    ),
                );
                return Ok(());
            }
        };

        let file = match crate::config::parse_at(text, path) {
            Ok(file) => file,
            Err(error) => {
                logging::warn(
                    None,
                    &format!(
                        "configuration reload rejected: {error}; keeping the current configuration{}",
                        Supervisor::recovery_hint(path)
                    ),
                );
                self.notifier
                    .send(Message::config_reload(false, error.to_string(), now_ms()));
                return Ok(());
            }
        };

        let mut next = match Config::resolve(Some(file), self.overrides.clone()) {
            Ok(config) => config,
            Err(error) => {
                logging::warn(
                    None,
                    &format!(
                        "configuration reload rejected: {error}; keeping the current configuration{}",
                        Supervisor::recovery_hint(path)
                    ),
                );
                self.notifier
                    .send(Message::config_reload(false, error.to_string(), now_ms()));
                return Ok(());
            }
        };

        if next.targets.is_empty() {
            logging::warn(
                None,
                &format!(
                    "configuration reload rejected: no targets configured; keeping the current configuration{}",
                    Supervisor::recovery_hint(path)
                ),
            );
            self.notifier.send(Message::config_reload(
                false,
                "no targets configured".to_string(),
                now_ms(),
            ));
            return Ok(());
        }

        if next.database != self.config.database {
            logging::warn(
                None,
                &format!(
                    "database changed to {}; restart the monitor to use it",
                    next.database.display()
                ),
            );
            next.database = self.config.database.clone();
        }

        if next.listen != self.config.listen {
            logging::warn(
                None,
                &format!(
                    "listen changed to {}; restart the monitor to use it",
                    next.listen.as_deref().unwrap_or("no page")
                ),
            );
            next.listen = self.config.listen.clone();
        }

        let notify_changed = next.notify != self.config.notify;
        if notify_changed {
            logging::warn(
                None,
                "notification settings changed; restart the monitor to use them",
            );
            next.notify = self.config.notify.clone();
        }

        let log_detail = format!(
            "configuration updated from {} ({} target(s), interval {} ms, timeout {} ms)",
            path.display(),
            next.targets.len(),
            next.interval.as_millis(),
            next.timeout.as_millis()
        );
        self.swap(next, &log_detail)?;

        let mut detail = format!(
            "{} ({} target(s), interval {} ms, timeout {} ms)",
            path.display(),
            self.config.targets.len(),
            self.config.interval.as_millis(),
            self.config.timeout.as_millis()
        );
        if notify_changed {
            detail.push_str("; notification settings need a restart");
        }
        self.notifier
            .send(Message::config_reload(true, detail, now_ms()));
        Ok(())
    }
}

/// The message every chosen channel receives once the monitor is ready, so a start is
/// visible where the incidents are: the configuration it read, the targets it started,
/// and the address of the page when it serves one.
pub fn startup_event(config: &Config, source: Option<&Path>, serving: Option<&str>) -> Message {
    Message::startup(
        source.map(|path| path.display().to_string()),
        config.targets.iter().map(Target::key).collect(),
        serving.map(|address| format!("http://{address}/")),
        now_ms(),
    )
}

/// Probe every target until shutdown, persisting transitions.
///
/// One delivery thread runs per declared channel. Without a channel no thread is
/// started and the process makes no outbound connection beyond the probes. `startup`,
/// when present, reaches the channels the root list enables once every worker runs.
pub fn run(
    config: &Config,
    prober: Arc<dyn Probe>,
    store: Arc<Mutex<Store>>,
    shutdown: &Shutdown,
    reload: Option<Reload>,
    start: Start,
) -> Result<(), RunError> {
    let handle = notify::start(&config.notify, shutdown)?;
    let notifier: Arc<dyn Notifier> = Arc::clone(&handle.notifier);

    let result = run_with_notifier(config, prober, store, shutdown, reload, notifier, start);
    handle.shutdown(Some(Message::shutdown(now_ms())));
    result
}

/// Probe every target until shutdown, handing each recorded transition to `notifier`.
pub fn run_with_notifier(
    config: &Config,
    prober: Arc<dyn Probe>,
    store: Arc<Mutex<Store>>,
    shutdown: &Shutdown,
    reload: Option<Reload>,
    notifier: Arc<dyn Notifier>,
    start: Start,
) -> Result<(), RunError> {
    let Start {
        startup,
        promotion,
        fallback,
        snapshot: start_snapshot,
    } = start;
    config.require_targets().map_err(|_| RunError::NoTargets)?;

    logging::info(
        None,
        &format!(
            "monitoring {} target(s) every {} ms, timeout {} ms, database {}",
            config.targets.len(),
            config.interval.as_millis(),
            config.timeout.as_millis(),
            config.database.display()
        ),
    );

    let retention_days = Arc::new(AtomicU64::new(config.retention_days));
    let retention = {
        let store = Arc::clone(&store);
        let days = Arc::clone(&retention_days);
        let stop = shutdown.clone();
        thread::spawn(move || retention_worker(store, days, stop))
    };

    let channel = Arc::clone(&notifier);
    let first_round = Arc::new(FirstRound::new(config.targets.len()));
    let mut supervisor = Supervisor {
        config: config.clone(),
        overrides: reload
            .as_ref()
            .map(|reload| reload.overrides.clone())
            .unwrap_or_default(),
        prober,
        store,
        notifier,
        retention_days,
        first_round: Arc::clone(&first_round),
        workers: Vec::new(),
    };
    supervisor.start_workers()?;

    if let Some(startup) = startup {
        channel.send(startup);
    }

    let mut watcher = reload.as_ref().map(|reload| Watcher::new(&reload.path));
    let watched_path = reload.map(|reload| reload.path);

    let mut failure = None;
    let mut gate_done = false;
    loop {
        if shutdown::stopped(shutdown) {
            break;
        }
        if let Some(error) = supervisor.reap() {
            failure = Some(error);
            break;
        }
        if let (Some(watcher), Some(path)) = (watcher.as_mut(), watched_path.as_ref()) {
            if let Some(change) = watcher.poll() {
                if let Err(error) = supervisor.apply(change, path) {
                    failure = Some(error);
                    break;
                }
            }
        }
        if !gate_done && first_round.complete() {
            gate_done = true;
            let local_failure = first_round.local_failure();
            if local_failure.is_none() {
                if let Some(snapshot) = &start_snapshot {
                    match crate::snapshot::write(&snapshot.path, &snapshot.bytes) {
                        Ok(()) => logging::info(
                            None,
                            &format!(
                                "active configuration snapshot written to {}",
                                snapshot.path.display()
                            ),
                        ),
                        Err(error) => logging::error(
                            None,
                            &format!(
                                "cannot write the active configuration snapshot {}: {error}",
                                snapshot.path.display()
                            ),
                        ),
                    }
                }
            }
            match (&promotion, local_failure) {
                (Some(candidate), None) => {
                    promote_candidate(candidate, &channel, watcher.as_mut());
                }
                (Some(candidate), Some(failure)) => {
                    reject_candidate(
                        candidate,
                        &failure,
                        fallback.as_ref(),
                        &mut supervisor,
                        &channel,
                    )?;
                }
                (None, _) => {}
            }
        }
        shutdown::pause(reload::POLL_INTERVAL, shutdown);
    }

    supervisor.stop_workers();
    let _ = retention.join();
    logging::info(None, "monitor stopped");

    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Publish a verified candidate once the first probe round is clean.
fn promote_candidate(
    candidate: &Candidate,
    notifier: &Arc<dyn Notifier>,
    watcher: Option<&mut Watcher>,
) {
    match candidate.commit() {
        Ok(Some(backup)) => {
            logging::info(
                None,
                &format!(
                    "configuration promoted: {} -> {} (previous kept as {})",
                    candidate.source.display(),
                    candidate.main.display(),
                    backup.display()
                ),
            );
            let detail = format!(
                "{} (promoted from {})",
                candidate.main.display(),
                candidate.source.display()
            );
            notifier.send(Message::config_reload(true, detail, now_ms()));
            if let Some(watcher) = watcher {
                // The write is ours: it must not become a reload of the workers.
                watcher.rebase(&candidate.bytes);
            }
        }
        Ok(None) => {}
        Err(error) => {
            logging::error(
                None,
                &format!(
                    "cannot promote the candidate configuration: {error}; the running configuration stays in effect"
                ),
            );
            notifier.send(Message::config_reload(false, error.to_string(), now_ms()));
        }
    }
}

/// Swap back to the main configuration when a candidate cannot probe.
fn reject_candidate(
    candidate: &Candidate,
    failure: &str,
    fallback: Option<&Config>,
    supervisor: &mut Supervisor,
    notifier: &Arc<dyn Notifier>,
) -> Result<(), RunError> {
    logging::error(
        None,
        &format!(
            "candidate configuration {} cannot probe: {failure}",
            candidate.source.display()
        ),
    );
    logging::warn(
        None,
        &format!("{} is back in effect", candidate.main.display()),
    );
    notifier.send(Message::config_reload(
        false,
        format!("{} rejected: {failure}", candidate.source.display()),
        now_ms(),
    ));
    if let Some(fallback) = fallback {
        let detail = format!(
            "configuration reverted to {} after the candidate could not probe",
            candidate.main.display()
        );
        supervisor.swap(fallback.clone(), &detail)?;
    }
    Ok(())
}

fn monitor_target(
    target: Target,
    prober: Arc<dyn Probe>,
    store: Arc<Mutex<Store>>,
    notifier: Arc<dyn Notifier>,
    mut previous: TargetState,
    shutdown: Shutdown,
    first_round: Arc<FirstRound>,
) -> Result<(), RunError> {
    let name = target.display();
    let key = target.key();
    let mut reported = false;

    while !shutdown::stopped(&shutdown) {
        let outcome = prober.probe(
            &target.endpoint,
            target.interface.as_deref(),
            target.timeout,
        );

        if !reported {
            reported = true;
            first_round.record(&key, &outcome);
        }

        match &outcome {
            Ok(verdict) => logging::debug(Some(&name), &format!("verdict {verdict:?}")),
            Err(error) => logging::warn(Some(&name), &format!("probe failed: {error}")),
        }

        if let Some(transition) = state::transition(previous, &outcome) {
            let cause = transition.cause.as_ref();
            let event = Event {
                target: target.key(),
                interface: target.interface.clone(),
                label: target.label.clone(),
                priority: target.priority,
                previous_state: transition.from,
                state: transition.to,
                occurred_at_ms: now_ms(),
                rtt_us: transition.rtt_us,
                cause: cause.map(|failure| failure.code().to_string()),
                cause_message: cause.map(|failure| failure.message().to_string()),
            };

            {
                let mut store = store.lock().map_err(|_| RunError::Lock)?;
                store.record_transition(&event)?;
            }

            // The event is committed: only now may the operator hear about it (INV-12).
            let message = match event.state {
                TargetState::Down => Some(Message::target_down(
                    TargetRef::new(event.target.clone(), target.label.clone()),
                    Cause::new(
                        cause
                            .map(|failure| failure.code().to_string())
                            .unwrap_or_default(),
                        cause
                            .map(|failure| failure.message().to_string())
                            .unwrap_or_default(),
                    ),
                    event.occurred_at_ms,
                )),
                TargetState::Up => Some(Message::target_up(
                    TargetRef::new(event.target.clone(), target.label.clone()),
                    event.occurred_at_ms,
                )),
                TargetState::Unknown => None,
            };
            if let Some(message) = message {
                notifier.send(message);
            }
            logging::debug(Some(&name), "notification queued");

            previous = transition.to;
            logging::info(Some(&name), &describe(&transition));
        }

        shutdown::pause(target.interval, &shutdown);
    }

    Ok(())
}

fn describe(transition: &Transition) -> String {
    match transition.rtt_us {
        Some(rtt_us) => format!(
            "{} -> {} (rtt {:.3} ms)",
            transition.from.as_str(),
            transition.to.as_str(),
            rtt_us as f64 / 1_000.0
        ),
        None => format!("{} -> {}", transition.from.as_str(), transition.to.as_str()),
    }
}

fn retention_worker(
    store: Arc<Mutex<Store>>,
    days: Arc<AtomicU64>,
    shutdown: Shutdown,
) -> Result<(), RunError> {
    while !shutdown::stopped(&shutdown) {
        // The window can change on reload; the pass itself stays hourly so a sleeping
        // disk is not woken every minute.
        let window_days = days.load(Ordering::Relaxed);
        if window_days > 0 {
            let cutoff = now_ms() - window_days as i64 * 86_400_000;
            let result = {
                let store = store.lock().map_err(|_| RunError::Lock)?;
                store.prune(cutoff)
            };
            match result {
                Ok(0) => {}
                Ok(removed) => {
                    logging::info(None, &format!("retention removed {removed} event(s)"))
                }
                Err(error) => logging::error(None, &format!("retention failed: {error}")),
            }
        }
        shutdown::pause(Duration::from_secs(3_600), &shutdown);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, FileConfig, FileTarget, Overrides};

    fn config() -> Config {
        Config::resolve(
            Some(FileConfig {
                targets: Some(vec![
                    FileTarget {
                        ip: Some("1.1.1.1".to_string()),
                        ..FileTarget::default()
                    },
                    FileTarget {
                        method: Some("tcp".to_string()),
                        host: Some("1.1.1.1".to_string()),
                        port: Some(443),
                        ..FileTarget::default()
                    },
                ]),
                ..FileConfig::default()
            }),
            Overrides::default(),
        )
        .expect("a valid configuration")
    }

    #[test]
    fn the_startup_message_names_the_file_the_targets_and_the_page() {
        let text = startup_event(
            &config(),
            Some(Path::new("/etc/network-monitor.toml")),
            Some("0.0.0.0:8642"),
        )
        .text();

        assert!(
            text.starts_with(
                "READY network-monitor\nConfig: /etc/network-monitor.toml\nTargets: 2\n"
            ),
            "unexpected: {text}"
        );
        assert!(text.contains("\n- icmp://1.1.1.1\n"), "unexpected: {text}");
        assert!(
            text.contains("\n- tcp://1.1.1.1:443\n"),
            "unexpected: {text}"
        );
        assert!(
            text.contains("Web: http://0.0.0.0:8642/\n"),
            "unexpected: {text}"
        );
    }

    #[test]
    fn a_start_without_a_file_or_a_page_says_so() {
        let text = startup_event(&config(), None, None).text();

        assert!(text.contains("Config: none\n"), "unexpected: {text}");
        assert!(text.contains("Web: none\n"), "unexpected: {text}");
    }

    fn tcp_config(database: &Path, port: u16) -> Config {
        Config::resolve(
            Some(FileConfig {
                interval_ms: Some(50),
                timeout_ms: Some(500),
                database: Some(database.display().to_string()),
                targets: Some(vec![FileTarget {
                    method: Some("tcp".to_string()),
                    host: Some("127.0.0.1".to_string()),
                    port: Some(port),
                    ..FileTarget::default()
                }]),
                ..FileConfig::default()
            }),
            Overrides::default(),
        )
        .expect("a valid configuration")
    }

    #[test]
    fn a_candidate_that_cannot_probe_is_swapped_back_to_the_main_configuration() {
        use std::sync::atomic::{AtomicBool, Ordering};

        use crate::notify::fake::FakeNotifier;
        use crate::notify::Event;
        use crate::probe::fake::FakeProbe;
        use crate::probe::{Code, Details, ProbeError};

        let directory = tempfile::tempdir().unwrap();
        let main_path = directory.path().join("network-monitor.toml");
        std::fs::write(&main_path, b"interval_ms = 50\n").unwrap();
        let original = std::fs::read(&main_path).unwrap();
        let database = directory.path().join("events.db");
        let main = tcp_config(&database, 9);
        let candidate_settings = tcp_config(&database, 10);
        let candidate = Candidate {
            source: directory.path().join("next.toml"),
            main: main_path.clone(),
            bytes: b"interval_ms = 25\n".to_vec(),
            stamp: 1_794_096_000_000,
        };

        let failure = ProbeError::of(
            Code::TcpSocket,
            &Details {
                reason: Some("operation not permitted"),
                ..Details::default()
            },
        );
        let probe = Arc::new(FakeProbe::scripted(vec![Err(failure)]));
        let prober: Arc<dyn Probe> = probe.clone();
        let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));
        let notifier = Arc::new(FakeNotifier::new());
        let sink: Arc<dyn Notifier> = notifier.clone();
        let stop = Shutdown::new();
        let running = stop.clone();
        let start = Start {
            promotion: Some(candidate),
            fallback: Some(main),
            ..Start::default()
        };

        let handle = thread::spawn(move || {
            run_with_notifier(
                &candidate_settings,
                prober,
                store,
                &running,
                None,
                sink,
                start,
            )
        });

        let rejected = Arc::new(AtomicBool::new(false));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !notifier
            .messages()
            .iter()
            .any(|message| matches!(&message.event, Event::ConfigReload { applied: false, .. }))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "no rejection notification"
            );
            thread::sleep(std::time::Duration::from_millis(25));
        }
        rejected.store(true, Ordering::SeqCst);

        // After the swap the main target is probed again: the supervisor adopted it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !probe
            .calls()
            .iter()
            .any(|call| call.endpoint.to_string() == "tcp://127.0.0.1:9")
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the main target was not probed"
            );
            thread::sleep(std::time::Duration::from_millis(25));
        }

        stop.request();
        handle
            .join()
            .expect("the monitor thread does not panic")
            .unwrap();

        assert_eq!(
            std::fs::read(&main_path).unwrap(),
            original,
            "the main file is untouched"
        );
        let backups: Vec<String> = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".bak"))
            .collect();
        assert!(backups.is_empty(), "no promotion, no backup: {backups:?}");
    }
}
