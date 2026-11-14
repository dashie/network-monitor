# Agent objective — network-monitor

Copy this file to `.agent/goal.md` and keep it current. `.agent/run.sh` sends its
content verbatim as the prompt of an unattended run, so write it for an agent that
cannot ask questions.

## Objective

One paragraph: the outcome you want, in terms of observable behaviour.

## Done criteria

- [ ] Every criterion is verifiable by a command or a test, not by opinion.
- [ ] The build, the tests, and the linter pass.
- [ ] Documentation affected by the change is updated.

## Constraints

- Own only the files needed for the objective; do not refactor unrelated code.
- Never modify `.devcontainer/`, `.agent/`, or CI configuration unless the objective
  requires it.
- Never commit or push: the sandbox refuses it and the user owns Git history.
- Write everything in English.

## Verification

List the exact commands the agent must run before reporting completion.

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Out of scope

List what must not be touched, so the agent does not expand the change.

## Notes

Known context, links to issues, or previous attempts worth reading.
