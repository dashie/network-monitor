# network-monitor — Agent Instructions

`network-monitor` is a command-line reachability monitor written in Rust. It probes a list of IP addresses with ICMP echo requests defined in a TOML file, optionally from a specific network interface, and records only the moments a target becomes reachable or unreachable in an embedded SQLite database.

The repository holds the documentation baseline and the implementation that produces one dependency-free executable for macOS, Linux, and Windows.

<!-- skill:block:agents-intro before="## Non-negotiable rule: English only" -->
This file is the single source of truth for AI agents working in this repository. It
is read natively as `AGENTS.md` by GitHub Copilot CLI, GitHub Copilot in VS Code,
the VS Code Local harness, and OpenAI Codex; `CLAUDE.md` imports it so Claude-based
harnesses read the same text. Never duplicate these instructions in another file.
<!-- skill:end:agents-intro -->

<!-- skill:block:agents-english-only -->
## Non-negotiable rule: English only

Every artefact in this repository is written in English, and only in English. There
is no second language, no localised copy, and no translation.

This applies to:

- documentation, headings, tables, and diagrams;
- commit messages, branch names, pull request and issue text;
- source code, identifiers, file and folder names, test names, fixtures;
- comments, log messages, error messages, and user-facing strings;
- configuration files and inline script comments.

If a request implies writing any of these in another language, do not comply: write
the artefact in English and state that English is the only allowed language. A
non-English artefact is a defect, not a style preference.
<!-- skill:end:agents-english-only -->

<!-- skill:block:agents-what-this-is -->
## What this repository is

This repository is documented in layers, each with a single owner document. Read the
document you need before changing behaviour.

| Document | What it defines | Update it when |
|---|---|---|
| [README.md](./README.md) | Purpose, quick start, documentation map | Setup or entry points change |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Branching, review, definition of done | The contribution process changes |
| [docs/PRODUCT.md](./docs/PRODUCT.md) | Users, scope, functional behaviour, business rules | Requirements or scope change |
| [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) | arc42 structure, C4 views, components, contracts, invariants | Structure, interfaces, or constraints change |
| [docs/ADR.md](./docs/ADR.md) | Index of architecture decisions and their rationale | A decision is made, accepted, or superseded |
| [docs/GLOSSARY.md](./docs/GLOSSARY.md) | One precise definition per domain term | A domain term is introduced or redefined |
| [docs/DEVELOPER-GUIDE.md](./docs/DEVELOPER-GUIDE.md) | Build, run, test, extend, and the Git standards | Tooling, commands, or conventions change |

Rules for these documents:

- They are written in English, in Markdown, and use the exact paths above.
- They are the source of truth. When code and document disagree, fix whichever is
  wrong in the same change; never leave the contradiction in place.
- Never duplicate a definition. A term lives in the glossary, a decision lives in the
  ADR, a requirement lives in the product document. Link instead of copying.
- Keep them short. A document nobody reads is worse than no document.
<!-- skill:end:agents-what-this-is -->

<!-- Project documents: list project-specific documents below, one line each, using the
     same three columns as the map above. Keep them outside the managed block so an
     update of this file never drops them. -->

<!-- skill:block:agents-engineering-principles before="## Git standards" -->
## Engineering principles

Three rules decide how code is designed, written, and delivered in this repository.
They are not style preferences: a change that violates one is incomplete.

### 1. Simplicity first (KISS)

Choose the simplest design that satisfies the requirement, at every level — system
structure, module boundaries, data model, and individual functions.

- One obvious way per problem: no speculative abstraction, framework, or indirection
  layer for a problem this project does not have.
- Every moving part is a cost. Remove a dependency, a layer, or a configuration knob
  before adding one.
- Prefer a small readable implementation over a clever one. If a reviewer needs a
  diagram to follow a function, simplify the function.
- Extra complexity must be justified by a documented constraint. When it is not, the
  simpler design wins and the trade-off is recorded in [docs/ADR.md](./docs/ADR.md).

### 2. Modularity and decoupling

The system is built from modules with explicit, narrow responsibilities. Each module
is independently testable, independently evolvable, and independently understandable.

- One module owns one responsibility and hides its internals behind a published
  contract. No other module reads its data or calls its private helpers.
- Modules depend only on each other's public contract, never on internals, and never
  form dependency cycles.
- Changing a module's internals must not force a change in its consumers; testing a
  module must not require starting the others — substitute fakes at the contract
  boundary.
- Boundaries exist to make development, troubleshooting, and refactoring cheap: a
  defect is localised in one module, and a rewrite stays local to one module.

### 3. Tests ship with the change

Every change includes the tests of the part it changes, in the same commit. A change
without tests is not done, not reviewable, and not mergeable.

- New behaviour: at least one test that fails without the change and passes with it.
- Changed behaviour: update the affected tests in the same change; never delete or
  weaken a test to make a build green.
- Defect fix: a regression test that reproduces the defect before the fix.
- Tests are a deliverable like the code: readable, deterministic, isolated from other
  modules, and fast enough to run on every change.
- The commands are in [docs/DEVELOPER-GUIDE.md](./docs/DEVELOPER-GUIDE.md#test); the
  definition of done is in
  [CONTRIBUTING.md](./CONTRIBUTING.md#definition-of-done).
<!-- skill:end:agents-engineering-principles -->

<!-- skill:block:agents-git-standards -->
## Git standards

Commit messages follow Conventional Commits and stay concise:

- Format: `<type>(<scope>): <subject>`, imperative, lowercase after the type, at most
  72 characters, no trailing period.
- One logical change per commit; split a change whose subject needs the word "and".
- Breaking changes use `!` after the type or scope plus a `BREAKING CHANGE:` footer.
- Body only when the reason is not obvious; explain why, not what.

The complete contract — types, scopes, subject and body rules, footers, and examples —
is [.github/copilot-commit-message-instructions.md](./.github/copilot-commit-message-instructions.md).
Pull request titles and descriptions follow
[.github/copilot-pull-request-instructions.md](./.github/copilot-pull-request-instructions.md).
Branch naming, review, and merge policy are in
[docs/DEVELOPER-GUIDE.md](./docs/DEVELOPER-GUIDE.md#git-standards).
<!-- skill:end:agents-git-standards -->

<!-- skill:block:agents-git-approval -->
## Git: user-approved changes only

Git history belongs to the user. Never commit or push on your own initiative.

- Never run `git commit`, `git push`, `git tag`, `git merge`, `git rebase`,
  `git reset --hard`, `git clean`, `git add` toward an unreviewed commit, or
  `gh pr create` / `gh pr merge`, unless the user explicitly approves that exact
  action in the current turn.
- Read-only commands are always allowed: `git status`, `git diff`, `git log`,
  `git show`, `git branch --list`.
- Staging is not approval, and an approval never carries over to a later turn.
- When the work is ready, stop and report: what changed, which files, how it was
  verified, and the proposed commit message. The user reviews, approves, and
  commits.
<!-- skill:end:agents-git-approval -->

<!-- skill:block:agents-working-agreement -->
## Working agreement

1. Read [docs/PRODUCT.md](./docs/PRODUCT.md) and
   [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) before changing behaviour.
2. Prefer the existing patterns in the codebase over new abstractions.
3. Make surgical changes: touch only what the task requires, keep unrelated
   refactors out of the change.
4. Respect the invariants listed in [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md);
   a change that breaks one is a breaking change and needs an ADR.
5. Ask instead of inventing: if a requirement, term, or constraint is missing from
   the documents, ask the user; do not guess and do not silently expand scope.
6. Update the document affected by the change in the same commit.
7. Run the build, tests, and linter defined in
   [docs/DEVELOPER-GUIDE.md](./docs/DEVELOPER-GUIDE.md) before reporting completion.
8. Keep the design simple (KISS) and the modules decoupled: solve the problem in one
   place, at the level where it belongs, with the fewest moving parts.
9. Ship the tests of the part you change in the same change; a change without tests
   is incomplete.
10. Never commit secrets, credentials, or generated artefacts.
11. Never commit or push: only the user approves and saves a change in Git.
12. Write everything in English.
<!-- skill:end:agents-working-agreement -->
