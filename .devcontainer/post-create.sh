#!/usr/bin/env bash
# One-time container setup for the network-monitor AI sandbox.
# Runs as the non-root user after the container is created.
set -euo pipefail

echo "==> AI sandbox setup for network-monitor"

# Git identity lives only in this container; no host gitconfig is mounted.
git config --global user.name "${SANDBOX_GIT_NAME:-AI Sandbox}"
git config --global user.email "${SANDBOX_GIT_EMAIL:-ai-sandbox@localhost}"
git config --global init.defaultBranch main
git config --global core.pager cat
git config --global advice.detachedHead false

if ! git config --global --get-all safe.directory | grep -qx "/workspace"; then
  git config --global --add safe.directory /workspace
fi

cargo --version
cargo fetch --locked || echo "cargo fetch failed: check the network and Cargo.lock"

echo "==> toolchain"
git --version || true
node --version || true
npm --version || true
copilot --version || true

copilot_home="${COPILOT_HOME:-$HOME/.copilot}"
if [ ! -f "${copilot_home}/config.json" ]; then
  cat <<'EOF'

==> GitHub Copilot CLI is not authenticated in this container yet.
    Run `copilot` once and complete the device login. Credentials are stored in the
    COPILOT_HOME volume and survive rebuilds. Never copy host credentials into the
    sandbox.
EOF
fi

cat <<'EOF'

==> Sandbox ready.
    Git history is read-only: `git commit`, `git push`, `git tag`, `git merge`,
    `git rebase`, `git reset` and `git clean` are refused by /usr/local/bin/git.
    Long unattended runs: bash .agent/run.sh --detach
    Details: docs/AGENT-SANDBOX.md

EOF
