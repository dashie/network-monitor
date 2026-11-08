//! The monitoring loop: one interval loop per target, one shared store.

pub mod state;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::config::{Config, Target};
use crate::logging;
use crate::probe::Probe;
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
    #[error("the event store is unavailable")]
    Lock,
    #[error("a monitor thread stopped unexpectedly")]
    Panic,
}

/// Probe every target until shutdown, persisting transitions.
pub fn run(
    config: &Config,
    prober: Arc<dyn Probe>,
    store: Arc<Mutex<Store>>,
    shutdown: &Shutdown,
) -> Result<(), RunError> {
    config.require_targets().map_err(|_| RunError::NoTargets)?;

    let initial_states = {
        let store = store.lock().map_err(|_| RunError::Lock)?;
        store.load_states()?
    };

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

    let mut workers = Vec::with_capacity(config.targets.len());
    for target in &config.targets {
        let target = target.clone();
        let prober = Arc::clone(&prober);
        let store = Arc::clone(&store);
        let shutdown = shutdown.clone();
        let interval = config.interval;
        let previous = initial_states
            .get(&target.ip.to_string())
            .copied()
            .unwrap_or(TargetState::Unknown);

        workers.push(thread::spawn(move || {
            monitor_target(target, prober, store, interval, previous, shutdown)
        }));
    }

    let retention = if config.retention_days > 0 {
        let store = Arc::clone(&store);
        let days = config.retention_days;
        let shutdown = shutdown.clone();
        Some(thread::spawn(move || {
            retention_worker(store, days, shutdown)
        }))
    } else {
        None
    };

    let mut first_error = None;
    for worker in workers {
        match worker.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
            Err(_) => {
                if first_error.is_none() {
                    first_error = Some(RunError::Panic);
                }
            }
        }
    }

    if let Some(retention) = retention {
        let _ = retention.join();
    }

    logging::info(None, "monitor stopped");

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn monitor_target(
    target: Target,
    prober: Arc<dyn Probe>,
    store: Arc<Mutex<Store>>,
    interval: Duration,
    mut previous: TargetState,
    shutdown: Shutdown,
) -> Result<(), RunError> {
    let name = target.display();

    while !shutdown::stopped(&shutdown) {
        let outcome = prober.probe(target.ip, target.interface.as_deref());

        match &outcome {
            Ok(verdict) => logging::debug(Some(&name), &format!("verdict {verdict:?}")),
            Err(error) => logging::warn(Some(&name), &format!("probe failed: {error}")),
        }

        if let Some(transition) = state::transition(previous, &outcome) {
            let event = Event {
                target: target.ip.to_string(),
                interface: target.interface.clone(),
                label: target.label.clone(),
                previous_state: transition.from,
                state: transition.to,
                occurred_at_ms: now_ms(),
                rtt_us: transition.rtt_us,
            };

            {
                let mut store = store.lock().map_err(|_| RunError::Lock)?;
                store.record_transition(&event)?;
            }

            previous = transition.to;
            logging::info(Some(&name), &describe(&transition));
        }

        pause(interval, &shutdown);
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
    days: u64,
    shutdown: Shutdown,
) -> Result<(), RunError> {
    let window_ms = days as i64 * 86_400_000;

    while !shutdown::stopped(&shutdown) {
        let cutoff = now_ms() - window_ms;
        let result = {
            let store = store.lock().map_err(|_| RunError::Lock)?;
            store.prune(cutoff)
        };
        match result {
            Ok(0) => {}
            Ok(removed) => logging::info(None, &format!("retention removed {removed} event(s)")),
            Err(error) => logging::error(None, &format!("retention failed: {error}")),
        }
        pause(Duration::from_secs(3_600), &shutdown);
    }

    Ok(())
}

/// Sleep, but wake up promptly when a shutdown is requested.
fn pause(duration: Duration, shutdown: &Shutdown) {
    const STEP: Duration = Duration::from_millis(100);
    let mut remaining = duration;
    while !remaining.is_zero() {
        if shutdown::stopped(shutdown) {
            return;
        }
        let slice = remaining.min(STEP);
        thread::sleep(slice);
        remaining -= slice;
    }
}
