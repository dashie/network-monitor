---
title: Add a simulate command that serves a deterministic history without a database
date: 2026-11-15
status: done
type: feature
impact: high
area: cli, simulation, store contract, probe, web
version_bump: minor
tags: [simulation, cli, determinism, architecture, demo]
depends_on: []
branch_recommended: true
---

## Summary

A new subcommand, `network-monitor simulate --listen <ADDR>`, publishes the same read-only
page, JSON API and CSV export the tool already has — over a history it **generates
deterministically** from the targets found in the configuration, instead of reading a
database. It opens no database file, performs no real probe, and writes nothing anywhere.
`--history <DAYS>d` (default `30d`) states how far back the simulated history goes and
`--seed <N>` (default `0`) fixes the sequence, so the same command line and the same instant
always produce the same answers.

The two real modes — `monitor` and `serve` — do not change at all: same flags, same files,
same behaviour, and no simulation code on their path except the shared read contract they now
go through.

## Why

The page can only be seen against a database that some monitor filled over days. That makes
four things expensive or impossible:

- a demo, a screenshot, a documentation example, or a review of a layout change needs a
  seeded database fabricated by hand and kept somewhere;
- the read-only page and the REST API cannot be exercised without a monitor process or a
  fixture file, so an operator has no way to look at the tool before deploying it;
- `tests/web.rs` builds a fixture database for every behaviour it pins, which couples every
  page test to the store and its schema;
- there is no way to reproduce a reported rendering with a known, shareable dataset: a
  screenshot arrives without the rows that produced it.

A simulation mode fixes all four, but only if it is honest: **deterministic** (same seed, same
data, in any process, on any day), **stateless** (nothing stored, nothing cached, nothing in
memory between requests), and **total** (every route the real page answers answers here too).
Without the last property the mode becomes a second product that drifts from the first.

## Goals

- `network-monitor simulate --listen 127.0.0.1:8660` starts and serves `/`, `/target`,
  `/api/v1/targets`, `/api/v1/faults`, `/api/v1/daily`, `/api/v1/openapi.json` and
  `/export/faults.csv.gz` with data generated from the `[[targets]]` of the configuration;
  a test walks every route and asserts a `200` and a non-empty body.
- `/healthz` answers `ok`, so a deployment check reads the same in both modes.
- No database is touched: started in an empty directory with no `--database` and no
  `database` key, the process creates no file, and `ls` before/after is identical.
- No real probe: the process starts and serves with no raw-socket privilege (the suite already
  runs unprivileged, INV-3) and makes no outbound connection.
- Determinism, three ways, each a test:
  - two processes with the same `--seed` and `--history`, asked for the same explicit
    `?from=…&to=…`, return byte-identical bodies;
  - the same process asked twice returns byte-identical bodies;
  - two different `--seed` values return different bodies for the same window.
- History independence of the window: a UTC day covered by both `--history 30d` and
  `--history 60d`, with the same seed, holds exactly the same faults and the same total down
  time.
- The past is bounded by `--history`: with `--history 30d`, an instant 45 days ago has no
  data — no event, no fault, no band — and the page states `unknown` there rather than
  inventing history. The same holds for the detail page's 360-day selector: the days before
  the window are empty, not generated.
- `--history` above `3660d` is refused with a message stating the maximum, so one request can
  never walk an unbounded number of days.
- INV-1 holds in the generated data: no target has two consecutive events with the same
  state, so the timeline segments never contain two adjacent segments of the same state.
- The real modes are untouched: `monitor`, `serve`, `query` and `import` gain no flag and no
  behaviour change; the whole existing suite passes unmodified except for the mechanical
  signature changes listed below.
- The startup log states the mode, the window, the seed, and that no database is used, in one
  line.

## Non-goals

- No `--simulation` flag on `monitor` or `serve`, and no `simulation`/`seed` keys in the TOML
  file: the mode is a command, not configuration.
- No banner, badge or API field marking simulated data. The page is identical to the real one;
  only the startup log says which mode is running.
- No simulated probe loop and no write path: nothing is "recorded" and no state is kept
  between requests. Time advancing is what makes new data appear.
- No simulated `[telegram]` delivery, no simulated retention, no simulated import.
- `query` and `import` keep reading and writing the real database; they gain nothing.
- No change to the schema, to the CSV columns, or to the OpenAPI document.
- No new crate: the pseudo-random source is written here.

## Affected modules

**Source**
- `src/cli.rs` — the `Simulate` subcommand: `--listen` (required), `--history` (default
  `30d`), `--seed` (default `0`).
- `src/main.rs` — dispatch of the new command; startup log line.
- `src/history.rs` *(new)* — the read contract: `trait History`, `trait HistorySource`,
  `HistoryError`. Every read entry point of the page, the API and the export goes through it.
- `src/store/mod.rs` — `impl History for Store`, `impl HistorySource for DatabaseSource`, and
  `StoreError → HistoryError`; the pure aggregation helpers the simulated side needs, shared
  with the SQL path.
- `src/sim/mod.rs` *(new)* — the generator: which target gets which kind of life, and the
  stitched daily schedule of a target.
- `src/sim/rng.rs` *(new)* — `splitmix64`, the only randomness in the mode.
- `src/sim/history.rs` *(new)* — `impl History for SimulatedHistory` and
  `impl HistorySource for SimulatedSource`, over the generator.
- `src/probe/mod.rs` — the simulated half of the method contract (`simulated_day`).
- `src/probe/icmp.rs`, `src/probe/tcp.rs`, `src/probe/dns.rs`, `src/probe/http.rs` — each
  method's `simulated_day`, so a method's failure vocabulary and latency profile stay in the
  module that owns them.
- `src/web/mod.rs` — handlers take `&dyn HistorySource` instead of `database: &Path`;
  `run`/`serve` signatures follow.
- `src/web/api/mod.rs`, `src/web/api/targets.rs`, `src/web/api/faults.rs`,
  `src/web/api/daily.rs` — take the source, open a `History` per request.
- `src/web/export.rs` — `write_csv_gz` takes `&dyn History`.
- `src/query/mod.rs` — *(maybe)* only if it shares a type that moves; expected unchanged.
- `src/config.rs` — *(maybe)* only if `--history` parsing needs a helper next to
  `parse_millis`; expected unchanged.

**Tests**
- `src/sim/mod.rs`, `src/sim/rng.rs`, `src/sim/history.rs` — *(new)* unit tests: determinism,
  day independence, coverage of the day, INV-1, bounded past, filters and paging.
- `src/store/mod.rs` — a drift guard: the pure helpers and the SQL path return the same rows
  for the same events.
- `tests/simulate.rs` — *(new)* end-to-end: every route answers; two processes, same seed,
  byte-identical; different seed, different body; no file created; `--history` bounds the past.
- `tests/cli.rs` — the new command's parsing: `--listen` required, `--history` default,
  `--seed` default and rejection of a malformed value.
- `tests/web.rs` — *(maybe)* only the helper that starts a server, if its signature changes.

**Documentation / configuration**
- `docs/USER-GUIDE.md` — `How to use it` (the new command and its three options) and
  `Configuration` (the two options with type, default and effect; the fact that no database is
  used). The guide is self-contained, so the simulated mode must be usable from it alone.
- `docs/PRODUCT.md` — a functional requirement for the mode and its determinism.
- `docs/ARCHITECTURE.md` — a `sim` component entry, the `History` contract in the component
  table and the interfaces, the data model untouched, and INV-10 extended with its simulated
  sibling.
- `docs/ADR.md` — ADR-0032: the read contract, the per-day deterministic generator, the
  decision to make it a subcommand, and the rejection of the alternatives.
- `docs/GLOSSARY.md` — **Simulation**, **Simulated history**, **History source**.
- `docs/DEVELOPER-GUIDE.md` — the module map (`src/sim`, `src/history.rs`) and how to run the
  simulated board while developing the page.
- `Cargo.toml` — version `0.7.0`.

## Architecture impact

One new seam and one new component.

The seam: today `web`, `web::api` and `web::export` open a `Store` per request from a path and
call its read methods, and every test that pins a page behaviour must build a database. The
plan introduces `trait History` (the eleven read methods those three call sites use) and
`trait HistorySource` (its factory, because a SQLite connection is per request and not
`Sync`), implemented twice: `DatabaseSource` (the existing code, unchanged in behaviour) and
`SimulatedSource`. After the change `web` depends on `history` and no longer on `store`, which
is the decoupling the operator asked for: the database becomes one implementation of a
contract, not the shape of the page.

The component: `sim`, which owns determinism. Its rule is that the generated data is a pure
function of `(seed, target key, UTC day, instant)`; nothing is stored, nothing is cached
between requests, and the whole history of a request is regenerated each time. The past is
bounded by `--history`; the future needs no writer, because data "arrives" as the clock
advances.

Invariants: INV-10 (the page never writes) is preserved and extended — the simulated source
has no write method at all, so there is nothing to violate. INV-1 must hold in the generated
sequence. INV-2 is unaffected (no new configuration key). INV-3 is strengthened: the simulated
probe is a pure function with no socket.

## Decision-record impact

A new record is required: ADR-0032 — *Serve a deterministic simulated history behind one read
contract*. It records the decision to make the mode a subcommand rather than a pair of global
flags, the per-day seed construction (and why the generated data must not depend on the
requested window), the choice of a hand-written `splitmix64` over a new dependency, and the
rejected alternatives: a seeded fixture database, a `--simulation` flag on `monitor`/`serve`, a
stateless `verdict_at(instant)` probe sampled to find transitions, and marking the page
visually. Amends nothing; complements ADR-0008 (the page is server-side) and ADR-0010.

## Proposed solution

The order is: contract first (nothing user-visible, everything still works), then the
generator, then the command, then the proof.

### Step 1 — Extract the read contract and make the real path use it

Create `src/history.rs` with `HistoryError`, `trait History` (the eleven methods `web`,
`web::api` and `web::export` call today: `states`, `find_target`, `resolve_target`,
`timeline`, `daily_drops`, `faults`, `count_faults`, `recent_faults`, `recent_faults_for`,
`events`, `events_after`), and `trait HistorySource { fn open(&self) -> Result<Box<dyn History>, HistoryError>; }`.
In `src/store/mod.rs` implement both traits for `Store` and `DatabaseSource { path }`, mapping
`StoreError` into `HistoryError` without losing its message. Change `src/web/mod.rs`,
`src/web/api/*` and `src/web/export.rs` to take `&dyn HistorySource` and to open a `History`
per request, as they open a `Store` today.

**Verify:** `cargo test` — the whole suite passes; in particular `tests/web.rs` is unchanged
and still green, and the page's 503 path for an unusable database still yields `503` with the
store's message (a test already covers it). `cargo clippy --all-targets -- -D warnings` is
clean, and `rg "crate::store" src/web` finds only test code.

### Step 2 — Share the aggregation rules between the two implementations

The SQL path aggregates in SQLite and the simulated path must aggregate in Rust without
disagreeing. Move the rules the simulated side needs into pure functions in `src/store/mod.rs`
(next to the existing `timeline_segments`): from a list of events, the faults, the per-UTC-day
buckets and the current state per target; the SQL methods keep their queries and the pure
helpers are used by the simulated implementation. Add a drift guard: a store test seeds a
database with a fixed event list and asserts that `faults`, `daily_drops` and `states` return
exactly what the pure helpers produce for the same list.

**Verify:** `cargo test store` — the new drift-guard test passes, and the existing fault,
daily and state tests are unchanged.

### Step 3 — Write the deterministic source and the per-method simulated schedules

`src/sim/rng.rs`: `splitmix64` seeded from a hashed tuple (seed, target key, day, purpose), so
every draw is reproducible and order-independent. `src/probe/{icmp,tcp,dns,http}.rs`: add
`pub fn simulated_day(endpoint: &Endpoint, seed: u64, day_start_ms: i64) -> Vec<SimulatedEpisode>`
— the simulated form of the probe of that method — where an episode is
`{ state, from_ms, to_ms, rtt_us, cause }` and the causes come from the closed set the method
already owns. `src/sim/mod.rs`: give each target a life from `hash(seed, key)` — always up,
occasional faults, or flapping — and stitch its episodes per UTC day so that the schedule of a
day depends only on `(seed, key, day)` and covers exactly that day. A fault may cross midnight:
the day that begins inside one opens with that fault, which keeps every day self-contained
without clipping real outages at the boundary.

**Verify:** `cargo test sim::rng sim::probe_sim` — a same-seed/same-day call returns the same
episodes; a different seed returns different ones; episodes cover the day with no gap and no
overlap; no two consecutive episodes share a state (INV-1); a down episode always carries a
cause of its own method.

### Step 4 — Implement `History` over the generator

`src/sim/history.rs`: `SimulatedSource { targets: Vec<ConfiguredTarget>, seed, history_days }`
and `SimulatedHistory`, which regenerates the covered days on each call and answers every trait
method through the Step 2 helpers, honouring the filters the real one honours: the half-open
window, `limit`/`offset`, the keyset cursor of `faults`, `target` resolution by label, key or
bare address (the same rule as `Store::resolve_target`), and an instant before
`now - history_days` holding no data. The current state of a target is the last episode at or
before `now`.

**Verify:** `cargo test sim::history` — one test per trait method mirroring the store's own
semantics, plus the two determinism properties (same call twice; same call in two
`SimulatedSource` instances), plus "a day covered by 30d and by 60d is identical".

> **🔶 CHECKPOINT CP-1 — the contract and the generator are stable**
> **Stable state:** `history` is the read seam, `Store` is one implementation and still serves
> the real pages exactly as before; `sim` generates a deterministic, INV-1-clean history and
> answers every read method in unit tests. Nothing user-visible has changed yet, so this is a
> safe point to stop, review the trait shape and the generator's realism, and go back to the
> real modes if the seam is wrong.
> **Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
>
> The implementer must stop here, report, and wait for explicit user confirmation.

### Step 5 — Wire the `simulate` command

`src/cli.rs`: `Simulate { listen: String, history: HistorySpan, seed: u64 }` with `--listen`
required, `--history` defaulting to `30d` and accepting `30d` or a bare `30` (days, 1 to 3660
inclusive), `--seed` defaulting to `0`. `src/main.rs`: resolve the configuration as the other
commands do, require targets (a configuration without targets is refused with the existing
message), build a `SimulatedSource`, and call `web::run(&source, &listen, …)`. Log one `INFO`
line stating the mode, the number of targets, the window, the seed, and that no database is
used. `--database` and `[database]` are accepted and ignored; `[telegram]` is not started.

**Verify:** `cargo run -- --config network-monitor.toml simulate --listen 127.0.0.1:8660`
prints the log line and `curl -s localhost:8660/ | head -3` returns the page; `simulate` without
`--listen` exits with clap's "required argument" error; `--history 0` and `--history 4000d` are refused with a message naming the
accepted range.

### Step 6 — Prove the mode end to end

`tests/simulate.rs` starts the binary twice with a temporary configuration and exercises it as
a client: every route answers `200` with a non-empty body, `/healthz` included; the same URL with an explicit
`?from=…&to=…` yields byte-identical bytes from both processes and on a second request; a
different `--seed` changes the body; a temporary directory is empty after the run; a window
older than `--history` states no fault. `tests/cli.rs` pins the parsing.

**Verify:** `cargo test --test simulate --test cli` passes.

### Step 7 — Document and bump the version

Update the user guide (the command and both options, with defaults), the product requirement,
the architecture (component, contract, invariant), the ADR index and ADR-0032, the glossary
terms, the developer guide's module map, and `Cargo.toml` to `0.7.0`.

**Verify:** `rg -n "simulate|--history|--seed" docs/USER-GUIDE.md` names every option with its
default; `rg -n "0.7.0" Cargo.toml` matches; `cargo test --doc` passes.

### Step 8 — Make the density of faults an option (added after CP-1, at the user's request)

The board was as busy as the constants in `sim` decided, so a quiet board (to work on the
layout) and a board in trouble (to show the fault table and the banner) could not be asked for.
`src/sim/mod.rs`: the rate is the number of faults a day of a target that fails now and then
holds; `Life::weight` scales it (`AlwaysUp` 0, `Occasional` 1, `Flapping` 8), the day draws its
count uniformly in `[0, 2 × expected]`, and a day above `MANY_FAULTS` draws short faults so they
fit. `src/probe/mod.rs` and the four methods: `simulated_day` takes the rate and passes it on.
`src/sim/history.rs`: `SimulatedSource`/`SimulatedHistory` carry it. `src/cli.rs`:
`--fault-rate <faults>`, `FaultRate` newtype, default `3`, `0` to `1000`, anything else refused
with the range. `src/main.rs`: the source is built with it and the startup line states it.

**Verify:** `cargo test --test simulate --test cli sim::` — a rate of `0` serves a board with no
fault and no down time in the API, a higher rate draws more faults than a lower one, a rate that
is negative, infinite or above `1000` is refused, and every determinism test still passes.

## Security considerations

The mode is opt-in at startup, on an interface the operator names, and adds no input parsing on
the request path beyond the query strings the real page already parses and validates. It opens
no file and no socket outward, so it cannot leak a history it does not have. Two points to keep
in mind while implementing:

- the simulated page has the same absence of authentication as the real one; the startup log
  must be loud enough that a simulated board is never mistaken for a real one on a shared host;
- `--seed` and `--history` come from the command line and must be validated (an absurd
  `--history` would make a request walk an unbounded number of days): cap it at a documented
  maximum, and keep the per-request work bounded by the window the page asks for.

## Verification plan

- **Automated:** `tests/simulate.rs` (routes, determinism across processes, no file, bounded
  past), `tests/cli.rs` (parsing and defaults), `src/sim/*` unit tests (determinism, INV-1, day
  coverage, filters and paging), `src/store/mod.rs` drift guard.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- **Manual:** run `simulate --listen 127.0.0.1:8660` on the development machine, open `/`, a
  target page, the API reference and the CSV export; confirm the distribution looks like a
  board (some targets always up, some flapping, faults with plausible durations and causes) and
  that a reload 30 s later shows the same past with a little more present.
- **Rollback:** the command is additive and the only shared change is the read seam of Step 1;
  reverting the commit that introduces `history` restores the previous `web`/`store` coupling
  with no data or schema consequence, since no file format changed.

## Open questions

None: every branch that would change the files, the steps or the verification is decided.

- [x] CLI shape: a dedicated `simulate` verb rather than two global flags.
- [x] `--listen` is required; `--history` defaults to `30d`, `--seed` to `0`.
- [x] `--history` accepts 1 to 3660 days; anything outside is refused at startup.
- [x] No visual marker: the startup log line only.
- [x] Realistic faults, with causes from the existing closed set of each method.
- [x] A fault may cross UTC midnight; the day that starts inside it opens with that fault.
- [x] Days before `now - history` are empty, including in the detail page's 360-day selector.
- [x] `/healthz` answers `ok` in the simulated mode too.
- [x] Distribution of target lives, as the starting point of one constant in `sim`: about
      20 % always up, 70 % with 1–4 faults a day of 20 s to 40 min, 10 % flapping with 10–60
      faults a day. It is a constant, so it can be tuned after CP-1 without touching the shape
      of anything else.

## Implementation checklist

- [x] Step 1 — Extract the read contract and make the real path use it
- [x] Step 2 — Share the aggregation rules between the two implementations
- [x] Step 3 — Write the deterministic source and the per-method simulated schedules
- [x] Step 4 — Implement `History` over the generator
- [x] CP-1 — Stop and confirm the contract and the generator (confirmed by the user: "ok, prosegui")
- [x] Step 5 — Wire the `simulate` command
- [x] Step 6 — Prove the mode end to end
- [x] Step 7 — Document and bump the version
- [x] Step 8 — Make the density of faults an option (`--fault-rate`)
- [x] All open questions resolved (none left; the distribution constant is tunable)
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

### Step 1 — the read contract

- **Decision:** `web::run` and `web::serve` take `Arc<dyn HistorySource>` rather than
  `&dyn HistorySource`. The accept loop spawns a detached thread per connection, so the
  source must be `'static` and shared; `HistorySource` is declared `Send + Sync` for exactly
  this reason. The per-request seam (`Box<dyn History>` handed out by `open()`) is unchanged
  from the plan.
- **Extension (approved by usage of the plan's own `Verify:` line):** `src/history.rs`
  re-exports `TargetState` and `TimelineSegment` in addition to the row types the plan named.
  Without them `src/web/{page,detail,stats}.rs` still imported from `crate::store`, which
  Step 1's verification forbids. `rg "crate::store" src/web` now finds only the test module
  of `src/web/export.rs`, which builds a real `Store` fixture.
- **Change to an existing test helper:** `tests/web.rs::start_with` built a `PathBuf` and
  passed `&database` to `web::serve`. The signature changed, so the helper now passes
  `Arc::new(DatabaseSource::new(database))`. No assertion and no scenario changed, which is
  the condition the plan attached to this file.

### Step 3 — the generator

- **Decision:** the day schedule (`life`, `outages`, `day`) lives in `src/sim/mod.rs`, next to
  the source that will serve it, and not in an extra `src/sim/schedule.rs`: the plan's affected
  modules list only `src/sim/mod.rs` and `src/sim/rng.rs`, and the generator is a hundred lines.
- **Decision:** the failure vocabulary of each method lives in that method's module, but the
  schedule itself does not know what a probe is. `sim::rng` is a leaf, the generator uses only
  it, `probe::{icmp,tcp,dns,http}::simulated_day` dresses the schedule with its own latencies
  and causes, and `sim::history` (Step 4) puts the two together. The dependency therefore runs
  one way: `probe → sim::{rng, generator}` and `sim::history → probe`, with no cycle.
- **Decision:** an ICMP failure can only be `ICMP.NO_REPLY`, because that is the only verdict an
  echo request can state; TCP, DNS and HTTP pick among the three causes their method already
  owns. A simulated failure states the default probe timeout (2 s) through
  `probe::SIMULATED_TIMEOUT`, because the simulator has no configuration to read and the
  message must not print `unknown`.
- **Discovery:** the first cross-method test failed because `life` is drawn once per
  `(seed, key)`, so one seed can be "always up" for every day. The sweep now varies the seed as
  well as the day, which is what a real board of targets looks like anyway.

### Step 4 — the read contract over the generator

- **Decision:** `SimulatedSource` holds `Vec<config::Target>` rather than a new
  `ConfiguredTarget` type. `config::Target` already is the configured target (endpoint, label,
  interface, priority), and a second type with the same four fields would be a copy to keep
  in step.
- **Decision:** the simulated past *opens with a verdict* at `now - history_days`, exactly as
  a monitor that started that many days ago would have recorded `unknown → up|down` as its
  first event. That single rule makes `first_seen_at_ms`, the first lane segment and the
  fault that may be open at the beginning of the past all fall out of the shared helpers
  instead of needing special cases.
- **Decision:** an event's id is `occurred_at_ms * 1000 + target position`, so it does not
  depend on the window that was asked for, it sorts in time order, and the keyset cursors of
  `faults` and of the export walk work unchanged.
- **Decision:** a window is generated with one day of lead-in (so the state at its start and a
  fault that opened earlier are known) and, for faults, one day of lead-out (so a fault that
  ends after the window still states when it ended, as the reader's subquery does).
- **Decision:** `states()` regenerates the whole covered past, because `first_seen_at_ms` is
  the first verdict of the history; `recent_faults*` walks backwards in 30-day steps and stops
  as soon as the page is full, so a quiet board does not always pay for the whole window.
- **Discovery:** MSRV is 1.75, so `Option::is_none_or` (1.82) is not available;
  `map_or(true, …)` is used instead. Clippy's `incompatible_msrv` catches this.

### Step 5 — the command

- **Decision:** `--history` is parsed by a `HistorySpan` newtype in `src/cli.rs` (`30d` or `30`,
  1 to 3660 days), so clap rejects a bad value with its own message and `main` never parses a
  second time.
- **Decision:** the simulated past is anchored to whole UTC days —
  `first_ms = start of today - (days - 1)` — instead of `now - days`. Measured: with the
  sliding anchor the same URL returned a body that differed by the milliseconds between the
  two requests, because the *beginning* of the simulated past moved with the clock. Anchored
  to the day, the same URL is byte-identical across requests and across two processes of the
  same seed. `--history 30d` therefore reads as "the thirty days that end today".
- **Discovery:** the `\` line continuation of a long Rust string literal is stripped by the
  Python heredoc used to write the file, leaving the spaces inside the literal. The log line
  was written on one line instead.

### Step 6 — the end-to-end proof

- **Discovery:** the answers that state the clock cannot be compared byte for byte, and that is
  not a defect of the simulation: `meta.generatedAt` is the instant of the request, and
  `upForMs` (API) and "up for" (page) are the *age* of a state, which moves between two requests
  of the same process. `tests/simulate.rs` compares the faults and the daily histogram of a
  window of whole days that has already happened — `from` and `to` fixed, no clock in the
  answer — and drops the `generatedAt` line.
- **Decision:** the parsing of the new command is pinned twice on purpose: `src/cli.rs` for the
  values, `tests/cli.rs` for what the operator sees (the required `--listen`, the range in the
  message, "no targets configured", and the startup line — read on a run that then fails to
  bind, so the test needs no long-lived process).

### Step 7 — the documentation

- **Decision:** the simulated board is documented as its own subsection of the guide's command
  line (`Simulated mode`), with the two options and their defaults in a table, because that is
  where a reader looks for a command. The `database` row of the global keys states that
  `simulate` ignores it.
- **Decision:** the developer guide gains a "Without a network: the simulated board" recipe next
  to the other run recipes, naming the constants that make the simulation plausible, so tuning
  it does not require reading `sim`.
- `INV-13` is the simulated sibling of `INV-10`, and `INV-10` points at it.

### Step 8 — the fault rate (an extension)

- **Extension, approved by the user in this conversation:** after CP-1 the user asked for a
  parameter that makes the faults more or less frequent, chose a number over a named level
  ("`--fault-rate`"), and asked for this plan to be updated rather than a new one opened. The
  goals, the seam, the steps above and their verification are untouched; `--fault-rate` is the
  one input of the generator that used to be a constant, and it is documented as such.
- **Decision:** the rate is stated against a target *that fails now and then*, and the life of a
  target scales it (never fails 0, typical 1, flapping 8). That keeps the heterogeneity that
  makes a board look real — one host rock solid, one flaky — while giving one number to turn.
  A board-wide average would have been a second constant to explain and would have removed the
  reason the mixture exists.
- **Decision:** the daily count is drawn uniformly in `[0, 2 × expected]`, whose mean is the
  expected itself, so a day still differs from the next without a distribution to import; a day
  above eight faults draws faults of at most five minutes so the day can hold them.
- **Decision:** the default is `3`, which keeps the board close to what CP-1 was reviewed with
  (about 45 faults a day over seven targets) and reads as a round number. The old fixed ranges
  (1–4 and 10–60 a day) are gone: they were the constant this step replaces.
- **Discovery:** clap refuses `--fault-rate -1` before the parser sees it, as an unknown flag, so
  the process-level test states the upper bound and the malformed forms, and the unit test of
  `FaultRate` states the negative one.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-15
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(cli): serve a deterministic simulated history with simulate`
- **Quality gate**: tests ✓ (324 + 23 + 7 + 1 + 18 + 6 + 5 + 6 + 6 + 4 + 59 passed, 0 failed) | lint ✓ (0 warnings) | types ✓ — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/sim/history.rs` | `SimulatedHistory::rows()` regenerates the **whole covered past** into a `Vec<Event>` on every `states()`/`find_target()`, so `/`, `/target` and `/api/v1/targets` cost `history_days × fault_rate × targets` and ignore the window the page asks for. Measured with the two documented maxima, `--history 3660d --fault-rate 1000`: `/` answered `200` in **70.9 s** with **1.71 GB** RSS; `--fault-rate 100` → 10.8 s, 1.50 GB; the defaults (`--history 3660d`, rate 3) → 0.53 s, 89 MB. The plan's Security considerations require "the per-request work bounded by the window the page asks for", so this is an unmet constraint, not a preference. The fix is local: the current row needs `first_seen_at_ms = first_ms()` and `episode_at(target, now_ms)` — one day — not the past. | open |
| 2 | minor | `README.md` | A new subcommand is a run step, and the Quick start named `monitor`, `serve`, `query` and `import` but not `simulate`. Added after the `serve` block: `# Look at that page with no network and no database: a generated history, the same for the same seed` + `./target/release/network-monitor simulate --listen 127.0.0.1:8660 --history 30d --seed 0`. | fixed in review |
| 3 | minor | `docs/ARCHITECTURE.md` | Two sentences the change made wrong: §1 "One invocation is either a long-running `monitor` … or a short-lived `query`" now reads "… or one of the short-lived commands — `serve` and `simulate` that read a history and publish it, `query` and `import` that read or restore the database"; §2 "**No listening socket.** The process opens no port; the only integration surface is the SQLite file…" now reads "**No listening socket unless asked.** The process opens a port only when `--listen` is given (INV-11)…". | fixed in review |
| 4 | minor | `tech-plans/007-simulate-mode.md` (Goals) | The first goal names `/api/v1/daily`; the route is `/api/v1/faults/daily`, which is what `tests/simulate.rs` walks and what the goal intends. Plan text; the reviewer does not edit the plan. | open (plan wording) |
| 5 | minor | `src/web/templates/detail.html`, `src/web/detail.rs`, `tests/web.rs`, `docs/USER-GUIDE.md`, `docs/ARCHITECTURE.md` | The tree carried an unrecorded change the plan never specified: the detail page's way back reads `HOME` instead of `targets`, which failed the two assertions in `src/web/detail.rs` (602, 873) and `tests/web.rs` (659). At the user's explicit decision in this round it is **part of the change**: the template (already staged by the user), the three assertions, the user guide's `HOME • <label> (<address>)` line and the `web` contract in the architecture now agree. Recorded here because it changes a user-visible label and because it edits existing tests, which a review does not do on its own. | fixed in review (approved by the user) |
| 6 | nit | `docs/USER-GUIDE.md` | "What the tool does" is four source lines; the guide rule asks for at most three. Pre-existing and untouched by this change. | open (known) |

Scope, from `git diff --name-only origin/main` (35 files) against `## Affected modules`: every file is listed except three categories, each accounted for — `Cargo.lock` (generated by the version bump, in scope by convention), `src/lib.rs` (registering `pub mod history; pub mod sim;`, the mechanical consequence of a new module), and `src/web/{page,detail,stats}.rs` (import-only, from `crate::store` to `crate::history`, required by this plan's own Step 1 `Verify:` line and recorded in the Implementation Notes). `src/web/templates/detail.html` is the exception of finding 5.

### Gap analysis
- Goals: 10/11 achieved, 1 partial — "The real modes are untouched: … the whole existing suite passes unmodified": the suite is unmodified except the mechanical signature change **and** the three assertions of the approved `HOME` label (finding 5). Every other goal has its evidence: `tests/simulate.rs::every_route_answers_over_the_simulated_history` (routes), `…the_simulated_mode_writes_nothing` (no file), `…two_processes_of_the_same_seed…` and `src/sim/history.rs::tests::the_same_call_twice…`/`two_sources_of_the_same_seed…` (determinism), `…a_day_covered_by_thirty_days_is_the_same_day_covered_by_sixty` (history independence), `…a_window_before_the_simulated_past_states_no_fault` (bounded past), `cargo test --test cli simulate_refuses_a_history_outside_the_simulated_range` (refusal above 3660d), `src/sim/mod.rs::tests` and `sim::probe_sim` (INV-1), `the_startup_log_states_the_simulation_and_that_no_database_is_used` (one log line).
- Scope: in scope, with the three mechanical categories above.
- Deviations: all nine recorded in `## Implementation Notes` are legitimate; `--fault-rate` is Step 8, an extension the user asked for and approved in this conversation, and the whole-past `states()` is the one deviation that the review rejects (finding 1).
- Additions: `--fault-rate` (Step 8, approved); the three diagrams of `docs/ARCHITECTURE.md` converted from Mermaid to PlantUML at the user's request, documentation only, verified by rendering them with PlantUML 1.2026.10 (C4 stdlib resolves offline); `INV-13`.
- Deferred: none.
- Executed evidence beyond the suite: the binary run by hand on `127.0.0.1:8660` (`/`, `/target`, `/api/v1/*`, `/api/v1/docs`, `/export/faults.csv.gz` → 2 307 rows, `/icon.*`, `/style.css`, `/app.js`), `--fault-rate 0|3|20` measured at 0 / 136 / 660 faults over three days on seven targets, and the timings of finding 1.

### Known limitations
- Finding 1: the work a page costs is bounded by `--history` and `--fault-rate`, not by the window. Acceptable at the defaults (0.53 s for the maximum history), not at the documented maxima together.
- The simulated past is anchored to whole UTC days, so the first day of the window enters the past at midnight and a request that spans midnight sees a different beginning than one a minute earlier.
- How many faults a day can hold is capped by the durations: above roughly 500 faults a day per target the drawn faults overlap and are dropped, so `--fault-rate` saturates rather than growing without limit.

### Recommendations for future work
- Fix finding 1 with `episode_at` + `first_ms()`, and pin it with a test that asks `/` for `--history 3660d --fault-rate 1000` and asserts an answer inside a second.
- `events_after` still walks the whole past to serve an export page; the same treatment (generate the page's days only) would make the export of a long history cheap.
- `sim::Life` and the weights it scales are the remaining constants of the generator; if a board ever needs per-target rates, that is the place.

### Review round 1 — finding 1 (the cost of `states()`)

- **Fix:** `SimulatedHistory::rows()` no longer regenerates the covered past. It reads the
  episode each target is inside now (`episode_at`, one day) and, for `last_change_at_ms`, the
  start of the *run* that episode belongs to: a day is cut at midnight whether or not anything
  happened there, so the row walks back day by day while the state holds (`run_start_ms`), and
  a target whose `Life` is `AlwaysUp` is known to be up since `first_ms()` without walking at
  all. `first_seen_at_ms` is `first_ms()`, which is where the history — and its opening
  verdict — begins.
- **Why the walk is not a reintroduction of the finding:** it is bounded by the length of the
  current run, not by `--history`. A target that fails has a run of a few days at most; the
  only long runs belong to targets that never fail, which the `Life` shortcut answers in
  constant time.
- **Measured, same machine, `--history 3660d --fault-rate 1000`:** `/` 70.9 s → **0.85 s**
  (RSS 1.71 GB → 126 MB); `/api/v1/targets` 68.5 s → **0.06 s**; `/target` **1.86 s**, which is
  the cost of the 30 days the page draws at that density, not of the past. With the same
  density over `--history 30d`, `/` costs 0.86 s: the cost no longer follows the history.
- **Tests added:** `sim::history::tests::the_last_change_of_a_row_is_the_start_of_the_run_the_timeline_draws`
  (cross-checks the row against the independent timeline path),
  `…::a_target_that_never_fails_has_been_up_since_the_history_began` (3 660 days, a wall-clock
  guard and the exact `last_change_at_ms`), and
  `tests/simulate.rs::the_page_answers_at_the_widest_settings` (the three page routes answer
  under five seconds at `--history 3660d --fault-rate 1000`).
- **Not addressed, by design:** finding 4 is wording inside `## Goals` of this plan, which the
  implementer may not edit; finding 6 is a pre-existing line count in the user guide.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-15
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(cli): serve a deterministic simulated history with simulate`
- **Quality gate**: tests ✓ (326 + 23 + 7 + 1 + 18 + 6 + 5 + 6 + 7 + 4 + 59 passed, 0 failed) | lint ✓ (0 warnings) | types ✓ — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/sim/history.rs` | Round 1: `rows()` regenerated the whole covered past. **Verified fixed**: `rows()` reads the episode the target is inside now and `run_start_ms` walks back only while the run holds (with an `AlwaysUp` shortcut). Re-measured by hand at `--history 3660d --fault-rate 1000`: `/` 70.9 s → **0.89 s**, `/api/v1/targets` 68.5 s → **0.07 s**, `/target` **1.87 s** (30 days of a board that fails 500 times a day), RSS after the page 126 MB. With the same density over `--history 30d`, `/` costs 0.86 s: the page no longer follows the history. Two new unit tests cross-check the row against the timeline and pin the always-up case; `tests/simulate.rs::the_page_answers_at_the_widest_settings` guards it end to end. | fixed in round 2 |
| 2 | blocker | `src/sim/history.rs`, `src/web/export.rs` | The same rule is still broken by the methods that take no window: `faults()` and `count_faults()` default to `[first_ms, now)`, `events()` to the whole past, and `events_after()` always starts at `first_ms()`. Each calls `transitions()`, which materialises every event of the covered past. Measured at `--history 3660d --fault-rate 1000`: `GET /api/v1/faults` (no parameter — the request a dashboard makes, and the one the README shows) **did not answer within 120 s** and drove RSS to **4.16 GB**; `GET /export/faults.csv.gz` did not answer within 60 s (10 bytes sent) and drove RSS to **5.28 GB**. At the documented maximum history with the default rate, `GET /api/v1/faults` answers in **1.66 s** for **100 rows**: the work follows `--history`, not the answer. The surface is unauthenticated with `Access-Control-Allow-Origin: *`, so one anonymous request can exhaust the process on a documented configuration — an unbounded-input weakness against the plan's own constraint ("the per-request work bounded by the window the page asks for"). | open |
| 3 | minor | `docs/ARCHITECTURE.md` (`sim` guarantees) | Once finding 2 is fixed, the `sim` contract must state the property finding 1 introduced and finding 2 completes: a request costs what it returns, not the covered past. Deliberately not written while it is false. | open (with finding 2) |
| 4 | minor | `tech-plans/007-simulate-mode.md` (Goals) | Carried from round 1: the first goal names `/api/v1/daily`; the route is `/api/v1/faults/daily`. Plan text. | open (plan wording) |
| 5 | nit | `docs/USER-GUIDE.md` | Carried from round 1: "What the tool does" is four source lines; the rule asks for at most three. Pre-existing. | open (known) |

Scope: unchanged from round 1 — 35 files against `## Affected modules`, with `Cargo.lock` (generated by the version bump), `src/lib.rs` (module registration) and the import-only edits in `src/web/{page,detail,stats}.rs` accounted for in the Implementation Notes, plus the approved `HOME` label of round 1. This round added no production file: `src/sim/history.rs`, `tests/simulate.rs` and this plan.

### Gap analysis
- Goals: 10/11 achieved, 1 partial — the same partial as round 1, unchanged (the approved `HOME` label inside "the real modes are untouched"). Every other goal still has its evidence, and finding 1's fix keeps `up for` truthful: `the_last_change_of_a_row_is_the_start_of_the_run_the_timeline_draws` proves the row agrees with the timeline about when the run began.
- Scope: in scope.
- Deviations: the round-1 fix is recorded in the Implementation Notes with its measurements and its reasoning; `--fault-rate` remains the approved Step 8.
- Additions: none this round beyond the fix and its three tests.
- Deferred: none.
- Steps: 8/8 `done` at the checklist level; finding 2 is a defect of the delivered behaviour, not an unticked step, which is why the checklist stays 14/14 and the verdict is still `NEEDS WORK`.

### Known limitations
- Finding 2: an unbounded fault or event query, and every export page, still cost the covered past.
- The simulated past is anchored to whole UTC days, so a request that spans midnight sees a different first day than one a minute earlier.
- Above roughly 500 faults a day per target the drawn faults overlap and are dropped, so `--fault-rate` saturates rather than growing without limit.

### Recommendations for future work
- Fix finding 2 with one rule: **the work of a request is proportional to what it returns**. Walk back in steps from `now` until the page (or the count's window) is full, the way `newest_faults` already does; count faults per day arithmetically instead of through `Event`s; start `events_after` at `day_start_of(after_id / 1_000)`, which is the instant the id already encodes. Then add `GET /api/v1/faults` with no parameters and one export page to `the_page_answers_at_the_widest_settings`, and write finding 3.
- After that, the remaining cost of the widest settings is the detail page's 30 days at that density (1.87 s), which is the requested window and acceptable.

### Review round 2 — finding 2 (an answer costs what it returns)

- **Fix, one rule applied everywhere:** every read of the simulated source is now built from
  **one day at a time**. `faults` fills its page walking back from the newest day and stops
  when the page is full; `events` does the same; `events_after` starts at the day the cursor
  names, because an id states its instant (`id >> TARGET_SHIFT`); `daily_drops` builds each
  bucket from the instants of the faults of that day and the day before; `count_faults` counts
  the draws of each day and keeps no fault, no event and no string. The four reads therefore
  hold a page or a day, never the covered past.
- **Decision:** `event_id` moved from `instant × 1 000 + target` to
  `instant << 12 | target`, so the instant can be read back exactly and an export page costs
  that page. The shift gives room for 4 096 targets, four times what a process with one thread
  per target can run. The ids are simulated and stored nowhere, so no history changed.
- **Decision:** a day's faults are now capped at `MAX_FAULTS_PER_DAY` (`DAY_MS / 20 s`): a rate
  above that describes a board that is down, and the extra draws used to be dropped as
  overlapping anyway. The cap makes the highest rate cheap and the board honest.
- **Decision:** the histogram rule grew a sibling, `store::daily_drops_from_spans`, taking only
  `(started_at, ended_at)`; `daily_drops_from` delegates to it. A histogram must not need the
  labels and the causes of the faults it counts, and the shared rule still lives in one place,
  which the drift guard keeps in check.
- **Bug found and fixed by the new test `the_count_of_a_window_is_the_page_over_it`:** the count
  and the page disagreed twice — once because the count ignored the fault of the day before the
  window, and once because both walks counted faults that had not happened yet when the window
  reached into the future. `faults` and `count_faults` now clamp their window to
  `[first_ms, now]`.
- **Discovery:** `day()` took the spill of the previous day from the faults the day *kept*
  rather than from the ones it *drew*; when every fault of a day was swept away by the fault
  before it, the day had no spill and the next day opened `up` inside a running fault. The spill
  now comes from the draws, and the day, the count and the episodes agree.
- **Measured by hand, `--history 3660d --fault-rate 1000`, before → after:**
  `/` 0.89 s → **0.07 s**; `/api/v1/targets` → **0.05 s**; `/api/v1/faults` (no window)
  ≥120 s and 4.16 GB → **11.1 s and 19 MB**, and the same route over 30 days → **0.20 s**;
  `/api/v1/faults/daily` 1.23 s → **0.82 s**; `/target` 1.87 s → **0.51 s**; the first 100 KB of
  the export ≥60 s and 5.28 GB → immediate. The 11 s that remain are the count of ~12.8 M
  faults, which is what a client asks for when it states no window; the memory is flat.
- **Tests added:** `the_count_of_a_window_is_the_page_over_it` (twelve three-day windows, count
  against page); `tests/simulate.rs::every_answer_costs_what_it_returns_at_the_widest_settings`
  extended to `/api/v1/faults/daily`, a bounded fault window, the unbounded fault list and the
  first page of the export.

## Review Notes

- **Round**: 3
- **Review date**: 2026-11-15
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(cli): serve a deterministic simulated history with simulate`
- **Quality gate**: tests ✓ (327 + 23 + 7 + 1 + 18 + 6 + 5 + 6 + 7 + 4 + 59 passed, 0 failed) | lint ✓ (0 warnings) | types ✓ — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | blocker (round 2) | `src/sim/history.rs`, `src/web/export.rs` | Round 2: `faults`, `count_faults`, `events` and `events_after` regenerated the whole covered past. **Verified fixed**, measured by the reviewer at `--history 3660d --fault-rate 1000`: `/` 0.16 s, `/api/v1/targets` 0.08 s, `/api/v1/faults/daily` 0.83 s, `/target?days=360d` 4.6 s, `/api/v1/faults` over 30 days 0.21 s, the first 100 KB of the export immediate, and a full export streamed for 16 s at a flat **24 MB**. The process stayed between **11 MB and 159 MB** across every route (4.16 GB and 5.28 GB before). | fixed in round 3 |
| 2 | major | `src/sim/history.rs:636` | `count_faults` walks its days with `let mut spill_end = i64::MIN`, so the first day of the window is counted from `raw_outages` without the spill the day before applies. `outages()` — and therefore the page — drops a fault that starts while the fault of the day before is still running; the count keeps it. The `total` of a page is then larger than the number of faults that page can ever reach. **Proved black-box on the API**, not by reading: `--history 20d --fault-rate 500`, windows that open at midnight and last two hours, `target=icmp://1.1.1.1` — 3 of 24 windows disagree, `total` 20/27/25 against 19/26/24 rows. The same 30 windows opened mid-day agree, which is why the unit test and the round-2 measurements missed it. | open |
| 3 | minor | `src/sim/history.rs` (tests) | `the_count_of_a_window_is_the_page_over_it` cannot see finding 2: it uses `history(30, 11)` at the default rate over windows that start mid-day, where a spill never coincides with the window's first day. The property needs a high rate and midnight-aligned windows. | open (with finding 2) |
| 4 | minor | `tech-plans/007-simulate-mode.md` (Goals) | Carried from rounds 1 and 2: the first goal names `/api/v1/daily`; the route is `/api/v1/faults/daily`. Plan text. | open (plan wording) |
| 5 | nit | `docs/USER-GUIDE.md` | Carried: "What the tool does" is four source lines; the rule asks for at most three. Pre-existing. | open (known) |

Documentation: nothing to correct this round. The `sim` guarantee written in round 3 ("an answer costs what it returns … the one answer that is a count over the whole window it was given stays a scan of arithmetic") was checked against the routes and holds; the user guide was checked again and still states the two options with the defaults and the ranges the code reads (`30d`, `0`, `3`, `1`–`3660`, `0`–`1000`), remains self-contained and untouched.

### Gap analysis
- Goals: 10/11 achieved, 1 partial — the approved `HOME` label inside "the real modes are untouched", unchanged from round 1.
- Scope: in scope. 36 files against `## Affected modules`, with `Cargo.lock` (generated), `src/lib.rs` (module registration), the import-only edits in `src/web/{page,detail,stats}.rs`, and the approved `HOME` label accounted for in the notes.
- Deviations: the round-3 rewrite is recorded in the Implementation Notes with its measurements; the `event_id` shift, the `MAX_FAULTS_PER_DAY` cap and `daily_drops_from_spans` are decisions, all written down.
- Additions: none this round beyond the fix, the cap and the tests.
- Deferred: none.

### Known limitations
- Finding 2: the count of a window is off by the faults swept at the window's first day.
- An unbounded fault list still costs the window it was given: 11 s at `--history 3660d --fault-rate 1000` for the count of ~12.8 M faults, with flat memory. Bounding `from`/`to` costs a client nothing and takes the same request to 0.2 s.
- The simulated past is anchored to whole UTC days, so a request that spans midnight sees a different first day than one a minute earlier.
- Above roughly 500 faults a day per target the drawn faults overlap, so `--fault-rate` saturates; the cap in `MAX_FAULTS_PER_DAY` states the ceiling.

### Recommendations for future work
- Fix finding 2 by starting the walk with the spill of the day before the window, exactly as `outages()` does; then extend finding 3's property test to a high rate and midnight-aligned windows, which is the class of case that has now escaped twice.
- **Escalation, three rounds in:** rounds 1 and 2 were each a boundary the previous round's tests did not reach — first the whole past, then the edges of a day. Both times the defect was in a rule that exists twice (the count's walk and the page's walk). The structural remedy is to make the count use the same primitive as the page — `outages()` for the day — and to keep one property test (`total` equals the page over the same window) that sweeps several rates and window alignments, instead of asserting on one fixture.

### Review round 3 — finding 2 (a fault is one fault)

- **Fix, in two halves, because the review's single symptom had two causes:**
  1. `count_faults` walked its first day with `spill_end = i64::MIN`, so it counted the fault
     that the spill of the day before sweeps away and stated a `total` the page could never
     reach. It now starts from `raw_outages(first_day - DAY)`, which is exactly the rule
     `outages()` applies.
  2. The fault **list** took its starts from the episodes, and a day that begins inside a fault
     opens with a `down` stretch that starts at midnight — so the same fault was listed twice,
     once where it began and once at the midnight it ran through. `day_faults` now takes the
     starts from the draws of the day (`outages`), the one list of faults there is, and reads
     the episode only for what it alone states: the cause. This is also what makes the count
     and the list agree by construction rather than by two implementations of one rule.
- **Decision:** `store::FaultEpisode::into_fault` is removed — with the fault list built from
  the draws, nothing builds a `Fault` from an episode any more. `fault_episodes` stays: it is
  the pure rule the drift guard checks the SQL fault query against, which is what Step 2 asked
  of it.
- **Evidence the fix holds, measured on the API by the reviewer's own probe, repeated by the
  implementer: `--history 20d --fault-rate 500`, three targets, 24 windows of two hours that
  open exactly at midnight, plus 66 windows that do not: 138 windows, 0 disagreements, and
  **0** faults listed at exactly a midnight (1 before). The histogram of each of 7 days equals
  the length of that day's fault list.
- **Tests:** `the_count_of_a_window_is_the_page_over_it` now sweeps three rates and three
  window alignments inside the day (midnight, one hour in, midday) over six windows each — and
  it is a real regression test: with the count's fix reverted it fails
  (`78` against `71` at rate 500, midnight). `a_fault_is_listed_once_in_the_day_it_began` pins
  the second half: every listed fault is a start the day drew, and no id is listed twice.
- **Measured after the fix, `--history 3660d --fault-rate 1000`:** `/` 0.13 s,
  `/api/v1/targets` 0.06 s, faults over 30 days 0.19 s, the histogram 0.89 s, `/target` 0.42 s,
  the unbounded fault list 11.2 s (its `total` of ~12.8 M faults), RSS 15 MB.

## Review Notes

- **Round**: 4
- **Review date**: 2026-11-15
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(cli): serve a deterministic simulated history with simulate`
- **Quality gate**: tests ✓ (328 + 23 + 7 + 1 + 18 + 6 + 5 + 6 + 7 + 4 + 59 passed, 0 failed) | lint ✓ (0 warnings) | types ✓ — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major (round 3) | `src/sim/history.rs` | Round 3: the count and the list disagreed on a window boundary, and a fault crossing midnight was listed twice. **Verified fixed** by the reviewer's own probes, not by reading: `--history 20d --fault-rate 500`, 3 targets + an aggregating query, 4 window shapes (midnight-aligned, midday-aligned, a whole day, a window straddling midday), 15 days each — 240 windows, **0** disagreements once the probe's own `limit=1000` cap is accounted for; a whole day's `total` equals the sum of six four-hour windows (30 cases, 0 bad); paging with `offset` walks the same list as one page (30 cases, 0 bad); two processes of the same seed answer identically. | fixed in round 4 |
| 2 | major | `src/sim/history.rs:266` | `day_faults` states `ended_at_ms: Some(outage.to_ms)` for every fault, so the simulated source never states an open fault. `store::Fault` documents the opposite — "`None` while the target is still down" — and the store test `recent_faults_are_newest_first_and_keep_the_open_one` pins it. Observed on the API: the newest fault of `dns://example.com@9.9.9.9` states `endedAt` `16:37:22` with `ongoing: false` while the clock said `16:36:50`. The user guide promises the fault table shows "and counting" while a target is still down, which in the simulated mode can never happen: every row carries a finished duration, and the running outage states an end in the future. | open |
| 3 | minor | `tech-plans/007-simulate-mode.md` (Goals) | Carried from rounds 1–3: the first goal names `/api/v1/daily`; the route is `/api/v1/faults/daily`. Plan text. | open (plan wording) |
| 4 | nit | `docs/USER-GUIDE.md` | Carried: "What the tool does" is four source lines; the rule asks for at most three. Pre-existing. | open (known) |

Documentation: nothing to correct. The `sim` guarantee of round 3 was re-read against the routes and holds; the user guide still states the three options with the defaults and ranges the code reads (`30d`, `0`, `3`, `1`–`3660`, `0`–`1000`) and remains self-contained. Finding 2 is a defect of the code against the guide, not a stale guide.

### Gap analysis
- Goals: 10/11 achieved, 1 partial — the approved `HOME` label inside "the real modes are untouched", unchanged since round 1.
- Scope: in scope, unchanged.
- Deviations: all recorded in the Implementation Notes; the round-3 fix removed `FaultEpisode::into_fault` because nothing built a `Fault` from an episode any more, which is written down.
- Additions: none.
- Deferred: none.
- Steps: 8/8, checklist 14/14. Finding 2 is a defect of the delivered behaviour, not an unticked step.

### Known limitations
- Finding 2: the running fault states an end and a finished duration instead of "and counting".
- An unbounded fault list costs the window it is given: 11.1 s at `--history 3660d --fault-rate 1000` for the count of ~12.8 M faults, memory flat at 15 MB; bounding `from`/`to` takes the same request to 0.19 s.
- The simulated past is anchored to whole UTC days.
- `--fault-rate` saturates at `MAX_FAULTS_PER_DAY`.

### Recommendations for future work
- Fix finding 2 with one line — `ended_at_ms: (outage.to_ms <= self.now_ms).then_some(outage.to_ms)` — and a test that a fault still running at `now` states no end, mirroring the store's `recent_faults_are_newest_first_and_keep_the_open_one`. While there, check the neighbouring semantics of the type: `first_seen_at_ms`, `last_rtt_us` and `state_since` were each verified, `ended_at_ms` was not.
- **Escalation, four rounds in.** Each round closed one class and the next found its neighbour: the whole past (1), the edges of the past (2), the edges of a day (3), and now the semantics of the type returned rather than the window arithmetic (4). The pattern is that the simulated source was verified *by measurement on the routes the plan named*, and each new measurement class found one. The remedy that has actually worked is a property compared against an independent path — the timeline against the row, the count against the page, the list against the store's own contract — so the last remaining check should be a test that walks **every field of every DTO** the contract returns and compares it with the store's stated meaning, rather than one more hand-picked case.

### Review round 4 — finding 2 (the fault that has not ended)

- **Fix:** one statement of when a fault ended, `SimulatedHistory::ended_at`, used by the two
  readers that state it: `day_faults` (the fault list and the export) and the spans of
  `daily_drops` (the histogram). A fault whose drawn end is still in the future has not ended,
  so it states none, and the histogram measures no down time in the time that has not happened
  — which is the rule `store::Fault` documents and `FR-26` requires of a window that reaches
  into the future.
- **Test:** `a_fault_still_running_states_no_end_and_no_future_down_time` builds a history with
  `now` standing inside a fault, asserts the running fault states no end, that the histogram of
  a window reaching six hours into the future measures no more down time than has happened,
  and that the same fault states its end once `now` is past it. Reverting the fix fails it.

## Review Notes

- **Round**: 5
- **Review date**: 2026-11-15
- **Verdict**: PASS
- **Suggested commit**: `feat(cli): serve a deterministic simulated history with simulate`
- **Quality gate**: tests ✓ (329 + 23 + 7 + 1 + 18 + 6 + 5 + 6 + 7 + 4 + 59 passed, 0 failed) | lint ✓ (0 warnings) | types ✓ — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major (round 4) | `src/sim/history.rs` | Round 4: every simulated fault stated an end, so a running outage showed a future end and a finished duration, and the guide's "and counting" was unreachable. **Verified fixed** by the reviewer on a live board (`--fault-rate 1000`, `--history 20d`): four targets were down at the moment of the probe, and **all four** stated their newest fault with `ongoing: true`, `endedAt: null`, `endedAtMs: null`; the page rendered "and counting". The histogram of a window reaching six hours into the future measured **0** ms of down time beyond the present for those four targets. | fixed in round 5 |
| 2 | minor | `tech-plans/007-simulate-mode.md` (Goals) | Carried from rounds 1–4: the first goal names `/api/v1/daily`; the route is `/api/v1/faults/daily`. Plan text, which the implementer and the reviewer both may not edit. | open (plan wording, not gating) |
| 3 | nit | `docs/USER-GUIDE.md` | Carried: "What the tool does" is four source lines; the rule asks for at most three. Pre-existing and unrelated to this change. | open (known) |

Regression checks repeated by the reviewer in this round, independently of the implementation's report: 144 fault windows across four window shapes, three targets and the aggregating query — `total` equals the page everywhere; a whole day's `total` equals six four-hour windows; `offset` paging walks the same list as one page; two processes of the same seed answer identically; `/` 0.08 s, `/api/v1/targets` 0.06 s, faults over 30 days 0.19 s, the histogram 0.86 s, `/target` 0.46 s, the unbounded fault list 11.1 s (its count of ~12.8 M faults) and a flat 15 MB of memory. Documentation needed no correction: the guide's "and counting" and the store's "`None` while the target is still down" are now what the simulated source does.

### Gap analysis
- Goals: 11/11 achieved — the one partial of rounds 1–4 (the approved `HOME` label inside "the real modes are untouched") is recorded in the Implementation Notes and approved by the user, so nothing is missing.
- Scope: in scope. 36 files against `## Affected modules`; `Cargo.lock` (generated by the version bump), `src/lib.rs` (module registration) and the import-only edits in `src/web/{page,detail,stats}.rs` are accounted for, and the `HOME` label is the one extension, approved in round 1.
- Deviations: all recorded — the `Arc<dyn HistorySource>` signatures, the whole-UTC-day anchor, `Vec<config::Target>`, the event-id shift, `MAX_FAULTS_PER_DAY`, `daily_drops_from_spans`, the removal of `FaultEpisode::into_fault`, and the fault list reading the draws rather than the episodes.
- Additions: `--fault-rate` (Step 8, approved), the PlantUML diagrams (documentation only, requested), `INV-13`.
- Deferred: none.
- Steps: 8/8 done, checklist 14/14, no open question.

### Known limitations
- An unbounded fault list costs the window it is given: 11.1 s at `--history 3660d --fault-rate 1000` for the count of ~12.8 M faults, with flat memory; a client that states `from`/`to` gets the same request in 0.19 s.
- The simulated past is anchored to whole UTC days, so a request that spans midnight sees a different first day than one a minute earlier.
- `--fault-rate` saturates at `MAX_FAULTS_PER_DAY`, and the whole simulation is a set of constants at the top of `src/sim/mod.rs`, deliberately tunable.

### Recommendations for future work
- The `sim` component was found defective four times, each time on a class the previous round's tests did not reach. The remedy that worked was always a property compared with an independent path; the one still missing is a sweep over **every field of every DTO** the contract returns, checked against the meaning the store states for it — `ended_at_ms` was the last field nobody had compared.
- A full export of the widest board is gigabytes; the stream is flat in memory, but a `from`/`to` on the export route would let a client take it in days rather than in years.
