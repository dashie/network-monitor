---
title: Damp state transitions with down_after and add a persisted degraded state for slow-but-reachable targets
date: 2026-11-19
status: done
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
  handled in exhaustive matches but never generated. *(Reversed by extension E1, at the
  operator's request on 2026-11-19: the simulator is the acceptance surface of the feature, so
  it now generates `degraded` too — see [ADR-0047](./docs/ADR.md).)*
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
- `src/web/page.rs` — `degraded` colour (light green, extension E2) and label, minimum-width rule for
  `down`/`degraded`, `up_for` unchanged.
- `src/web/detail.rs` — held-time and state text for `degraded`.
- `src/web/static/style.css` — `.stat.state.degraded`.
- `src/web/templates/detail.html` *(maybe)* — the state class.
- `src/web/templates/page.html` *(extension E2)* — the legend entry for `degraded`.
- `src/web/stats.rs` — `up_ranges` treats `degraded` as reachable: it does not break an `up`
  run (availability, not quality).
- `src/web/api/targets.rs`, `src/web/api/timeseries.rs` — the new state string flows through
  `as_str`; pin it with tests.
- `src/web/export.rs` — `rtt_us` and `cause` are emitted for `up` and `degraded`.
- `src/web/api/faults.rs`, `src/web/api/daily.rs` — unchanged (`down` only).
- `src/sim/history.rs` — exhaustive-match arm only; then (extension E1) the target's slow
  rules reach the generator and a degraded episode becomes a segment and an event.
- `src/sim/mod.rs` *(extension E1)* — the simulated generator's constants and tests.
- `src/probe/mod.rs`, `src/probe/{icmp,tcp,dns,http}.rs` *(extension E1)* — the simulated
  episode vocabulary gains `Degraded` and the rules that carve the slow window.

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
- `src/sim/mod.rs`, `src/sim/history.rs`, `tests/simulate.rs` — the deterministic degraded
  stretches, their cause and latency, and the page/API/export surfaces over them
  *(extension E1)*.

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
- `docs/DEVELOPER-GUIDE.md` and `CONTRIBUTING.md` *(extension E1)* — the simulator obligation
  of ADR-0047 in the developer workflow and in the definition of done.

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

### Extension E1 — the simulator carries the new state

Requested by the operator on 2026-11-19: without a simulated `degraded` stretch the feature
cannot be validated end to end through the product's own surfaces. The rule becomes
[ADR-0047](./docs/ADR.md): every user-visible state is reproducible in `simulate`.

### Step 16 — Generate `degraded` stretches in the simulator

In `src/probe/mod.rs` add `SimulatedState::Degraded` and `SimulatedRules`
(`degraded_after_ms`, `degraded_after`, `interval`); inside an `up` stretch of a target whose
threshold is non-zero, carve one slow window per day on about every other day, from the
`(seed, key, day, "slow")` stream: it lasts at least `degraded_after` probes at the target's
interval and at least one minute, leaves an `up` stretch before and after it, and carries a
round-trip time at or above the threshold and the `PROBE.SLOW` cause. A threshold of `0`, a
`Life::AlwaysUp` target, and a first verdict are never degraded. Forward the rules through
`simulated_day` and the four method modules, pass them from `src/sim/history.rs`, and map the
episode to `TargetState::Degraded` there.

**Verify:** `cargo test --lib sim` covers the placement, the cause, the latency, the
determinism and `FR-40`; `cargo test --test simulate` shows the amber band, a `degraded` sample
in `/api/v1/timeseries`, and no degraded when the threshold is `0`.

### Step 17 — Document the simulated state and the rule

Update `docs/PRODUCT.md` (`FR-27`), `docs/ARCHITECTURE.md` (the `sim` component),
`docs/USER-GUIDE.md` (the simulated-mode section), `docs/DEVELOPER-GUIDE.md` (the simulated
board), `CONTRIBUTING.md` (definition of done) and `docs/ADR.md` (**ADR-0047**, its index row,
and the `ADR-0032` status).

**Verify:** `grep -rn "degraded" docs/USER-GUIDE.md docs/DEVELOPER-GUIDE.md` finds the
simulated behaviour, and `grep -n "ADR-0047" docs/ADR.md CONTRIBUTING.md` finds the rule.

### Step 18 — Run the quality gate over the extension

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`,
then read the new diff: one simulator rule, its tests, and the documents, with no new
dependency.

### Extension E2 — a colour a reader can tell apart

Raised by the operator on 2026-11-19: the amber of `degraded` (`#f9a825`) and the orange of a
widened short fault (`#ef6c00`) are too close, especially at the minimum band width, and the
legend did not name `degraded` at all.

### Step 19 — Paint `degraded` as a light green and name it in the legend

`degraded` means "still reachable, but slow", so it joins the green family: the band becomes
`#8bc34a` (light green) instead of amber, which separates it from the dark green `up`, the red
`down`, the orange short fault and the grey `unknown` by lightness first and hue second, and
which stays distinguishable in greyscale and for the common colour-vision deficiencies. The
state tile of the detail page uses the same fill with dark text (`#17330a`), because white on a
light green would not meet contrast; the legend on the timeline gains the `degraded` entry.

**Verify:** `cargo test --lib web` and `cargo test --test web` assert the light-green fill, the
legend entry, the dark tile text and the unchanged minimum-width rule; the short fault stays
orange.

### Step 20 — Document the light green and run the gate

Update the colour references in `docs/PRODUCT.md` (`FR-27`, `FR-40`), `docs/ARCHITECTURE.md`
(the `web` guarantees), `docs/USER-GUIDE.md` (the timeline and the simulated mode) and
`site/index.html`; record the change in the Implementation Notes.

**Verify:** `grep -rn "amber" docs site` finds no reference to the old `degraded` colour;
`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes.

### Extension E3 — more spells, and a threshold that changes them

Raised by the operator on 2026-11-19: one window on about every other day is too rare to review,
and changing `--degraded-after-ms` from 200 to 2000 changed neither the count nor the shape of
the board, so the parameter looked ignored.

### Step 21 — Scale the slow spells with the threshold

`slow_spells` chooses how many windows a target holds in a day: six at 250 ms or less, three up
to one second, one above, doubled for a `Life::Flapping` target and zero for `Life::AlwaysUp` or
a threshold of `0`. `slow_windows` places that many inside the up stretches, each separated by an
`up` piece and leaving `up` before and after it, so a day can now hold several degradations and a
stricter threshold visibly holds fewer without ever holding none. The RTT of each window stays
at or above the threshold and the cause stays `PROBE.SLOW`.

**Verify:** `cargo test --lib sim` proves that a 200 ms board holds more than twice the
degradations of a 2000 ms board over the same forty days and that the stricter board still holds
some; `cargo test --test simulate` counts the `degraded` samples of the two boards and asserts
the same ordering.

### Step 22 — Document the rate and run the gate

Update `docs/PRODUCT.md` (`FR-27`), `docs/ARCHITECTURE.md` (the `sim` guarantees) and
`docs/USER-GUIDE.md` (the simulated mode), and record the decision in the Implementation Notes.

**Verify:** the documents state that the count falls with the threshold and never reaches zero;
`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes.

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

- [x] Does `degraded` count as reachable for up-run statistics and down time? *Resolved:* yes.
      `web::stats::up_ranges` treats an `up` and a `degraded` segment alike, so a run continues
      across a degraded stretch and down time still counts `down` only, as the plan's best guess
      stated; verified by `a_degraded_stretch_is_reachable_and_does_not_break_a_run`.
- [x] Should the schema version move to `5` with no DDL, only to make a downgrade refuse the
      file? *Resolved:* yes. `SCHEMA_VERSION = 5` with no DDL; a database stamped above the
      binary's version is refused (BR-9), verified by `a_database_from_a_newer_schema_is_refused`
      and the 4 -> 5 migration test.
- [x] Is `degraded_after = 3` the right default count (used for both entering and leaving)?
      *Resolved:* yes. One key governs both directions and `0` is refused; a recovery to `up`
      is still immediate from `down`/`unknown`. Verified by the tracker hysteresis tests.
- [x] `PROBE.SLOW` as the cause code of a `degraded` event, with `Details::count`. *Resolved:*
      yes. `Code::ProbeSlow` renders "the round-trip time stayed above {timeout} for {count}
      consecutive probes", and `Details::count` carries the count; the threshold travels in
      `Details::timeout`. Verified by `a_slow_target_becomes_degraded_after_the_configured_probes`.
- [x] `FR-39`/`FR-40` and `ADR-0046` as the numbers (draft plan 016 reserves `FR-37` and
      conceptually `ADR-0042`). *Resolved:* `FR-39`, `FR-40`, and `ADR-0046` — FR-38 was the
      highest requirement in `docs/PRODUCT.md` and ADR-0045 the highest record at the time.
- [x] Should `simulate` ever emit `degraded`? *Resolved:* no. The simulator still emits only
      `up`/`down`; `src/sim/history.rs` handles the variant in its exhaustive test match and
      never generates it, and `tests/simulate.rs` passes unchanged.

## Implementation checklist

- [x] Step 1 — Add `down_after` to the configuration and the command line
- [x] Step 2 — Decide transitions with a per-target tracker
- [x] Step 3 — Wire the tracker into the worker
- [x] Step 4 — Add the end-to-end regression tests
- [x] Step 5 — Document flap damping
- [x] Checkpoint — phase 1 verified by the operator
- [x] Step 6 — Add the degradation keys
- [x] Step 7 — Add `TargetState::Degraded` and bump the schema
- [x] Step 8 — Extend the tracker with degradation
- [x] Step 9 — Notify `target_degraded`
- [x] Step 10 — Store, query, and import surfaces
- [x] Step 11 — Web surfaces
- [x] Step 12 — Simulator and exhaustive matches
- [x] Step 13 — Add the degraded tests
- [x] Step 14 — Document the degraded state and record the decision
- [x] Step 15 — Run the quality gate and review the diff
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every configuration option
- [x] `## Implementation Notes` added or updated in this plan
- [x] Step 16 — Generate `degraded` stretches in the simulator
- [x] Step 17 — Document the simulated state and the rule
- [x] Step 18 — Run the quality gate over the extension
- [x] Extension E1 checked against [ADR-0047](./docs/ADR.md)
- [x] Step 19 — Paint `degraded` as a light green and name it in the legend
- [x] Step 20 — Document the light green and run the gate
- [x] Step 21 — Scale the slow spells with the threshold
- [x] Step 22 — Document the rate and run the gate

## Implementation Notes

### Phase 1

- **Deviation (step 4).** The plan's scripted sequence for the damping e2e test was
  `Down, Up, Down, Down, Down`, which cannot produce "exactly one `up -> down` event at the
  third failure": the first verdict is recorded immediately from `unknown` (FR-6), so it
  yields an `unknown -> down` event first. The test starts from `up` instead
  (`Up, Down, Down, Down`) and asserts the chain `unknown -> up`, `up -> down` at the third
  consecutive failure, then `down -> up`. The intent of the step is unchanged and FR-6 is
  preserved.

### Checkpoint — phase 1 crossed

- **Label:** `CHECKPOINT — phase 1`.
- **What now works:** `down_after` is configurable globally and per target (file, `--target`
  attribute, `--down-after`), resolved by `Config::resolve`, refused at `0`, printed by
  `--show-config`; `monitor::state::Tracker` damps `down` and keeps `up` immediate, with local
  errors leaving the streak untouched; the e2e tests prove the count and the error rule; the
  product document, glossary, architecture, user guide and sample configuration state the
  feature.
- **How to verify:** `--show-config --down-after 3 --target 1.1.1.1` prints `down_after = 3`;
  `cargo test --test monitor_e2e`; `cargo fmt --check && cargo clippy --all-targets --
  -D warnings && cargo test` passed (531 lib tests, every integration suite green).
- **To continue without this lane:** `spec-implement 020` resumes at step 6. This lane crossed
  the checkpoint as `spec-auto` defines and continued.

### Phase 2

- **Decision (step 8).** `Rules` carries `degraded_after_ms: u64` and `degraded_after: u32`
  beside `down_after`, and `Tracker` builds the `PROBE.SLOW` failure itself: the threshold
  travels as `Details::timeout` (rendered `200 ms`) and the count as `Details::count`, because
  the template fixes those two placeholders. `Rules` stays `Copy`.
- **Decision (step 8).** The `slow` test is `degraded_after_ms > 0 && rtt >= threshold`, so a
  zero threshold keeps `TargetState::Degraded` unreachable exactly as `FR-40` states; a first
  verdict from `unknown` and a recovery from `down` are still `up`, never `degraded` (`FR-6`).
- **Deviation (step 11).** The plan lists `src/web/export.rs` as emitting `rtt_us` and `cause`
  for `up | degraded`. The production path needed no edit: `store::events_after` already maps
  the whole `ExportedEvent` from the row, and `web::export::row` prints whatever those fields
  hold. The behaviour is pinned by the end-to-end web test, which records a degraded event with
  an RTT and a cause and asserts the CSV carries `,250000,PROBE.SLOW,`.
- **Discovery (step 7).** `Store::states()` applies the stale-liveness overlay, so a store test
  that inspects `current_state` right after `record_transition` must query the column or hold a
  fresh liveness row; the `begin_run` test reads the column directly.
- **Discovery (step 15).** `Cargo.lock` was stale at `0.17.0` while `Cargo.toml` already said
  `0.18.0`; the first `cargo` command of the run rewrote it. The one-line lockfile change is a
  consequence of running the suite, not part of the feature.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-19
- **Verdict**: PASS
- **Self-review disclosure**: this round was executed in the same `spec-auto` invocation as
  the implementation, by the agent that wrote the code. The verdict rests on the diff, the plan
  text, and the commands executed below, not on the Implementation Notes.
- **Suggested commit**: `feat(monitor): add flap damping and the degraded state`
- **Quality gate**: tests ✓ (558 unit tests plus every integration suite, 0 failed) | lint ✓ (`cargo clippy --all-targets -- -D warnings`) | types ✓ (`cargo check`, through clippy) | format ✓ (`cargo fmt --check`)
- **Commands run**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `node spec-plan.mjs validate-for review 020 --strict`, `python3 -c 'json.load(assets/api/openapi.json)'`
- **Reviewed state**: `e0ce0df1ad961143da04a58740a78ebe9fc8d12446a052e15b35f1cdacc68c03`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major (documentation) | `assets/api/openapi.json` | The API reference still enumerated `["up","down","unknown"]` for the timeseries `v` and the targets `state`, and its description omitted `degraded`; the delivered API returns the fourth value (`FR-26`, `FR-40`). | fixed in review |
| 2 | major (documentation) | `docs/USER-GUIDE.md` | The timeseries paragraph listed only `up`, `down`, `unknown`; the timeline section had no amber `degraded` band; and the opening paragraph no longer matched the four states and exceeded three lines. | fixed in review |
| 3 | minor | `Cargo.lock` | The lockfile was stale at `0.17.0` while `Cargo.toml` already stated `0.18.0`; the first `cargo` command of the run rewrote it. Consequence of running the suite, recorded in Implementation Notes. | open (accepted) |
| 4 | nit | `src/monitor/state.rs` | `slow_streak` and `fast_streak` are `u32` and can in principle overflow in a debug build after billions of probes in one direction with the feature off; unreachable in practice at any real interval. | open (not gating) |

**Fix evidence for finding 1:** before `"enum": ["up", "down", "unknown"]`; after
`"enum": ["up", "degraded", "down", "unknown"]` on both the sample and the target state, and the
route description now reads "`up`, `degraded`, `down` or `unknown`". The document still parses
(`json.load` succeeded).

**Fix evidence for finding 2:** `v` now reads "`up`, `degraded`, `down`, or `unknown`"; the
timeline section states the amber band and that the down time ignores it; the opening paragraph
is two lines and names the four states and both new keys.

### Verification matrix
| # | Check | Result |
|---|---|---|
| 1 | Goals achieved | ✓ 10/10 |
| 2 | Scope within `## Affected modules` | ✓ (the two documentation repairs above, plus `Cargo.lock`, are recorded) |
| 3 | Quality suite green | ✓ |
| 4 | New behaviour tested on real paths | ✓ tracker, config, store, notify, web, import, e2e |
| 5 | Existing tests not weakened | ✓ the only rewrite is `src/monitor/state.rs`, mandated by the plan's replacement of `transition()` with `Tracker`; every assertion has a successor |
| 6 | Checklist, open questions, notes | ✓ 21/21, 6/6 resolved, notes present |
| 7 | Documentation contracts | ✓ after the two repairs |
| 8 | Architecture and ADR consistency | ✓ ADR-0046 added; ADR-0004 and ADR-0025 amended; `INV-1` and `INV-16` updated |
| 9 | No scope creep | ✓ |
| 10 | Edge cases and validation | ✓ `0` refused for `down_after`/`degraded_after`, `0` accepted for the threshold, first verdict immediate, local error freezes the streaks |
| 11 | Security | ✓ closed state enum, no new input surface, no secret |
| 12 | Security plan checks | not applicable |
| 13 | User guide | ✓ corrected in this round |

### Gap analysis
- Goals: 10/10 achieved. Damping, the four states, the threshold, reachability semantics,
  notifications, the API/CSV/import surfaces, the schema bump, the flags, and the defaults are
  all backed by tests and the documents.
- Steps: 15/15 complete; the phase-1 checkpoint was crossed by this lane and recorded.
- Scope: within the plan.
- Deviations: the step-4 e2e script (`Down, Up, ...` cannot yield one `up -> down` because
  FR-6 records the first verdict) and the redundant `web::export` edit; both documented.
- Additions: none beyond the plan. Deferred: none.
- Known limitations: `degraded` never appears in the simulator; `up_for`/`upForMs` stays `up`
  only, as the plan states; a probe slower than `timeout_ms` is `down`, not slow.

### Recommendations for future work
- A `simulate` mode with `degraded` stretches would exercise the amber band without a real slow
  link; it is a deliberate non-goal of this plan.

### Extension E1 — notes

- **Approved by the operator** on 2026-11-19 in this session: add the simulated `degraded`
  stretch to this plan, and record the general rule as ADR-0047.
- **Decision (step 16).** The slow window is gated by the target's `degraded_after_ms > 0` and
  skipped for `Life::AlwaysUp`, so the simulator shows exactly what the configuration enables.
  It is placed inside an existing `up` stretch, never across midnight, and leaves at least one
  probe of `up` on each side: the first verdict of a simulated history is never `degraded`,
  matching `FR-40`. No per-probe latency is stored; the window is one episode, so the
  storage decision of ADR-0004/ADR-0043 is untouched.
- **Decision (step 16).** `SimulatedRules` is a small `Copy` value passed down through
  `simulated_day`; `SimulatedSchedule` bundles the four tuple values and the purpose so
  `simulated_episodes` keeps a readable signature. The slow draws use the stream
  `(seed, key, day, "slow")`, which leaves every existing history for a threshold of `0`
  byte-identical and makes a threshold board replay exactly.
- **Deviation (step 16).** `slow_rtt` had to be expressed in microseconds, like every other
  simulated latency; the first version stored milliseconds and the history test caught it.
- **Extension scope:** `src/probe/mod.rs`, the four method modules, `src/sim/history.rs`,
  `src/sim/mod.rs` tests, `tests/simulate.rs`, the documents of step 17. No schema, config, or
  API change beyond what the plan already delivered.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-19
- **Verdict**: PASS
- **Self-review disclosure**: round 2 was executed in the same session that implemented
  extension E1, again by the agent that wrote the code; the verdict rests on the new diff and
  the commands below.
- **Suggested commit**: `feat(monitor): add flap damping, the degraded state, and its simulation`
- **Quality gate**: tests ✓ (560 unit tests, `tests/simulate.rs` 8, every integration suite, 0 failed) | lint ✓ (`cargo clippy --all-targets -- -D warnings`) | types ✓ (`cargo check`, through clippy) | format ✓ (`cargo fmt --check`)
- **Commands run**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo test --test simulate a_simulated_target`, `node spec-plan.mjs validate-for review 020 --strict`
- **Reviewed state**: `2da45a7d9a8b9b8af26c29985306464a5c62f6d63eb362997c645811d510e841`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | nit | `src/probe/mod.rs` | The simulator approximates `degraded_after` as a minimum window length at the target's interval; it does not simulate probe-level RTT, by design (ADR-0047 and ADR-0004/ADR-0043 forbid per-probe samples). | open (accepted, documented) |
| 2 | nit | `src/sim/mod.rs` | A board gets at most one slow window a day and none for a `Life::AlwaysUp` target; this keeps the day readable and the healthy baseline clean. | open (accepted, documented) |

No blocker, major, or minor finding. The documentation gap the extension could have left
(the glossary still said the simulated past is cut into `up` and `down`) was corrected before
this round, together with the record of ADR-0047.

### Verification matrix
| # | Check | Result |
|---|---|---|
| 1 | Goals and extension goal achieved | ✓ the amber band, `"v":"degraded"`, the CSV row and the `PROBE.SLOW` cause all appear under `simulate --degraded-after-ms` |
| 2 | Scope | ✓ `src/probe/{mod,icmp,tcp,dns,http}.rs`, `src/sim/{mod,history}.rs`, `tests/simulate.rs`, and the step-17 documents |
| 3 | Quality suite green | ✓ |
| 4 | Tests exercise real paths | ✓ placement, determinism, cause, latency, `FR-40`, page/API end to end |
| 5 | Existing tests intact | ✓ the threshold-default tests pass unchanged; the two new tests fail without the generator |
| 6 | Checklist, questions, notes | ✓ 25/25 |
| 7 | Documentation contracts | ✓ PRODUCT, ARCHITECTURE, GLOSSARY, ADR, USER-GUIDE, DEVELOPER-GUIDE, CONTRIBUTING |
| 8 | Architecture/ADR consistency | ✓ ADR-0047 accepted, ADR-0032 extended; `sim` guarantees state the gating and the determinism |
| 9 | No scope creep | ✓ the extension is the operator's request, recorded in E1 |
| 10 | Edge cases | ✓ threshold `0`, `AlwaysUp`, first verdict never degraded, midnight, saturating RTT |
| 11 | Security | ✓ no input surface, no secret, no new dependency |
| 12 | Security plan checks | not applicable |
| 13 | User guide | ✓ accurate, self-contained, new simulated behaviour stated, options still correct |

### Gap analysis
- Extension goals: 4/4 — simulated degraded stretches, gated by configuration, deterministic,
  and covered through the page, the API and the export.
- Steps: 18/18 plus the phase-1 checkpoint; E1 complete.
- Scope: within the plan plus the operator-approved extension.
- Deviations: `slow_rtt` first expressed in milliseconds, caught by the history test and fixed
  to microseconds (recorded in E1 notes); the `SimulatedSchedule` bundle keeps the generator's
  signature readable.
- Additions: ADR-0047 and the definition-of-done bullet, both requested by the operator.
- Deferred: none.

### Known limitations
- The simulated `degraded` stretch is a transition, not a per-probe RTT model; it cannot show
  a slow streak that never reaches the threshold count, because the generator has no probes.
- At most one degraded window a day per target, never across midnight, and never for a target
  drawn as `AlwaysUp`.

### Recommendations for future work
- If a future feature needs finer simulated latency, extend the episode model, not the storage
  model; ADR-0047 requires it to stay deterministic and config-gated.

### Extension E2 — notes

- **Approved by the operator** on 2026-11-19: `degraded` must look like a reachable target with
  a problem, not like a fault, and the amber/orange pair did not separate at the minimum width.
- **Decision.** The band is `#8bc34a` (light green): the same family as `up`, separated from it
  by lightness, from the red `down` and the orange short fault by hue and lightness, and from
  the grey `unknown` by saturation. The detail tile uses the same fill with dark text
  (`#17330a`), because white on a light green fails contrast; the timeline legend gains the
  `degraded` entry it never had.
- **Discovery.** The missing legend entry was the larger defect: amber was unexplained even
  where it was visible. The palette collision only made it worse.
- **Deviation.** The documents that say "white on a tile" now state the dark type of the
  `degraded` tile; `ADR-0046` keeps its text and gains an `Amended` line.

## Review Notes

- **Round**: 3
- **Review date**: 2026-11-19
- **Verdict**: PASS
- **Self-review disclosure**: round 3 was executed in the same session that implemented
  extension E2, by the agent that wrote the code; the verdict rests on the diff and the
  commands below.
- **Suggested commit**: `feat(web): tell a degraded band apart from a short fault`
- **Quality gate**: tests ✓ (560 unit tests, `tests/simulate.rs` 8, every integration suite, 0 failed) | lint ✓ (`cargo clippy --all-targets -- -D warnings`) | types ✓ (`cargo check`, through clippy) | format ✓ (`cargo fmt --check`)
- **Commands run**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo test --test web a_degraded`, `cargo test --test simulate`, `node spec-plan.mjs validate-for review 020 --strict`
- **Reviewed state**: `23b1d73a5f5c94f33862e4ee2cfcb17e2c6f9962a0212d1885168ea755f936cc`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major (fixed in review) | `src/web/templates/page.html` | The legend did not name `degraded` at all, so an amber band was unexplained even where it was visible. The entry now reads `degraded` with the light-green swatch. | fixed in review |
| 2 | major (fixed in review) | `src/web/page.rs`, `src/web/static/style.css` | `#f9a825` (degraded) and `#ef6c00` (short fault) are a few degrees apart in hue and indistinguishable at the 2 px minimum width. The degraded band is now `#8bc34a`, light green of the reachable family; the detail tile carries dark type for contrast. | fixed in review |
| 3 | minor (fixed in review) | `docs/PRODUCT.md` (`FR-17`) | The page description still listed only green/red/grey bands for `up`/`down`/`unknown`; it now names the light-green `degraded` band and the dark tile type. | fixed in review |
| 4 | nit | `src/web/page.rs` | The legend names the state, but a reader still learns the meaning of "degraded" from the detail page or the guide; a tooltip on the swatch would be a small future improvement. | open (not gating) |

### Verification matrix
| # | Check | Result |
|---|---|---|
| 1 | Goal of E2 achieved | ✓ light green, legend entry, short fault still orange |
| 2 | Scope | ✓ `page.rs`, `page.html`, `style.css`, `detail.rs`, tests, documents |
| 3 | Quality suite green | ✓ |
| 4 | Tests exercise real paths | ✓ the fill, the legend, the tile, the minimum width, the simulation |
| 5 | Existing tests intact | ✓ assertions moved from amber to light green, none weakened |
| 6 | Checklist, questions, notes | ✓ 27/27 |
| 7 | Documentation contracts | ✓ PRODUCT, ARCHITECTURE, USER-GUIDE, site, ADR-0046 amendment |
| 8 | Architecture/ADR consistency | ✓ the `web` guarantee states the new colour and the dark tile type |
| 9 | No scope creep | ✓ operator-requested extension |
| 10 | Edge cases | ✓ the legend swatch no longer satisfies the "a band is drawn" assertion, which now matches `fill="#8bc34a"` only |
| 11 | Security | ✓ presentation only |
| 12 | Security plan checks | not applicable |
| 13 | User guide | ✓ accurate, self-contained, no internal links added |

### Gap analysis
- E2 goals: 3/3 — distinct colour, legend entry, documented.
- Steps: 20/20 plus the phase-1 checkpoint; E1 and E2 complete.
- Scope: within the plan plus the operator-approved extension.
- Deviations: the tile uses dark type; the "white on a tile" sentence is qualified in three
  documents. No other deviation.
- Additions: none beyond E2.
- Deferred: none.

### Known limitations
- Colour remains the primary cue; the lightness difference from `up` is visible in greyscale,
  but a pattern would be needed for a fully colour-independent encoding of every state.
- The legend explains the state only by name; the guide carries the meaning.

### Recommendations for future work
- If the palette grows again, state the chart palette and the legend in one place (a constant
  pair of fill and label) so a new state cannot ship without its legend entry.

### Extension E3 — notes

- **Approved by the operator** on 2026-11-19: the simulated `degraded` must be common enough to
  review in a normal window, and `--degraded-after-ms` must visibly change the board.
- **Decision.** The threshold now drives the rate, not only the latency: `slow_spells` gives
  six windows a day at 250 ms or less, three up to one second, one above (twice for a flapping
  target, none for `AlwaysUp` or `0`). `slow_windows` places that many in the up stretches,
  separated by `up`, so a day can hold several and a stricter threshold holds fewer but never
  none. The RTT of each window stays at or above the threshold.
- **Measured.** Seed 7, the two ICMP targets, the last two days, `step=60`: `--degraded-after-ms
  200` serves 40 `degraded` samples, `2000` serves 6. Before this extension both were the same
  handful (one window on about every other day), which is what the operator saw.
- **Deviation.** The plan's step 16 and its verification text say "one window on about every
  other day"; E3 supersedes that rate, and the documents now state the falling count.

## Review Notes

- **Round**: 4
- **Review date**: 2026-11-19
- **Verdict**: PASS
- **Self-review disclosure**: round 4 was executed in the same session that implemented
  extension E3, by the agent that wrote the code; the verdict rests on the diff, the measured
  counts and the commands below.
- **Suggested commit**: `feat(sim): scale the simulated degraded spells with the threshold`
- **Quality gate**: tests ✓ (560 unit tests, `tests/simulate.rs` 8, every integration suite, 0 failed) | lint ✓ (`cargo clippy --all-targets -- -D warnings`) | types ✓ (`cargo check`, through clippy) | format ✓ (`cargo fmt --check`)
- **Commands run**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo test --test simulate a_simulated_target`, a manual two-server count of `degraded` samples, `node spec-plan.mjs validate-for review 020 --strict`
- **Reviewed state**: `45e7414998a2056a46400174a4b9d0e2426d58f124ba531d6edf69c7872e9f3e`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major (fixed in review) | `src/probe/mod.rs` | `--degraded-after-ms` changed only the latency of a window, never its number or shape, so the parameter looked ignored; the board held one window on about every other day. A 200 ms board now serves 40 `degraded` samples over two days and a 2000 ms board 6, both non-empty. | fixed in review |
| 2 | nit | `src/probe/mod.rs` | The rate steps at 250 ms and 1000 ms are simulator constants, not configuration; they are documented as the demo model and do not claim to mirror a real network. | open (accepted, documented) |

### Verification matrix
| # | Check | Result |
|---|---|---|
| 1 | Operator complaint resolved | ✓ more frequent, and the threshold changes the count |
| 2 | Scope | ✓ `src/probe/mod.rs`, the two simulator tests, the three documents |
| 3 | Quality suite green | ✓ |
| 4 | Tests exercise real paths | ✓ unit count over forty days and an end-to-end sample count |
| 5 | Existing tests intact | ✓ determinism, coverage, first-verdict and fault tests unchanged |
| 6 | Checklist, questions, notes | ✓ 29/29 |
| 7 | Documentation contracts | ✓ PRODUCT, ARCHITECTURE, USER-GUIDE |
| 8 | Architecture/ADR consistency | ✓ ADR-0047's config-gated, deterministic rule holds |
| 9 | No scope creep | ✓ operator-requested extension |
| 10 | Edge cases | ✓ threshold `0`, `AlwaysUp`, no room in a short up stretch, first verdict |
| 11 | Security | ✓ simulation only |
| 12 | Security plan checks | not applicable |
| 13 | User guide | ✓ states that the count falls with the threshold and never reaches zero |

### Gap analysis
- E3 goals: 2/2 — enough spells to review, a threshold that visibly changes them.
- Steps: 22/22 plus the phase-1 checkpoint; E1, E2 and E3 complete.
- Scope: within the plan plus the operator-approved extension.
- Deviations: the rate supersedes the "about every other day" of step 16, recorded in E3 notes.
- Additions: none beyond E3.
- Deferred: none.

### Known limitations
- The rate is a demo heuristic with two steps; it does not model a real RTT distribution, and a
  threshold above one second always yields one window a day rather than none.
- A target drawn as `AlwaysUp` never degrades, keeping the healthy baseline clean.

### Recommendations for future work
- If the simulated rate must mirror a real network more closely, give the target an RTT
  distribution and derive the rate from the distance between it and the threshold; that is more
  faithful but less predictable for a demo, and ADR-0047 does not require it.

## Closure

- **Closed**: 2026-11-19
- **Version**: 0.18.0 → 0.19.0
- **Executed by**: opencode (`deepseek-flash`)
- **Tokens**: unknown
- **Cost**: unknown
