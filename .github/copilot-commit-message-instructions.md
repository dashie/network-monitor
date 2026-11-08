<!-- skill:block:commit-message-standard -->
# Commit message instructions

These instructions define how every commit message in this repository is written, by a
person or by a tool. A commit message explains one change to a reader who cannot see the
working session: it must stand on its own, stay short, and be consistent with every
other commit.

Every text in a commit message is written in English.

## Format

The format is Conventional Commits:

```
<type>[optional scope][!]: <subject>

[optional body]

[optional footer(s)]
```

Exactly one blank line separates the subject from the body, and the body from the
footers.

## Type

Exactly one type, chosen from this list:

| Type | Use for |
|---|---|
| `feat` | A new user-visible capability |
| `fix` | A defect correction |
| `docs` | Documentation only |
| `style` | Formatting, whitespace, or lint fixes with no behaviour change |
| `refactor` | Restructuring with no behaviour change |
| `perf` | A performance improvement |
| `test` | Adding or fixing tests |
| `build` | Build system, dependencies, packaging |
| `ci` | Continuous integration configuration |
| `chore` | Maintenance that fits nowhere else |
| `revert` | Reverting a previous commit |

Never invent a new type. If none fits, the change is probably too broad and must be
split.

## Scope

Optional, lowercase, and drawn from the repository layout: a directory, a component, or
a module name (for example `api`, `cli`, `docs`). Use the narrowest scope that names the
changed part. Omit the scope when the change is repository-wide.

## Subject

- Imperative mood, as if completing "this commit will …": `add retry to metrics upload`.
- Lowercase after the type and scope, except for proper nouns.
- At most 72 characters, no trailing period.
- State the effect on the system, not the activity or the file touched:
  `reject empty metric batches`, not `work on validation` or `update validator.py`.
- No issue or ticket number in the subject; it belongs in a footer.
- No emoji, no em dash, no `[tags]`, no capitalised first letter of the type.

## Body

Omit the body when the diff already explains the change. Include it when the reason is
not obvious from the code:

- Explain *why*, not *what*; the diff shows the what.
- Wrap at 72 characters, separate paragraphs with a blank line.
- State the consequence the reader could plausibly get wrong, for example a trade-off,
  an unchanged behaviour that looks changed, or a workaround and its cause.
- Never include commented-out code, debug output, secrets, credentials, tokens, or
  personal data.

## Footers and breaking changes

- Reference the issue with `Refs: #123`, or `Closes: #123` when the commit closes it.
- List co-authors with `Co-authored-by: Name <email>`.
- A breaking change appends `!` after the type or scope and requires a footer that
  starts with `BREAKING CHANGE:` and states what breaks and how to migrate.

## Do not

- Do not use `wip`, `temp`, `fixes`, `stuff`, `update`, or `changes` as the subject.
- Do not describe the act of editing a file when the effect is something else.
- Do not combine unrelated changes: if the subject needs the word "and", split the
  commit.
- Do not reference a person in the subject or body; the author is recorded by Git.

## Examples

```
feat(upload): batch metrics requests by minute
```

```
fix(parser): reject timestamps without timezone

Naive timestamps were stored as UTC, silently shifting non-UTC input by the local
offset. Reject them at the boundary until the parser can infer the zone.
```

```
docs(adr): record storage engine choice
```

```
refactor!: move metric types into a shared module

BREAKING CHANGE: metric payloads now use `observed_at` instead of `timestamp`;
consumers must read the new field before upgrading.
```

Rejected, with the reason:

```
Added retries                  past tense, no type, no scope
fix: fix bug                   describes the activity, not the effect
feat: add X and fix Y          two changes, must be split
chore: update package.json     names the file, not the outcome
feat(api): retry uploads (3x)  parentheses and numbers belong in the body
```
<!-- skill:end:commit-message-standard -->
