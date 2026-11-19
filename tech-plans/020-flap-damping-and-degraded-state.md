---
title: Damp state transitions with down_after and add a persisted degraded state for slow-but-reachable targets
date: 2026-11-19
status: draft
type: feature
impact: high
area: monitor, config, store, web, notify, query, import, docs
version_bump: minor
tags: [flap-damping, degraded, state-machine, schema, notifications]
depends_on: []
branch_recommended: true
---

## Summary

Two coupled changes to the state model, shipped in two phases.

Phase 1 adds **flap damping**: a global and per-target `down_after` (default `1`, today's
behaviour) requires N consecutive failed probes before a target is recorded `down`. The
recovery to `up` stays immediate. The counting tracker lives in the monitor worker; local
probe errors freeze the streak and never count (INV-5).

Phase 2 adds a fourth state, `degraded`: a reachable target whose round-trip time stays at
or above `degraded_after_ms` for `degraded_after` consecutive probes is recorded as
`degraded` instead of `up`, and returns to `up` after `degraded_after` consecutive fast
probes. `degraded_after_ms = 0` disables the feature, so the default behaviour is unchanged.
`degraded` is a true transition: it is persisted in `events`/`targets`, drawn as an amber
band on the page, returned by the JSON API, exported and imported, and notified as
`target_degraded`. The event-only storage decision (ADR-0004) is preserved; the schema
version becomes `5` so an older binary refuses the database (BR-9) instead of failing to
parse the new state.

## Why

`docs/PRODUCT.md` lists two open questions that this plan answers: **flap damping** (a
single lost packet currently means an outage) and the invisibility of degradation — a link
that is reachable but slow never transitions, so it never appears in the history. The
operator's own question about Uptime Kuma's retries and latency graphs made both concrete.

Phase 1 directly reduces false incidents and alert noise on a flaky link; phase 2 records
"reachable but bad", which the transition-only model cannot express today, without storing
per-probe samples (ADR-0043 is untouched: degradation is a transition, not a sample).

## Goals

- `down_after = 3` turns three consecutive failed probes into one `down` transition; one or
  two failures followed by a success record nothing and notify nothing.
- `down_after = 1` (the default) reproduces today's behaviour exactly; every existing test
  passes unchanged.
- A local probe error neither increments nor resets a streak: N real failures before and
  after a local error still produce one `down`.
- The first verdict of a target (state `unknown`) is recorded immediately, regardless of
  `down_after`, preserving FR-6 and the gap/restart behaviour.
- With `degraded_after_ms = 200` and `degraded_after = 3`, a reachable target returning
  RTT ≥ 200 ms for three consecutive probes is recorded `up -> degraded` once, and returns
  `degraded -> up` after three consecutive probes below 200 ms.
- `degraded_after_ms = 0` (the default) makes `TargetState::Degraded` unreachable.
- A `degraded` transition is visible on the page as an amber band, on `query`, in
  `/api/v1/targets` (`"state":"degraded"`), `/api/v1/timeseries` (`"v":"degraded"`), and in
  the CSV export; the export imports back as it is.
- A `degraded` event carries the cause `PROBE.SLOW` with a standard message, and every
  channel enabled for the target receives `target_degraded`.
- A `degraded` target is still reachable: it is not a fault, down time is unchanged, and a
  fault that was open closes on the `degraded` event exactly as on any other event.
- Every new key has a command-line override (INV-2) and is printed by `--show-config`.

## Non-goals

- Symmetric down/up damping: only `down_after` is added (`up` recovery is immediate, as
  today; `stabilize_seconds` keeps delaying the recovery *notification*).
- Persisting per-probe RTT samples, percentiles, packet-loss counts, or an RTT aggregate
  table. The history stays transition-only.
- Degradation in the simulator: `simulate` emits only `up`/`down`; the `Degraded` variant is
  handled in exhaustive matches but never generated.
- A separate cause family for degradation beyond one `PROBE.SLOW` code.
- Changing `FR-17`'s orange minimum-width marker for brief faults; `degraded` gets its own
  amber band with the same minimum-width rule.
- Validating `degraded_after_ms` against `timeout_ms` (a threshold above the timeout simply
  never triggers, because a probe that exceeds the timeout is `down`).
- A configuration UI, a new transport, or a new dependency.

## Affected modules

**Source — configuration and CLI**
- `src/config.rs` — `FileConfig`, `FileTarget`, `TargetSpec`, `Target`, `Config`, `Overrides`;
  defaults, merge, validation, `to_toml`, `TargetSpec::parse_spec`, `TargetSpec::from_file`.
- `src/cli.rs` — `--down-after`, `--degraded-after-ms`, `--degraded-after`.
- `src/main.rs` — pass the three overrides into `Config::resolve`.
- `src/init.rs` — unchanged (defaults leave both features off); confirm no key is added.

**Source — state machine and monitor**
- `src/monitor/state.rs` — `Tracker`, the damping/degradation decision, tests.
- `src/monitor/mod.rs` — build the tracker from the loaded state, feed each outcome, map a
  `degraded` transition to a notification.
- `src/probe/failure.rs` — `Code::ProbeSlow` (`PROBE.SLOW`), its template, `Details::count`,
  `Code::ALL` length.
- `src/probe/mod.rs` — only if `Details` re-export needs a touch (no transport change).

**Source — store and history**
- `src/store/mod.rs` — `TargetState::Degraded` (`as_str`/`parse`), `begin_run`'s
  `current_state IN ('up','down','degraded')`, exhaustive matches, schema tests.
- `src/store/schema.rs` — `SCHEMA_VERSION = 5`.
- `src/history.rs` — no type change; re-exports only.
- `src/import.rs` — `TargetState::parse` accepts `degraded`; no format change beyond the value.
- `src/query/render.rs` — the `degraded` label in the table and JSON.

**Source — notifications**
- `src/notify/message.rs` — `Event::TargetDegraded`, constructor, `event_name`, `severity`,
  `text`, `summary`, `json`.
- `src/notify/policy.rs` — map `TargetDegraded`, report at once, suppress a repeated one,
  cancel a pending recovery; leaving `degraded` for `up` uses the existing window.
- `src/notify/mod.rs` — routing only; confirm no change.

**Source — web**
- `src/web/page.rs` — amber colour and `degraded` label, minimum-width rule for
  `down`/`degraded`, `up_for` unchanged.
- `src/web/detail.rs` — held-time and state text for `degraded`.
- `src/web/static/style.css` — `.stat.state.degraded`.
- `src/web/templates/detail.html` *(maybe)* — the state class.
- `src/web/stats.rs` — `up_ranges` treats `degraded` as reachable: it does not break an `up`
  run (availability, not quality).
- `src/web/api/targets.rs`, `src/web/api/timeseries.rs` — the new state string flows through
  `as_str`; pin it with tests.
- `src/web/export.rs` — `rtt_us` and `cause` are emitted for `up` and `degraded`.
- `src/web/api/faults.rs`, `src/web/api/daily.rs` — unchanged (`down` only).
- `src/sim/history.rs` — exhaustive-match arm only.

**Tests**
- `src/monitor/state.rs` — damping and degradation tables, first-verdict bypass, local-error
  freeze, streak reset on recovery.
- `tests/monitor_e2e.rs` — a scripted flapping target records no `down` until `down_after`;
  a slow-but-reachable target records `up -> degraded -> up`.
- `src/config.rs` — defaults, per-target overrides, refusals, target-spec attributes,
  `to_toml` round-trip.
- `src/store/mod.rs` — `degraded` parse/render, `begin_run` closes `degraded` to `unknown`,
  timeline with a degraded segment, schema migration 4 -> 5.
- `src/web/{page,detail,stats}.rs`, `src/web/api/{targets,timeseries}.rs`,
  `src/web/export.rs` — degraded colour, label, statistics, JSON, CSV.
- `src/notify/{message,policy}.rs` — `target_degraded` text/JSON/severity, immediate report,
  suppression, recovery window.
- `src/import.rs` — a degraded row round-trips; an older archive still imports.
- `tests/cli.rs` — the three flags and `--show-config` output.

**Documentation / configuration**
- `docs/PRODUCT.md` — scope bullets, `FR-3`/`FR-5`/`FR-6`/`FR-25`/`FR-26`/`FR-36` amendments,
  new `FR-39` (damping) and `FR-40` (degraded), `BR-1`/`BR-2` wording, resolve the
  **flap damping** open question.
- `docs/GLOSSARY.md` — **Damping**, **Degraded**, **Slow threshold**.
- `docs/ARCHITECTURE.md` — state model and transition rule, component guarantees, `INV-5`,
  `INV-16`, the schema-version table, the page colour legend.
- `docs/ADR.md` — new **ADR-0046** (a fourth state and damping) with index row; one-line
  amendments to `ADR-0004` (still transitions only) and `ADR-0022`/`ADR-0025` where they
  enumerate the states.
- `docs/USER-GUIDE.md` — `What it is`, `Configuration` (global and target tables), the
  behaviour section, and the notification table (`target_degraded`).
- `uptime-monitor.sample.toml` — commented `down_after`/`degraded_after_ms`/
  `degraded_after` examples.
- `README.md`, `site/index.html` — one sentence each that lists the states or damping.
- `docs/HISTORY-IMPORT.md` — one line that an imported `degraded` row is accepted.

## Architecture impact

`monitor::state` stops being a pure two-argument function and becomes a small per-target
state machine (`Tracker`) that owns three counters and the last recorded state. The
`Probe` contract, the transports, the store writer, the notification delivery, and the web
read path are unchanged. The state vocabulary grows from `up | down | unknown` to
`up | degraded | down | unknown`; every exhaustive `match` on `TargetState` must be revisited
by the compiler. The database changes no column: `state` and `previous_state` are `TEXT`,
so only new values appear. The schema version becomes `5` without DDL, so `migrate` updates
`schema_meta` and an older binary refuses the file under BR-9.

The page legend and the statistics acquire a fourth band; `up` runs keep meaning
"reachable", so a `degraded` stretch does not split an `up` run and down time is unchanged.
`INV-1` and `INV-7` are untouched; `INV-16` is extended to close `degraded` as `unknown`
across a gap.

## Decision-record impact

**ADR-0046 — Record a reachable-but-slow target as `degraded`, and damp `down` with a
consecutive-failure count.** Context: a single lost packet is currently an outage and slow
links are invisible. Decision: `down_after` (default 1) gates `down`; `degraded_after_ms`
(default 0, off) plus `degraded_after` (default 3) gate `degraded`; `degraded` is a
transition with cause `PROBE.SLOW`; it counts as reachable for availability and does not
break an up run. Consequences: false outages drop, degradation is visible, the state set and
every consumer grow by one value, and a `degraded` alert can precede an `up` alert. Alternatives:
persisting latency samples (rejected, reverses ADR-0004/0043), a notification-only signal
(rejected, no history and not on the page), a separate `slow` flag on an `up` event (rejected,
breaks BR-1 and the current-state projection), symmetric damping (rejected, KISS).

## Proposed solution

### Phase 1 — flap damping

### Step 1 — Add `down_after` to the configuration and the command line

In `src/config.rs`: add `down_after: Option<u64>` to `FileConfig`, `FileTarget`,
`TargetSpec`, `Overrides`; add `down_after: u32` to `Target` and `Config`; default `1`;
refuse `0` naming `down_after`; resolve per target with the global value as default. In
`src/cli.rs` add the global `--down-after <COUNT>` and pass it in `src/main.rs`. In
`TargetSpec::parse_spec` accept the attribute `down_after`.

```toml
interval_ms = 5000
# Phase 1
down_after = 1            # consecutive failed probes before `down`; 1 = today's behaviour

[[targets]]
ip = "1.1.1.1"
label = "cloudflare-dns"
down_after = 3            # a flaky link needs three real failures
```

**Verify:** `cargo test --lib config` covers the default, a global value, a per-target
override, the `0` refusal, the `down_after=3` attribute, and a `to_toml` round-trip;
`cargo test --test cli` shows `down_after = 3` in `--show-config`.

### Step 2 — Decide transitions with a per-target tracker

Replace the free `transition(previous, outcome)` in `src/monitor/state.rs` with:

```rust
pub struct Tracker {
    state: TargetState,   // last recorded state
    down_streak: u32,
    slow_streak: u32,
    fast_streak: u32,
}

impl Tracker {
    pub fn start(state: TargetState) -> Self { ... }
    pub fn observe(&mut self, outcome: &Result<Verdict, ProbeError>, rules: &Rules)
        -> Option<Transition> { ... }
}

pub struct Rules { pub down_after: u32, /* phase 2 fields added later */ }
```

Rules, each unit-tested:

- `Err(_)`: return `None` and leave all three counters unchanged (INV-5).
- `Ok(Down(cause))`: `down_streak += 1`, zero the two slow/fast counters. If the tracker
  state is `unknown`, transition at once (FR-6). Otherwise transition when
  `down_streak >= down_after`.
- `Ok(Up { rtt })`: zero `down_streak`. From `unknown` or `down`, transition to `up` at
  once and reset the quality counters (phase 2 refines the slow case). From `up`, count
  nothing yet in phase 1.
- On any transition, reset the counters that produced it and set `state`.

**Verify:** `cargo test --lib monitor::state` proves: three failures then one success under
`down_after = 3` record nothing; the third consecutive failure records one `down`; a local
error between two failures does not reset the streak; `down_after = 1` reproduces every
current test.

### Step 3 — Wire the tracker into the worker

In `src/monitor/mod.rs`, construct `Tracker::start(previous)` per target and replace the
`state::transition(previous, &outcome)` call with `tracker.observe(&outcome, &rules)`. Keep
the rest of `monitor_target` — `record_transition`, the notification branch, `previous` — and
derive `previous` from the tracker. The `FirstRound` gate stays on the raw outcome and is
not affected by damping.

**Verify:** `cargo test --test monitor_e2e` and `cargo test --lib monitor` pass; the existing
notification and promotion tests are unmodified.

### Step 4 — Add the end-to-end regression tests

In `tests/monitor_e2e.rs`, add a scripted fake probe: `Down, Up, Down, Down, Down` under
`down_after = 3` writes exactly one `up -> down` event, at the third failure, and one
notification. Add a variant proving that a local `ProbeError` between failures neither
delays nor triggers the transition.

**Verify:** `cargo test --test monitor_e2e`.

### Step 5 — Document flap damping

Update `docs/PRODUCT.md` (new `FR-39`, the `FR-3`/`FR-5` amendments, resolve the **flap
damping** open question), `docs/GLOSSARY.md` (**Damping**), `docs/USER-GUIDE.md` (the
configuration table and an example), `docs/ARCHITECTURE.md` (the transition rule),
`docs/ADR.md` (a one-line amendment to `ADR-0004` if needed), and
`uptime-monitor.sample.toml`.

**Verify:** `rg -n "down_after|FR-39|Damping" docs uptime-monitor.sample.toml` finds each
item, and every documented default matches `config.rs`.

> **CHECKPOINT — phase 1.** Run the quality gate, stop, and wait for the operator to confirm
> the damping behaviour before starting phase 2. `spec-auto` crosses this point and reports it.

### Phase 2 — persisted `degraded` state

### Step 6 — Add the degradation keys

Add `degraded_after_ms: Option<u64>` (default `0`, disabled) and `degraded_after: Option<u64>`
(default `3`) to `FileConfig`, `FileTarget`, `TargetSpec`, `Overrides`, `Target`, `Config`;
refuse `degraded_after == 0`; add `--degraded-after-ms` and `--degraded-after`; accept the
target attributes `degraded_after_ms` and `degraded_after`; print both in `to_toml`.

```toml
# Phase 2 — off by default
degraded_after_ms = 0     # RTT at/above which a reachable probe is slow; 0 disables
degraded_after = 3        # consecutive slow probes to enter degraded (and fast ones to leave)

[[targets]]
tcp = "1.1.1.1:443"
label = "cloudflare-tls"
degraded_after_ms = 200
degraded_after = 3
```

**Verify:** `cargo test --lib config`; `--show-config` round-trips both keys.

### Step 7 — Add `TargetState::Degraded` and bump the schema

In `src/store/mod.rs` add the variant with `as_str() == "degraded"` and the `parse` arm; add
`'degraded'` to `begin_run`'s `current_state IN (...)`. In `src/store/schema.rs` set
`SCHEMA_VERSION = 5` (no DDL; the generic `migrate` updates `schema_meta`). Fix every
exhaustive `match` the compiler reports.

**Verify:** `cargo test --lib store` proves a v4 database migrates to v5 without losing a
row, a `degraded` target is closed to `unknown` by `begin_run`, and a v5 file is refused by
a binary that supports only v4 (BR-9).

### Step 8 — Extend the tracker with degradation

Add to `src/monitor/state.rs`: `Rules { degraded_after_ms: u64, degraded_after: u32 }`; the
`up`-verdict branch classifies `slow = degraded_after_ms > 0 && rtt >= threshold`:

- from `up`: `slow_streak`/`fast_streak` count; `slow_streak >= degraded_after` →
  `Transition { to: Degraded, rtt_us: Some(rtt), cause: Some(ProbeSlow) }`.
- from `degraded`: `fast_streak >= degraded_after` → `up`; a slow probe keeps `degraded`.
- from `down`/`unknown`: the first success is `up` (immediate), then the `up` rules apply.
- a `down` verdict from `degraded` follows the same `down_after` rule as from `up`.

In `src/probe/failure.rs` add `Code::ProbeSlow => "PROBE.SLOW"` with template
`"the round-trip time stayed above {timeout} for {count} consecutive probes"`, add
`Details::count`, and update `Code::ALL`.

**Verify:** `cargo test --lib monitor::state` covers enter/leave, hysteresis, the disabled
threshold, `down` from `degraded`, and the exact `PROBE.SLOW` message; `cargo test --lib
probe::failure` proves every code has a template.

### Step 9 — Notify `target_degraded`

In `src/notify/message.rs` add `Event::TargetDegraded { target, cause }`,
`Message::target_degraded`, `event_name() == "target_degraded"`, `severity() == Warn`, and
the `text`/`summary`/`json` arms (JSON `"state":"degraded"`). In `src/notify/policy.rs` map
it to `TargetState::Degraded` and report it at once like `down`, suppressing a repeated
`degraded` and cancelling a pending recovery; a later `up` uses the existing stabilisation
window. In `src/monitor/mod.rs` map a `degraded` transition to the new message.

**Verify:** `cargo test --lib notify` proves the text, summary, JSON, severity, the immediate
report, the suppression, and that `down -> degraded` is impossible while `up -> degraded`
is reported.

### Step 10 — Store, query, and import surfaces

Make `query state`/`query events` print `degraded`; confirm `import` parses it and that an
older archive still imports; update any exhaustive matches in `src/import.rs` and
`src/query/render.rs`.

**Verify:** `cargo test --lib query`, `cargo test --lib import` (a degraded row round-trips,
a pre-degraded archive imports unchanged).

### Step 11 — Web surfaces

`src/web/page.rs`: colour `degraded` amber (`#f9a825`), label it `degraded`, apply the
minimum-width rule to `down` and `degraded` (only `down` keeps the orange marker);
`src/web/detail.rs` and `src/web/static/style.css` (`.stat.state.degraded`) render the
state; `src/web/stats.rs` makes `up_ranges` continue across `degraded`; `src/web/export.rs`
emits `rtt_us` and `cause` for `up | degraded`. `/api/v1/targets`, `/api/v1/timeseries` and
the CSV then carry the new value through `as_str`.

**Verify:** `cargo test --lib web` and `cargo test --test web` cover the amber band, the
label, an up run crossing a degraded stretch, the JSON state, and the CSV row; `/api/v1/faults`
and `/api/v1/faults/daily` are unchanged.

### Step 12 — Simulator and exhaustive matches

Add the `TargetState::Degraded` arm where the compiler requires it in `src/sim/history.rs`;
the simulator still emits only `up`/`down`. Do not add degraded generation.

**Verify:** `cargo test --test simulate` passes unchanged.

### Step 13 — Add the degraded tests

Add the config, store, state, notification, web, import and e2e tests listed under
**Affected modules**, including the schema migration and the `down_after`/degraded
interaction in `tests/monitor_e2e.rs`.

**Verify:** `cargo test`.

### Step 14 — Document the degraded state and record the decision

Update `docs/PRODUCT.md` (`FR-40`, `BR-1`/`BR-2`, `FR-25`/`FR-26`/`FR-36`), `docs/GLOSSARY.md`
(**Degraded**, **Slow threshold**), `docs/ARCHITECTURE.md` (state model, `INV-16`, the schema
version table, the colour legend), `docs/ADR.md` (**ADR-0046** and its index row, plus the
`ADR-0004`/`ADR-0022` amendments), `docs/USER-GUIDE.md` (both config tables, behaviour, the
notification table), `docs/HISTORY-IMPORT.md` (one line), `uptime-monitor.sample.toml`,
`README.md` and `site/index.html`.

**Verify:** `rg -n "degraded|degraded_after_ms|degraded_after|ADR-0046|FR-40" docs README.md
site/index.html uptime-monitor.sample.toml` finds each item, and every figure matches the code.

### Step 15 — Run the quality gate and review the diff

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`,
then read `git diff` once end to end: no unrelated file, no new dependency, no secret.

## Security considerations

Not a `security` plan. The new keys are bounded integers validated by `Config::resolve`; the
state string is a closed enum, so an imported or stored value cannot inject HTML or SQL
(`TargetState::parse` rejects anything else). No listener or egress is added. The web page
already escapes dynamic text and the new state flows through that path. No secret is
involved.

## Verification plan

- **Automated.** The unit, integration and e2e tests of steps 1–13; the schema migration and
  BR-9 downgrade test; the unchanged probe, promotion, reload and notification suites.
- **Quality gate.** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  (from `.spec-workflow.yml`).
- **Manual.** With a fake slow target (`tc qdisc` or a scripted `--target`), set
  `degraded_after_ms = 200`; confirm the amber band, the `target_degraded` message and the
  `degraded` CSV row. With `down_after = 3`, drop three packets and confirm one `down`
  event, not three.
- **Rollback.** Phase 1: revert the commit; `down_after = 1` is the default, so an old binary
  reads the configuration unchanged once the key is removed. Phase 2: a binary older than
  schema 5 refuses the database (BR-9); restore the pre-change database from the export
  (`import`), or keep the `degraded_after_ms = 0` default to avoid new values before
  downgrading.

## Open questions

- [ ] Does `degraded` count as reachable for up-run statistics and down time? Current best
      guess: yes — down time counts `down` only and an `up` run continues across `degraded`;
      alternative: `degraded` splits the run.
- [ ] Should the schema version move to `5` with no DDL, only to make a downgrade refuse the
      file? Current best guess: yes, it is the only way BR-9 can protect a database holding
      `degraded` values.
- [ ] Is `degraded_after = 3` the right default count (used for both entering and leaving)?
      Current best guess: yes, one key for both directions; alternative: separate
      `recovered_after`.
- [ ] `PROBE.SLOW` as the cause code of a `degraded` event, with `Details::count`. Current
      best guess: yes; alternative: leave the cause empty and show only the RTT.
- [ ] `FR-39`/`FR-40` and `ADR-0046` as the numbers (draft plan 016 reserves `FR-37` and
      conceptually `ADR-0042`). Current best guess: `FR-39`, `FR-40`, `ADR-0046`.
- [ ] Should `simulate` ever emit `degraded`? Current best guess: no, it stays `up`/`down`
      (non-goal), so the simulator's determinism and fault-rate model do not change.

## Implementation checklist

- [ ] Step 1 — Add `down_after` to the configuration and the command line
- [ ] Step 2 — Decide transitions with a per-target tracker
- [ ] Step 3 — Wire the tracker into the worker
- [ ] Step 4 — Add the end-to-end regression tests
- [ ] Step 5 — Document flap damping
- [ ] Checkpoint — phase 1 verified by the operator
- [ ] Step 6 — Add the degradation keys
- [ ] Step 7 — Add `TargetState::Degraded` and bump the schema
- [ ] Step 8 — Extend the tracker with degradation
- [ ] Step 9 — Notify `target_degraded`
- [ ] Step 10 — Store, query, and import surfaces
- [ ] Step 11 — Web surfaces
- [ ] Step 12 — Simulator and exhaustive matches
- [ ] Step 13 — Add the degraded tests
- [ ] Step 14 — Document the degraded state and record the decision
- [ ] Step 15 — Run the quality gate and review the diff
- [ ] All open questions resolved
- [ ] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [ ] Documentation listed under Affected modules is updated
- [ ] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every configuration option
- [ ] `## Implementation Notes` added or updated in this plan
