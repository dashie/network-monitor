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
| ADR-0011 | Stream the history export as a hand-written ZIP | superseded by ADR-0012 | 2026-11-09 | Chunked response, deflate on the fly, paged reads, `flate2` only |
| ADR-0012 | Stream the export as a gzipped CSV instead of a ZIP | accepted | 2026-11-09 | gzip is a single stream: no container, no size known in advance |
| ADR-0013 | Import the exported CSV back into the database | accepted | 2026-11-09 | One positional source, format sniffed, batched writes, idempotent |
| ADR-0014 | Replace the window the archive covers on import | accepted | 2026-11-09 | `--replace-window`: delete the covered period, rebuild the projection |
| ADR-0015 | Export every event so one file serves export and import | accepted | 2026-11-09 | One row per event with label and interface; round trip is lossless |
| ADR-0016 | Build the container image with a dev profile and a cached `target` | accepted | 2026-11-10 | `release-dev` profile plus a BuildKit cache mount; `PROFILE=release` publishes |

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

Selecting a lane opens `/target?target=…`, which resolves an address or a label to one target and renders one bar per day for the last 30 days from the `down` transitions, bucketed by UTC day with zero-filled days. The same buckets fill a table under the chart that repeats each day's disconnections and adds the total down time of the day, so the exact durations sit next to the bars that sum them.

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

**Status:** accepted · **Date:** 2026-11-09
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
