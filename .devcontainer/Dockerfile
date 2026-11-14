# AI sandbox image for network-monitor.
# Design rules: non-root user, no sudo, no Docker socket, no host credentials.

ARG BASE_IMAGE=mcr.microsoft.com/devcontainers/rust:1
FROM ${BASE_IMAGE}

ARG DEBIAN_FRONTEND=noninteractive
ARG USERNAME=dev
ARG USER_UID=1000
ARG USER_GID=1000

USER root

# Base tooling for agent work. Kept small on purpose.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        jq \
        ripgrep \
        less \
        nano \
        tmux \
        unzip \
        procps \
    && rm -rf /var/lib/apt/lists/*

# Node.js is the runtime of the GitHub Copilot CLI. The base image may already
# provide it; install only when missing.
RUN if ! command -v node >/dev/null 2>&1; then \
        curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
        && apt-get install -y --no-install-recommends nodejs \
        && rm -rf /var/lib/apt/lists/*; \
    fi

# Stack tooling for this repository, applied by stack-tools.sh.
COPY --chown=root:root stack-tools.sh /tmp/stack-tools.sh
RUN chmod 0755 /tmp/stack-tools.sh && /tmp/stack-tools.sh && rm -f /tmp/stack-tools.sh

# GitHub Copilot CLI, pinned.
RUN npm install --global --no-fund --no-audit @github/copilot@1.0.112 \
    && npm cache clean --force

# Non-root user, no privilege escalation.
# Official dev-container images ship a `vscode` user on UID/GID 1000; it is removed so
# the sandbox user keeps UID 1000 and bind-mounted files stay writable on Linux hosts.
RUN set -eux; \
    if dpkg -s sudo >/dev/null 2>&1; then \
        SUDO_FORCE_REMOVE=yes apt-get purge -y sudo || true; \
    fi; \
    rm -f /usr/bin/sudo /usr/bin/sudoedit; \
    rm -rf /etc/sudoers /etc/sudoers.d; \
    ! command -v sudo; \
    for existing in $(awk -F: -v uid="${USER_UID}" '$3==uid {print $1}' /etc/passwd); do \
        userdel -r "$existing" 2>/dev/null || true; \
    done; \
    if getent group "${USER_GID}" >/dev/null; then \
        groupmod -n "${USERNAME}" "$(getent group "${USER_GID}" | cut -d: -f1)"; \
    else \
        groupadd --gid "${USER_GID}" "${USERNAME}"; \
    fi; \
    useradd --uid "${USER_UID}" --gid "${USER_GID}" --create-home --shell /bin/bash "${USERNAME}"; \
    mkdir -p "/home/${USERNAME}/.copilot" "/home/${USERNAME}/.npm" "/home/${USERNAME}/.cache" "/home/${USERNAME}/.local"; \
    chown -R "${USER_UID}:${USER_GID}" "/home/${USERNAME}"

# git-guard: a `git` shim that refuses history-changing commands.
# /usr/local/bin precedes /usr/bin in PATH, so it wins.
COPY --chown=root:root git-guard.sh /usr/local/bin/git
RUN chmod 0755 /usr/local/bin/git

USER ${USERNAME}
WORKDIR /workspace

CMD ["sleep", "infinity"]
