#!/usr/bin/env bash
# Stack tooling for network-monitor inside the AI sandbox image.
# Runs as root during the image build. Keep it idempotent and non-interactive.
set -eux

rustup component add rustfmt clippy
rustup target add x86_64-unknown-linux-musl
apt-get update \
    && apt-get install -y --no-install-recommends musl-tools sqlite3 \
    && rm -rf /var/lib/apt/lists/*
