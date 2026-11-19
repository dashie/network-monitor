# Architecture Decision Records — uptime-monitor

An architecture decision record (ADR) captures a decision together with the context
that made it necessary and the consequences it creates. Records are append-only: a
decision is superseded by a new record, never rewritten or deleted. Write a record
when a decision is costly to reverse, affects more than one component, or constrains
future work.

Status values: `proposed`, `accepted`, `deprecated`, `superseded by ADR-XXXX`.

This file is append-only. The product name was normalised across the records below when
the tool was renamed (ADR-0040); the decisions themselves are unchanged.

## Index

| ID | Title | Status | Date | Decision |
|---|---|---|---|---|
| ADR-0001 | Adopt the documentation and repository baseline | accepted | 2026-11-08 | Adopt the standard documents and record the intake assumptions |
| ADR-0002 | Implement in Rust as a single static binary | accepted | 2026-11-08 | Rust stable, `rusqlite` bundled, per-platform static builds |
| ADR-0003 | Probe with raw ICMP sockets instead of the system `ping` | accepted | 2026-11-08 | In-process raw ICMP with native interface binding |
| ADR-0004 | Persist transitions only, in embedded SQLite with WAL | accepted; a third table is added by ADR-0041 | 2026-11-08 | Two tables at the time, event-only writes, concurrent reads |
| ADR-0005 | Mirror every configuration option as a command-line flag | accepted | 2026-11-08 | Precedence `cli > file > default`, effective configuration printed on demand |
| ADR-0006 | Ship one binary with `monitor` and `query` subcommands | accepted | 2026-11-08 | Single artifact; query opens the database read-only |
| ADR-0007 | Reload the configuration file while monitoring runs | accepted | 2026-11-08 | Poll the file every second and apply a valid change without a restart |
| ADR-0008 | Serve a read-only web page from the binary | accepted | 2026-11-09 | Minimal HTTP and server-side SVG; no framework and no client library |
| ADR-0009 | Serve the page from the monitoring process | accepted; the down-time column is withdrawn by ADR-0022 | 2026-11-09 | `monitor --listen` runs the same `web` component; one process records and shows |
| ADR-0010 | Render the timeline without client-side interaction | accepted; the reload is held while a cause is open (ADR-0030) | 2026-11-09 | Fixed window, 30 s meta refresh, no zoom; last 10 disconnections listed |
| ADR-0011 | Stream the history export as a hand-written ZIP | superseded by ADR-0012 | 2026-11-09 | Chunked response, deflate on the fly, paged reads, `flate2` only |
| ADR-0012 | Stream the export as a gzipped CSV instead of a ZIP | accepted | 2026-11-09 | gzip is a single stream: no container, no size known in advance |
| ADR-0013 | Import the exported CSV back into the database | accepted | 2026-11-09 | One positional source, format sniffed, batched writes, idempotent |
| ADR-0014 | Replace the window the archive covers on import | accepted | 2026-11-09 | `--replace-window`: delete the covered period, rebuild the projection |
| ADR-0015 | Export every event so one file serves export and import | accepted | 2026-11-09 | One row per event with label and interface; round trip is lossless |
| ADR-0016 | Build the container image with a dev profile and a cached `target` | accepted | 2026-11-10 | `release-dev` profile plus a BuildKit cache mount; `PROFILE=release` publishes |
| ADR-0017 | Localise the instants in the browser and offer the window as links | accepted | 2026-11-10 | UTC in the markup, `Intl` in the browser; `/?window=6h` for 24/12/6/4/2/1 h |
| ADR-0018 | Report the average `up` run with its spread and the availability | accepted; the availability figure is withdrawn by ADR-0022 | 2026-11-10 | Mean ± σ of the runs, clipped by the window; `unknown` excluded from availability; the day still running states the last 24 hours |
| ADR-0019 | Open the timeline on the last hour | accepted | 2026-11-10 | Default span 1 h; links read 1, 2, 4, 6, 12, 24 h from left to right |
| ADR-0020 | Give every target its own replace window on import | accepted | 2026-11-11 | `--replace-window` deletes per target, inside the span that target's own rows cover |
| ADR-0021 | Offer the detail page 30 to 360 days | accepted | 2026-11-11 | `?days=90d` selects the period, 30 days by default; a count label only where a column can carry it |
| ADR-0022 | Drop the down-time and availability columns from the timeline | accepted | 2026-11-13 | A lane states `DROPS`, `AVG UP`, `MAX`, `UP FOR`; the freed width goes to the bands |
| ADR-0023 | One probe contract with four methods and a prefixed target key | accepted | 2026-11-14 | `Endpoint` per method, one module each, `method://endpoint` as the stored identity; no schema change |
| ADR-0024 | Use rustls, a hand-written DNS codec and socket2 instead of an async stack | accepted | 2026-11-14 | `ring` provider with embedded roots, one UDP question decoded here, `socket2` for interface-bound TCP |
| ADR-0025 | Give every failure a code and a message, record the cause of a fault, and call the rows faults | accepted | 2026-11-14 | `probe::failure` holds the closed set of codes and one template each; schema version 2 adds `cause` and `cause_message`; the CSV grows two trailing columns and `/export/faults.csv.gz` replaces `/export/disconnections.csv.gz`, which answers `308` |
| ADR-0026 | Expose the host personal skills read-only instead of copying them | deprecated: the sandbox is removed | 2026-11-14 | One read-only bind of `~/.copilot/skills` onto the same path; nothing else from the host home |
| ADR-0027 | Notify state changes over Telegram, with a stabilisation window and a dedicated delivery thread | superseded by ADR-0033 | 2026-11-14 | Global `[telegram]` section read from the file only, token redacted everywhere; one message per incident; bounded queue and five attempts off the probe path |
| ADR-0028 | Publish a read-only JSON API under `/api/v1` with an envelope and `limit`/`offset` | accepted | 2026-11-14 | Object root with `data` and `meta`, never a bare array; `limit` 1–1000, errors with a stable code; hand-written JSON, no new crate; an OpenAPI document served by the binary; `Access-Control-Allow-Origin: *` on an unauthenticated read-only surface |
| ADR-0029 | Render the API reference in the browser from the document the binary serves | accepted | 2026-11-15 | `/api/v1/docs` loads a vendored Swagger UI at `/api/v1/openapi.json`, embedded with `include_str!` so no network and no CDN are needed; `validatorUrl` off, `GET`/`HEAD` only; the footer of both pages links to it; a body larger than the socket buffer exposed and fixed a non-blocking accepted socket |
| ADR-0030 | Hold the reload while a cause is open | accepted | 2026-11-15 | The `meta` moves into `noscript`; the script drives the 30 s countdown and holds it while a cause is open, and the open message states the hold and offers `Refresh now` |
| ADR-0031 | Order the lanes by a priority the configuration states | accepted | 2026-11-15 | `priority` on a target, stored in `targets.priority` (schema 3) and written by the monitor on start, reload, and the event that creates the row; page and API order by `priority IS NULL, priority, target` |
| ADR-0032 | Serve a deterministic simulated history from a dedicated `simulate` verb | accepted | 2026-11-15 | A `History`/`HistorySource` seam the page, the API and the export read through; `Store` and a generator that replays whole UTC days from `(seed, key, day)`; no probe, no database, no state kept between requests; `--listen` required, `--history` 1–3660 days, `--seed` |
| ADR-0033 | Deliver notifications through declared channels with a typed message | accepted | 2026-11-16 | A channel is declared once and enabled by name; one typed `Message` rendered per transport; Telegram, webhook and log file; one queue, thread and policy per channel; secrets file-only |
| ADR-0034 | Verify a configuration without starting, promote a candidate only after it probes, and keep one active snapshot | accepted | 2026-11-17 | `check` is a subcommand, not a flag; promotion runs in the monitor after the first probe round; one timestamped backup per deployment and one `.active` recovery file written once per start |
| ADR-0035 | Rotate a log file channel by size, and only on a write | accepted | 2026-11-17 | `max_bytes`/`max_files` on a `logfile` channel, default off; the line is never split, a failed rotation is one `WARN`; the diagnostic log stays on stderr and the deployment bounds it |
| ADR-0036 | Scaffold a local setup with an init verb, not an installer | accepted; the working-directory consequence is amended by ADR-0038 | 2026-11-17 | A fixed folder template with one unprivileged TCP target, a commented ICMP example, `data/` and `logs/`, and an enabled rotating event log; missing or empty folder only, no configuration flags, nothing outside the folder |
| ADR-0037 | Embed the user guide and print it as Markdown, without a renderer | accepted | 2026-11-17 | `include_str!` of `docs/USER-GUIDE.md`; `manual` with `man` alias; raw Markdown to stdout; TTY-only hint on stderr; no renderer, pager, subprocess, man page, or new dependency |
| ADR-0038 | Resolve a configuration file's relative paths against the file | accepted | 2026-11-17 | File paths use the file's directory, flag paths use the working directory, absolute paths are unchanged; rebasing happens once in `config::parse_at` |
| ADR-0039 | Let the configuration file state the listen address | accepted | 2026-11-17 | Top-level `listen`; `monitor` binds it when the flag is absent, `check` verifies it, `serve`/`simulate` ignore it; a changed value needs a restart |
| ADR-0040 | Rename the tool to uptime-monitor | accepted | 2026-11-17 | `uptime-monitor` names the tool, the binary, the crate, the config and database defaults, and the deployment; a hard cut with no alias and no storage change |
| ADR-0041 | Record monitor downtime as unknown instead of carrying the last state | accepted | 2026-11-17 | A single-row `monitor_liveness` table (schema 4); a start after a gap writes `up\|down -> unknown` at the last alive instant; a stale reader overlays `unknown`; a fault splits at the gap so the unobserved span is not down time |
| ADR-0043 | Derive a regular series from the timeline instead of storing samples | accepted | 2026-11-18 | `GET /api/v1/timeseries` projects one lane onto an epoch grid of `step` seconds; half-open window, paging over samples, `unknown` before the first event and after `now`; no new table |
| ADR-0044 | Accept relative and zoned instants on the JSON API through chrono/chrono-tz | accepted | 2026-11-18 | `now`, signed relative instants, dates and date-times without an offset, and `tz` (fixed offset or IANA name); exact `s`/`m`/`h`, calendar `d`/`w`/`mo`/`y`; UTC default; embedded tzdata |
| ADR-0045 | Stream a large series as JSON Lines with the meta on the first line | accepted | 2026-11-18 | `GET /api/v1/timeseries/stream`, `application/x-ndjson`: `{"meta": …}` first, then one sample object per line; same parameters and errors; chunked and gzip like every data route |
| ADR-0046 | Record a reachable-but-slow target as `degraded`, and damp `down` with a consecutive-failure count | accepted | 2026-11-19 | `down_after` (default 1) gates `down`; `degraded_after_ms` (default 0, off) and `degraded_after` (default 3) gate `degraded`; it is a transition with cause `PROBE.SLOW`, counts as reachable for availability, and schema 5 makes an older binary refuse the file |
| ADR-0047 | Make every user-visible behaviour reproducible in the simulator | accepted | 2026-11-19 | A change that adds a state, an event, or a read surface extends `simulate` in the same plan and is gated by the same configuration; an exemption is recorded in the plan and approved by the operator |

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

**Status:** accepted; the data model gains a third table for liveness in [ADR-0041](#adr-0041--record-monitor-downtime-as-unknown-instead-of-carrying-the-last-state); the fourth state of [ADR-0046](#adr-0046--record-a-reachable-but-slow-target-as-degraded-and-damp-down-with-a-consecutive-failure-count) is still a transition, never a sample · **Date:** 2026-11-08

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
**Amended by:** ADR-0030 — the 30 s refresh is driven by the script and held while a cause is open; a browser without the script keeps the `meta`.
**Amends:** ADR-0008. The page, the hand-written HTTP loop, and the server-side SVG stay as decided there; only the process that serves them changes.

### Context

ADR-0008 put the page in `serve`, a second process over the same database. On a NAS that means two containers, two restart policies, and two mounts to keep in step, only to look at the history of a process that is already running. The operator asked for one command that records and shows, and for numbers that answer "how bad was today" without reading a table: disconnections and total down time per target, plus a per-day view of the last month.

### Decision

`monitor` accepts `--listen ADDRESS:PORT` and runs the `web` component in its own thread, sharing the `Shutdown` handle. The listener is bound before the first probe, so an unusable address fails at start instead of minutes later, and the web thread is joined when the monitor stops.

The address is a command-line option and not a configuration key: the page is a way of looking at a running process, not a property of the monitored targets, so it stays out of the file a controller rewrites.

The timeline page adds two columns per lane — the number of disconnections and the total down time of the window being shown — computed from the same segments that draw the bands. The columns follow the displayed window, so zooming changes them; a down segment that started before the window opened counts as down time but not as a disconnection.

Selecting a lane opens `/target?target=…`, which resolves an address or a label to one target and renders one bar per day for the last 30 days from the `down` transitions, bucketed by UTC day with zero-filled days. The same buckets fill a table under the chart that repeats each day's disconnections and adds the total down time of the day, so the exact durations sit next to the bars that sum them.

Every day carries its count, a day without drops included, drawn at the baseline in the muted colour: an unlabelled day is indistinguishable from a day the page failed to measure, and the chart has no other way to say "measured, nothing happened".

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

## ADR-0011 — Stream the history export as a hand-written ZIP

**Status:** superseded by ADR-0012 · **Date:** 2026-11-09
**Relates to:** ADR-0008 (the page and the hand-written HTTP loop it generates).

### Context

The page lists the last 10 disconnections, so the operator asked for the whole history in one file, downloadable from the page itself. The `/tmp` of a `scratch` image may not be writable and the database can grow to hundreds of thousands of events, so the response must not be assembled in memory or on disk before it starts.

### Decision

`GET /export/disconnections.zip` answers `200` with `Content-Type: application/zip` and `Transfer-Encoding: chunked`, and writes a single-entry ZIP while the CSV is generated: local header with the "data descriptor" flag, deflated stream, descriptor with the real checksum and sizes, then the central directory and the end record. Sizes and checksum are therefore never required up front, which is what makes streaming possible without seeking.

The events are read in pages of 1 000 by `Store::downs_after`, keyed on the event id, so every read transaction is short and a slow client cannot hold the WAL checkpoints of the writer.

Compression uses `flate2` with its pure-Rust `rust_backend`. The `zip` crate is kept as a **dev-dependency only**, to read the archive back in tests.

### Consequences

- Memory stays flat regardless of the history size: the response holds one page of episodes and the deflate buffers, nothing else.
- No temporary file, so nothing depends on a writable `/tmp` inside the image.
- About 120 lines of binary format in `web::export`, tested by reading the result back with a real ZIP reader.
- A client that disconnects mid-download leaves a truncated archive; the error is logged and the connection closed.
- The entry uses the 1980 DOS timestamp format, so dates before that are clamped.

### Alternatives considered

- `zip::ZipWriter` at runtime: rejected, it requires `Write + Seek`, and a socket cannot seek; wrapping it in a temporary file would need a writable disk and would delay the first byte until the whole archive exists.
- A stored (uncompressed) ZIP: rejected, it is a copy of the CSV in a wrapper — 298 KB instead of 116 KB on a 90-day history — for no gain in simplicity.
- `Content-Encoding: gzip` on a plain CSV: rejected, a browser that saves the response would keep the gzip bytes without the filename the ZIP gives, and the requirement was an archive.
- A `query export` subcommand: rejected, the download belongs where the numbers are; the CLI already covers scripted access through `query events`.

## ADR-0012 — Stream the export as a gzipped CSV instead of a ZIP

**Status:** accepted · **Date:** 2026-11-09
**Supersedes:** ADR-0011.

### Context

ADR-0011 streamed the export as a ZIP, which needs a container: local header, data descriptor, central directory, DOS timestamps, and a hand-written 32-bit size arithmetic. The page serves one file, so the archive bought nothing that gzip does not give — and the container was about 160 of the 283 non-test lines of `web::export`.

### Decision

`GET /export/disconnections.csv.gz` answers `200` with `Content-Type: application/gzip`, `Transfer-Encoding: chunked`, and a streamed gzip of the CSV, built with `flate2::write::GzEncoder`.

gzip is a single stream: a fixed 10-byte header, deflate data, and a trailer with the checksum and the length — both computable at the end. Nothing has to be known before the first byte, which is exactly the property tar lacks and the reason the ZIP needed a data descriptor.

`zip` disappears from `Cargo.toml`; `flate2` stays the only runtime addition, and the tests decompress the response with `flate2::read::GzDecoder`.

### Consequences

- `web::export` drops from 367 to 180 lines, and the 160 lines of container arithmetic with it.
- The download is smaller than the ZIP of the same CSV: 92 KB against 116 KB for a 90-day history, because there is no per-entry header and no directory.
- No 32-bit size fields of our own; gzip's `ISIZE` is a modulo-2^32 trailer value that readers use only as a hint.
- The file no longer opens with a double click on Windows or macOS: `gunzip`, `zcat`, `tar -xzf -` on Windows 11, or any archive tool. The operator asked for this trade explicitly.
- The CSV, not the compression, is the contract: the endpoint name says `.csv.gz` so the content is obvious from the URL.

### Alternatives considered

- Keep the ZIP: rejected, it is the same data behind a container written by hand for no gain on a single file.
- `tar.gz`: rejected, a tar header carries the file size, so an unknown-length response would need an extra pass over the database with a frozen upper bound, or a buffer.
- `Content-Encoding: gzip` on a plain CSV: rejected, a browser that saves the response keeps gzip bytes under a `.csv` name.
- Offer both formats: rejected for now, two code paths for one file; one format is easy to add later if a Windows-only consumer appears.

## ADR-0013 — Import the exported CSV back into the database

**Status:** accepted · **Date:** 2026-11-09
**Relates to:** ADR-0012 (the export format the import consumes).

### Context

A redeploy replaces the container and, with it, the database: the image is ephemeral, the volume may be recreated, and the history is gone. The page can hand out the history as a gzipped CSV, but nothing could put it back.

### Decision

Add an `import` subcommand that reads the same file the page exports and writes the events back:

- **Source:** one positional argument. `-` is standard input, a string starting with `http://` or `https://` is a URL, anything else is a path.
- **Format:** detected from the first two bytes (`1f 8b` is gzip), so a plain CSV and a gzipped one need no flag.
- **Streaming:** the file is read line by line and events are written in transactions of 500 rows; the archive is never loaded whole, and the URL body is de-chunked while it is parsed.
- **Semantics:** a row becomes a `down` at its start and, when closed, an `up` at its end. Nothing is invented: the export does not carry the `up` that preceded the first disconnection, so the restored history starts with `unknown → down`.
- **Idempotence:** `Store::import_events` inserts an event only when the same target, state, and instant are not there yet, and the target projection takes the minimum `first_seen_at_ms` and the maximum `last_change_at_ms`, so importing the same file twice changes nothing and merging an older history never moves a target backwards in time.
- **Invariants:** the importer tracks the state of each target while it reads and skips a `down` that would follow another `down`, so BR-1 holds even for a file that was edited by hand.

The client is denied by construction: no TLS, so `https` is refused with a message telling the operator to download the file first.

### Consequences

- The round trip restores every disconnection and the current state; it does not restore labels and interfaces, which the next probe of the monitor writes from the configuration.
- A broken line is skipped and counted, never guessed at; the command exits 0 and reports `imported N events … (M rows skipped)`.
- The import writes only the database: no probe, no listener, no configuration needed beyond `--database`.
- A CSV edited by hand is accepted as long as it keeps the columns; the duration and `ongoing` columns are informational and ignored.
- The HTTP client is about a hundred lines and supports only `http://`, `Content-Length`, `Transfer-Encoding: chunked`, and no redirects.
- The import is meant to run with the monitor stopped: the monitor decides transitions from the state it loaded at startup, so a concurrent import leaves it with a stale view of the history it now sits on top of.

### Alternatives considered

- `cp` the SQLite file: rejected for the redeploy case, where the file may be gone; it is still the right tool when it survives.
- An HTTP `POST` upload endpoint on the page: rejected, it turns a read-only page into a write surface reachable from the network, and the CLI already covers the restore.
- A JSON or SQL dump format: rejected, the CSV the page already exports is the one artefact the operator has.
- Restoring by re-deriving events from `query events` JSON: rejected, more parsing code for the same data.
- A full HTTP client crate: rejected, `https`, redirects, and connection pooling are not needed for one GET on a LAN.

## ADR-0014 — Replace the window the archive covers

**Status:** accepted, window scope superseded by ADR-0020 · **Date:** 2026-11-09
**Extends:** ADR-0013 (the import itself).

### Context

A merge only ever adds: importing a corrected archive over the days it covers leaves the old events in place, so the history ends up with both the wrong and the right version of the same outage. The operator needs to re-apply an archive to the same database and get exactly the archive inside it.

### Decision

`import --replace-window` deletes every event of every target inside the window the archive covers, then writes the archive:

- **The window** is `[earliest start, latest end]` of the rows, both inclusive; an open row contributes its start. Everything inside it disappears, whatever the target, because an archive describes the whole period it spans, not only the rows it lists.
- **Two passes.** The window is only known after the last row, so the archive is read once to find it and once to apply it. A path is re-opened; standard input and an URL are spooled once into `.import-<pid>.tmp` next to the database and removed afterwards — an exported URL is regenerated per request, so two downloads would not be the same file.
- **The name says what it does.** `--replace` alone would suggest emptying the database; `--overwrite` hides that the rest of the period survives.
- **The projection is rebuilt.** After the deletion the `targets` rows are recomputed from the events that remain (`MIN`/`MAX`, newest state and round-trip time) and a target left with no event is dropped, so the page never describes a history that is no longer there.
- **The seam is truthful.** Imports write `down`/`up` pairs without deciding a previous state, and one pass at the end recomputes `previous_state` from the timestamps and drops the events that repeat the state before them. The transition at the seam therefore reads `up -> down`, and rows that arrive out of order — a concatenated or hand-sorted file — produce the same chain as an ordered one.
- **An empty archive has no window**, so it is refused before anything is deleted.

### Consequences

- Re-applying a corrected archive is idempotent in the useful sense: the window ends up equal to the archive, the rest is untouched.
- The normalisation pass is global: it also repairs a chain that arrived disordered from an earlier import, and it costs one ordered scan of the table per import.
- The deletion is one statement inside the same process: no partial window is ever visible to a reader mid-import, because each batch commits separately and readers see either side of a commit.
- Importing an archive whose targets are unknown to the database removes events of other targets inside the window: that is the point, and it is also the sharp edge of the option.
- The spool copy costs the size of the archive in disk next to the database, and it is removed on exit.
- `busy_timeout` still applies: a replace on a running monitor has the same caveats as a merge.

### Alternatives considered

- Delete per target, only for the targets in the archive: rejected, it leaves the other targets' events inside the same period mixed with a period that the archive rewrote.
- Delete everything and import: rejected, it is `--replace` and contradicts the requirement to keep the rest of the period.
- Stream with a single pass and delete lazily around each row: rejected, it would delete per row and leave events of other targets inside the window alive.
- Require the archive to carry its own window (a comment header): rejected, the format is the one the page exports and must stay that.

## ADR-0015 — One file for export and import: every event

**Status:** accepted · **Date:** 2026-11-09
**Supersedes:** the disconnection-only export of FR-18 and the row semantics of ADR-0013.

### Context

The export wrote one row per disconnection, with the return to `up` folded into an `ended_at` column. The import read that file and unfolded it. Two shapes for one thing: the file could not be read by a tool that wanted the events, an always-up target was never exported at all, the first `unknown -> up` of every target was lost on a restore, and every change to the table meant a change on both sides.

### Decision

The export writes **every event**, one row each, with the columns the import reads:

```csv
target,label,interface,state,occurred_at,rtt_us
192.168.1.1,gateway,eth1,down,2026-11-09T04:42:01.236Z,
192.168.1.1,gateway,eth1,up,2026-11-09T05:12:01.236Z,15000
```

- `label` and `interface` come from the target row (the events table stores neither the label nor a historical name), so a restore recreates the identity of the target, not just its history.
- `rtt_us` is empty for `down`; `previous_state` is not exported, because it is a property of the chain and the import recomputes it from the timestamps.
- The rows arrive in insertion order, which is chronological per target; the import does not depend on that order anyway.
- The import replays the rows as they are, and refuses the old header with a message that asks for a fresh export instead of guessing.

### Consequences

- A round trip is **lossless**: 5 events in, 5 events out, same states, same instants, same label, interface, and round-trip times; the always-up target comes back too.
- One format to document, one set of columns to keep stable, and a file that any tool can read: a spreadsheet, `pandas`, `sqlite3`.
- The archive is roughly twice the size of the disconnection-only one for the same period, because the `up` rows are back in it.
- `--replace-window` now takes its window from the earliest and latest **event** it carries, which is the same instant range as before.
- A file from an earlier version is refused; there is no compatibility reader to maintain, and the message says what to do.

### Alternatives considered

- Keep both formats and sniff one from the other: rejected, two readers for one feature, and the ambiguity never ends.
- Export events but keep reading the old shape on import: rejected, the same complexity with a longer life.
- Export a database copy instead of a CSV: rejected in ADR-0013 and still rejected; the page has a portable file, and `cp` remains the answer when the volume survives.

## ADR-0016 — Build the container image with a dev profile and a cached `target`

**Status:** accepted · **Date:** 2026-11-10

### Context

The image is built by a Docker context that points at the NAS, so the compilation runs
on that machine: four cores, a 2014-era desktop CPU. The `release` profile asks for fat
LTO and a single codegen unit, and the Dockerfile copied `Cargo.toml`, `Cargo.lock`, and
`src` before one `cargo build`, so every change to a source file invalidated the only
build layer and recompiled all dependencies plus the bundled SQLite amalgamation.

Measured on that NAS, with this crate: a cold build took 2 min 13 s, of which 32 s was
the edit-to-binary cost of the crate alone. The remaining 1 min 41 s was paid again for
every one-line change.

### Decision

- A `release-dev` profile inherits `release` and overrides `lto = "thin"` and
  `codegen-units = 16`. The Dockerfile builds it by default through the `PROFILE`
  build argument.
- The build step mounts a BuildKit cache on `/src/target` and copies the binary out of
  the mount in the same step, so the artifacts survive the layer invalidation without
  ending up in an intermediate layer.
- `PROFILE=release` builds the artifact that leaves the development machine; releases
  keep fat LTO and one codegen unit.

### Consequences

- On the NAS a cold build is 2 min 14 s and a rebuild after a source change is 20 s
  (0 s of it in Docker layer export), against 2 min 13 s before for every change.
- The binary is slightly larger and marginally slower than a fat-LTO one; the
  difference is not measurable for an ICMP monitor and never reaches a published
  artifact.
- The build requires BuildKit (Docker 23 or newer) and a `target` cache in the daemon.
  Clearing the daemon cache costs one cold build and nothing else.
- The dependency-cache trick with a stub `main.rs` was rejected: Cargo fingerprints on
  modification time, and the real sources copied afterwards can look older than the
  stub artifacts, which silently ships the stub binary.

### Alternatives considered

- Keep the single `cargo build` and accept the cold build on every change: rejected, it
  pays 1 min 41 s of dependency compilation for a one-line edit.
- Pre-build dependencies in their own layer through a stub source: rejected, unsafe for
  the reason above, and it needs a `touch` on every source to stay correct.
- Build on the laptop and cross-compile to `x86_64-unknown-linux-musl`: rejected, it
  adds a cross toolchain and a second build path for an image that is deployed from one
  host.
- Lower the optimisation level of the whole `release` profile: rejected, it would
  degrade the published artifact to speed up a development loop.

## ADR-0017 — Localise the instants in the browser and offer the window as links

**Status:** accepted · **Date:** 2026-11-10
**Amends:** ADR-0010. The timeline is still generated once, server-side, with no zoom, no pan, no `fetch` and no client-side data handling; the window becomes selectable with plain links, and the browser formats the instants.

### Context

Every instant on the page was a UTC RFC 3339 string. That is right for a query and wrong for a status board: the reader looks at the page at a glance and thinks in the clock of the room, not in UTC. `BR-6` already separates the two — storage and wire format are UTC epoch milliseconds, and the time zone is a matter of presentation — so this is a presentation defect, not a data one.

The window was fixed at 24 hours unless the operator edited `from`/`to` by hand, and a disconnection that happened an hour ago is easier to read over one hour than over a day.

### Decision

The server writes every instant twice: the machine-readable UTC value in the `datetime` attribute of a `<time>` element, or in `data-datetime`/`data-until` on an SVG `<text>` and `<title>`, and a readable UTC fallback as the text. A small script, served as `/app.js` and loaded at the end of each page, rewrites that text through `Intl.DateTimeFormat` with no explicit locale, so the visitor reads the format and the time zone of its own browser. Nothing else runs on the client: no data is loaded, nothing is re-rendered, no state is kept.

The fallback stays visible when the script does not run, and it always names `UTC`, so a reader is never shown a wrong zone: the page degrades to exact but unlocalised, never to plausible but wrong.

The timeline offers the last 24, 12, 6, 4, 2 and 1 hour as links — `/?window=6h` — with 24 hours as the default and the current one marked `aria-current="page"`. The value is validated against that list and an unknown one falls back to the default, so a hand-edited URL never fails. An explicit `from`/`to` still wins.

The per-day buckets of the detail page stay UTC days, because that is what `BR-6` and the store mean by a day; the browser localises their format with `timeZone: 'UTC'` and the header keeps saying so.

### Consequences

- A readable page for a reader whose clock is not UTC, with no configuration: the server needs neither the locale nor the time zone.
- The page needs a script for the local presentation only. Without it the reader sees UTC, explicitly labelled.
- The window lives in the URL, so the 30 s meta refresh keeps it and a bookmark reproduces it; nothing about the choice reaches the server, so no cookie and no session.
- Tick count and tick width are still decided on the server, so a locale with long month names can crowd the axis of a window longer than a day and a half.
- `/?window=…` joins `/?from=…&to=…` as a public URL contract, and the offered list is a contract too: removing a value breaks the link that used it.

### Alternatives considered

- Format on the server from `Accept-Language`: rejected, the header carries a language and not the time zone of the reader, which is the half that matters here.
- Send the time zone to the server (`?tz=Europe/Rome`) and render there: rejected, every zone becomes a distinct response, and a script is still needed to discover the zone.
- Keep the fixed window and the hand-edited URL: rejected, the operator asked for a control, and a link is the smallest one that works without script.
- A `<select>` with a `fetch` of the new range: rejected, it reintroduces the client-side data handling ADR-0008 removed, for a choice a link already expresses.
- A date library on the server with the zone of the host: rejected, the host runs in UTC and would localise nothing.

## ADR-0018 — Report the average `up` run with its spread

**Status:** accepted · **Date:** 2026-11-10
**Amends:** ADR-0009. The per-lane numbers and the per-day table stay as decided there; two of them are added.

### Context

The page answered "how bad was it" — disconnections and down time — and not "how long does it stay up". The operator asked for the average duration of a continuous `up` run, with a dispersion figure, on the timeline window, on the detail page, and per day.

The obvious companion of a mean is the variance, which is the wrong figure twice over: it is expressed in squared milliseconds, so it cannot be written next to a duration, and it describes a symmetric distribution, while run lengths are heavily right-skewed — a few long runs against many short ones. The median and the interquartile range describe that shape better; the operator chose the mean, and the mean is what the page states, with σ beside it so the unit stays a duration.

### Decision

`web::stats` turns the segments of a window into the `up` runs, then into a mean and a population standard deviation in whole milliseconds, and into an availability share. Every figure comes from the segments the page draws, so a reader can check it against the bands, and none of them needs a query the page does not already run.

A run that the window opens or closes in the middle of **counts as it appears**: the first and the last run are lower bounds, never overestimates. Excluding them would leave a window with zero or one disconnection with no average at all, which is the common case on a healthy link.

The sample size travels with the figure: the lane tooltip and the detail header state how many runs the mean is built on, and an average of one run states no spread at all rather than `± 0s`.

The longest `up` run of the same window travels with the mean, in a `MAX` column on the lane and under the mean of every table row. A window that holds one stable run and a storm of brief ones has a mean that describes neither — an operator who saw `17h` in `UP FOR` read `16m±2h06` as a contradiction — and the longest run is the one figure that says the link was not uniformly bad without replacing the mean the operator asked for.

Availability is the share of the **probed** time spent `up`. `unknown` — before the first probe of a target, or after a gap — is left out of numerator and denominator instead of counting as down, because it is time nobody measured (`BR-4` says a local failure is never a down state; the same reasoning applies to a page that would otherwise report a day-old target as 5 % available).

The day buckets of the detail page are split at the UTC day boundary, so a run that crosses midnight contributes a piece to each day it touches.

The first row of that table is the current UTC day, and a day so far is not a day: in its first hours the mean of its pieces is the mean of one short run, which says nothing about the link. That row therefore states the average `up` run of the **last 24 hours**, a rolling window, tagged `last 24 h` and styled apart, while its disconnections and its down time stay the running totals of the day: those are counts, and a partial count is still the count so far. One name covers the figure on both pages, `AVG UP`, so a reader does not look for two statistics that are one.

### Consequences

- Three figures that a reader can verify by eye against the chart, and a fourth per lane that answers "how long does it stay up".
- A window whose target is down or unknown for its whole length states `—`, not zero: there is no average of nothing.
- The mean is pulled up by a long final run and pulled down by a short first one; the count in the tooltip is what tells the reader how much to trust it.
- The detail page needs the segments, not only the aggregated day buckets, so the route runs `Store::timeline` for the 30-day window and picks the target. One more read per request, no schema change.
- A per-day average and the average of the same period are not the arithmetic combination of one another: the period uses the runs, the days use the pieces. Stated in `BR-10` so nobody reads one as the sum of the other.
- The newest row of the detail table answers two different windows: the day so far for its counts, the last 24 hours for its average. The tag is what keeps that from being read as one window.
- Mean and longest run are read together: `16m±2h06` with `max 17h33` says at once that the link flapped and then settled. Neither figure alone says it.

### Alternatives considered

- Variance instead of standard deviation: rejected, it is in squared milliseconds and cannot sit next to a duration.
- Median and interquartile range: proposed, rejected by the operator in favour of the mean. The tooltip carries the sample size, which is what makes the mean readable.
- Only complete runs inside the window: rejected, it erases the figure exactly when there are one or two disconnections, and it would hide the current run, which is the one a reader looks at.
- Availability over the whole window, `unknown` counting as down: rejected, it reports a freshly deployed target as nearly always unavailable.
- Store the runs or the aggregates in the database: rejected, it adds a table and a migration for figures that are derived from the events in one pass and must match the drawing exactly.

## ADR-0019 — Open the timeline on the last hour

**Status:** accepted · **Date:** 2026-11-10
**Amends:** ADR-0017. The links, their validation, the `aria-current` marker, and the localisation stay as decided there; the default span and the order of the links change.

### Context

ADR-0017 offered the six windows with 24 hours as the default, because that was the span the page had always drawn. In use the page is opened to answer "what is happening now": a day of green bands squeezes a recent disturbance into a strip a few pixels wide, and the reader clicks back to a narrower window on almost every visit. A status board that reloads every 30 s is read in seconds.

### Decision

The page opens on the last hour, and the links read left to right from the narrowest window to the widest — 1 h, 2 h, 4 h, 6 h, 12 h, 24 h — so the reading order matches the zoom. The set of windows, its validation, and the fallback for an unknown value are unchanged: an unknown `window` still falls back to the default, which is now one hour, and an explicit `from`/`to` still wins.

### Consequences

- The first thing a reader sees is the last hour, with the lane statistics, the streak columns, and the last-10 table unchanged.
- The whole day is one click and one bookmark away (`/?window=24h`).
- A disturbance older than an hour is invisible on the default view; the window links exist for exactly that, and the per-target page keeps its 30 days.
- The 30 s refresh keeps whatever window the URL carries, so the choice survives the reload.

### Alternatives considered

- Keep 24 hours as the default: rejected, it is the span a reader has to undo most often.
- Derive the default from the probe interval: rejected, it guesses from a number that says nothing about how the page is read, and hides the choice instead of exposing it.

## ADR-0020 — Give every target its own replace window on import

**Status:** accepted · **Date:** 2026-11-11
**Supersedes:** the window scope of ADR-0014. Everything else in that record — two
passes, the spool, the rebuilt projection, the truthful seam — stays as decided.

### Context

ADR-0014 defined the replace window as one span for the whole database: the earliest start
and the latest end of the archive, with every event of every target inside it deleted. It
rejected a per-target deletion because an archive describes the whole period it spans, and
mixing a rewritten period with events the archive never mentioned was said to be worse
than losing them.

A real migration showed the cost. A history imported from another monitoring tool
covered 3 July to 11 November for the gateway, whose monitor kept writing until the
end, and stopped on 7 November for the DNS monitor, which had been paused. Replacing
the window deleted four days of `1.1.1.1` events that the database had recorded itself
and that the archive did not describe at all: not a stale version of the archive's rows,
but rows the archive had never seen. They survived only in a backup taken before the
import.

The assumption behind ADR-0014 — one archive, one period, every target measured across all
of it — does not hold when the archive is assembled from several monitors, each with its
own life.

### Decision

The window is computed per target, from that target's own rows, and the deletion is scoped
to it:

- **Coverage is a map**, `target -> (first instant, last instant)` of that target's rows.
  A target absent from the archive is not in the map and nothing of it is deleted.
- **The deletion is one statement per target**, `DELETE … WHERE target = ? AND occurred_at
  BETWEEN ? AND ?`, so an archive that ends earlier for one monitor cannot reach the
  events of another.
- **The report still prints one line**, with the span that holds every window: the operator
  reads `replaced 2 events inside the window of each target (… to …)`, and `query` remains
  the place to see per-target detail.
- **Everything else is untouched**: two passes over the same bytes, the spool for stdin and
  URLs, the rebuilt projection, the normalisation of `previous_state`, and the refusal of
  an empty archive.

### Consequences

- Re-applying a corrected archive still makes the period equal to the archive for every
  target it contains, and now leaves the other targets alone even when their events fall
  inside a wider span.
- The invariant an operator can rely on: `--replace-window` only ever deletes rows of the
  targets the file mentions, and only between the first and the last instant that file
  mentions for each of them.
- A target whose monitor stopped before the others keeps its own newer history, which is
  the case that motivated the change.
- The archive is no longer authoritative for the whole span it brackets: the gaps inside a
  target's window are still gaps, and a row of that target inside its window is still
  replaced, so the "archive wins inside the window" property is kept per target.

### Alternatives considered

- Keep the global window and document the hazard: rejected, it was documented and it still
  destroyed data — a documented way to lose rows is a defect, not a feature.
- Add a second flag (`--replace-target`) and leave the old behaviour: rejected, it keeps two
  deletion semantics to explain, and the global one has no user who wants it.
- Delete per row around each imported event: rejected, it cannot express "this target's
  window begins here", so it leaves holes inside the window it claims to replace.

## ADR-0021 — Offer the detail page 30 to 360 days

**Status:** accepted · **Date:** 2026-11-11
**Extends:** ADR-0019 (the timeline windows, whose links this reuses).

### Context

The detail page always drew 30 days. A history imported from another monitor covers
months, and a reader asking "how bad was the summer" had no way to see it: the database
held the answer and the page stopped at a month.

### Decision

The page keeps 30 days as its default and offers 30, 60, 90, 180, and 360 days as links,
shortest first, the selected one marked — the same control, the same markup, and the same
reading direction the timeline uses for its windows (ADR-0019).

- **The period is in the URL**: `/target?target=…&days=90d`, so a bookmark and the 30 s
  reload keep it. `days=90` and `days=90d` are both read; a value outside the five offered
  falls back to 30 days instead of failing, like an unknown `window` on the timeline.
- **The period travels through the query**: `daily_drops`, the timeline segments the run
  statistics come from, and every figure of the header use the same span, so a card never
  describes a different period from the chart under it.
- **The chart labels the axis with about six dates** whatever the period, instead of one
  every five days, which overlapped once the bars got thin.
- **A daily count is drawn only where a column can carry it** (a slot of 24 px or more,
  which is the 30-day view). Past about seven weeks the columns are a few pixels apart and
  the numbers merged into a row of digits; the table below still states every day exactly,
  which is where the reader looks at that zoom.

### Consequences

- The page answers a question that spans months without a new endpoint or a second page.
- The header texts, the page title, and the tooltips name the chosen period, so a
  screenshot can never be mistaken for the default month.
- A 360-day view lists 360 table rows and draws 360 bars; the response grows with the
  period and stays a few hundred kilobytes, all rendered on the server as before.
- The zero-count rule of the 30-day view is not universal: at a longer period the reader
  has the table instead of a wall of overlapping numbers.

### Alternatives considered

- A date-picker pair (`from`/`to`): rejected, the timeline already accepts free instants
  and nobody used them; five links state the intention in one click.
- Keep 30 days and add the whole history as a second page: rejected, two pages that differ
  only in their span, and the reader loses the selector they already know from the
  timeline.
- Label every bar at every period and shrink the font: rejected, unreadable below a certain
  width whatever the font size.

## ADR-0022 — Drop the down-time and availability columns from the timeline

**Status:** accepted · **Date:** 2026-11-13
**Supersedes:** the down-time column of ADR-0009 and the availability figure of ADR-0018.
The average `up` run with its spread, `MAX`, and `UP FOR` stay as decided there.

### Context

A lane carried six numbers: `DROPS`, `DOWN`, `AVG UP`, `MAX`, `UP FOR`, `UP %`. Two of
them restated what the lane already draws. The red bands above the numbers are the down
time of the window, so `DOWN` measured a reader's eye against a stroke; the availability
percentage compressed the same bands into one figure that the detail page never repeats
and that an operator cannot act on from the timeline, because the day where the link
failed is one click away.

Six columns also cost the chart its width: the statistics block took 330 of the 1000 px
of the drawing, and the bands — the reason the page exists — got the remainder.

### Decision

A lane states four figures: the disconnections of the window, the average `up` run with
its spread, the longest `up` run of the window, and how long the target has been `up`
without a drop. The down-time column and the availability column are gone.

The code follows the columns out: `WindowStats` becomes `window_drops`, which counts the
disconnections the window contains, and `up_permille` and `format_share` disappear with
their tests. Nothing else in the binary read them.

The statistics block narrows to the four remaining columns and the freed width goes to
the bands, whose track grows from 480 to 562 px. The first column moves right with the
new boundary, so its header no longer sits under the last tick of the axis.

Down time is still stated per day and for the whole period on the detail page, which is
where a reader compares days and decides what to fix.

### Consequences

- The lane answers "how bad was it" by the bands and the drop count, "how long does it
  stay up" by the mean and the longest run, and "is it up right now, and since when" by
  the streak, in four numbers instead of six.
- A reader who wants the down time of a target on the timeline opens the lane; the figure
  is there for every day of the period, which the timeline figure was not.
- The bands are about a fifth wider for the same page width.
- No schema change, no export change, no new query: every removed figure came from the
  segments the page already had.
- `Availability` leaves the glossary, and `BR-10` keeps only the rule that a run clipped
  by the window counts as it appears.

### Alternatives considered

- Keep the availability and drop `MAX`: rejected, the longest run is the figure that says
  a link was not uniformly bad, and it is the one the operator asked for after reading a
  mean of brief runs as a contradiction.
- Keep six columns and shrink the font: rejected, it trades readability of every column
  for two that repeat the drawing.
- Remove down time from the detail page too: rejected, the per-day table is where the
  figure belongs; the timeline was the wrong place for it, not the product.

## ADR-0023 — One probe contract with four methods and a prefixed target key

**Status:** accepted · **Date:** 2026-11-14

### Context

The product answered "is this address reachable" with ICMP only. An operator needs the same
history for a TCP port, a DNS name, and an HTTPS endpoint, and ICMP is often the one check a
network refuses. The monitor, the store, the page, and the query path were written around one
`Probe` that took an `Ipv4Addr`, and the database keyed a target by that address.

### Decision

A probe is an `Endpoint`: the method (`icmp`, `tcp`, `dns`, `http`) plus the parameters that
identify what is checked and, for HTTP, the statuses that count as `up`. One `Probe` trait
takes an endpoint, an optional interface, and the timeout of the request. Each method is one
module — `probe/icmp.rs`, `probe/tcp.rs`, `probe/dns.rs`, `probe/http.rs` — with its own tests,
and an enum `Prober` dispatches on the endpoint. The identity of a target is the endpoint
rendered as a canonical string (`tcp://1.1.1.1:443`), stored in the existing `target` column:
no new column, no new table. A writable open rewrites the bare addresses of an older history
into their `icmp://` form, once and idempotently.

### Consequences

- Adding a method is adding one file, one endpoint variant, and one dispatch arm; the monitor,
  the store, and the page do not change.
- `--show-config` prints each target with only the keys of its method, and the output parses
  back into the same targets.
- Per-target `timeout_ms` and `interval_ms` replace the global `set_timeout` on the prober:
  the timeout belongs to the request.
- The identity of a history changes on the first writable open of a database written by the
  previous version. An older binary reading the rewritten database would key the same target
  twice; the release notes state it and a backup removes it.
- `ProbeError` must be decided per method, which FR-20 to FR-22 and INV-5 now state.

### Alternatives considered

- A `method` column with a composite primary key: rejected, because every read path, link,
  export, and filter would need to carry two fields for one concept.
- Four separate monitors: rejected, because one history, one page, and one process per NAS is
  the product.
- One `Probe` per target chosen at startup with `Box<dyn Probe>`: rejected, because the
  dispatch is static and an enum keeps the call path visible.

## ADR-0024 — Use rustls, a hand-written DNS codec and socket2 instead of an async stack

**Status:** accepted · **Date:** 2026-11-14

### Context

HTTPS needs TLS, DNS needs a message codec, and interface-bound TCP needs a socket created by
hand. The obvious libraries bring constraints: `hickory-resolver` 0.26 requires Rust 1.88 and a
tokio runtime, while the project declares `rust-version = "1.75"` and runs one synchronous
thread per target; `openssl` would add a C dependency to a binary that must stay statically
linked; `aws-lc-rs` would add cmake and NASM to the build of the container image.

### Decision

- TLS: `rustls` 0.23 with `default-features = false` and the `ring` provider, roots from
  `webpki-roots` compiled into the binary, ALPN `http/1.1`, verification not configurable.
- DNS: a hand-written codec in `probe/dns.rs` for the question and the answer, over one UDP
  socket, with the system resolver used only when a target names no server.
- TCP with an interface: `socket2` for a socket that can be bound before connecting, with the
  platform-specific bind kept in `probe/interface.rs`.

### Consequences

- The project stays on Rust 1.75, synchronous, and single-binary; the musl container build
  still works (a C compiler is already in the builder for `ring`).
- The binary grows by the TLS stack and the root list; INV-8 (no runtime dependency) holds
  because everything is compiled in.
- The trust store is ours: a corporate proxy that re-signs traffic requires a rebuild, and the
  system store is not consulted. Recorded in FR-14.
- The DNS codec is our surface: bounds, pointer loops, and malformed messages are ours to get
  right, and the tests pin them.
- The system-resolver path cannot tell a missing name from an unreachable resolver; a target
  that needs the distinction names its resolver (FR-21).

### Alternatives considered

- `hickory-resolver`: rejected, Rust 1.88 and a tokio runtime for one question.
- System TLS (`openssl`, `native-tls`): rejected, a C dependency and a runtime library in a
  static binary.
- `rustls` with `aws-lc-rs`: rejected, cmake and NASM in the image build.
- Reading `/etc/resolv.conf` ourselves: deferred, it reintroduces platform code for a case the
  explicit resolver already covers.

## ADR-0025 — Give every failure a code and a message, record the cause of a fault, and call the rows faults

**Date:** 2026-11-14
**Status:** accepted

### Context

A target that stopped answering was recorded as a bare fact. The transports knew why — `tcp` told a
refusal from a reset from a timeout, `dns` read the RCODE, `http` held the status — and every one
of those distinctions collapsed into `Verdict::Down`, a unit variant. Local failures were worse: a
free-form `String` built with `format!` at thirty-one call sites, so two sites could describe the
same condition with different words and no log line carried a token to search for.

The vocabulary was wrong at the same time. The rows were called disconnections, which names one
cause out of the eight the methods can report, reads badly next to a `DNS.NXDOMAIN`, and would read
worse as soon as the table carried a row that is not a lost connection.

### Decision

1. Every failure of a probe cycle is a `Failure { code, message }` built from `probe::failure`. The
   **code** comes from a closed set of 26 values — 27 once [ADR-0046](#adr-0046--record-a-reachable-but-slow-target-as-degraded-and-damp-down-with-a-consecutive-failure-count) adds `PROBE.SLOW`, the cause of a degradation, which is a verdict and not a local failure —, uppercase and prefixed with the layer that
   reported it (`ICMP.NO_REPLY`, `TCP.REFUSED`, `DNS.NXDOMAIN`, `HTTP.UNEXPECTED_STATUS`,
   `PROBE.WRONG_METHOD`). The **message** is one sentence per code, declared once as a template and
   rendered with a `Details` record of the occurrence; a detail a code does not supply reads
   `unknown`, so a reader never sees a placeholder.
2. `Verdict::Down` carries the `Failure`; `ProbeError` carries one too, and keeps being never
   persisted (INV-5). `Verdict` therefore stops being `Copy`.
3. A recorded occurrence is a **`Fault`**: `Store::recent_faults`, the heading "Last 10 faults", the
   export path and the file name all use that word.
4. Schema version 2 adds `events.cause` and `events.cause_message`; the migration adds both columns
   to a version 1 database and keeps every row, which reads `NULL` for the cause.
5. The CSV grows the same two columns, at the end, so an archive written before this change still
   imports. `/export/faults.csv.gz` replaces `/export/disconnections.csv.gz`, and the old path
   answers `308` for one release.
6. The page reveals the message of a fault in a row of its own, opened by a click on the fault row.
   This ends the "no client-side interaction beyond following a link" rule of FR-17; a browser
   without JavaScript keeps the message in the `title`.

### Consequences

- An operator reads one vocabulary everywhere: a page row, a log line, a CSV column, and a `query`
  output all state the same code.
- The message is stored as it was rendered, not re-rendered from the current configuration, so
  history states what the probe reported with the timeout in force at that moment. Changing a
  template does not rewrite the past.
- The migration stays in the writable open. A read-only process that finds a version 1 database
  reads it as it is, selecting `NULL` in place of the two columns it lacks, so `serve` and
  `query` never write, can run against a snapshot or a volume mounted read-only, and the page can
  never alter the history (`INV-10`, `BR-8`). The cost is a read path that knows two shapes of the
  same table: the two cause expressions are chosen from the version the open found. A 0.2 database
  read by `serve` or `query` therefore states no cause until a `monitor` or an `import` has
  upgraded the file, which is what the user guide says.
- A database written by 0.3 is refused by 0.2 rather than misread, which the user guide states in
  its upgrade section.
- The three codes the plan's table did not foresee (`ICMP.INTERFACE`, `TCP.CONNECT`,
  `DNS.SERVER_FAILURE`) exist because forcing those failures into a neighbour code would have
  produced a message that lied about what happened.
- The page gains its first control besides the links, and with it a third script of about fifteen
  lines.

## ADR-0026 — Expose the host personal skills read-only instead of copying them

**Status:** deprecated since 2026-11-15 — the dev-container sandbox is removed · **Date:** 2026-11-14

### Context

The sandbox home is a named volume and no host home directory is mounted, so the
personal skills the agent relies on, stored under `~/.copilot/skills` on the host,
were invisible inside the container and were never discovered. Copying them into the
volume with `docker cp` drifts after every edit. Committing them as project skills
puts personal tooling into this repository's history, under its review rules. The
skills already live in their own Git repository, so any second copy is a source of
drift.

### Decision

Bind `${localEnv:HOME}/.copilot/skills` read-only onto `/home/dev/.copilot/skills`,
inside the existing `~/.copilot` volume. Nothing else from the host home is exposed
and the mount is `type=bind,readonly`. This is the only exception to the "no host
home directory" guarantee of the isolation model.

### Consequences

- Host and sandbox share one copy: `git pull` on the host is enough to update the
  sandbox, and a write from the sandbox fails with `EROFS`, so the agent cannot
  corrupt the personal tooling. New skills are created on the host only.
- A skill is trusted content: the agent follows its text and may run the scripts it
  ships, as `dev`, inside the container. Read-only protects the host copy from the
  agent, not the agent from a skill. Capabilities stay dropped, there is no Docker
  socket and no host credential, so the blast radius remains the container and the
  worktree.
- The exception is narrow on purpose: `~/.copilot/config.json`, `session-state/`, and
  the session store hold credentials and conversation history and are never mounted.
- If the host directory is absent, Docker creates an empty root-owned directory at the
  source instead of failing, and the symptom is silently missing skills.
  `${localEnv:HOME}` is resolved on the client, so an unset `HOME` produces the same
  symptom. The runbook checks the source before starting the container.

## ADR-0027 — Notify state changes over Telegram, with a stabilisation window and a dedicated delivery thread

**Status:** superseded by [ADR-0033](#adr-0033--deliver-notifications-through-declared-channels-with-a-typed-message) · **Date:** 2026-11-14

### Context

The tool records exactly the moment a target changes state, and the operator learns
about it only by opening the page or querying the database. The signal is already
precise; what is missing is a push. The alternatives were an external script polling
the SQLite file and a full uptime platform, and the project exists to avoid both.

A naive push is worse than none. With an interval of a few seconds an unstable link
produces one message per probe, and the operator mutes the channel — losing the
outage that mattered. What is wanted is one message per incident, and a recovery
statement only when the recovery is real.

Two constraints shape the decision. The token of a Telegram bot is a credential, so
it must never be printed and it must never be typed on a command line that ends up in
a shell history or a process list. And a notification must never be able to delay a
probe or change the record: `INV-1`, `INV-5` and `INV-7` stay what they are.

### Decision

1. One optional, global `[telegram]` section: `bot_token`, `chat_id`, and
   `stabilize_seconds` (default 60, `0` reports a recovery at once). Without the
   section the binary behaves exactly as before and opens no connection beyond its
   probes. Recording, the schema, the query command and the page are untouched.
2. The section is read from the file only, with no flag and no environment variable.
   This is a deliberate, documented exception to `FR-2` and to `ADR-0005`, taken
   because the token is a secret. `Config` and the client redact it in `Debug`,
   `--show-config` prints `<redacted>`, and no error string ever contains the request
   path, so a `WARN` line is safe to paste into an issue.
3. A recorded `down` is reported immediately, with the target, the instant, and the
   code and message of the cause — the pair `ADR-0025` introduced. A recorded `up` is
   held by a per-target policy and reported only after `up` has held continuously for
   the window; a `down` inside the window cancels it and reports nothing, because the
   operator was already told the target was down. After a notified `up`, the next
   `down` is a new incident.
4. Telegram, not a webhook and not a shell hook: one fixed host over TLS with the roots
   compiled into the binary (`ADR-0024`), no inbound port, no user-supplied URL, no
   command executed from the configuration. Delivery is a `POST` of a
   `x-www-form-urlencoded` body to `api.telegram.org`.
5. The decision runs on the delivery thread, not in the probe loop. Workers hand a
   `Notification` to a bounded queue of 32 messages that drops the oldest instead of
   blocking; the thread applies the policy and sends. A `up` therefore closes its
   window even when the target is probed once every five minutes. One message is
   attempted five times, waiting 1, 2, 4 and 8 seconds, then dropped with one `WARN`;
   a non-retryable answer stops at once.
6. The notifier state is not persisted. A restart reports the first observed state
   under the same rules, which the operator asked for; there is no outbox and no
   catch-up of messages missed while the process was down.
7. A reload never restarts the notifier: a changed `[telegram]` section is reported
   and takes effect from the next start, exactly like a changed database path. The
   per-target windows therefore survive a reload.

### Consequences

- The process gains one optional outbound dependency. `INV-12` states the limit: a
  notification is never a condition of recording, and a delivery failure never changes
  a stored event. The monitor calls the notifier only after `record_transition`
  returned, and the notifier has no access to the store.
- The message is plain text with a fixed layout — state, label and key, cause,
  RFC 3339 instant — so no template configuration is needed and the reader can grep a
  channel by code.
- Per-target settings, quiet hours, and an alternative transport are not supported by
  design: the database stays the integration point for everything else.
- A lost message is not remembered. The state machine records the attempt, so the
  "one `down` per incident" rule holds across a failed delivery, and repairing a
  channel means reading the history.
- The token lives in clear text in the file the operator controls; the runbook asks
  for mode `0600` and gives the BotFather revocation as the recovery.

### Alternatives considered

- **Blocking the probe loop on the send.** Rejected: one slow or unavailable endpoint
  would delay every probe of that target and break the interval contract.
- **A generic webhook or a command hook.** Rejected: a URL or a command from the
  configuration is an arbitrary outbound request or an arbitrary execution, and both
  need a surface this project refuses to have. Telegram gives one fixed host and a
  credential the operator can revoke.
- **Mail (SMTP).** Rejected: a second protocol, authentication, and a delivered-versus-
  accepted ambiguity for no gain over a chat.
- **Persisting the notification state, or an outbox in the database.** Rejected: a
  second source of truth, a schema change, and a queue that grows while the channel is
  broken.
- **`stabilize_seconds` as a command-line flag.** Rejected together with the rest of
  the section: one key of a secret-bearing section on the command line is a leak, and
  a section that is half overridable is worse than one that is not.
- **A second endpoint for retries with a longer backoff.** Rejected: five attempts
  cover the load-shedding answer of the Bot API, and a sixth variant of the retry loop
  would be complexity without a requirement.

## ADR-0028 — Publish a read-only JSON API under `/api/v1` with an envelope and `limit`/`offset`

**Status:** accepted · **Date:** 2026-11-14

### Context

The history is machine-readable only where a machine would have to be placed next to it: the
SQLite file needs the volume, `query --format json` needs a shell on the host, and the CSV
export is a bulk transfer. The page answers people, one window at a time, and offers no filter
beyond the target. A dashboard, a status page, or a bot of the operator's own cannot read the
history over the network the page already uses.

Adding a second surface has two real risks: a dependency that the single executable does not
need, and a contract that drifts from the page and from the store. Both are addressed here
rather than later.

### Decision

1. One prefix, `/api/v1`, on the socket `--listen` already binds. No new listener, no new
   thread, no configuration key, no schema change. The API is as public as the page: it reads
   the database read-only and needs no credential (`BR-8`).
2. An envelope, never a bare array: `{"data": …, "meta": …}` on success and
   `{"error": …, "meta": …}` on failure. `meta` echoes the page (`limit`, `offset`), the
   resolved filters, the total matching rows, and the instant the answer was built, so a client
   never guesses what it asked for or how much there is.
3. `limit` and `offset` on every array, `limit` bounded at 1 000 and defaulting to 100. Cursor
   paging was rejected: the queries are indexed and a page is cheap, and a cursor would add
   state to a stateless read.
4. A closed parameter set per route: a name the route does not accept is a `400`, not something
   silently ignored, and every error states a stable `code` and the offending `parameter`.
5. No new crate. The project already renders JSON by hand for `query`; those primitives moved
   to `json`, and the API uses them. The alternative — `serde_json` — buys nothing for a
   handful of fixed shapes and would be the first dependency this binary does not need.
6. The OpenAPI 3.1 document is hand-written, embedded with `include_str!`, and served at
   `/api/v1/openapi.json`. The test that guards it checks both directions: every documented
   property is answered, and every documented path answers.
7. Records state every measured fact and no constant. A fault opens on a `down` event, so its
   state is `down` and its round-trip time is `NULL` by construction: both are omitted, and the
   interface the probe left from — which alternates — is added.
8. `Access-Control-Allow-Origin: *` with an `OPTIONS` preflight. The data is already public on
   the same host, no credential is read and nothing is written, so a wildcard grants exactly
   what the page grants and lets a dashboard on another origin read it.
9. A data answer is streamed: `Transfer-Encoding: chunked`, one record per chunk, gzip through
   a `GzEncoder` writing into the same chunked stream, and no `Content-Length`. Buffering the
   document and relying on the `limit` cap was rejected: the cap bounds a request, not the
   memory a slow client can make the process hold, and the export already proved the chunked
   writer. `/api/v1/faults` reads in keyset pages of 200 with `(occurred_at_ms, id)` as the
   cursor, so a walk never repeats a row that a concurrent insert pushed down.
10. One file per route: `web/api/mod.rs` states the contract and `targets.rs`, `faults.rs`,
    `daily.rs`, and `openapi.rs` hold one route each. A single `api.rs` was the first form and
    was rejected as it grew: a route is now read, tested, and changed in isolation.
11. An error states a fixed message and the store detail goes to the log. The database path and
    the SQLite text are not part of a public, unauthenticated answer, and the request path is
    not echoed back.
12. The accepted instant range is stated rather than implied, and it is a **fixed calendar
    window**: `1970-01-01T00:00:00Z` to `2100-01-01T00:00:00Z`, both bounds accepted, refused
    with `400` outside it, validated once in `parse_instant` so no route and no formula keeps its
    own copy of the rule. A ceiling anchored to `now` was rejected: on any day of a month, "this
    month" asks for the first day of the next, so a mobile bound refuses the most natural window
    there is. The arithmetic of the daily boundaries saturates as well, as defence in depth for
    callers outside this API. Two guards, one rule, and no input reaches an expression that can
    panic — the full `i64` range could not keep that promise, because calendar arithmetic over
    ±292 million years both overflows and runs for 2×10¹¹ days.
13. The connection accounting is panic-safe: a `Slot` guard counts a worker and releases the
    count from its `Drop`, so a handler that unwinds costs its answer and never a slot. The
    count is the 32-connection gate, and a leaked slot used to make the whole surface deaf.

### Consequences

- The API is a second contract to keep in step with the store and the page. The drift is held
  by tests that run the real server and by the OpenAPI document served from the binary itself.
- A day total applies the rule of `FR-17` instead of a second definition: runs are split at
  midnight and the UTC day still running states the last 24 hours, flagged `partial`.
- Paging a list that grows at the top can repeat a record between two requests. This is stated
  in the user guide with the remedy — a fixed `from`/`to` window — rather than solved with a
  cursor, which would put state on a read-only endpoint.
- A streamed answer has no length, so a client must read it to the end and a failure after the
  status line cannot change the status: the connection ends truncated and the log states why.
  `HEAD` mirrors the streamed headers and states no `Content-Length`.
- The cursor is internal: clients still page with `limit`/`offset`, and the first read of a
  response honours their `offset`, so the transport changed without the contract changing.
- Anyone who does not want the API exposed must not pass `--listen`, the rule the page already
  follows. A flag to publish the page without the API was rejected: it would split one
  read-only surface into two configurations of the same thing.
- The failure contract is part of the API: a request that fails inside the server closes its
  connection with no status line and leaves the next request served normally. Both statements
  were forced by a denial of service found in review — an extreme instant panicked a handler,
  and the panic skipped the release of its connection slot — so a client must treat a silent
  close as a failed request, and the server must never trade capacity for a failure.
- The daily route reads only the page it answers and starts its timeline at the page whenever
  the page lies in the future; a window that reaches the present still adds the last 24 hours
  for the day still running.
- A window may end in the future, and the days it states that have not begun carry no
  measurement: an open disconnection ends at the end of the window but never later than the
  present, and the timeline is read to the present, so no answer invents down time or an `up`
  run for time that has not happened.
- A library caller that hands `up_runs_per_day` or `day_slices` a span wider than the domain
  still pays for the span, day by day: both walks are proved to terminate, not to be constant
  time, and the domain is what keeps the API inside a bounded cost (47 482 days at most).

## ADR-0029 — Render the API reference in the browser from the document the binary serves

**Status:** accepted · **Date:** 2026-11-15

### Context

The API states its contract in an OpenAPI 3.1 document, but only a machine reads it: a person
who wants to call a route reads the raw JSON or the tables of the user guide, and the two are
two copies of the same facts. The standard way to publish that document to a person is a
renderer in the browser, and the usual way to deliver one is a CDN script tag.

A CDN would make an internet connection a condition of reading the documentation of a tool
deployed on a NAS or a LAN, and it would put a third party in the trust boundary that
`ARCHITECTURE.md` states as "no external service is trusted and none is required". Writing the
renderer here was the other option: it would parse the document, which means a JSON reader this
project does not have, and it would re-implement a standard interface to no benefit.

### Decision

1. `GET /api/v1/docs` serves `assets/api/docs.html`, which renders `/api/v1/openapi.json` with
   Swagger UI. The page holds no copy of a route, a parameter, or a property: the document is
   the single source of truth, so there is nothing that can drift from it.
2. Swagger UI 5.35.0 is vendored as two assets — `swagger-ui.css` and `swagger-ui-bundle.js` —
   together with its Apache-2.0 `LICENSE`, and served under `/api/v1/docs/`. They are embedded
   with `include_str!` and compressed like the page, so the executable stays self-contained and
   the reference works on a host with no route to the internet.
3. The page sets `layout: "BaseLayout"`, `validatorUrl: null` and
   `supportedSubmitMethods: ["get", "head"]`: no top bar, no call to `validator.swagger.io` on
   every load, and a **Try it out** that offers exactly the methods the API answers.
4. The prefix `/api/v1/docs` belongs to the reference — the page and the two files —, and an
   address under it that is neither answers `404` in the error shape of the API.
5. The footer of both pages links to the reference, so it is one step from the history the
   reader is already looking at.
6. The reference carries the palette of the application instead of the default one. It opens
   with the masthead of the page and links its stylesheet, and `assets/api/theme.css` restates
   the rest in the tokens of `static/style.css`, starting from the "newspaper" Swagger UI theme
   — flat surfaces, hairlines instead of shadows, no tinted method panels — and using system
   colours (`CanvasText`) plus translucent greys, so it follows the light and the dark scheme
   the page already follows without a second stylesheet per scheme. The vendored files are not
   edited: the upgrade stays a replacement of three files.

### Consequences

- The repository and the executable grow by about 1.9 MB of vendored JavaScript and CSS, which
  the binary embeds for the same reason it embeds the icon. It is the one third-party artefact
  of the web surface; upgrading it means replacing three files and the version this record
  states.
- The reference is a second consumer of the document, not a second contract: the drift guard of
  `tests/web.rs` still checks every documented property against a real answer, and a second
  test checks that the page names the document it renders, that it opens with the stylesheet of
  the page with its own, and that the bundle arrives whole.
- The palette of the reference is ours: a Swagger UI upgrade can rename a class and leave one
  element in the default colours. The theme is read in the browser, against a running server,
  rather than proved by a test; the surfaces it must not touch are named in its comments.
- Rendering happens in the browser, so the reference needs JavaScript. The document and the
  user guide remain readable without it, and the page states the address of the document in its
  own masthead.
- A defect the bundle exposed is fixed in the same change: on macOS an accepted socket inherits
  the listener's non-blocking flag, so `write_all` stopped with `EAGAIN` once a body exceeded
  the socket buffer — about 320 KB. Every earlier answer fitted, so nothing caught it; the
  bundle and the gzipped CSV of a long history do not. `web::handle` now clears the flag and
  serves every connection blocking, and the test asserts the full length of a body larger than
  that buffer.

## ADR-0030 — Hold the reload while a cause is open

**Status:** accepted · **Date:** 2026-11-15
**Amends:** ADR-0010. The 30 s refresh leaves the `meta` of the head for the script; the page, the
server-side rendering, and the refusal of client-side chart state stay.

### Context

The page reloads itself every 30 s from `<meta http-equiv="refresh">`, and a fault opens the
standard message of its cause in a row under itself. A reader who is half way through that
message loses it, and the row it belongs to, at the next tick: the page is a status board read in
seconds, but a cause is a sentence a person reads.

Measured in Chromium against a running server, before choosing a mechanism:

- once the parser has armed the refresh, neither `remove()` on the element nor rewriting its
  `content` calls it off — the armed refresh still fires;
- a `#fragment` in the URL turns the refresh into a same-document navigation, so it silently
  never reloads at all;
- a `meta` inside `<noscript>` is not armed at all when scripting is enabled, and is honoured
  exactly as before when it is not.

The reload therefore cannot be paused after the fact. The page must not arm it in the first place
when it is able to drive the countdown itself.

### Decision

1. The `meta` moves into `<noscript>`: a browser without the script keeps precisely the reload it
   had, and a browser with the script arms no refresh of its own.
2. The script drives the countdown, taking the interval from `data-refresh` on the body, and holds
   it while a cause is open. Closing the message starts a full interval again.
3. The page states the hold in a banner across its width, between the masthead and the content,
   and offers `Refresh now` there. The hold belongs to the page, not to the message: one banner
   states it wherever the reader is, and because the banner sits in the flow it covers neither the
   chart nor the table. It is `hidden` while no cause is open, so a page with nothing open carries
   no empty band. It is drawn as an alert — the red of a fault, a warning sign at its left on the
   centre line of the text, and the action in the same red — because it reports a condition the
   reader did not choose, and the colour is the one the chart already uses for `down`.
4. Nothing else moves: no `fetch`, no polling, no client-side data state.

### Consequences

- The reload becomes client-side behaviour, in the same script that opens causes. A browser
  without the script behaves exactly as ADR-0010 states.
- A reader can hold the page still indefinitely by leaving a message open; that is the intent, the
  page says so, and the refresh is one selection away.
- The interval is stated twice, in the `noscript` and in `data-refresh`, because they are the two
  alternatives for two kinds of browser; a test asserts both, and the script reads the one the
  page carries rather than repeating a literal.
- While a cause is open the content sits one banner lower, and returns when it closes. The shift
  follows a selection the reader made, and a compact bar keeps it small.
- The hold is about the reload only. The data still arrives once per interval, and the footer
  manual still states the 30 s reload.

### Alternatives considered

- Reopen the same cause after the reload — `#fragment` or `sessionStorage`: rejected. The message
  rows are identified by their position in the last ten, so a fault arriving at the top would make
  the page reopen the wrong message; and the page would still blink under the reader every 30 s.
- Pause by removing the `meta` or rewriting its `content` once the page has loaded: rejected,
  measured not to work.
- A note inside the open message, stating the hold where the reader is: implemented first and
  rejected by the operator, because the state is the page's and not the message's, and it is read
  as part of the message rather than as a statement about the page.
- A notice pinned to a corner of the viewport, or a bar pinned to its top edge: rejected, the
  first covers the table at every width measured, the second hides the masthead and the identity
  of the page.
- Fetch the timeline and patch the page instead of reloading: rejected, it is the client-side
  state ADR-0010 removed, for a page a human glances at.

## ADR-0031 — Order the lanes by a priority the configuration states

**Status:** accepted · **Date:** 2026-11-15

### Context

The lanes of the page were ordered by the target key — `ORDER BY target` over the `targets`
table —, which is alphabetical by scheme and address: a board showed the `dns://` targets first,
then `https://`, then `icmp://`, then `tcp://`, whatever the operator cared about. The order that
matters to a person who runs the board is "the link first, then the servers, then the rest", and
it is a property of that deployment, not of the key.

Two constraints shape any answer:

- The page must be able to state the order. `serve` has no targets at all — it reads the
  database and nothing else — so an order that lives only in the configuration file cannot reach
  the page in that mode, and `monitor --listen` would then disagree with `serve`.
- The order must change when the configuration changes, not when a target happens to transition.
  A stable target records no event for days, so "the next event carries the new order" is not an
  answer.

### Decision

1. A target may state `priority = <integer>` in the configuration file or `priority=<integer>` in
   the command-line target form. Any integer, negative included; absent means "no priority".
2. The priority is stored in the `targets` projection — `priority INTEGER`, schema version 3 —
   written by the monitor, which is the only component that has the configuration. The page and
   the API order the lanes with one shared rule:
   `ORDER BY priority IS NULL, priority, target`.
3. The monitor writes it twice, on purpose: on the event that creates a target's row (so a target
   seen for the first time is ordered from its first event), and for every target it already
   knows when it starts or applies a reload (so a change reaches an existing row at once, without
   waiting for a transition).
4. A database written before version 3 is read in the order it always had — the page never
   migrates the file (INV-10) — and the column appears at the next writable open.
5. Two targets may state the same priority: the key breaks the tie, so the order never depends on
   the order of the writes.

### Consequences

- The page keeps one source of truth for its order, the database, and `serve` keeps working with
  no configuration at all.
- The order is a fact about the last configuration a target was seen under: a target removed from
  the configuration keeps the priority it had, exactly as it keeps its label.
- The JSON API lists the targets in the same order as the page. `priority` is not published as a
  property: the contract stays about the state of the targets, and the order is observable
  without it.
- The schema moves to version 3. A 0.5 binary reading this database refuses it (BR-9); a binary
  of this version reading an older database migrates it on the next writable open, as it always
  did.
- An imported archive states no priority (the file has no column for it), so an import leaves the
  priorities of the database as they are.

### Alternatives considered

- Order by label: rejected, it makes the order alphabetical in a field meant for humans and gives
  no way to say "this one first".
- Keep the order in the configuration and sort in the web layer: rejected, `serve` has no
  configuration, so the two modes would disagree.
- Store the order in a separate table: rejected, `targets` already holds the configuration facts
  of a target — label and interface — and one more column needs no second table, no join, and no
  second notion of "a target".
- Renumber on write so the numbers are dense: rejected, the number is the operator's, not ours.

## ADR-0032 — Serve a deterministic simulated history from a dedicated `simulate` verb

**Status:** accepted; extended by [ADR-0047](#adr-0047--make-every-user-visible-behaviour-reproducible-in-the-simulator) · **Date:** 2026-11-15

**Amended:** 2026-11-15 — how busy the board is, is an option: `--fault-rate` states the faults a
day of a target that fails now and then holds, and the life of a target scales it. The shape of
the generator, the determinism and the seam are unchanged.

### Context

Every part of the page — the lanes, the fault table, the daily histogram, the detail page over
360 days, the export — needs a history to show, and until now the only way to get one was to run
a monitor against real targets for real days and wait. Developing the page, demonstrating a
deployment, or reproducing a report about a fault therefore needed either a live network or a
database recorded earlier, and neither shows the window a change affects.

What the simulated mode must be:

- **Opt-in and side-effect free.** No probe, no database read, no database write, nothing kept
  between requests: the same command must be safe to run on a laptop and next to a deployment.
- **Deterministic.** Two runs of the same command must show the same history, down to the
  timestamp of a fault, so a page can be compared, a screenshot explained, and a defect
  reproduced.
- **Bounded.** A request may ask for 360 days; the work must stay proportional to the window
  asked for, not to the whole simulated past.

The obvious implementation — generate the history once, hold it in memory, serve it — fails the
last two: it makes determinism depend on when the request arrives, and it makes memory
proportional to the longest window anybody can ask for.

### Decision

1. **A read seam.** `history::History` states the eleven questions the page, the API and the
   export ask — state, timeline, histogram, faults with their paging, events, export pages — and
   `history::HistorySource` hands out one `History` per request. `store::Store` implements both
   over SQLite (so `serve` and `monitor --listen` read the same seam they always effectively
   read), and `sim::SimulatedSource` implements them over the generator. The web layer names
   only the seam, which is what makes the two modes the same page.
2. **A verb, not a flag.** `uptime-monitor simulate --listen … --history 30d --seed 0` is a
   separate command, because it neither probes nor records and because a flag would have to
   change the meaning of two existing commands at once.
3. **Determinism by construction, not by memory.** A target's day is a pure function of
   `(seed, target key, UTC day)`: which kind of life it has, when its faults start, how long they
   last, what their cause is, how long an answer takes. Nothing is stored; a request regenerates
   the days of the window it asks for. `sim::rng` is a hand-written `splitmix64` seeded by an
   FNV-1a mix of that tuple, so a draw never depends on the order of previous draws and no crate
   is added.
4. **Each method contributes its own failures and latencies.** `probe::{icmp,tcp,dns,http}`
   each expose `simulated_day`, the simulated half of the probe contract, so a simulated failure
   states a code and a message of the vocabulary that method already owns. The generator itself
   knows nothing about probes.
5. **One day is self-contained, and a fault may cross midnight.** A day is generated with the day
   before it, so the day that begins inside a fault opens with that fault; a fault is one outage
   and not two.
6. **The past is whole UTC days.** `--history 30d` covers the thirty days that end today;
   nothing exists before them and nothing after now. A past that started at the instant the
   request arrived would move with the clock and two requests a second apart would disagree.
7. **Aggregation is shared with the SQL path.** Faults, the daily histogram and the current state
   are computed by pure functions in `store` over a list of events, and a drift guard feeds the
   same events to both implementations and asserts they agree.
8. **The mode states itself in the log and nowhere else.** One `INFO` line names the mode, the
   number of targets, the window and the seed, and says that no probe runs and no database is
   used. The page is the page: no banner, no badge, no extra API field.

### Consequences

- The page, the API and the export are exercised against 360 days of plausible history in
  milliseconds, with no network and no fixture database.
- The seeded history is the same on every platform and in every process, so an integration test
  can compare two runs byte for byte (the two answers that state a clock — `generatedAt` and the
  age of a state — are excluded, and the test says so).
- The realism of the simulation is a set of constants in `sim` (how often a target fails, how long
  a fault lasts, the latency ranges), not a shape to be redesigned when it is tuned.
- `web` no longer names the database module; `History` is the only way to read a history, and a
  third source would be a new implementation of one trait.
- The public interface grows by one command and two options; no file format, no schema and no
  configuration key changes, so the mode can be removed by deleting a directory.
- The simulated board has the same absence of authentication as the real one, and the log line is
  the only thing that says which one is running.

### Alternatives considered

- **Two global flags (`--simulate`, `--seed`)** on `monitor` and `serve`: rejected by the
  operator, and rightly — it makes two commands mean something else when a flag is present,
  including "do not write the database you were told to write".
- **Generate once and keep it in memory**: rejected, it forfeits determinism across processes and
  makes memory grow with the requested window.
- **A fixture SQLite database checked into the repository**: rejected, it ages, it cannot cover
  360 days in a reasonable size, and it cannot be re-seeded.
- **A `--seed`-driven `FakeProbe` with a real database**: rejected, it keeps a probe loop and a
  file, so a simulation would need minutes to produce a week of history and would leave state
  behind.
- **Adding a crate for the randomness (`rand`)**: rejected, `splitmix64` is a dozen lines and the
  determinism the mode needs is exactly the part a general crate would leave to its own versioning.

## ADR-0033 — Deliver notifications through declared channels with a typed message

**Status:** accepted · **Date:** 2026-11-16 · Supersedes [ADR-0027](#adr-0027--notify-state-changes-over-telegram-with-a-stabilisation-window-and-a-dedicated-delivery-thread)

**Amended:** 2026-11-16 — an orphan channel is no longer a configuration error: a channel that
no list enables starts, receives nothing, and is reported as one `WARN` line. The model, the
routing and the other refusals are unchanged.

**Amended:** 2026-11-17 — the rotation alternative no longer holds for a `logfile` channel: it
rotates its own file by size ([ADR-0035](#adr-0035--rotate-a-log-file-channel-by-size-and-only-on-a-write)).
The diagnostic log still stays on stderr, and the supervisor rotates it.

### Context

ADR-0027 admitted exactly one transport, Telegram, and welded it to the decision: the policy
produced the text, the notifier carried the text, and the only seam was a `Sender`. The operator
now needs three destinations with different natures — a chat, an automation endpoint, and a
durable file — and needs to scope them: a file may follow one target, a chat may follow all of
them.

The obvious way to scope a channel is to repeat its block on each target. That creates a second
problem instead of solving the first: the same destination is then declared several times, and
the process must decide which scope, level, or format wins. The same configuration would also
hide a mistake — a channel declared but never enabled — as silence.

The log line carries no logger name, so a file sink could not say which stream a line belongs
to, and an operator cannot filter notification diagnostics from process lines.

### Decision

1. **One typed message.** `notify::Message` carries an event — start, stop, reload, channel
   start, target down, target up — with its scope, target key and label, state, cause, and
   instant. Each channel renders it in its own form; no channel parses another's output.
2. **Declared once, enabled by name.** `[[notification]]` declares a channel with a required,
   unique `name` and a `type`; it has no scope. The root `notify` list enables channels for
   global events and as the default for every target; a target's `notify` list replaces it, an
   absent key inherits, and `notify = []` silences the target. A typo in a name, a duplicate
   name, an orphan declaration and a `[telegram]` clash stop the process before the first probe.
3. **One destination, one channel.** A channel is identified by its real output destination:
   the normalised filename, the canonical URL (lower-case scheme and host, resolved port, path
   and query), or the `chat_id`. Two channels may not share one, so one event reaches a
   destination exactly once (`INV-14`).
4. **One queue, thread and policy per channel.** A slow or failing transport delays no other and
   never a probe. Each queue holds 32 messages and drops the oldest; each delivery is rendered
   once and attempted five times, waiting 1, 2, 4, and 8 seconds.
5. **Three transports.** Telegram keeps its text and classification; a webhook posts the JSON
   document (schema 1) with an optional bearer token that requires `https`, never follows a
   redirect, and never prints the token or the URL path; a log file appends one line, as the
   diagnostic format or as one JSON document per line, with its own logger name and level.
6. **A logger name on every line.** The format becomes
   `<RFC 3339> <LEVEL> <logger> [<target>] <message>`, with `monitor` and `notification` as the
   two loggers; a log file channel writes the same shape.
7. **No new crate.** The webhook reuses `rustls` through a shared `tls::connect`, and JSON is
   written by the existing `json::string`; a `serde_json` dependency would grow the static
   binary for one document.
8. **Secrets stay in the file.** The channel declarations and the routing lists have no flag;
   the bot token and the webhook token are read from the file alone, redacted by
   `--show-config`, never logged, and a webhook is named by `scheme://host[:port]` only.
9. **Failures are loud and local.** A channel that cannot open stops the process at startup; a
   live delivery failure is one `WARN` line and affects neither recording nor another channel.
   A changed notification configuration is reported on reload and applies at the next start.
10. **The legacy alias.** `[telegram]` still works for one release: it declares a channel named
    `telegram` and enables it globally.

### Consequences

- Egress is no longer one fixed host: a webhook reaches an operator-supplied URL. TLS
  verification, no proxy, no redirect and an 8 KiB answer bound remain.
- The configuration reads as a list of channels plus a routing table, so a mistake is a named
  error at startup instead of a silent silence.
- A channel has one configuration. Two targets that need two formats or two levels use two
  channels and two destinations, which is the deliberate limit of the model.
- `notify` is split like `probe`: a contract, a policy, a routing and one module per transport,
  each tested with a fake channel and no network.
- The log line format changes for every line; the user guide, `FR-15` and the architecture's
  logging contract change in the same commit.
- The delivery thread now runs per channel; a channel that is enabled nowhere is refused, so a
  thread never runs without a reason.

### Alternatives considered

- **Per-channel `targets` lists with merge rules** (scope union, level precedence, format
  agreement): rejected by the operator — repeating a destination invites contradictions that
  must then be resolved, and the resolution is harder to explain than a name.
- **One policy thread fanning out sequentially**: rejected, a dead endpoint would delay every
  other channel by its full retry budget.
- **One policy thread with a worker per channel**: two levels of queue for no benefit; one
  queue and one policy per channel is the same isolation with fewer moving parts.
- **`serde_json`**: rejected, the document is written by `json::string` and needs no crate.
- **A plugin or dynamic channel loading**: rejected, it would put executable code in the
  configuration and break the single-binary constraint.
- **Rotation inside the tool**: rejected; the operator rotates files with the platform's
  mechanism, and a rotation feature would add state and a failure mode for no requirement.
- **Keeping `[telegram]` as the only Telegram form**: rejected; a second way to declare one
  channel would defeat "declared once", so it is an alias for one release and no more.

## ADR-0034 — Verify a configuration without starting, promote a candidate only after it probes, and keep one active snapshot

**Status:** accepted · **Date:** 2026-11-17

### Context

Deploying a new configuration meant restarting the monitor and learning about its mistakes
one restart at a time. `--show-config` validates the file and the merge, but a missing target
list, an interface that does not exist on the host, an unusable database path, a busy listening
address, an unopenable log file, a revoked Telegram token, and a missing raw-socket privilege
only surfaced after the process started. The mirror-image gap was the reload path: a broken
edit is rejected and the process keeps running the previous configuration **from memory**, while
the file on disk keeps the rejected content, and nothing holds a copy of the configuration that
was working a minute before.

### Decision

1. **Verification is a subcommand, `check`,** not another mode of `--show-config`. It runs the
   conditions a start runs in the order a start runs them — target list, interfaces, event
   database, listening address, channels, and optionally one real probe per target — prints one
   `ok`/`warn`/`fail` line each on stdout, exits non-zero on a `fail`, opens the database
   read-only, sends no notification, and never prints a secret. `--show-config` stays a pure
   printer (FR-12).
2. **Promotion happens in the running process, not in a `promote` command.** `monitor --try`
   reads and verifies the candidate once, keeps its bytes, and arms a promotion. When every
   target has completed its first probe without a local failure, the candidate replaces the main
   file and the watcher is rebased, so the process never re-reads the candidate and never treats
   its own write as an operator edit. A first probe that fails locally swaps the supervisor back
   to the main configuration in place.
3. **The gate is the first probe round, not the `READY` notification.** `READY` says the
   workers started; it does not say a raw socket could be opened or an interface bound. The
   first real outcome of every target is the earliest fact that the configuration works.
4. **Two files, two lifetimes.** The deployment history is one timestamped sibling
   `<main>.<UTC>.bak` per promotion; recovery is `<main>.active`, written once per start and
   never by a reload, so it always means "the last configuration that started and probed" — a
   fixed point, not a moving target. There is no `.active.bak` and no snapshot rotation.
5. **The snapshot holds the accepted bytes, not the `--show-config` output**, because that
   output redacts the tokens and cannot start the process.
6. **The fallback is opt-in.** `--fallback-to-active` starts from the snapshot when the main
   file cannot be used and logs one `WARN`; without it the start fails and the error names the
   snapshot and its age. Starting from an older configuration silently would hide a failed
   deployment.
7. **Secrets stay private.** Promotion publishes through a `0600` temporary and a rename; the
   snapshot is created with mode `0600` on Unix and ignored by Git together with `*.bak`.

### Consequences

- A deployment can be verified while the live monitor runs, promoted without a restart, and
  rejected without losing the configuration that works.
- An absent `--try` file is a quiet `INFO` no-op, so the flag can stay in a unit file forever;
  an identical candidate promotes nothing and produces no backup.
- The monitor grows one decision at start, one gate after the first probe round, and one file
  operation isolated in `promote` and `snapshot`.
- `check` performs outbound traffic (one `getMe`, one webhook connection, and with `--probe` one
  probe per target) and may create a missing log-file channel's file; it changes nothing else.
- A batch of accepted hot edits is not in the snapshot until the next restart, which is what
  the recovery point means and is stated in the user guide.
- The main configuration file is now writable by the process; before, the monitor only read it.

### Alternatives considered

- **`--check` on `--show-config`:** rejected — it would turn a printer into a prober and a
  channel client, contradict FR-12, and force every script that prints the configuration to
  perform network calls.
- **A separate `promote` command run after the fact:** rejected — it cannot know whether the
  configuration probed, so it would promote content that never ran, or require a second process
  to inspect the first.
- **Gate on the `READY` notification:** rejected — it states intent to probe, not a probe result.
- **Rewrite the file after each accepted reload:** rejected — the file would become the process's
  state instead of the operator's, and a rejected edit would be silently overwritten.
- **Snapshot rotation or a history of snapshots:** rejected — the file is rewritten once per
  start, so there is exactly one, and a timestamped backup already covers deployments.
- **Fallback on by default:** rejected — an unattended host would start from an older
  configuration without anyone learning the deployment failed.
- **JSON output for `check`:** rejected — one plain line per check is enough to read and to
  grep, and the read-only JSON API already serves machines.
- **Retention of `.bak` files:** left out of this decision; a promotion is one per deployment and
  a backup deleted at a threshold can be the one the operator needs.

## ADR-0035 — Rotate a log file channel by size, and only on a write

**Status:** accepted · **Date:** 2026-11-17 · Amends the rotation alternative of [ADR-0033](#adr-0033--deliver-notifications-through-declared-channels-with-a-typed-message)

### Context

ADR-0033 rejected rotation inside the tool on the grounds that a log file was one audit sink
and the operator rotated it with `logrotate`. That argument does not survive several channels.
Each `logfile` channel is its own destination with its own file, so a host needs one
`logrotate` stanza per channel, hand-kept in step with the configuration. A container has no
`logrotate` at all, so the file grows until the volume is full and the history goes with it.
`copytruncate` also leaves a window between the copy and the truncation where a line can be
lost, and `create` leaves the process writing to the renamed inode until it reopens.

The diagnostic log is a different case. The tool has no log file: `src/logging.rs` writes
every line to stderr, and that is what composes with `journald`, with `docker logs`, and with a
supervisor. What was missing is that the deployment never bounded that file: the compose
project declared no `logging` stanza, so the default `json-file` driver kept every line for
ever.

### Decision

1. **The channel rotates its own file.** A `logfile` declaration gains `max_bytes` (default
   `0`, which disables rotation and keeps today's behaviour byte for byte) and `max_files`
   (default `5`, range 1–100). An out-of-range value is a configuration error naming
   `notification[i].max_bytes` or `.max_files` before the first probe.
2. **Size is the only trigger.** No clock, no time zone, and no process that has to be awake at
   midnight.
3. **A line is never split.** Before writing a line that would take the file past `max_bytes`,
   the channel renames `<filename>` to `<filename>.1`, shifts the older files up, and deletes
   what falls past `max_files`. A single line larger than the limit is written whole and the
   next line rotates. A rotation happens on a write only: never at start, on a reload, or in
   the `check` verification.
4. **A failed rotation loses nothing.** A rename the filesystem refuses is one `WARN` on
   stderr, and the line is still written to the current file; the tool does not retry in a
   loop.
5. **The diagnostic log stays on stderr, with no `--log-file` flag.** Its destination must not
   depend on a configuration that may be the thing that is broken, and a sink the process opens
   at startup is one more way for it to refuse to start. The deployment bounds it instead: the
   compose service declares the `json-file` driver with `max-size: 10m` and `max-file: "3"`,
   a bare `docker run` passes the same `--log-opt`, `StandardOutput=journal` leaves rotation to
   `journald`, and a shell redirect needs a `logrotate` stanza with `copytruncate`.

### Consequences

- A host needs no `logrotate` stanza for a channel, and a container needs none at all; the
  configuration states the file and its bound together.
- `max_bytes = 0` is the default, so enabling rotation is a deliberate decision and an
  existing deployment keeps one file until the operator asks for otherwise.
- The channel keeps one piece of private state, the current size. No other component reads it
  and the `Channel` contract does not change.
- `<filename>.1` is the most recent rotation and a higher suffix is older, which is what
  `logrotate` produces by default and what a `tail -F` reader follows.
- The user guide states the whole map — terminal, systemd, redirect, container — in one place,
  because the answer depends on how the tool runs, not on the tool.

### Alternatives considered

- **Keep rejecting rotation and require `logrotate` per channel:** rejected — a container has
  no `logrotate`, and one hand-kept stanza per channel drifts from the configuration.
- **Time-based rotation:** rejected — it makes behaviour depend on a clock and a time zone, and
  a process that was down at midnight skips the rotation.
- **Compression of rotated files:** rejected — it adds a codec and a write path for a file a
  host keeps for days.
- **Reuse `max_bytes`/`max_files` for a log file of the tool's own:** rejected — it duplicates
  every line, diverges from `journalctl` and `docker logs`, reads its settings from a
  configuration it may have to report on, and adds one more way for the process to refuse to
  start. The one-place-to-configure argument is answered by shipping the compose stanza and by
  journald's host-wide bound.
- **Rotate at start or in `check`:** rejected — inspecting a configuration must not rename the
  file an operator is reading.

## ADR-0036 — Scaffold a local setup with an init verb, not an installer

**Status:** accepted · **Date:** 2026-11-17

### Context

The first run needs a configuration, a predictable database location, and a log directory. The
sample configuration is a reference, not a scaffold: it lists several targets, several need
`root`/`CAP_NET_RAW`, and its database path is a reference. `Config::resolve` resolved
`database` against the process working directory at the time of this decision (amended by
ADR-0038), and a `logfile` channel creates its file but not its directory (`FR-29`). The product is one binary for appliances and containers, where the
repository may not be present, and the NFR says "no installer".

### Decision

1. Add `init <FOLDER>`, one-shot and with no options of its own. It accepts a missing or empty
   folder and refuses a file or a non-empty folder, so it never overwrites.
2. The template is fixed: the defaults, one unprivileged TCP target, a commented ICMP example,
   `data/` and `logs/`, and an enabled size-rotated `logfile` channel `events` at
   `logs/events.log`.
3. `init` validates the template through the ordinary parse/resolve pipeline before touching the
   folder, writes the configuration last with `create_new`, and writes nothing outside the
   folder.
4. `init` is not an installer: no `/etc`, `/var/lib`, user, supervisor, or `systemctl`. A later
   `install` may reuse the template, but that is a separate decision.
5. The generated configuration is user-owned; `init` never overwrites or merges it.

### Consequences

- A first run is three commands: `init`, `check`, `monitor`; the folder is the unit of
  deployment and is run from its own directory.
- Relative paths are not rewritten on disk; the printed `cd` makes the working directory
  explicit. *(Amended by [ADR-0038](./ADR.md): a relative path in the file now resolves
  against the file's directory, so the `cd` is no longer required.)*
- `data/` is a convention (`Store::open_rw` would create it), while `logs/` is required by the
  channel.
- Existing behaviour and configuration precedence are unchanged; no key is added.

### Alternatives considered

- **Documentation-only or copying the sample:** rejected — the sample is not a clean first run.
- **A full OS installer:** rejected — a separate decision, and against the no-installer NFR.
- **`--target`, `--listen`, or `--force`:** rejected — they duplicate the configuration file or
  weaken the one-shot contract.
- **A diagnostic log file or a `2>>` recipe:** rejected — rotation belongs to the supervisor and
  `ADR-0035` keeps diagnostics on stderr.
- **A generated run script:** rejected — no shell script belongs in the execution path.
- **Defaulting to `.`:** rejected — the command's purpose is to create a folder, and `.` can be a
  non-empty repository.

## ADR-0037 — Embed the user guide and print it as Markdown, without a renderer

**Status:** accepted · **Date:** 2026-11-17

### Context

The binary is shipped to appliances and containers where the repository and the website may not
be present. The user guide is the standalone manual; the binary already embeds HTML, OpenAPI,
and Swagger UI with `include_str!`. A true mdcat-like renderer would need a parser and renderer
dependency plus table and wrapping code, and the guide has 136 table rows, 30 code blocks, and
lines up to 537 characters, so a half-renderer is worse than raw Markdown. A curated short
manual would duplicate the guide and drift; a native man page would need a second format and an
installation step.

### Decision

1. `uptime-monitor manual` prints the exact bytes of
   `include_str!("../docs/USER-GUIDE.md")` to stdout; `man` is a visible alias.
2. No renderer, pager, subprocess, native man page, or curated duplicate is added.
3. When stdout is a terminal, one hint goes to stderr; otherwise stdout is pure Markdown for
   `less -R` or `mdcat --ansi`.
4. The command reads no configuration, starts nothing, and treats a closed stdout pipe as a
   quiet success; any other write error exits non-zero.

### Consequences

- The manual is offline, version-locked to the build, and there is one source of truth.
- The binary grows by about 54 KB against a 15 MB footprint limit.
- The raw output is readable but not styled; the operator pipes it to a pager or renderer when
  they want one.
- A closed stderr cannot turn a successful manual into a panic.

### Alternatives considered

- **An mdcat-like renderer:** rejected — a parser and renderer dependency plus table and
  wrapping code, conflicting with the minimal-dependency principle.
- **A curated short manual:** rejected — it duplicates the guide and drifts.
- **A native roff man page:** rejected — a second format and an installation step.
- **Spawning a pager:** rejected — an external process and platform differences.
- **Printing only when stdout is a terminal:** rejected — it breaks pipelines.

## ADR-0038 — Resolve a configuration file's relative paths against the file

**Status:** accepted · **Date:** 2026-11-17

### Context

A configuration file may name a `database` and a `logfile` channel's `filename`. Until now
every relative path was resolved by the operating system against the process working
directory: `config::normalise_path` joined a `logfile` name to `std::env::current_dir()`, and
the SQLite open did the same for the database. A service manager, a container, or any caller
that names an absolute `--config` therefore had to set a working directory too, or the data
landed somewhere else. The scaffolded folder from ADR-0036 taught "run from this folder",
which does not survive a unit file.

### Decision

1. A relative path written in the configuration file resolves against the directory that
   holds the file: `database = "events.db"` in `/etc/uptime-monitor/config.toml` names
   `/etc/uptime-monitor/events.db`, and a `logfile` `filename` follows the same rule.
2. A relative path given on the command line resolves against the working directory, and an
   absolute path in either layer is used as written. The layer decides the base; there is no
   extra key, environment variable, or per-path switch (BR-3).
3. Rebasing happens once, after parsing, in `config::parse_at`; the merge in
   `Config::resolve` keeps its signature and precedence, and every file the process reads —
   the main file, the candidate, the active snapshot, the reload — goes through it.
4. `.` and `..` are folded lexically and a symlink is not followed, matching the existing
   `normalise_path` contract. Nothing is moved or copied when the base changes.

### Consequences

- `--show-config` states the absolute path the process will use for a path that came from the
  file; a default or a flag value prints as written.
- The scaffolded folder works from any working directory, and a service manager needs only the
  absolute `--config`.
- A deployment that relied on the working directory and upgrades without reading the guide
  will find its database and event log beside the configuration file. The guide states the
  migration (absolute paths) and nothing is rewritten automatically.
- ADR-0036's consequence "relative paths are not rewritten" is amended: the paths are still
  not rewritten on disk, but they are now read against the file.

### Alternatives considered

- **Keep the working directory for every layer:** rejected — it is the inconsistency this
  record removes, and it makes an absolute `--config` depend on an ambient setting.
- **A per-key or per-file switch:** rejected — two ways to read the same path, and a decision
  the operator would have to make for every key.
- **Resolve every path, including a flag, against the file:** rejected — a flag is typed in a
  shell, so the working directory is the base the operator already sees.
- **Follow symlinks to canonicalise the file's directory:** rejected — it makes the result
  depend on the filesystem and diverges from the existing lexical helper.

## ADR-0039 — Let the configuration file state the listen address

**Status:** accepted · **Date:** 2026-11-17

### Context

`monitor --listen` is the only way to publish the page from the monitoring process
(ADR-0009), and `docker-compose.yml`, `docs/DEVELOPER-GUIDE.md`, and every unit file repeated
the address on the command line. Every other configuration key already had a file form, and
`--show-config` could not state the address the process would bind. `serve` and `simulate` are
page-only commands with their own flag and, for `serve`, their own default.

### Decision

1. Add one top-level key, `listen = "ADDRESS:PORT"`, to the configuration file.
2. `monitor` binds the file's address when `--listen` is absent; the flag wins when both are
   given; neither means no page, so INV-11 is preserved.
3. `check` verifies the effective address — the flag when given, the file otherwise — in its
   `listen` line.
4. `serve` and `simulate` keep their own `--listen` flag and ignore the key; `simulate`
   continues to require the flag.
5. A running monitor cannot rebind: a reload that changes `listen` logs one `WARN` and keeps
   the bound address, exactly as a changed `database` is reported and used from the next start.

### Consequences

- A deployment carries the page address in the same file as the targets, and a start that
  copies the file to another host carries it too.
- `--show-config` prints the merged address, so the effective configuration states whether a
  socket will be opened.
- The file can expose the page on `0.0.0.0`; the existing warning that the page has no
  authentication now also applies to a value read from the file.
- The address is not validated at resolve time; `--show-config` prints it, `check` verifies it,
  and `monitor` fails at bind, exactly as the flag behaves today.

### Alternatives considered

- **Keep the flag only:** rejected — it is the last key without a file form and forces a
  second source of truth in every deployment.
- **A `[web]` or `page.listen` table:** rejected — one key does not justify a section, and the
  effective configuration would gain a layer with no second member.
- **Extend the key to `serve` and `simulate`:** rejected for this change — `serve` has a
  documented default and `simulate` deliberately requires an explicit address (ADR-0032).
- **An environment variable:** rejected — there is no environment-variable layer (BR-3).

## ADR-0040 — Rename the tool to uptime-monitor

**Status:** accepted · **Date:** 2026-11-17

### Context

The previous name described one transport among the four the tool probes: ICMP, TCP, DNS,
and HTTP. The tool's value is not the network layer it uses but the record of when a target stops
and starts answering, and the name appears in the binary, the Cargo package, the configuration
file, the database default, the container image, the unit file, and the public URL. Every new
page and runbook had to explain that a name taken from the network layer also covers HTTP
status codes and DNS answers. The project is alpha and has one operator's deployment, so the rename is cheapest
now.

### Decision

1. `uptime-monitor` is the name of the tool, the executable, the Cargo package, and the public
   identity. The library crate is `uptime_monitor`, the default configuration file is
   `uptime-monitor.toml`, and the default database is `uptime-monitor.db`.
2. The rename is a hard cut: no second binary name, no alias symlink, no fallback to the old
   file names, and no environment-variable shim. An existing installation renames the files or
   keeps stating its paths explicitly; the user guide documents both.
3. The schema, the storage format, the REST routes, the configuration keys, and every behaviour
   are unchanged. An existing database is readable under any file name, and the binary migrates
   no file at startup.
4. The deployment names change together: the image and container `uptime-monitor`, the
   configuration mount `/etc/uptime-monitor/config.toml`, the data directory
   `/var/lib/uptime-monitor`, and the volume `uptime-monitor-data`. A deployment that carries
   the old name copies the old volume once, keeps it until the history is verified, and then
   removes it.
5. The landing page and its crawl files state the new name and the new URL. Renaming the Gitea
   repository, the GitHub mirror, and the local checkout is an operator step, not part of the
   binary's behaviour.
6. Earlier records in this file and the plans in `tech-plans/archive/` are re-termed by this
   change; their decisions are unchanged, and this record is the current naming authority.

### Consequences

- For a 0.12 user the binary name, the default configuration and database names, the unit file,
  the image, the container, the mount, the data directory, and the volume all change at once.
  The upgrade steps make this explicit and no event is lost, because the schema is untouched.
- The previous name survives only in the migration instructions, which must state which files
  and volumes to rename; it is not a supported interface.
- The name is generic: other products use "uptime" for availability percentages. The tagline
  stays "Reachability monitoring" and the page states that the tool records transitions, which
  is the differentiator the name alone does not carry.
- Links that search for the product now use "uptime" rather than "network", which matches what
  an operator types when looking for this capability.

### Alternatives considered

- **Keep the previous name:** rejected — it named one of the four methods and forced the same
  correction in every new document.
- **Keep the previous name as an alias binary and fall back to the old file names:** rejected — it
  creates two permanent vocabularies, doubles the surface every test and document must cover,
  and hides the change instead of stating it.
- **Rename the old files automatically on first start:** rejected — a startup that silently
  renames an operator's configuration and database is a surprising write, is hard to undo, and
  contradicts the explicit `init` and configuration-promotion model.
- **`uptime-log`, `reachwatch`, `blip`:** considered; `uptime-monitor` is the clearest to a new
  user, and the tagline carries the precision the name lacks.


## ADR-0043 — Derive a regular series from the timeline instead of storing samples

**Status:** accepted · **Date:** 2026-11-18 · **Extends:** [ADR-0004](#adr-0004--persist-transitions-only-in-embedded-sqlite-with-wal), [ADR-0028](#adr-0028--publish-a-read-only-json-api-under-apiv1-with-an-envelope-and-limitoffset)

### Context

The history stores transitions only ([ADR-0004](#adr-0004--persist-transitions-only-in-embedded-sqlite-with-wal)), and the API publishes those transitions and per-day aggregates. A dashboard, an export, or a comparison between targets needs the state at regular instants instead. Persisting per-probe observations would reverse the storage decision, enlarge the database without adding a fact the events do not already carry, and make the recording cost follow the probe interval rather than the changes that matter.

### Decision

`GET /api/v1/timeseries` derives samples on demand from one target's timeline:

1. The grid is absolute: samples are the multiples of `step * 1000` inside `[from, to)`, so two requests with different windows still line up. `step` is an integer `1`–`86400` defaulting to `5`.
2. The window is half-open: `from` is inclusive and `to` is exclusive, and the first sample is the first grid instant not earlier than `from`. `from` defaults to one hour before `to`, and `to` defaults to the request's `now`.
3. A sample states the timeline segment that contains its instant. Before the target's first event the state is `unknown`; an instant after `now` is `unknown`, because unelapsed time is not measured.
4. `limit` and `offset` count samples. `total` is the number of samples in the whole window, and only the timeline between the first and last sample of the page is read, through `History::timeline_of`, so the cost of a request follows `limit` and not the width of the window.
5. `target` is required, so one answer is one lane; a client that wants every target loops over `/api/v1/targets`.
6. `meta` states `step` and, inside `filters`, the zone the request named, so a client can reproduce the grid.

Nothing is stored: the route reads the same segments the page reads under the same invariants (INV-1, INV-7).

### Consequences

- The series is always exactly the event history: a corrected or imported event changes the samples without a backfill, and no two facts can disagree.
- A page costs one lane read and at most `limit` samples; a very wide window is cheap as long as the page is small, which is why the window is not capped by the domain.
- A target that has never been probed states `unknown` for every sample; a target currently down keeps stating `down` until its return event.
- Instants after `now` are `unknown` rather than the last known state, so a future window states what has been measured, not what a client hopes.
- A client cannot obtain several targets in one answer; the per-target key is repeated in every record, which costs bytes but keeps each record self-contained.

### Alternatives considered

- **Persist one row per probe:** rejected — it reverses ADR-0004 and ADR-0022, makes the database grow with the probe rate, and stores a fact trivially derived from the transitions.
- **Return the timeline segments and let the client resample:** rejected — every consumer would reimplement the grid, the `unknown` rule, and the paging, and the API would not answer the question it was asked.
- **Place samples at `from + k * step`:** rejected — two requests for overlapping windows would produce different grids, so concatenating pages or comparing targets would not line up.
- **Read the whole window before paging:** rejected — a 48 000-day window at five seconds would materialise hundreds of millions of samples for one page.

## ADR-0044 — Accept relative and zoned instants on the JSON API through chrono/chrono-tz

**Status:** accepted · **Date:** 2026-11-18 · **Amends:** the parameter description of [ADR-0028](#adr-0028--publish-a-read-only-json-api-under-apiv1-with-an-envelope-and-limitoffset)

### Context

The JSON API accepted only an RFC 3339 UTC instant or epoch milliseconds. Operators think in local calendar days and relative windows ("the last six hours", "yesterday"), and a whole local day cannot be expressed when the offset changes at a daylight-saving boundary. Consumers therefore converted in their own code, each with its own handling of DST and of the calendar.

### Decision

Every JSON API route that reads `from` or `to` uses one parser, `web::api::instant`:

1. Accepted forms are `now`; a signed relative instant `[+-]N` with the unit `s`, `m`, `h`, `d`, `w`, `mo`, or `y`; epoch milliseconds; and `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM`, or `YYYY-MM-DDTHH:MM:SS` with an optional `.fff` fraction and an optional `Z` or `±HH:MM` suffix.
2. Relative `s`, `m`, and `h` are exact durations. `d`, `w`, `mo`, and `y` are calendar steps in the effective zone (`w` is seven days, `y` is twelve months, and a month clamps the day-of-month), so they survive a DST change as a local calendar would.
3. `tz` accepts `UTC`, a fixed offset such as `+02:00`, and an IANA name such as `Europe/Rome`; it defaults to UTC and only applies to a value without an explicit offset. A suffix always wins.
4. A local time that a zone skips when DST starts is `400 INVALID_PARAMETER`; a local time that repeats when DST ends takes its earlier occurrence.
5. Every resolved instant stays inside the existing calendar domain `1970-01-01T00:00:00Z … 2100-01-01T00:00:00Z`, and the whole request resolves against one `now`.
6. The zone database is embedded at build time by `chrono-tz`; `chrono` supplies the calendar arithmetic. No system tzdb is read.

The page, the CLI, the export, and the import keep their current parsers; this is an API contract decision, not a change to the domain representation, which stays UTC epoch milliseconds (INV-6).

### Consequences

- Two dependencies are added: `chrono` and `chrono-tz`, whose data is compiled into the binary. The musl release and Windows therefore need no system time zone data, but the binary grows and the lock file must pin both crates.
- A local calendar day or month is expressible across DST, and a relative window is stable regardless of the client's clock arithmetic.
- A name with no zone on the server fails at parse time with a fixed `400`, and the store detail never reaches the answer.
- The query string is form-decoded, so a literal `+` arrives as a space; the parser restores it, and a client is still expected to percent-encode `+` as `%2B` in a URL.
- The meta echoes the spelling the request used, not a normalised offset, so two spellings of the same zone can appear in two answers.

### Alternatives considered

- **Fixed offsets only:** rejected — it cannot express DST or a local calendar step, which is the reason the change exists.
- **Read the system time zone database:** rejected — Windows and the musl static build have no reliable copy, so the same request would answer differently per deployment.
- **Full ISO 8601 duration strings (`-PT1H`, `-P1M`):** rejected — more verbose, and `M` is ambiguous between month and minute; the suffixed form matches the existing `--since` grammar.
- **A time zone per parameter:** rejected — one zone per request is simpler to document and to reason about, and a client that needs two zones sends two requests.


## ADR-0045 — Stream a large series as JSON Lines with the meta on the first line

**Status:** accepted · **Date:** 2026-11-18 · **Extends:** [ADR-0043](#adr-0043--derive-a-regular-series-from-the-timeline-instead-of-storing-samples)

### Context

The timeseries route is chunked, but its body is one JSON array: a client that reads a line at a time still has to parse an array, and a page of 100 000 samples is an awkward document to consume incrementally. A consumer that wants to plot or forward samples as they arrive needs a format where each record is independently parseable and the end of a record is a newline.

### Decision

Add `GET /api/v1/timeseries/stream`. It computes the same plan as `/api/v1/timeseries` and writes it as JSON Lines:

1. The media type is `application/x-ndjson`, the most widely understood spelling of JSON Lines; each line is one JSON object terminated by `\n`, no array and no commas.
2. The first line is a meta object, `{"meta": {…}}`, with the same facts as the non-streaming `meta` (page, `step`, target, window, total, generated instant, and `tz` when named). Every following line is one sample, `{"t": …, "v": …}` plus `ts` when requested.
3. The parameters, the grid, the paging, the `ts`/`tz` rendering, and the `limit` bounds are the ones of the non-streaming route; `limit` may reach 100 000.
4. The answer is chunked and gzipped like every other data route, and answers `HEAD`.
5. A refusal (bad parameter, unknown target, missing database) happens before the first line and is the usual JSON error object with its HTTP status. A failure after the first line cannot change the status, so the stream closes and the reason stays in the log, exactly as the chunked routes already behave.

The route is a separate path rather than content negotiation on the same path: a client opts in explicitly, the two forms are separately documented and cached, and a proxy cannot answer with the wrong representation.

### Consequences

- A client reads a sample as soon as its line arrives and needs no incremental array parser; `jq -c` and every line-oriented tool consume it directly.
- The protocol has one implicit rule: the first line is meta, the rest are samples. There is no per-line `type` discriminator, which keeps each sample small; a client that misreads the first line as a sample fails visibly on the missing `t`.
- gzip compresses the stream on the fly; a client that wants to consume incrementally must decompress incrementally, which HTTP clients do by default.
- Two renderings share one plan function, so the grid, the `unknown` rule, and the parameters cannot drift between them.
- The meta line states the total before the samples, so a consumer can size its progress without a second request.

### Alternatives considered

- **Keep only the JSON array:** rejected — it forces an incremental array parser and hides each record until the array closes or the client implements a streaming parser.
- **Choose the form with `Accept: application/x-ndjson`:** rejected — an explicit path is simpler to document, test, and cache, and avoids a proxy or a client library that rewrites `Accept`.
- **Server-Sent Events:** rejected — SSE is a long-lived event protocol with its own framing, reconnection, and `EventSource` constraints; this is a finite document, and NDJSON is the smaller, standard answer.
- **A typed envelope per line (`{"type": "meta"}` / `{"type": "sample"}`):** rejected — it costs bytes on every sample for a discriminator the first-line convention already supplies.
- **Meta as the last line:** rejected — a consumer would not learn the total until the end, and could not tell whether the first line was a sample.


## ADR-0041 — Record monitor downtime as unknown instead of carrying the last state

**Status:** accepted · **Date:** 2026-11-17

### Context

The database recorded transitions only, and the first probe after a restart was compared
against the last stored state. Nothing recorded how long the process had been absent, so a
restart could paint hours in which nothing was measured as `up`, or move the start of a real
outage to the restart. `docs/HISTORY-IMPORT.md` documented the production cost of this and a
manual SQL repair: an `unknown` row at the last measured instant closes the open fault, because
the fault query closes a fault at the next event of any state.

### Decision

Keep a single-row `monitor_liveness` table, owned by `store` like the other tables. The monitor
upserts it before the first probe of a run, refreshes it on a bounded heartbeat, and closes it
at a clean stop.

1. `gap_after_ms` is a global key and `--gap-after-ms` its flag; the default is `60000`, `0`
   disables the feature, and a non-zero value below `5000` is refused.
2. A start whose previous row is older than the threshold writes one `up|down -> unknown` event
   at the last instant the previous run was alive for every configured target whose stored state
   is `up` or `down`. The event is placed strictly after the target's newest event, so the
   existing fault query closes an open fault at the gap start. The first probe then records
   `unknown -> up|down`.
3. A reader whose argument instant is later than the alive instant by more than the threshold
   overlays `unknown`: the current state, the timeline tail, the up-run statistics at that
   boundary, and the detail page. `serve` still opens the file read-only.
4. A fault that spans a gap ends at the gap start. If the target is still down after the
   restart, a new fault starts at the first observation. The unobserved interval is therefore
   never counted as down time.
5. `import` accepts `unknown` rows, so the marker round-trips through an export.

The existing `unknown` token is reused; no new state is introduced.

### Consequences

- The schema becomes version 4. An older binary refuses a version-4 database, so the documented
  pre-upgrade backup is the rollback.
- A restart can create an extra fault when a target is down before and after the gap, in
  exchange for down time that stays a lower bound.
- The heartbeat costs at most one row update every 15 seconds, and nothing when the threshold is
  `0`.
- The liveness row is single-writer, following the existing single-writer assumption; two
  monitor processes on one database are still unsupported.
- A database with no liveness record — including the first start after the upgrade — claims no
  gap.
- A reload never calls `begin_run`, so it records no gap; it refreshes the threshold so readers
  see a changed value at once.

### Alternatives considered

- **A historical `gaps` table or a read-time gap overlay:** rejected — it adds a second source of
  gap truth and a new read path, while the `unknown` events already express the same interval
  and are understood by older readers.
- **A clean-stop marker only:** rejected — a crash is the common case and would produce no gap,
  which is the defect being fixed.
- **One incident across the gap with an upper-bound duration:** rejected — it would count the
  unobserved interval as down time, which the runbook exists to prevent.
- **A separate `indeterminate` state token:** rejected — it changes every reader, the export and
  the import, and the grey `unknown` band already says "not measured".
- **Retroactive gap detection for a database with no liveness record:** rejected — no previous
  liveness exists, so no gap can be claimed.


## ADR-0046 — Record a reachable-but-slow target as `degraded`, and damp `down` with a consecutive-failure count

**Status:** accepted · **Date:** 2026-11-19
**Amended:** 2026-11-19 — the band is light green (`#8bc34a`), not amber, so a `degraded`
stretch separates from the orange of a widened short fault and stays in the green family of a
reachable target (extension E2 of plan 020).

### Context

A single lost packet was an outage: the state machine compared one verdict per cycle with
the last recorded one, so a flaky link produced a storm of false incidents and alerts. On
the other side, a link that stays reachable but slow never transitions at all, so the
history — which stores transitions only — could not express "reachable but bad", and the
operator could not see the degradation a latency graph would show. Both gaps were open
questions in the product document.

### Decision

1. `down_after` is a global and per-target count, default `1`, with `--down-after` as its
   flag: a target that was `up` is recorded `down` only after that many consecutive failed
   probes. One success resets the count, a local probe failure neither increments nor resets
   it, and the first verdict after a start or a gap is still recorded at once (`FR-6`).
2. `degraded_after_ms` (default `0`, which disables the feature) is the round-trip time at
   or above which a reachable probe is slow, and `degraded_after` (default `3`) is the number
   of consecutive slow probes needed to enter `degraded` — and of fast ones to leave it.
   Both are global and per-target keys with a flag.
3. `degraded` is a fourth state and a real transition: persisted in `events` and `targets`,
   drawn as a light-green band, served by the page and the JSON API, exported and imported, and
   notified as `target_degraded` with the cause `PROBE.SLOW` and the round-trip time.
4. `degraded` counts as reachable: it is not a fault, down time ignores it, an `up` run
   continues across it, and a fault that was open closes on it like on any other event.
5. Schema version `5` adds no column; it exists so a binary that predates the fourth state
   refuses a database that may hold `degraded` (`BR-9`) instead of failing to parse it.

### Consequences

- False outages drop, and a degradation becomes visible, notifiable and queryable without a
  new table: the storage stays proportional to transitions (`ADR-0004`, `ADR-0043`).
- Every consumer of the state vocabulary grows by one value, and every exhaustive match must
  be revisited by the compiler; the CSV and the import accept one more token.
- A `degraded` alert can precede an `up` alert, so an operator sees a recovery after a warning
  of the same episode.
- Both features are off by their defaults — `down_after = 1` and `degraded_after_ms = 0` —
  so an existing configuration records exactly what it recorded before.

### Alternatives considered

- Persisting per-probe latency samples and deriving the state from them: rejected, it reverses
  `ADR-0004` and `ADR-0043` and makes the database grow with the probe rate.
- A notification-only degradation signal: rejected, it leaves no history and never reaches
  the page or the API.
- A `slow` flag on an `up` event: rejected, it breaks `BR-1` and the current-state projection
  that every reader shares.
- Symmetric damping with a separate `recovered_after`: rejected as unnecessary complexity;
  one count governs both directions, and a recovery to `up` stays immediate.
- Validating `degraded_after_ms` against `timeout_ms`: rejected, a threshold above the timeout
  simply never triggers, because a probe slower than the timeout is `down`.


## ADR-0047 — Make every user-visible behaviour reproducible in the simulator

**Status:** accepted · **Date:** 2026-11-19

### Context

The simulator ([ADR-0032](#adr-0032--serve-a-deterministic-simulated-history-from-a-dedicated-simulate-verb))
exists to review, demonstrate and test the page, the API and the export without a network and
without waiting days for real data. When the `degraded` state was added
([ADR-0046](#adr-0046--record-a-reachable-but-slow-target-as-degraded-and-damp-down-with-a-consecutive-failure-count)),
the plan left it out of the generator, so the one surface that makes a state reviewable in a
few seconds could not show it: the feature could be read in the code and in the tests, but not
seen, and a reviewer had to trust a hand-built database or a deliberately throttled link.

### Decision

Every feature that adds or changes a user-visible state, event, or read surface must be
reproducible in `simulate`, and that work belongs to the same plan and the same change that adds
the feature — not to a follow-up.

- The simulated form is gated by **the same configuration** the monitor reads, so the board
  cannot show behaviour the configuration disables (`degraded_after_ms = 0` still means no
  `degraded`).
- It stays deterministic: the new behaviour is drawn from a stream keyed by
  `(seed, target key, UTC day, purpose)`, so the same command replays it and a request costs
  what it returns.
- It must be visible through the normal surfaces — the page, the JSON API, the export — and
  covered by `tests/simulate.rs`, which is the acceptance evidence of the change.
- An exemption is allowed only when the behaviour cannot be modelled without a real network,
  device, or wall-clock effect: the plan states the reason and the operator approves it. Silence
  is not an exemption.

### Consequences

- A plan's steps include the simulator work and its tests; "the simulator still emits only
  `up`/`down`" is no longer an acceptable non-goal for a state the monitor can record.
- The generator gains one small, gated rule per feature instead of a parallel model: here a
  slow window inside an up stretch, not per-probe latency samples.
- Review and demonstration stop depending on network conditions, privileges, or time: a
  screenshot, a defect and a comparison all reproduce from a seed.
- The deterministic contract of `ADR-0032` is unchanged; an exemption that breaks it is not
  allowed.

### Alternatives considered

- Build a fixture database by hand or with `import`: rejected, it is not reproducible, not
  shareable as a command, and drifts from the generator.
- Rely on a real slow link (`tc`/`netem`): rejected, it needs privileges and a network, and CI
  must run without either.
- Keep the simulator a fixed `up`/`down` model and accept the gap: rejected by this record;
  it made a shipped state unverifiable through the product's own surface.
