# Multi-stage build: the release binary is statically linked (musl, SQLite bundled),
# so the runtime stage needs nothing but the executable.

FROM rust:1-alpine AS build
RUN apk add --no-cache build-base
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM scratch
COPY --from=build /src/target/release/network-monitor /network-monitor

# Default configuration: a bare `docker run` starts and probes the container's own
# loopback, which proves the raw socket and the database work. Mount a file over this
# path to monitor real targets.
COPY docker/config.toml /etc/network-monitor/config.toml

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