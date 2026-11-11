# Product — network-monitor

## Purpose and vision

Give an operator a tool that answers "which link or which peer is broken right now" with a record of exactly when it broke and when it recovered, using a few kilobytes of storage and no infrastructure.

`network-monitor` replaces the ping loop, the scattered shell history, and the heavyweight uptime platform with one binary that fits on a NAS, in a container, and in a cron-free long-running process.

## Users

- **Homelab and NAS owner.** Runs QNAP, a small server, or a Raspberry Pi with several uplinks and wants to know when a WAN link, a VPN tunnel, or a peer stops answering — without deploying a monitoring stack.
- **System or network administrator.** Needs to verify that reachability is identical from a specific interface (`eth1`, `wg0`, `pppoe0`) and not merely from the default route.
- **On-call engineer.** Asks "since when is $host down, and did it flap?" and needs an exact timestamped history that survives restarts.
- **Edge or embedded operator.** Deploys on a device where only a single static binary can be shipped and where database servers are impossible.
- **Automation author.** Consumes the SQLite database or the JSON output of the query command from scripts and dashboards.

## Problem and value proposition

Checking whether a host is reachable is trivial — `ping 1.1.1.1` — but keeping a reliable record of *when* reachability changed is not.

Today this is handled with a ping loop that logs every reply (megabytes of log for one day), with an uptime platform that must be hosted and maintained, or with ad-hoc manual checks that leave no history at all. A ping loop also loses the essential piece of information in a multi-homed host: *which interface* the probe left from. `ping` can bind the source address or device (`ping -I eth1 1.1.1.1`), but the standard monitoring tools cannot, and the result depends on the routing table of the moment rather than on the configured intent.

The consequence is a history that is either too large, absent, or not attributable to an interface.

`network-monitor` records transitions instead of samples: one row when a target goes down, one when it comes back, timestamped in UTC. Storage is proportional to incidents, queries are instant, and the database can be read while monitoring continues.

Probes are bound to the configured interface with the semantics of `ping -I`, the whole configuration is overridable from the command line, and the entire tool is a single executable with no runtime dependency — on a laptop, in Docker, or on a QNAP.

## Scope

### In scope

- A TOML configuration file with global defaults (`interval_ms`, `timeout_ms`, `database`, `retention_days`) and a list of targets (`ip`, optional `interface`, optional `label`).
- A command-line parameter for every configuration option, with the precedence `command line > configuration file > built-in default`.
- ICMP echo probing of IPv4 targets, one echo request per target per interval, with an explicit timeout.
- Binding a probe to a named network interface on macOS, Linux, and Windows, with the semantics of `ping -I <interface> <address>`; the default-route interface is used when no interface is configured.
- Persistence of state transitions only, in an embedded SQLite database created and migrated by the binary itself.
- Concurrent reads: the `query` subcommand reads the event history and the current state while monitoring runs.
- Configurable retention of events older than `retention_days`.
- A single statically linked executable per platform (macOS, Linux, Windows) and a container image for Linux deployments, including QNAP Container Station.
- Line-based logging on stderr and meaningful exit codes for scripting.

### Out of scope

- Any programmatic API: the web page returns HTML for a browser; scripts use the database or the `query` command.
- Alerting and notification (mail, webhook, Telegram, MQTT): the database is the integration point.
- Metrics export (Prometheus, OpenTelemetry) and dashboards.
- Probe types other than ICMP echo: no TCP, UDP, HTTP, DNS, or traceroute probes.
- IPv6 targets: the first milestone probes IPv4 only; ICMPv6 is tracked as a follow-up.
- High availability, clustering, and replication of the database.
- Windows service and macOS launchd integration as a packaged installer: the binary is shipped, the supervisor is the operator's choice.

## Functional behaviour

Each requirement is observable and independently verifiable. Implementation details
belong in [ARCHITECTURE.md](./ARCHITECTURE.md).

- `FR-1`: The tool reads a TOML configuration file. `--config <path>` must exist and parse; without the option, `./network-monitor.toml` is read when present and skipped otherwise, so a fully command-line invocation needs no file. A malformed file, or an explicit path that does not exist, is a startup error reported on stderr with a non-zero exit code.
- `FR-2`: Every configuration option can be given on the command line and overrides the file; targets are supplied with the repeatable `--target <IP>[,interface=<name>][,label=<name>]` option, which replaces the target list of the file when present.
- `FR-3`: The tool sends one ICMP echo request per target per configured interval and classifies the result as reachable (a matching echo reply arrived within `timeout_ms`) or unreachable (no matching reply within `timeout_ms`).
- `FR-4`: When a target has an interface configured, the probe leaves through that interface with the semantics of `ping -I <interface> <address>`; when it has none, the default-route interface of the operating system is used. The interface is given by name on Linux and macOS (`eth1`, `en0`) and by numeric interface index on Windows.
- `FR-5`: A state transition is recorded only when the verdict of a probe cycle differs from the last recorded verdict of that target. Repeated verdicts produce no database write.
- `FR-6`: The first probe cycle of a target always records a transition, from `unknown` to the observed `up` or `down`.
- `FR-7`: The tool stores events in an embedded SQLite database, creating the file, the schema, and the migration metadata when they are absent.
- `FR-8`: The event history and the current state of every target can be read while the monitor is running, without stopping or pausing it.
- `FR-9`: The `query` subcommand reports the current state per target and the event history filtered by target and time range, in a human-readable table and in JSON selected by `--format json`.
- `FR-10`: When `retention_days` is greater than zero, events older than that window are deleted periodically; `0` keeps every event.
- `FR-11`: `SIGINT` and `SIGTERM` stop the monitor cleanly within one probe cycle, with no partially written event.
- `FR-12`: `--show-config` prints the effective configuration, after defaults, file, and command line have been merged, and exits without probing.
- `FR-13`: Invalid configuration — an unknown option, a non-positive interval or timeout, an unparseable or IPv6 address, a target listed twice, a configured interface that does not exist, or a `monitor` run without targets — aborts startup with a message naming the offending key and a non-zero exit code.
- `FR-14`: The tool runs as a single executable with no runtime dependency, and as a container image that needs only the `NET_RAW` capability.
- `FR-15`: Every probe cycle and every transition is observable on stderr according to `--log-level` (`error`, `warn`, `info`, `debug`), in a stable line-based format.

- `FR-16`: While monitoring runs, the tool re-reads the configuration file within one second of a change and applies the new interval, timeout, target list, and retention without a restart, writing one `INFO` line that states the configuration was updated. A file that cannot be read, does not parse, or leaves no targets is rejected with a warning and the previous configuration stays in effect. A changed `database` path is reported and used only from the next start.

- `FR-17`: The read-only web page over the event database is available in two ways from the same binary: `serve --listen …` publishes it without probing, and `monitor --listen …` publishes it from the monitoring process, so one command both records and shows the history. The page shows one lane per target with green, red, and grey bands for `up`, `down`, and `unknown`; the window is the last hour by default and the links select the last 1, 2, 4, 6, 12, or 24 hours, narrowest first, so `/?window=24h` is a bookmarkable state. It has no client-side interaction beyond following a link, and the browser reloads it every 30 s. Every instant is written in UTC and formatted by the browser with the locale and the time zone of the reader; without the script the page shows the same instants in UTC, labelled as such. Each lane states, from left to right, the disconnections, the total down time of the window it shows, the average `up` run with its spread, the longest `up` run of the window, how long the target has been `up` without a drop, and the share of the probed time spent `up`, so the same row answers "how bad was it", "how long does it stay up", and "is it up right now, and since when". Below the chart the last 10 disconnections of any target are listed with their start and their duration, an ongoing one included. The per-day buckets of the detail page are UTC days by `BR-6`, so their dates are localised in format but keep the UTC day they belong to. Selecting a lane opens a page for that target: a breadcrumb at the top leads back to the timeline, the target's current state and the totals of the period sit under the title, and a selector offers the last 30, 60, 90, 180, or 360 days — shortest first, 30 days when the query names none — with the choice carried in the URL (`/target?target=…&days=90d`) so it survives the 30 s reload and can be bookmarked. One bar per day follows for the chosen period, with the count above every day while the columns have room for it, a quiet day stating a muted zero at the baseline so a measured nothing is not read as missing data; past about seven weeks the counts are omitted and the table carries the exact numbers. A table with one row per day states the number of disconnections, the total down time, and the average `up` run of that day, with the longest run of the same window under the mean. The first row is the current UTC day, which is still running, so its average `up` run covers the last 24 hours instead, tagged `last 24 h` and styled apart from the figures of a complete day; both pages label that figure `AVG UP`. The badge next to the title states the current state and, while it is `up`, how long it has been up without a drop. The header of that page states the disconnections, the down time, and the average `up` run of the whole period with the longest run of it. Every request opens the database read-only and answers `503` with the reason when it is missing. The page has no authentication. The server stops with the process that runs it, on `SIGINT` or `SIGTERM`.

- `FR-18`: The page offers a download of the whole history, as a gzip-compressed CSV file (`/export/disconnections.csv.gz`). The CSV has one row per event with the target, its label, its interface, the state reached (`up` or `down`), the instant, and the round-trip time when the event is a return to `up`. The columns are the ones the import reads, so an exported file can be imported back as it is. The file is compressed while it is sent: the server reads the history in pages and never builds the file in memory.

- `FR-19`: The `import` subcommand restores a history from the file the page offers for download, so a redeployed service does not lose its record. The source is a file path, an `http://` URL, or `-` for standard input. The format is recognised from the content — a gzipped or a plain CSV — with no flag, and it is the format `FR-18` writes: one row per event, replayed as it is, labels and interfaces included. A file in the disconnection-only format of an earlier version is refused with a message asking for a fresh export. Importing the same file twice changes nothing: an event already present is skipped and counted. Rows that are not part of the format are skipped and counted, and the command reports how many events it wrote, for how many targets, how many rows it ignored, and how many events it dropped as repeats. The rows may arrive in any order: the state each event replaced, and the removal of an event that repeats the state before it, are worked out from the timestamps, not from the order of the lines. The command only reads the archive and only writes the database; it probes nothing and serves nothing. `https` is refused with an explanation, because the binary carries no TLS stack. With `--replace-window` the import replaces instead of merging: for every target the archive mentions, the events between that target's earliest and latest instant in the archive are deleted before the archive is written, and that target's events before and after its own window are kept, so a corrected archive can be re-applied to the same database; a target the archive does not mention keeps all of its events. An archive with no row has no window: the command fails and deletes nothing.

## Business rules

Rules that always hold, independent of implementation.

- `BR-1`: A target never has two consecutive events with the same state. Rationale: the database models transitions, so a duplicated state is corruption and would also mean a lost transition.
- `BR-2`: A target starts in the `unknown` state. Its first recorded event is the transition out of `unknown`; the event carries `unknown` as the previous state.
- `BR-3`: The effective configuration is always the merge `command line > configuration file > built-in default`, with no hidden environment variable and no implicit third source.
- `BR-4`: A local failure — missing privilege, missing interface, socket error, unwritable database — is never recorded as a target going down. It is reported as an error, and it never changes a target's state. Rationale: the tool must not confuse "I cannot see the network" with "the network is down".
- `BR-5`: A configured interface that does not exist at startup is a fatal configuration error, never a silent fallback to the default interface. Rationale: a silent fallback would measure a different path than the one configured.
- `BR-6`: Timestamps are stored as UTC epoch milliseconds in integer columns; time zone and locale affect presentation only. Rationale: queries and retention stay exact and portable.
- `BR-7`: The current state of a target and its last event are written in the same SQLite transaction, so a reader never observes a state that contradicts the latest event.
- `BR-8`: The database is the integration point for other tools. The tool opens at most one listening socket, the read-only web page, and only when `--listen` is given: `serve` never probes, `monitor` probes and serves at the same time. Without `--listen` nothing listens and the only network traffic is the ICMP probe. Rationale: a page must never be able to alter the history, and a monitor that was not asked to serve must not expose a surface.
- `BR-9`: A database written by a newer schema version than the running binary is reported as an error and left untouched, never migrated downwards.
- `BR-10`: Statistics of the `up` runs are computed from the segments the page draws, and a run that the window opens or closes in the middle of counts as it appears: the first and the last run of a window are lower bounds, never overestimates. Time in `unknown` is not measured, so it is left out of the availability figure instead of counting as down. Rationale: a figure a reader can check against the bands above it, and a target first seen an hour ago says nothing about the day before.

## Non-functional requirements

| Attribute | Target |
|---|---|
| Storage growth | Under 1 MB per 10 000 state transitions; a target that never changes state costs no rows after its first event |
| Probe overhead | One ICMP echo request and one reply per target per interval; no measurable traffic when a target is stable |
| CPU | Below 1 % of one core with 50 targets at a 5 s interval |
| Memory | Resident memory below 20 MB with 100 targets |
| Startup | Configuration parsed and first probe sent within 200 ms |
| Query latency | `query state` returns in under 50 ms on a database with 1 000 000 events |
| Concurrency | Readers never block the writer for more than the WAL checkpoint interval; the writer proceeds with a `busy_timeout` of 5 s |
| Durability | A crash loses at most the event being written; the schema stays consistent |
| Portability | One binary per platform, no runtime dependency, no installer, no shell script in the execution path |
| Footprint | Release binary under 15 MB, container image under 30 MB |
| Reload latency | A change written to the configuration file is detected and applied within 2 s, without losing the last known state |
| Web page | 50 targets over a 24 h window | Rendered in under 300 ms on the NAS; a request never writes to the database |

## Success metrics

- A 30-day run on a QNAP with 20 targets at a 5 s interval produces a database smaller than 1 MB.
- The event count equals the number of observed transitions in an independent audit: no duplicate, no missing transition.
- `query events` returns in under 50 ms while the monitor is running, verified on the deployment device.
- The container image starts on Container Station with only `NET_RAW` added and no privileged mode.
- A contributor reproduces the build and the test suite from a clean checkout by following the developer guide, without asking a question.
- The same binary artifact, built once per platform, runs on macOS, Linux, and Windows without recompilation on the target.

## Assumptions and open questions

### Assumptions

- The implementation language is Rust, chosen in the intake of this baseline; the deployment model is a single statically linked executable.
- SQLite is linked into the binary; no system `libsqlite3` is required on the host.
- The default interval is 5 000 ms and the default timeout is 2 000 ms; both are configurable.
- The first milestone probes IPv4 addresses only. IPv6 is explicitly deferred.
- Retention is disabled by default (`retention_days = 0`), so no event is deleted unless the operator asks for it.
- Probes are privileged: the process needs `root`, `CAP_NET_RAW`, or Administrator on Windows.
- On Windows the interface is given as a numeric interface index; resolving an adapter name is not implemented in this version.
- The QNAP deployment path is Docker through Container Station; a native QPKG is not planned.
- The configuration file is re-read once per second while monitoring runs; a change is applied without a restart, and a changed database path still needs one.
- Alerting is out of scope: the database is the integration point for other tools.
- The tool name and the executable name are both `network-monitor`.
- The web page binds `127.0.0.1` by default; exposing it on the LAN (`--listen 0.0.0.0:8642`) is an explicit operator choice and carries no authentication.

### Open questions

- **IPv6 support** — which interface scoping rules apply to link-local addresses, and is ICMPv6 a separate probe type or the same one? Owner: maintainer.
- **Flap damping** — should a configurable confirmation count (`down` after N consecutive failures) be added, or is one probe per cycle the contract? Owner: maintainer.
- **Alerting hooks** — is a post-transition command hook acceptable as an escape hatch, or does it violate the no-dependency rule? Owner: maintainer.
- **Multiple addresses per target** — should a label group several addresses, or is one label one address? Owner: maintainer.
- **Web authentication** — is a shared token or HTTP basic authentication worth the complexity for LAN exposure? Owner: maintainer.

## Related documents

- [ARCHITECTURE.md](./ARCHITECTURE.md) — how the system is built
- [GLOSSARY.md](./GLOSSARY.md) — the domain language used above
