#!/usr/bin/env bash
# git-guard: enforces the read-only Git policy inside the AI sandbox.
#
# Installed as /usr/local/bin/git, which precedes /usr/bin/git in PATH. Every
# history-changing subcommand is refused; everything read-only passes through.
# This is a guardrail for a cooperative agent, not a jail: real isolation comes
# from the container. See docs/AGENT-SANDBOX.md.
set -uo pipefail

BLOCKED_SUBCOMMANDS="commit push tag merge rebase reset clean cherry-pick revert filter-branch update-ref"

real_git=""
for candidate in /usr/bin/git /bin/git /usr/local/git/bin/git /opt/git/bin/git; do
  if [ -x "$candidate" ]; then
    real_git="$candidate"
    break
  fi
done

if [ -z "$real_git" ]; then
  echo "git-guard: no real git binary found" >&2
  exit 127
fi

subcommand=""
skip_next=0
for argument in "$@"; do
  if [ "$skip_next" -eq 1 ]; then
    skip_next=0
    continue
  fi
  case "$argument" in
    -C|-c|--git-dir|--work-tree|--namespace|--exec-path|--config-env)
      skip_next=1
      continue
      ;;
    --git-dir=*|--work-tree=*|--namespace=*|--exec-path=*|--config-env=*)
      continue
      ;;
    -*)
      continue
      ;;
    *)
      subcommand="$argument"
      break
      ;;
  esac
done

for blocked in $BLOCKED_SUBCOMMANDS; do
  if [ "$subcommand" = "$blocked" ]; then
    cat >&2 <<EOF
git-guard: 'git $subcommand' is disabled in this sandbox (read-only Git policy).

Git history belongs to the user. Prepare the change, run the tests, and report what
changed together with a proposed commit message. The user reviews, approves, and
commits on the host.

Read-only commands are allowed: git status, git diff, git log, git show, git branch.

Users only: bypass deliberately with the real binary, for example
  $real_git $subcommand ...
EOF
    exit 97
  fi
done

exec "$real_git" "$@"
