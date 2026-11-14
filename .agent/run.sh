#!/usr/bin/env bash
# Unattended launcher for the GitHub Copilot CLI inside the AI sandbox.
#
# It exists so a run can last hours: every permission prompt is removed, the
# output is logged to .agent/logs/, the session can be detached into tmux, and a
# wall-clock timeout protects against a stuck loop.
#
# Privileged flags used here (verified against the Copilot CLI command reference):
#   --allow-all              grant all tool, path, and URL permissions
#   --no-ask-user            disable the ask_user tool (no questions, ever)
#   --max-autopilot-continues automatic continuations outside an objective
#   -p PROMPT                programmatic run, exits when the objective is done
#
# Enterprise policy can suppress the allow-all flags; the CLI then prints a policy
# warning and keeps asking for permissions. See docs/AGENT-SANDBOX.md.
set -euo pipefail

ORIGINAL=("$@")
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG_DIR="$REPO_ROOT/.agent/logs"
GOAL_FILE="$REPO_ROOT/.agent/goal.md"
CONTINUES="${MAX_AUTOPILOT_CONTINUES:-50}"
MODEL="${COPILOT_MODEL:-}"
RESUME=""
SESSION_NAME="${AGENT_SESSION_NAME:-agent}"
TIMEOUT="${AGENT_TIMEOUT:-0}"
DETACH=0
DRY_RUN=0
EXTRA=()

usage() {
  cat <<'EOF'
Usage: .agent/run.sh [options] [-- extra copilot arguments]

  --goal FILE        Objective file (default: .agent/goal.md)
  --continues N      Max automatic continuations (default: 50)
  --model NAME       Model to use, e.g. claude-sonnet-5.5 or auto
  --resume ID        Resume a previous session id
  --timeout SECONDS  Wall-clock timeout, 0 disables (default: 0)
  --detach           Run inside a detached tmux session named after --session
  --session NAME     tmux session name (default: agent)
  --dry-run          Print the command instead of running it
  -h, --help         Show this help
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --goal) GOAL_FILE="$2"; shift 2 ;;
    --continues) CONTINUES="$2"; shift 2 ;;
    --model) MODEL="$2"; shift 2 ;;
    --resume) RESUME="$2"; shift 2 ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --session) SESSION_NAME="$2"; shift 2 ;;
    --detach) DETACH=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    --) shift; EXTRA=("$@"); break ;;
    *) echo "run.sh: unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v copilot >/dev/null 2>&1 || {
  echo "run.sh: the copilot CLI is not installed in this container" >&2
  exit 127
}

if [ ! -f "$GOAL_FILE" ]; then
  echo "run.sh: goal file not found: $GOAL_FILE" >&2
  echo "        fill .agent/goal.md with the objective and the done criteria first" >&2
  exit 1
fi

mkdir -p "$LOG_DIR"
PROMPT="$(cat "$GOAL_FILE")"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
LOG_FILE="$LOG_DIR/$STAMP-run.log"

ARGS=(
  --allow-all
  --no-ask-user
  --max-autopilot-continues="$CONTINUES"
  --log-dir "$LOG_DIR"
)
[ -n "$MODEL" ] && ARGS+=(--model "$MODEL")
[ -n "$RESUME" ] && ARGS+=(--resume "$RESUME")
[ "${#EXTRA[@]}" -gt 0 ] && ARGS+=("${EXTRA[@]}")
ARGS+=(-p "$PROMPT")

if [ "$DRY_RUN" -eq 1 ]; then
  printf 'copilot'; printf ' %q' "${ARGS[@]}"; printf '\n'
  echo "# prompt from $GOAL_FILE, log target $LOG_FILE"
  exit 0
fi

if [ "$DETACH" -eq 1 ] && [ "${AGENT_RUN_CHILD:-0}" != "1" ]; then
  command -v tmux >/dev/null 2>&1 || {
    echo "run.sh: tmux is required for --detach" >&2
    exit 127
  }
  quoted=""
  for argument in "${ORIGINAL[@]}"; do
    quoted+=" $(printf '%q' "$argument")"
  done
  tmux new-session -d -s "$SESSION_NAME" -c "$REPO_ROOT" \
    "AGENT_RUN_CHILD=1 AGENT_SESSION_NAME=$SESSION_NAME $0$quoted; echo '[agent-run] session ended'; exec bash"
  echo "[agent-run] detached in tmux session '$SESSION_NAME'"
  echo "[agent-run] attach: tmux attach -t $SESSION_NAME"
  echo "[agent-run] logs:   $LOG_DIR"
  exit 0
fi

echo "[agent-run] objective: $GOAL_FILE"
echo "[agent-run] log:       $LOG_FILE"
echo "[agent-run] timeout:   ${TIMEOUT}s (0 = none)"
echo

set +e
if [ "$TIMEOUT" -gt 0 ] 2>/dev/null; then
  timeout --signal=INT --kill-after=60 "$TIMEOUT" copilot "${ARGS[@]}" 2>&1 | tee -a "$LOG_FILE"
  status="${PIPESTATUS[0]}"
else
  copilot "${ARGS[@]}" 2>&1 | tee -a "$LOG_FILE"
  status="${PIPESTATUS[0]}"
fi
set -e

echo
echo "[agent-run] finished with status $status"
echo "[agent-run] log: $LOG_FILE"
if [ "$status" -ne 0 ]; then
  echo "[agent-run] resume with: .agent/run.sh --resume <session-id>"
fi
exit "$status"
