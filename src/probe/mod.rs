//! Probe contract and the ICMP implementation.
//!
//! [`Probe`] is the only boundary the monitor knows: the transport can be replaced by
//! [`fake::FakeProbe`], so the decision logic is tested without raw sockets or
//! privileges (INV-3).

pub mod fake;
pub mod icmp;
pub mod interface;

use std::net::Ipv4Addr;
use std::time::Duration;

use thiserror::Error;

/// The outcome of one probe cycle.
///
/// A local failure is *not* a verdict: it is a [`ProbeError`], so the monitor can
/// never record "the target is down" when the host merely could not probe (INV-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A matching echo reply arrived within the timeout.
    Up { rtt: Duration },
    /// No matching echo reply arrived within the timeout.
    Down,
}

/// A failure of this host to perform the probe.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProbeError {
    #[error("{reason}")]
    Local { reason: String },
}

impl ProbeError {
    pub fn local(reason: impl Into<String>) -> Self {
        ProbeError::Local {
            reason: reason.into(),
        }
    }
}

/// One ICMP echo request, executed through an optional interface.
pub trait Probe: Send + Sync {
    fn probe(&self, target: Ipv4Addr, interface: Option<&str>) -> Result<Verdict, ProbeError>;
}

pub use icmp::IcmpProbe;
