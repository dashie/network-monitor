# network-monitor

Event-based reachability monitor with ICMP, TCP, DNS, and HTTP checks, per-interface probes, TOML configuration, and an embedded SQLite history.

Website: <https://dashie.github.io/network-monitor/> · Licence: MIT — see [LICENSE](./LICENSE).

## Overview

`network-monitor` answers one narrow question continuously and cheaply: is this target reachable right now, from this interface?

A target is a method plus what it checks: an ICMP echo request to an address, a TCP connection to a port, a DNS record that must resolve, or an HTTP status that must fall inside an expectation. ICMP and TCP probes can be bound to a named interface, exactly like `ping -I eth1 1.1.1.1`; when no interface is configured the operating system routing table decides, so the probe leaves through the default interface.

Unlike a ping loop or an uptime checker that samples every probe, it writes to the database only when a target changes state: one record when a target stops answering and one when it answers again. The history therefore grows with the number of incidents, not with the number of probes.

Every option of the configuration file can also be given on the command line, where it overrides the file. The database is SQLite in WAL mode, so a `query` invocation reads the event history while the monitor keeps running.

The deliverable is a single executable with no runtime dependencies, built from Rust for macOS, Linux, and Windows, and shipped as a container image for NAS and server deployments such as QNAP Container Station.

## Quick start

```bash
# Build the single, dependency-free binary (Rust 1.75 or newer)
cargo build --release

# Or scaffold a runnable setup in ./monitor: config, database, and event log, no privileges
./target/release/network-monitor init ./monitor
(cd ./monitor && ../target/release/network-monitor check --config network-monitor.toml)
(cd ./monitor && ../target/release/network-monitor monitor --config network-monitor.toml \
  --listen 127.0.0.1:8642)

# Read the full manual offline (Markdown); pipe it to a pager or renderer
./target/release/network-monitor manual | less -R

# Verify the configuration first: one ok/warn/fail line per check, no probe loop, no write
./target/release/network-monitor check --config ./network-monitor.toml

# Run the monitor; raw ICMP sockets need elevated privileges on every platform
sudo ./target/release/network-monitor monitor --config ./network-monitor.toml

# Same command, with the page on port 8642: one process records and shows
sudo ./target/release/network-monitor monitor --config ./network-monitor.toml \
  --listen 0.0.0.0:8642

# Read the recorded transitions from a second shell while the monitor runs
./target/release/network-monitor query events --config ./network-monitor.toml

# Serve the page alone, without probing or NET_RAW
./target/release/network-monitor serve --database ./network-monitor.db --listen 127.0.0.1:8642

# Look at that page with no network and no database: a generated history, the same for the same seed
./target/release/network-monitor simulate --listen 127.0.0.1:8660 --history 30d --seed 0

# Read the same history as JSON from that server: read-only, no credential, no second port
curl -s http://127.0.0.1:8642/api/v1/faults?limit=10

# The same routes turned into a reference a browser can read and call, offline:
# http://127.0.0.1:8642/api/v1/docs

# Restore the history from the file that page downloaded; back up the database first
# (docs/DEVELOPER-GUIDE.md#back-up-before-you-import)
./target/release/network-monitor import ./faults-20261109.csv.gz \
  --database ./network-monitor.db

# Run the test suite (no privileges needed: the ICMP prober is replaced by a fake)
cargo test
```

To hear about an outage without opening the page, declare a notification channel and enable
it by name: a Telegram chat, a webhook, or a log file, one message per incident and a
recovery only once `up` has held for the stabilisation window. The declarations and the
routing lists are in the sample configuration; setup and failure modes are in
[docs/NOTIFICATIONS.md](docs/NOTIFICATIONS.md).

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

# One example per method; see network-monitor.toml for every key and its default.
[[targets]]
method = "tcp"
host = "1.1.1.1"            # address or name
port = 443
label = "cloudflare-https"

[[targets]]
method = "dns"
name = "example.com"        # resolved by the system resolver
resolver = "9.9.9.9"        # optional: ask this server instead
label = "example-via-quad9"

[[targets]]
method = "http"
url = "https://example.com/health"   # up when the status is 200-399 by default
label = "example-health"
```

## Status

**Alpha.** The feature set in [docs/PRODUCT.md](docs/PRODUCT.md) is implemented and covered by the test suite: layered configuration with live reload, four probe methods — ICMP, TCP, DNS, and HTTP/HTTPS — with per-interface binding for the first two, event-only SQLite persistence, querying while monitoring runs, retention, a read-only web page with 24 h state bands, per-target statistics, the latest faults with the cause of each, and a 30-day per-target view, and a read-only JSON API under `/api/v1` that serves the targets, the faults, and the per-day totals to a dashboard, rendered as a reference at `/api/v1/docs` that needs no internet access. The container image builds and probes with only `NET_RAW` added; HTTPS verification uses roots compiled into the binary.

Not done yet: the Windows raw-socket path is written but not exercised on a Windows host, IPv6 targets are deferred, HTTP checks do not follow redirects, and no binary is published as a release.

## Documentation

| Document | Purpose |
|---|---|
| [Landing page](https://dashie.github.io/network-monitor/) | The public description of the project: what it does, the four checks, the screenshots, the quick start, the limits, the questions |
| [docs/USER-GUIDE.md](docs/USER-GUIDE.md) | Install, configure, operate, troubleshoot — the manual to hand to a user |
| [docs/PRODUCT.md](docs/PRODUCT.md) | Users, scope, functional requirements, business rules |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | arc42 structure, C4 views, component contracts, invariants |
| [docs/ADR.md](docs/ADR.md) | Architecture decisions and their rationale |
| [docs/GLOSSARY.md](docs/GLOSSARY.md) | One precise definition per domain term |
| [docs/DEVELOPER-GUIDE.md](docs/DEVELOPER-GUIDE.md) | Build, run, test, extend, and the Git standards |
| [docs/HISTORY-IMPORT.md](docs/HISTORY-IMPORT.md) | Runbook: import a history from another monitor into a database |
| [docs/NOTIFICATIONS.md](docs/NOTIFICATIONS.md) | Runbook: declare, enable, verify, and repair a notification channel |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Branching, review, definition of done |
| [AGENTS.md](AGENTS.md) | Rules for human and AI contributors |

## Licence

MIT. See [LICENSE](./LICENSE): use it, modify it, ship it, keep the notice.

## Contributing

Read [CONTRIBUTING.md](./CONTRIBUTING.md) before opening a pull request, and
[AGENTS.md](./AGENTS.md) for the rules that apply to human and AI contributors alike.
Every text artefact in this repository is written in English.
