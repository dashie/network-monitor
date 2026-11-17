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

Two files live beside the configuration and are not configuration layers:

- `<main>.<UTC timestamp>.bak` is a **deployment backup**. A promotion of `--try` writes one,
  and the file holds the configuration that was replaced.
- `<main>.active` is the **active snapshot**: the exact bytes of the configuration the last
  start began from. It is written once after the first probe round of a start, never by an
  edit or a reload, and it is what "the last configuration that worked" means on the host.

Both carry the same secrets as the file they copy; both are created with mode `0600` on Unix,
and `.gitignore` excludes `*.active` and `*.bak` so a working copy cannot commit them.

### Global keys

| Key | Default | Meaning |
|---|---|---|
| `interval_ms` | `5000` | Pause after a probe, per target |
| `timeout_ms` | `2000` | Maximum wait for an answer, per probe |
| `database` | `./network-monitor.db` | SQLite file; keep it on a local filesystem. Ignored by `simulate`, which reads and writes no database |
| `retention_days` | `0` | Delete events older than this; `0` keeps everything |
| `notify` | empty | Channel names enabled for global events and as the default for every target; see [Notifications](#notifications) |

### Common target keys

| Key | Method | Meaning |
|---|---|---|
| `method` | all | `icmp` (default when omitted), `tcp`, `dns`, `http` |
| `label` | all | Name in logs and on the page; the key is shown when absent |
| `priority` | all | Where the lane sits on the page: lowest first, absent = after the others |
| `timeout_ms` | all | Overrides the global timeout for this target |
| `interval_ms` | all | Overrides the global interval for this target |
| `interface` | `icmp`, `tcp` | Bind the probe to a device; refused on the other methods |
| `notify` | all | Declared channel names for this target: replaces the root list, `[]` silences it; see [Notifications](#notifications) |

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

### Notifications

A notification channel tells the operator about an incident without opening the page. A channel
is declared once, at the top level of the file, in a `[[notification]]` entry and enabled by
name, globally or per target.
Three types exist: Telegram, a webhook, and a log file. The feature is opt-in: with no
`[[notification]]` entry nothing is sent and the process opens no connection other than its
probes.

`name` and `type` are required, a name is unique, and a channel has no scope of its own.

| Common key | Default | Meaning |
|---|---|---|
| `name` | — | Required. Unique, and only ASCII letters, digits, `.`, `_` and `-`; the only way a channel is enabled anywhere |
| `type` | — | Required: `telegram`, `webhook` or `logfile` |
| `stabilize_seconds` | `60` | How long `up` must hold before a recovery is reported; `0` reports it at once |

#### Telegram

| Key | Default | Meaning |
|---|---|---|
| `bot_token` | — | Required. The token BotFather issued for the bot |
| `chat_id` | — | Required. The chat or channel the messages go to, e.g. `-1001234567890` |

```toml
[[notification]]
name = "telegram"
type = "telegram"
bot_token = "123456789:replace-me"
chat_id = "-1001234567890"
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
4. Write the declaration and the root `notify` list into the configuration file, `chmod 600` it,
   and restart the monitor.
5. Check the channel: the first probe of every target reports its observed state, so a target
   that answers produces one `UP` message, held by the stabilisation window.

#### Webhook

| Key | Default | Meaning |
|---|---|---|
| `url` | — | Required. `http://` or `https://`; the path and the query are part of the destination |
| `token` | — | Optional bearer credential; requires an `https` URL so it is not sent in clear |

```toml
[[notification]]
name = "hook"
type = "webhook"
url = "https://automation.example.test/hooks/network-monitor"
token = "replace-me"
```

Every event is one `POST` with `Content-Type: application/json; charset=utf-8`; when a token is
configured it travels as `Authorization: Bearer <token>`. A `2xx` answer is success; `429`, any
`5xx`, and a transport failure are retried, five attempts in total; a redirect is not followed
and any other `4xx` is not retried. The body is one JSON document:

```json
{
  "schema": 1,
  "event": "target_down",
  "occurred_at": "2025-11-24T16:00:00.000Z",
  "target": { "key": "icmp://1.1.1.1", "label": "cloudflare-dns" },
  "state": "down",
  "cause": { "code": "ICMP.NO_REPLY", "message": "no reply within 2000 ms" }
}
```

`target`, `state` and `cause` appear only on a target event, and `label` only when the target
has one. `event` is `target_down`, `target_up`, `startup`, `config_reload`, `channel_started`
or `shutdown`. A global event adds one member of its own and nothing else: `startup` states
the configuration, the target keys and the page; `reload` states whether it was applied and
why; `channel` states the name and the type of the channel that started. A `shutdown` adds
nothing.

The token is a secret: it is read from the file only, redacted by `--show-config` and in every
error, and refused on an `http` URL. The URL path may be a secret too, so a log line and an
error name only `scheme://host[:port]`.

#### Log file

| Key | Default | Meaning |
|---|---|---|
| `filename` | — | Required. Append-only; created with mode `0600` on Unix; its directory must exist |
| `logger` | `notification` | The logger name on every line, or the `logger` member in JSONL; the same characters as a `name` |
| `level` | `info` | The most verbose level written: `error`, `warn`, `info`, `debug` |
| `format` | `text` | `text` (the diagnostic line) or `jsonl` (one JSON document per line) |

```toml
[[notification]]
name = "audit"
type = "logfile"
filename = "/var/log/network-monitor/audit.log"
logger = "notification"
level = "info"
format = "jsonl"
```

A `text` line is exactly a diagnostic line, so the same file can be read by the same tools:

```text
2025-11-24T16:00:00.000Z WARN  notification [icmp://1.1.1.1] DOWN cloudflare-dns (icmp://1.1.1.1) cause=ICMP.NO_REPLY: no reply within 2000 ms
```

A `jsonl` line is the webhook document plus `"logger":"notification"`, one compact document per
line. The file is appended to and never rotated by the tool: rotate it with `logrotate` or the
platform's mechanism, and create its directory yourself.

#### Enabling a channel

`[[notification]]` only declares a channel. The root `notify` list enables channels for global
events — startup, shutdown, reload, channel start — and for every target that does not state
its own list. A target's `notify` list replaces the root list for that target; an absent key
inherits; `notify = []` silences the target.

```toml
notify = ["telegram"]            # global events, and the default for every target

[[targets]]
ip = "1.1.1.1"                   # inherits ["telegram"]

[[targets]]
ip = "192.168.1.1"
notify = ["telegram", "audit"]   # this target also writes the audit file

[[targets]]
ip = "10.0.0.1"
notify = []                      # this target notifies nobody
```

A typo in a name, a duplicate name, two channels on one destination, and a `[telegram]` clash all
stop the process before the first probe; `--show-config` prints the declarations and the lists,
which makes the mistake visible. A declared channel that no list enables is not an error: the
process starts, logs one `WARN` line naming it, and the channel receives nothing.

`[telegram]` from 0.7 is still accepted for one release: it declares a channel named `telegram`
and enables it globally. The declarations and the lists are file-only, like the secrets they
carry: there is no flag for them. `--target` replaces the file target list and with it the
per-target lists; the root list still applies to the targets given on the command line, so a
channel that only a file target enables still starts, states its `CHANNEL` line, and receives
no target event.

#### What is sent, and when

- The monitor starts: one `READY` message as soon as every target is being probed, stating the
  configuration file it read, the keys of the targets it started, and the address of the page
  when it serves one.
- A target stops answering: one `DOWN` message at once, with the cause as a code and a sentence.
- A target answers again: one `UP` message only after `up` has held continuously for
  `stabilize_seconds`. A target that drops again inside that window produces no message at all,
  because the operator was already told it is down.
- After an `UP` message, the next drop is a new `DOWN`. A repeated `DOWN` in the same incident is
  silent, so a flapping link cannot flood the channel even when the interval is a few seconds.
- The configuration file is reloaded: one `CONFIG` message, applied or rejected with the reason.
- A channel starts: one `CHANNEL` message on that channel itself, so even a target-only file
  states that it began.
- The monitor stops: one `STOPPED` message to the channels the root list enables, attempted once.
- A restart reports the first observed state under the same rules; the tool keeps no
  notification state between runs.

The Telegram text keeps the 0.7 layout — state, label and key, cause, UTC instant:

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
with one `WARN` line naming the reason. A channel never delays a probe and never changes what is
recorded: the database remains the history, the channels are a copy of it. A channel that cannot
open — an unwritable file, a malformed URL — stops the process at startup.

A changed notification declaration or list is reported on stderr and applied at the next start,
exactly like a changed `database` path. A secret never appears in `--show-config` (which prints
`<redacted>`), in a log line, or in an error message.

The failure modes of each channel — the answer it receives, and whether it is retried — are
listed in [Troubleshooting](#troubleshooting), later in this guide.

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
--target 1.1.1.1,interface=eth1,label=uplink,priority=1
--target tcp:1.1.1.1:443,label=cloudflare-https,priority=2
--target dns:example.com@9.9.9.9,record=AAAA
--target http:https://example.com/health,expect=204,timeout_ms=1500
```

### Subcommands

| Command | Does |
|---|---|
| `monitor [--listen <addr>] [--try <path>] [--fallback-to-active]` | Probe the targets and record transitions; optionally serve the page, promote a candidate, or start from the active snapshot |
| `check [--listen <addr>] [--probe]` | Verify the configuration without starting the monitor; exit non-zero when it would not start |
| `serve [--listen <addr>]` | Serve the page alone, read-only, without probing (default `127.0.0.1:8642`) |
| `simulate --listen <addr> [--history 30d] [--seed 0] [--fault-rate 3]` | Serve the page over a generated history, with no probe and no database |
| `query state [--format table\|json]` | Current state of every target |
| `query events [target] [--since 24h] [--limit 100] [--format json]` | Recorded transitions, newest first |
| `import <path\|http://url\|-> [--replace-window]` | Restore a history exported from the page |

`query events <target>` accepts the label, the full key, or a bare ICMP address. Diagnostics go
to stderr as `<timestamp> <LEVEL> [<target>] <message>`; data goes to stdout, so a query can be
piped.

### Verifying a configuration (`check`)

`check` runs what a start runs, in the order a start runs it, and stops short of probing and
writing:

```bash
network-monitor check --config ./network-monitor.toml --listen 0.0.0.0:8642
```

```
ok    targets            6 target(s)
ok    interface          icmp://1.1.1.1 eth0
warn  database           ./network-monitor.db does not exist yet; the next start creates it
warn  listen             0.0.0.0:8642 is in use — the running monitor?
ok    channel            none declared
warn  active             no snapshot yet
```

| Option | Meaning |
|---|---|
| `--listen <address:port>` | Also verify that this address can be bound. An address in use is a `warn` — the running monitor is the usual cause — and anything else that prevents the bind is a `fail` |
| `--probe` | Perform one real probe per target. `up` is `ok`, `down` is a `warn` with its cause, and a local probe failure (no `NET_RAW`, missing interface, unresolvable resolver) is a `fail` |

Every line is one of `ok`, `warn`, or `fail`. The command exits `0` when no line is a `fail`
and `1` otherwise, and `warn` never changes the exit code. It prints the token of a channel as
`<redacted>`, starts no worker, sends no notification, and opens an existing event database
read-only. The only thing it may create is the file of a `logfile` channel, exactly as a start
would; a missing event database is a `warn`, because the next start creates it.

The checks, in order, are: the target list; every target's interface; the event database; the
`--listen` address when given; every declared notification channel (one read-only Telegram
`getMe`, one connection per webhook, one open and close per log file); the active snapshot; and,
only with `--probe`, one real probe per target.

### Simulated mode

`simulate` serves the same page, the same REST API and the same export over a history that is
generated instead of recorded. It is meant for looking at the page, demonstrating a deployment,
or reviewing a change without waiting days for real data, and it is safe to run anywhere: it
runs no probe, opens no database, and writes no file.

```bash
network-monitor simulate --listen 127.0.0.1:8660 --history 30d --seed 0
```

| Option | Default | Meaning |
|---|---|---|
| `--listen <address:port>` | required | Address and port to serve on; nothing is served without it |
| `--history <days>` | `30d` | How many days of past to present, written `30d` or `30`, from `1` to `3660` |
| `--seed <number>` | `0` | Which history to generate: the same seed always shows the same one, another seed shows another |
| `--fault-rate <faults>` | `3` | How many faults a day a target that fails now and then holds, `0` to `1000`; `0` makes a board where nothing goes down |

The targets are the ones in the configuration (or on `--target`), and a configuration without
targets is refused with the usual message. Everything else the tool knows about them — the
method, the label, the interface, the priority — shapes the generated board: each target gets a
life from the seed, and most targets are up nearly always while some fail a few times a day and
a few flap. The past is whole UTC days that end today; nothing exists before its first day, and
nothing exists after now. A fault may cross midnight and stays one fault.

`--fault-rate` is the knob for how busy the board is. Its number is the faults a day of a target
that fails now and then, and it is measured against that target: one target in five never fails
whatever the number, and one in ten flaps and gets about eight times as many faults, drawn short
so they fit in the day. `--fault-rate 3` (the default) is a board where something happens every
few hours; `0` is a board where nothing ever goes down, useful when you are working on the
layout; `20` is a board in trouble.

Two runs of the same command show the same faults, with the same causes, durations and
latencies, on any machine. `--database` and the `database` key of the file are accepted and
ignored, no notification channel is started, and the startup log states which mode is running:

```
INFO  simulating 7 target(s) over the last 30 day(s), seed 0, 3 fault(s) per day: no probe runs and no database is read or written
```

## The web page

`monitor --listen` or `serve --listen` publishes one read-only page. Every page opens with the
same masthead — the mark, the name of the application, and its tagline — flush left, at the
same height, so nothing moves when you go from one page to the other. That block is a link:
selecting the mark, the name, or the tagline returns to the timeline from any page, the API
reference included. Nothing joins the identity block: the toolbar under it states what the page
is showing, on the left of the line, and carries the control that changes it, on the right.

- `/` — one lane per target over the selected window (1 h by default; 1, 2, 4, 6, 12, 24 h).
  The lanes are ordered by the `priority` the configuration gives a target, lowest number
  first; the targets without one follow them, by key. The same order is the one the JSON API
  lists. A lane is drawn as one bar: only its two outer ends are rounded, so the bands of the states
  meet square. A fault too short to reach a visible width is drawn at a minimum width and
  coloured **orange instead of red**, so a brief outage is not lost in a wide window and its
  width is never mistaken for its duration; hover it for the exact instants. The legend names
  that band `short fault`.
  A lane states the drops, the average `up` run with its spread, the longest run, and how long
  the target has been up without a drop. Below the chart opens a table of the last 10 faults,
  under a rule that marks where it begins: a row names the target as its lane does — the label,
  with the key in smaller type under it, or the key alone when the target has no label — and
  states the start, the cause of the fault, and — in the last column — the duration. A fault
  still running states it is `and counting`: on a line under the duration, and beside it on the
  same line once the table is stacked. The four columns start
  from a stated share of the width (15, 30, 45 and 10 per cent) and every row is the same
  height. A share is a starting point, never a rule that lets a value be cut in two: a target
  name, a target address, a cause code, each half of an instant, and the average `up` run with
  its spread (`7h54±6h12`) always stay on one line. Only an instant may take two lines, the
  date above the time, and never inside either half. When a column needs more room than its
  share, the table grows wider and scrolls sideways on its own, so the page around it stays
  where it is. On a narrow window — under 700 px — the same values take two columns of two
  lines each: the target, its address and its start on the left, the cause and the duration
  on the right. Nothing is hidden and the column titles move with their values: `target`
  over `started`, `cause` over `lasted`. Two things change with them: the titles no longer
  stay put while the table scrolls, and while a target is still down "and counting"
  follows the duration on the same line instead of taking one of its own.
- `/target?target=<key>&days=30d` — one target for 30, 60, 90, 180, or 360 days: a bar per day,
  the last 10 faults of that target in the same table as the timeline page, and a per-day table.
  The toolbar states the page on one line, the parts separated by a dot:
  `HOME • <label> (<address>)`. `HOME` is the way back to the list page — a link to `/`,
  whatever page you came from —, and the label with the address in parentheses is the target,
  in bold. Under the toolbar, four widgets: the first states the state of the target — its word
  and how long it has held, white on a tile of the colour the chart gives that state —, and the
  other three state the faults, the down time, and the average `up` run of the period. They
  keep that order at every window width and take one of three shapes: four across while they
  fit, then two and two, and on a narrow window the state alone on its line above the other
  three. A figure inside a widget never breaks.
  The table lower down follows them: on the same narrow window each day states its date with
  its fault count beside it, and the down time with the average `up` run under them.
- `Download History` — the whole history as a gzipped CSV, from `/export/faults.csv.gz`, named
  `faults-YYYYMMDD.csv.gz`. On a target page the same link downloads **only that target's**
  history (`/export/faults.csv.gz?target=<key>`) and the file is named after it —
  `faults-<target>-YYYYMMDD.csv.gz`, using the name the page shows. The address the page used
  before this version, `/export/disconnections.csv.gz`, still answers and redirects to the
  whole history.
- `/healthz` — `ok` while the process runs.

**Selecting a fault opens its cause underneath the row** — one fault at a time, and the code
opens it too. The message is drawn as part of that row and not as a fault of its own: the two
share one surface and one bar down their left, the fault gives up the rule between them while
the message is open, and the message sits indented inside that bar, which closes the pair. Its
text is the standard sentence of that failure, with the details of the occurrence in it, for
example "the connection to 9.9.9.9:443 was refused". While a cause is open, a red banner with a
warning sign across the top of the page states that the automatic reload is held, and offers
`Refresh now`. A browser without JavaScript shows the same message on hover, and keeps its plain
30 s reload. A row recorded before this version states no cause and opens nothing.

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

The page reloads itself every 30 s and formats the instants in your browser's time zone. It
holds that reload while a cause is open, so a message you are reading is never taken away: a
red banner at the top of the page says so, and offers `Refresh now` for the moment you are done.
It has
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

### Deploy a new configuration

Verify while the running monitor keeps working, then let the process promote the candidate only
after it probes:

1. Write the candidate beside the main file, for example `next.toml`.
2. Verify it: `network-monitor check --config ./next.toml --listen 0.0.0.0:8642`. A `fail` line
   names the reason; fix it before going on.
3. Add `--try ./next.toml` to the start script or the unit file and restart, or run the process
   with it once. The monitor starts with the candidate.
4. When every target has completed its first probe without a local failure, the candidate is
   copied over `network-monitor.toml`, the previous file is kept as
   `network-monitor.toml.<UTC timestamp>.bak`, and one `CONFIG` notification is sent. A first
   probe that fails locally cancels the promotion: the process logs the rejection, swaps back
   to the main configuration in place, and leaves both files untouched.
5. `--try` may stay in the unit file permanently: a missing candidate is a quiet `INFO` no-op,
   and a candidate identical to the main file promotes nothing and writes no backup.

A promotion never stops the monitor. If the backup or the replace fails (a permission, a full
disk), the error is logged, `CONFIG rejected` is sent, and the running configuration stays in
effect.

### Recover a configuration edited by mistake

The process rejects a broken edit and keeps running the previous configuration **from memory**,
but the file on disk still holds the broken content. The active snapshot is the way back:

```bash
cp network-monitor.toml.active network-monitor.toml   # the reload applies it within a second
```

A restart with the broken file still in place fails, and the error names
`network-monitor.toml.active` and its age. Either copy the snapshot over the file, or start with
`monitor --fallback-to-active`: the process logs one `WARN`, starts from the snapshot, and keeps
watching the main file, so fixing it applies through the normal reload. The flag runs the same
verification `check` runs on the main file (without `--listen` and `--probe`), so it also covers a
file that parses but would not start — an interface that is absent, a target list that is empty, a
channel that cannot be verified.

To undo a promotion, copy the `<main>.<timestamp>.bak` file over the main file and restart; the
reload picks it up without any other change. The snapshot is refreshed at the next start that
probes cleanly, so accepted hot edits are not in it until then — that is what makes it a fixed
recovery point.

### Retention, logs, signals

- `retention_days > 0` deletes older events periodically; `0` keeps everything.
- `--log-level debug` logs every verdict; `info` logs every transition (the default), `warn`
  local probe failures, `error` fatal conditions.
- Every diagnostic line is `<RFC 3339> <LEVEL> <logger> [<target>] <message>`, where the logger
  is `monitor` for the process and `notification` for a channel. A log file channel writes the
  same line, or the JSON document, under the logger name it was given.
- A start writes, at `info`, `initializing`, the configuration file it read (or that it read
  none), one `notification channel <name> (<type>) configured for <destination>` line per
  declared channel, `notification defaults: <names>` (or `none`), one
  `initialized target <key>` line per target, and — when the page is served —
  `web interface ready on http://<address>/` with the address it listens on.
- `SIGINT`/`SIGTERM` stop the process between two probe cycles, with no partial write.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `interface 'eth1' ... does not exist` | The name differs on this host (`en0`, `eth0`, `lo0`). The process refuses to start rather than probe the wrong interface. |
| A `fail` line from `check` | The configuration would not start, and the subject names the condition: `targets` (no target), `interface` (a name that does not exist on this host), `database` (cannot be read, or a schema newer than the binary), `listen` (an address that cannot be bound), `channel` (a channel that cannot open or verify), `probe` (a local failure such as a missing `NET_RAW`). The detail names the key, the file, or the reason. |
| `Operation not permitted` at startup | ICMP needs `root` or `CAP_NET_RAW`. Drop `icmp` targets, or grant the capability. `check --probe` reports the same condition as a `fail probe` line before the restart. |
| A candidate is never promoted | The first probe of a target failed locally, so the promotion was cancelled and the main configuration stayed in effect. The log states the target and the reason (`candidate configuration … cannot probe: …`); run `check --probe` on the candidate to see it before the next attempt. |
| The start fails and names `network-monitor.toml.active` | The main file is broken; the snapshot is the last configuration that started. Copy it back, or add `--fallback-to-active` to start from it and fix the main file afterwards. |
| `network-monitor.toml.active` is older than my last accepted edit | By design: the snapshot is written once per start, after a clean first probe round. An accepted hot edit is not in it until the next restart, and that is what makes it a recovery point rather than a moving target. |
| `no targets configured` | Add `[[targets]]` or `--target`; an empty list is refused. |
| `invalid configuration: targets[0].ip: does not apply to the tcp method` | A key of another method. Each target lists only the keys of its own method. |
| `duplicate target tcp://1.1.1.1:443` | The same key twice. Two methods or two ports are two targets. |
| `the event database ... does not exist` on the page | `serve` reads a database that must exist; start `monitor` once, or point `--database` at the right file. |
| `Is a directory` on start | A bind mount whose source file did not exist: create the file before `docker compose up`. |
| `database error: attempt to write a readonly database` | The database on the volume belongs to another user — typically one copied in from another machine — and `cap_drop: ALL` also drops `CAP_DAC_OVERRIDE`, so root inside the container cannot write it: `docker run --rm -v network-monitor-data:/data alpine chown -R 0:0 /data`. |
| A `dns` target without `resolver` flaps | The system resolver is unreachable and the platform reports it as a missing name. Use `dns://name@server`. |
| An `http` target is always `down` while `curl` works | The status is outside `expect_status` (a 3xx below the default? a 401?), or a TLS-inspecting proxy is at work and its root is not in the binary. |
| The page shows two lanes for one host | The database carries both a bare address and its `icmp://` key (an older binary wrote after the rewrite). Import a backup or merge the rows. |
| `unknown notification channel 'x'` | A `notify` list names a channel that is not declared. `--show-config` prints both, so the typo is visible. The process refuses to start. |
| `WARN notification channel 'x' is declared but nothing enables it` | No `notify` list names the channel, so it receives nothing. The monitor starts anyway: add `x` to the root list or to a target's list, or remove the declaration. |
| `channels 'one' and 'two' both write to …` | Two channels share one destination. Declare one channel per file, URL or chat; a target picks channels by name. |
| A channel stays silent | Check, in this order: the channel is declared and named in the target's effective `notify` list (`--show-config`); the event is not hidden by the channel's `level`; for Telegram the bot is an **administrator** of the chat, `chat_id` copies `channel_post.chat.id` and starts with `-100`; for a webhook the endpoint answers `2xx`. The log shows no `WARN notification attempt …` line. A `400 Bad Request: chat not found` is a wrong `chat_id`; `401 Unauthorized` a revoked token; `429` too many messages at once. |
| A `WARN notification attempt 1/5 failed` line repeats five times | The channel is unreachable or rejecting the call. The reason on the line names the answer without the secret; `--log-level debug` adds nothing, because a failed send is always a warning. Other channels are unaffected, and so is the event history — read it with `query events`. |
| A log file channel writes nothing | The event is below the channel's `level` (`error` hides warnings and information), or the directory of `filename` does not exist. The tool creates the file, never its directory. |
| The process refuses to start with `cannot open …/audit.log for append` | The file or its directory is not writable by the process user. Create the directory and grant write access; a channel that cannot open is fatal at startup. |
| No `UP` message arrives after an outage while the page shows the target up | The recovery is still inside the stable `up` window (`stabilize_seconds`, 60 s by default). Wait for the window to close; a drop inside it cancels the message by design. |

## Limits and deliberate choices

- IPv4 only; no IPv6 targets.
- HTTP checks do not follow redirects, send no body, and cannot present credentials or a client
  certificate.
- DNS supports `A` and `AAAA` over UDP on port 53, no DNSSEC, no DoT/DoH.
- `interface` is supported on `icmp` and `tcp` only.
- The page has no authentication and no write path.
- The trust store is the one compiled into the binary.
- Notification channels are Telegram, a webhook, and a log file; no mail, MQTT, Slack-specific
  payload or command hook, and no rotation by the tool.
- There is exactly one active snapshot and no history of snapshots, and the deployment backups
  are never pruned by the tool. A candidate is promoted only after every target has probed once
  without a local failure; reachability of a target is not a condition.

Each item is deliberate rather than missing: leaving out IPv6, redirects, DNSSEC, DoT/DoH and
client certificates keeps one probe cheap and the binary portable, the trust store travels
inside the binary so an appliance needs no CA package, and the page is read-only so the monitor
holds no credentials.
