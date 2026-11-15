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
7. [REST API](#rest-api)
8. [Operations](#operations)
9. [Troubleshooting](#troubleshooting)
10. [Limits and deliberate choices](#limits-and-deliberate-choices)

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

### Telegram notifications

The tool can post one message per incident to a Telegram chat. The feature is opt-in: without
the section nothing is sent and the process opens no connection other than its probes.

`[telegram]` is a single global section of the file, and it is the one documented exception to
the rule that every key has a command-line flag: the bot token is a secret, so it is read from
the file only.

| Key | Default | Meaning |
|---|---|---|
| `bot_token` | — | Required. The token Telegram issued for the bot |
| `chat_id` | — | Required. The chat or channel the messages go to, e.g. `-1001234567890` |
| `stabilize_seconds` | `60` | How long `up` must hold before a recovery is reported; `0` reports it at once |

```toml
[telegram]
bot_token = "123456789:replace-me"
chat_id = "-1001234567890"
stabilize_seconds = 60
```

Setting it up, from an empty Telegram account:

1. Talk to **@BotFather**, send `/newbot`, choose a name and a user name, and copy the token it
   answers with. Treat that token like a password.
2. Add the bot to the channel or group that should receive the messages, and make it an
   **administrator**: a bot cannot post to a channel otherwise.
3. Find the `chat_id`: post a message in the channel and open
   `https://api.telegram.org/bot<token>/getUpdates` in a browser. For a channel post the id is
   in `channel_post.chat.id` and starts with `-100`; a private chat has a positive id. An empty
   answer means the bot has not seen a message yet.
4. Write the section into the configuration file, `chmod 600` it, and restart the monitor.
5. Check the channel: the first probe of every target reports its observed state, so a target
   that answers produces one `UP` message, held by the stabilisation window.

What is sent, and when:

- The monitor starts: one `READY` message as soon as every target is being probed, stating the
  configuration file it read, the keys of the targets it started, and the address of the page
  when it serves one. A restart sends it again.
- A target stops answering: one `DOWN` message at once, with the cause as a code and a sentence.
- A target answers again: one `UP` message only after `up` has held continuously for
  `stabilize_seconds`. A target that drops again inside that window produces no message at all,
  because the operator was already told it is down.
- After an `UP` message, the next drop is a new `DOWN`. A repeated `DOWN` in the same incident is
  silent, so a flapping link cannot flood the chat even when the interval is a few seconds.
- A restart reports the first observed state under the same rules; the tool keeps no
  notification state between runs.

The message is plain text with a fixed layout — state, label and key, cause, UTC instant:

```text
DOWN cloudflare-dns (icmp://1.1.1.1)
Cause: ICMP.NO_REPLY — no reply within 2000 ms
2025-11-24T16:00:00.000Z
```

The startup message states one fact per line and closes with the instant:

```text
READY network-monitor
Config: /etc/network-monitor/network-monitor-qubo.toml
Targets: 2
- icmp://192.168.1.1
- tcp://1.1.1.1:443
Web: http://0.0.0.0:8642/
2025-11-24T16:00:00.000Z
```

An `UP` message is the first and the last line; a target without a label shows only its key;
only a `DOWN` carries a cause.

A failed delivery is attempted five times, waiting 1, 2, 4, and 8 seconds, and then dropped
with one `WARN` line naming the reason. A `400` or `401` answer is not retried, because the
token or the chat cannot be fixed by waiting. Notification never delays a probe and never changes
what is recorded: the database remains the history, the channel is a copy of it.

A changed `[telegram]` section is reported on stderr and applied at the next start, exactly like
a changed `database` path. The bot token never appears in `--show-config` (which prints
`<redacted>`), in a log line, or in an error message.

The failure modes of the Bot API — the answer Telegram returns, and whether it is retried —
are listed in [Troubleshooting](#troubleshooting), later in this guide.

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
| `--compression <mode>` | `auto` (default) gzips the HTML pages and the JSON answers when the client accepts it; `off` sends them plain, for a proxy that compresses |
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

`monitor --listen` or `serve --listen` publishes one read-only page. Every page opens with the
same masthead — the mark, the name of the application, and its tagline — flush left, at the
same height, so nothing moves when you go from one page to the other. A page may state what it
is showing at the right end of the masthead, on the same centre line as that block; the
toolbar under it carries the control that changes what is shown.

- `/` — one lane per target over the selected window (1 h by default; 1, 2, 4, 6, 12, 24 h).
  A lane is drawn as one bar: only its two outer ends are rounded, so the bands of the states
  meet square. A fault too short to reach a visible width is drawn at a minimum width and
  coloured **orange instead of red**, so a brief outage is not lost in a wide window and its
  width is never mistaken for its duration; hover it for the exact instants.
  A lane states the drops, the average `up` run with its spread, the longest run, and how long
  the target has been up without a drop. Below the chart opens a table of the last 10 faults,
  under a rule that marks where it begins: a row names the target as its lane does — the label,
  with the key in smaller type under it, or the key alone when the target has no label — and
  states the start, the cause of the fault, and — in the last column — the duration, with
  "and counting" on a line of its own while the target is still down. The four columns keep a
  stated share of the width (15, 30, 45 and 10 per cent) and every row is the same height.
- `/target?target=<key>&days=30d` — one target for 30, 60, 90, 180, or 360 days: a bar per day,
  the last 10 faults of that target in the same table as the timeline page, and a per-day table.
  The masthead states the target at its right end: its label and address, then its status
  (up/down + duration). A back button in the toolbar always returns to the list page,
  whatever page you came from.
- `Download History` — the whole history as a gzipped CSV, from `/export/faults.csv.gz`, named
  `faults-YYYYMMDD.csv.gz`. On a target page the same link downloads **only that target's**
  history (`/export/faults.csv.gz?target=<key>`) and the file is named after it —
  `faults-<target>-YYYYMMDD.csv.gz`, using the name the page shows. The address the page used
  before this version, `/export/disconnections.csv.gz`, still answers and redirects to the
  whole history.
- `/healthz` — `ok` while the process runs.

**Selecting a fault opens its cause underneath the row** — one fault at a time. The message is
the standard sentence of that failure, with the details of the occurrence in it, for example
"the connection to 9.9.9.9:443 was refused". A browser without JavaScript shows the same message
on hover. A row recorded before this version states no cause and opens nothing.

The causes an operator will meet, by the layer that reported them:

| Code | What happened |
|---|---|
| `ICMP.NO_REPLY` | No echo reply before the timeout |
| `TCP.REFUSED` | The port answered with a refusal: nothing listens there |
| `TCP.RESET` | The connection was closed by the peer |
| `TCP.TIMEOUT` | No answer before the timeout: the packet is likely filtered |
| `DNS.NXDOMAIN` | The server answered that the name does not exist |
| `DNS.NO_RECORD` | The name exists without a record of the requested type |
| `DNS.TIMEOUT` | The server did not answer before the timeout |
| `DNS.UNRESOLVED` | The system resolver could not resolve the name |
| `HTTP.UNEXPECTED_STATUS` | The endpoint answered a status outside the expectation |
| `HTTP.TLS` | The certificate handshake failed |

A local failure — the machine could not perform the check — never reaches the page: it has a code
of its own in the log (`TCP.RESOLVE`, `TCP.INTERFACE`, `HTTP.MALFORMED`, `PROBE.WRONG_METHOD`, …),
and the log line reads `probe failed: CODE — message`, so a code can be searched for.

The pages are gzipped when the browser asks for it — it always does, and the saving is about
80 % — and sent plain otherwise, or always plain with `--compression off` when a proxy in front
does the compression. The archive is a gzip file and is never compressed twice.

The page reloads itself every 30 s and formats the instants in your browser's time zone. It has
**no authentication**: anyone who can reach the port reads the history. Keep the default bind
(`127.0.0.1`) or firewall the port.

The CSV columns are `target,label,interface,state,occurred_at,rtt_us,cause,cause_message`;
`target` is the key, and the last two state the code and the message of the cause, empty on a
return to `up`. The same file is what `import` reads back, and an archive written before the
cause columns existed — six columns, ending after `rtt_us` — imports with no cause.

## REST API

The same server publishes the history as JSON under `/api/v1`: the targets, the faults, and
the per-day totals. It is read-only, needs no authentication — anyone who can open the page can
read it — and it answers on the address given to `--listen`. The contract is served by the
binary itself: `GET /api/v1/openapi.json` is an OpenAPI 3.1 document naming every route,
parameter, and property.

Open the reference in a browser at `http://<host>:8642/api/v1/docs`: it renders that same
document, route by route and schema by schema, and its **Try it out** button sends a `GET` to
the route from the page. The renderer ships inside the executable, so the reference works on a
host with no internet access, and because it reads the document the binary serves, it cannot
disagree with it.

| Route | What it returns |
|---|---|
| `GET /api/v1/targets` | Every target, ordered by key |
| `GET /api/v1/faults` | The faults, newest first |
| `GET /api/v1/faults/daily` | Faults, down time, and `up` runs per UTC day, oldest first |
| `GET /api/v1/openapi.json` | The OpenAPI document of the routes above |
| `GET /api/v1/docs` | The routes above, rendered as a reference in the browser |

| Parameter | Routes | Accepted | Default |
|---|---|---|---|
| `target` | faults, daily | canonical key (`icmp://1.1.1.1`) or label | every target |
| `from` | faults, daily | RFC 3339 UTC (`2026-11-14T00:00:00Z`) or epoch milliseconds, inclusive, inside the calendar domain below | none on faults; 30 days before `to` on daily |
| `to` | faults, daily | the same two forms, exclusive, inside the same domain | none on faults; now on daily |
| `limit` | all | `1` to `1000` | `100` |
| `offset` | all | `0` or more | `0` |

An instant lies between `1970-01-01T00:00:00Z` and `2100-01-01T00:00:00Z`, both bounds
accepted, in either form; a value outside is `400 INVALID_PARAMETER`, naming the parameter and
both bounds. `to` may be in the future — that is how a whole month or year is asked for:
`from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z` is November 2026 on any day of November, and
the days after today state `0` faults, `0` down time and no `up` run, because the time that has
not happened yet is not measured.

A parameter the route does not accept, a parameter given twice, a value out of range, and a
`from` that is not earlier than `to` answer `400` and name the parameter, so a typo is never read
as a default. An empty `target=` is the same as naming none.

Every answer is an object, and the records sit in `data`:

```json
{
  "data": [
    {
      "id": 42,
      "target": "icmp://1.1.1.1",
      "label": "cloudflare-dns",
      "interface": "eth1",
      "occurredAt": "2026-11-14T20:15:00.000Z",
      "occurredAtMs": 1763151300000,
      "cause": "ICMP.NO_REPLY",
      "causeMessage": "no reply within 2000 ms",
      "endedAt": "2026-11-14T20:17:30.000Z",
      "endedAtMs": 1763151450000,
      "durationMs": 150000,
      "ongoing": false
    }
  ],
  "meta": {
    "limit": 100,
    "offset": 0,
    "total": 1,
    "filters": {"target": "icmp://1.1.1.1", "from": null, "to": null},
    "generatedAt": "2026-11-14T22:00:00.000Z"
  }
}
```

`meta.total` is how many records match the filters, whatever the page holds, so a client pages
by adding `limit` to `offset` until it reaches the total.

`/api/v1/faults/daily` covers the last 30 days unless `from` names an earlier instant, so a
longer history is asked for explicitly; every record states its `target`, or `null` when the
answer aggregates every target into one series. The window may be any width the domain allows:
only the days the page returns are read, so the cost follows `limit`, not the span. A day at
either edge of the window counts only the part that falls inside it — a fault that opened before `from` is down time
inside the window but not a fault of it — and the UTC day still running states its average and
longest `up` run over the last 24 hours.

The data routes stream their answer: the response is `Transfer-Encoding: chunked`, carries no
`Content-Length`, and is compressed on the fly when the client advertises `gzip`. A client reads
the document to the end rather than waiting for a length, and `HEAD` states the same headers and
no body. `/api/v1/faults` reads the history in pages of 200 faults, so a `limit` of 1000 is one
answer of ten short reads rather than a thousand rows held in memory. Paging a list that grows at the top —
faults always do — can repeat a record between two requests; use `from` and `to` for a window
that does not move.

An error states the status, a stable code, the offending parameter when there is one, and one
sentence:

```json
{
  "error": {
    "status": 400,
    "code": "INVALID_PARAMETER",
    "parameter": "limit",
    "message": "limit must be an integer between 1 and 1000"
  },
  "meta": {"generatedAt": "2026-11-14T22:00:00.000Z"}
}
```

The message is a fixed sentence: the detail of a failure — a database path, a SQLite error —
stays in the log of the process and never reaches a reader. The codes are
`INVALID_PARAMETER`, `INVALID_RANGE`, `UNKNOWN_TARGET`, `NOT_FOUND`,
`METHOD_NOT_ALLOWED`, `DATABASE_UNAVAILABLE` — the `503` the page also states, for a database
that is missing or was never opened by this tool — and `QUERY_FAILED`.

A failure inside a request costs that request only: the connection closes without a status
line, the reason is logged by the process, and the next request is served normally. A request
that fails never takes capacity away from the ones that follow.

```bash
curl -s '127.0.0.1:8642/api/v1/targets'
curl -s '127.0.0.1:8642/api/v1/faults?target=cloudflare-dns&from=2026-11-01T00:00:00Z&limit=50'
curl -s '127.0.0.1:8642/api/v1/faults/daily?to=2026-11-15T00:00:00Z'
curl -s '127.0.0.1:8642/api/v1/faults/daily?target=icmp://1.1.1.1&from=2026-07-01T00:00:00Z'
```

The answers are gzipped when the client advertises `gzip` and `--compression` is `auto`, state
`Cache-Control: no-store`, and carry `Access-Control-Allow-Origin: *` with an `OPTIONS`
preflight, so a dashboard on another origin reads the same public history.

## Operations

### Back up and restore

```bash
sqlite3 network-monitor.db ".backup 'backup.db'"      # while it runs
network-monitor query events --format json > dump.json
curl -sO http://nas:8642/export/faults.csv.gz   # snapshot from the page
```

Restoring: stop the process, put the database back, start it. Restoring from the CSV:

```bash
network-monitor import faults-20261109.csv.gz --database ./events.db
```

Import merges by default; `--replace-window` replaces what the archive covers and keeps the
history before and after it. Do not point an import at the live database without a backup. The
import reads plain `http://` URLs; download an `https` archive with `curl` first.

### Upgrade

1. Export or back up the database.
2. Replace the binary or the image and restart.
3. The schema is migrated by the first writable open, `monitor` or `import`. **From 0.1 to
   0.2** the identity of every target changes to the `method` form: that open also rewrites
   bare addresses to `icmp://…` once. An older binary started against that database afterwards
   keys its ICMP targets again by bare address, so the same target can appear twice — keep the
   backup from step 1 until the upgrade is confirmed.
4. **From 0.2 to 0.3** the schema becomes version 2: the first `monitor` or `import` adds the
   two cause columns and keeps every event, whose cause reads empty. A 0.2 binary then refuses
   that database (it states the schema is newer) instead of misreading it, so upgrade every
   process that opens the same file. The export moves to `/export/faults.csv.gz` and the file is
   named `faults-YYYYMMDD.csv.gz`; the old address still answers. An archive exported by 0.2
   imports into 0.3 unchanged.

`serve` and `query` never write, so they open a 0.2 database as it is: its rows state no cause
until a `monitor` or an `import` has upgraded the file. That also means the page runs against a
snapshot, a backup, or a volume mounted read-only.

### Retention, logs, signals

- `retention_days > 0` deletes older events periodically; `0` keeps everything.
- `--log-level debug` logs every verdict; `info` logs every transition (the default), `warn`
  local probe failures, `error` fatal conditions.
- A start writes, at `info`, the lines that state what the process does: `initializing`, the
  configuration file it read (or that it read none), whether Telegram is configured, one
  `initialized target <key>` line per target, and — when the page is served —
  `web interface ready on http://<address>/` with the address it listens on.
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
| The Telegram channel stays silent | Check, in this order: the section is present and parses (`--show-config` prints it with `<redacted>`); the bot is an **administrator** of the channel, not merely a member; `chat_id` copies `channel_post.chat.id` and starts with `-100`; the log shows no `WARN notification attempt …` line. A `400 Bad Request: chat not found` is a wrong `chat_id`; `401 Unauthorized` a revoked token; `429` too many messages at once. |
| A `WARN notification attempt 1/5 failed` line repeats five times | The channel is unreachable or rejecting the call. The reason on the line names the answer of the Bot API without the token; `--log-level debug` adds nothing, because a failed send is always a warning. The event history is unaffected — read it with `query events`. |
| No `UP` message arrives after an outage while the page shows the target up | The recovery is still inside the stable `up` window (`stabilize_seconds`, 60 s by default). Wait for the window to close; a drop inside it cancels the message by design. |

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
