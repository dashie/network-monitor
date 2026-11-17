---
title: Resolve configuration-file paths against the file and accept a listen key
date: 2026-11-17
status: done
type: feature
impact: medium
area: configuration, CLI, monitor startup and reload, documentation
version_bump: minor
tags: [configuration, paths, listen, cli, reload, compatibility]
depends_on: []
branch_recommended: false
---

## Summary

A relative path in the configuration file now resolves against the file's directory, while a
command-line path stays relative to the working directory and an absolute path is unchanged.
The file also gains a `listen` key: `monitor` binds it when `--listen` is absent, the flag
wins, and `check` verifies it; `serve` and `simulate` keep their own flag.

## Why

The current rule is inconsistent with what a configuration file means. `Config::resolve`
receives a `FileConfig` with no knowledge of the file it came from, and
`config::normalise_path` (the only path helper, called from `resolve_channel` for a
`logfile` filename) joins a relative path to `std::env::current_dir()`. A `database` value is
not even normalised at resolve time; the SQLite open resolves it against the working
directory. Reproduced on the built binary:

- A file at `/tmp/.../conf/uptime-monitor.toml` containing
  `database = "events.db"` and a `logfile` channel at `filename = "audit.log"` prints, from
  the repository root, `database = "events.db"` and
  `filename = "/home/ubuntu/src/uptime-monitor/audit.log"`: the log file lands in the
  working directory, not beside the file that declares it.
- A configuration containing `listen = "127.0.0.1:8642"` is refused by
  `FileConfig`'s `deny_unknown_fields` with `unknown field 'listen'`, so the address can only
  live on the command line. `docker-compose.yml` and `docs/DEVELOPER-GUIDE.md` both state
  that limitation in prose ("`--listen` has no key in the file").

The cost of leaving it: every `check`, `query`, `serve`, `import`, and supervised start must
be launched from the directory where the relative paths were written; a unit file, a container,
or a cron-style command that names an absolute `--config` silently writes the database and the
event log into whatever directory the supervisor chose. The `init` folder already teaches the
opposite habit ("run the commands from the folder"), and its own generated paths break when a
service manager does not.

Adding `listen` to the file closes the last configuration key without a file form: an
operator who already edits one file per host does not have to duplicate the address in a unit
file, and a deploy that copies the file to another host carries the page address with it.
ADR-0036 recorded the working-directory consequence as deliberate for `init`; the semantics now
change, so that consequence is amended rather than left to contradict the plan.

One plan covers both changes: they share `FileConfig`, `Config::resolve`, and the same load
sites, and a split would review the same merge twice without a stable intermediate state.

## Goals

- A relative `database` in a file loaded from `/some/dir/config.toml` resolves to
  `/some/dir/<value>`; `--show-config` prints that absolute path.
- A relative `filename` of a `logfile` channel in the same file resolves to
  `/some/dir/<value>`; `check` creates or opens it there, from any working directory.
- An absolute path in the file is unchanged apart from lexical `.`/`..` folding.
- A relative `--database` (and every other command-line path) still resolves against the
  working directory.
- A file may state `listen = "ADDRESS:PORT"`; `FileConfig` accepts it,
  `Config::resolve` merges it with the flag, and `--show-config` prints `listen` when set.
- `monitor` binds the file's address when `--listen` is absent, binds the flag's address when
  both are given, and binds nothing when neither is given (INV-11 is preserved).
- `check --config <file>` verifies the file's address (or the `--listen` override) in a
  `listen` line, and exits non-zero when it cannot bind.
- A reload that changes `listen` logs one `WARN` naming the new address and keeps the
  already bound one; the monitor keeps running.
- `--show-config` output containing `listen` parses back into the same effective
  configuration.
- `serve` and `simulate` behave exactly as before and do not read the key.
- The existing suite passes, including the scaffolded `init` folder run from another
  working directory.

## Non-goals

- No migration of existing files, databases, or logs: nothing is moved, copied, or renamed
  when the resolution rule changes. An operator whose relative paths relied on the working
  directory must make them absolute or move the file.
- No environment-variable layer and no per-key switch: the layer decides the base, it is not
  itself configurable (BR-3 stays true).
- No expansion of `~`, `$VAR`, or environment variables in a path.
- No symlink resolution: the path given as `--config` names the base directory, and its
  lexical form is used, matching `normalise_path`'s existing contract.
- `listen` does not apply to `serve` or `simulate`; their `--listen` flag and defaults are
  unchanged. Extending it there is a later plan if an operator asks.
- No validation of the address at resolve time: `--show-config` prints it, `check` verifies
  it, and `monitor` fails at bind, exactly as the flag behaves today.
- No listener authentication, TLS, proxy, or new socket: the security posture of the page is
  unchanged.
- No change to how the configuration file itself is found: `--config` and the default
  `./uptime-monitor.toml` still resolve against the working directory.
- No change to the snapshot and backup naming (`<main>.active`, `<main>.<timestamp>.bak`);
  they stay beside the main file.

## Affected modules

**Source**

- `src/config.rs` — `listen` in `FileConfig`, `Config`, and `Overrides`; `parse_at` plus a
  private `rebase_paths` helper; the merge in `Config::resolve`; the `listen` rendering in
  `Config::to_toml`; unit tests for both rules.
- `src/cli.rs` — `Overrides.listen` from `monitor --listen`, the updated help text of that
  flag, and a parse test.
- `src/main.rs` — `load_snapshot` and `resolve_candidate` use `parse_at`; `monitor_command`
  passes the effective `Config::listen` to `run_monitor`; `check` falls back to
  `settings.listen`; `serve` and `simulate` stay untouched.
- `src/monitor/mod.rs` — the reload path uses `config::parse_at`; a changed `listen` is
  reported and the running address is kept.
- `src/init.rs` — the `TEMPLATE` comment stops saying the folder must be the working
  directory and says the paths are relative to the file.
- `src/check.rs` *(maybe)* — the `Options::listen` doc comment now names the file fallback;
  no logic change is expected.

**Tests**

- `tests/config_precedence.rs` — a file `listen` value, a `monitor --listen` override, and
  the file path base through `config::load`.
- `tests/cli.rs` — `--show-config` prints the resolved database path of a file in another
  directory; `check` creates the log file beside the config; the configured `listen` is
  verified; the flag overrides it; the monitor binds the configured address.
- `tests/reload.rs` — a changed `listen` produces the warning and the process keeps running.
- `tests/init.rs` — the scaffolded folder is checked from outside the folder and its log is
  created beside its configuration.
- `src/config.rs` (unit tests) — `parse_at` rebasing, absolute paths, `..` folding, and the
  `listen` merge.

**Documentation / configuration**

- `docs/USER-GUIDE.md` — `What the tool does` (no change), `From an empty folder`, the
  `Configuration > Layers` path rule, the `Global keys` row for `listen`, the
  `Command line > Subcommands` override note, `Verifying a configuration` (`check`), the
  `Simulated mode` non-goal, `Deploy a new configuration` (candidate base directory),
  `Upgrade` (migration note), and `Troubleshooting` (a file in the wrong folder).
- `docs/PRODUCT.md` — `BR-8` and the new `BR-11`; `FR-2`, `FR-12`, `FR-16`, `FR-17`,
  `FR-30`, and the new `FR-35`.
- `docs/ARCHITECTURE.md` — section 2 (`No listening socket unless asked`), section 4
  (`Layered configuration as data`), section 5 (`config` contract and the monitor start),
  section 7 (`Scaffolded folder`, `Configuration sources`), and `INV-2`/`INV-11` wording.
- `docs/ADR.md` — the index rows and `ADR-0038`, `ADR-0039`; a note that `ADR-0036`'s
  working-directory consequence is amended by `ADR-0038`.
- `docs/GLOSSARY.md` — `Check` (configured address), `Web page` (flag or file).
- `docs/DEVELOPER-GUIDE.md` — the compose note that "`--listen` has no key in the file", and
  `Adding a configuration option` gains the path-base rule.
- `uptime-monitor.sample.toml` — a commented `listen`, and the path-base note beside
  `database`.
- `uptime-monitor.toml` — the same path-base note, with `listen` left commented so a
  development start does not open a page.
- `docker-compose.yml` — the comment that claims the key does not exist; the command itself
  may keep `--listen`, which now demonstrates the flag winning over the file (no key is
  added to the compose content).
- `README.md` — no change: every command shown there remains valid; list it here so review
  sees it was considered.
- `site/index.html` — no change: no snippet or claim about the path base or `listen`; list it
  here so review sees it was considered.

## Architecture impact

The `config` component gains one input it always implied: the directory of the file it is
reading. Parsing a file is now two steps — `parse` reads TOML into `FileConfig`, and
`parse_at` additionally rebases the paths that file states — so the merge stays a pure
function and `Config::resolve` keeps its current signature and precedence. The effective
`Config` gains one optional field, `listen`, which the monitor and `check` read; no other
component learns a new responsibility.

The invariants are preserved, not weakened:

- `INV-2` still holds: the file key has a command-line override (`monitor --listen`).
- `INV-11` is restated: a process opens at most one listener, and only when a listen address
  is given on the command line or in the file. `serve` and `simulate` remain separate
  listeners of their own commands, as before.
- `INV-9` still holds: a reload that cannot be applied never stops monitoring; a changed
  `listen` joins the changed `database` as a value reported and used from the next start.

Sections 2, 4, 5, 7, and the invariant list must be updated with this wording.

## Decision-record impact

Two decisions outlive the change and reverse or extend earlier ones:

- `ADR-0038` records the path base: file paths against the file, command-line paths against
  the working directory, absolute paths unchanged; rejected alternatives are "everything
  against the working directory" and a per-key switch. It amends the consequence of
  `ADR-0036` that "relative paths are not rewritten".
- `ADR-0039` records `listen` in the file: one key, `monitor` binds it, `check` verifies it,
  `serve` and `simulate` keep their own flag; rejected alternatives are making the address a
  `serve`-only key, a second `page.listen` table, and an environment variable.

## Proposed solution

### Step 1 — Rebase the paths a configuration file states

In `src/config.rs`, add a private `FileConfig::rebase_paths(&mut self, config_path: &Path)`
that joins `database` and every `notification[].filename` to the file's directory when the
value is relative, then folds `.` and `..` with the existing `normalise_path`; an absolute
value is only folded. Add `pub fn parse_at(text: &str, path: &Path) -> Result<FileConfig,
ConfigError>` that parses and rebases, and make `config::load` use it. Keep `parse` as the
pure parser for callers that validate text before a file exists (`init::validate_template`).

**Verify:** `cargo test --lib config` passes, including new unit tests for a relative
`database`, a relative `filename`, an absolute path, `..` folding, and a path with no parent
(`uptime-monitor.toml` stays relative to the working directory).

### Step 2 — Use the file-aware parser everywhere a file is read

Switch `load_snapshot` and `resolve_candidate` in `src/main.rs`, and the reload parse in
`src/monitor/mod.rs`, from `config::parse` to `config::parse_at`. `config::load` already
covers the ordinary start and every short-lived command that reads the default file.

**Verify:** `cargo test --test cli` and `cargo test --test config_precedence` pass with the
new cases: a file in a temp subdirectory run from its parent prints an absolute `database`
inside the subdirectory, and `check` creates `logs/events.log` beside the file, with no file
created in the working directory.

### Step 3 — Add `listen` to the file and the merge

Add `listen: Option<String>` to `FileConfig`, `Config`, and `Overrides`. In
`Config::resolve`, merge `clean(overrides.listen).or(clean(file.listen))`; an absent value
stays `None` (no page), and an empty value is treated as absent like every other optional
string.
In `src/cli.rs`, map `Command::Monitor { listen, .. }` into `Overrides.listen` so
`--show-config` sees the same effective value, and update the flag help to name the file key.
Render the value in `Config::to_toml` after `database`, with
`skip_serializing_if = "Option::is_none"`.

**Verify:** `cargo test --lib config` and `cargo test --test config_precedence` pass: the file
value alone, the flag alone, the flag winning over the file, no value at all, and a
`--show-config` round trip through `parse`.

### Configuration excerpt (required when the plan adds or changes configuration)

`uptime-monitor.sample.toml`, before:

```toml
interval_ms = 5000
timeout_ms = 2000

# SQLite event database. Keep it on a local filesystem: WAL is not supported on
# network shares.
database = "./tmp/uptime-monitor.db"
retention_days = 0
```

After, with the key present but commented so the sample does not open a page by default:

```toml
interval_ms = 5000
timeout_ms = 2000

# The page the monitor serves when --listen is absent. Without this key and without
# --listen, no page is served. `serve` and `simulate` ignore this key and keep their
# own --listen flag.
# listen = "127.0.0.1:8642"

# SQLite event database. A relative path is relative to this file, not to the
# working directory. Keep it on a local filesystem: WAL is not supported on network
# shares.
database = "./tmp/uptime-monitor.db"
retention_days = 0
```

The command line keeps its own base and wins over the file:

```bash
uptime-monitor monitor --config /etc/uptime-monitor/config.toml --listen 0.0.0.0:8642
# the flag overrides a `listen` key; --database stays relative to the working directory
```

**Verify:** the key in the sample parses and appears with this spelling in `docs/USER-GUIDE.md`
(`Global keys`); `uptime-monitor --show-config --config uptime-monitor.sample.toml` prints
the same merged value when the comment is removed.

### Step 4 — Bind the effective address and let `check` verify it

In `src/main.rs`, resolve the address once the effective configuration is chosen
(`effective.listen` after `choose_configuration`, so a candidate's key applies while it
runs), and pass it to `run_monitor`. For `Command::Check`, pass
`listen.or(settings.listen.clone())` into `check::Options`, so `check --config X` verifies the
configured address and `check --listen Y` verifies the override. Leave `Command::Serve` and
`Command::Simulate` reading only their own flag.

**Verify:** `cargo test --test cli` passes with new cases: a config with
`listen = "127.0.0.1:0"` reports `ok listen 127.0.0.1:0 is available`; a free port written in
the file is bound by a spawned `monitor` and answers an HTTP request; `monitor --listen`
overrides the file and the file's port is not used; a config without the key and a start
without the flag opens no listener.

### Step 5 — Keep the listener when the file changes under a running monitor

In `Supervisor::apply` (`src/monitor/mod.rs`), after the existing `database` comparison, add
the same treatment for `listen`: when `next.listen != self.config.listen`, log one `WARN`
such as `listen changed to <address>; restart the monitor to use it`, keep the current
value (`next.listen = self.config.listen.clone()`), and let the rest of the reload proceed.
A `--listen` flag masks the file for the whole process, so this warning cannot fire when the
flag was given.

**Verify:** `cargo test --test reload` passes with a test that starts a monitor from a file,
rewrites `listen` with another free port, waits for the `WARN` in the captured stderr, and
asserts the process is still running and the old address still answers.

### Step 6 — Align the scaffold with the new base

In `src/init.rs`, change the `TEMPLATE` comment from "Run from this folder" to state that the
paths inside the file are relative to the file, so the folder can be checked or started from
anywhere. Keep `Report::render`'s `cd` line: it remains valid and makes the simplest first run
explicit. Add an integration test that runs `check --config <folder>/uptime-monitor.toml`
from the parent and asserts `<folder>/logs/events.log` exists while the parent has no
`logs/` or `events.db`.

**Verify:** `cargo test --test init` passes, including the new cross-directory test.

### Step 7 — Update the documentation and the sample configuration

Update, in this order, so each statement matches the code that now exists:

1. `docs/ADR.md`: add `ADR-0038` and `ADR-0039` with the index rows, and amend the
   `ADR-0036` consequence in one sentence.
2. `docs/PRODUCT.md`: `BR-11` (path base), the amended `BR-8`, and `FR-2`, `FR-12`, `FR-16`,
   `FR-17`, `FR-30`, plus `FR-35` (`listen`).
3. `docs/ARCHITECTURE.md`: the constraint, the layered-configuration bullet, the `config`
   contract, the monitor start, the deployment view, `INV-2` and `INV-11`.
4. `docs/USER-GUIDE.md`: the sections named in *Affected modules*, including the migration
   note in `Upgrade` and the `serve`/`simulate` exclusion in `Limits and deliberate choices`.
   Keep the guide self-contained and do not link it to the repository documents.
5. `docs/GLOSSARY.md`, `docs/DEVELOPER-GUIDE.md` (the compose note and `Adding a
   configuration option`), `uptime-monitor.sample.toml`, `uptime-monitor.toml`, and the
   `docker-compose.yml` comment.
6. `README.md` and `site/index.html` need no edit; walk them once and record the confirmation
   in `## Implementation Notes`.

**Verify:** `grep -rn "working directory\|has no key in the file\|only when --listen" docs README.md NOTES.md docker-compose.yml` returns only statements that are still true; every hit is either reworded or justified in the plan. `cargo run -- manual | grep -n "listen"` shows the embedded guide with the new key (the guide is embedded at build time, so a rebuild is required).

### Step 8 — Run the complete quality gate

No file changes in this step: it is the gate every previous step must pass. Run the project
profile command from the repository root.

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
exits zero.

## Security considerations

- **New listener source, same surface.** A `listen` key lets a file open a page where only a
  flag could before. That is the same unauthenticated, read-only page and the same `bind`
  call; the exposure is opt-in and the user guide repeats that `0.0.0.0` exposes it on the
  LAN. `check` still proves the address can be bound before a start.
- **Path resolution and trust.** A configuration file already chooses absolute database and
  log paths; resolving relative ones against the file adds no new privilege, because writing
  the file is already equivalent to choosing the target. A file that carries secrets must
  stay mode `0600`, as the guide states; no secret is introduced into a path or a log line.
- **No new dependency, network code, or environment input.** The change touches parsing and
  one merge; it does not read an environment variable, expand `~`, or resolve symlinks.
- **Residual risk.** An operator who relied on the working-directory base and upgrades
  without reading the migration note will find the database and the event log beside the
  configuration file after the upgrade. Nothing is destroyed and no history is rewritten;
  the guide names the condition and the remedy (absolute paths).

## Verification plan

- **Automated:** the new cases in `tests/config_precedence.rs`, `tests/cli.rs`,
  `tests/reload.rs`, `tests/init.rs`, and the `src/config.rs` unit tests listed in *Affected
  modules*. They pin the file base, the CLI base, absolute paths, the `listen` merge and
  precedence, the `--show-config` round trip, the `check` line, the bound page, the reload
  rule, and the scaffold from another directory.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- **Manual:** create `conf/uptime-monitor.toml` with `database = "events.db"` and a
  `logfile` channel at `logs/events.log` (create `conf/logs/` first), then from the parent
  run `--show-config` and `check`; confirm the absolute paths and that `conf/logs/events.log`
  exists. Start `monitor` briefly and confirm `conf/events.db` is created. Add
  `listen = "127.0.0.1:8642"`, start `monitor`, open the page, restart with
  `--listen 127.0.0.1:8643` and confirm the override. With the monitor running, change the
  key and confirm one `WARN` and that the original port still answers.
- **Rollback:** rebuild or restore the previous binary. There is no schema migration and no
  file rewrite; files already created beside the configuration stay there, and a
  configuration made absolute behaves identically on both versions.

## Open questions

- [x] Path base — decided: file paths against the file, command-line paths against the working
  directory, absolute paths unchanged. The rejected alternative (keep the working directory)
  leaves the reproduced inconsistency in place.
- [x] `listen` scope — decided: `monitor` binds it, `check` verifies it, `serve` and `simulate`
  keep their own flag. The narrower scope keeps `simulate`'s "`--listen` is required" contract
  and `serve`'s documented default untouched.
- [x] `--show-config` rendering — decided: a path that came from the file prints absolute, so
  the effective configuration states where the process will actually write; a default or a
  flag value prints as given.
- [ ] Should `serve` read the key in a later release? Current best guess: only if an operator
  asks; the page-only command keeps its explicit flag for now. Owner: maintainer.
- [ ] Should a start warn when a relative file path now points beside the configuration rather
  than the working directory? Current best guess: no; the resolved absolute path is already
  printed by `--show-config` and in the startup log, and a warning on every start is noise.
  Owner: maintainer.

## Implementation checklist

- [x] Step 1 — Rebase the paths a configuration file states
- [x] Step 2 — Use the file-aware parser everywhere a file is read
- [x] Step 3 — Add `listen` to the file and the merge
- [x] Step 4 — Bind the effective address and let `check` verify it
- [x] Step 5 — Keep the listener when the file changes under a running monitor
- [x] Step 6 — Align the scaffold with the new base
- [x] Step 7 — Update the documentation and the sample configuration
- [x] Step 8 — Run the complete quality gate
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

- **Decision** — `Config::resolve` applies `clean()` to both the flag and the file value of
  `listen`, so an empty string is absent in either layer, matching every other optional string
  key. `database` keeps its existing treatment (no trimming) because the plan did not ask for a
  behaviour change there.
- **Decision** — `Cli::overrides` reads `monitor --listen` into `Overrides.listen`; this is what
  makes `uptime-monitor monitor --listen X --show-config` print `listen = "X"` and keeps a
  single merge path for the monitor, the candidate, and the snapshot.
- **Deviation** — the reload test lives in `tests/cli.rs`
  (`a_changed_listen_address_needs_a_restart`), not in `tests/reload.rs` as the step suggested.
  The supervisor’s `WARN` goes to the process stderr; `tests/reload.rs` runs in-process and
  cannot capture it, while the CLI harness already redirects stderr to a file and can also
  prove the old address still answers.
- **Discovery** — the `logfile` filename is rebased by `parse_at` and then passed through
  `normalise_path` again in `resolve_channel`; the second pass is idempotent, so no change was
  needed there and the unit tests that call `Config::resolve` directly keep their old behaviour.
- **Discovery** — `init`’s `Report::render` still prints `cd <folder>`; it remains valid and
  makes the first run explicit, so it was kept. The `TEMPLATE` comment now states that its paths
  are relative to the file.
- **Discovery** — `README.md` and `site/index.html` were walked: every command they show remains
  correct, and neither makes a claim about the path base or about `--listen` having no key, so
  they needed no edit.
- **Evidence** — the page test binds a free loopback port, starts `monitor` with only a
  `listen` key, waits for an HTTP status (`200` or `503` while the database is being created),
  and checks the `web interface ready` line in the captured stderr.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-17
- **Verdict**: PASS
- **Review type**: self-review performed in the same `spec-auto` invocation as the
  implementation; this is not an independent reviewer.
- **Suggested commit**: `feat(config): resolve file paths beside the config and add listen key`
- **Quality gate**: tests ✓ (463 lib + 51 + 10 + 18 + 9 + 7 + 6 + 7 + 5 + 8 + 7 + 3 + 4 + 60
  passing; 2 + 3 + 5 pre-existing ignored) | lint ✓ | types ✓
  (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | minor | `docs/USER-GUIDE.md` | The `listen` row named the key but not its accepted value; it now states `ADDRESS:PORT` | fixed in review |
| 2 | minor | `docs/USER-GUIDE.md` | The new text referenced `BR-11`, an internal identifier that breaks the guide’s self-containment; the references were removed and the rule is stated in prose | fixed in review |
| 3 | minor | `docs/PRODUCT.md`, `docs/ARCHITECTURE.md` | Two statements still described LAN exposure as possible only with `--listen`; both now name the flag or the file key | fixed in review |
| 4 | minor | `docs/USER-GUIDE.md` | `Deploy a new configuration` did not say that a candidate’s `listen` address applies while it runs; one sentence now states it | fixed in review |
| 5 | nit | `src/config.rs` | A non-UTF-8 `--config` directory is joined to the file path through `to_string_lossy`; a lossy path is possible on Unix. Accepted as a known limitation, not fixed | open |

### Gap analysis

- Goals: 12/12 achieved. File-relative database and log file, absolute folding, CLI base,
  `listen` merge and rendering, monitor bind, flag override, `check`, reload warning,
  `--show-config` round trip, unchanged `serve`/`simulate`, and the cross-directory scaffold
  are each backed by a unit or integration test named in *Affected modules*.
- Scope: in scope. Every modified file is listed under *Affected modules*; `src/check.rs` was
  not modified (the `(maybe)` entry), and `README.md`/`site/index.html` were confirmed to need
  no change.
- Deviations: the reload test lives in `tests/cli.rs` instead of `tests/reload.rs`, recorded in
  *Implementation Notes*; the CLI harness can capture the `WARN` and confirm the old address.
- Additions: none beyond the plan.
- Deferred: none.

### Known limitations

- A configuration path that is not valid UTF-8 on Unix is converted lossily when joined to a
  relative `database`/`filename`; no such path exists on Windows, and the previous rule did not
  consult the file path at all.
- `serve` and `simulate` deliberately ignore the `listen` key and keep their own flag.
- Nothing migrates an existing deployment’s paths: an operator must make a path absolute if it
  must stay where it is.

### Recommendations for future work

- `spec-close` applies `version_bump: minor` (0.11.0 → 0.12.0) and records the behaviour change
  in the release note or changelog if the repository keeps one.
- If page-only deployments ask for it, a later plan can let `serve` read the key; this review
  found no need to widen the scope now.
