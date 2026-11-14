# Multi-stage build: the release binary is statically linked (musl, SQLite bundled),
# so the runtime stage needs nothing but the executable.

FROM rust:1-alpine AS build
RUN apk add --no-cache build-base
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets ./assets

# `release-dev` is `release` with thin LTO and parallel codegen: the same optimised
# binary for practical purposes, and a link step that no longer dominates the build
# on the NAS. The cached target directory survives between builds, so a change to a
# source file recompiles this crate only instead of every dependency. Pass
# `--build-arg PROFILE=release` to produce the artifact that gets published.
ARG PROFILE=release-dev
RUN --mount=type=cache,target=/src/target \
    cargo build --profile "$PROFILE" --locked && \
    cp "target/$PROFILE/network-monitor" /usr/local/bin/network-monitor

FROM scratch
COPY --from=build /usr/local/bin/network-monitor /network-monitor

# Default configuration: a bare `docker run` starts and probes the container's own
# loopback, which proves the raw socket and the database work. Mount a file over this
# path to monitor real targets, or bake your own file into the image — it travels with
# the image, so no mount is needed wherever the image is loaded:
#
#   docker build --build-arg CONFIG=network-monitor.toml -t network-monitor:dev .
#
# The file must be inside the build context, and its `database` must point at the
# mounted volume (`/var/lib/network-monitor/events.db`), otherwise the history stays in
# the container layer.
ARG CONFIG=docker/config.toml
COPY ${CONFIG} /etc/network-monitor/config.toml

# Raw ICMP sockets need NET_RAW and nothing else. Run the container as the default
# user while dropping every other capability:
#
#   docker run --cap-drop ALL --cap-add NET_RAW \
#     -v "$PWD/network-monitor.toml:/etc/network-monitor/config.toml:ro" \
#     -v network-monitor-data:/var/lib/network-monitor \
#     network-monitor
#
# Add --network host when the configuration binds targets to physical interfaces
# (`interface = "eth1"`): a bridge container only sees its own eth0, not the host
# interfaces. docker-compose.yml already does this.
#
# The configuration must point `database` at the mounted volume, for example
#   database = "/var/lib/network-monitor/events.db"
#
# Passing a command replaces CMD, so the default --config disappears with it: write
#   docker run ... network-monitor:dev monitor --config /etc/network-monitor/config.toml
# when the monitor subcommand is given explicitly.
ENTRYPOINT ["/network-monitor"]
CMD ["monitor", "--config", "/etc/network-monitor/config.toml"]