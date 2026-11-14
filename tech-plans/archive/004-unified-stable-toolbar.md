---
title: Unify the masthead across pages and move the web UI into editable files
date: 2026-11-14
status: done
type: feature
impact: low
area: web/ui
version_bump: patch
tags: [ui, ux, visual-stability, templates, developer-experience]
depends_on: []
branch_recommended: false
---

## Summary

Both pages open with the same identity block — mark, `Network Monitor`, `Reachability
monitoring` — flush left and on the same centre line, inside a masthead of fixed height. The
detail page carries no breadcrumb and states its target at the right end of the same
masthead: `label | address` over `UP for 5d 3h`. Its toolbar holds the way back and the
period selector, at a height that matches the timeline. The page HTML, the stylesheet and
the script are files under `src/web/`: a debug build reads them on every request, a release
build embeds them, so adjusting the UI needs neither a rebuild nor a restart.

## Why

Three problems, all visible to whoever opens the page.

- **Layout shift and a different head on the two pages.** The masthead and toolbar had no
  fixed height, so the page moved when the reader went from `/` to a target, and the detail
  page opened with a breadcrumb the timeline does not have.
- **The identity block drifted.** The masthead was assembled from Rust constants; when the
  target information joined it, `justify-content: space-between` pushed the name and the
  tagline towards the middle of the page, and two integration tests went red.
- **The UI could not be debugged.** HTML, CSS and JavaScript were Rust string constants
  pasted into the response, so devtools showed them inside the document, with no file and no
  lines to break on, and every tweak cost a rebuild and a restart.

**Evidence:** the failing pair `both_pages_show_the_mark_the_server_serves` and
`the_page_context_and_its_selector_share_one_line` after the first masthead iteration; the
`<style>` and `<script>` blocks in the served HTML before the files existed; the masthead
and toolbar boxes differing between `/` and `/target` in devtools.

**Cost of leaving it:** every UI adjustment stays a compile-test cycle, and the masthead can
regress again without a test catching it.

## Goals

- Both pages open with the same identity block (mark, `Network Monitor`, `Reachability
  monitoring`), flush left and vertically centred, inside a `72px` masthead.
- The detail page carries no breadcrumb, and the target — `label | address`, then the state
  badge with its duration — sits at the right end of the masthead, on the same centre line.
  A target without a label states the address alone.
- A back button in the detail toolbar returns to wherever the reader came from.
- All UI markup, CSS and JavaScript live in files under `src/web/`; a debug build serves
  them from disk on every request, a release build embeds them; no new dependency and no
  template engine.
- `/style.css` and `/app.js` are served with their content types and `Cache-Control:
  no-store`.
- Existing behaviour is preserved: the 30-second reload, the localisation of instants, the
  cause toggle, the window and period links, and the escaping of every label.
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes.

## Non-goals

- A template engine or a client-side framework: the markup a loop builds — the chart SVG,
  the fault and day-by-day tables, the lane rows, the window and period links — stays in
  Rust, because there is no engine to express it in.
- Reading the icons (`assets/icon/icon.svg`, `favicon-32.png`) from disk in debug builds:
  they stay embedded.
- Any change to probing, storage, query, export, notifications or the CLI.
- The responsive breakpoints, the colour scheme and the chart geometry.

## Affected modules

**Source**
- `src/web/templates.rs` — *(new)* the asset loader: one `Asset` enum, five files, the source
  copy in a debug build and the `include_str!` copy otherwise.
- `src/web/templates/page.html` — *(new)* the timeline: head, masthead, toolbar, legend,
  fault table, footer, script tag.
- `src/web/templates/detail.html` — *(new)* one target: head, masthead with the target block,
  toolbar with the way back, stats, chart, tables, footer.
- `src/web/templates/brand.html` — *(new)* the identity block both pages pull through
  `{brand}`.
- `src/web/static/style.css` — *(new)* the stylesheet of both pages.
- `src/web/static/app.js` — *(new)* lane link, way back, cause toggle, localisation.
- `src/web/mod.rs` — the `/style.css` and `/app.js` routes, the `templates` module export.
- `src/web/page.rs` — the timeline: `{brand}` from the loader; `STYLE`, `ICON_LINKS`,
  `BRAND`, `PAGE_NOTES`, `footer`, `REFRESH_SECONDS` and the inline scripts removed.
- `src/web/detail.rs` — the target page: the target block of the masthead, the back button,
  the period links; the same constants removed.

**Tests**
- `tests/web.rs` — shared brand block, toolbar contents, the two asset routes, the mark.
- `src/web/templates.rs` — the loader: every asset read from the source tree, the embedded
  copies, an unknown template.
- `src/web/page.rs`, `src/web/detail.rs` — rendering tests, now reading `stylesheet()` and
  `script()` instead of a Rust constant.

**Documentation / configuration**
- `docs/ARCHITECTURE.md` — the `web` component: identity block and target in the masthead,
  the two served assets, the path list in the security section.
- `docs/PRODUCT.md` — FR-17: the target page states the target at the right end of the
  masthead, and the way back is the toolbar button.
- `docs/USER-GUIDE.md` — "The web page": shared masthead, target at its right end, back
  button.
- `docs/DEVELOPER-GUIDE.md` — repository tree and "Web templates and assets".
- `docs/ADR.md` — ADR-0010: the script is served as `/app.js`.

## Architecture impact

The web module gains one small loader (`src/web/templates.rs`) that returns an asset as
text: the file under `src/web/` while developing, the `include_str!` copy in a release
build. The HTTP surface grows by two read-only routes, `/style.css` and `/app.js`, both
served with their content type and `Cache-Control: no-store`. Nothing else moves: the
server stays a hand-written HTTP loop with no dependency, no template engine, and no write
path, and the shipped binary still carries every page it serves.

## Decision-record impact

No new record: the shipped behaviour is unchanged — one binary with embedded assets, no
framework, no client library — and reading them from disk happens only in a debug build.
ADR-0010 was amended, because the script it describes is no longer inline but served as
`/app.js`.

## Proposed solution

### Step 1 — Fix the height of the masthead and of the toolbar

`src/web/static/style.css`: `.masthead` gets `min-height: 72px` and `.toolbar`
`min-height: 56px`, both `align-items: center`, so the content never resizes the bars.

**Verify:** devtools on `/` and on `/target?target=…`: both boxes measure `72px` and `56px`,
and the masthead starts at the same `y`.

### Step 2 — Remove the breadcrumb from the detail page

Delete the `<nav class="breadcrumb">` block and the `.breadcrumb` rules from the
stylesheet.

**Verify:** the detail page renders no breadcrumb; `cargo test` stays green.

### Step 3 — Anchor the identity block to the left

The block lives once, in `src/web/templates/brand.html`, and both page files pull it through
`{brand}`, filled by `templates::brand()`. `.masthead .brand { margin-right: auto; }` holds
it against the left edge whatever a page appends to the masthead, and `.masthead`
vertically centres it.

**Verify:** the block measures the same box on both pages; `every_page_opens_with_the_same_brand_block`
compares the two and asserts it opens the masthead.

### Step 4 — State the target at the right end of the masthead

`detail.html` closes the masthead with `<div class="target-info">`: the label and the address
it names on one line (`label | address`), the badge and its duration on the next
(`up for 5d 3h`). The block is right-aligned and on the same centre line as the identity
block — no `align-self` — and a target without a label states the address alone.

**Verify:** `the_masthead_states_the_label_and_the_address_of_the_target` and
`a_target_without_a_label_states_its_address_alone`; the two cases checked in the browser.

### Step 5 — Add the way back to the detail toolbar

The toolbar opens with `<button class="back-button">← Back</button>`, then the period links.
The handler lives in `app.js` and calls `history.back()`, so the reader returns to the window
they left.

**Verify:** `the_way_back_sits_at_the_top_of_the_page`; the button clicked in the browser
returns to `/` with the window preserved.

### Step 6 — Move the page HTML into files

`src/web/templates/page.html` and `detail.html` hold the whole markup, placeholders included;
`page.rs` and `detail.rs` fill only the placeholders that carry data and keep the loops that
build the chart, the tables and the link lists.

**Verify:** both pages render as before — `cargo test`, then the browser.

### Step 7 — Read the templates from disk while developing

`src/web/templates.rs`: `load_template(name)` reads `src/web/templates/<name>.html` from
`CARGO_MANIFEST_DIR` on every request when `debug_assertions` is on, and falls back to the
`include_str!` copy when the file cannot be read or the build is a release one.

**Verify:** edit `page.html` against a running debug server — the next request shows the
change; `a_debug_build_reads_every_asset_from_the_source_tree` guards it.

### Step 8 — Move the stylesheet and the script into files, and serve them

`src/web/static/style.css` and `src/web/static/app.js`; the pages link them with
`<link rel="stylesheet" href="/style.css">` and `<script src="/app.js"></script>`, and
`src/web/mod.rs` serves both with their content type and `Cache-Control: no-store`. The
script keeps the four behaviours — lane link, way back, cause toggle, localisation — each one
guarded by the element it drives, so one file serves both pages.

**Verify:** `the_pages_link_the_stylesheet_and_the_script_the_server_serves`; devtools lists
both as files with their own lines; the interactions still work.

### Step 9 — Move the head and the footer into the pages

The icon links and the footer — the rule, the notes, their wording — live in each page file;
`ICON_LINKS`, `PAGE_NOTES`, `footer()` and `REFRESH_SECONDS` are gone, so the refresh
interval and the copy that names it cannot disagree.

**Verify:** `both_pages_close_with_a_short_manual`; the icon links still advertise
`/icon.svg` and `/icon.png`.

### Step 10 — Update the tests and the documentation

Unit and integration tests updated to the new structure, the loader covered, `docs/USER-GUIDE.md`,
`docs/DEVELOPER-GUIDE.md` and ADR-0010 updated.

**Verify:** the quality gate below; the guide read against the running page.

## Security considerations

No new trust boundary: the two routes serve constant text chosen by the server, never by
the request, and neither opens the database. Every value that reaches the markup is still
escaped where it enters (`escape()` before the placeholder replacement), and the new
responses keep `X-Content-Type-Options: nosniff` and `Cache-Control: no-store`. The debug
loader reads a fixed set of five files under `src/web/`, never a path from a request;
the name it accepts comes from the renderers, not from the URL.

## Verification plan

- **Automated:** `every_page_opens_with_the_same_brand_block`,
  `the_masthead_states_the_label_and_the_address_of_the_target`,
  `a_target_without_a_label_states_its_address_alone`,
  `the_way_back_sits_at_the_top_of_the_page`,
  `the_page_context_and_its_selector_share_one_line`,
  `the_pages_link_the_stylesheet_and_the_script_the_server_serves`,
  `a_debug_build_reads_every_asset_from_the_source_tree`, `an_unknown_template_is_refused`.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- **Manual:**
  1. Load `/` and `/target?target=…` and compare the masthead and toolbar boxes in devtools:
     `72px`, `56px`, brand block at `x=24`, target block right-aligned on the same centre
     line.
  2. Confirm the detail page has no breadcrumb, and that the label and address share one
     line while the badge and duration share the next; repeat with a target that has no
     label.
  3. Click `← Back` and confirm the return, then a fault row and confirm the cause opens.
  4. Edit a template, `style.css`, `app.js`, then `brand.html` against the running debug
     server: each change shows on the next refresh, without a rebuild or a restart.
- **Rollback:** revert the `src/web/` changes. The rendered pages keep working because the
  release build embeds every file it serves.

## Open questions

- [x] **Target information placement:** the toolbar first, as drafted, or the masthead?
  **Resolved:** right end of the masthead — in the toolbar it repeated the address the hint
  already carried, and it is the identity of the page, not a control.
- [x] **Back button exact position:** **Resolved:** first element of the detail toolbar,
  before the period links.
- [x] **Status format:** **Resolved:** the badge plus `for …`, e.g. `UP for 5d 3h`.
- [x] **How the UI is edited while developing:** **Resolved:** the files are read at runtime
  in a debug build and embedded in a release one.

## Implementation checklist

- [x] Step 1 — Fix masthead and toolbar heights
- [x] Step 2 — Remove the breadcrumb
- [x] Step 3 — Anchor the identity block to the left
- [x] Step 4 — State the target at the right end of the masthead
- [x] Step 5 — Add the way back
- [x] Step 6 — Move the page HTML into files
- [x] Step 7 — Read the templates from disk while developing
- [x] Step 8 — Move and serve the stylesheet and the script
- [x] Step 9 — Move the head and the footer into the pages
- [x] Step 10 — Update the tests and the documentation
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide reflects user-visible behaviour and navigation flow
- [x] Implementation Notes added to this plan
- [x] Review round 1 findings addressed

## Implementation Notes

### Delivered design

- **Masthead**, both pages: `min-height: 72px`, `align-items: center`. The identity block —
  mark, `Network Monitor`, `Reachability monitoring` — comes from `brand.html` through
  `{brand}` and is held against the left edge by `margin-right: auto`.
- **Toolbar**, both pages: `min-height: 56px`. The timeline shows the window and its links;
  the target page opens with `← Back` and lists the periods.
- **Target block**, detail page only: right end of the masthead, on the same centre line —
  `label | address` on one line, the badge with its duration (`up for 5d 3h`) on the next.
  Without a label the address stands alone.
- **Files**: `templates/page.html`, `templates/detail.html`, `templates/brand.html`,
  `static/style.css`, `static/app.js` (lane link, way back, cause toggle, localisation).
- **Loader**: `templates.rs` reads those five files per request in a debug build, embeds
  them in a release build, and falls back to the embedded copy when a file cannot be read.
- **Routes**: `/style.css` as `text/css; charset=utf-8`, `/app.js` as
  `text/javascript; charset=utf-8`, both with `Cache-Control: no-store`.

### Deviations from the first draft

1. **Target information**: the draft put it at the right of the toolbar. Delivered at the
   right end of the masthead: in the toolbar it repeated the address the hint already
   carried, and it describes the page rather than controls it.
2. **Identity block**: the first attempt placed the target block inside the masthead behind
   `justify-content: space-between`, which pushed the name and the tagline towards the
   middle and left two integration tests failing. The auto margin on `.brand` replaced the
   modifier and anchors the block whatever follows it.
3. **Way back**: drafted as a link to `/`; delivered as a `history.back()` button, so the
   return keeps the window the reader came from instead of dropping the query.
4. **Scope grew at the user's request**: the UI left the Rust strings — templates first,
   then the stylesheet and the script, then the head, the footer and the brand block —
   because every layout tweak otherwise cost a rebuild and a restart, and devtools had no
   file to break on.

### Discoveries

- With three children a `space-between` masthead centres the middle one; an auto margin on
  the identity block is what actually anchors it to the left.
- CSS assertions written against a Rust constant tested the constant, not what the browser
  receives; they now read `stylesheet()` and `script()`.
- The `<meta http-equiv="refresh">` value and the prose naming it had drifted from
  `REFRESH_SECONDS`; both live in the page file now.
- A target without a label would have stated its address twice once the hint left the page;
  the target block states it once.

### Testing

- `cargo test`: 344 tests passed in 12 suites (263 in the library, 22 in `tests/web.rs`;
  2, 3 and 5 ignored by design — network-dependent and root-only).
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo check --release`
  clean.
- Browser: masthead `72px` starting at `x=24` and `y=32` on both pages, brand box
  `231.34 × 51.5` at `y=33.75`, target block centred at `y=59.5` on both sides, back button
  returning, cause row toggling, instants localised.
- Live editing proved for all five files by adding a marker, reading it back from the
  running server, and removing it.

### Review fixes (round 1)

The first review found the architecture and product documents still describing the removed
breadcrumb, and a missing path in the route list. Addressed in the implementation:

- `docs/ARCHITECTURE.md` — the `web` **Output** names the identity block, the target at the
  right end of the masthead and the way back instead of the breadcrumb; "asks for nothing
  else" now reads "asks only for the page, its stylesheet, and its script"; the security
  section lists the stylesheet and the script among the served paths.
- `docs/PRODUCT.md` — FR-17 states the target and its state at the right end of the
  masthead and the back button in the toolbar, and no longer mentions a breadcrumb.
- `docs/USER-GUIDE.md` — "top right" became "right end of the masthead, on the same centre
  line as that block".
- `tests/web.rs` — the doc comment of `every_page_opens_with_the_same_brand_block` says
  "right end"; `the_page_context_and_its_selector_share_one_line` again proves that the
  period selector and the back button sit inside the toolbar.
- The two documents above were added to `## Affected modules`, where they were missing.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-14
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(web): unify the masthead and serve the ui as editable files`
- **Quality gate**: tests ✓ (344 passed) | lint ✓ | fmt ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `docs/ARCHITECTURE.md:180` | The `web` **Output** contract still says the detail page is "opened with a breadcrumb that leads back to the timeline". The breadcrumb is gone: the document contradicts the delivered behaviour. | open |
| 2 | major | `docs/ARCHITECTURE.md:77`, `:294` | Line 77 says "the browser reloads the page every 30 s and asks for nothing else", and the security section lists the served paths as "the timeline, the detail page, the streamed export, the health check, and the icon". The browser now also fetches `/style.css` and `/app.js`; both statements are stale. | open |
| 3 | major | `docs/PRODUCT.md:77` (FR-17) | FR-17 still states "a breadcrumb at the top leads back to the timeline". The requirement contradicts the delivered page and was not revised. | open |
| 4 | minor | `docs/USER-GUIDE.md:336` | "A page may state what it is showing at the top right of the masthead" is inconsistent with the delivered block, which is centred on the right end, and with the next bullet that says "at its right end". | open |
| 5 | minor | `tests/web.rs:672` | The doc comment of `every_page_opens_with_the_same_brand_block` still says "at the top right". | open |
| 6 | minor | `tests/web.rs:583` | `the_page_context_and_its_selector_share_one_line` now asserts the toolbar, the back button and the period nav individually; it no longer proves the period selector is inside the toolbar, which the test name states. | open |

### Gap analysis
- Goals: 7/7 achieved — masthead/brand/target geometry pinned by `every_page_opens_with_the_same_brand_block` and the `details` tests; both routes served with `text/css` and `text/javascript` and `no-store` (checked with `curl -I` against the built binary); debug reads proved for all five files; behaviour preserved by the web suite.
- Scope: every changed source, test and documentation file is inside `## Affected modules`. The staged deletion of `tech-plans/003-telegram-notifications.md` belongs to the archive of plan 003 and must not be part of this commit.
- Deviations: the three that changed the design (target block at the right end of the masthead, `history.back()` instead of `href="/"`, UI moved into files) are recorded in `## Implementation Notes`.
- Additions: the UI externalisation went beyond the first draft but was requested by the user during the work and is written into the plan's goals, steps and notes.
- Deferred: none.
- Under-declared scope: `docs/ARCHITECTURE.md` and `docs/PRODUCT.md` are not in `## Affected modules`, which is why findings 1–3 were not caught by the implementer.

### Known limitations
- The chart SVG, the fault and day-by-day tables, the lane rows and the window/period links stay in Rust: expressing them in files would need a template engine the server deliberately does not have.
- The icons (`assets/icon/icon.svg`, `favicon-32.png`) are still embedded at compile time, so editing them still needs a rebuild.
- `/style.css` and `/app.js` are served plain, while the HTML is gzipped when the client accepts it.

### Recommendations for future work
- Add the two documentation findings to the plan, fix them together with findings 4 and 5, then re-run this review.
- Restore the co-location assertion of finding 6 by checking that the `<nav>` is inside the toolbar slice.
- If the UI keeps growing, revisit gzip for the two assets; the current `no-store` keeps editing instant, which is the point.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-14
- **Verdict**: PASS
- **Suggested commit**: `feat(web): unify the masthead and serve the ui as editable files`
- **Quality gate**: tests ✓ (344 passed, 263 in the library and 22 in `tests/web.rs`) | lint ✓ | fmt ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `docs/ARCHITECTURE.md` | The `web` output contract described the removed breadcrumb. Now names the identity block, the target at the right end of the masthead and the way back. | resolved |
| 2 | major | `docs/ARCHITECTURE.md` | "asks for nothing else" now reads "asks only for the page, its stylesheet, and its script"; the served-path list includes the stylesheet and the script. | resolved |
| 3 | major | `docs/PRODUCT.md` | FR-17 restated without the breadcrumb: target and state at the right end of the masthead, back button in the toolbar. | resolved |
| 4 | minor | `docs/USER-GUIDE.md` | Now "at the right end of the masthead, on the same centre line as that block". | resolved |
| 5 | minor | `tests/web.rs` | The brand-block test comment says "right end". | resolved |
| 6 | minor | `tests/web.rs` | Co-location restored: the test again proves the back button and the period selector sit inside the toolbar. | resolved |

### Gap analysis
- Goals: 7/7 achieved.
- Scope: every changed file is inside `## Affected modules`, which now lists `docs/ARCHITECTURE.md` and `docs/PRODUCT.md`.
- Deviations: the three design deviations remain documented in `## Implementation Notes`.
- Additions: the UI externalisation, approved by the user during the work.
- Deferred: none.

### Known limitations
- The chart SVG, the tables, the lane rows and the link lists stay in Rust; expressing them in files would need a template engine the server does not have.
- The icons are still embedded at compile time.
- The stylesheet and the script are served plain while the HTML is gzipped.

### Recommendations for future work
- None blocking. If the UI grows further, revisit gzip for the two assets.


