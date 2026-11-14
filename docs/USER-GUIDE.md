# User Guide — network-monitor

The manual for the person who installs, configures, and operates `network-monitor`: what a
target is, how to write the configuration for the four check methods, how to run the tool and
read its page, how to back up and upgrade, and what to do when something does not work. Every
term is used in one precise sense, and every rule this guide relies on is restated where it is
needed, so the manual can be read and distributed on its own.

## Contents

1. [What the tool does](#what-the-tool-does)
2. [Install](#install)
3. [Quick start](#quick-start)
4. [Configuration](#configuration)
5. [Command line](#command-line)
6. [The web page](#the-web-page)
7. [Operations](#operations)
8. [Troubleshooting](#troubleshooting)
9. [Limits and deliberate choices](#limits-and-deliberate-choices)

## What the tool does

`network-monitor` probes each target on an interval and records in SQLite only when the answer
changes, so history grows with incidents, not probes. A target is one check — ICMP, TCP, DNS,
or HTTP — with its own key; it is `unknown`, `up`, or `down`, and a local failure changes nothing.

## Install

### Requirements

| Method | Needs |
|---|---|
| `icmp` | `root`, `CAP_NET_RAW`, or Administrator: raw ICMP sockets |
| `tcp`, `dns`, `http` | nothing beyond an ordinary unprivileged process |
| any method with `interface` | the capability to bind a socket to a device (`icmp` and `tcp` only) |

The tool is one executable with no runtime dependency. HTTPS verification uses certificate
roots compiled into the binary, so no CA package is required and the system trust store is not
consulted.

### From source

```bash
cargo build --release
sudo ./target/release/network-monitor monitor --config ./network-monitor.toml
```

A static Linux binary is produced by
`cargo build --release --target x86_64-unknown-linux-musl`; the container image in this guide
carries that same binary, so no shared library has to be present on the appliance.

### Container

```bash
docker run -d --name network-monitor \
  --cap-drop ALL --cap-add NET_RAW --network host \
  -v network-monitor-data:/var/lib/network-monitor \
  -v "$PWD/network-monitor.toml:/etc/network-monitor/config.toml:ro" \
  network-monitor:dev \
  monitor --config /etc/network-monitor/config.toml \
  --listen 0.0.0.0:8642
```

- `--cap-add NET_RAW` is needed for `icmp` targets only; a container monitoring `tcp`, `dns`,
  and `http` runs with `--cap-drop ALL` and nothing added.
- `--network host` is what makes `interface = "eth1"` mean the physical interface of the host,
  and what makes the page reachable on the host address without publishing a port. Host
  networking is Linux only.
- The configuration may also be baked into the image, with
  `docker build --build-arg CONFIG=<file> -t network-monitor:dev .`, or carried by a compose
  project in the `content` of a `configs` entry. Container Station on a QNAP launches the same
  image, so the flags above — host networking, the volume, the capabilities — are all it needs.

## Quick start

```toml
# network-monitor.toml
interval_ms = 5000
timeout_ms = 2000
database = "/var/lib/network-monitor/events.db"

[[targets]]
method = "icmp"
ip = "1.1.1.1"
interface = "eth1"
label = "uplink"

[[targets]]
method = "tcp"
host = "1.1.1.1"
port = 443
label = "cloudflare-https"

[[targets]]
method = "dns"
name = "example.com"
resolver = "9.9.9.9"
label = "example-via-quad9"

[[targets]]
method = "http"
url = "https://example.com/health"
expect_status = ["200", "3xx"]
label = "example-health"
```

```bash
sudo network-monitor monitor --config ./network-monitor.toml --listen 0.0.0.0:8642
network-monitor query state --config ./network-monitor.toml     # another shell
```

Open `http://<host>:8642/` for the page. Every option above can also be given on the command
line, and `--show-config` prints the result of merging the two.

## Configuration

### Layers

`command line > file > built-in default`. `--config <path>` must exist; without the option
`./network-monitor.toml` is read when present and skipped otherwise. A changed file is re-read
within one second while the monitor runs and applied without a restart; a file that does not
parse is rejected with a warning and the running configuration stays in effect. A changed
`database` path is reported and used from the next start.

### Global keys

| Key | Default | Meaning |
|---|---|---|
| `interval_ms` | `5000` | Pause after a probe, per target |
| `timeout_ms` | `2000` | Maximum wait for an answer, per probe |
| `database` | `./network-monitor.db` | SQLite file; keep it on a local filesystem |
| `retention_days` | `0` | Delete events older than this; `0` keeps everything |

### Common target keys

| Key | Method | Meaning |
|---|---|---|
| `method` | all | `icmp` (default when omitted), `tcp`, `dns`, `http` |
| `label` | all | Name in logs and on the page; the key is shown when absent |
| `timeout_ms` | all | Overrides the global timeout for this target |
| `interval_ms` | all | Overrides the global interval for this target |
| `interface` | `icmp`, `tcp` | Bind the probe to a device; refused on the other methods |

A target states only the keys of its method; a key that belongs to another method is a
configuration error, not something ignored.

### ICMP

```toml
[[targets]]
method = "icmp"
ip = "1.1.1.1"          # IPv4 address, required
interface = "eth1"      # optional, like ping -I eth1 1.1.1.1
label = "uplink"
```

`up` when a matching echo reply arrives within the timeout. The probe is one request, so a
target answers every `rtt + interval` and a silent one every `timeout_ms + interval`.

### TCP

```toml
[[targets]]
method = "tcp"
host = "1.1.1.1"        # address or name, required
port = 443              # 1-65535, required
interface = "eth1"      # optional
label = "cloudflare-https"
```

`up` when the handshake completes; nothing is sent and the connection is closed at once. A
refusal, a reset, and a timeout are `down`; a name that does not resolve is a local failure.

### DNS

```toml
[[targets]]
method = "dns"
name = "example.com"    # required
resolver = "9.9.9.9"    # optional: the server to ask, port 53
record = "A"            # optional: A (default) or AAAA
label = "example-via-quad9"
```

With a `resolver` the tool sends one question to that server:

| Answer | State |
|---|---|
| A record of the requested type | `up` |
| `NXDOMAIN`, or no record of that type | `down` |
| `SERVFAIL`, `REFUSED`, `NOTIMP`, truncated or unusable answer | local failure, no state change |
| No answer within the timeout | `down` |

Without `resolver` the system resolver is asked, which also answers from the local hosts file
and mDNS. There, *any* failure — a missing name as much as an unreachable resolver — is
`down`, because the platform does not expose the difference. When the distinction matters, name
the server.

### HTTP and HTTPS

```toml
[[targets]]
method = "http"
url = "https://example.com/health"   # required, http:// or https://
expect_status = ["200", "3xx"]       # optional, default 200-399
label = "example-health"
```

One `GET` with `Connection: close`, no body, no redirect following, no authentication. A status
inside the expectation is `up`, a status outside it is `down`, and a refusal, a timeout, or a
TLS failure is `down`. `expect_status` accepts classes (`2xx`) and exact codes (`204`). A
malformed answer, or a response head larger than 8 KiB, is a local failure.

HTTPS verifies the certificate chain against the roots compiled into the binary; verification
cannot be disabled. A proxy that re-signs traffic requires a build with its root.

### Identity and duplicates

A target's key is its method plus what identifies it: `icmp://1.1.1.1`, `tcp://1.1.1.1:443`,
`dns://example.com@9.9.9.9`, `https://example.com/health`. Two entries with the same key are a
configuration error; the same host checked by two methods, or on two ports, is two targets.
A URL with the default port states it once — `https://example.com:443/` is the same key as
`https://example.com/`.

## Command line

| Flag | Meaning |
|---|---|
| `--config <path>` | Configuration file |
| `--interval-ms <n>` | Probe interval, overrides the file |
| `--timeout-ms <n>` | Probe timeout, overrides the file |
| `--database <path>` | Event database |
| `--retention-days <n>` | Retention window |
| `--target <spec>` | Target list, replaces the file list; repeatable |
| `--log-level <level>` | `error`, `warn`, `info`, `debug` |
| `--compression <mode>` | `auto` (default) gzips the HTML pages when the browser accepts it; `off` sends them plain, for a proxy that compresses |
| `--show-config` | Print the effective configuration and exit |

`--target` takes `[method:]address[,key=value]…`:

```bash
--target 1.1.1.1,interface=eth1,label=uplink
--target tcp:1.1.1.1:443,label=cloudflare-https
--target dns:example.com@9.9.9.9,record=AAAA
--target http:https://example.com/health,expect=204,timeout_ms=1500
```

### Subcommands

| Command | Does |
|---|---|
| `monitor [--listen <addr>]` | Probe the targets and record transitions; optionally serve the page |
| `serve [--listen <addr>]` | Serve the page alone, read-only, without probing (default `127.0.0.1:8642`) |
| `query state [--format table\|json]` | Current state of every target |
| `query events [target] [--since 24h] [--limit 100] [--format json]` | Recorded transitions, newest first |
| `import <path\|http://url\|-> [--replace-window]` | Restore a history exported from the page |

`query events <target>` accepts the label, the full key, or a bare ICMP address. Diagnostics go
to stderr as `<timestamp> <LEVEL> [<target>] <message>`; data goes to stdout, so a query can be
piped.

## The web page

`monitor --listen` or `serve --listen` publishes one read-only page:

- `/` — one lane per target over the selected window (1 h by default; 1, 2, 4, 6, 12, 24 h).
  A lane states the drops, the average `up` run with its spread, the longest run, and how long
  the target has been up without a drop. Below the chart, the last 10 disconnections, each named as its lane is: the label when the target has one, in which case the untitled column beside it repeats the key, and the key alone otherwise.
- `/target?target=<key>&days=30d` — one target for 30, 60, 90, 180, or 360 days: a bar per day,
  the last 10 disconnections of that target in the same table as the timeline page, and a per-day
  table.
- `Download History` — the whole history as a gzipped CSV.
- `/healthz` — `ok` while the process runs.

The pages are gzipped when the browser asks for it — it always does, and the saving is about
80 % — and sent plain otherwise, or always plain with `--compression off` when a proxy in front
does the compression. The archive is a gzip file and is never compressed twice.

The page reloads itself every 30 s and formats the instants in your browser's time zone. It has
**no authentication**: anyone who can reach the port reads the history. Keep the default bind
(`127.0.0.1`) or firewall the port.

The CSV columns are `target,label,interface,state,occurred_at,rtt_us`; `target` is the key.
The same file is what `import` reads back.

## Operations

### Back up and restore

```bash
sqlite3 network-monitor.db ".backup 'backup.db'"      # while it runs
network-monitor query events --format json > dump.json
curl -sO http://nas:8642/export/disconnections.csv.gz  # snapshot from the page
```

Restoring: stop the process, put the database back, start it. Restoring from the CSV:

```bash
network-monitor import disconnections-20261109.csv.gz --database ./events.db
```

Import merges by default; `--replace-window` replaces what the archive covers and keeps the
history before and after it. Do not point an import at the live database without a backup. The
import reads plain `http://` URLs; download an `https` archive with `curl` first.

### Upgrade

1. Export or back up the database.
2. Replace the binary or the image and restart.
3. The schema is migrated on open. **From 0.1 to 0.2** the identity of every target changes to
   the `method` form: the first writable open rewrites bare addresses to `icmp://…` once. An
   older binary started against that database afterwards keys its ICMP targets again by bare
   address, so the same target can appear twice — keep the backup from step 1 until the
   upgrade is confirmed.

### Retention, logs, signals

- `retention_days > 0` deletes older events periodically; `0` keeps everything.
- `--log-level debug` logs every verdict; `info` logs every transition (the default), `warn`
  local probe failures, `error` fatal conditions.
- `SIGINT`/`SIGTERM` stop the process between two probe cycles, with no partial write.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `interface 'eth1' ... does not exist` | The name differs on this host (`en0`, `eth0`, `lo0`). The process refuses to start rather than probe the wrong interface. |
| `Operation not permitted` at startup | ICMP needs `root` or `CAP_NET_RAW`. Drop `icmp` targets, or grant the capability. |
| `no targets configured` | Add `[[targets]]` or `--target`; an empty list is refused. |
| `invalid configuration: targets[0].ip: does not apply to the tcp method` | A key of another method. Each target lists only the keys of its own method. |
| `duplicate target tcp://1.1.1.1:443` | The same key twice. Two methods or two ports are two targets. |
| `the event database ... does not exist` on the page | `serve` reads a database that must exist; start `monitor` once, or point `--database` at the right file. |
| `Is a directory` on start | A bind mount whose source file did not exist: create the file before `docker compose up`. |
| `database error: attempt to write a readonly database` | The database on the volume belongs to another user — typically one copied in from another machine — and `cap_drop: ALL` also drops `CAP_DAC_OVERRIDE`, so root inside the container cannot write it: `docker run --rm -v network-monitor-data:/data alpine chown -R 0:0 /data`. |
| A `dns` target without `resolver` flaps | The system resolver is unreachable and the platform reports it as a missing name. Use `dns://name@server`. |
| An `http` target is always `down` while `curl` works | The status is outside `expect_status` (a 3xx below the default? a 401?), or a TLS-inspecting proxy is at work and its root is not in the binary. |
| The page shows two lanes for one host | The database carries both a bare address and its `icmp://` key (an older binary wrote after the rewrite). Import a backup or merge the rows. |

## Limits and deliberate choices

- IPv4 only; no IPv6 targets.
- HTTP checks do not follow redirects, send no body, and cannot present credentials or a client
  certificate.
- DNS supports `A` and `AAAA` over UDP on port 53, no DNSSEC, no DoT/DoH.
- `interface` is supported on `icmp` and `tcp` only.
- The page has no authentication and no write path.
- The trust store is the one compiled into the binary.

Each item is deliberate rather than missing: leaving out IPv6, redirects, DNSSEC, DoT/DoH and
client certificates keeps one probe cheap and the binary portable, the trust store travels
inside the binary so an appliance needs no CA package, and the page is read-only so the monitor
holds no credentials.
