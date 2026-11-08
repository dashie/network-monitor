<!-- skill:block:pull-request-standard -->
# Pull request description instructions

These instructions define how a pull request title and description are written in this
repository, by a person or by a tool. The description is what a reviewer reads before
the diff: it must let them decide, quickly, whether the change is correct, complete, and
in scope.

Every text in a pull request is written in English.

## Title

The title is a Conventional Commits subject, because a squash merge turns it into the
commit subject on the default branch:

- Format: `<type>[(scope)][!]: <subject>`.
- Imperative, lowercase after the type, at most 72 characters, no trailing period.
- The same type list, scope rules, and breaking-change marker as the commit message
  standard.

## Description

Keep it short. Reviewers stop reading long descriptions; they read diffs.

1. **Problem** — what is wrong or missing today, in two or three sentences.
2. **Approach** — what the change does and why this approach instead of the obvious
   alternative.
3. **Verification** — the exact commands run and their outcome, plus what a reviewer
   should try by hand.
4. **Scope** — what the change deliberately does not do.

Omit a section rather than filling it with `N/A`.

## Required content

- Link the issue the pull request resolves: `Closes #123` or `Refs #123`.
- Name the affected modules or components.
- Mention schema, configuration, or contract changes and their migration.
- State the user-visible effect, or explicitly state that there is none.
- Include screenshots or recordings for visible interface changes.

## Rules

- One concern per pull request; split unrelated changes.
- Every pull request includes the tests of the code it touches; without them it is not
  ready for review.
- No change reaches into another module's internals; a change to a published contract
  comes with an architecture decision record link.
- Prefer the simplest design that works; speculative abstraction is rejected in review.
- Never include secrets, credentials, tokens, or personal data in the description,
  the title, or a screenshot.
- Do not paste a raw diff, a file list, or a generated changelog.
<!-- skill:end:pull-request-standard -->
