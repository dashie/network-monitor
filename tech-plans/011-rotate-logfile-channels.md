---
title: Rotate a logfile channel by size, and bound the main log in the deployment
date: 2026-11-17
status: draft
type: feature
impact: medium
area: notifications, logfile channel, configuration, container deployment
version_bump: minor
tags: [notifications, logfile, rotation, disk, configuration, deployment]
depends_on: []
branch_recommended: false
---

## Summary

`ADR-0033` rejected rotation inside the tool when a `logfile` channel was a single audit
sink an operator rotated with `logrotate`. That reasoning does not survive several channels:
every `logfile` channel is its own destination with its own file, so a host needs one
`logrotate` stanza per channel, hand-kept in step with the configuration; and a container
has no `logrotate` at all, so the file grows until the volume is full and the monitor loses
its history with it.

This plan adds size-based rotation to the `logfile` channel: two keys per channel,
`max_bytes` (default `0`, which keeps today's append-forever behaviour byte for byte) and
`max_files` (default `5`). A file that would exceed `max_bytes` is renamed to `<filename>.1`
before the line that would cross the limit is written, the older files shift up to
`max_files`, and the oldest is deleted. A rotation that fails is one `WARN` on stderr and the
line is still written: a notification is never lost to housekeeping.

The main log of the application is a different case and is answered where it belongs. The
tool has no log file: its diagnostics go to stderr, so whatever file exists is the one its
supervisor created — `journald`, a shell redirect, or the container engine — and rotation is
theirs. What is missing is that this repository never states it and never bounds it:
`docker-compose.yml` declares no `logging` stanza, so the default `json-file` driver grows
without a limit, and a host run says nothing about the journal, about a redirect, or about the
`copytruncate` a redirect needs. This plan therefore bounds the container case in the compose
project and documents, in one place, which layer bounds which log for all three ways of
running the tool: a terminal, a systemd unit, a container.

## Why

- One `logfile` channel has one file; several channels have several files, each of which needs
  its own external rotation stanza. The configuration states the file, so the rotation of that
  file belongs next to it.
- `logrotate` in `copytruncate` mode has a window between the copy and the truncation where a
  line can be lost, and in `create` mode the tool keeps writing to the renamed inode until it
  reopens. Both are silent failures of an audit trail.
- The provided deployment is a container (`Dockerfile`, `docker-compose.yml`), where no
  external rotator runs beside the process.
- Today the file is "never rotated by the tool" (`FR-29`), so a long-running host with a busy
  target list fills the filesystem with one file, and the failure is discovered when the
  monitoring stops writing.
- The main log is not a file of this tool: `src/logging.rs` writes every diagnostic to stderr,
  there is no `--log-file` flag, and that is the right design — a daemon that writes to one
  stream composes with `journald`, with `docker logs` and with a supervisor. What is wrong is
  the deployment: `docker-compose.yml` declares no `logging` stanza, so the default `json-file`
  driver keeps every line for ever, and `NOTES.md` never tells a `docker run` operator to pass
  `--log-opt max-size`. The repository therefore documents the rule and bounds neither case.
- The reason the tool must not own that file, even though it owns a channel's rotation, is that
  the diagnostic log has to work precisely when the configuration does not: a destination and a
  policy read from the TOML would be unavailable exactly when the TOML is what needs
  explaining. A file the tool opened at startup would also add a way for a broken sink to stop
  the process (the `logfile` channel contract), which is the failure plan `010` exists to
  prevent.

Evidence that nothing exists yet: `logfile` accepts only `filename`, `logger`, `level`, and
`format` ([src/config.rs](../../src/config.rs:1255), `resolve_channel`), the channel keeps no
size, the docs state the file is never rotated (`docs/PRODUCT.md` `FR-29`,
`docs/USER-GUIDE.md` "Notification channels", `docs/NOTIFICATIONS.md`), and no file in the
repository mentions a container log limit.

## Goals

- A `logfile` channel accepts `max_bytes` (default `0`; `0` disables rotation, otherwise
  `1024` to `1073741824`) and `max_files` (default `5`; `1` to `100`); an out-of-range value is
  a configuration error naming `notification[i].max_bytes` / `.max_files` before the first
  probe.
- With `max_bytes = 0` the behaviour is exactly today's: one file, appended, no new file, and
  `--show-config` does not print the two keys.
- With `max_bytes > 0`, a line is written whole: when the current size plus the line would
  exceed the limit, the file is rotated first, so no line is ever split between two files.
- `<filename>.1` is the most recent rotation and `<filename>.<max_files>` the oldest kept; a
  rotation deletes what falls past `max_files`.
- A rotation that cannot complete (rename refused, directory read-only) logs one `WARN` and
  leaves the line in the current file: no event is lost, and the tool does not retry in a loop.
- Rotation happens on a write only. It never happens at startup, on a reload, or during the
  `check` verification of `FR-30`, so inspecting a configuration never rotates a file.
- `--show-config` prints the two keys for a channel that rotates, and the output still parses
  back into the same channel.
- Two channels on one destination stay refused (`INV-14`), so two channels never rotate the
  same file.
- The compose project caps the engine's own log for the container: `logging` with the
  `json-file` driver, `max-size: 10m` and `max-file: "3"`, so the main log is bounded by the
  deployment and not by luck; `NOTES.md` and the developer guide state the equivalent
  `--log-opt` for a bare `docker run`.
- One user-guide section, "Rotate the logs", states the whole map in one place, for every way
  of running the tool: run from a terminal, stderr goes to the terminal and there is nothing to
  rotate; supervised by systemd, `StandardOutput=journal` (the default) leaves rotation to
  `journald` (`SystemMaxUse`, `SystemMaxFileSize`, `MaxRetentionSec`), and `append:` needs
  `logrotate`; redirected with `2>>`, the file is `logrotate`'s and the stanza must use
  `copytruncate`, because the process keeps the descriptor it was given; in a container, the
  driver's `max-size`/`max-file`. A `logfile` channel is `max_bytes`/`max_files` in every case,
  and nothing else writes a file.
- The user guide carries a copy-pasteable `systemd` unit and a `logrotate` stanza, so a host
  operator needs no other document; the guide is distributed standalone, so neither snippet may
  live only in `NOTES.md`.
- The tool still has no log file of its own and gains no `--log-file` flag.

## Non-goals

- No time-based rotation (daily or hourly): the size is the only trigger, so the behaviour does
  not depend on a clock, a time zone, or a process that happened to be down at midnight.
- No compression of rotated files: it adds a codec, a write path, and a failure mode for a file
  a host keeps for days.
- No rotation of the diagnostic log inside the tool: it goes to stderr and belongs to `systemd`,
  `journald`, or the container log driver. This plan bounds that layer in the deployment
  (`docker-compose.yml`, `NOTES.md`, the user guide) and stops there.
- No `--log-file` and no second destination for the diagnostics, even though the mechanism of
  Step 2 would serve it: it would duplicate every line, diverge from `docker logs` and
  `journalctl`, read its own settings from the configuration it may have to report on, and add
  one more way for the process to refuse to start. The scattered-configuration objection is
  answered in the deployment instead: the compose stanza ships in this repository, and on a host
  `StandardOutput=journal` is systemd's default with one host-wide `SystemMaxUse`.
- No CLI flags for the two keys: notification declarations are the documented exception to the
  command-line layer (`ADR-0033`), like `bot_token`, so `INV-2` gains no new obligation.
- No change to `retention_days`, which prunes events in the database and is unrelated to files.
- No automatic recommendation of a value at runtime: the sample configuration and the user
  guide state one, and the process stays silent.

## Affected modules

**Source**

- `src/config.rs` — `FileNotification` gains `max_bytes` and `max_files`; the `logfile` branch of
  `resolve_channel` validates them, adds them to its `reject_foreign` list so they cannot be
  attached to another channel type, and carries them on `ChannelSettings::LogFile`; `to_toml`
  prints them only when the channel rotates.
- `src/notify/logfile.rs` — the channel tracks the current size, decides the rotation for a
  line, performs the shift, reopens the file, and logs the one `WARN` on failure; unit tests
  live here. Plan `010` also touches this file, to add the `verify` method to the same
  `impl Channel`; the two changes are independent, but if both are in flight, land `010` first
  and keep rotation out of `verify`.
- `src/notify/mod.rs` *(maybe)* — only if the `WARN` needs the notifier's logger name; the
  transport-failure path already exists.
- `src/check.rs` *(maybe)* — a `warn` line for a `logfile` channel that appends without
  rotation, if `FR-30` of plan `010` has landed; see Open questions.

**Tests**

- `src/notify/logfile.rs` — unit tests: no rotation at `0`; rotation just before the line that
  would cross the limit; a single line larger than the limit is written whole and the next line
  rotates; the shift produces `.1` … `.max_files` and deletes the oldest; a read-only directory
  leaves the line written and logs once; a rotated file keeps mode `0600`.
- `tests/notify_channels.rs` — an end-to-end run of the binary with a small `max_bytes` proves
  the files appear while the process runs, and that `--show-config` round-trips the keys.

**Documentation / configuration**

- `docs/PRODUCT.md` — amend `FR-29` with the two keys, the limit, the shift, and the failure
  behaviour.
- `docs/ADR.md` — `ADR-0035 — Rotate a log file channel by size, and only on a write`, amending
  the rotation alternative of `ADR-0033`, plus its index entry.
- `docs/ARCHITECTURE.md` — the notifications component and the guarantees paragraph of the
  channel: rotation is internal state of the channel, no other component sees it.
- `docs/USER-GUIDE.md` — the `logfile` key table gains both rows with type, default, range, and
  effect; the "Limits and deliberate choices" entry that states the file is never rotated is
  rewritten; a new "Rotate the logs" section under "Operations" states the whole map, for a
  terminal run, a systemd unit, a redirected stream, and a container, with a copy-pasteable unit
  and `logrotate` stanza (the latter with `copytruncate` and the reason), and states what a
  `tail -F` reader sees when a channel rotates.
- `docs/NOTIFICATIONS.md` — replace "is never rotated by the tool" with the two keys and what
  happens to the file at the limit.
- `docker-compose.yml` — one `logging` block on the service: driver `json-file`,
  `max-size: 10m`, `max-file: "3"`, with a comment saying why the main log is bounded here and
  not by the tool.
- `NOTES.md` — the bare `docker run` recipe gains `--log-opt max-size=10m --log-opt
  max-file=3`, in the same section that already lists the flags of a real deployment.
- `docs/DEVELOPER-GUIDE.md` — the "Without compose" run example carries the same two options,
  and "Adding a notification channel" notes that a channel writing to a file owns its rotation.
- `docs/GLOSSARY.md` *(maybe)* — "Rotated log file": one file the channel renames at the size
  limit, the suffix being its age.
- `network-monitor.sample.toml` and `docker/config.toml` *(maybe)* — one commented example of a
  bounded `logfile` channel, so the sample states a value an operator can copy.

## Architecture impact

The `logfile` channel keeps one responsibility — render one message and append it — and gains
one piece of private state, the current size, which no other component reads. No new module,
no new dependency, no change to the `Channel` contract (unless plan `010` has landed, in which
case rotation must stay out of `verify`). `docs/ARCHITECTURE.md` must state that rotation is
invisible outside the channel and that a rotation never splits or loses a line.

## Decision-record impact

A new decision record is required: `ADR-0035`. `ADR-0033` rejected rotation inside the tool;
this plan reverses that decision for the `logfile` channel and must say why the earlier
reasoning no longer holds (several sinks, containers without `logrotate`, the copy/truncate
window), why the trigger is size and not time, why the default is off, and why there is no
compression. It also records the other half: the diagnostic log stays on stderr and gets no
`--log-file` flag, because one stream is what composes with `journald` and with the container
engine, because its destination must not depend on a configuration that may be the thing that
is broken, and because a sink the process opens at startup is one more way for it to refuse to
start. The record states the rejected alternative — reuse `max_bytes`/`max_files` for a log file
of the tool's own — and why the one-place-to-configure argument is answered by shipping the
compose stanza and by journald's host-wide bound, not by a second log destination.

## Proposed solution

### Step 1 — Add the two keys and validate them

In `src/config.rs`: `FileNotification` gains `max_bytes: Option<u64>` and `max_files:
Option<u64>`; the `logfile` branch of `resolve_channel` adds both to `reject_foreign`, applies
the defaults (`0`, `5`), validates `max_bytes == 0 || (1024..=1_073_741_824).contains(..)` and
`(1..=100).contains(max_files)`, and stores them on the resolved `LogFile`; the serialiser in
`Config::to_toml` prints `max_bytes` and `max_files` only when `max_bytes > 0`.

**Verify:** `cargo test config` passes with tests for the defaults, both refusals with the
`notification[i].key` in the message, the acceptance of `0` and of the two bounds, and a
`--show-config` round-trip that parses back into the same channel.

### Step 2 — Rotate on a write

In `src/notify/logfile.rs`: the channel keeps `size: u64`, set from the file's metadata in
`open` and incremented by the bytes written in `send`. Before writing, when `max_bytes > 0`,
`size > 0`, and `size + line_bytes + 1 > max_bytes`, `rotate()` first: rename
`<filename>.<i>` to `<filename>.<i+1>` for `i` from `max_files - 1` down to `1` (a missing file
is not an error), delete `<filename>.<max_files>`, rename `<filename>` to `<filename>.1`, reopen
the file with the same `create`/`append`/`0600` options, and reset `size` to `0`. A rename that
fails returns the reason, which `send` turns into one `logging::warn` and then continues with
the write to the current file: the line is never lost and the loop never retries.

**Verify:** `cargo test logfile` passes with the unit tests listed under Tests, including a
`max_bytes` smaller than one line (the line is written whole, the next line rotates) and a
read-only directory (line written, one `WARN`, no panic, no partial file).

### Step 3 — Prove it end to end and update the documentation

Add the `tests/notify_channels.rs` case and write the documentation listed under Affected
modules: `FR-29`, `ADR-0035` with its index entry, the architecture paragraph, the user-guide
key table and limits, `docs/NOTIFICATIONS.md`, the glossary term if used, and the commented
example in the sample configuration.

**Verify:** `cargo test --test notify_channels` passes; every documented key exists in
`src/config.rs` with the same default (`grep -n "max_bytes\|max_files" src/config.rs`) and in
the user-guide table; `network-monitor.sample.toml` still parses (`./target/debug/network-monitor
--config network-monitor.sample.toml --show-config` exits `0`).

### Step 4 — Bound the main log in the deployment

The tool keeps writing its diagnostics to stderr; nothing in this step changes `src/logging.rs`.
In `docker-compose.yml`, add to the `network-monitor` service:

```yaml
    # The main log is the engine's file, not the tool's: stdout and stderr go to the
    # json-file driver, whose default keeps every line for ever. Bound it here, where
    # the deployment owns it; `logfile` channels are bounded by max_bytes/max_files.
    logging:
      driver: json-file
      options:
        max-size: "10m"
        max-file: "3"
```

Then state the same bound for a bare `docker run` in `NOTES.md` and in the "Without compose"
example of `docs/DEVELOPER-GUIDE.md` (`--log-opt max-size=10m --log-opt max-file=3`).

The host case gets the same treatment in the user guide, because a host run is where the
question is easiest to get wrong:

```ini
# /etc/systemd/system/network-monitor.service
[Service]
ExecStart=/usr/local/bin/network-monitor monitor --config /etc/network-monitor/config.toml
# stderr goes to the journal, which owns rotation:
#   journald.conf: SystemMaxUse=500M  SystemMaxFileSize=50M
StandardOutput=journal
StandardError=journal
```

```text
# /etc/logrotate.d/network-monitor  — only for `StandardOutput=append:` or a shell redirect
/var/log/network-monitor/monitor.log {
    size 10M
    rotate 3
    compress
    copytruncate   # the process keeps the descriptor it was given: `create` would
                   # leave it writing to the renamed inode
    missingok
    notifempty
}
```

Write the user-guide section "Rotate the logs" described under Affected modules with these two
snippets, the container settings, and the rule that a `logfile` channel is bounded by its own
two keys in every one of those cases.

**Verify:** `docker compose config` (or `docker-compose config`) prints the `logging` block of
the service and the file still round-trips through a YAML parser; the two documented `--log-opt`
flags appear in `NOTES.md`, the developer guide, and the user guide; the user-guide section names
all four ways of running the tool (terminal, redirect, systemd, container) and carries both
snippets, and `grep -n "copytruncate" docs/USER-GUIDE.md` finds the stanza with its reason.

### Step 5 — Bump the version and run the quality gate

Raise the minor version by one from whatever `Cargo.toml` states at implementation time
(`0.9.0` → `0.10.0` if plan `010` landed first, `0.8.0` → `0.9.0` otherwise) and let `cargo`
update `Cargo.lock`.

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
passes with no new warnings, and `git diff --stat Cargo.lock` shows only the crate's own
version.

## Security considerations

- A rotated file carries the same content as the file it came from — target addresses, labels,
  causes — so it is created by the rename of a `0600` file and keeps that mode; a newly created
  file after a rotation is again opened with `0600` on Unix.
- The rename pattern never leaves the directory of the configured file: suffixes are appended to
  the configured name, and a symbolic link is followed exactly as today's `open` follows it.
- Rotation does not write the reason of a failed rotation into the log file itself (that would
  feed a sink that is failing); it goes to stderr, as every transport failure already does.
- `check` (`FR-30`) and `open` never rotate, so verifying a configuration cannot destroy the
  file an operator is reading.

## Verification plan

- **Automated:** `cargo test logfile`, `cargo test config`, `cargo test --test notify_channels`.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo
  test`.
- **Manual:** with a scratch configuration and `max_bytes = 4096`, `max_files = 3`, run
  `monitor` against a flapping target and confirm `<file>`, `<file>.1`, `<file>.2`, `<file>.3`
  appear and no file exceeds the limit by more than one line; then make the directory read-only
  and confirm one `WARN`, the line still written, and monitoring unaffected. On the NAS, after
  `compose up`, `docker inspect --format '{{.HostConfig.LogConfig}}' network-monitor` states the
  `json-file` driver with the two limits, and `du -h $(docker inspect --format
  '{{.LogPath}}' network-monitor)` stops growing at 10 MiB while the service keeps logging.
- **Rollback:** set `max_bytes = 0` (or delete the key) and the behaviour returns to the one
  documented today; remove the `logging` block and the engine default returns; the code rollback
  is one revert, as no other component depends on the two keys or on the compose stanza.

## Open questions

- [ ] Should `max_bytes` default to a value instead of `0`? Best guess: keep `0` — enabling
  rotation silently would rename a file an external shipper may be watching, and the sample
  configuration suggests `10485760` for an operator who wants it.
- [ ] Should `check` (`FR-30`, plan `010`) report a `warn` line for a `logfile` channel with
  `max_bytes = 0`? Best guess: yes, once plan `010` has landed, because that is exactly the
  moment to notice a sink that grows without a bound; it is one line in `check` and one test.
- [ ] Should the repository ship `deploy/network-monitor.service` and
  `deploy/network-monitor.logrotate` instead of keeping both snippets inline in the guide? Best
  guess: inline for now — the user guide is distributed standalone and must carry them anyway,
  and a `deploy/` folder is a follow-up if host installs become a supported shape.
- [ ] Compress the rotated file? Best guess: no, for the reason in Non-goals.
- [ ] Should the suffix carry a timestamp (`<file>.20261117T031500Z`)? Best guess: no — `.1` is
  ordered, needs no clock, and matches what `logrotate` produces by default.

## Implementation checklist

- [ ] Step 1 — `max_bytes` and `max_files` in configuration and in `--show-config`
- [ ] Step 2 — size-based rotation in the `logfile` channel
- [ ] Step 3 — end-to-end test and documentation (`PRODUCT`, `ADR`, `ARCHITECTURE`, `USER-GUIDE`, `NOTIFICATIONS`, sample configuration)
- [ ] Step 4 — bound the main log: compose driver, `docker run` flags, systemd unit, redirect + `logrotate`
- [ ] Step 5 — version bump and quality gate
- [ ] All open questions resolved
- [ ] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [ ] Documentation listed under Affected modules is updated
- [ ] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [ ] `## Implementation Notes` added or updated in this plan
