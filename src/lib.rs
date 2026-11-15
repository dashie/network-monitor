//! `network-monitor`: event-based ICMP reachability monitoring.
//!
//! This library exists so the integration tests exercise exactly the modules the
//! binary uses. `src/main.rs` is a thin entry point over [`cli`], [`config`],
//! [`monitor`], [`query`], and [`store`].

pub mod cli;
pub mod config;
pub mod history;
pub mod import;
pub mod json;
pub mod logging;
pub mod monitor;
pub mod notify;
pub mod probe;
pub mod query;
pub mod reload;
pub mod shutdown;
pub mod sim;
pub mod store;
pub mod time;
pub mod tls;
pub mod web;
