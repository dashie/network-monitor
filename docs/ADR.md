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
