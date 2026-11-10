# Developer Guide — network-monitor

## Tech stack

- **Language:** Rust, stable toolchain, edition 2021, minimum supported version 1.75.
- **Compile-time crates:** `clap` (derive) for the command line, `serde` and `toml` for the configuration, `rusqlite` with the `bundled` feature for SQLite, `thiserror` for typed errors, `libc` on Unix for the raw socket, and a hand-written `ws2_32` binding on Windows.
- **Test crates:** `assert_cmd`, `predicates`, `tempfile`.
- **Storage:** SQLite 3 compiled into the binary, WAL journal mode.
- **Release artifacts:** one static executable per platform — `x86_64-unknown-linux-musl`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
- **Packaging:** a multi-stage Docker image for Linux, deployed to QNAP through Container Station.

## Prerequisites

- Rust 1.75 or newer through `rustup` (`rustup toolchain install stable --component rustfmt,clippy`).
- The musl target and a musl linker for static Linux builds: `rustup target add x86_64-unknown-linux-musl`, plus `musl-gcc` (Debian/Ubuntu: `apt install musl-tools`).
- Docker 24 or newer, only to build and test the container image.
- Optional: the `sqlite3` command-line client for ad-hoc inspection of the database.
- Raw socket privileges for manual runs: `sudo` on macOS and Linux, an elevated shell on Windows. The test suite needs none.

## Setup

1. Install the toolchain: `rustup toolchain install stable --component rustfmt,clippy`.
2. Install the musl target: `rustup target add x86_64-unknown-linux-musl`, and install `musl-gcc` with your package manager.
3. Fetch the dependencies: `cargo fetch`.
4. Build: `cargo build --release`.
5. Run the suite: `cargo test`.
6. Optional, container: `docker build -t network-monitor:dev .`.
7. Optional, manual run: create `network-monitor.toml` from the example in [README.md](../README.md#quick-start), then `sudo cargo run -- monitor --config ./network-monitor.toml`.

Verify the setup before starting work: the build command below must succeed on a
clean checkout.

## Repository layout

```text
network-monitor/
├── Cargo.toml            crate metadata, dependencies, MSRV
├── Cargo.lock            pinned dependency graph, committed
├── Dockerfile            multi-stage static build for the container image
├── docker/config.toml    default configuration shipped in the image
├── docker-compose.yml    self-contained compose project (host networking)
├── .dockerignore         keeps the build context to the sources
├── network-monitor.toml  example configuration, kept in sync with the defaults
├── src/
│   ├── main.rs           entry point, subcommand dispatch, exit codes
│   ├── lib.rs            library surface used by the integration tests
│   ├── cli.rs            command-line surface, one flag per configuration key
│   ├── config.rs         defaults, TOML loading, precedence, validation
│   ├── logging.rs        line-based diagnostics on stderr
│   ├── reload.rs         configuration file watcher
│   ├── shutdown.rs       stop handle and signal handler
│   ├── time.rs           epoch milliseconds and RFC 3339 rendering
│   ├── monitor/
│   │   ├── mod.rs        interval loop per target, shutdown handling
│   │   └── state.rs      transition decision from the previous state and the verdict
│   ├── probe/
│   │   ├── mod.rs        Probe contract, Verdict, ProbeError
│   │   ├── icmp.rs       raw ICMP echo request and reply matching
│   │   ├── interface.rs  platform interface binding (Linux, macOS, Windows)
│   │   └── fake.rs       deterministic prober for tests
│   ├── store/
│   │   ├── mod.rs        connection, write transaction, retention, read queries
│   │   └── schema.rs     DDL, schema version, migrations
│   ├── query/
│   │   ├── mod.rs        filters and read path
│   │   └── render.rs     table and JSON rendering
│   ├── import.rs         restore a history from the exported CSV (path, URL, stdin)
│   └── web/
│       ├── mod.rs        HTTP loop, routes, window parsing
│       └── page.rs       HTML and SVG generation
├── tests/
│   ├── config_precedence.rs  every key, file and flag, in both directions
│   ├── monitor_e2e.rs        end-to-end loop, transitions only, restart continuity
│   ├── reload.rs             configuration reload, invalid file, database change
│   ├── store.rs              schema, concurrent read, retention, labels
│   ├── cli.rs                exit codes and query output
│   ├── web.rs                page, disconnection table, detail page, no writes
│   ├── import.rs             export round trip, plain CSV, stdin, URL, duplicates
│   └── icmp_privileged.rs    real raw-socket probe, ignored unless root
├── docs/                 product, architecture, decisions, glossary, developer guide
├── .github/              commit and pull request instruction contracts
└── .vscode/              editor settings that register those contracts
```

## Build

```bash
cargo build --release
cargo build --release --target x86_64-unknown-linux-musl
```

## Run

```bash
cargo run -- --help
cargo run -- monitor --config ./network-monitor.toml
cargo run -- monitor --config ./network-monitor.toml --listen 127.0.0.1:8642
cargo run -- serve --database ./network-monitor.db --listen 127.0.0.1:8642
cargo run -- import ./history.csv.gz --database ./network-monitor.db
cargo run -- query state --config ./network-monitor.toml
cargo run -- query events 1.1.1.1 --since 24h --config ./network-monitor.toml
cargo run -- --show-config --config ./network-monitor.toml
```

## Container and QNAP deployment

Build the image and move it to the NAS from the development machine. Two paths,
same artifact.

The image contains the executable and one default configuration at
`/etc/network-monitor/config.toml`. That default probes the container's own loopback
address and writes to `/var/lib/network-monitor/events.db`, so a bare `docker run`
starts and proves that the raw socket and the database work; nothing outside the
container is probed. Mount your own file over that path, or pass `--config` elsewhere,
to monitor real targets. Without a mounted volume the events live in the container
layer and disappear with it.

The compose project below needs none of that: it passes the targets on the command
line and keeps the history in a named volume. The paragraph applies when you choose to
mount a configuration file instead: create it before starting the project, because when
a bind-mount source does not exist Compose creates a directory in its place and the
monitor reports `Is a directory` instead of reading the file.

### Building and running through a Docker context

The simplest path when the NAS exposes its Docker daemon to a local context (`qubo` in
the examples): the local build context is streamed to the NAS, the build runs there,
and the image never travels as a file. With [docker-compose.yml](../docker-compose.yml)
the whole update is `docker -c qubo compose up -d --build`, described in the next
section; the commands here are the same steps written by hand.

```bash
docker context ls                                   # qubo -> tcp://qubo:2376
docker -c qubo build -t network-monitor:dev .
docker -c qubo run -d --name network-monitor \
  --cap-drop ALL --cap-add NET_RAW --network host \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  monitor --target 1.1.1.1,interface=eth1 --target 192.168.1.1 \
  --database /var/lib/network-monitor/events.db
```

`-c <context>` can be prepended to every command that must reach the NAS, including
`logs`, `stop`, `images` and the query container. The context is configured once:

```bash
docker context create qubo --docker "host=tcp://qubo:2376"
docker context create qubo --docker "host=ssh://admin@qubo"   # alternative over SSH
```

The compilation runs on the NAS, so the build costs what that CPU costs. Two choices
keep it short: the `release-dev` profile — `release` with thin LTO and parallel codegen
units — and a BuildKit cache mount on the `target` directory, so a change to a source
file recompiles this crate only. Measured on the QNAP (i5-4590S, four cores): about two
minutes for the first, cold build and about twenty seconds for the next one. Both are
Docker 23 or newer features; the cache lives in the daemon, not in the build context.

The image is tagged `network-monitor:dev` for development. Build and tag the artifact
that leaves the machine with the `release` profile:

```bash
docker -c qubo build --build-arg PROFILE=release -t network-monitor:0.1.0 .
```

**Through a registry**, multi-architecture in one command, built on the laptop:

```bash
docker buildx build --platform linux/amd64,linux/arm64 \
  --build-arg PROFILE=release \
  -t registry.example.com/network-monitor:0.1.0 --push .
```

**Without a registry and without a context**, export and load the image:

```bash
docker build -t network-monitor:dev .
docker save network-monitor:dev | gzip > network-monitor-dev.tar.gz
scp network-monitor-dev.tar.gz admin@nas:/share/Container/
ssh admin@nas 'gzip -dc /share/Container/network-monitor-dev.tar.gz | docker load'
```

### Through compose, from the machine that holds the sources

[docker-compose.yml](../docker-compose.yml) is the deployment: it builds the image,
sets the capabilities, keeps host networking and the page port, and lists the targets
on the command line. Nothing has to exist on the daemon before the first run — no
configuration file, no volume, no image — so one command updates the NAS from the
laptop, over the context:

```bash
docker -c qubo compose up -d --build   # build on the NAS, recreate, start
docker -c qubo compose logs -f
docker -c qubo compose down            # stop and remove; the volume stays
```

The build context is streamed to the daemon and only the changed layers are rebuilt,
so a one-line source change costs seconds. Edit the `--target` entries in the `command`
block to change what is probed. The history lives in the `network-monitor-data` volume
and survives `down` and `up`.

#### Rebuilding a project that is already running

`compose up -d --build` is the whole update: it rebuilds the image from the Dockerfile
and, because the image or the configuration changed, recreates the container — stop,
remove, create, start — then starts it again under `restart: unless-stopped`. A `down`
first is not needed, and the volume keeps the history across the recreation.

```bash
docker -c qubo compose up -d --build                 # rebuild, recreate, start
docker -c qubo compose build                         # build only; the old image keeps running
docker -c qubo compose up -d --force-recreate        # recreate even with an unchanged image
docker -c qubo compose up -d --pull always --build   # rebuild against a newer base image
docker -c qubo compose logs -f                       # confirm the new version started
```

When the rebuild produces an identical image (no source change) `up -d` leaves the
container alone: it already matches the definition. That is the intended behaviour;
`--force-recreate` is the way to restart it anyway. This path is the manual one in the
previous section — `build`, `rm -f`, `run` — with stop, remove and create left to
Compose.

To externalise the configuration later, mount the file read-only over
`/etc/network-monitor/config.toml` and pass `--config` instead of the `--target`
entries; the file must then exist on the daemon host, because a bind mount is resolved
there, not on the machine running compose. Host networking is Linux only, so the file
is meant for the NAS, not for Docker Desktop on macOS.

### Without compose

```bash
# Default configuration, database in the container layer: a proof of life.
docker run --rm --cap-drop ALL --cap-add NET_RAW network-monitor:dev

# Real deployment on the NAS: own configuration, named volume, restart policy.
docker -c qubo run -d --name network-monitor \
  --restart unless-stopped \
  --cap-drop ALL --cap-add NET_RAW \
  --network host \
  -v /share/Container/network-monitor/config.toml:/etc/network-monitor/config.toml:ro \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  monitor --config /etc/network-monitor/config.toml

# Read the history from a second container while the monitor runs. No capability
# and no host networking are needed to query.
docker -c qubo run --rm -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev query events --database /var/lib/network-monitor/events.db

# Effective configuration, and the live log stream.
docker -c qubo run --rm network-monitor:dev --show-config --config /etc/network-monitor/config.toml
docker -c qubo logs -f network-monitor

# Stop: SIGTERM, the monitor finishes the current cycle and exits.
docker -c qubo stop network-monitor
```

Passing a command replaces the image `CMD`, so the default `--config` disappears with
it: when `monitor` is given explicitly, `--config` must be given too. With no
arguments at all, the image uses its own default file. Targets can also come entirely
from the command line, with no mounted file — the command used to validate the QNAP
deployment:

```bash
docker -c qubo run -d --name network-monitor \
  --cap-drop ALL --cap-add NET_RAW --network host \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  monitor --target 1.1.1.1,interface=eth1 --target 192.168.1.1 \
  --database /var/lib/network-monitor/events.db
```

The database file is created by the monitor: on a fresh volume, a query started before
the first write reports that it does not exist yet.

### Inspecting a running container

The runtime image is `scratch`: it is not the `rust:1-alpine` build stage, which is
discarded. There is no shell, no `ip`, no `sqlite3` — only `/network-monitor`. An
`exec` of `/bin/sh` or `/bin/bash` fails with `no such file or directory` (exit 126 or
127). That is deliberate: the smallest possible surface and a 4.5 MB image.

`exec` can still run the binary itself, without mounting a second container:

```bash
docker -c qubo exec network-monitor \
  /network-monitor query state --database /var/lib/network-monitor/events.db
```

To take the database away:

```bash
# While it runs, copy the whole directory: recent writes live in the -wal file.
docker -c qubo cp network-monitor:/var/lib/network-monitor ./data

# After a clean stop the WAL is checkpointed, so the single file is complete.
docker -c qubo stop network-monitor
docker -c qubo cp network-monitor:/var/lib/network-monitor/events.db ./events.db
sqlite3 ./events.db 'select target, state, occurred_at_ms from events;'
```

To inspect the network the container actually sees, join its namespace with a tool
image; the interfaces are the same, including when host networking is in use:

```bash
docker -c qubo run --rm --network container:network-monitor alpine:3.20 ip -o link
```

Use `nicolaka/netshoot` instead of `alpine` when you need `ping`, `tcpdump`, or `ss`.
The host interfaces of the NAS are visible from its own shell over SSH, not from the
container when a bridge network is in use.

### Web page in the container

Host networking is already in use for the interface binding, so the monitor itself
serves the page on the NAS address and no port is published:

```bash
docker -c qubo run -d --name network-monitor \
  --restart unless-stopped \
  --network host \
  --cap-drop ALL --cap-add NET_RAW \
  -v /share/Container/network-monitor/config.toml:/etc/network-monitor/config.toml:ro \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  monitor --config /etc/network-monitor/config.toml \
  --database /var/lib/network-monitor/events.db --listen 0.0.0.0:8642
```

The page is then at `http://<nas>:8642/`; select a lane to open that target's 30-day
page. Keep the port on the LAN — there is no authentication — or bind `127.0.0.1:8642`
and reach it through an SSH tunnel.

`serve` remains available as a viewer that needs neither `NET_RAW` nor host
networking, for example on a laptop reading a copy of the database or against the same
volume:

```bash
docker -c qubo run -d --name network-monitor-web \
  --restart unless-stopped \
  -p 8642:8642 \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  serve --listen 0.0.0.0:8642 --database /var/lib/network-monitor/events.db
```

### Restoring the history after a redeploy

The download comes back as a history: point `import` at the file, at the running page,
or pipe it, and the disconnections are written into the target database.

```bash
# from the archive the page offered
network-monitor import ./disconnections-20261109.csv.gz --database /var/lib/network-monitor/events.db

# straight from the page
network-monitor import http://nas:8642/export/disconnections.csv.gz --database ./events.db

# from a pipe, with the format detected
gunzip -c disconnections.csv.gz | network-monitor import - --database ./events.db
```

The format is detected from the first two bytes, so a plain CSV works as it is. The rows need no order: the chain is rebuilt from the timestamps at the end of the import, and an event that repeats the state before it is dropped and counted. Running
the same import twice is harmless: an event already in the database is skipped and
counted, and the command reports `imported 2870 events for 2 target(s) … (0 rows
skipped, gzip format)`. Labels and interfaces are not part of the CSV; the next probe of
the monitor writes them back from the configuration.

Only `http://` URLs are accepted: the binary carries no TLS stack. Download an `https`
archive with `curl` and import the file.

A merge adds to what is there; `--replace-window` replaces what the archive covers. The
window is the earliest start and the latest end in the file, and everything of every
target inside it is deleted before the archive is written; the history before and after
stays.

```bash
# re-apply a corrected archive over the same days
network-monitor import corrected.csv.gz --replace-window --database ./events.db
```

The archive is read twice in that mode, so stdin and an URL are copied once into
`.import-<pid>.tmp` next to the database (the second pass must see the same bytes: an
exported URL is regenerated on every request); the copy is removed when the command
ends. An empty archive has no window, so the command refuses and deletes nothing.

Run the import with the monitor **stopped**: stop the service, import, start it again.
The monitor decides a transition from the state it loaded at startup, so a history
written under a running monitor leaves it with a stale view, and an imported row newer
than the live one can flip the current state on the page until the next probe. The page
itself (`serve`, or `monitor --listen`) can stay up: readers never block the writer, and
the next 30 s refresh shows the imported data.

Two writers do work — WAL plus a 5 s `busy_timeout` — and a 600 000-event import next to
a writer committing every 20 ms produced no lock error and a worst commit of 9 ms on a
local disk. It is still not the intended mode: on slow storage a batch that takes longer
than the timeout makes the monitor's write fail, and a store that cannot write stops the
monitor rather than losing transitions.

With `retention_days` set, imported events older than the cut-off are pruned on the next
retention pass; import a history you want to keep with retention disabled, or accept the
trim.

### Downloading the history

The page links it, and the same archive can be fetched from a script:

```bash
curl -O http://127.0.0.1:8642/export/disconnections.csv.gz
gunzip -c disconnections-*.csv.gz | head
```

One row per event, with the columns the import reads:

```csv
target,label,interface,state,occurred_at,rtt_us
192.168.1.1,gateway,eth1,up,2026-11-09T03:42:01.236Z,12000
192.168.1.1,gateway,eth1,down,2026-11-09T04:42:01.236Z,
192.168.1.1,gateway,eth1,up,2026-11-09T05:12:01.236Z,15000
```

The response is chunked and the CSV is deflated while it is sent, so the size of the
history does not change the memory the server needs. A failure in the middle of
the export is logged and truncates the download; retry it.

### Choosing the physical interface on a multi-NIC NAS

Interface names inside a container are not the names of the NAS interfaces. A default
bridge gives the container its own `eth0` on a virtual network, so an `interface`
value that names a NAS NIC cannot be resolved; the monitor refuses to start and names
the missing interface (BR-5).

To probe from a physical NIC, run the container with host networking
(`network_mode: host`, as in the compose file). The interfaces visible inside are
then the NAS interfaces, and `interface = "eth1"` selects the second physical port
exactly as on a bare host. List the names on the NAS with `ip -br link` or
`ifconfig -l`.

When network isolation is required instead, create one macvlan network per physical
interface and one container per interface, leaving `interface` unset: each container's
`eth0` is the parent NIC. Two NICs then mean two containers and two databases.

## Test

```bash
cargo test
cargo test --test monitor_e2e
cargo test --test reload
cargo test --test store
cargo test --test web
```

<!-- skill:block:devguide-test-rules section="## Test" -->
Fixed rules that apply to every change:

- Every change ships with the tests of the part it changes, in the same commit. A
  change without tests is not done.
- New behaviour needs a test that fails without the change and passes with it; a
  defect fix needs a regression test that reproduces the defect.
- Test each module independently: substitute the other modules at their contract
  boundary instead of starting them. A test that needs the whole system to run is a
  signal that the module is not decoupled enough.
- Never delete or weaken a test to make the build green; fix the cause.
<!-- skill:end:devguide-test-rules -->

## Lint and format

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## Extending the project

### Adding a configuration option

1. Add the field to `Config` with its default expressed in `Default`.
2. Add the matching `clap` argument in `cli.rs`; there is no option without a flag (INV-2).
3. Apply the merge in `config.rs`, which is the only place that knows the precedence order.
4. Decide whether the value applies to a running monitor: if it does, apply it in `Supervisor::apply` and cover it in `tests/reload.rs`; if it needs a restart, warn there instead.
5. Document the key in `network-monitor.toml`, in the README example, and in [PRODUCT.md](./PRODUCT.md) when it changes behaviour.
6. Test three cases: value from the file only, value from the flag only, flag overriding the file.

### Adding a probe type

1. Define the type in `probe/`, implement the `Probe` contract, and return the existing `Verdict` variants; do not add a variant that means "local failure".
2. Keep the platform-specific socket work in `probe/interface.rs` so the rest of the code stays portable.
3. Select it in the composition step, from configuration, without touching `monitor`.
4. Add a fake-driven test of the decision logic and one manual acceptance test with real privileges.

### Changing the schema

1. Append a migration step in `store/schema.rs` and bump `schema_meta.version`; never edit an applied migration.
2. Extend the read path in `store` and the rendering in `query` in the same change.
3. Update the data model in [ARCHITECTURE.md](./ARCHITECTURE.md) and add an ADR when the change is hard to reverse.
4. Test the migration against a fixture database of the previous version.

### Adding a query output

1. Add the filter to `QueryFilter` and the rendering to `query/render.rs`.
2. Keep stdout for data and stderr for diagnostics, so the output stays pipeable.
3. Test the exact output of the new form, including the empty result.

### Adding a test

1. Put the test next to the module for unit-level checks and in `tests/` for cross-module contracts.
2. Never open a raw socket in a test: use `probe::fake`, and a temporary database from `tempfile`.
3. Keep the test deterministic: inject the clock or assert on monotonic ordering instead of wall time.

<!-- skill:block:devguide-design-rules section="## Extending the project" -->
### Design rules

Fixed rules for adding anything to this project:

- Place the new code in the module that owns the responsibility. If no module owns
  it, add a module with one responsibility — do not grow an existing module for an
  unrelated concern.
- Depend on the other modules' public contracts only; never reach into their
  internals, and never introduce a dependency cycle.
- Start with the simplest implementation that satisfies the requirement (KISS). No
  speculative abstraction, extension point, or configuration for a need that is not
  there yet.
- Add the tests of the new code in the same change, at the module boundary, with the
  other modules replaced by fakes.
<!-- skill:end:devguide-design-rules -->

<!-- skill:block:devguide-git-standards -->
## Git Standards

These rules are mandatory for every contributor, human or AI. They define how work
enters this repository.

Automated agents never commit or push: they prepare the change and the commit
message, and only the user approves and saves it in Git (see
[AGENTS.md](../AGENTS.md#git-user-approved-changes-only)).

### Commit messages

Every commit message follows Conventional Commits and stays concise: one logical
change per commit, an imperative subject of at most 72 characters, no trailing
period, a body only when the reason is not obvious.

The complete contract — format, allowed types and scopes, subject and body rules,
footers, breaking changes, and accepted and rejected examples — lives in
[.github/copilot-commit-message-instructions.md](../.github/copilot-commit-message-instructions.md).
That file is the single source of truth for message format: it is what an assistant
reads when it drafts a commit message, and it is registered for that purpose in
[.vscode/settings.json](../.vscode/settings.json). Read it before writing a message;
do not restate it here.

### Branches and integration

- `main` is always releasable; never commit directly to it.
- Branch from `main` with `feat/<slug>`, `fix/<slug>`, `docs/<slug>`, or
  `chore/<slug>`; keep branches short-lived and rebase on `main` before review.
- One pull request per concern. Squash on merge so `main` keeps one commit per
  change, with the pull request title in Conventional Commits form.
- Long-running work uses feature flags or stacked pull requests, not long-lived
  branches.

### Pull requests

The title follows the commit message format, because a squash merge turns it into the
commit subject. The description states the problem, the approach, the verification, and
the deliberate exclusions. The contract is in
[.github/copilot-pull-request-instructions.md](../.github/copilot-pull-request-instructions.md).

### Language

Every artefact is written in English: commit messages, branch names, code,
identifiers, comments, tests, documentation, issue text, and user-facing strings.
<!-- skill:end:devguide-git-standards -->
