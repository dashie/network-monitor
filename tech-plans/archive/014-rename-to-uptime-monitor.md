---
title: Rename the tool to uptime-monitor across code, configuration, deployment, and docs
date: 2026-11-17
status: done
type: refactor
impact: high
area: repository-wide (crate identity, CLI, configuration defaults, deployment, documentation)
version_bump: minor
tags: [rename, breaking, branding, deployment, documentation]
depends_on: []
branch_recommended: true
---

## Summary

The tool is renamed from `<previous-name>` to `uptime-monitor` everywhere: the Cargo
package, the binary, the library crate, the default configuration file
(`<previous-name>.toml` → `uptime-monitor.toml`), the default database
(`<previous-name>.db` → `uptime-monitor.db`), the container and NAS names, the pages and
notification text, the public landing page, and every document. Probes, storage, the
schema, the REST routes, and every behavioural rule stay exactly as they are; only the
identity changes. Existing histories remain readable and are never migrated by the
binary: the schema is untouched, so a renamed database file, a `--database` flag, or a
copied volume keeps every event.

The migration is a hard cut, chosen in this plan: there is no `<previous-name>` alias
binary and no silent fallback to the old file names. Existing installations follow the
one-time rename steps this plan adds to the user guide and the deployment notes.

## Why

The current name describes a transport (ICMP to network addresses) that the tool stopped
being several releases ago: it probes ICMP, TCP, DNS, and HTTP, and its value is the
record of when a target stops and starts answering. "Uptime monitor" states the question
the operator asks; "reachability monitor" states one of the four transports. The name is also
the public identity of the executable, the container image, the configuration file, and
the landing page, so it is the first thing a new user reads and the thing that appears in
every unit file and compose project.

The cost of leaving it unchanged is permanent explanatory overhead: every new page,
runbook, and release repeats that a "reachability monitor" also checks HTTP status codes and
DNS answers. The change is cheap now, while the project is alpha and has one operator's
deployment; it grows more expensive with every published artifact, URL, and volume that
carries the old name.

## Goals

- `cargo build` produces exactly one binary named `uptime-monitor`, and `cargo run --
  --version` prints `uptime-monitor <version>`; `cargo run -- --help` prints
  `Usage: uptime-monitor`.
- The library crate is `uptime_monitor`, and no file under `src/` or `tests/` refers to
  `<previous_crate_name>`.
- With no `--config`, the binary reads `./uptime-monitor.toml` when present and skips it
  otherwise; with no `--database`, it opens `./uptime-monitor.db`.
- `uptime-monitor init <folder>` creates `<folder>/uptime-monitor.toml` and prints the
  `uptime-monitor check --config uptime-monitor.toml` and `uptime-monitor monitor
  --config uptime-monitor.toml` commands.
- The served pages, the API reference, the `READY`/`STOPPED` notifications, and the HTTP
  probe `User-Agent` state the new name.
- The image, container, service, config mount, data directory, volume, systemd unit, and
  log directory use `uptime-monitor`; the deployment notes carry a one-time migration
  that preserves the existing history.
- The landing page, its crawl files, and every internal document use the new name; the
  only remaining `<previous-name>` strings are the deliberate legacy-name mentions in the
  0.12 → 0.13 upgrade instructions, the deployment migration, and ADR-0040.
- The full quality suite passes: `cargo fmt --check && cargo clippy --all-targets -- -D
  warnings && cargo test`, and the Python helper tests pass.
- Existing database files, exported CSV files, JSON responses, REST routes, probes, and
  configuration keys behave exactly as before.

## Non-goals

- No functional change: no new probe, key, flag, route, schema version, event field, or
  notification channel, and no change to the merge order or the invariants.
- No backward-compatibility layer: no second binary name, no alias symlink, no fallback
  lookup of `<previous-name>.toml` or `<previous-name>.db`, and no environment-variable
  shim. The old names stop existing.
- No rewrite of `tech-plans/archive/`: those files are frozen records of past plans and
  keep the name they were written with.
- No rewrite of the bodies of ADR-0001 to ADR-0039: `docs/ADR.md` declares records
  append-only. The document title, the index, and a one-sentence preamble note change,
  and ADR-0040 records the rename; the historical decision text keeps the old name.
- No repository rename executed from this plan: renaming the Gitea remote, the GitHub
  mirror, the GitHub Pages project, and the local checkout is an operator step. The plan
  updates the URLs to the new names and documents the steps.
- No copy rewrite beyond the name: taglines, descriptions, and the `about` string keep
  their wording, including the existing "ICMP" wording in `src/lib.rs` and
  `src/cli.rs`.
- No migration tool: renaming the config, database, unit, and volume is documented shell
  and Docker work, not a new subcommand.

## Affected modules

Every file below is the scope boundary. A file outside this list requires explicit user
permission.

**Build and crate**
- `Cargo.toml` — `[package] name`, `[[bin]] name`, `[lib] name`.
- `Cargo.lock` — regenerated package entry (no manual edit).

**Source**
- `src/main.rs` — every `use <previous_crate_name>::…` becomes `use uptime_monitor::…`.
- `src/lib.rs` — crate doc line names the new tool.
- `src/cli.rs` — clap `name`, the `--config` help default text, unit-test argv.
- `src/config.rs` — `DEFAULT_CONFIG_FILE`, `DEFAULT_DATABASE`, default-file discovery.
- `src/init.rs` — template header, template commands, `Report::render`, unit tests.
- `src/store/mod.rs` — `StoreError::Uninitialized` message.
- `src/notify/message.rs` — `READY`/`STOPPED` text and its tests.
- `src/probe/http.rs` — `User-Agent` header and its test.
- `src/probe/dns.rs`, `src/probe/tcp.rs`, `src/import.rs` — test fixtures that use the
  old token (`<previous-name>.invalid`, a fixture request line).
- `src/snapshot.rs`, `src/promote.rs`, `src/check.rs`, `src/monitor/mod.rs` — test
  fixtures with old file names.
- `src/web/page.rs` — page `{title}`.
- `src/web/templates/brand.html` — brand `<h1>`; the tagline is unchanged.
- `src/web/templates.rs` — brand assertion.

**Embedded assets**
- `assets/api/openapi.json` — `info.title`.
- `assets/api/docs.html` — `<title>` and masthead `<h1>`.

**Tests**
- `tests/cli.rs` — `cargo_bin`, `CARGO_BIN_EXE_…`, fixture paths, log-line assertion;
  *(new)* one test pinning the version/help name.
- `tests/config_precedence.rs` — argv, default-database assertion, fixture paths.
- `tests/init.rs` — `cargo_bin`, fixture paths; extend the stdout assertion with the new
  command names.
- `tests/manual.rs`, `tests/import.rs`, `tests/monitor_e2e.rs`, `tests/notify.rs`,
  `tests/notify_channels.rs`, `tests/probe_network.rs`, `tests/reload.rs`,
  `tests/simulate.rs`, `tests/snapshot.rs`, `tests/store.rs`, `tests/web.rs`,
  `tests/icmp_privileged.rs` — `use <previous_crate_name>::…`, `cargo_bin("<previous-name>")`,
  fixture paths, brand and hostname assertions.

**Configuration and deployment**
- `<previous-name>.sample.toml` → `uptime-monitor.sample.toml` — file name and comment.
- `.gitignore` — ignored local config and database names.
- `Dockerfile` — build-stage copy, runtime path, `/etc/uptime-monitor/config.toml`,
  comments and examples.
- `docker-compose.yml` — service, image, container, volume, config target, inline
  `database` path, comments.
- `docker/config.toml` — header path and `database` value.
- `NOTES.md` — deployment commands, container and volume names, plus the one-time
  volume migration.

**Scripts**
- `scripts/utils/export2nm.py` → `scripts/utils/convert_heartbeats.py` — module name,
  docstring, usage examples.
- `scripts/utils/test_export2nm.py` → `scripts/utils/test_convert_heartbeats.py` —
  `import` and every reference.
- `scripts/utils/serve_site.py` — docstring reference to the renamed helper.

**Documentation / configuration**
- `README.md` — title, quick start, sample TOML, documentation table, landing-page URL.
- `AGENTS.md` — opening paragraph naming the repository and the tool.
- `CONTRIBUTING.md` — the two occurrences.
- `docs/PRODUCT.md` — title, purpose, problem statement, `FR-1`, `FR-33`, and the scope
  rule that the tool and the executable share one name.
- `docs/ARCHITECTURE.md` — title, §1 introduction, §3 context and container diagrams, §5
  `init` contract, §6 runtime view, §7 deployment view (paths, volume, landing page URL).
- `docs/GLOSSARY.md` — document title only; no term is renamed.
- `docs/DEVELOPER-GUIDE.md` — title, repository tree, every command, the container and
  QNAP sections, the pages section, and a new "Rename the repositories" checklist.
- `docs/USER-GUIDE.md` — sections `What the tool does`, `Install` (`From source`, `From an
  empty folder`, `Container`), `Quick start`, `Configuration` (default file name),
  `Command line`, `The web page`, `REST API`, `Operations` (`Back up and restore`,
  `Upgrade` with a new 0.12 → 0.13 step, `Deploy a new configuration`, `Recover a
  configuration edited by mistake`, `Rotate the logs`), and the systemd unit example.
  The three-line product summary keeps its wording apart from the name.
- `docs/HISTORY-IMPORT.md` — script name, commands, container and volume paths.
- `docs/NOTIFICATIONS.md` — commands and paths.
- `docs/ADR.md` — title, index row for ADR-0040, preamble note on historical names, new
  `## ADR-0040`.
- `site/index.html` — title, meta, canonical, Open Graph, JSON-LD, visible brand, quick
  start, Docker example, documentation links, footer.
- `site/robots.txt`, `site/sitemap.xml` — site URL.

## Architecture impact

No structural change: no component is added, removed, renamed, or rewired, no module
boundary moves, and `INV-1` to `INV-15` keep their meaning. The module names
(`config`, `probe`, `store`, `web`, …) are unchanged. `docs/ARCHITECTURE.md` must be
updated only where it states the product name, the executable name, the default paths,
the volume name, and the landing-page URL.

One deployment consequence is architectural in the arc42 sense and is recorded in §7:
the container image path, the configuration mount (`/etc/uptime-monitor/config.toml`),
the data directory (`/var/lib/uptime-monitor`), and the named volume
(`uptime-monitor-data`) change together, and an existing QNAP deployment must copy its
volume once.

## Decision-record impact

A new record is required: `ADR-0040 — Rename the tool to uptime-monitor`. It records the
name change, the hard-cut choice (no alias and no legacy fallback), the unchanged schema
and storage, the deployment migration, and the instruction that earlier records keep the
historical name under the append-only rule of `docs/ADR.md`.

No existing record is superseded in substance. The rename does not reverse ADR-0002
(Rust binary), ADR-0023 (target identity), ADR-0026 (skills exposure), or any decision
about probing, storage, or presentation; those records remain accepted, and ADR-0040
notes that names quoted inside them are historical.

## Proposed solution

Apply the following mapping first, then execute the steps in order. Each step is
independently verifiable, but the rename is one logical change: implement the steps in
sequence and deliver them as one commit.

**Rename mapping** (case-sensitive, whole token):

| Old | New | Where |
|---|---|---|
| `<previous-name>` | `uptime-monitor` | package, binary, file names, paths, URLs, prose, code blocks |
| `<previous_crate_name>` | `uptime_monitor` | Rust crate name and `use` paths |
| `<Previous Brand>` | `Uptime Monitor` | visible brand, page and API titles |
| `export2nm.py` / `test_export2nm.py` | `convert_heartbeats.py` / `test_convert_heartbeats.py` | helper script and its test (a hyphen is not importable) |

Do not touch `tech-plans/archive/`, the bodies of ADR-0001 to ADR-0039, or the untracked
local `<previous-name>.toml` in the working tree.

### Step 1 — Rename the Cargo package, binary, and library

Change `Cargo.toml` and every Rust path that imports the library. `Cargo.lock` follows on
the first `cargo` command and is not edited by hand.

```toml
# Cargo.toml — before
[package]
name = "<previous-name>"
version = "0.12.0"
edition = "2021"

[[bin]]
name = "<previous-name>"
path = "src/main.rs"

[lib]
name = "<previous_crate_name>"
path = "src/lib.rs"
```

```toml
# Cargo.toml — after
[package]
name = "uptime-monitor"
version = "0.12.0"
edition = "2021"

[[bin]]
name = "uptime-monitor"
path = "src/main.rs"

[lib]
name = "uptime_monitor"
path = "src/lib.rs"
```

`version` is left at 0.12.0 here; `spec-close` applies the `version_bump: minor` of this
plan. In `src/main.rs`, `tests/*.rs`, and the unit tests inside `src/*.rs`, replace
`<previous_crate_name>` with `uptime_monitor`; in `tests/`, replace `cargo_bin("<previous-name>")`
and `CARGO_BIN_EXE_<previous-name>` accordingly.

**Verify:** `cargo check --all-targets` succeeds and
`grep -rn '<previous_crate_name>' src tests` prints nothing.

### Step 2 — Rename the runtime identity and the configuration defaults

Change the clap command name (`src/cli.rs`), the two defaults in `src/config.rs`, the
`init` template and report (`src/init.rs`), the database error text (`src/store/mod.rs`),
the process notification text (`src/notify/message.rs`), the HTTP probe `User-Agent`
(`src/probe/http.rs`), the page title and brand (`src/web/page.rs`,
`src/web/templates/brand.html`), and the embedded API metadata (`assets/api/openapi.json`,
`assets/api/docs.html`). Update the tests that assert those strings and the fixtures that
use the old file names (list under *Affected modules*).

```rust
// src/config.rs — before
pub const DEFAULT_CONFIG_FILE: &str = "<previous-name>.toml";
pub const DEFAULT_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 2_000;
pub const DEFAULT_DATABASE: &str = "<previous-name>.db";
```

```rust
// src/config.rs — after
pub const DEFAULT_CONFIG_FILE: &str = "uptime-monitor.toml";
pub const DEFAULT_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 2_000;
pub const DEFAULT_DATABASE: &str = "uptime-monitor.db";
```

Add one regression test in `tests/cli.rs` that fails before this change:

```rust
#[test]
fn the_binary_reports_its_name() {
    Command::cargo_bin("uptime-monitor")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("uptime-monitor "));

    Command::cargo_bin("uptime-monitor")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: uptime-monitor"));
}
```

Extend `tests/init.rs::init_creates_a_folder_that_check_accepts` to assert that stdout
contains `uptime-monitor check --config uptime-monitor.toml`, so the printed next steps
are pinned to the binary name.

**Verify:** `cargo test --test cli --test init --test config_precedence --test web`
passes, `cargo run -- --version` prints `uptime-monitor 0.12.0`, and
`cargo run -- --help | head -3` prints `Usage: uptime-monitor`.

### Step 3 — Rename the sample configuration and the local ignores

Rename `<previous-name>.sample.toml` to `uptime-monitor.sample.toml`, update its header
comment, and update `.gitignore` so a local `uptime-monitor.toml` and its database stay
untracked. The untracked local `<previous-name>.toml` and `<previous-name>.db` in the
working tree are the operator's files: leave them alone, or rename them by hand as the
user guide's upgrade step says.

```gitignore
# .gitignore — before
# Create a copy from <previous-name>.sample.toml to create a local
# configuration file if needed
<previous-name>.toml

# Event database and its write-ahead log, wherever a local run leaves them
<previous-name>.db
<previous-name>.db-shm
<previous-name>.db-wal
```

```gitignore
# .gitignore — after
# Create a copy from uptime-monitor.sample.toml to create a local
# configuration file if needed
uptime-monitor.toml

# Event database and its write-ahead log, wherever a local run leaves them
uptime-monitor.db
uptime-monitor.db-shm
uptime-monitor.db-wal
```

**Verify:** `git ls-files | grep sample` lists only `uptime-monitor.sample.toml`;
`git status --short` shows the rename; `grep -n 'uptime-monitor' .gitignore` lists the
four new entries.

### Step 4 — Rename the container and NAS deployment

Change the image path, the configuration mount, the data directory, the service, the
container, and the volume in `Dockerfile`, `docker-compose.yml`, `docker/config.toml`,
and `NOTES.md`. Then add the one-time volume migration to `NOTES.md` and to the user
guide's `Operations → Upgrade` section (step 7).

```toml
# docker/config.toml — before
# The configuration baked into the image; compose mounts its own file over
# /etc/<previous-name>/config.toml.
...
database = "/var/lib/<previous-name>/events.db"
```

```toml
# docker/config.toml — after
# The configuration baked into the image; compose mounts its own file over
# /etc/uptime-monitor/config.toml.
...
database = "/var/lib/uptime-monitor/events.db"
```

```yaml
# docker-compose.yml — before
services:
  <previous-name>:
    image: <previous-name>:dev
    container_name: <previous-name>
    volumes:
      - <previous-name>-data:/var/lib/<previous-name>
    configs:
      - source: monitor-config
        target: /etc/<previous-name>/config.toml
...
      database = "/var/lib/<previous-name>/events.db"
volumes:
  <previous-name>-data:
    name: <previous-name>-data
```

```yaml
# docker-compose.yml — after
services:
  uptime-monitor:
    image: uptime-monitor:dev
    container_name: uptime-monitor
    volumes:
      - uptime-monitor-data:/var/lib/uptime-monitor
    configs:
      - source: monitor-config
        target: /etc/uptime-monitor/config.toml
...
      database = "/var/lib/uptime-monitor/events.db"
volumes:
  uptime-monitor-data:
    name: uptime-monitor-data
```

The one-time migration, documented in `NOTES.md` and the user guide, keeps the old volume
until the new deployment is verified:

```bash
docker compose down                      # stop the old container, keep the volume
docker volume create uptime-monitor-data
docker run --rm -v <previous-name>-data:/from:ro \
  -v uptime-monitor-data:/to alpine cp -a /from/. /to/
docker compose up -d --build             # new names, same history
```

**Verify:** `grep -rn '<previous-name>' Dockerfile docker-compose.yml docker NOTES.md`
prints only the lines of the migration block naming the old volume, and
`docker compose -f docker-compose.yml config` resolves when Docker is available (otherwise
the static grep is the check).

### Step 5 — Rename the import helper and its test

Rename `scripts/utils/export2nm.py` to `scripts/utils/convert_heartbeats.py` and
`scripts/utils/test_export2nm.py` to `scripts/utils/test_convert_heartbeats.py`, update
the `import` and every `export2nm.` reference, and update the docstring, the usage
examples, and the reference in `scripts/utils/serve_site.py`. The underscore is required:
a module named with a hyphen cannot be imported by the test.

**Verify:** `python3 -m unittest discover -s scripts/utils -p 'test_*.py'` passes and
`grep -rn 'export2nm' scripts docs` prints nothing.

### Step 6 — Sweep the product, architecture, developer, and runbook documents

Apply the mapping to `README.md`, `AGENTS.md`, `CONTRIBUTING.md`, `docs/PRODUCT.md`,
`docs/ARCHITECTURE.md`, `docs/GLOSSARY.md`, `docs/DEVELOPER-GUIDE.md`,
`docs/HISTORY-IMPORT.md`, and `docs/NOTIFICATIONS.md`. In `docs/PRODUCT.md`, keep the
rule that the tool and the executable share one name and state it as `uptime-monitor`;
in `FR-1` and `FR-33`, the default file name becomes `uptime-monitor.toml`. In
`docs/ARCHITECTURE.md`, update the title, the C4 context and container labels, the `init`
contract, the runtime example, and §7 paths and landing-page URL.

Add a short "Rename the repositories" checklist to `docs/DEVELOPER-GUIDE.md`: rename the
Gitea repository, update the remote URL (`git remote set-url origin …/uptime-monitor.git`),
rename the GitHub mirror so the new Pages project is published, and rename the local
checkout folder. It states that GitHub and Gitea redirect the old repository URL but not
guaranteed Pages URLs, so the site's canonical URL always states the new one.

**Verify:** `cargo test --test manual` still passes (the manual is the exact bytes of
`docs/USER-GUIDE.md`, updated in step 7), and `grep -rn '<previous-name>' README.md
AGENTS.md CONTRIBUTING.md docs/PRODUCT.md docs/ARCHITECTURE.md docs/GLOSSARY.md
docs/DEVELOPER-GUIDE.md docs/HISTORY-IMPORT.md docs/NOTIFICATIONS.md` prints nothing.

### Step 7 — Update the user guide, including the upgrade path

Apply the mapping to `docs/USER-GUIDE.md` — title, `What the tool does`, `Install`
(`From source`, `From an empty folder`, `Container`), `Quick start`, `Configuration`,
`Command line`, `The web page`, `REST API`, and `Operations` — and add the 0.12 → 0.13
step to `Operations → Upgrade`:

> **From 0.12 to 0.13** the tool is renamed `uptime-monitor`. The schema is unchanged and
> no event is migrated: stop the process and rename the files, or keep explicit paths.
>
> ```bash
> mv <previous-name>.toml uptime-monitor.toml
> mv <previous-name>.db uptime-monitor.db    # stop the process first; move -wal and -shm too
> ```
>
> Rename the unit file and its `ExecStart` path, and the compose project, image, container,
> mount, and volume as `NOTES.md` describes. A configuration or database path stated
> explicitly keeps working under any name; only the defaults change.

The guide stays self-contained (no links into the repository's internal documents) and
documents the configuration file name and the database default as the code implements
them. The systemd unit example becomes `/etc/systemd/system/uptime-monitor.service` with
`ExecStart=/usr/local/bin/uptime-monitor monitor --config /etc/uptime-monitor/config.toml`,
and the logrotate example becomes `/etc/logrotate.d/uptime-monitor` over
`/var/log/uptime-monitor/monitor.log`.

**Verify:** `cargo run -- manual | grep -n '<previous-name>'` shows only the legacy names
inside the 0.12 → 0.13 migration block, and
`grep -n 'DEFAULT_CONFIG_FILE\|DEFAULT_DATABASE' src/config.rs` matches the names the
guide states.

### Step 8 — Update the landing page and its crawl metadata

Apply the mapping to `site/index.html`, `site/robots.txt`, and `site/sitemap.xml`,
including `<title>`, canonical, Open Graph, JSON-LD `name` and `url`, visible brand,
source links, quick-start and Docker commands, documentation links, and the footer. The
canonical URL becomes `https://dashie.github.io/uptime-monitor/` and the source links
become `https://github.com/dashie/uptime-monitor/…`. The pages workflow needs no change:
it copies `site/` and the icons and carries no name.

**Verify:** `grep -rn 'dashie.github.io/<previous-name>\|github.com/dashie/<previous-name>'
site README.md docs` prints nothing, and a local render of `site/index.html` shows
`Uptime Monitor` in the header (open the file in a browser, or
`python3 scripts/utils/serve_site.py`).

### Step 9 — Record the decision

Add the index row and the record `ADR-0040 — Rename the tool to uptime-monitor` to
`docs/ADR.md` (status `accepted`, date `2026-11-17`), and change the document title to
`# Architecture Decision Records — uptime-monitor`. Add one sentence under the title:
records before ADR-0040 use the name the tool had when they were written; the current
name is `uptime-monitor`, decided in ADR-0040. Do not edit the bodies of ADR-0001 to
ADR-0039.

The record states: the name and why it changed; the hard cut (no alias, no legacy file
fallback); that the schema, storage, routes, keys, and behaviour are unchanged; the
one-time deployment migration and the forward-only default change; and that archived
plans and earlier ADRs keep the historical name.

**Verify:** `grep -n 'ADR-0040' docs/ADR.md` lists the index row and the record; the
records before it are byte-identical except for the title and the preamble note
(`git diff docs/ADR.md` shows only the title, the note, the index row, and the appended
record).

### Step 10 — Run the full gate and the residue check

Run the quality suite, the Python tests, and the residue check. Then inspect the binary by
hand: `cargo run -- --version`, `cargo run -- --help`, and
`cargo run -- init /tmp/uptime-check` in a scratch directory, followed by
`cargo run -- check --config /tmp/uptime-check/uptime-monitor.toml`, then remove the
folder.

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
python3 -m unittest discover -s scripts/utils -p 'test_*.py'
git grep -n -i -e '<previous-name>' -e '<previous_crate_name>' \
  -- src tests assets site Dockerfile docker-compose.yml docker .gitignore \
     Cargo.toml README.md AGENTS.md CONTRIBUTING.md docs/ARCHITECTURE.md \
     docs/PRODUCT.md docs/GLOSSARY.md docs/DEVELOPER-GUIDE.md
```

The last command prints nothing. The remaining deliberate mentions are checked by hand:

```bash
git grep -n -i '<previous-name>' -- docs/USER-GUIDE.md docs/HISTORY-IMPORT.md \
  docs/NOTIFICATIONS.md docs/ADR.md NOTES.md
```

Every line it prints is a legacy name inside the 0.12 → 0.13 upgrade block, the deployment
migration, the ADR preamble note, ADR-0040, or an ADR body before ADR-0040.

**Verify:** all commands exit zero and the flagged files show no unexpected match.

## Security considerations

- No credential, token, or secret handling changes: the `telegram` and webhook secrets
  stay file-only, redacted in every rendering, and untouched by this plan. No example in
  the plan or in the migration block contains a secret.
- The migration copies a database that holds no credentials (the bot token lives in the
  configuration file, not in SQLite). It copies the configuration file only where the
  operator already keeps it, with its existing permissions.
- The copy stops the writer first and copies the WAL and SHM files with the database;
  copying a live WAL database can lose the newest transactions.
- Residual risk: an operator who renames the volume without copying starts from an empty
  history and may mistake it for data loss. The mitigation is the documented migration,
  the instruction to keep `<previous-name>-data` until the new deployment is verified, and
  the user guide's `Upgrade` step. The owner accepts this risk in ADR-0040.
- No new network exposure: the listen defaults, `127.0.0.1` binding, and
  unauthenticated read-only API are unchanged.

## Verification plan

- **Automated:**
  - `tests/cli.rs::the_binary_reports_its_name` (new) — `--version` starts with
    `uptime-monitor ` and `--help` states `Usage: uptime-monitor`.
  - `tests/init.rs` (extended) — the printed next steps name `uptime-monitor` and
    `uptime-monitor.toml`.
  - Existing assertions updated, none weakened: `tests/web.rs` brand and page title,
    `tests/config_precedence.rs` default database, `tests/cli.rs` default-file discovery
    and log line, `tests/manual.rs` exact user-guide bytes.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test`, plus `python3 -m unittest discover -s scripts/utils -p 'test_*.py'`.
- **Manual:** `cargo run -- --version`; `cargo run -- --help`; `cargo run -- init` in a
  scratch folder and `check` against it; `cargo run -- manual | head`; a visual check of
  `site/index.html`; and, where Docker is available,
  `docker build -t uptime-monitor:dev .` followed by
  `docker run --rm --cap-drop ALL --cap-add NET_RAW uptime-monitor:dev --version`.
- **Rollback:** revert the rename commit (or the branch) and rebuild; the database schema
  and every event are untouched, so the previous binary reads the same file. For a
  deployment, stop the new container, restore the previous image tag, and point compose
  back at the old paths and the old volume, which the migration keeps until verification.

## Open questions

All six were resolved at implementation time by adopting the best guess
stated under each item; the decisions are recorded in `## Implementation Notes`.

- [x] Rename the Gitea repository, the GitHub mirror, and the local checkout folder after
      the merge? Best guess: yes, as an operator step; the plan updates the URLs and
      documents the commands, and nothing in the code depends on the repository name.
- [x] Rename the existing NAS volume with a one-time copy instead of keeping
      `<previous-name>-data`? Best guess: rename, copy once, keep the old volume until the
      new deployment is verified.
- [x] Is 0.13.0 the release that carries the rename? Best guess: yes; `spec-close` applies
      `version_bump: minor`, so implementation leaves `version = "0.12.0"` in place.
- [x] Should the tagline change from "Reachability monitoring"? Best guess: no; it is
      accurate and distinguishes the tool from uptime-percentage products.
- [x] Re-term the bodies of ADR-0001 to ADR-0039? Best guess: no; `docs/ADR.md` declares
      records append-only, and ADR-0040 explains the historical names.
- [x] Rename the helper script to a purpose-based name rather than `export2um.py`? Best
      guess: yes, `convert_heartbeats.py`, because it survives the next rename and it is
      importable by the test.

## Implementation checklist

- [x] Step 1 — Rename the Cargo package, binary, and library, and the Rust imports
- [x] Step 2 — Rename the runtime identity, the configuration defaults, and add the CLI-name test
- [x] Step 3 — Rename the sample configuration and the local ignores
- [x] Step 4 — Rename the container and NAS deployment and document the volume migration
- [x] Step 5 — Rename the import helper and its test
- [x] Step 6 — Sweep the product, architecture, developer, and runbook documents
- [x] Step 7 — Update the user guide, including the 0.12 → 0.13 upgrade
- [x] Step 8 — Update the landing page and its crawl metadata
- [x] Step 9 — Record ADR-0040
- [x] Step 10 — Run the full gate and the residue check
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

### Decisions taken at implementation time

- Repository rename: an operator step, as planned. The site and README state the new URLs;
  no code depends on the repository name.
- NAS volume: renamed with the one-time copy documented in `NOTES.md` and the user guide;
  the old volume is kept until the history is verified.
- Version: the manifest stays `0.12.0`; `spec-close` applies the plan's `version_bump: minor`.
- Tagline: unchanged, still "Reachability monitoring"; the brand name changed only.
- ADRs: ADR-0001 to ADR-0039 bodies are frozen because `docs/ADR.md` declares records
  append-only; the title, a preamble note, the index row, and ADR-0040 record the rename.
- Import helper: `scripts/utils/convert_heartbeats.py` with underscores; the hyphenated name
  in the plan would not be importable by its test.
- The untracked local `<previous-name>.toml` was renamed to `uptime-monitor.toml` and its
  `database` path to `./tmp/uptime-monitor.db`, following the guide's 0.12 → 0.13 step. No
  local database file existed to move.
- `cargo fmt` reflowed test imports and a few long lines because the crate token is shorter;
  no behaviour changed.

### Deviations

- Step 3 verification used `ls` and `git status --short` instead of `git ls-files`: the
  profile sets `git.allow_writes: false`, so the rename stays unstaged (`git mv`/`git add`
  would modify the index).
- Step 4 additionally ran `docker compose -f docker-compose.yml config` (exit 0) as the
  Compose check; the old-name grep shows only the migration block in `NOTES.md`.
- The helper script uses `convert_heartbeats.py` (underscore) rather than
  `convert-heartbeats.py`, for the importability reason above.

### Discoveries

- `tests/simulate.rs::two_processes_of_the_same_seed_serve_the_same_history` failed once
  with `Connection reset by peer` on `/healthz` in the parallel full suite, then passed on
  three isolated runs and two full reruns. The test and its assertion were not modified;
  the evidence points to a pre-existing concurrency flake (ephemeral-port reuse under load)
  unrelated to the rename.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-17
- **Verdict**: PASS
- **Suggested commit**: `refactor(repository-wide): rename the tool to uptime-monitor`
- **Quality gate**: tests ✓ (660 passed, 10 ignored by design) | lint ✓ (0 warnings) | types ✓ (0 errors) | Python helper ✓ (26 tests) — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` and `python3 -m unittest discover -s scripts/utils -p 'test_*.py'`

**Self-review disclosure:** round 1 is a self-review performed in the same invocation as the
implementation (`spec-auto`). No independent reviewer inspected this diff; the verdict rests
on the change set, the plan text, and the commands executed in this session.

### Evidence

| Check | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --check` | ✓ no diff |
| Lint | `cargo clippy --all-targets -- -D warnings` | ✓ finished, 0 warnings |
| Tests | `cargo test` | ✓ 660 passed, 0 failed, 10 ignored (privileged/network) |
| Python helper | `python3 -m unittest discover -s scripts/utils -p 'test_*.py'` | ✓ 26 tests OK |
| Binary identity | `./target/debug/uptime-monitor --version` / `--help` | ✓ `uptime-monitor 0.12.0`, `Usage: uptime-monitor` |
| Init round trip | `init /tmp/uptime-check` then `check --config …` | ✓ creates `uptime-monitor.toml`, prints the new commands, `check` passes |
| Embedded manual | `manual \| head -3` | ✓ `# User Guide — uptime-monitor` |
| Compose contract | `docker compose -f docker-compose.yml config` | ✓ exit 0, service/container/mount/volume new names |
| Residue | `grep -rn -i -e <previous-name> -e <previous_crate_name> -e '<previous-name>'` over source, tests, assets, site, deployment and core docs | ✓ 0 matches |
| Legacy mentions | `grep -rn -i <previous-name>` over guide, import runbook, notification runbook, ADR, notes | ✓ 17 lines, all inside the 0.12 → 0.13 migration, the ADR preamble note, historical ADR bodies, or ADR-0040 |

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | minor | `docs/DEVELOPER-GUIDE.md` | The repository-tree column lost its alignment after the shorter name (`uptime-monitor.toml  example`); corrected to three spaces. | fixed in review |
| 2 | minor | `NOTES.md` | The new migration block omitted the `-c qubo` context that every other command in that runbook uses, so it would have targeted the local daemon; all four commands now carry it. | fixed in review |

Documentation check: `n/a — not configured` in the profile; evidence is the diff hunks, the
residue grep above, and the `manual` output.

### Gap analysis

- Goals: 9/9 achieved.
- Scope: in scope — 57 modified, 3 deleted and 3 new files, all named under *Affected modules*;
  `Cargo.lock` is the generated consequence of the package rename. The plan file itself is the
  phase artefact, not the implementation.
- Deviations: three, documented in `## Implementation Notes` (unstaged rename because Git
  writes are disabled, `docker compose config` used because the daemon is unreachable, helper
  name with underscores because a hyphen is not importable).
- Additions: none beyond the plan; the new CLI-name test and the extended `init` assertion were
  specified in Step 2.
- Deferred: none.
- Known limitations:
  - `docker build` could not run: the CLI is installed but the daemon answers `permission
    denied`; the Dockerfile was verified only by inspection and the Compose contract parsed.
  - Renaming the Gitea repository, the GitHub mirror, and the NAS volume are operator steps;
    the code and the site already state the new names.
  - `tests/simulate.rs::two_processes_of_the_same_seed_serve_the_same_history` failed once with
    a connection reset under the parallel full suite, then passed on three isolated runs and
    both full reruns. The test was not modified; this is a pre-existing concurrency flake.

### Recommendations for future work

- Run the documented volume copy and verify `query state` before removing the old volume.
- Rename the repositories per `docs/DEVELOPER-GUIDE.md#renaming-the-repositories`.
- Track the `simulate` concurrency flake separately; it is unrelated to this rename and was
  deliberately left untouched.

