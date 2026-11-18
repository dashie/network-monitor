---
title: Record monitor downtime as unknown and show the indeterminate period
date: 2026-11-17
status: draft
type: feature
impact: high
area: store, monitor, web, query, import, notify
version_bump: minor
tags: [monitoring, history, schema, web]
depends_on: []
branch_recommended: true
---

## Summary

The monitor will keep a one-row liveness record in SQLite and use it at startup to
detect that the process was not running longer than a configurable threshold. When the
gap is long enough, it will write an `up|down -> unknown` transition at the last instant
it was alive for every configured target with a known state, so the restart's first probe
writes `unknown -> up|down`. The page, the JSON API and `query state` will also report a
target as `unknown` while a running reader can prove the monitor is stale. The same
`unknown` state and grey band are reused; no new state token is introduced. A fault that
spans a gap ends at the gap start, because the existing fault rule closes at the next
event; if the target is still down after the restart, a new fault starts. The unobserved
interval is therefore never counted as down time.

The feature is opt-out: `gap_after_ms` defaults to `60000`, `0` disables gap recording
and live staleness. Existing short restarts, reloads, probes, retention and the
notification channel model are unchanged.

## Why

Today `Supervisor::start_workers` loads the stored state from `Store::load_states` and the
first probe is compared against it. Nothing records when the process stopped or how long
it was absent. The consequences are visible in the current tests and reader code:

- A target that was `up` and is still `up` after a long absence produces no event, and
  `timeline_segments` paints the whole gap `up`.
- A target that was `up` and is `down` after a long absence records the drop at restart,
  losing the real start of the outage.
- A target that was `down` and recovered during the absence records the recovery at
  restart, overstating down time.
- A target that failed and recovered entirely while the monitor was off leaves no trace.
- `TargetState` has only `Unknown`, `Up` and `Down`; `Unknown` is used only before the
  first event, and the page already renders it as a grey band.

`docs/HISTORY-IMPORT.md` documents the cost of this defect in production: an imported
stale `down` turned 35 unmeasured hours into down time. The existing repair is a manual
SQL insert of a `down -> unknown` row. That repair works because the fault query closes a
fault at the next event of any state. This plan makes the binary write the gap
automatically, updates `import` so the marker round-trips, and keeps that split semantics:
the synthetic `unknown` event closes the old fault, so the unobserved hours are not
counted as down time. The runbook is simplified rather than reversed.

## Goals

- After a monitor absence longer than `gap_after_ms`, the next start inserts
  `up|down -> unknown` at the last alive instant for every configured target whose stored
  state is `up` or `down`, and the first probe records `unknown -> up|down`.
- `gap_after_ms` is a global key and `--gap-after-ms` flag; the default is `60000`,
  `0` disables the feature, and a non-zero value below `5000` is refused.
- `--show-config` prints the effective `gap_after_ms`, and the command-line value wins
  over the file as it does for every other key.
- A `serve` or `query` reader reports a target as `unknown` when the liveness record is
  older than `gap_after_ms`; the timeline tail from the last alive instant to the window
  end is `unknown`; up-run statistics break at that boundary; the detail page's current
  state is `unknown`.
- A fault that spans a gap ends at the gap start; if the target is still down after the
  restart, a new fault starts. The gap is never counted as down time, and the timeline
  shows it as `unknown`.
- `import` accepts CSV rows whose `state` is `unknown`, so an export/import round trip
  preserves a gap.
- The startup notification states the gap span and how many targets were marked
  unknown; the Telegram text, the log summary and the webhook JSON all carry it.
- Existing behaviour is preserved: a restart shorter than the threshold continues from
  the stored state with no new event; reloads do not create a gap; a local probe failure
  still changes nothing; no target ever has two consecutive events with the same state.

## Non-goals

- A distinct `indeterminate` state token. `unknown` is reused because it is already
  stored, rendered grey, exported, imported and understood by older readers.
- A historical `gaps` table or a read-time gap overlay. Gap history is represented by
  the `unknown` events themselves.
- Retroactive gap detection for a database that has no liveness record (for example the
  first start after upgrading from schema version 3). No previous liveness exists, so no
  gap can be claimed.
- Gap detection on a configuration reload. A reload restarts workers but the process
  kept running, so no gap exists.
- Multiple `monitor` processes on one database. The liveness row is single-writer and
  follows the existing single-writer assumption.
- A new probe method, a new notification transport, a down-time column on the timeline,
  or a change to the simulation.
- One incident across a monitor gap. The change deliberately splits a fault at the gap,
  which increases the fault count when the target is still down after the restart, in
  exchange for a lower-bound down time. The docs state the split and its consequence.

## Affected modules

**Source**
- `src/config.rs` — `FileConfig.gap_after_ms`, `Overrides.gap_after_ms`, `Config.gap_after`, defaults, validation, `to_toml`, unit tests.
- `src/cli.rs` — global `--gap-after-ms`, `uses_configuration`, `overrides`, parsing tests.
- `src/store/schema.rs` — schema version 4, `monitor_liveness` table, comments.
- `src/store/mod.rs` — `has_liveness`, `Liveness`, `MonitorGap`, `begin_run`, `heartbeat`, `end_run`, `liveness`, `stale_from`, `states_at`, `find_target_at`, timeline tail, split-fault tests, migration tests.
- `src/monitor/mod.rs` — begin the run before workers, heartbeat in the supervisor loop, end the run at stop, attach the gap to the startup message, reload handling, tests.
- `src/notify/message.rs` — `GapSummary`, optional gap on `Event::Startup`, `with_gap`, text/summary/JSON rendering, tests.
- `src/import.rs` — accept `unknown` rows, unit tests.
- `src/history.rs` — no signature change; confirm the re-exported contract still compiles. *(maybe)*
- `src/query/mod.rs` — no code change expected; `Store::states` carries the overlay. *(maybe)*
- `src/web/mod.rs`, `src/web/api/targets.rs`, `src/web/detail.rs` — no code change expected; the overlay is in the store. *(maybe)*
- `src/check.rs` — no code change expected; `Config::resolve` validates the new key. *(maybe)*
- `src/init.rs` — optional commented `gap_after_ms` line in the scaffold; no effective change. *(maybe)*
- `uptime-monitor.sample.toml` — document the new key.

**Tests**
- `tests/config_precedence.rs` — file/flag/default precedence and the `0`/minimum cases.
- `tests/cli.rs` — `--show-config` contains `gap_after_ms`, the flag is accepted, and `init`/`manual` reject it as a configuration flag.
- `src/store/mod.rs` — migration from v3, begin/heartbeat/end, gap events, live overlay, split-fault regression tests, read-only v3.
- `tests/store.rs` — reader/writer concurrency remains green with the liveness row.
- `tests/monitor_e2e.rs` — long restart writes the gap, short restart does not, stop writes `stopped_at_ms`.
- `tests/notify.rs` — startup gap text/summary/JSON.
- `tests/web.rs` — current state unknown while stale, grey timeline tail, detail page.
- `tests/import.rs`, `src/import.rs` — unknown rows round-trip and are no longer skipped.
- `tests/reload.rs` — a reloaded `gap_after_ms` is visible to readers and does not create a gap.

**Documentation / configuration**
- `docs/PRODUCT.md` — `FR-5`, `FR-6`, `FR-17`, `FR-25`, `BR-2`, new `FR-36` and `BR-12`, assumptions and non-functional note.
- `docs/ARCHITECTURE.md` — data model (three tables), component contracts, runtime start/stop, `INV-10`, new `INV-16`.
- `docs/ADR.md` — new ADR-0041 and index row.
- `docs/GLOSSARY.md` — **Monitor gap**, **Liveness record**, updated **Unknown state**, **Fault**, **Event database**, **State transition**.
- `docs/USER-GUIDE.md` — **What the tool does**, **Global keys**, **Command line**, **The web page**, **Notifications**, **Upgrade**, **Troubleshooting**, **Limits and deliberate choices**.
- `docs/HISTORY-IMPORT.md` — replace the manual repair section with the built-in gap behaviour; the gap is not down time.
- `docs/NOTIFICATIONS.md` — startup `gap` member, first observation after a gap, verification table.
- `docs/DEVELOPER-GUIDE.md` — no change expected; the existing “Changing the schema” checklist already covers the new table. *(maybe)*
- `site/index.html` — one feature/FAQ sentence that a restart gap is shown as `unknown`. *(maybe)*

## Architecture impact

The store gains a third table, `monitor_liveness`, with exactly one row. It is owned by
`store` like the other two tables; no other component issues SQL. The monitor writes it
before the first probe, updates it on a bounded heartbeat, and closes it on a clean stop.
The first probe still uses the existing transition decision; the only difference is that
`previous` is `unknown` after a long gap, so the existing rule records the first verdict.

The read path gains one derived rule: when `monitor_liveness` is present, enabled and
stale, `Store::states_at`, `Store::find_target_at` and `Store::timeline` turn the tail
after the last alive instant into `unknown`. The `History` trait keeps its signature, so
the page, API, detail page and simulation contract do not change shape. Older read-only
opens of a version-3 database have no table and keep their current behaviour.

The fault query and `fault_episodes` keep their current rule: a fault ends at the first
later event of any state. The synthetic `unknown` event is therefore the end of a fault
that was open when the monitor stopped. If the first probe after the gap is still `down`,
the `unknown -> down` event starts a new fault. This is the selected split semantics; it
is the smallest change and preserves the lower-bound down time documented in
`docs/HISTORY-IMPORT.md`.

`docs/ARCHITECTURE.md` must update the data model, the `store`/`monitor`/`notify`
component contracts, the runtime start/stop view, `INV-10`, and add an invariant that a
target never crosses a monitor gap without an `unknown` transition.

## Decision-record impact

A new ADR is required. The change alters the meaning of `unknown` (from “before the first
verdict” to “not observed”), adds a schema version, and writes events that are not direct
verdicts. It also chooses the split-fault rule, so the unobserved interval never counts as
down time; the alternative of one incident with an upper-bound duration is rejected.
Record it as **ADR-0041 — Record monitor downtime as unknown instead of carrying the last
state**, with the alternatives (gaps table and read-time overlay, clean-stop marker only,
one incident with an upper-bound duration, a separate indeterminate state) and their
consequences.

## Proposed solution

### Step 1 — Add `gap_after_ms` to the configuration and the command line

Add the global key to `FileConfig`, `Overrides` and `Config` in `src/config.rs`. Add
`DEFAULT_GAP_AFTER_MS: u64 = 60_000` and `MIN_GAP_AFTER_MS: u64 = 5_000`. Resolve the
value with the existing precedence, accept `0` as “disabled”, and refuse a non-zero value
below the minimum with a `ConfigError::Invalid` naming `gap_after_ms`. Also refuse a value
above `i64::MAX` so the SQLite `INTEGER` column cannot overflow. Store the effective value
as `Config.gap_after: Duration`. Extend `Config::to_toml` so `--show-config` prints
`gap_after_ms = 60000` beside the other globals.

Add the mirrored flag in `src/cli.rs`:

```rust
/// Minimum monitor downtime recorded as unknown; 0 disables
#[arg(long, global = true, value_name = "MILLISECONDS")]
pub gap_after_ms: Option<u64>,
```

Include it in `Cli::uses_configuration` (so `init` and `manual` reject it) and in
`Cli::overrides`. Update `tests/config_precedence.rs` and `tests/cli.rs` for the default,
the file value, the flag precedence, `0`, the minimum rejection, and the `--show-config`
line.

The configuration file gains the key in this context:

```toml
# uptime-monitor.sample.toml
interval_ms = 5000
timeout_ms = 2000

# Minimum monitor downtime recorded as unknown. The last known state is closed as
# unknown and the first probe after the gap records the observed state. 0 disables.
gap_after_ms = 60000

database = "./tmp/uptime-monitor.db"
retention_days = 0
```

**Verify:** `cargo test config` and `cargo test --test config_precedence --test cli` pass;
`uptime-monitor --show-config` prints `gap_after_ms = 60000`, and
`uptime-monitor --gap-after-ms 1000 --show-config` exits non-zero naming `gap_after_ms`.

### Step 2 — Add the schema-version-4 liveness table and the store API

Bump `SCHEMA_VERSION` to `4` in `src/store/schema.rs` and add a `LIVENESS_SCHEMA: i64 = 4`
constant for the read-only open. Add this table to `TABLES`:

```sql
CREATE TABLE IF NOT EXISTS monitor_liveness (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    started_at_ms INTEGER NOT NULL,
    last_seen_ms  INTEGER NOT NULL,
    gap_after_ms  INTEGER NOT NULL,
    stopped_at_ms INTEGER             -- null while running, set by a clean stop
);
```

The existing `migrate` already executes `TABLES` for an older file and updates the
version, so no `ALTER` step is needed; a version-3 file gains the empty table on its next
writable open. `Store` gains `has_liveness` (`true` for a writable open,
`version >= LIVENESS_SCHEMA` for a read-only open). Add:

```rust
pub struct Liveness {
    pub started_at_ms: i64,
    pub last_seen_ms: i64,
    pub gap_after_ms: u64,
    pub stopped_at_ms: Option<i64>,
}

pub struct MonitorGap {
    pub from_ms: i64,
    pub to_ms: i64,
    pub targets: usize,
}
```

`Store::begin_run(&mut self, now_ms, gap_after_ms, targets: &[String])` runs one
transaction:

1. Read the previous `monitor_liveness` row, if any.
2. Compute `alive_until = stopped_at_ms.unwrap_or(last_seen_ms)` and
   `gap_start = alive_until.max(newest_event + 1)` when `newest_event` is not already
   earlier than `alive_until`; otherwise `gap_start = alive_until`. The strict `+1`
   matters because `FAULTS_SELECT` and `fault_episodes` compare `occurred_at_ms` with
   `>`; an unknown event at the same millisecond would not close the fault in the split
   rule.
3. If a previous row exists, `gap_after_ms > 0`, `gap_start < now_ms`, and
   `now_ms - alive_until > gap_after_ms as i64`, insert an `unknown` event for every key
   in `targets` whose `current_state` is `up` or `down`:
   `previous_state = current_state`, `state = 'unknown'`, `occurred_at_ms = gap_start`,
   `rtt_us = NULL`, `cause = NULL`, `cause_message = NULL`, preserving the stored
   `interface`; then set that row's `current_state = 'unknown'`,
   `last_change_at_ms = gap_start`, `last_rtt_us = NULL`. Count the rows.
4. Upsert the one row with `started_at_ms = now_ms`, `last_seen_ms = now_ms`,
   `gap_after_ms`, `stopped_at_ms = NULL`.
5. Return `Some(MonitorGap { from_ms: gap_start, to_ms: now_ms, targets })` when a gap
   was recorded, otherwise `None`.

`Store::heartbeat(&self, now_ms, gap_after_ms)` updates `last_seen_ms` and
`gap_after_ms` (upserting the row if it is missing, which covers a reload that enables
the feature). `Store::end_run(&self, now_ms)` sets `last_seen_ms` and `stopped_at_ms` to
the same instant. `Store::liveness(&self)` returns `None` when `has_liveness` is false or
no row exists.

Add store tests for: a new database (one row, no gap), a previous row older than the
threshold (unknown events, projection, one gap, no duplicate state), a gap below the
threshold (no events), `gap_after_ms = 0` (no events), a clean stop using
`stopped_at_ms`, a crash using `last_seen_ms`, placing the marker strictly after the
newest event, and a version-3 read-only fixture (`liveness()` is `None`, no write
occurs).

**Verify:** `cargo test store` passes, including the new `begin_run`, `heartbeat`,
`end_run`, and migration tests; a read-only open of the version-3 fixture leaves its
`schema_meta.version` at `3`.

### Step 3 — Begin, heartbeat and end the run in the monitor

In `src/monitor/mod.rs`, before `Supervisor::start_workers` and after
`config.require_targets`, build `let keys: Vec<String> = config.targets.iter().map(Target::key).collect();`
and call:

```rust
let gap = {
    let mut store = store.lock().map_err(|_| RunError::Lock)?;
    store.begin_run(now_ms(), config.gap_after.as_millis() as u64, &keys)?
};
```

A `StoreError` is fatal (`RunError::Store`): without the liveness row the feature cannot
work. A returned gap is logged as one `WARN` naming the start, the end and the number of
targets marked unknown, and is attached to the startup message in Step 4.

In the supervisor loop, keep a `last_heartbeat: Instant`. On each pass, compute
`heartbeat_period` from the current `supervisor.config.gap_after`:

```rust
fn heartbeat_period(gap_after: Duration) -> Option<Duration> {
    if gap_after.is_zero() {
        None
    } else {
        Some((gap_after / 4).max(Duration::from_secs(1)).min(Duration::from_secs(15)))
    }
}
```

When the period has elapsed, call `store.lock()?.heartbeat(now_ms(), gap_after_ms)`. A
heartbeat failure is a `WARN` and is retried on the next pass; it does not stop the
monitor. This bounds the crash-triggered gap start error to one heartbeat period while
keeping a stable monitor’s writes at most one row update every 15 seconds, and no
heartbeat at all when `gap_after_ms = 0`.

In `Supervisor::swap`, after the new configuration is installed, call
`store.lock()?.heartbeat(now_ms(), next.gap_after.as_millis() as u64)` so a reload that
changes the threshold is visible to readers at once and a reload from `0` to a non-zero
value establishes a fresh `last_seen_ms` instead of looking stale. A reload never calls
`begin_run` and therefore never records a gap.

After `supervisor.stop_workers()` and the retention join, call
`store.lock()?.end_run(now_ms())`. A failure is one `WARN` and does not replace the
monitor’s existing exit status; the next start falls back to `last_seen_ms`.

Extend `tests/monitor_e2e.rs` with a long-restart case (age the row with a direct SQL
update or a second writable open, then start again) and assert the event chain
`up -> unknown -> down`, and with a clean-stop case asserting `stopped_at_ms` is set.

**Verify:** `cargo test --test monitor_e2e` and `cargo test monitor::` pass; the existing
`a_restart_continues_from_the_stored_state` still sees two events and no `unknown`.

### Step 4 — Carry the gap in the startup notification

In `src/notify/message.rs` add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapSummary {
    pub from_ms: i64,
    pub to_ms: i64,
    pub targets: usize,
}
```

Add `gap: Option<GapSummary>` to `Event::Startup`. Keep `Message::startup(...)` setting
`gap: None` so every existing call site keeps its current output. Add
`Message::with_gap(mut self, gap: Option<GapSummary>) -> Self`, which sets the field when
the message is a startup message and returns it unchanged otherwise.

Render the gap only when it is present:

- `text()` adds one line after `Web:`:
  `Gap: <from> to <to> (<targets> target(s) marked unknown)`.
- `summary()` appends ` gap=<from>..<to> targets=<targets>` on the same line.
- `json()` adds `,"gap":{"from":<rfc3339>,"to":<rfc3339>,"targets":<n>}` inside the
  existing `startup` object. Schema stays `1`; a startup without a gap keeps today’s
  exact document.

In `src/monitor/mod.rs`, after `begin_run` returns, map the `MonitorGap` to a
`GapSummary` and attach it to the `startup` message before it is sent:

```rust
let startup = startup.map(|message| {
    message.with_gap(gap.as_ref().map(|gap| GapSummary {
        from_ms: gap.from_ms,
        to_ms: gap.to_ms,
        targets: gap.targets,
    }))
});
```

Add tests in `src/notify/message.rs` for the text, summary and JSON with a gap, and
assert the existing no-gap fixture is byte-for-byte unchanged. Extend
`tests/notify.rs` if the startup message is exercised there.

**Verify:** `cargo test notify::message` and `cargo test --test notify` pass; the existing
startup text/JSON tests still match exactly.

### Step 5 — Turn a stale monitor into live `unknown` in the store readers

Add to `src/store/mod.rs`:

```rust
pub fn states_at(&self, now_ms: i64) -> Result<Vec<TargetStateRow>, StoreError>;
pub fn find_target_at(&self, needle: &str, now_ms: i64) -> Result<Option<TargetStateRow>, StoreError>;
fn stale_from(&self, at_ms: i64) -> Result<Option<i64>, StoreError>;
```

`stale_from` returns `None` when `has_liveness` is false, no row exists, or
`gap_after_ms == 0`. Otherwise it computes
`alive_until = stopped_at_ms.unwrap_or(last_seen_ms)` and returns `Some(alive_until)` only
when `at_ms.saturating_sub(alive_until) > gap_after_ms as i64`. `states()` becomes
`self.states_at(now_ms())`; `find_target()` becomes `self.find_target_at(needle, now_ms())`.

When `stale_from` is `Some`, `states_at` and `find_target_at` set every row whose state is
not already `unknown` to `Unknown`, set `last_change_at_ms = alive_until` and
`last_rtt_us = None`. The target key, interface, label and `first_seen_at_ms` stay.

`Store::timeline` uses `to_ms` as the reader instant. When `stale_from(to_ms)` is `Some`,
apply a pure helper that truncates every segment at `alive_until` and appends one
`Unknown` segment from `alive_until` to `to_ms` (or from `from_ms` when `alive_until` is
before the window). The helper skips zero-length segments and leaves a non-stale timeline
untouched. This makes the page, the API and the detail chart see the same tail without
changing the `History` trait; the daily down-time buckets follow the split rule in Step 6
and do not count the gap as down.

Add store tests for: a stale row over a window that starts before the last alive instant;
a stale row whose last alive instant is before the window; a non-stale row; a
`gap_after_ms = 0` row; a version-3 read-only file; and `states_at`/`find_target_at`
setting `unknown` and the last-change instant. Add a `tests/web.rs` case that serves a
stale database and asserts the grey tail and the `unknown` current state.

**Verify:** `cargo test store` and `cargo test --test web --test store` pass; a store
without a liveness row returns exactly the rows it returned before this step.

### Step 6 — Verify that faults split at the gap and down time stays a lower bound

No change to `FAULTS_SELECT` or `fault_episodes` is required. Both already close a fault
at the first later event of any state, including `unknown`. The synthetic gap event is
therefore the end of the fault that was open when the monitor stopped. If the target is
still `down` after the restart, the first probe writes `unknown -> down`, which starts a
new fault at the restart. The gap remains `unknown` on the timeline and is not down time.

This is the selected split semantics, and it preserves the repair described in
`docs/HISTORY-IMPORT.md`. The cost is an extra fault when a target is down before and
after the gap; the docs state that consequence.

Add regression tests to `src/store/mod.rs` for the exact chains:

- `down -> unknown -> up`: one fault, ending at the `unknown`, not at the `up`.
- `down -> unknown -> down -> up`: two faults, the first ending at the `unknown`, the
  second starting at the second `down`.
- `down -> unknown -> down`: two faults, the first closed and the second open.
- `up -> unknown -> down -> up`: one fault, started by the `down` after the gap.
- `down -> up -> unknown -> down -> up`: two faults, unchanged by the gap.

Extend the SQL-reader/pure-helper drift guard with these chains; both already agree on
the current rule, so the test proves the split behaviour rather than changing it. Update
`daily_drops` expectations so the gap is not counted in either fault’s down time.

**Verify:** `cargo test store` and `cargo test --test store --test web` pass; the
SQL-reader/pure-helper drift guard is green for every new sequence and no production
fault code was changed.

### Step 7 — Let `import` accept `unknown` rows

In `src/import.rs`, remove the `if state == TargetState::Unknown { return None; }` branch
from `Row::parse`. An unknown row is a real gap marker: its `previous_state` is worked out
with the other rows by `normalize_history`, it is not a repeated state, and the target
projection can legitimately be `unknown` after the import.

Update `src/import.rs`’s unit test `broken_rows_are_rejected`: remove the unknown row
from the rejected list and add a test that parses it. Update `tests/import.rs` with an
archive containing `down, unknown, down, up` and assert the chain keeps the unknown row
and one gap; add an export/import round-trip assertion if the existing round-trip test
does not already cover every state.

`docs/HISTORY-IMPORT.md` changes in Step 8. The export already writes every event, so no
change is needed in `src/web/export.rs`.

**Verify:** `cargo test --test import` and `cargo test import::` pass; an archive with an
`unknown` row imports without incrementing `skipped`.

### Step 8 — Update the documentation and the sample configuration

Update these documents in the same change:

- `docs/USER-GUIDE.md`:
  - **What the tool does** states that a restart gap is shown as `unknown`.
  - **Global keys** adds `gap_after_ms` with default `60000`, meaning, `0` disables and
    the `5000` minimum.
  - **Command line** adds `--gap-after-ms`.
  - **The web page** explains that a stale monitor shows the current state and the
    timeline tail as `unknown`, that a fault ends at the gap start, and that a new fault
    starts after the restart when the target is still down.
  - **Notifications** shows the optional `Gap:` line in the startup text and the `gap`
    member in the webhook JSON.
  - **Upgrade** states that schema version 4 adds `monitor_liveness`, that an older
    binary refuses a v4 database, and that the pre-upgrade backup is the rollback.
  - **Troubleshooting** explains a grey band after a restart, that a fault ends at the
    gap start, that a target still down after the restart starts a new fault, and that
    the gap is never counted as down time.
  - **Limits and deliberate choices** states the heartbeat write cost, the `0` opt-out,
    and the extra fault a restart can create when a target is down before and after the
    gap.
- `docs/PRODUCT.md`: update `FR-5`, `FR-6`, `FR-17`, `FR-25`, `BR-2`; add `FR-36` for
  liveness/gap detection and live staleness, and `BR-12` for the split rule: a fault ends
  at the gap start, a new fault starts after the restart, and the gap is not down time.
  Add the feature to **In scope**, and a note to the assumptions and non-functional table
  for the liveness heartbeat.
- `docs/ARCHITECTURE.md`: **Data model** says three tables and shows
  `monitor_liveness`; `events.state` now includes `'unknown'`; add the store,
  monitor and notify contracts; update the runtime start/stop view; update `INV-10` and
  add `INV-16` (“no target state crosses a monitor gap without an `unknown` transition;
  the liveness row is written before the first probe”); update the migration paragraph to
  version 4.
- `docs/ADR.md`: add ADR-0041 to the index and the records, status `accepted`, date
  `2026-11-17`, with the context, decision, consequences and the alternatives listed in
  **Decision-record impact**.
- `docs/GLOSSARY.md`: add **Monitor gap** and **Liveness record**; update **Unknown
  state**, **Fault**, **Event database** and **State transition** so they no longer say
  `unknown` only precedes the first verdict and so the fault definition states the split
  rule and the lower-bound duration.
- `docs/HISTORY-IMPORT.md`: replace the “closing a measurement gap with an `unknown` row”
  section. State that the binary now writes the marker automatically, that `import`
  accepts `unknown`, and that the gap ends the open fault so the unmeasured interval is
  not down time. Remove the manual SQL procedure, or keep it only as a historical note
  marked superseded; the split rule preserves its intent.
- `docs/NOTIFICATIONS.md`: add the `gap` member to the startup document description and
  the optional `Gap:` line to the text example; update the verification table’s “first
  probe after the restart” row to say the first observation after a gap reports `DOWN` or
  `UP` under the normal policy.
- `site/index.html` *(maybe)*: one sentence in the FAQ that a restart after a long absence
  is shown as `unknown` rather than carried forward.
- `uptime-monitor.sample.toml`: add the `gap_after_ms` block shown in Step 1.

Search the docs for the old claims before finishing:
`grep -rn "nothing in the binary writes\\|Two tables\\|before its first verdict" docs`.

**Verify:** the grep returns only statements that are still true; `uptime-monitor manual`
prints the updated guide; the sample configuration parses with `uptime-monitor check`.

### Step 9 — Run the quality gate and the manual acceptance check

Run the exact project quality gate:

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Manual acceptance, using a TCP target so no raw-socket privilege is needed:

1. Start a local TCP listener that keeps accepting, for example `nc -lk 127.0.0.1 19000`
   where available, or a short Python `accept()` loop.
2. Create a temporary config with `interval_ms = 500`, `gap_after_ms = 5000`, a
   `tcp:127.0.0.1:19000` target and a temporary database. Start `monitor` for a few
   seconds and stop it with `SIGINT`.
3. With the monitor stopped, set the liveness row to an old instant:
   `sqlite3 <db> "UPDATE monitor_liveness SET last_seen_ms = last_seen_ms - 60000, stopped_at_ms = NULL;"`.
4. Start `serve --listen 127.0.0.1:0` (or a fixed free port) and confirm `query state`
   and the page show the target as `unknown` after the threshold, with a grey tail.
5. Start `monitor` again; confirm `query events` shows `... -> unknown` and then
   `unknown -> up` for the target, and that the page closes the grey band at the new
   observation.
6. Stop the monitor and confirm `monitor_liveness.stopped_at_ms` is set.

**Verify:** the quality gate is green and the six manual steps produce the stated rows.

## Security considerations

The change adds no network listener, authentication surface, secret or untrusted input.
The new configuration value is a `u64` validated in `Config::resolve`; the liveness row
contains only timestamps and the threshold. `--show-config` prints it like any other
global, and no secret is involved. The read-only `serve`/`query` path still opens the
database read-only and issues only `SELECT`; `INV-10` is unchanged. The main operational
risk is a disclosure of the monitor’s stop/start timing in the database, which is the same
class of information as the event history the tool already stores.

## Verification plan

- **Automated:** the tests named in each step. In particular:
  - `src/config.rs`, `tests/config_precedence.rs`, `tests/cli.rs` for the key, flag,
    default, minimum, `0` and `--show-config`.
  - `src/store/mod.rs` for schema v4, `begin_run`, heartbeat, clean stop, the gap event
    chain, the live overlay, the v3 read-only fixture, and the split-fault sequences.
  - `tests/monitor_e2e.rs` for a long restart, a short restart, and `stopped_at_ms`.
  - `src/notify/message.rs` and `tests/notify.rs` for the startup gap in text, summary and
    JSON.
  - `tests/web.rs` for the current state and the grey timeline tail while stale.
  - `src/import.rs` and `tests/import.rs` for accepting and round-tripping `unknown`.
  - `tests/reload.rs` for a reloaded threshold and the absence of a reload gap.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- **Manual:** the six-step TCP acceptance check above. If the environment has
  `CAP_NET_RAW`, repeat once with an ICMP target to confirm the probe path is unchanged.
- **Rollback:** the schema change is additive, but a version-4 database is refused by an
  older binary (BR-9). Take the documented pre-upgrade backup of the SQLite file; to roll
  back, stop the monitor, restore that file, and run the older binary. The new
  `monitor_liveness` table and any `unknown` events are discarded by the restore. A
  binary-only rollback without the backup is not supported.

## Open questions

- [x] Which state token? Reuse `unknown`; it is already grey, exported, imported and
  understood by older readers.
- [x] Which fault semantics? Split at the gap: the fault ends at the gap start, a new
  fault starts after the restart when the target is still down, and the gap is not down
  time. This was the explicit user decision after the keep-open alternative was rejected;
  the runbook’s repair keeps its intent.
- [x] Which threshold default and disable rule? `gap_after_ms = 60000`, `0` disables, a
  non-zero value below `5000` is refused.
- [x] How far does live `unknown` go? Current state, timeline tail, up-run statistics and
  the detail page, all derived in the store.
- [x] What about targets removed while the monitor was off? Only targets in the starting
  configuration are marked; a removed target keeps its last state as today.
- [x] What about the first start after the schema upgrade? No previous liveness row, so no
  gap is claimed; the first start records the row and future restarts can detect gaps.
- [x] What about a reload? It never calls `begin_run`; a changed threshold is written to
  the liveness row, so readers see it, but no gap is created.

## Implementation checklist

- [ ] Step 1 — `gap_after_ms` key, flag, defaults, validation, sample config
- [ ] Step 2 — schema v4 `monitor_liveness`, `begin_run`/`heartbeat`/`end_run`
- [ ] Step 3 — monitor begin, heartbeat, reload, clean stop
- [ ] Step 4 — startup notification gap summary
- [ ] Step 5 — live `unknown` in states, find_target and timeline
- [ ] Step 6 — verify and test split faults at the gap
- [ ] Step 7 — import accepts `unknown`
- [ ] Step 8 — documentation, ADR, runbook, sample and landing page
- [ ] Step 9 — quality gate and manual TCP acceptance
- [x] All open questions resolved
- [ ] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [ ] Documentation listed under Affected modules is updated
- [ ] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every configuration option, or the plan records why it needs no change
- [ ] `## Implementation Notes` added or updated in this plan

## Implementation Notes

<!-- The implementer records progress, decisions taken during implementation, and any
     deviation from the plan here. Keep this section current so a later session can
     resume. -->

