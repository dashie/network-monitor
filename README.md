# network-monitor

Event-based ICMP reachability monitor with per-interface probes, TOML configuration, and an embedded SQLite history.

## Overview

`network-monitor` answers one narrow question continuously and cheaply: is this IP address reachable right now, from this interface?

It reads a TOML configuration listing the addresses to watch, sends one ICMP echo request to each address on a configurable interval — the same check `ping` performs — and can bind the probe to a named interface, exactly like `ping -I eth1 1.1.1.1`. When no interface is configured the operating system routing table decides, so the probe leaves through the default interface.

Unlike a ping loop or an uptime checker that samples every probe, it writes to the database only when a target changes state: one record when a target stops answering and one when it answers again. The history therefore grows with the number of incidents, not with the number of probes.

Every option of the configuration file can also be given on the command line, where it overrides the file. The database is SQLite in WAL mode, so a `query` invocation reads the event history while the monitor keeps running.

The deliverable is a single executable with no runtime dependencies, built from Rust for macOS, Linux, and Windows, and shipped as a container image for NAS and server deployments such as QNAP Container Station.

## Quick start

```bash
# Build the single, dependency-free binary (Rust 1.75 or newer)
cargo build --release

# Run the monitor; raw ICMP sockets need elevated privileges on every platform
sudo ./target/release/network-monitor monitor --config ./network-monitor.toml

# Read the recorded transitions from a second shell while the monitor runs
./target/release/network-monitor query events --config ./network-monitor.toml

# Run the test suite (no privileges needed: the ICMP prober is replaced by a fake)
cargo test
```

A minimal `network-monitor.toml`:

```toml
# Defaults for every target; each one can be overridden on the command line.
interval_ms = 5000
timeout_ms = 2000
database = "./network-monitor.db"
retention_days = 0          # 0 keeps every event forever

[[targets]]
ip = "1.1.1.1"
interface = "eth1"          # probe out of eth1, like ping -I eth1
label = "cloudflare-dns"

[[targets]]
ip = "192.168.1.1"          # no interface: the default-route interface is used
```

## Status

**Alpha.** The feature set in [docs/PRODUCT.md](docs/PRODUCT.md) is implemented and covered by the test suite: layered configuration, ICMP probing with per-interface binding, event-only SQLite persistence, querying while monitoring runs, and retention. The container image builds and probes with only `NET_RAW` added.

Not done yet: the Windows raw-socket path is written but not exercised on a Windows host, IPv6 targets are deferred, and no binary is published as a release.

## Documentation

| Document | Purpose |
|---|---|
| [docs/PRODUCT.md](docs/PRODUCT.md) | Users, scope, functional requirements, business rules |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | arc42 structure, C4 views, component contracts, invariants |
| [docs/ADR.md](docs/ADR.md) | Architecture decisions and their rationale |
| [docs/GLOSSARY.md](docs/GLOSSARY.md) | One precise definition per domain term |
| [docs/DEVELOPER-GUIDE.md](docs/DEVELOPER-GUIDE.md) | Build, run, test, extend, and the Git standards |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Branching, review, definition of done |
| [AGENTS.md](AGENTS.md) | Rules for human and AI contributors |

## Contributing

Read [CONTRIBUTING.md](./CONTRIBUTING.md) before opening a pull request, and
[AGENTS.md](./AGENTS.md) for the rules that apply to human and AI contributors alike.
Every text artefact in this repository is written in English.
