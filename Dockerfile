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

# Raw ICMP sockets need NET_RAW and nothing else. Run the container as the default
# user while dropping every other capability:
#
#   docker run --cap-drop ALL --cap-add NET_RAW \
#     -v "$PWD/network-monitor.toml:/etc/network-monitor/config.toml:ro" \
#     -v network-monitor-data:/var/lib/network-monitor \
#     network-monitor
#
# The configuration must point `database` at the mounted volume, for example
#   database = "/var/lib/network-monitor/events.db"
ENTRYPOINT ["/network-monitor"]
CMD ["monitor", "--config", "/etc/network-monitor/config.toml"]
