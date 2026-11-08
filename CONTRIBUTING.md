# Contributing to network-monitor

`network-monitor` is small, operational software: its value is that its behaviour is predictable, its storage is tiny, and it runs unchanged on a laptop, in a container, and on a NAS. Contributions are judged on those properties first.

Every text artefact — code, comments, identifiers, documentation, commit messages, issues — is written in English.

<!-- skill:block:contributing-before-you-start -->
## Before you start

- Read [docs/PRODUCT.md](./docs/PRODUCT.md) for the intended behaviour and
  [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) for the constraints you must preserve.
- Read [docs/DEVELOPER-GUIDE.md](./docs/DEVELOPER-GUIDE.md) to set up your environment.
- Open an issue before starting any change that alters behaviour, scope, or public
  interfaces. Trivial fixes (typos, broken links, formatting) can go straight to a
  pull request.
- All text — code, comments, identifiers, documentation, commit messages, issues —
  is written in English only.
<!-- skill:end:contributing-before-you-start -->

## Development workflow

1. Open an issue (or pick an existing one) describing the problem and the expected observable behaviour. Anything that changes behaviour, configuration, the schema, or a public contract needs an issue first.
2. Branch from `main` with a name from the scheme below, one logical change per branch.
3. Implement the change together with its tests, and update the document it touches in the same branch.
4. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` locally; a red pipeline wastes a review round.
5. Open a pull request titled in Conventional Commits form, describing the problem, the approach, and the verification.
6. Address review feedback by pushing commits to the same branch; do not force-push a branch under review without saying so.
7. Squash-merge into `main` once approved, with the pull request title as the commit subject, then delete the branch.

## Branching

Trunk-based, with `main` always releasable and every change entering through a pull request.

- Base branch: `main`. Never commit directly to it.
- Names: `feat/<slug>`, `fix/<slug>`, `docs/<slug>`, `chore/<slug>`, `refactor/<slug>`. Lowercase, hyphen-separated, derived from the issue.
- Lifetime: days, not weeks. Rebase on `main` before requesting review so the history stays linear.
- One concern per branch: a branch whose title needs the word "and" is two branches.

<!-- skill:block:contributing-commit-messages -->
## Commit messages

Commits follow Conventional Commits and must stay concise: one logical change per
commit, an imperative subject, no trailing period.

The complete contract is
[.github/copilot-commit-message-instructions.md](./.github/copilot-commit-message-instructions.md);
read it before writing a message. Branching, review, and merge policy are in
[docs/DEVELOPER-GUIDE.md](./docs/DEVELOPER-GUIDE.md#git-standards).
<!-- skill:end:contributing-commit-messages -->

## Review

- One approving review is required before merge; the author merges.
- Reviewers check correctness first, then the invariants of [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), then style.
- A new configuration option must come with its command-line override, its default, and a test of the precedence rule.
- A change to a published contract (configuration keys, subcommands, output format, schema, probe interface) needs the matching update to [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and, when it is hard to reverse, an ADR in [docs/ADR.md](docs/ADR.md).
- A new dependency must be justified in the pull request: the project provides a single binary, so every crate is weighed against that goal.
- Review conversations are resolved by the author, not dismissed. Discuss in the thread; apply the decision in code.
- Never weaken or delete a test to make a review simpler.

## Definition of done

A change is done when all of the following hold:

- [ ] `cargo test` passes and the new or changed behaviour is covered by a test that fails without the change.
- [ ] `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean.
- [ ] Every new configuration key has a default, a command-line override, and a precedence test.
- [ ] The documents touched by the change are updated in the same pull request.
- [ ] All text is in English.
- [ ] No `TODO` without a linked issue.
- [ ] No new runtime dependency: the release binary still links only the platform C runtime.
- [ ] The pull request description states the verification performed.

<!-- skill:block:contributing-pull-requests -->
## Pull requests

- One concern per pull request; split unrelated changes.
- Description states the problem, the approach, and how it was verified.
- Link the issue the pull request resolves.
- Include screenshots or recordings for visible changes.
- Every pull request includes the tests of the code it touches; without them it is
  not ready for review.
- Keep the modules decoupled: no change reaches into another module's internals, and a
  change to a published contract comes with an ADR.
- Prefer the simplest design that works (KISS); speculative abstraction is rejected in
  review.
- Ensure CI is green and review conversations are resolved before merging.
<!-- skill:end:contributing-pull-requests -->
