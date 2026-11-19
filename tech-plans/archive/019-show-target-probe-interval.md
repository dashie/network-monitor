---
title: Show the configured probe interval in the target page toolbar
date: 2026-11-19
status: done
type: feature
impact: low
area: web, config, docs
version_bump: minor
tags: [web, detail-page, interval]
depends_on: []
branch_recommended: false
---

## Summary

The toolbar of the target detail page gains a third part after `HOME` and the target:
the target's effective probe interval, rendered as `every 5s`. The 30 s page reload,
the API, and every other page stay unchanged. When the target is not in the active
configuration, the part is omitted and the toolbar reads exactly as it does today.

## Why

The detail page states which target it shows but not how often that target is probed,
so a reader cannot tell how fresh the history is. The interval is already configured
(`interval_ms`, global or per target) and already known to every process that serves
the page; it is simply not exposed. The user asked for it next to the target identity.

## Goals

- `/target?target=<key>` for a target present in the active configuration renders
  `HOME • <label> (<address>) • every <interval>` (label/address omitted as today for
  a target without a label).
- The value shown is the target's effective interval: its own `interval_ms` when set,
  otherwise the global `interval_ms`.
- The interval is formatted exactly, never truncated: whole minutes as `Nm`, otherwise
  whole seconds as `Ns`, otherwise milliseconds as `Nms`.
- A target whose key is not in the active configuration renders no third part and no
  stray separator.
- The existing behaviour of every other page, the API, the 30 s reload, and the print
  sheet is unchanged.

## Non-goals

- The 30 s browser reload is not made configurable and is not what this label states.
- The timeline lanes and the JSON API do not gain the interval.
- No database schema change and no persistence of the interval: the configuration
  remains the only source.
- A live `interval_ms` reload under `monitor --listen` keeps probing at the new value,
  but the toolbar states the interval the web server started with until restart. This
  is documented, not fixed, in this change (the map is a startup snapshot).

## Affected modules

**Source**
- `src/web/mod.rs` — add `TargetIntervals` (`from_targets`, `get` with the legacy
  `icmp://` fallback), thread it through `run`, `serve`, and `handle`, and look up the
  interval for the `/target` route.
- `src/web/detail.rs` — `target_page` takes `interval: Option<Duration>` and renders the
  third crumb; tests updated and extended.
- `src/web/templates/detail.html` — add `{interval}` immediately after `{identity}` in
  the `.crumbs` line.
- `src/web/page.rs` — add `format_interval` next to `format_span`, with tests.
- `src/web/static/style.css` — (maybe) one `.target-interval` rule matching the address
  styling; the dot separator needs no change because it is the existing
  `.toolbar .crumbs > * + *::before` rule.
- `src/main.rs` — build `TargetIntervals::from_targets(&settings.targets)` and pass it
  at the three `web::run`/`web::serve` call sites (`serve`, `simulate`, `monitor --listen`).

**Tests**
- `src/web/page.rs` — unit tests for `format_interval`.
- `src/web/detail.rs` — unit tests for the third crumb present and omitted.
- `tests/web.rs` — helpers pass `TargetIntervals::default()`; one new integration test
  serves a map and asserts the page states `every 5s`.

**Documentation / configuration**
- `docs/USER-GUIDE.md` — the `/target` bullet of “The web page”: the toolbar now has
  three parts, and the third is the target's probe interval (not the 30 s reload).
- `docs/PRODUCT.md` — `FR-17`: both toolbar clauses that say “the parts separated by a
  dot” gain the interval and the omission rule.
- `docs/ARCHITECTURE.md` — `web` component Input (the interval map, a startup snapshot)
  and Output (the detail toolbar); no other section.
- `docs/DEVELOPER-GUIDE.md` — no change (it does not document these signatures).
- `site/index.html` — no change (it does not describe the target page toolbar).

## Architecture impact

`web` gains one read-only input derived from the configuration. The history source and
the database stay the only source of history; the interval map is configuration data
passed alongside the source, because the database does not store `interval_ms`. The
`web` component entry in `docs/ARCHITECTURE.md` must state the new input and the
startup-snapshot semantics.

## Decision-record impact

No new decision record: this is a display-only addition with no new technology,
contract, or reversal.

## Proposed solution

### Step 1 — Add the interval formatter

In `src/web/page.rs`, add beside `format_span`:

```rust
/// A configured interval, exact and never truncated: `2m`, `45s`, `500ms`.
pub fn format_interval(ms: u64) -> String {
    if ms % 60_000 == 0 {
        format!("{}m", ms / 60_000)
    } else if ms % 1_000 == 0 {
        format!("{}s", ms / 1_000)
    } else {
        format!("{ms}ms")
    }
}
```

Add unit tests: `500 -> "500ms"`, `1500 -> "1500ms"`, `5_000 -> "5s"`,
`60_000 -> "1m"`, `90_000 -> "90s"`, `120_000 -> "2m"`.

**Verify:** `cargo test format_interval`.

### Step 2 — Add `TargetIntervals` and thread it through the web layer

In `src/web/mod.rs`:

```rust
use std::collections::HashMap;
use std::time::Duration;
use crate::config::Target;

/// The effective probe interval of every configured target, keyed by canonical key.
/// The database stores transitions, not configuration, so the server receives this map
/// from the process that read the configuration; it is a snapshot taken at startup.
#[derive(Debug, Clone, Default)]
pub struct TargetIntervals {
    by_key: HashMap<String, Duration>,
}

impl TargetIntervals {
    pub fn from_targets(targets: &[Target]) -> Self {
        Self {
            by_key: targets.iter().map(|target| (target.key(), target.interval)).collect(),
        }
    }

    /// The interval of `key`, also accepting a bare address written before methods
    /// existed, the same fallback `Store::find_target` uses.
    pub fn get(&self, key: &str) -> Option<Duration> {
        self.by_key.get(key).copied().or_else(|| {
            (!key.contains("://"))
                .then(|| self.by_key.get(&format!("icmp://{key}")).copied())
                .flatten()
        })
    }
}
```

Change the signatures and pass the map down:

- `pub fn run(source, intervals: TargetIntervals, listen, shutdown, compression)`
- `pub fn serve(listener, source, intervals: TargetIntervals, shutdown, compression)`
- in `serve`, wrap in `Arc`, clone per accepted connection, and pass
  `intervals.as_ref()` to `handle`
- `fn handle(stream, source, intervals: &TargetIntervals, compression)`
- in the `/target` route: `let interval = intervals.get(&target.target);` and pass it to
  `detail::target_page`.

Add a unit test for `get`: exact `icmp://1.1.1.1` matches; bare `1.1.1.1` matches the
same entry; an unknown key returns `None`.

**Verify:** `cargo check` (call sites are updated in Step 4).

### Step 3 — Render the third crumb

In `src/web/detail.rs`, add the parameter and the fragment:

```rust
pub fn target_page(
    target: &TargetStateRow,
    days: &[DailyDrops],
    segments: &[TimelineSegment],
    recent: &[Fault],
    now_ms: i64,
    period_days: u32,
    interval: Option<Duration>,
) -> String {
    // ...
    let interval_part = match interval {
        Some(interval) => format!(
            "<span class=\"target-interval\">every {}</span>",
            format_interval(interval.as_millis() as u64)
        ),
        None => String::new(),
    };
    // ...
    .replace("{interval}", &interval_part)
}
```

Import `format_interval` from `super::page` and `std::time::Duration`.

In `src/web/templates/detail.html`, change the toolbar line to:

```html
<p class="crumbs"><a class="back-button" href="/">HOME</a><span class="target-identity">{identity}</span>{interval}</p>
```

The dot before the third part comes from the existing CSS:
`.toolbar .crumbs > * + *::before { content: "•"; … }`; no markup change is needed.
Optionally add `.toolbar .crumbs .target-interval { font-variant-numeric: tabular-nums; opacity: 0.8; }`
to `style.css`.

Update every existing `target_page` call in the tests with a trailing `None`, then add:

- `the_toolbar_states_the_interval_of_the_target` — `Some(Duration::from_secs(5))`;
  assert the crumbs contain
  `</span><span class="target-interval">every 5s</span>` after the identity.
- `a_target_without_a_known_interval_states_only_its_identity` — `None`; assert the page
  contains no `target-interval` and no `every `.

**Verify:** `cargo test detail`.

### Step 4 — Supply the map from the process that read the configuration

In `src/main.rs`:

- `Command::Serve`: `web::run(source, TargetIntervals::from_targets(&settings.targets), &listen, &stop, compression)?;`
- `Command::Simulate`: same map from `settings.targets`, passed to `web::run`.
- `run_monitor`: `web::serve(listener, source, TargetIntervals::from_targets(&settings.targets), &web_stop, compression)`

Import `TargetIntervals` from `crate::web`.

Configuration context (unchanged — this change only displays the existing key):

```toml
# uptime-monitor.toml
interval_ms = 5000

[[target]]
ip = "1.1.1.1"
label = "cloudflare-dns"
interval_ms = 10000   # optional; overrides the global value for this target
```

**Verify:** the value the page shows matches `target.interval` after merging
(`Config::resolve`), i.e. the per-target override when present, the global otherwise.

### Step 5 — Integration coverage

In `tests/web.rs`:

- `start_with` passes `TargetIntervals::default()` to `web::serve`.
- Add `start_with_intervals(database, compression, intervals)`; existing helpers keep
  their signatures and delegate with the default.
- Add `the_target_page_states_the_configured_probe_interval`: seed the database, build
  the map with `TargetIntervals::from_targets` over a `Target` whose `interval` is five
  seconds, request `/target?target=1.1.1.1`, and assert the body contains
  `class="target-interval">every 5s</span>` after the identity.

**Verify:** `cargo test --test web`.

### Step 6 — Update the documents

- `docs/USER-GUIDE.md`, “The web page”, `/target` bullet: change
  `HOME • <label> (<address>)` to `HOME • <label> (<address>) • every <interval>` and
  state that the third part is the target's probe interval (`interval_ms`, global or
  per target), shown only when the target is in the active configuration, and that it is
  not the 30 s page reload.
- `docs/PRODUCT.md`, `FR-17`: both toolbar paragraphs gain the third part and the
  omission rule.
- `docs/ARCHITECTURE.md`, `web` component: Input gains the interval map (a startup
  snapshot from the configuration); Output’s toolbar clause gains the third part.

**Verify:** grep the updated strings; every documented behaviour matches
`TargetIntervals::get` and the template.

## Security considerations

No security exposure: the value is a non-secret integer from a configuration the page
already reads, rendered as digits and unit letters; no new input, endpoint, or query
parameter is accepted. Existing HTML escaping is unaffected.

## Verification plan

- **Automated:** the new formatter, lookup, renderer, and integration tests above.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- **Manual:** `cargo run -- serve --listen 127.0.0.1:8080` with a config whose target sets
  `interval_ms = 10000`; open `/target?target=…` and confirm `every 10s`; remove the
  target from the config, restart, and confirm the part is gone.
- **Rollback:** revert the commit; no data migration is involved.

## Implementation Notes

- Steps 1–5 implemented as written: `format_interval` in `src/web/page.rs`,
  `TargetIntervals` in `src/web/mod.rs`, the `{interval}` crumb in `detail.rs` and
  `detail.html`, the map supplied from `main.rs` at `serve`, `simulate`, and
  `monitor --listen`, and the integration test in `tests/web.rs`.
- The optional `.target-interval` rule in `src/web/static/style.css` was **not** added.
  That file is marked `(maybe)` in Affected modules, the default toolbar styling already
  renders the part correctly, and the dot separator comes from the existing
  `.toolbar .crumbs > * + *::before` rule.
- The “no interval when unknown” unit test asserts only the absence of
  `target-interval`: the page footer legitimately contains the words “every 30 seconds”,
  so a naive `every ` search fails.
- No checkpoints were present in this plan.

## Open questions

- [x] Which value is shown? The effective probe interval (`interval_ms`), confirmed by
      the user; the 30 s reload is out of scope.
- [x] What wording? `every <interval>`, confirmed by the user.
- [x] What when the target is not in the active configuration? Omit the third part,
      confirmed by the user.
- [x] Live reload: accept the startup snapshot as a documented limitation (best guess,
      chosen for KISS; a shared reloadable map would couple `monitor` to `web`).

## Implementation checklist

- [x] Step 1 — Add `format_interval` and its tests
- [x] Step 2 — Add `TargetIntervals` and thread it through `web`
- [x] Step 3 — Render the third crumb and update the detail tests
- [x] Step 4 — Supply the map from `main.rs` at the three call sites
- [x] Step 5 — Add the integration test in `tests/web.rs`
- [x] Step 6 — Update the user guide, product, and architecture documents
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects the new toolbar and still documents `interval_ms`
- [x] `## Implementation Notes` added or updated in this plan

## Review Notes

### Round 1 — 2026-11-19

**Disclosure:** this round is a self-review performed in the same invocation as the
implementation (`spec-auto`). It is not an independent review; the verdict below rests on
the diff, the plan text, and the commands executed here, not on the implementation notes.

**Change set:** `src/web/page.rs`, `src/web/mod.rs`, `src/web/detail.rs`,
`src/web/templates/detail.html`, `src/main.rs`, `tests/web.rs`, `docs/USER-GUIDE.md`,
`docs/PRODUCT.md`, `docs/ARCHITECTURE.md`. `Cargo.lock` was touched by `cargo` alone.

**Quality gate (implementation pass):** `cargo fmt --check && cargo clippy --all-targets --
-D warnings && cargo test` — PASS (524 lib tests, 73 web integration tests, all suites
green).
**Quality gate (review pass, re-run):** same command — PASS. The tree was unmodified in
source between the two runs.

**Verification matrix**

| Goal | Evidence | Result |
|---|---|---|
| Three-part toolbar `HOME • target • every N` | `web::detail::tests::the_toolbar_states_the_interval_of_the_target`; `tests/web.rs::the_target_page_states_the_configured_probe_interval`; manual `simulate` curl returned `<p class="crumbs">…<span class="target-interval">every 5s</span></p>` | PASS |
| Effective interval (per-target, else global) | `TargetIntervals::from_targets` reads the resolved `Target::interval`; unit test; manual simulate showed `every 5s` for the global default | PASS |
| Exact formatting, no truncation | `web::page::tests::intervals_are_formatted_exactly` (`500ms`, `1500ms`, `90s`, `2m`) | PASS |
| Omitted when the target is not configured, no stray separator | `web::detail::tests::a_target_without_a_known_interval_states_only_its_identity`; the dot is emitted by `.crumbs > * + *::before`, so an empty part adds no child and no dot | PASS |
| Older bare `icmp://` keys still resolve | `web::tests::the_interval_of_a_target_is_found_by_its_key_and_by_a_bare_address` | PASS |
| Other pages, API, 30 s reload, print unchanged | full suite green; the toolbar is inside the block the print stylesheet hides | PASS |

**Gap analysis**

- Every step of *Proposed solution* is implemented; the optional CSS rule is the only
  deliberate omission, recorded in *Implementation Notes* because the file is marked
  `(maybe)` and needs no change.
- Documentation is consistent with the code: the user guide, `FR-17`, and the `web`
  component in the architecture document all state the third part and the omission rule.
  No stale “two parts” description remains.
- No decision record is needed: no new technology, contract, or trade-off.
- The startup-snapshot limitation for `monitor --listen` is recorded as a non-goal and in
  the architecture input; it matches the implementation.

**Findings**

| Severity | Finding | Action |
|---|---|---|
| Minor | `Cargo.lock` changes the root package version `0.16.0 → 0.17.0`; this is a pre-existing manifest/lock mismatch that any `cargo` command rewrites, not part of the plan. | Left as is; every subsequent build re-applies it. |
| Minor | In the integration test, `assert!(crumbs < identity)` adds little because the exact-markup assertion already carries the check. | Not fixed: the review pass does not patch code. |

No blocker and no major finding.

**Verdict**: PASS

## Closure

- **Closed**: 2026-11-19
- **Version**: 0.17.0 → 0.18.0
- **Executed by**: OpenCode (`deepseek/deepseek-flash`)
- **Tokens**: unknown
- **Cost**: unknown
