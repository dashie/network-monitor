# Architecture — network-monitor

This document follows [arc42](https://arc42.org) and uses C4 diagrams. Product intent
lives in [PRODUCT.md](./PRODUCT.md); decisions and their rationale live in
[ADR.md](./ADR.md).

## 1. Introduction and goals

`network-monitor` is a single-process Rust command-line program. One invocation is either a long-running `monitor` that owns the ICMP probing loop and the SQLite writer, or a short-lived `query` that reads the same database concurrently in WAL mode.

Top quality goals, in order:

1. **Portability without dependencies.** One statically linked executable runs on macOS, Linux, and Windows; nothing else must be installed on the host or in the container.
2. **Storage proportional to incidents.** Only state transitions are persisted, so a month of monitoring costs kilobytes.
3. **Interface-accurate reachability.** A probe bound to `eth1` measures `eth1`, with the semantics of `ping -I eth1`.
4. **Queryable while running.** Reading the history never requires stopping the monitor and never blocks it.
5. **Testability without privileges.** The prober sits behind a contract, so the state machine, the store, and the queries are tested with a fake and no raw sockets.

## 2. Constraints

- **Language and toolchain.** Rust stable, edition 2021, MSRV declared in `Cargo.toml` (1.75 or newer).
- **Static linkage.** `rusqlite` with the `bundled` feature compiles SQLite into the binary; Linux releases are built for `x86_64-unknown-linux-musl`. No system library may be required at run time.
- **Privileges.** Raw ICMP sockets need `root`, `CAP_NET_RAW`, or Administrator on Windows. The container requests only `NET_RAW` and drops every other capability.
- **Platform interface binding.** `SO_BINDTODEVICE` on Linux, `IP_BOUND_IF` on macOS, `IP_UNICAST_IF` on Windows (by numeric interface index).
- **Filesystem.** The SQLite file must live on a local filesystem; WAL on SMB or NFS shares is not supported and is documented as such.
- **Synchronous design.** No async runtime, no daemon framework, no plugin system. Threads are used per target, and that is the whole concurrency model.
- **No listening socket.** The process opens no port; the only integration surface is the SQLite file and the query command.
- **Language of the repository.** English only, for every artefact.

## 3. Context and scope

```mermaid
C4Context
    title System context — network-monitor

    Person(operator, "Operator", "Writes the TOML configuration, starts the monitor, reads the history")
    System(monitor, "network-monitor", "Probes targets with ICMP echo requests and records state transitions")

    System_Ext(hosts, "Monitored hosts", "IP endpoints that answer, or do not answer, ICMP echo requests")
    System_Ext(os, "Operating system", "Provides raw ICMP sockets, the routing table, and interface binding")
    System_Ext(runtime, "Container runtime", "Runs the image on a NAS or server, granting NET_RAW")
    System_Ext(consumer, "Query consumer", "Reads the event database through the query command or any SQLite client")

    Rel(operator, monitor, "Runs and configures", "CLI + TOML")
    Rel(operator, consumer, "Runs queries", "CLI")
    Rel(runtime, monitor, "Launches", "Docker")
    Rel(monitor, os, "Opens raw sockets, binds an interface", "syscalls")
    Rel(monitor, hosts, "Sends echo requests, receives replies", "ICMP")
    Rel(monitor, consumer, "Exposes the event database", "SQLite WAL")
```

| External element | Input to the system | Output from the system |
|---|---|---|
| Operator | Configuration file, command-line options, signals | Human-readable logs on stderr, exit codes, query output |
| Monitored hosts | ICMP echo replies, or nothing | ICMP echo requests, one per target per interval |
| Operating system | Raw socket capability, routing table, interface list, clock | Socket creation and bind calls, packets |
| Container runtime | Process launch, `NET_RAW` capability, mounted configuration and volume | Exit status, log stream |
| Query consumer | Read-only SQL or `query` invocations | Rows and reports |

Trust boundaries: the database file is written only by the `monitor` process and read by anyone with filesystem access to it; the raw socket capability is the only privileged operation. No external service is trusted and none is required.

## 4. Solution strategy

- **One binary, two verbs.** `monitor` and `query` share the configuration, the schema, and the process image. Deployment is one artifact; the query path opens the database read-only and cannot repair it.
- **Event-only persistence with a current-state projection.** An append-only `events` table holds transitions; a `targets` table holds the current state and the last probe outcome. The writer updates both in one transaction, so a reader never sees a state that contradicts the latest event.
- **Layered configuration as data.** Defaults, file, and command line are merged into one immutable effective configuration before anything else runs; each layer is a pure function with a test.
- **Probe behind a contract.** `Probe::probe(target, interface) -> Result<Verdict, ProbeError>` separates the transport from the decision. `Verdict::{Up, Down}` answers "the peer is silent"; `ProbeError::Local` answers "the host cannot probe". The type system keeps the two apart.
- **Platform code isolated in one module.** Only `probe/interface` knows about `SO_BINDTODEVICE`, `IP_BOUND_IF`, and `IP_UNICAST_IF`; the rest of the code is platform-independent.
- **Thread per target, shared store.** Each target runs its own interval loop; transitions are serialized through one SQLite writer connection, guarded by a mutex.
- **WAL for concurrent reads.** `journal_mode = WAL` with `synchronous = NORMAL` and a 5 s `busy_timeout` lets the query process read while the monitor writes.

<!-- skill:block:arch-principles section="## 4. Solution strategy" -->
### Architecture principles

Fixed principles that constrain every design decision and every change:

- **Simplicity first (KISS).** The architecture is the simplest one that satisfies the
  requirements: the fewest components, layers, dependencies, and configuration knobs
  that work. Complexity is admitted only with a documented constraint and recorded in
  [ADR.md](./ADR.md).
- **Modular by default.** The system is split into modules with one responsibility
  each, behind a published contract. Modules are independently testable, independently
  evolvable, and independently debuggable: no dependency on another module's
  internals, no dependency cycles, no change that ripples across unrelated modules.
- **Tests with every change.** Every development effort ships with the tests of the
  part it touches. Testability is an architectural property: a module that cannot be
  tested in isolation, without starting the others, is not modular enough.
<!-- skill:end:arch-principles -->

## 5. Building block view

```mermaid
C4Container
    title Containers — network-monitor

    Person(operator, "Operator", "Configures and queries the monitor")

    Container(cli, "network-monitor executable", "Rust, static binary", "Parses configuration, probes targets, writes transitions, renders queries")
    ContainerDb(db, "Event database", "SQLite, WAL", "Append-only transition events and the current state of each target")

    System_Ext(os, "Operating system", "Raw ICMP sockets and interface binding")
    System_Ext(hosts, "Monitored hosts", "ICMP echo responders")

    Rel(operator, cli, "Runs monitor and query", "CLI")
    Rel(cli, db, "Reads and writes", "SQLite file")
    Rel(cli, os, "Opens a raw socket, binds the interface", "syscalls")
    Rel(cli, hosts, "Sends echo requests", "ICMP")
```

| Component | Responsibility | Published contract |
|---|---|---|
| `cli` | Parse the command line, dispatch the subcommand, map every option to a configuration override, set the exit code | `clap` definition, `Command` enum |
| `config` | Load the TOML file, validate it, merge defaults, file, and overrides into one effective configuration | `Config`, `TargetSpec`, `ConfigError` |
| `probe` | Execute one ICMP echo request for a target through an interface and classify the outcome | `Probe` trait, `Verdict`, `ProbeError` |
| `probe::interface` | Resolve an interface name to the platform handle and bind the socket to it | `bind_socket(fd, interface)` per platform |
| `monitor` | Run one interval loop per target, decide whether a transition occurred, dispatch it to the store | `run(config, prober, store)` |
| `store` | Own the schema, migrations, the event write transaction, retention, and the read queries | `Store`, `Event`, `TargetState`, `StoreError` |
| `query` | Read current state and event history, apply filters, render a table or JSON | `QueryFilter`, `Renderer` |

### Component contracts

### `config`

- **Input:** an optional TOML file and the command-line overrides.
- **Output:** an immutable `Config` with global defaults resolved and every target validated, or a `ConfigError` naming the offending key.
- **Guarantees:** the merge order is `command line > file > default`; no value is invented at probe time; validation (positive interval and timeout, parseable IP, existing interface when privileged checks are possible) happens before the first probe.

### `probe`

- **Input:** a target address and an optional interface name.
- **Output:** `Verdict::Up { rtt }`, `Verdict::Down`, or `ProbeError::Local { reason }`.
- **Guarantees:** one echo request per call; the wait is bounded by `timeout_ms`; a reply from a different address or with a different identifier is discarded; a local failure (socket, permission, interface) is never reported as `Down`.

### `store`

- **Input:** transitions from the monitor, retention commands, read queries.
- **Output:** persisted rows, or `StoreError`.
- **Guarantees:** a transition writes the event and the current state in one transaction; the schema is created and migrated on open; a database from a newer schema version is refused; the writer connection is the only writer in the process.

### `query`

- **Input:** a filter (target, time range, limit) and an output format.
- **Output:** a table or JSON document on stdout, or an error on stderr with a non-zero exit code.
- **Guarantees:** the database is opened read-only; the command works while the monitor holds a write transaction; the JSON shape is stable enough to be parsed by scripts.

### `monitor`

- **Input:** the effective configuration, a `Probe` implementation, a `Store`.
- **Output:** database writes, log lines, and a process exit status.
- **Guarantees:** at most one in-flight probe per target; a transition is written at most once per cycle; shutdown on `SIGINT` or `SIGTERM` happens between two cycles, never inside a transaction.

### Data model

Two tables, owned exclusively by the `store` component. Timestamps are UTC epoch milliseconds.

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;
PRAGMA busy_timeout = 5000;

CREATE TABLE IF NOT EXISTS schema_meta (
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    target         TEXT    NOT NULL,   -- IP address, canonical form
    interface      TEXT,               -- null when the default route was used
    previous_state TEXT    NOT NULL,   -- 'unknown' | 'up' | 'down'
    state          TEXT    NOT NULL,   -- 'up' | 'down'
    occurred_at_ms INTEGER NOT NULL,   -- UTC epoch milliseconds
    rtt_us         INTEGER             -- null when state is 'down'
);

CREATE INDEX IF NOT EXISTS events_target_time
    ON events (target, occurred_at_ms);

CREATE TABLE IF NOT EXISTS targets (
    target            TEXT PRIMARY KEY,
    interface         TEXT,
    label             TEXT,
    current_state     TEXT    NOT NULL,   -- 'unknown' | 'up' | 'down'
    first_seen_at_ms  INTEGER NOT NULL,
    last_change_at_ms INTEGER NOT NULL,
    last_rtt_us       INTEGER
);
```

- **Ownership:** only the `store` component reads or writes these tables. The `query` component goes through the same module, with a read-only connection.
- **Retention:** the job deletes from `events` only. It never deletes a `targets` row, so a target keeps its current state and its last transition time.
- **Migrations:** `schema_meta.version` is compared on open. An older database is migrated forward step by step; a newer one is refused (BR-9).

## 6. Runtime view

The main flow: an operator starts the monitor, a target stops answering, and a second process reads the transition while monitoring continues.

```mermaid
sequenceDiagram
    autonumber
    participant Op as Operator shell
    participant Mon as monitor
    participant Pr as probe (ICMP)
    participant St as store (SQLite)
    participant Q as query

    Op->>Mon: monitor --config network-monitor.toml
    Mon->>St: open database, migrate, load current state
    loop every interval_ms, per target
        Mon->>Pr: probe(target, interface)
        Pr->>Pr: open raw socket, bind interface, send echo, wait timeout_ms
        Pr-->>Mon: Up(rtt) | Down | LocalError
        alt verdict differs from current state
            Mon->>St: BEGIN, INSERT event, UPDATE targets, COMMIT
        else verdict unchanged
            Mon->>Mon: discard, no write
        end
    end
    Q->>St: SELECT events WHERE target = ? (read-only, WAL)
    St-->>Q: rows
    Op->>Mon: SIGTERM
    Mon->>St: finish cycle, close connection
```

Takeaway: the write path is one transaction per transition, and the read path never waits for the probing loop.

## 7. Deployment view

- **Local development.** `cargo run -- monitor --config ./network-monitor.toml`, database at `./network-monitor.db`, raw sockets through `sudo` on macOS and Linux or an elevated shell on Windows.
- **Bare host.** The release binary copied anywhere on `PATH`, the configuration in `/etc/network-monitor/config.toml`, the database in `/var/lib/network-monitor/events.db`, supervised by systemd, launchd, or the Windows scheduler. The binary requires `CAP_NET_RAW` or `root`.
- **Container.** A multi-stage image builds the static musl binary and copies it onto a minimal base. The process runs as the container user with `--cap-drop ALL --cap-add NET_RAW`, the configuration mounted read-only, and a volume for the database. No port is exposed.
- **QNAP Container Station.** A project that pulls the image, mounts `/share/Container/network-monitor/config.toml` read-only and `/share/Container/network-monitor/data` as the database volume, with restart policy `unless-stopped`. The volume must be a local share, not a network share.
- **Configuration sources.** The TOML file, then the command line; the container passes the latter through the compose `command` field. There is no environment-variable layer.
- **Upgrades.** Replace the binary or image and restart. The schema is migrated on open; a rollback to an older binary against a newer schema is refused, so the database must be backed up before a downgrade.

## 8. Cross-cutting concepts

### Error handling

Typed errors with `thiserror`. Startup errors (configuration, privileges, unwritable database) are fatal: one stderr line and a non-zero exit code. Probe-local errors are logged and retried on the next cycle without touching the state (BR-4). Database errors during operation are logged at `error` level; a store that cannot write stops the monitor rather than losing transitions silently.

### Logging

Line-based output on stderr, never stdout: stdout carries only query results, so a query can be piped into `jq`. Format: `<RFC 3339 timestamp> <LEVEL> [<target>] <message>`. Transitions are logged at `info`, every probe cycle at `debug`, local probe failures at `warn`, fatal conditions at `error`. The level comes from `--log-level` and defaults to `info`.

### Security

The only privileged operation is opening the raw socket; the container keeps only `NET_RAW` and drops everything else. The database is created with mode `0600`. The configuration may contain addresses and interface names but no secret, and the tool opens no port, so there is no authentication surface. Probes are generated by this process, so no input from the network is parsed as a command.

### Configuration

Three layers merged in a fixed order: built-in defaults, TOML file, command line. The result is validated once and printed by `--show-config`. Keys are stable; a removed key is a breaking change recorded in [ADR.md](./ADR.md). The annotated example lives in [README.md](../README.md#quick-start).

### Data ownership

The `store` component owns the schema, the migration order, and every statement. No other component issues SQL. The `query` component uses read-only access and cannot repair, migrate, or compact a database.

## 9. Architecture decisions

See [ADR.md](./ADR.md). Decisions are recorded there and are not duplicated here.

## 10. Quality requirements

| Attribute | Scenario | Target | Verified by |
|---|---|---|---|
| Storage efficiency | 20 targets monitored for 30 days at a 5 s interval, 50 transitions per day | Database under 1 MB | Integration test with a synthetic event stream, plus a production measurement on QNAP |
| Query latency | A `query events` on a database with 1 000 000 events | Under 50 ms | Benchmark test on a fixture database |
| Concurrency | `query` running while the monitor writes one transition per second | No failed read, writer blocked under 100 ms | Integration test with a concurrent reader |
| Portability | The same source tree built on macOS, Linux, and Windows | Build and test suite green on all three; binary runs with no installed dependency | CI matrix |
| Probe accuracy | A target reachable only through `eth1` while the default route uses `eth0` | Reported reachable when probed on `eth1`, unreachable when probed on `eth0` | Manual acceptance test on a two-interface host |
| Robustness | The monitored interface is removed while the monitor runs | No false `down` event; warning logged; recovery when the interface returns | Integration test with a fake prober returning `LocalError` |
| Testability | The full suite executed in a container without `NET_RAW` | Green, because every test uses the fake prober | CI, unprivileged job |

## 11. Risks and technical debt

### Risks

- **Capability on QNAP firmware.** Some Container Station versions do not forward `NET_RAW` reliably. Mitigation: the acceptance test on the target device is part of the first milestone; privileged mode stays a documented fallback, not the default.
- **Windows interface resolution.** `IP_UNICAST_IF` needs an interface index, while an operator naturally knows an adapter name. Mitigation: this version requires the numeric index and reports a configuration error otherwise; name resolution through `GetAdaptersAddresses` is the follow-up.
- **IPv4 only.** IPv6 needs a separate socket family and link-local scope handling. Mitigation: tracked as an open question; the probe contract already returns a verdict per target, so a second probe type fits without touching the monitor.
- **Disk sleep on NAS.** WAL checkpoints can wake sleeping disks. Mitigation: tune `wal_autocheckpoint`, run retention rarely, and keep the database on an SSD volume where possible.
- **Thread per target.** At several hundred targets the thread count and the scheduling jitter grow. Mitigation: measured at 100 targets in the quality requirements; revisit beyond that.

### Technical debt

- The Windows raw-socket path (`ws2_32`, `IP_UNICAST_IF`) compiles for the target but has not been exercised on a Windows host: it is the first thing to verify, together with name-to-index resolution.
- `query` renders JSON by hand instead of using a serializer. Acceptable for two shapes; revisit when a third appears.
- The monitor is a single writer guarded by a mutex. Concurrent `query` processes are verified in tests, but several `monitor` processes against one database are not supported and not detected.

## 12. Glossary

See [GLOSSARY.md](./GLOSSARY.md).

## Invariants

Properties that must hold after every change. Each invariant names how it is
enforced or verified.

- `INV-1`: A target never has two consecutive events with the same `state`. Enforced by `monitor::state::transition`, verified by its unit tests and by the end-to-end test in `tests/monitor_e2e.rs`.
- `INV-2`: Every configuration key has a command-line override, and the precedence is `command line > file > default`. Enforced by the `config` merge function, verified by a test that walks every key.
- `INV-3`: The monitor, the store, and the query depend on the `Probe` contract only; no test requires raw sockets or privileges. Enforced by module visibility, verified by the suite running in an unprivileged container.
- `INV-4`: The event database is readable while the monitor writes. Enforced by WAL mode and a single writer, verified by an integration test with a concurrent reader.
- `INV-5`: A local probe failure is never persisted as a state change. Enforced by the `Verdict`/`ProbeError` split, verified by a test that feeds `LocalError` for several cycles.
- `INV-6`: Timestamps are stored as UTC epoch milliseconds and converted only for presentation. Enforced by the column types, verified by a store test under a non-UTC `TZ`.
- `INV-7`: The event and the current state of a target are committed in the same transaction. Enforced by the store write function, verified by a test that interrupts the writer after `BEGIN`.
- `INV-8`: The release binary has no dependency beyond the platform C runtime, and SQLite is compiled in. Enforced by the static build, verified by the `ldd` check in CI.
