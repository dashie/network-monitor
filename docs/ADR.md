# Architecture Decision Records — network-monitor

An architecture decision record (ADR) captures a decision together with the context
that made it necessary and the consequences it creates. Records are append-only: a
decision is superseded by a new record, never rewritten or deleted. Write a record
when a decision is costly to reverse, affects more than one component, or constrains
future work.

Status values: `proposed`, `accepted`, `deprecated`, `superseded by ADR-XXXX`.

## Index

| ID | Title | Status | Date | Decision |
|---|---|---|---|---|
| ADR-0001 | Adopt the documentation and repository baseline | accepted | 2026-11-08 | Adopt the standard documents and record the intake assumptions |
| ADR-0002 | Implement in Rust as a single static binary | accepted | 2026-11-08 | Rust stable, `rusqlite` bundled, per-platform static builds |
| ADR-0003 | Probe with raw ICMP sockets instead of the system `ping` | accepted | 2026-11-08 | In-process raw ICMP with native interface binding |
| ADR-0004 | Persist transitions only, in embedded SQLite with WAL | accepted | 2026-11-08 | Two tables, event-only writes, concurrent reads |
| ADR-0005 | Mirror every configuration option as a command-line flag | accepted | 2026-11-08 | Precedence `cli > file > default`, effective configuration printed on demand |
| ADR-0006 | Ship one binary with `monitor` and `query` subcommands | accepted | 2026-11-08 | Single artifact; query opens the database read-only |
| ADR-0007 | Reload the configuration file while monitoring runs | accepted | 2026-11-08 | Poll the file every second and apply a valid change without a restart |
| ADR-0008 | Serve a read-only web page from the binary | accepted | 2026-11-09 | Minimal HTTP and server-side SVG; no framework and no client library |
| ADR-0009 | Serve the page from the monitoring process | accepted | 2026-11-09 | `monitor --listen` runs the same `web` component; one process records and shows |
| ADR-0010 | Render the timeline without client-side interaction | accepted | 2026-11-09 | Fixed window, 30 s meta refresh, no zoom; last 10 disconnections listed |

## Records

## ADR-0001 — Adopt the documentation and repository baseline

**Status:** accepted · **Date:** 2026-11-08

### Context

The repository was empty. A human and an AI agent must be able to work on it without guessing the product intent, the architecture constraints, the domain language, or the delivery process.

### Decision

Adopt the standard baseline: `README.md`, `CONTRIBUTING.md`, `docs/PRODUCT.md`, `docs/ARCHITECTURE.md`, `docs/ADR.md`, `docs/GLOSSARY.md`, `docs/DEVELOPER-GUIDE.md`, `AGENTS.md`, `CLAUDE.md`, the commit-message and pull-request instruction files, and the editor settings that register them. `AGENTS.md` is the single source of agent instructions; `CLAUDE.md` imports it.

The intake produced these facts: a CLI tool, a TOML configuration, per-interface ICMP probes, event-only persistence in embedded SQLite, queryable while running, configurable interval, every option overridable on the command line, portable to macOS, Linux, and Windows, one executable with no dependencies, deployable through Docker on QNAP. The operator chose Rust, native raw ICMP sockets, one binary with subcommands, configurable retention, and Docker as the QNAP path.

### Consequences

- The documents are the contract; a change to behaviour updates the document it touches in the same change.
- Assumptions taken instead of asked are listed in [PRODUCT.md](./PRODUCT.md#assumptions-and-open-questions) and are revisable through a new record.
- Every artefact is written in English.

### Alternatives considered

- Code first, documentation later: rejected, because the first implementation would encode undocumented decisions.
- An external wiki: rejected, because the documentation must version with the code and be readable by an agent in the repository.

## ADR-0002 — Implement in Rust as a single static binary

**Status:** accepted · **Date:** 2026-11-08

### Context

The tool must run on macOS, Linux, and Windows, inside a minimal container on a QNAP, and must ship as one executable with no runtime dependency. It needs raw ICMP sockets, interface-level socket options, and an embedded SQLite database.

### Decision

Rust stable, edition 2021, with `rusqlite` and the `bundled` feature so SQLite is compiled into the binary. Linux releases are built for `x86_64-unknown-linux-musl`; macOS and Windows use their native targets. Dependencies stay compile-time only and small: `clap`, `serde`, `toml`, `socket2`, `thiserror`, and the platform crates `libc` and `windows-sys`.

### Consequences

- One artifact per platform, no runtime, no installer, no shared library.
- The build machine needs a Rust toolchain and a musl linker for Linux releases.
- The binary is a few megabytes, dominated by the bundled SQLite.

### Alternatives considered

- Go: viable, and rejected because the operator chose Rust for this project.
- Zig: insufficiently mature SQLite and raw-socket ecosystem for this scope.

## ADR-0003 — Probe with raw ICMP sockets instead of the system `ping`

**Status:** accepted · **Date:** 2026-11-08

### Context

The probe must behave like `ping -I eth1 1.1.1.1`: one echo request, a bounded wait, and a packet constrained to a named interface. The system `ping` differs between platforms — `-I` on macOS and Linux, `-S` and `-4` on Windows — and its output is localised.

### Decision

Open a raw ICMP socket in-process, bind it to the interface with the platform option (`SO_BINDTODEVICE`, `IP_BOUND_IF`, `IP_UNICAST_IF`), send one echo request with a unique identifier and sequence number, and wait for the matching reply up to `timeout_ms`. Classify the outcome as `Verdict::Up { rtt }`, `Verdict::Down`, or `ProbeError::Local`.

### Consequences

- Requires `root`, `CAP_NET_RAW`, or Administrator; the requirement is documented and tested on the deployment target.
- Reply matching must be implemented: a reply from another address or with another identifier is discarded.
- The suite runs unprivileged through the fake prober, so the logic is tested without the privilege.
- No dependency on the host's `ping` implementation, its flags, or its output language.

### Alternatives considered

- Executing `ping` and parsing its output: rejected, because of localisation, flag drift, and one process per probe.
- Unprivileged datagram ICMP: rejected, because interface binding is not portable across the three platforms.

## ADR-0004 — Persist transitions only, in embedded SQLite with WAL

**Status:** accepted · **Date:** 2026-11-08

### Context

Sampling every probe produces a large time series for little information: what matters is when a target changed state. The history must be readable while monitoring runs, on a device where no database server can be installed.

### Decision

Store only state transitions, in two tables: `events` (append-only) and `targets` (current state and last probe outcome). Use SQLite with `journal_mode = WAL`, `synchronous = NORMAL`, a `busy_timeout` of 5 s, and a single writer connection. Retention deletes old `events` rows only.

### Consequences

- Storage is proportional to incidents, not to time.
- Per-probe round-trip history is lost: only the RTT at the transition is kept. Counts of lost packets are not answerable, which is accepted.
- A reader with plain SQLite access can query the file at any time.
- The database must live on a local filesystem.

### Alternatives considered

- Prometheus with remote storage: rejected, it is an external service and a second product to operate.
- InfluxDB or TimescaleDB: rejected, they are servers, not an embedded file.
- Append-only text logs: rejected, because concurrent query and retention would be reimplemented badly.

## ADR-0005 — Mirror every configuration option as a command-line flag

**Status:** accepted · **Date:** 2026-11-08

### Context

The requirement states that every option of the configuration file can also be expressed or overridden on the command line, so an operator can try a single target or a different interval without editing a file — and so the container command can override the mounted configuration.

### Decision

Every key of the TOML file has a command-line counterpart. The effective configuration is the merge `command line > file > default`, computed once before the first probe, and printable with `--show-config`. Targets are given with the repeatable `--target <IP>[,interface=<name>][,label=<name>]`, which replaces the file's list when present. There is no environment-variable layer.

### Consequences

- The `Config` struct and the `clap` definition must stay in step; a test walks every key to prove the mapping.
- The precedence rule is a single function with its own tests, not logic scattered across the code.
- Adding a configuration key is a three-part change: field, flag, documentation.

### Alternatives considered

- Environment variables only: rejected, they are not discoverable and not typed.
- File only: rejected, it violates the requirement and makes quick tests awkward.

## ADR-0006 — Ship one binary with `monitor` and `query` subcommands

**Status:** accepted · **Date:** 2026-11-08

### Context

The history must be queryable while monitoring runs, and the deployment on QNAP and in Docker must stay trivial: one image, one executable, nothing else to install.

### Decision

One executable with the subcommands `monitor` and `query` (plus `--show-config`). `query` opens the database read-only and refuses a schema newer than it understands. Both share the configuration loader and the store module.

### Consequences

- One artifact to build, copy, and containerise.
- The query path cannot repair, migrate, or compact a database; those operations belong to `monitor`.
- Version skew between a running monitor and a query binary is possible and handled by refusing a newer schema with a clear message.

### Alternatives considered

- A separate query binary: rejected, it doubles the artifacts and the release matrix.
- An HTTP query API: rejected, it opens a listening port and an authentication surface that the product does not need.

## ADR-0007 — Reload the configuration file while monitoring runs

**Status:** accepted · **Date:** 2026-11-08
**Supersedes:** the restriction to restart after a configuration change, stated in ADR-0001.

### Context

Operators change the target list and the probe interval on a running host, typically over SSH or by editing a mounted file. Restarting the monitor to pick the change up is an outage in the history: until the restart, transitions are not recorded.

The configuration file is small, local, and edited by a human or a deployment tool.

### Decision

Poll the content of the configuration file once per second and compare it with the content that is currently active. When it differs, parse and resolve it through the same precedence chain as at startup (`command line > file > default`), then apply the interval, timeout, target list, and retention without restarting the process, and log one `INFO` line stating that the configuration was updated.

A file that cannot be read, does not parse, or resolves to no targets is rejected with a warning and the running configuration stays in effect. A changed database path is reported and applied only at the next start, because the open connection owns the schema and the write lock.

### Consequences

- A change is applied within about one second, at the granularity of the poll.
- Applying a change restarts the per-target workers. The current state is reloaded from the database, so no duplicate or spurious transition is written.
- Retention follows the same rule but is evaluated on its hourly pass, so a new window can take up to an hour to apply.
- The database path, the schema, and the command line overrides are never changed by a reload.
- A half-written file is safe: it is rejected, and the next poll reads the completed content.

### Alternatives considered

- An OS file watcher (`inotify`, `kqueue`, Windows change notifications): rejected, it adds a dependency and platform-specific edge cases for a file read that costs microseconds once per second.
- A `SIGHUP` reload: rejected as the primary mechanism, because it requires the operator to remember a signal and does not work when the file is edited by a controller; it also cannot be exercised in a container without `docker kill -s HUP`.
- A restart: rejected, it interrupts the recording of transitions for the time of the restart.

## ADR-0008 — Serve a read-only web page from the binary

**Status:** accepted · **Date:** 2026-11-09
**Supersedes:** BR-8 and the "no web interface, no listening socket" exclusion stated with ADR-0001.

### Context

The history lives in a SQLite file, and reading it from a NAS means either SSH plus `query` or copying the file. A picture of the last day per target — when it went red and how long it stayed red — answers the usual question faster than a table.

The product rule said the tool opens no port, so that rule has to be changed deliberately, not silently.

### Decision

Add a `serve` subcommand to the same binary. It binds the address given by `--listen` (default `127.0.0.1:8642`) and answers three things with a hand-written HTTP/1.1 loop: the page, the same page with only the chart fragment for a zoom, and a health check.

The chart is SVG generated in Rust from `store::timeline`, which turns the events into segments that cover the window exactly. The browser only asks for another window; there is no client-side chart library and no JSON API.

Every request opens the database read-only, so the page sees new transitions without restarting and can never write. The page has no authentication: the default bind is localhost, and LAN exposure is an explicit choice.

### Consequences

- The monitor still opens no port; only `serve` does, and only while it runs.
- A page view is a read-only transaction: it cannot corrupt, migrate, or lock the history beyond the WAL read.
- The chart has no tooltips beyond the native `<title>`, no series other than the state bands, and no live refresh: a reload shows the new state.
- Adding a second process to the container is enough; it needs the same volume, no `NET_RAW`.
- No authentication means the port must be firewalled when it leaves localhost.

### Alternatives considered

- A static HTML report from `query report`: rejected by the operator, because on a NAS it has to be regenerated and downloaded before it can be looked at.
- A separate sidecar or nginx: rejected, it adds a second artifact to build, ship, and keep in step with the schema.
- A client-side chart library (uPlot, ECharts, Chart.js): rejected, the chart is four to eight rectangles per target and a time axis; a library adds weight and a supply chain for features not used.
- A JSON API: rejected, nothing consumes it yet; the page is generated where the data is.

## ADR-0009 — Serve the page from the monitoring process

**Status:** accepted · **Date:** 2026-11-09
**Amends:** ADR-0008. The page, the hand-written HTTP loop, and the server-side SVG stay as decided there; only the process that serves them changes.

### Context

ADR-0008 put the page in `serve`, a second process over the same database. On a NAS that means two containers, two restart policies, and two mounts to keep in step, only to look at the history of a process that is already running. The operator asked for one command that records and shows, and for numbers that answer "how bad was today" without reading a table: disconnections and total down time per target, plus a per-day view of the last month.

### Decision

`monitor` accepts `--listen ADDRESS:PORT` and runs the `web` component in its own thread, sharing the `Shutdown` handle. The listener is bound before the first probe, so an unusable address fails at start instead of minutes later, and the web thread is joined when the monitor stops.

The address is a command-line option and not a configuration key: the page is a way of looking at a running process, not a property of the monitored targets, so it stays out of the file a controller rewrites.

The timeline page adds two columns per lane — the number of disconnections and the total down time of the window being shown — computed from the same segments that draw the bands. The columns follow the displayed window, so zooming changes them; a down segment that started before the window opened counts as down time but not as a disconnection.

Selecting a lane opens `/target?target=…`, which resolves an address or a label to one target and renders one bar per day for the last 30 days from the `down` transitions, bucketed by UTC day with zero-filled days.

### Consequences

- One process, one container, one restart policy for the common case; `serve` stays as a viewer that needs no `NET_RAW`.
- The web thread opens its own read-only connection, so the writer is never blocked beyond the WAL read and the page can never write.
- The page shows a running monitor: a stopped monitor leaves the page unreachable, which is the intended signal.
- With host networking there is no port to publish; without it, the operator publishes the port of the monitor.
- The numbers depend on the displayed window, so the statistics of a zoomed chart are not those of the last 24 hours.

### Alternatives considered

- Two compose services, as in ADR-0008: rejected, it duplicates the lifecycle of the same artifact for a read-only view.
- A configuration key for the address: rejected, a page is not configuration of the monitored targets, and it would invite a controller to move the listener by accident.
- A JSON API with a client-side chart: rejected for the reasons in ADR-0008, and it would add a second contract to keep stable.
- A separate `stats` subcommand for the numbers: rejected, the numbers belong next to the bands they describe.

## ADR-0010 — Render the timeline without client-side interaction

**Status:** accepted · **Date:** 2026-11-09
**Amends:** ADR-0008. The page, the hand-written HTTP loop, and the server-side SVG stay; the wheel zoom, the drag pan, the double-click reset, and the fragment endpoint are removed.

### Context

The interactive chart was built and then rejected by the operator: the zoom did not behave reliably in practice and looked wrong on a status board. It also made the per-lane numbers depend on a stray wheel event, so the same page showed different totals depending on how it had been touched.

### Decision

The timeline is generated once, server-side, for a fixed window: the last 24 hours by default, or the explicit `from`/`to` of the URL. The browser reloads the page every 30 s with `<meta http-equiv="refresh">`; there is no `fetch`, no wheel handler, no pointer handling, and no fragment endpoint. A small script turns a click on a lane into a navigation to its detail page, because some engines ignore an `<a href>` that wraps SVG shapes.

Below the chart the page lists the last 10 disconnections of any target — start, duration, and "and counting" for the one still open — read with `Store::recent_downs`.

### Consequences

- One request and one response per open page every 30 s; nothing to synchronise, nothing to break.
- The statistics always describe the window that is drawn, with no hidden client state.
- `/?from=…&to=…` stays available for a bookmarkable window even though no control produces it.
- A narrower window needs a URL edit; `query events` covers the forensic case.
- The page states the refresh interval instead of hiding it.

### Alternatives considered

- Keep the zoom and repair it: rejected, the feature is not wanted on a status board.
- A JavaScript timer or a `fetch` loop instead of the meta refresh: rejected, the browser already provides the behaviour and the script would be more code to get wrong.
- Server-sent events or WebSocket updates: rejected, a second protocol and a long-lived connection for a page read by one person.
- A refresh interval derived from `interval_ms`: rejected, the page is read by a human, and 30 s is already below the attention span of a monitoring glance.
