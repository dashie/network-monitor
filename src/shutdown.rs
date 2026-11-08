//! Stop signal shared between the signal handler and the monitoring loops.
//!
//! The monitor receives a [`Shutdown`] handle, so a test drives its own flag instead
//! of a process-wide one. The operating-system handler writes to a second, static
//! flag that every handle observes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A stop switch. Cloning it shares the same state with every clone.
#[derive(Clone, Debug, Default)]
pub struct Shutdown {
    requested: Arc<AtomicBool>,
}

impl Shutdown {
    pub fn new() -> Self {
        Shutdown::default()
    }

    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
    }

    pub fn requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

/// Set by the signal handler. Only [`install`] ever sets it.
static SIGNALLED: AtomicBool = AtomicBool::new(false);

pub fn signalled() -> bool {
    SIGNALLED.load(Ordering::SeqCst)
}

/// True when a signal or an explicit request asked the process to stop.
pub fn stopped(shutdown: &Shutdown) -> bool {
    shutdown.requested() || signalled()
}

/// Install the handlers that translate an interrupt into a clean stop (FR-11).
///
/// On Windows the default console handler terminates the process; SQLite still leaves
/// a consistent database because every write is one transaction.
pub fn install() {
    #[cfg(unix)]
    unsafe {
        let handler = handle as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

#[cfg(unix)]
extern "C" fn handle(_signal: libc::c_int) {
    SIGNALLED.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_shares_the_state() {
        let shutdown = Shutdown::new();
        let clone = shutdown.clone();
        assert!(!shutdown.requested());
        clone.request();
        assert!(shutdown.requested());
    }

    #[test]
    fn each_handle_starts_clear() {
        let first = Shutdown::new();
        let second = Shutdown::new();
        first.request();
        assert!(!second.requested());
    }
}
