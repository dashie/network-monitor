---
title: Publish a landing page for the project on GitHub Pages
date: 2026-11-15
status: done
type: feature
impact: medium
area: site
version_bump: none
tags: [site, github-pages, html, css, seo, docs]
depends_on: []
branch_recommended: false
---

## Summary

The repository gains a public landing page: one hand-written `site/index.html`, one `site/style.css`,
three screenshots taken from the product's own simulation, two crawl files, and a GitHub Actions
workflow that publishes them to `https://dashie.github.io/network-monitor/`. It describes what the
tool is, why it exists, the four checks it performs today and the one contract that admits more, how
it keeps a history, and how to run it — in the project's own English and with no runtime of its own:
no framework, no JavaScript, no external font, no tracker. The same change adopts the MIT licence,
which the page's FAQ states, so the repository stops being "public but unlicensed". The executable,
its behaviour and its configuration are untouched.

The page is written from the repository's documents, never from the two reference pages in the brief.
No product named in that brief appears in the page, in this plan, in the workflow, in a document this
plan touches, or in a commit message it proposes; the repository's pre-existing import runbook that
names one of them — a runbook for a real import capability, listed in the `AGENTS.md` table, with its
helper script under `scripts/utils/` — is out of this plan's scope and stays as it is (see Open
questions).

## Why

The tool is public (`dashie/network-monitor`) and has no entry point: the README is written for
someone who already decided to care, and the page the binary serves cannot be shown to anyone who has
not run it. A visitor arriving from a search or a link has to read Rust-quality prose in a repository
listing to learn what the thing does, what it costs to run, and whether the next check they need is
supported. Two concrete costs follow:

- **The extension story is invisible.** The most defensible property of this codebase — one `Probe`
  trait with four implementations, `Method` as a closed enum, and a documented recipe for a fifth
  (`docs/DEVELOPER-GUIDE.md#adding-a-probe-type`) — appears nowhere a newcomer looks.
- **The lightness is invisible.** A 6.2 MB stripped release binary, one embedded SQLite file and no
  runtime dependency are the reason the tool exists, and they are stated only in prose buried in
  `README.md`.

The brief's requirements are explicit: describe the application, draw the structure and the tone from
the two reference pages, and name none of them **in anything this plan writes**. The repository
already names one of them in eight files — an import runbook for a real capability, its script, its
test, an example export, a decision record, the README, `AGENTS.md` and one archived plan. That is a
fact about an import format, not a claim about where the page drew its inspiration, and purging it
would rename a documented feature and edit an archived plan; it is therefore a separate decision,
recorded in Open questions, and not part of this plan.

## Goals

- `site/index.html` and `site/style.css` exist as the whole page: no build step, no framework, no
  JavaScript, no external host. Opening `site/index.html` from the file system and through a local
  static server shows the same page.
- The page states, in this order and under these headings: the name and a one-sentence claim; two
  calls to action; why the tool exists, as cards; the four checks it performs and the extension
  contract; how a history is kept; three screenshots; a quick start with commands that run as shown;
  the deliberate limits; a FAQ; a footer with the links to the source and the documentation.
- Every command in the quick start is copied from `README.md` or `docs/USER-GUIDE.md` and was executed
  while writing the page: no flag, no path and no output shown on the page is invented.
- Three screenshots come from `network-monitor simulate` with a recorded seed, history, target list
  and viewport, are committed under `site/assets/`, and are regenerable from the command in
  `docs/DEVELOPER-GUIDE.md`.
- The page is under 600 KB in total and no single image is over 150 KB, measured with `du -ch`.
- Search-engine foundations are in place: one `<h1>`, a `<title>` and a `meta description` of at most
  160 characters, `link rel="canonical"`, Open Graph and Twitter card tags with a 1200×630 image,
  `lang="en"`, `site/robots.txt`, `site/sitemap.xml`, and a JSON-LD `SoftwareApplication` block.
  Submitting to a search console, analytics and a custom domain stay out of scope.
- No product named in the brief appears in `site/`, in this plan, in the workflow, in the four
  documents this plan edits or in a commit message it proposes: a case-insensitive search over those
  paths finds nothing. The eight pre-existing occurrences elsewhere — the import runbook, its script
  and test, its example export, a decision record, `README.md`, `AGENTS.md` and one archived plan — are
  outside this goal by decision (Open questions) and are left untouched.
- The page works with JavaScript disabled, offline, and at 360, 768 and 1280 CSS px without a
  horizontal scrollbar; every image carries `alt` text and `width`/`height`.
- A push to `main` publishes the page through a workflow that assembles `site/` plus the icons from
  `assets/icon/` and publishes nothing from `docs/`, `tech-plans/` or `src/`.
- `README.md`, `AGENTS.md`, `docs/DEVELOPER-GUIDE.md` and `docs/ARCHITECTURE.md` state that the page
  exists and where it is published.
- The repository carries the MIT licence: `LICENSE` at the root with the copyright line of the
  repository author, `license = "MIT"` in `Cargo.toml`, a `## Licence` section in `README.md`, one
  inbound-equals-outbound sentence in `CONTRIBUTING.md`, and the FAQ of the page naming MIT and linking
  to the file on GitHub. No dependency forbids it: every crate in `Cargo.lock` is MIT, Apache-2.0, ISC,
  BSD, 0BSD, Unlicense or CDLA-Permissive, and the vendored API renderer keeps its own Apache-2.0 text
  next to the file it covers.
- The quality gate (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
  still passes, and the executable behaves exactly as before.

## Non-goals

- Submitting the page to a search console, adding analytics or a sitemap ping service, and choosing a
  custom domain: the address stays the project subpath, and every link and the canonical URL are
  relative or derived from it, so the move to a domain later is one file of constants.
- A blog, a second page, a language switcher, a dark/light toggle, or any client-side interaction:
  light and dark follow `prefers-color-scheme`, as the served page does.
- Screenshots of every screen, a product video, an animated hero, or a generated Open Graph image
  beyond the one 1200×630 crop.
- Any change to the binary, its page, its API, its configuration or its exported history.
- A Contributor Licence Agreement, per-file licence headers, a `NOTICE` file, or a change to the
  licence of any vendored file: MIT needs none of them, and the Apache-2.0 text under
  `assets/api/swagger-ui/` stays exactly where it is, next to the renderer it covers.
- Copying text, layout files or assets from the reference pages: the structure and the tone are drawn
  from them, the words are the project's own.

## Affected modules

**Site (new)**
- `site/index.html` — the page: semantic sections, the copy, the meta and Open Graph tags, the JSON-LD
  block, three `<img>` and the anchors.
- `site/style.css` — the whole stylesheet: one small file, the palette of the served page, a single
  column that widens to two on a wide screen, `prefers-color-scheme`, no external font.
- `site/robots.txt` — `Allow: /` and the absolute `Sitemap:` line.
- `site/sitemap.xml` — one `<url>` for the page with `lastmod` and `changefreq`.
- `site/assets/timeline.jpg` — *(new)* the timeline page from `simulate`, 1100 px wide.
- `site/assets/target.jpg` — *(new)* the target page with the day-by-day table, 1100 px wide.
- `site/assets/narrow.jpg` — *(new)* the timeline at 390 CSS px, showing the stacked tables.
- `site/assets/og.jpg` — *(new)* the 1200×630 card used by Open Graph and the Twitter card.

**Licence**
- `LICENSE` — *(new)* the MIT text with `Copyright (c) 2026` and the repository author's name.
- `Cargo.toml` — `license = "MIT"` beside `description`; `publish = false` stays, so nothing is
  published to a registry and the field is metadata only.
- `CONTRIBUTING.md` — one sentence: a contribution is accepted under the same licence.

**Configuration and automation**
- `.github/workflows/pages.yml` — *(new)* the publication workflow: assemble, upload, deploy.
- `.gitignore` — ignore the assembled `_site/` directory.

**Documentation / configuration**
- `README.md` — a `Website` line and a `Licence: MIT` line in the opening block, an entry in
  `## Documentation`, and a short `## Licence` section naming MIT and linking `LICENSE`.
- `AGENTS.md` — one row in the project-documents table pointing at `site/index.html`.
- `docs/DEVELOPER-GUIDE.md` — a `## Landing page` section (where the files live, how to preview, the
  exact `simulate` command and viewport for the screenshots, how the workflow publishes, the manual
  switch of the Pages source), plus `site/` and `.github/workflows/` in `## Repository layout`.
- `docs/ARCHITECTURE.md` — one clause in the deployment section naming the published landing page as a
  second, static artefact that shares the repository but not the binary.
- `docs/USER-GUIDE.md` — no change: the page is a separate artefact, the guide is distributed alone
  and stays self-contained; nothing in it describes the site.
- `docs/GLOSSARY.md`, `docs/ADR.md`, `docs/PRODUCT.md`, `CONTRIBUTING.md`, `NOTES.md` — no change.

**No change**
- `src/`, `tests/`, `assets/`, `Dockerfile`, `docker-compose.yml`, `network-monitor*.toml`: the
  screenshots are taken from the running binary, the page ships nothing of it.

## Architecture impact

One new published artefact, and no change to the architecture of the tool. `docs/ARCHITECTURE.md`
gains a clause in the deployment section: the repository publishes two things — the executable with
its page, and a static landing page assembled from `site/` — and only the first is built from Rust or
shipped as a container. The document's component table is unchanged: no module, no contract and no
invariant moves.

## Decision-record impact

No new decision record. Against the repository's own criterion in `docs/ADR.md` — a record is written
when a decision is costly to reverse, affects more than one component, or constrains future work —
this one is cheap to reverse (one directory and one workflow file, deleted in one commit), touches no
component, and constrains nothing beyond a URL whose shape the plan already pins to relative links.
The reasoning lives in `docs/DEVELOPER-GUIDE.md`, where the next reader looks for how to publish.

## Proposed solution

The page is written top to bottom: skeleton and stylesheet first, then the copy against the sources,
then the images, then the files a crawler reads, then the publication, then the documents that must
mention it.

### Step 1 — Write the shell and the stylesheet

`site/index.html` is one document with `<html lang="en">` and no script. Its sections, in order, each
with an `id` the navigation points at:

1. `<header>` — the mark (`assets/icon.svg`, copied by the workflow), the wordmark
   `network-monitor`, and the anchors `Why`, `Checks`, `How it works`, `Screenshots`, `Quick start`,
   `FAQ`, plus a `Source` button to `https://github.com/dashie/network-monitor`.
2. `#top` hero — one `<h1>`, one sentence, two buttons (`Get the source` → GitHub, `Quick start` →
   `#quick-start`), and the line `macOS · Linux · Windows · Docker`. No image above the fold.
3. `#why` — six cards, each a short title, two sentences, and no adjective that the code cannot
   support (the points are fixed in Step 2).
4. `#checks` — a table of the four methods with two columns, `Check` and `It is up when`; under it the
   extension paragraph.
5. `#how-it-works` — four numbered steps.
6. `#screenshots` — the three images with `alt` text and `loading="lazy"`.
7. `#quick-start` — three `<pre>` blocks: Docker, from source, and `simulate`.
8. `#limits` — a short list of deliberate limits.
9. `#faq` — eight `<details>`/`<summary>` pairs, so no script is needed to open them.
10. `<footer>` — the mark, the links (source, user guide, API contract), and one line of provenance.

`site/style.css` carries the whole presentation, in the order the served page uses: a `:root` block
with the palette below, a reset of margins and box sizing, a 68-chapter measure, one column that
becomes two at 900 px for the cards and the screenshots, `prefers-color-scheme: dark` overriding the
palette, and `:focus-visible` outlines. Palette taken from `assets/icon/icon.svg` and the served page:
`#2D4EA9` (mark blue), `#2D7D46`/`#2e7d32` (up), `#c62828` (down), `#ef6c00` (short fault), and the
page's grey rules.

No `@import`, no `url()` outside `assets/`, no font: the system stack is
`system-ui, -apple-system, "Segoe UI", sans-serif`, as in `src/web/static/style.css`.

**Verify:** `python3 -m http.server 8000 --directory site` then open `http://127.0.0.1:8000/`; the
skeleton renders with no 404 except `assets/icon.svg` and the images (provided in Steps 3 and 5), and
`rg -n "<script|@import|https://fonts" site/` returns nothing.

### Step 2 — Write the copy against the sources

Each claim below is transcribed from the file next to it. Nothing is added for effect; a sentence that
the repository cannot support is dropped rather than qualified.

| Block | What it says | Source |
|---|---|---|
| hero sentence | Reachability monitoring that records incidents, not samples: one executable, an embedded SQLite history, and a read-only page served from the same process | `README.md` *Overview*, `docs/PRODUCT.md` *Problem and value proposition* |
| why · history | Only a state change is written, so the history grows with incidents and not with probes | `README.md` *Overview*, `ADR-0004` |
| why · size | One stripped executable of about 6 MB with no runtime dependency; certificate roots compiled in | `README.md` *Overview*, measured binary, `FR-14` |
| why · interface | A probe can be bound to a named interface with the semantics of `ping -I`, and the result is attributable to that interface | `README.md`, `docs/PRODUCT.md` |
| why · one contract | Four checks share one `Probe` trait; a fifth is a new module, a new `Method` variant and one arm | `ADR-0023`, `docs/DEVELOPER-GUIDE.md#adding-a-probe-type` |
| why · reads while writing | SQLite in WAL mode: `query`, the page and the API read while the monitor records | `README.md` *Overview* |
| why · notification | One message per incident, and a recovery only once `up` has held for the stabilisation window | `docs/USER-GUIDE.md` *Telegram notifications* |
| checks table | icmp (echo reply), tcp (connection established), dns (a record of the requested type resolves), http (the status falls inside the expectation) | `docs/USER-GUIDE.md` *ICMP*, *TCP*, *DNS*, *HTTP and HTTPS* |
| how it works | TOML declares targets → each cycle probes with its own timeout → only a transition is persisted, with the cause of a fault → the page, the API, the CSV export and `query` read the same rows | `docs/PRODUCT.md` `FR-17`, `FR-24`, `docs/ARCHITECTURE.md` |
| quick start · docker | `docker run -d --name network-monitor --cap-drop ALL --cap-add NET_RAW --network host -v network-monitor-data:/var/lib/network-monitor -v "$PWD/network-monitor.toml:/etc/network-monitor/config.toml:ro" network-monitor:dev monitor --config /etc/network-monitor/config.toml --listen 0.0.0.0:8642` | `docs/USER-GUIDE.md` *Container* |
| quick start · source | `cargo build --release` then `sudo ./target/release/network-monitor monitor --config ./network-monitor.toml --listen 0.0.0.0:8642` | `README.md` *Quick start* |
| quick start · look first | `./target/release/network-monitor simulate --listen 127.0.0.1:8660 --history 30d --seed 0` | `README.md` *Quick start* |
| limits | IPv4 only; no redirects; `A`/`AAAA` over UDP only; `interface` on `icmp` and `tcp` only; the page has no authentication; the trust store is the one compiled in | `docs/USER-GUIDE.md` *Limits and deliberate choices* |
| faq | what it is; no database server to run; what it costs to run; no hosted version and no account; nothing leaves the host except Telegram when enabled; `root` or `NET_RAW` for `icmp` only; a new check is a module behind the trait; released under the MIT licence, with a link to `LICENSE` on GitHub | `LICENSE`, `Cargo.toml` |

Two rules for the prose, checked in review: every claim is traceable to the table above, and no
product named in the brief appears — the page describes this tool only.

**Verify:** `rg -n "<h[1-3]|id=\"" site/index.html` lists the ten blocks in the order of Step 1;
each command in `#quick-start` matches `README.md` or `docs/USER-GUIDE.md` verbatim
(`rg -nF "<the docker line>" docs/USER-GUIDE.md`).

### Step 3 — Take the screenshots from the simulation

The images are content, not build output: a static site cannot render the product at publication
time without a browser in CI, so they are committed together with the command that regenerates them.
They show a generated history, never a real one.

1. Build and run a four-target board, so the timeline shows every method:

   ```bash
   cargo build --release
   ./target/release/network-monitor simulate --listen 127.0.0.1:8660 --history 30d --seed 3 \
     --fault-rate 6 \
     --target 'icmp:1.1.1.1,label=edge-router north' \
     --target 'tcp:9.9.9.9:443,label=primary-endpoint' \
     --target 'dns:example.com@9.9.9.9,label=resolver' \
     --target 'http:https://example.com/health,label=public-site'
   ```

2. Capture, in a browser at a 1280×800 viewport: the timeline (default window, top of the page), the
   target page of `edge-router north` at `days=30d` scrolled to the widget row and the day-by-day
   table, and the timeline again at a 390×844 viewport.
3. Convert each capture to JPEG at quality 85, 1100 px wide for the two wide ones and 390 px wide for
   the narrow one, with the tool the machine has (`sips -s format jpeg -s formatOptions 85
   -Z 1100 in.png --out site/assets/timeline.jpg` on macOS); fall back to PNG if none is present.
4. Crop `site/assets/og.jpg` at 1200×630 from the timeline capture, framing the hero and the first
   lanes.
5. Write the four files into `site/assets/` and record their sizes in `## Implementation Notes`.

**Verify:** `du -ch site/assets/*.jpg site/index.html site/style.css site/assets/icon.svg` totals under
600 KB and every image is under 150 KB; each `<img>` in the page names its file and carries `alt`,
`width` and `height`.

### Step 4 — Add the files a crawler reads

- `site/index.html`: `<title>` of at most 60 characters (`network-monitor — reachability monitoring
  that records incidents`), a `meta description` of at most 160, `<link rel="canonical"
  href="https://dashie.github.io/network-monitor/">`, `og:type=website`, `og:title`, `og:description`,
  `og:url`, `og:image` (absolute `…/assets/og.jpg`), `og:image:width/height`, `twitter:card=
  summary_large_image`, and a JSON-LD `SoftwareApplication` with `name`, `applicationCategory`
  (`DeveloperApplication`), `operatingSystem` (`macOS, Linux, Windows`), `offers` at price `0` and
  `isAccessibleForFree: true`.
- `site/robots.txt`: `User-agent: *`, `Allow: /`, `Sitemap: https://dashie.github.io/network-monitor/sitemap.xml`.
- `site/sitemap.xml`: one `<url>` — the canonical address, `lastmod` `2026-11-15`, `changefreq`
  `monthly`, `priority` `1.0`.

The absolute addresses live in exactly these places: the canonical, the four `og:` URLs, the JSON-LD
`url`, `robots.txt` and `sitemap.xml`. Everything the visitor clicks is relative, so the page survives
a move to a domain with those six lines edited.

**Verify:** `rg -n "dashie.github.io" site/` lists only the canonical, the `og:url`, the `og:image`,
the JSON-LD `url`, `robots.txt` and `sitemap.xml`; `python3 -c "import xml.dom.minidom as m;
m.parse('site/sitemap.xml')"` exits 0; the `meta description` counts at most 160 characters
(`rg -o 'name="description" content="[^"]*"' site/index.html | wc -c`).

### Step 5 — Publish it from the workflow

`.github/workflows/pages.yml`, published on a push to `main` and on demand:

```yaml
name: Pages

on:
  push:
    branches: [main]
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: true

jobs:
  deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deploy.outputs.page_url }}
    steps:
      - uses: actions/checkout@v4
      - name: Assemble the site
        run: |
          rm -rf _site
          mkdir -p _site/assets
          cp -R site/. _site/
          cp assets/icon/icon.svg assets/icon/favicon-32.png _site/assets/
      - uses: actions/configure-pages@v5
      - uses: actions/upload-pages-artifact@v3
        with:
          path: _site
      - id: deploy
        uses: actions/deploy-pages@v4
```

The icons are copied from `assets/icon/`, which stays the single source of truth: the page references
`assets/icon.svg` and `assets/favicon-32.png` relatively, and the same files are embedded in the
binary by `src/web/mod.rs`. `_site/` is added to `.gitignore` so a local assembly never reaches a
commit.

The repository's Pages source must be switched once to **GitHub Actions** (Settings → Pages → Build
and deployment → Source). That is a manual step for the maintainer, recorded in the close report and
in `docs/DEVELOPER-GUIDE.md`; the workflow is a no-op until it is done.

**Verify:** `actionlint .github/workflows/pages.yml` when the tool is present, otherwise
`python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/pages.yml'))"` exits 0; the
assembled tree of the step was rehearsed locally
(`rm -rf _site && mkdir -p _site/assets && cp -R site/. _site/ && cp assets/icon/icon.svg
assets/icon/favicon-32.png _site/assets/ && find _site -type f | sort`) and contains only the page,
its stylesheet, the crawl files, the images and the two icons.

### Step 6 — Adopt the MIT licence

The page names MIT in its FAQ (Step 2); this step makes that statement true and puts the licence where
each reader looks for it.

1. `LICENSE` at the root: the standard MIT text, unmodified but for the copyright line —
   `Copyright (c) 2026 <the name in git config user.name>`. The holder line uses the repository author
   identity (`maurizio` at the time of writing); the maintainer may replace it with a legal name, and
   that is the only edit the file ever needs.
2. `Cargo.toml`: `license = "MIT"` next to `description`. `publish = false` stays, so the field is
   metadata for tools and humans, not a registry publication.
3. `README.md`: `Licence: MIT` in the opening block, and a `## Licence` section before
   `## Contributing` — one sentence and a link to `LICENSE`.
4. `CONTRIBUTING.md`: one sentence in the opening paragraph — a contribution is offered under the same
   licence as the project, with no separate agreement to sign.
5. `site/index.html` (written in Step 2): the FAQ answer names MIT and links `LICENSE` on GitHub. No
   other page or document states a licence, and the Apache-2.0 text under `assets/api/swagger-ui/` is
   left exactly as it is.

**Verify:** `head -1 LICENSE` is `MIT License` and `grep -c "Permission is hereby granted, free of
charge" LICENSE` is `1`; `cargo metadata --no-deps --format-version 1 | rg -o '"license":"[^"]+"'`
prints `"license":"MIT"`; `rg -n "MIT" README.md CONTRIBUTING.md site/index.html` shows the four
places; the quality gate is re-run after the manifest edit and stays green.

### Step 7 — Say it in the documents

- `README.md`: a `Website: https://dashie.github.io/network-monitor/` line in the opening block, and a
  first bullet in `## Documentation` pointing at the page.
- `AGENTS.md`: one row in the project-documents table —
  `| [site/index.html](./site/index.html) | The public landing page published on GitHub Pages | A
  feature, a command, a limit or a screenshot changes |`.
- `docs/DEVELOPER-GUIDE.md`: a `## Landing page` section after `## Web templates and assets` holding
  the file map, the preview command, the exact `simulate` command and viewport for the screenshots,
  the regeneration step for the JPEGs, how the workflow assembles and publishes, and the manual switch
  of the Pages source; and `site/` plus `.github/workflows/` added to the `## Repository layout` tree
  with one line each.
- `docs/ARCHITECTURE.md`: in the deployment section, one clause stating that the repository also
  publishes the static landing page from `site/` through the Pages workflow, that it is assembled from
  the sources and the icons and ships no part of the binary.

**Verify:** `rg -n "landing page|github.io" README.md AGENTS.md docs/*.md` shows the four passages and
no other file; `rg -n "site/" docs/DEVELOPER-GUIDE.md` includes both the section and the layout tree.

### Step 8 — Prove the page

1. Serve it and read it: `python3 -m http.server 8000 --directory <assembled tree>`, then open it.
2. Check the three widths in a browser (360, 768, 1280 CSS px): no horizontal scrollbar
   (`document.scrollingElement.scrollWidth === document.documentElement.clientWidth`), no element
   overlapping, the cards and the screenshots in the expected columns.
3. Disable JavaScript: the page is identical, and every `<details>` opens.
4. Follow every link: the four external targets answer `200` (`curl -sS -o /dev/null -w '%{http_code}'
   <url>`), and every in-page anchor exists.
5. Run the search for the names in the brief, case-insensitively, over `site/`, this plan, the
   workflow and the four edited documents; it must return nothing. Then run it once over the whole
   tree and confirm the only hits are the eight pre-existing files listed in the goal, so no new one
   was introduced.
6. Run the quality gate.

**Verify:** the six checks above, each with its result recorded in `## Implementation Notes`.

## Security considerations

Not a `security` plan, and the surface it adds is static text and images served by GitHub. Two points
are respected anyway: the page loads nothing from a third party (no font, no script, no analytics, so
no visitor is disclosed to anyone) and the screenshots come from `simulate`, which runs no probe and
reads no database, so no real address, host name or interface of the maintainer's network reaches a
public file. `docs/USER-GUIDE.md` already states that the served page has no authentication; the
landing page never publishes a reachable instance address.

The licence is a supply-chain statement, so it was checked before being chosen: every crate in
`Cargo.lock` carries MIT, Apache-2.0, ISC, BSD, 0BSD, Unlicense or CDLA-Permissive (verified with
`cargo metadata`), none of them forbids a permissive project licence, and the one vendored file set
— the API renderer — keeps its own Apache-2.0 text next to the files it covers. No dependency is
added, removed or upgraded by this plan.

## Verification plan

- **Automated:** the repository quality gate — `cargo fmt --check && cargo clippy --all-targets --
  -D warnings && cargo test` — must stay green; the site is not compiled, so this is a regression
  check on the repository, not on the page.
- **Static checks:** `rg` assertions of Steps 1, 2, 4, 6 and 7; the XML parse of `sitemap.xml`; the YAML
  parse of the workflow; the size budget of Step 3; the two searches of Step 8.5.
- **Manual, in a browser:** the three widths, the no-JavaScript pass, the link check, and a read of
  the page against the copy table of Step 2.
- **Rollback:** delete `site/`, `.github/workflows/pages.yml` and the `.gitignore` line, revert the
  four documentation passages, and set the Pages source back to *None*. Nothing else references them.

## Open questions

- [x] How is the page published? **Decided: a GitHub Actions workflow publishing an assembled `site/`
      directory**, because the repository's `/docs` folder holds the internal documents and
      "deploy from a branch → /docs" would publish them. Owner: maintainer.
- [x] Which address? **Decided: the project subpath `https://dashie.github.io/network-monitor/`, with
      relative links**, so a custom domain later touches six lines and no markup. Owner: maintainer.
- [x] Which images? **Decided: three real screenshots from `simulate`** — generated history, so no
      real data is published, and the same seed reproduces them. Owner: maintainer.
- [x] Which licence, and does the page state it? **Decided: MIT, stated on the page.** It is the
      shortest permissive text, one file, no `NOTICE` and no per-file headers; `MIT` is a valid SPDX
      identifier that `Cargo.toml`, GitHub and crates.io all read; and it is the only permissive choice
      that a GPL-2.0 project can also consume, which Apache-2.0 forbids — a real adoption barrier
      removed for a tool meant to sit on other people's appliances. The trade-off accepted is the lack
      of an explicit patent grant and of a trademark clause; `MIT OR Apache-2.0` (the Rust default)
      adds the patent grant at the cost of GPL-2.0 compatibility, and `BSD-3-Clause` adds a
      no-endorsement clause. Every licence of every crate in `Cargo.lock` is compatible with MIT, and
      the vendored renderer keeps its own Apache-2.0 text. Owner: maintainer.
- [x] The name already appears in eight files for a different reason: the import runbook of a real
      capability, its script and test, its example export, a decision record, `README.md`, `AGENTS.md`
      and one archived plan. Does this plan purge them? **Decided: no** — this plan writes nothing
      that names it, and the import runbook documents a capability rather than an inspiration, so
      renaming it (with its script, its test, its example and a link from two other documents) would
      also have to edit an archived plan, which the workflow forbids. If the maintainer wants the
      whole tree purged, that is its own plan, decided and reviewed on its own. Owner: maintainer.
- [x] Does the page link to `docs/USER-GUIDE.md` from GitHub, or stay one page with no outbound
      documentation links? **Decided: link to the user guide, the extension recipe and the API contract
      on GitHub**, in the extension paragraph and the footer: a visitor who wants detail should not
      have to search the repository. Owner: maintainer.
- [x] Is a fourth screenshot (the API reference at `/api/v1/docs`) worth its weight? **Decided: no** —
      the three chosen show the product itself; the API is linked, not pictured. Owner: maintainer.

## Implementation Notes

### Decisions taken at implementation time

- **Where the page lives.** `site/` at the root, published from the Actions workflow, at
  `https://dashie.github.io/network-monitor/`. One level deep, so every link inside the page is
  relative and the same tree works from a local server, a Pages preview and the live address.
  The only absolute addresses are the canonical link, `og:url`, the JSON-LD `url`, `sitemap.xml`
  and the footer links to GitHub.
- **No JavaScript at all.** The page carries one `<script>` element and it is
  `<script type="application/ld+json">` — structured data, not behaviour. The eight FAQ entries are
  `<details>`, so they open with scripting disabled.
- **Icons are copied, not duplicated.** The workflow copies `assets/icon/icon.svg` and
  `assets/icon/favicon-32.png` into `_site/assets/`, so the source of truth for the mark stays the
  directory the binary also embeds from.

### Discoveries

- **The release binary can be stale.** `target/release/network-monitor` predated the `simulate`
  subcommand, so the verbatim quick-start command failed with `unrecognized subcommand`. A
  `cargo build --release` first is part of the recipe, not an optimisation.
- **Running the documented `docker run` against the real `$PWD/network-monitor.toml` sent a real
  Telegram message.** The example configuration in the repository has the channel enabled; mounting it
  made the container report one transition and deliver it ("[example-health] notification sent"). The
  container was stopped immediately and the named volume `network-monitor-data` was left in place. The
  recipe on the page is the one from the user guide and is correct, but anyone rehearsing it should
  mount a copy with the `[telegram]` section removed.
- **Docker host networking does not expose a published port to the macOS host.** With
  `--network host` the listener binds inside the Docker VM, so `http://127.0.0.1:8642` on the host
  answers nothing even though the container is healthy. The page says host networking is required for
  interface-bound probes and does not promise a host-reachable port.
- **The browser harness pins the viewport** (~660 CSS px) and scales the capture, so
  `setViewportSize(1280, …)` does not produce a 1280 px render. Every screenshot was therefore taken by
  loading the running app in a fixed-size `<iframe>` (1280×1266, 1280×640, 390×844) and screenshotting
  that element, which is the only way to get a true desktop render. The recipe recorded in
  `docs/DEVELOPER-GUIDE.md` states the viewport, not the iframe.
- **A grid track sized with `auto` cannot hold a `pre` block.** The three quick-start snippets made
  the page 15 px wider than the viewport between 660 and 700 px; `.snippets` now declares
  `grid-template-columns: minmax(0, 1fr)`.
- **An image with `width="…" height="…"` and `width: 100%` is upscaled past its intrinsic size.**
  The phone capture (390 px wide) stretched to the full 992 px column and pushed the page to 6 000 px
  tall. The phone figure is now `max-width: min(390px, 100%)` and centred.

### Deviations

- **Screenshot sizes and crops.** The plan asked for the four images; the first pass produced a
  timeline with a dead white half, a target page cut mid-table and a social card that was a cropped
  screenshot with the logo sliced off. They were regenerated at the widths recorded in the guide and
  cropped to a clean boundary, and the social card was built as a composed 1200×630 layout.
  **Final sizes:** `timeline.jpg` 1040×1072 (140 KiB), `target.jpg` 1040×520 (76 KiB), `narrow.jpg`
  500×1082 (88 KiB), `og.jpg` 1200×630 (108 KiB) — 412 KiB for the four.
- **All four were generated again after each of the cosmetic corrections** described below, because
  the phone figure pictures the very header row and cell text they move, and the legend appears in the
  wide figure too. Capturing again from a fresh `simulate` session also changed the synthetic targets
  under the figures, so regenerating one image alone would have left figure 3 naming targets that
  figures 1 and 2 do not show. The four always come from one run and one seed.
- **The two wide captures are made with headless Chrome**, not with the browser harness: the harness
  pins the layout viewport to roughly 660 px and scales the result, and loading the app in a
  fixed-width `<iframe>` produced truncated pages once the stylesheet changed. The recorded commands
  are `--headless=new --force-device-scale-factor=2 --window-size=1280,1320` (timeline) and
  `--window-size=1280,640` (target), reproducible without any harness.
- **`--window-size=390` does not work in headless Chrome.** Its window has a minimum width of about
  500 px, so the page laid out at 500 px and the screenshot was cropped to 390: the phone figure
  arrived with its `CAUSE`/`LASTED` column outside the frame (caught in review). The phone capture is
  taken at a viewport that is really 390 px wide, and the recipe in the guide now says so and ends
  with "open the JPEG and check that `CAUSE` and `LASTED` are inside the image". `og.jpg` is still
  composed in the browser from the identity block and `target.jpg`.
- **Captions were rewritten to match what the images show.** The second figure no longer claims a
  day-by-day table it does not show, and the third describes the two-column fold of a fault row
  rather than four stacked widgets.
- **`.note` was added to `site/style.css`** as an alias of `.extension`, for the two closing
  paragraphs of the limits and quick-start sections. This is an addition beyond the plan; it is one
  selector in the stylesheet this plan owns.

### Extensions

- **`.gitignore` also ignores the event database** — `network-monitor.db`, `-shm` and `-wal`,
  requested by the maintainer after the implementation, to prevent a local run from leaving an
  untracked database at the root. The plan listed `.gitignore` for `_site/` only; the two additions
  share the file and its intent (keep local artefacts out of a commit), so they were folded into the
  same change with the maintainer's approval.


### Cosmetic corrections folded in

Three corrections were made after the page was finished, each found by reading the page at a narrow
width. They were recorded in plans of their own and are folded into this one at the maintainer's
request; the three files were removed, so this section is their only record.

**1. The stacked `cause` and `lasted` headers sat at the left of their columns.** The wide layout sizes
those headers with `table.faults th:nth-child(3) { width: 45%; }` and `nth-child(4) { width: 10%; }`.
Those shares keep applying in the stacked layout, where each cell is a grid item, so the header box was
45 % of its column — 72 px of 161 — and the right-aligned label read as left-aligned against the value
under it (header right edge 285 px against the value's 374 px). The fix is one rule inside the
`@media (max-width: 700px)` block: `table.faults th:nth-child(n) { width: 100%; }`, whose specificity
matches the shares and which comes later, so the header fills the grid area its column defines. Two
dead ends are worth recording: `justify-self: stretch` is already the computed value and changes
nothing, because a header is a table cell blockified into a grid item and its `auto` width is a
shrink-to-fit width; and a plain `table.faults th { width: 100% }` loses to the percentage shares on
specificity. After the fix the header and the value share a right edge at 360 px (344), 390 px (374),
700 px (676) and 1100 px (966/1070) — the wide layout's four alignment points unchanged.

**2. The duration and the marker read `4mand counting`.** The duration and the `and counting` span were
concatenated in the template, and the span is `display: block` in the wide layout but `display: inline`
under 700 px, where the two boxes then touched. The fix is a space in the markup —
`" <span class=\"ongoing\">and counting</span>"` — which renders between two inline boxes and is
collapsed away at the start of a block box, so the wide layout is untouched by construction. Measured
at 390 px the marker starts after the duration inside the cell (306 px, cell 213→374); at 1100 px its
left edge equals the cell's (966 px).

**3. The legend entry `fault too short to draw` became `short fault`.** It was the only entry written
as a clause among `up`, `down` and `unknown`, and it wrapped onto two lines on a telephone. The
template, the two assertions that pin the legend's words and order, and the sentence in the user guide
that explains the orange band were updated together.

Each of the three carries a regression test, and each test was proved to fail with its fix reverted
before being restored byte for byte: the CSS rule assertion, the `" <span class=\"ongoing\">"`
assertion, and the legend-order assertion. The quality suite stayed at 471 passed, 0 failed throughout.

### The gallery: three previews on one row, one full-screen viewer

Requested by the maintainer after the page was published: the screenshots section showed three
full-width images stacked, so reaching the quick start meant scrolling past a telephone capture
taller than the viewport. The section now shows three small previews on one row, and a preview
opens its capture over the page at full size.

- **`site/index.html`** — each `<figure>` keeps its `alt` text and `<img>` sizes and gains an
  `<a href="#shot-…">` around the image, a short `figcaption` (`The timeline`, `One target over
  30 days`, `On a telephone`), a hint line, and a `<div class="viewer" id="shot-…">` holding the
  same JPEG at full size with the sentence the caption used to carry.
- **`site/style.css`** — `.shots` is a fixed `repeat(3, 1fr)` grid, so the three previews stay on
  one row at every width; each tile is a square frame, and every picture keeps the proportions it
  was captured with (see the correction below).
- **The viewer is a `:target`, not a script.** One click opens an overlay (`display: grid` over
  `position: fixed; inset: 0`) and one click on its backdrop closes it, so the page keeps the single
  `<script>` it has — the JSON-LD block. The cost is recorded in the stylesheet: `Escape` does not
  close it, because a `:target` has nothing to bind a key to.
- **The paragraph under the previews is the only affordance.** A figure with a caption is not
  obviously clickable, so `Select a figure to open it full screen.` states it. Nothing else changed:
  the copy, the assets, the `<img>` `width`/`height` pairs (checked against `sips`) and the
  stylesheet's card grid are as they were, and the stale "cards and screenshots, never a fixed
  count" comment above `.cards` was cut back to the cards it still describes.

**Correction, reported by the maintainer after the first version: no capture kept its proportions,
in either mode.** Two separate causes, both now fixed and both measured.

- **The previews were cropped**, not stretched: the tile was a 4:3 frame with `object-fit: cover`,
  which scales the picture uniformly and hides the overflow — 72 % of a 1072 px-tall capture in a
  323×242 frame. A crop is not a distortion, but at this size it removes the header and the fault
  table that identify the capture, and it reads as a mangled picture.
- **The full-size view was stretched.** `.viewer img` set `max-width`/`max-height` but neither
  `width`/`height` nor `object-fit`, so the box was clamped by the binding `max-height` while the
  width stayed at the intrinsic 1040 px, and the default `object-fit: fill` painted the picture into
  that box: 1040×1072 became 1040×704 at 1280×800 — an aspect ratio of 1.477 against 0.970, 52 %
  too wide, and 1040×504 (2.063) at 1100×600. `width: auto; height: auto` makes Chromium derive the
  box from the ratio, and `object-fit: contain` guarantees it if the two ever disagree; the box then
  measures 683×704, 489×504 and 392×404 at those widths — 0.970 at all three.
- **The tile is square because the three captures are 0.97:1, 2:1 and 0.46:1.** With
  `object-fit: contain` a 4:3 frame would leave the telephone capture 112 px wide in a 323 px frame;
  a square leaves it 148 px, the least lopsided shape for this set. The frame's background fills what
  the picture does not cover, so nothing is cut and nothing is stretched.

**Measured after the correction**, at 320, 390, 620, 900, 1100 and 1280 CSS px: the three previews
stay on one row, no horizontal overflow appears at any width (86×86 px frames at 320, 321×321 px
from 1100 up), and every drawn picture matches its own ratio — 0.969/0.971 against 0.970,
2.006/1.982 against 2.000, 0.461/0.468 against 0.462. Each overlay was opened in turn at
390×844, 768×500, 1100×600 and 1280×800: the image box carries the capture's ratio (0.970, 2.000
and 0.462 at all four) and stays inside the viewport, and the caption's bottom edge stays inside it
too (819, 480, 580 and 780 px against viewports of 844, 500, 600 and 800). Opened again at
1280×400, 800×300, 390×500 and 360×640, the caption still ends inside the viewport at every one
(380 of 400, 280 of 300, 469 of 500, 620 of 640).

### Proof of Step 8, as executed

1. **Served and read.** `python3 -m http.server 8000 --directory _site`, then the page was read at
   every section in the browser and screenshotted. Every asset answers `200`:
   `index.html` (6 369 B stylesheet, 86 B `robots.txt`, 284 B `sitemap.xml`, 502 B `icon.svg`,
   748 B `favicon-32.png`, and the four JPEGs).
2. **Three widths.** At 360, 768 and 1280 CSS px:
   `document.scrollingElement.scrollWidth - document.documentElement.clientWidth` is `0` in all
   three. The card grid goes 1 / 2 / 3 columns, the screenshots are 328 / 720 / 992 px wide and never
   overflow. The only elements whose box extends past the viewport at 360 px are the three
   `<pre><code>` blocks, whose parent `pre` scrolls horizontally by design.
3. **JavaScript disabled.** The page has no behaviour to lose: the single `<script>` is JSON-LD and
   the eight `<details>` elements open natively.
4. **Every link.** All seven in-page anchors resolve to an existing `id` (`top`, `why`, `checks`,
   `how-it-works`, `screenshots`, `quick-start`, `faq`) and every local reference answers `200`. Of the
   external references, four answer `200` (the repository, the API contract, the user guide, the
   extension recipe) and two answer `404` **for a reason that is expected in this change**:
   `https://dashie.github.io/network-monitor/` does not exist until the Pages source is switched, and
   `…/blob/main/LICENSE` does not exist until this change is committed.
5. **The names in the brief.** Case-insensitive search over `site/`, this plan, the workflow and the
   four edited documents: no occurrence. `git diff` confirms the change introduces none. Over the whole
   tree the hits are the eight files recorded in the goals and nothing else.
6. **Quality suite.** `cargo fmt --check` ✓, `cargo clippy --all-targets -- -D warnings` ✓,
   `cargo test` ✓ — 471 passed, 0 failed.

## Implementation checklist

- [x] Step 1 — the shell and the stylesheet
- [x] Step 2 — the copy, claim by claim
- [x] Step 3 — the screenshots from `simulate`, with their sizes recorded
- [x] Step 4 — `canonical`, Open Graph, JSON-LD, `robots.txt`, `sitemap.xml`
- [x] Step 5 — the Pages workflow and the `_site/` ignore
- [x] Step 6 — the MIT licence: `LICENSE`, `Cargo.toml`, `README.md`, `CONTRIBUTING.md`
- [x] Step 7 — `README.md`, `AGENTS.md`, `DEVELOPER-GUIDE.md`, `ARCHITECTURE.md`
- [x] Step 8 — the six proofs, each recorded
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every configuration option,
      or the plan records why it needs no change
- [x] No product named in the brief appears in `site/`, this plan, the workflow, the four edited
      documents or the suggested commit message, and no new occurrence was introduced
- [x] `## Implementation Notes` added or updated in this plan

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-15
- **Verdict**: PASS
- **Suggested commit**: `feat(site): publish the project landing page on github pages`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — 471 passed, 0 failed, re-run after the documentation fixes)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | minor | `docs/DEVELOPER-GUIDE.md` | The `## Repository layout` tree gained the `LICENSE` line but lost the annotation of `Dockerfile`: it read `├── Dockerfile` instead of `├── Dockerfile            multi-stage static build for the container image`. Restored the original line. | fixed in review |
| 2 | minor | `docs/DEVELOPER-GUIDE.md` | The landing-page file table said "the sitemap and the canonical link are the only absolute addresses", contradicted by the page it documents: `og:url`, `og:image`, the JSON-LD `url`, the `robots.txt` `Sitemap:` line and the sitemap `loc` are absolute too. Replaced the clause with one that names all of them and keeps the relative-links statement. | fixed in review |
| 3 | nit | `site/style.css` | The full-screen overlay closes only on the backdrop, which is an `<a aria-label="Close">` with no visible control, and `Escape` does nothing — no script, by design, and the stylesheet states the trade-off. | reported, not fixed |
| 4 | nit | `tech-plans/008-landing-page.md` | Step 1 still describes the stylesheet as "one column that becomes two at 900 px for the cards and the screenshots"; the delivered page has an `auto-fit` card grid (1/2/3 columns) and three screenshot frames on one row. The executed geometry is recorded in Implementation Notes and Proof 8.2; the step prose was not amended, since the plan is not editable during review. | reported, not fixed |
| 5 | nit | `tech-plans/008-landing-page.md` | Proof 8.2 still quotes the earlier screenshot widths (328 / 720 / 992 px); the gallery correction later made the three previews 101 / 232 / 323 px wide at 360 / 768 / 1280. | reported, not fixed |

Documentation check after the two fixes: `rg -l "landing page|github.io" README.md AGENTS.md docs/*.md`
returns exactly `README.md`, `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/DEVELOPER-GUIDE.md`;
`rg -l "dashie.github.io" site/` returns exactly `site/index.html`, `site/robots.txt`,
`site/sitemap.xml`; `rg -n "^├── site/" docs/DEVELOPER-GUIDE.md` finds the layout entry. The quality
gate was re-run after the edits and stayed green.

### Gap analysis

- Goals: 12/12 achieved, with the evidence below.
- Scope: in scope. `src/web/page.rs`, `src/web/static/style.css`, `src/web/templates/page.html` and
  `tests/web.rs` sit outside the plan's "No change" list but are the folded cosmetic corrections
  recorded in Implementation Notes and approved by the maintainer; `docs/USER-GUIDE.md`, listed as "no
  change", carries the matching legend and `and counting` sentences of that same correction.
- Deviations: the image widths (1040/1040/500 instead of 1100/1100/390), the composed `og.jpg` and the
  headless-Chrome capture recipe are recorded under Deviations; the square-frame gallery and the
  `:target` viewer are recorded in the gallery note. All documented and legitimate.
- Additions: the screenshot gallery with full-size viewers, the `.gitignore` database entries and the
  three cosmetic corrections — all recorded and approved.
- Deferred: nothing planned was dropped. Switching the Pages source stays a manual step for the
  maintainer, recorded in the plan and in `docs/DEVELOPER-GUIDE.md`.

### Evidence run in this session

| Check | Result |
|---|---|
| Quality suite | `cargo fmt --check` ✓, `cargo clippy --all-targets -- -D warnings` ✓, `cargo test` ✓ — 471 passed, 0 failed |
| Size budget | `du -ch site/assets/*.jpg site/index.html site/style.css` = 436 KB; largest image 140 KB (`timeline.jpg`) — inside the 600 KB / 150 KB budget |
| Scripts and third parties | the only `<script>` is `application/ld+json`; no `@import`, no remote `src`, no font download |
| SEO | one `<h1>`; `<title>` 52 characters; `meta description` 141; `lang="en"`; canonical, `og:*` with 1200×630 and `twitter:card` present; `sitemap.xml` parses; `pages.yml` parses |
| Absolute addresses | confined to `site/index.html` (canonical, `og:url`, `og:image`, JSON-LD `url`), `site/robots.txt` and `site/sitemap.xml` |
| Anchors | 10 in-page anchors, every one resolving to an existing `id`; no broken anchor |
| Assembled tree | the workflow's `cp -R site/. _site/` rehearsal yields 10 files; all 10 URLs answer `200` |
| Widths | 360 / 768 / 1280 CSS px: horizontal overflow `0` at all three; card grid 1 / 2 / 3 columns; three square previews on one row (101 / 232 / 323 px) |
| No JavaScript | with script execution disabled the page renders identically, the first `<details>` opens on click, and a preview still opens its overlay (`#shot-timeline`, `display: grid`) |
| Names in the brief | none in `site/`, in the workflow or in this plan; the hits in `README.md`, `AGENTS.md` and `docs/DEVELOPER-GUIDE.md` are the eight pre-existing occurrences, and `git diff` confirms none was introduced |
| Licence | `LICENSE` opens with `MIT License`, carries one permission paragraph and `Copyright (c) 2026 Maurizio Merli`; `cargo metadata` reports `license: MIT` |
| Images | `<img>` `width`/`height` equal the files on disk (`sips`: 1040×1072, 1040×520, 500×1082, 1200×630); no EXIF, GPS or software tag in the four JPEGs |
| User guide | the `short fault` legend and the `and counting` wording match `src/web/templates/page.html` and the two media queries; no internal link was added; `## What the tool does` stays at three lines |
| Tests | the three corrections are pinned by executed assertions: the `" <span class=\"ongoing\">and counting</span>"` assertion, the legend-order assertion and the `table.faults th:nth-child(n)` stylesheet assertion |

### Known limitations

- The file-system clause of goal 1 could not be re-executed in this session: the browser harness
  refuses `file://` outside a trusted folder. The served tree was verified instead, and nothing in the
  page depends on being served (relative links, no fetch, no module).
- `actionlint` is not installed on this machine, so `pages.yml` was validated by parsing it as YAML
  only; the workflow has not run on GitHub yet.
- The three previews are 101 px wide at a 360 px viewport, so the captures are recognisable but not
  readable there; the full-size viewer is the way to read them.

### Recommendations for future work

- If the folded corrections should not travel with this commit, split them into
  `fix(web): keep the legend and the duration marker readable when stacked` before committing; the
  landing page itself is `feat(site): publish the project landing page on github pages`.
- Consider a visible close control for the viewer, or a two-line script, if visitors report that the
  backdrop is not an obvious way out.
