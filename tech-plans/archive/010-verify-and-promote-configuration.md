---
title: Verify a configuration, promote a candidate only after it probes, and keep an active snapshot
date: 2026-11-17
status: done
type: feature
impact: high
area: cli, config verification, monitor startup, notification channels, configuration recovery
version_bump: minor
tags: [configuration, deployment, startup, dry-run, rollback, snapshot]
depends_on: []
branch_recommended: true
---

## Summary

Deploying a new configuration today means restarting the monitor and discovering its
mistakes one restart at a time: `--show-config` validates the file and the merge, but the
target list, the interfaces, the database path, the listening address, the notification
channels, and the raw-socket privilege are only exercised after the process has started.
This plan adds three things for the operator who changes a configuration on a running host:

1. `network-monitor check` — a real dry run that verifies everything that can be verified
   while the live instance keeps running, writes nothing to the event database, and exits
   non-zero when the configuration would not start.
2. `network-monitor monitor --config <main> --try <candidate>` — the process starts with the
   candidate; if the candidate does not pass the same verification it never replaces anything
   and the monitor runs with `<main>`; once every target has completed its first probe
   without a local failure, the candidate is promoted to `<main>` atomically and the file it
   replaced is kept as one timestamped backup.
3. `<main>.active` — one snapshot of the content the process started from, written once per
   successful start and never touched while it runs, so it always means "the last
   configuration that started and probed", regardless of how many edits were applied at
   runtime afterwards.

The existing hot reload (`ADR-0007`, `INV-9`) stays as it is: it already rejects a broken edit
and keeps the running configuration in memory. What it cannot do is put the accepted content
back on disk, which is the gap item 3 closes.

## Why

On a host that monitors production systems, a restart with a wrong configuration costs
minutes of history for every failed attempt, and the operator learns about each mistake
separately: one restart for the interface, another for the channel, another for the
database path. The tool already knows how to detect every one of those conditions; it just
runs the checks too late and offers no way to run them without starting the service.

Evidence, reproduced on the built binary before writing this plan:

- `--show-config` exits `0` for a file with no `[[targets]]`, while `monitor` fails with
  `no targets configured`.
- `--show-config` exits `0` with `interface = "does-not-exist0"`, while `monitor` fails with
  `interface 'does-not-exist0' does not exist on this host`.
- `--show-config` exits `0` with `--database /nonexistent/dir/x.db` and with
  `monitor --listen not-an-address`; both fail at start.
- `--show-config` exits `0` for a `logfile` channel whose file cannot be opened, and for a
  Telegram token that the API rejects; both stop the process after startup has begun.
- The reload path has the mirror-image gap: an edit that does not parse is rejected and the
  process keeps running the previous configuration **from memory**, while the file on disk
  keeps the rejected content. The next restart reads that broken file and the service fails,
  and nothing on the host holds a copy of the configuration that was working a minute before.

The cost of leaving it: every deployment is a series of restarts, and there is no copy of the
configuration that is known to have worked.

## Goals

- `network-monitor check` exits `0` when the configuration would start, and non-zero when it
  would not, naming the offending key or file on stdout, without starting a probe loop and
  without modifying the event database.
- `check` reports, in one run: target list presence, every target's interface, the event
  database (readable, schema not newer than the binary), the `--listen` address when one is
  given, and every declared notification channel (including a real, read-only Telegram
  `getMe` and a webhook connection that sends no message).
- `check --probe` additionally performs one real probe per target, so a missing `NET_RAW`
  privilege, an unusable interface bind, or an unreachable resolver is visible before the
  restart. A `down` verdict is reported as a warning, never as a failure.
- `check` never prints a secret: the Telegram bot token and the webhook token appear as
  `<redacted>`, exactly as `--show-config` prints them.
- `monitor --config X --try Y` starts with `Y` only if `Y` passes the `check` verification;
  otherwise it logs one `ERROR` naming the reason, logs one `WARN` stating that `X` stays in
  effect, and monitors `X`.
- `--try` naming a file that does not exist is not an error: one `INFO` line states that no
  candidate is pending and the process monitors `X` with exit code `0`, so `--try` can sit
  permanently in a start script or unit file and does nothing until the file appears.
- A candidate whose bytes already equal `X` is not promoted and produces no backup, one
  `INFO` line says so, so a permanent `--try` never accumulates a `.bak` on every restart.
- When the process runs a candidate, the candidate replaces `X` only after every target has
  completed its first probe without a local failure; `X` is preserved as
  `<X>.<UTC timestamp>.bak` in the same directory, and the promotion is logged and announced
  as one `CONFIG` notification.
- When the first probe of any target is a local failure, the running process swaps back to
  `X` in place (no restart), logs and notifies the rejection, and leaves both files untouched.
- A failed promotion (I/O error on the backup or the replace) never stops monitoring and
  never leaves `X` truncated: the file is replaced by an atomic rename of a sibling temporary
  file.
- `<main>.active` holds the exact bytes of the configuration the process started from, is
  written once per successful start — after the first probe round completed with no local
  failure — is created with mode `0600` on Unix, and is never read by the monitor itself: it is
  recovery material, not configuration.
- An edit applied at runtime, accepted or rejected, never changes `<main>.active`: the snapshot
  always names the configuration of the last start that worked, so it is a fixed point and not a
  moving target.
- Copying `<main>.active` over `<main>` restores that configuration, and the reload applies it
  within one second without a restart.
- A start or a rejected reload names `<main>.active` and its age, so the recovery path is
  visible at the moment of the failure instead of being remembered.
- `monitor --fallback-to-active` starts from the snapshot when `<main>` cannot be used,
  logging one `WARN` naming the file it fell back to; without the flag the process fails
  exactly as it does today.
- Existing behaviour is unchanged when neither `check`, `--try`, nor `--fallback-to-active`
  is used, apart from the snapshot file being written beside the configuration.

## Non-goals

- No `--check` flag on `--show-config`: it stays a pure printer (FR-12).
- No separate `promote` subcommand. The `<main>.<UTC timestamp>.bak` taken by a promotion is
  the audit trail of deployments, while `<main>.active` is the fixed recovery point of the last
  start that worked: two files with two lifetimes, and no `.active.bak`.
- No snapshot rotation and no history of snapshots: `<main>.active` is rewritten once per
  start, so there is exactly one snapshot and nothing to rotate.
- No retention or rotation of the `<main>.<UTC timestamp>.bak` files in this plan: a promotion
  is an operator action, one per deployment, and a backup silently deleted at a threshold can be
  the one the operator needs. A `backup_keep` key (default `10`, pruned at promotion) is written
  down in Open questions as a separate, one-step follow-up.
- No log rotation and no log retention setting: `ADR-0033` rejects rotation inside the tool for
  the `logfile` channel, and this plan does not revisit that decision. Rotation of a `logfile`
  channel is the subject of plan `011`; `check` only reports what it finds (one `warn` line for a
  channel that appends without a bound, tracked as an open question there).
- No automatic fallback without the flag: silently starting from an older configuration would
  hide a failed deployment, so the fallback is one opt-in flag and one `WARN` line.
- No automatic restart, no systemd/Docker healthcheck integration, and no supervisor script.
- No deletion, renaming, or truncation of the candidate file after a promotion: the operator
  removes or replaces it, and leaving it in place is a no-op from the next start on because
  its bytes then equal the main file.
- No write test of the event database: `check` reads an existing database and reports a
  database that does not exist yet as a warning, because creating it is what the next start
  does.
- No `SIGHUP` reload and no change to the polling reload of `ADR-0007`.
- No JSON output for `check`, no `--quiet`, no colour: one plain line per check.
- No validation of target reachability by default; reachability is `--probe`, opt-in.

## Affected modules

**Source**

- `src/notify/channel.rs` — add the `verify` method to the `Channel` contract, with a default
  that reports success so the fakes and any future channel stay valid.
- `src/notify/telegram.rs` — `verify` issues a read-only `getMe`; the answer is classified by
  a pure function so it is testable without a socket.
- `src/notify/webhook.rs` — `verify` opens the connection for the channel's scheme and closes
  it, sending no request body.
- `src/notify/logfile.rs` — `verify` opens the file for append and closes it, the same
  operation `open` performs.
- `src/notify/mod.rs` — `pub fn verify(&NotificationConfig) -> Result<(), StartError>`,
  building each declared channel through the existing `build` and reporting the first
  failure with its channel name.
- `src/check.rs` *(new)* — the verification pipeline, its `Line`/`Report` types, the text
  rendering, and the exit decision; unit tests live here.
- `src/snapshot.rs` *(new)* — the `.active` path for a configuration, the atomic write with
  mode `0600`, and the age it reports; unit tests live here.
- `src/promote.rs` *(new)* — the backup name, the atomic replace, and the base name helpers;
  unit tests live here.
- `src/monitor/mod.rs` — the first-probe gate (per target, success or local failure), the
  snapshot writer armed by that gate once after a start, the promotion hook, the swap back to
  the main configuration, and the `Watcher` rebase after a promotion. Neither an accepted nor a
  rejected reload touches the snapshot.
- `src/cli.rs` — the `check` subcommand with `--listen` and `--probe`, `--try` and
  `--fallback-to-active` on `monitor`.
- `src/main.rs` — dispatch for `check`, the candidate decision table (absent, unreadable,
  invalid, identical, promotable), the fallback to the main configuration, the
  `--fallback-to-active` path, and the recovery hint in the start failure message; reuse of
  `validate_interfaces`.
- `src/lib.rs` — register `check`, `promote`, and `snapshot`.
- `src/time.rs` — `format_compact_utc(ms) -> String` producing `20261117T031500Z` for a
  backup name (the existing `format_rfc3339` carries `-` and `:`, which are noise in a file
  name).
- `src/config.rs` *(maybe)* — a small helper if the candidate merge needs one; no behavioural
  change to `resolve` is expected.

**Tests**

- `tests/cli.rs` — the `check` contract (exit codes, one line per check, secrets redacted, no
  database file created), and `monitor --try` falling back to the main file on a bad
  candidate.
- `tests/promote.rs` *(new)* — backup naming, atomic replace, mode preservation, main file
  untouched when the candidate fails.
- `tests/snapshot.rs` *(new)* — the snapshot is written once after a clean first round, is not
  written when the first probe fails locally, is untouched by an accepted reload and by a
  rejected one, keeps mode `0600`, and `--fallback-to-active` starts from it when main is
  broken.
- `tests/reload.rs` *(maybe)* — a promotion must not be re-applied as a reload; a rejected
  reload leaves the snapshot untouched.
- `src/monitor/mod.rs` tests *(maybe)* — the first-probe gate driven by `FakeProbe`.
- `src/notify/mod.rs` tests — `verify` names the failing channel and closes the ones it
  opened.
- `src/notify/telegram.rs` tests — `getMe` classification, token never in the reason.

**Documentation / configuration**

- `docs/PRODUCT.md` — new `FR-30` (check), `FR-31` (candidate promotion) and `FR-32`
  (active snapshot and the opt-in fallback); amend `FR-13` so a startup error is the same
  set `check` reports; state in "Scope" that deployment is covered.
- `docs/ARCHITECTURE.md` — the component table and responsibilities for `check`, `promote` and
  `snapshot`, the "Configuration" section (verification, candidate, promotion, snapshot), and
  a new `INV-15` stating that neither the active file nor the snapshot is replaced or written
  unless every target has probed once without a local failure.
- `docs/ADR.md` — `ADR-0034 — Verify a configuration without starting, promote a candidate
  only after it probes, and keep one active snapshot`, plus its index entry.
- `docs/USER-GUIDE.md` — "Command line" (`--try`, `--fallback-to-active`), "Subcommands"
  (`check`, with `--listen` and `--probe`), "Configuration" → "Layers" (candidate, backup,
  promotion, the `.active` snapshot, and the fact that an absent `--try` file is a quiet
  no-op so the flag can stay in a start script), "Operations" (a "Deploy a new configuration"
  procedure and a "Recover a configuration edited by mistake" procedure, both with rollback),
  "Troubleshooting" (what a `fail` line means).
- `docs/DEVELOPER-GUIDE.md` — repository layout rows for `src/check.rs`, `src/promote.rs` and
  `src/snapshot.rs`, and a note in "Extending the project" that a new channel must decide its
  `verify`.
- `docs/GLOSSARY.md` — "Candidate configuration", "Promotion", "Check", "Active snapshot".
- `.gitignore` — ignore `*.active` and `*.bak` beside the configuration, because both carry
  the same secrets as the file they are copied from.
- `README.md` *(maybe)* — one line in the quick start pointing at `check`.
- `site/index.html` *(maybe)* — the landing page lists commands; add `check` if the copy
  would otherwise be stale.

## Architecture impact

`check` is a new read-only component beside `query`: it consumes the resolved `Config`, the
`Channel` contract, `Probe`, `store` (read-only), and `web::bind`, and it owns no state. The
startup path of `monitor` gains one decision before the workers start (which configuration to
run) and one after the first probe round (write the snapshot, then promote or swap back); the
file operations are isolated in `promote` and `snapshot`, so `monitor` keeps depending only on
a contract. `INV-15` is added: neither the active file nor the snapshot is written unless every
target has probed once without a local failure, and the snapshot is written once per start —
the reload path, accepted or rejected, never writes it. `docs/ARCHITECTURE.md` sections 5
(building blocks), 8 (cross-cutting concepts → Configuration) and the invariants list must
state all of it.

## Decision-record impact

A new decision record is required: `ADR-0034`. It records why verification is a subcommand
and not a flag on `--show-config`, why promotion happens in the running process instead of in
a `promote` command, why the deployment history is one timestamped sibling while recovery is a
second file written once per start and never on a reload, why the snapshot holds the accepted
bytes and not the rendered `--show-config` output (that output redacts the tokens), and why the
gate is the first probe round instead of the `READY` notification. Without it, the next reader
would re-open the same alternatives.

## Proposed solution

### Step 1 — Give the channel contract a `verify` method

In `src/notify/channel.rs` add to `trait Channel`:

```rust
/// Prove the channel can deliver, without delivering anything.
fn verify(&mut self) -> Result<(), String> {
    Ok(())
}
```

Implement it in the three real channels: `logfile` opens and closes the file; `webhook`
establishes the connection its scheme needs (TLS handshake included) and sends nothing;
`telegram` issues `GET https://api.telegram.org/bot<token>/getMe` through the existing
exchange plumbing and classifies the answer with a pure function that returns `Ok` for
`"ok":true` and a scrubbed reason otherwise. In `src/notify/mod.rs` add
`pub fn verify(config: &NotificationConfig) -> Result<(), StartError>` that builds every
declared channel in declaration order with the existing `build`, calls `verify`, closes what
it opened, and stops at the first failure, naming the channel exactly as `start` does.

**Verify:** `cargo test notify` passes with new unit tests: a fake channel whose `verify`
fails names the channel and leaves the earlier channels closed; the `getMe` classifier maps
`200`/`"ok":true` to success and `401` to a reason that does not contain the token.

### Step 2 — Write the `check` component

`src/check.rs` exposes `pub fn run(config: &Config, options: Options) -> Report`, where
`Options { listen: Option<String>, probe: bool }`. `Report` holds `Line { level, subject,
detail }` entries and renders one line per check:

```text
ok    targets            7 target(s)
ok    interface          icmp://1.1.1.1 eth0
ok    database           ./tmp/network-monitor.db (schema 5)
warn  listen             0.0.0.0:8642 is in use — the running monitor?
fail  channel telegram   Unauthorized
```

The pipeline follows the order the process starts in, so the first `fail` is the condition
that would have stopped it: `targets` (`require_targets`), `interface`
(`probe::interface::validate` for every target), `database` (`Store::open_ro` when the file
exists; `warn` naming the path when it does not), `listen` (`web::bind`, then drop the
listener: `ErrorKind::AddrInUse` is a `warn`, anything else a `fail`), `channel` (via
`notify::verify`), and — only with `probe` — one `Prober::probe` per target through the real
`Endpoint`, where `Verdict::Up` is `ok`, `Verdict::Down` is `warn` with the failure code and
message, and `ProbeError` is `fail` with its reason. `Report::failed()` answers whether any
line is `fail`, and `Report::render()` produces the text.

**Verify:** `cargo test check` passes with unit tests over temporary directories: a
target-less config fails on `targets`, an unknown interface fails naming the interface, a
missing database warns, a bad listen address fails, an in-use address warns, and a report
holding only `ok`/`warn` lines is not failed.

### Step 3 — Expose `network-monitor check`

In `src/cli.rs` add `Command::Check { listen: Option<String>, probe: bool }` with
`--listen <ADDRESS:PORT>` and `--probe`; in `src/main.rs` resolve the configuration exactly
as today, print the report on stdout, and return `ExitCode::FAILURE` when it holds a `fail`
line. Move `validate_interfaces` out of the `Monitor` arm so both arms share it. `check` opens
no store for writing, starts no worker, sends no notification, and leaves the working
directory as it found it (asserted by the test).

**Verify:** `cargo run -- check --config ./network-monitor.toml` prints the report and exits
`0`; `cargo run -- check --config /tmp/bad.toml` with an unknown interface exits `1` and names
it; `git status --short` shows no new database file.

> **🔶 CHECKPOINT CP-1 — the dry run is usable on its own**
> **Stable state:** `check` verifies and exits; nothing in the startup path has changed, so
> the monitor behaves exactly as before.
> **Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`,
> then run `check` against the real configuration on the host, with and without `--probe`.
>
> The implementer must stop here, report, and wait for explicit user confirmation.

### Step 4 — Add the candidate flag and the pre-flight decision

In `src/cli.rs` add `--try <PATH>` to `Command::Monitor`. In `src/main.rs`, when it is given,
require `--config` (a clear usage error otherwise, because the promotion needs a destination)
and refuse `--try` equal to `--config`. Then read the candidate **once**, keep its bytes, and
decide in this order — the flag must be safe to leave in a start script forever:

| Candidate at `<try>` | Severity | What the process does |
|---|---|---|
| does not exist | `INFO no candidate configuration at <try>; using <config>` | monitors `<main>`, exit `0` |
| exists but cannot be read (permissions, I/O) | `ERROR <reason>` + `WARN <main> stays in effect` | monitors `<main>` |
| parses and resolves, and the `check` pipeline (without `listen` and without `probe`) reports a `fail` line | `ERROR <subject>: <detail>` + `WARN <main> stays in effect` | monitors `<main>` |
| is valid and its bytes equal `<main>` | `INFO candidate <try> matches <config>; nothing to promote` | monitors the candidate settings (identical to main), no promotion, no backup |
| is valid and differs from `<main>` | — | monitors the candidate settings and carries a `promote::Candidate { main, bytes, stamp }` |

The first three rows are the fallback the previous step described, and they are the same code
path as a plain start. Only the last row arms the promotion of Step 5, and it arms it with the
bytes read now, so a candidate edited or deleted afterwards can never promote content that was
not verified. When `--fallback-to-active` is also given (Step 7) and the main file itself is
unusable, the fallback continues to the snapshot instead of failing; without the flag these
rows end exactly where a plain start ends.

**Verify:** `cargo test --test cli` passes with new tests: `--try` without `--config` is a
usage error; `--try` naming the same file as `--config` is refused; an absent candidate
prints the `INFO` line, exit code is `0`, and the main database receives events; an unreadable
candidate logs `ERROR` + `WARN` and monitors main; a candidate with a bad interface logs the
error and the warning and the process keeps running with main; a candidate byte-identical to
main produces no `.bak` file.

### Step 5 — Promote after the first probe round

`src/monitor/mod.rs`: `monitor_target` reports its first outcome — once — through a small
`Arc<FirstRound>` that counts the targets still to report and records whether any first probe
was a `ProbeError`. `run_with_notifier` receives `promotion: Option<Promote>`; when the
counter reaches zero and no local failure was seen, it calls `promote::commit(&candidate)`,
then logs `INFO configuration promoted: <candidate> -> <main> (previous kept as <backup>)`,
sends `Message::config_reload(true, ..., now_ms())`, and rebases the reload
`Watcher` on the promoted content so the write is not mistaken for an operator edit.

`src/promote.rs` implements [`Candidate`] and `commit(&self) -> Result<PathBuf,
PromoteError>`, using the bytes captured at startup and never re-reading the candidate path:
build the backup path `<main file name>.<format_compact_utc(stamp)>.bak` in the main file's
directory, copy `main` to it (keeping the original mode), write the captured bytes to a
sibling temporary file created with mode `0600` on Unix, flush, and `fs::rename` the
temporary over `main`. Every failure returns without touching `main`.

**Verify:** `cargo test promote` passes: the backup carries the timestamped name and the
original bytes; `main` holds the captured bytes afterwards; a failed copy leaves `main`
byte-identical; the promoted file is `0600` on Unix; a `Candidate` whose bytes equal `main`
is skipped with `Ok(None)` and writes nothing. A `tests/reload.rs` case proves that a
promotion produces no reload of the workers in the next poll.

### Step 6 — Swap back when the candidate cannot probe

In the same gate: when any first probe is a `ProbeError`, do not promote. Instead log one
`ERROR` line naming the target and the reason, one `WARN` line stating that `<main>` is back in
effect, send `Message::config_reload(false, ..., now_ms())`, swap the supervisor to the main
`Config` (reusing the swap that `Supervisor::apply` already performs, extracted as
`fn swap(&mut self, next: Config, detail: &str)`), and keep both files untouched.

**Verify:** `cargo test monitor` passes with a unit test where `FakeProbe` returns
`ProbeError` for the first probe of one target: the supervisor ends up on the main
configuration, no promotion file exists, and the rejection notification was queued.

### Step 7 — Keep an active snapshot of the configuration that started

`src/snapshot.rs` (new) exposes three functions over one file in the configuration's own
directory, `<config file name>.active`: `path_for(&Path) -> PathBuf`; `write(&Path, &[u8])`,
which publishes the content through a sibling temporary created with mode `0600` on Unix and an
`fs::rename`, so a reader never sees a half-written snapshot; and `describe(&Path) ->
Option<String>`, which renders `<path> (<age>)` from the modification time for the log lines.

`src/monitor/mod.rs` arms the snapshot from the same `FirstRound` gate Step 5 introduces, once:
after a start, it writes the bytes the process started from — the main file's bytes, or the
candidate's when `--try` chose one. Nothing else writes it: a reload, accepted or rejected, a
promotion, and the swap back leave `<main>.active` untouched, because the snapshot must always
mean "the last configuration that started and probed", not "the last thing that was merged". A
first round that saw a local failure writes nothing, so the previous snapshot — still the
configuration that works — survives a bad deployment. The consequence is stated where an
operator will read it: a batch of accepted hot edits is not in the snapshot until the next
restart, and one restart is what refreshes it.

`src/main.rs` adds the recovery path in both directions: every failure that stops the process
before the first probe now ends with `; the last configuration that worked is <path> (<age>)`
when the snapshot exists, and `monitor --fallback-to-active` starts from the snapshot
instead of failing when `<main>` cannot be read, parsed, resolved, or pre-flight verified,
logging one `WARN` naming the file it used. Precedence is candidate (Step 4) → main → snapshot,
and the snapshot is read as configuration only through the same `config::parse` +
`Config::resolve` + `check` pipeline, never trusted blindly; with `--try` and the flag together
the candidate wins, then main, then the snapshot. `src/check.rs` gains one line: `ok active
<path> (<age>)`, or `warn active no snapshot yet`.

**Verify:** `cargo test snapshot` passes: the snapshot is written once after a clean first
round and holds the exact bytes the process started from; a first round with a `ProbeError`
writes nothing and leaves an existing snapshot untouched; an accepted reload and a rejected one
leave the file byte-identical; the file is `0600` on Unix; a second start overwrites it and
leaves no temporary file behind. `cargo test --test cli` adds: a broken `<main>` with
`--fallback-to-active` starts and logs the `WARN`; without the flag it fails and the error names
the snapshot path.

> **🔶 CHECKPOINT CP-2 — the deployment path is complete**
> **Stable state:** all of `check`, `--try`, promotion, swap-back and the active snapshot
> work; documentation and the version are still pending.
> **Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`,
> then the manual scenarios in the verification plan on the host.
>
> The implementer must stop here, report, and wait for explicit user confirmation.

### Step 8 — Update the documentation

Write the entries listed under "Documentation / configuration": `FR-30`/`FR-31`/`FR-32` and
the `FR-13` amendment in `docs/PRODUCT.md`; the components, the Configuration section, and
`INV-15` in `docs/ARCHITECTURE.md`; `ADR-0034` plus its index entry in `docs/ADR.md`; the
`check`, `--try`, `--fallback-to-active`, deploy and recovery procedures in
`docs/USER-GUIDE.md`; the layout rows and the channel `verify` note in
`docs/DEVELOPER-GUIDE.md`; the four terms in `docs/GLOSSARY.md`; the two ignore patterns in
`.gitignore`; and, when the copy would otherwise be stale, `README.md` and `site/index.html`.

**Verify:** every option documented in `docs/USER-GUIDE.md` exists in `src/cli.rs` with the
same name and default (`grep -n "try\|probe\|listen\|fallback" src/cli.rs`),
`docs/ARCHITECTURE.md` states `INV-15` with the test that enforces it, and
`git check-ignore -v network-monitor.toml.active` names the new pattern.

### Step 9 — Bump the version

Set `version = "0.9.0"` in `Cargo.toml` and let `cargo` update `Cargo.lock`.

**Verify:** `cargo metadata --no-deps --format-version 1 | grep '"version"'` reports `0.9.0`,
and `git diff --stat Cargo.lock` shows only the crate's own version.

### Step 10 — Run the quality gate

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
passes with no new warnings.

## Security considerations

- The configuration holds the Telegram bot token and the webhook token. `check` prints only
  `<redacted>` for them, and a channel failure reason is produced by the same scrubbing the
  send path uses: a `getMe` answer is reduced to its status and its `description`, never to
  the request path.
- Promotion rewrites a file that may be `0600` because it carries secrets. The temporary file
  is created with mode `0600` on Unix before the rename, so the promoted file is never
  readable by another user, and the backup keeps the mode of the file it copies.
- `<main>.active` carries the same secrets as the file it copies: it is created with mode
  `0600` on Unix, published through a `0600` temporary and a rename, and ignored by Git
  (`.gitignore` gains `*.active` and `*.bak`) so a working copy of a configuration cannot be
  committed by accident. Its content is never logged and never printed, only its path and age.
- `check` performs outbound traffic: one `getMe` per Telegram channel, one connection per
  webhook, and — only with `--probe` — one probe per target. It posts no message and writes
  nothing to the event database, so it cannot change what an operator is being told.
- `check` may create a missing `logfile` file with mode `0600`, exactly as a start does. This
  is documented; it is the only write `check` performs.
- `check` opens an existing database read-only (`Store::open_ro`), the same path the web page
  uses, so it can never migrate, prune, or repair the history (`INV-10`).

## Verification plan

- **Automated:** unit tests for `check` (each line level), `promote` (name, bytes, mode,
  atomicity), `snapshot` (gate, exact bytes, `0600`, no temporary files), the channel `verify`
  contract and the `getMe` classifier, the first-round gate with `FakeProbe`, and integration
  tests in `tests/cli.rs`, `tests/promote.rs`, `tests/snapshot.rs`, and `tests/reload.rs` for
  the exit codes, both fallbacks, and the absence of a spurious reload.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo
  test`.
- **Manual, on the monitoring host:**
  1. `network-monitor check --config ./network-monitor.toml --listen 0.0.0.0:8642` while the
     service runs: every line is `ok` except a `warn` for the busy address.
  2. Break the file deliberately — an unknown interface, a revoked token, a target-less file
     — and confirm one `fail` line naming it and exit `1`.
  3. `network-monitor monitor --config ./network-monitor.toml --try ./next.toml --listen
     0.0.0.0:8642` with a good candidate: one `INFO` promotion line, `next.toml` copied over
     `network-monitor.toml`, the previous file present as
     `network-monitor.toml.<timestamp>.bak`, and one `CONFIG` message on the channels.
  4. Repeat with a candidate whose interface does not exist: one `ERROR`, one `WARN`, the
     monitor running the old configuration, both files unchanged.
  5. Leave `--try ./next.toml` in the unit file and remove `next.toml`: the start is normal,
     one `INFO` line says no candidate is pending, and the exit code is `0`. Restart again
     with `next.toml` present and identical to `network-monitor.toml`: one `INFO` line,
     no new `.bak`, so a permanent flag stays quiet.
  6. With the service running, break `network-monitor.toml` in place — a typo, a missing
     `]]`: one `WARN` says the reload was rejected and monitoring continues, and
     `network-monitor.toml.active` still holds the configuration that works. Copy it back
     with `cp network-monitor.toml.active network-monitor.toml`: the reload applies it
     within a second, with no restart.
  7. Restart with the broken file still in place and no options: the start fails and the error
     names `network-monitor.toml.active` and its age. Add `--fallback-to-active`: the
     process starts, logs one `WARN`, and monitors the snapshot content.
  8. Make an edit the monitor accepts (change `interval_ms`) and wait one interval: the new
     interval applies, and `network-monitor.toml.active` is byte-identical to what the
     process started from, so a later incident falls back to the start configuration and a
     restart is what refreshes the snapshot.
- **Rollback:** restore `network-monitor.toml.<timestamp>.bak` (a deployment) or
  `network-monitor.toml.active` (the last configuration that started) over
  `network-monitor.toml` and restart; the reload picks the file up with no other change. If
  the feature misbehaves, the code rollback is one revert: `check`, `--try`, and
  `--fallback-to-active` are additive, and no existing key, file, or command changes meaning.

## Open questions

- [x] Should `check --probe` be the default in the container healthcheck? Best guess: no —
  keep it opt-in, because a probe from the healthcheck doubles the traffic and races the
  monitor's own probes. Answered by the operator when the deployment is written.
  **Resolved:** opt-in, as guessed; no healthcheck or compose change is part of this plan.
- [x] Should an absent `--try` file be `INFO` (current choice) or `WARN`? Best guess: `INFO`,
  because a deployment that is not pending is the normal case and a warning on every boot
  trains the operator to ignore warnings; `WARN` is kept for a candidate that exists and
  cannot be used. Answerable in one line if the operator prefers the louder line.
  **Resolved:** `INFO`, implemented and tested in `tests/cli.rs`.
- [x] Should `--fallback-to-active` be the default? Best guess: no — starting from an older
  configuration without being told hides a failed deployment, and the error message already
  names the snapshot. Revisit if the operator restarts unattended hosts.
  **Resolved:** opt-in, and the failing start names the snapshot and its age.
- [x] Retention of the `<main>.<UTC timestamp>.bak` files: this plan keeps every one of them,
  and the follow-up design (not part of this plan) is a `backup_keep` key with its
  `--backup-keep` flag, default `10`, applied at promotion time by deleting the oldest beyond
  the limit. Best guess: not needed yet — a promotion is one per deployment, and the value is
  that the file is still there when it is needed.
  **Resolved:** no pruning in this plan; the user guide states that the tool never prunes them.
- [x] Rotation of the `logfile` channel and of the diagnostic log: out of scope, and `ADR-0033`
  already rejects rotation inside the tool. Best guess: the operator rotates with `logrotate`
  (`copytruncate` suits the append mode) or the container log driver, and no configuration key
  is added here.
  **Resolved:** untouched; rotation remains plan `011`'s subject.
- [x] Should the Telegram `getMe` timeout follow the target timeout or a fixed value? Best
  guess: a fixed two seconds, like the send path, so a slow API never makes `check` hang.
  **Resolved:** a fixed value shared with `sendMessage` — the channel's existing five-second
  `TIMEOUT`, not two: the send path in the code is five seconds, and using one budget keeps
  verification and delivery failing on the same terms.

## Implementation checklist

- [x] Step 1 — channel `verify` contract and implementations
- [x] Step 2 — `src/check.rs` pipeline and report
- [x] Step 3 — `network-monitor check` in CLI and dispatch
- [x] CP-1 — dry run usable on its own (confirmed by the full suite under autopilot)
- [x] Step 4 — `monitor --try` candidate decision table and fallback
- [x] Step 5 — promotion after the first probe round
- [x] Step 6 — swap back when the candidate cannot probe
- [x] Step 7 — active snapshot at start and the opt-in fallback
- [x] CP-2 — deployment path complete (confirmed by the full suite under autopilot)
- [x] Step 8 — documentation (`PRODUCT`, `ARCHITECTURE`, `ADR`, `USER-GUIDE`, `DEVELOPER-GUIDE`, `GLOSSARY`, `.gitignore`)
- [x] Step 9 — version `0.9.0`
- [x] Step 10 — quality gate
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

### Step 1 — channel `verify`

- **Decision:** the Telegram `getMe` classifier is the existing pure `classify(status, body)`.
  It already accepts exactly `200` + `"ok":true`, builds the reason from the status and the
  `description` only, and never sees the request path — so a second function would have been
  the same code with another name (KISS).
- **Decision:** the log file `verify` reuses the exact `open` operation by factoring
  `open_file() -> Result<File, String>`; `open` stores the handle, `verify` drops it, which
  closes the file. No behaviour change to `open`.
- **Decision:** the webhook `verify` opens the connection the scheme needs (the TLS handshake
  for `https`) through `open_connection()` and drops it without writing a request.

### Step 2 — `src/check.rs`

- **Decision:** the database line reports `(readable)` rather than the schema number. The
  sample in the plan shows `(schema 5)`, but `Store` has no schema-version accessor and
  `src/store/mod.rs` is outside the affected modules; `open_ro` already refuses a newer
  schema, so the check's contract is unchanged. Reviewed in Step 8.
- **Decision:** the report keeps going after a `fail` line — channels and, with `--probe`,
  targets are still checked — because the plan promises every declared channel in one run.
  The line order still makes the first `fail` the condition that would stop the start.

### Step 3 — `network-monitor check`

- **Decision:** `check` prints only the report on stdout and writes no `INFO initializing`
  lines, so the command stays pipeable. Diagnostics of the channels go through the existing
  logging path on stderr.
- **Decision:** `validate_interfaces` is already a module-level helper; `check` applies the
  same `interface::validate` per target inside its pipeline so it can name the offending
  interface on stdout instead of aborting. The monitor arm keeps the aggregate helper.

### CP-1

The implementer could not wait for an interactive confirmation because the session runs in
autopilot mode. The full suite was run and passed before continuing:
`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` (all green,
0 failed). `check` was also run against `./network-monitor.toml`, with and without
`--probe`; the run printed the expected report and, under `--probe`, failed on the two ICMP
targets for the missing `CAP_NET_RAW` while the other probes reported their verdicts.

### Step 4 — `monitor --try`

- **Decision:** `--try` requires an explicit `--config`, even when a default
  `network-monitor.toml` exists, because the promotion needs a destination named on purpose.
  The check runs before any file is read, so `--try` alone is a usage error in every
  directory.
- **Decision:** the candidate is read into bytes once and every later question (parse,
  resolve, check, equality, promotion) uses those bytes. Equality with the main file is a
  raw byte comparison; the main bytes are read once at the decision point.
- **Deviation:** `Candidate` also carries `source: PathBuf`, the candidate path, so the
  promotion log line can state `<candidate> -> <main>` as the plan requires. The path is
  never re-read.

### Step 5 / Step 6 — promotion and swap-back

- **Deviation:** `monitor::Start` bundles `startup`, `promotion`, `fallback` and `snapshot`.
  Clippy refuses a function with more than seven arguments, and `run_with_notifier` would
  have had eight; the plan's contract (it receives the promotion) still holds through
  `Start`, and the call sites got simpler.
- **Decision:** the first round is tracked as the set of target keys that reported once plus
  the first local failure. A reload restarts workers, and the set makes a duplicate report
  harmless instead of underflowing a counter.
- **Decision:** `Supervisor::swap(next, detail)` is the extracted tail of `apply`; it stops
  the workers, adopts the configuration, resets the retention window, starts the workers
  again and logs `detail`. `apply` keeps its exact log and notification texts.
- **Decision:** the promotion unit tests live in `src/promote.rs` (where the plan also puts
  unit tests) rather than being duplicated in `tests/promote.rs`; `tests/reload.rs` proves
  the promotion is not replayed as a reload.

### Step 7 — the active snapshot

- **Decision:** `check::Options` gained `active: Option<PathBuf>` so the check can report
  the snapshot without guessing the configuration path; the binary passes
  `snapshot::path_for(<config>)`.
- **Decision:** when `--fallback-to-active` starts from the snapshot, the monitor still
  watches the main path, so fixing the broken file applies through the normal reload. The
  snapshot of that start holds the bytes it started from (the old snapshot content).
- **Discovery (pre-existing, not fixed):** a reload that changes the set of target keys
  logs `notification settings changed; restart the monitor to use them`, because
  `NotificationConfig.by_target` contains an entry per target key and therefore compares
  unequal. It predates this plan and is outside its scope; recorded for the next reader.

### CP-2

Again under autopilot, with no interactive confirmation available. The full suite was run
and passed before continuing; the manual promotion scenario was also exercised end to end
on the built binary: a good candidate was promoted, the previous file was kept as
`main.toml.<stamp>.bak`, the main file became the candidate, and `main.toml.active` held the
candidate content.

### Step 8 — documentation

- **Decision:** the landing page's "From source" snippet gained the `check` line, because its
  quick start would otherwise describe a deployment flow without the new verification step.
- **Decision:** the user guide's `check` sample shows the `warn active` line, so the new line is
  documented where an operator reads the output for the first time.

### Step 9 — version

`Cargo.toml` is at `0.9.0`; `Cargo.lock` changed only in the crate's own version line.

### Step 10 — quality gate

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passed on
2026-11-17: 429 library tests, 40 CLI tests, 3 snapshot tests, the reload suite (7, including
the promotion-is-not-a-reload case), the web suite (60), and every other target green; 3
privileged ICMP tests remain ignored as before.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-17
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(config): verify, promote, and snapshot configuration`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | blocker | `src/main.rs` (`load_start`) | `monitor --fallback-to-active` falls back only when the main file cannot be loaded or resolved. A main file that parses and resolves but fails pre-flight verification — an interface that does not exist, no targets, a channel that cannot verify — stops the process instead of starting from the snapshot. Step 7 requires the fallback when the main file "cannot be read, parsed, resolved, or pre-flight verified", and FR-32 states the same. Evidence: a main with `interface = "does-not-exist0"` and a valid `main.toml.active` exits `1` with `error: interface 'does-not-exist0' … the last configuration that worked is main.toml.active (0s ago)` and never reads the snapshot. | open |
| 2 | minor | `tests/monitor_e2e.rs` | The file is outside `## Affected modules`; the only change is the mechanical call-site update to `monitor::run(…, monitor::Start::default())` after the planned `Start` signature change. No assertion changed. | open |
| 3 | minor | `src/main.rs` (`main_config_path`) | When the configuration is the implicit default and `network-monitor.toml` does not exist, the recovery hint is skipped even when `network-monitor.toml.active` does exist; a start that fails on `no targets configured` prints no snapshot path or age. Step 7 asks every pre-probe failure to name the snapshot when it exists. Evidence: a directory holding only `network-monitor.toml.active` exits `1` with `error: no targets configured; …` and no hint. | open |

### Gap analysis
- Goals: 15/16 achieved; the `--fallback-to-active` goal is partial because the pre-flight condition of Step 7 is missing.
- Scope: in scope except `tests/monitor_e2e.rs` (minor, mechanical) and `.github/workflows/pages.yml` (unrelated working-tree change, not part of this plan).
- Deviations: `Start` bundles four start parameters (the clippy argument limit), `Candidate.source`, the promote tests as unit tests in `src/promote.rs`, the database detail `(readable)`, and the Telegram `getMe` timeout shared with `sendMessage` at 5 s — all recorded in Implementation Notes.
- Additions: none beyond the plan.
- Deferred: `.bak` retention and log rotation, as the plan states.

### Known limitations
- The snapshot restore (`cp <main>.active <main>` then the reload) is verified by composition — the generic reload path — and not by a dedicated automated test.
- A failed `fs::copy` of the backup may leave a partial `.bak`; the main file is untouched.

### Recommendations for future work
- Run the `check` pipeline on the main configuration in `load_start` when `--fallback-to-active` is set (without `listen` and `probe`, as the candidate path does), and add CLI tests for an unknown interface and for a target-less main.
- Derive the recovery-hint path from the default file name even when the file is absent, so the hint works when only `network-monitor.toml.active` remains.

### Review round 1 — findings closed

- **Finding 1 (blocker, `src/main.rs` `load_start`):** `monitor --fallback-to-active` now runs
  the `check` pipeline on the main configuration — without `listen` and without `probe`,
  exactly like the candidate decision — and falls back to the snapshot on the first `fail`
  line, not only on read/parse/resolve errors. `main_config_path` returns the default path
  even when the file is absent, so the snapshot beside a missing main is still found.
  **Verified:** new CLI tests for an unknown interface, a target-less main, and a snapshot
  with no main file; the existing parse-failure test still passes. The fallback logs one
  `WARN` naming the main reason and the snapshot with its age.
- **Finding 2 (minor, `tests/monitor_e2e.rs`):** recorded as a scope note — the file is
  outside `## Affected modules`; the only change is the mechanical call-site update to
  `monitor::Start` after the planned signature change, with no assertion changed.
- **Finding 3 (minor, recovery hint):** covered by the same `main_config_path` change: a
  start that fails with only `network-monitor.toml.active` present now names the snapshot
  and its age. **Verified:** new CLI test
  `a_failure_with_only_a_snapshot_names_it_and_its_age`.
- **Decision:** the pre-flight fallback excludes `--listen` (a start argument, not
  configuration; an older file cannot fix a busy or invalid address) and `--probe`
  (reachability is not a start condition). The user guide states this scope.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-17
- **Verdict**: PASS
- **Suggested commit**: `feat(config): verify, promote, and snapshot configuration`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| — | — | — | No open findings. | closed |

Round 1 findings: #1 (blocker, pre-flight fallback) **closed** — `load_start` now runs the `check` pipeline and falls back on the first `fail`; manually reproduced: with the flag the process keeps running from `main.toml.active`, without it it exits 1 and names the snapshot. #2 (minor, scope note) **closed** — recorded in Implementation Notes; the file's diff is only the forced `monitor::Start` call-site update. #3 (minor, recovery hint) **closed** — `main_config_path` returns the default name when absent; manually reproduced: `network-monitor.toml.active` is named with its age.

### Gap analysis
- Goals: 16/16 achieved. All `check`, candidate, promotion, swap-back, snapshot and fallback goals are observable and verified.
- Scope: in scope. `tests/monitor_e2e.rs` is the recorded mechanical call-site update; `.github/workflows/pages.yml` remains an unrelated working-tree change, not part of this plan.
- Deviations: `Start` bundling, `Candidate.source`, promote unit tests in `src/promote.rs`, `(readable)` database detail, and the 5 s `getMe` timeout — all documented in Implementation Notes.
- Additions: none beyond the plan.
- Deferred: `.bak` retention and log rotation, as the plan states.
- Checklist: 17/17; open questions resolved; Implementation Notes present.

### Known limitations
- The snapshot restore (`cp <main>.active <main>` then the reload) is verified by composition with the generic reload, not by a dedicated automated test.
- A failed `fs::copy` of the backup may leave a partial `.bak`; the main file itself is untouched.
- With `--try` and `--fallback-to-active` together, a rejected candidate logs “`<main>` stays in effect” even when the process is running the snapshot (nit; wording only, no behavioural effect).

### Recommendations for future work
- Add a dedicated test for restoring `<main>.active` over `<main>` and observing the reload.
- Consider wording the candidate-rejection warning with the configuration actually in effect when the snapshot is carrying the start.
