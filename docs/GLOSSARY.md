# Glossary — network-monitor

The domain language of this project. Each term has exactly one definition; code,
documentation, and conversations use these terms and no synonyms. Add a term when it
appears in a requirement, in a component name, or in a public interface.

| Term | Definition |
|---|---|
| **Config file** | The TOML file that declares global defaults and the list of targets; read once at startup. |
| **Day total** | The record the JSON API states for one UTC day: the faults that started in it, the time spent down, the mean and the longest `up` run, and whether the day is still running. A day the window clips counts only what falls inside it. |
| **Default interface** | The interface the operating system routing table selects for a destination when no interface is configured for the probe. |
| **Envelope** | The shape of every answer of the JSON API: an object whose `data` holds the records and whose `meta` states the page, the filters applied, the total matching records, and the instant the answer was built. A bare array is never the root, and an error states `error` instead of `data`. |
| **Event** | One persisted row describing a single state transition of a single target. |
| **Event database** | The SQLite file that holds the `events` and `targets` tables and is the only integration point of the tool. |
| **History** | The read contract of the page, the JSON API and the export: the eleven questions they ask of a recorded past — the state of every target, one timeline, the histogram of a window, the faults with their paging, the events, the pages of an export. `store::Store` answers it over SQLite and the **simulation** answers it over generated days. |
| **History source** | Where a request gets its **History** from: one instance lives for the process and hands out one per request. The **event database** read per request and the **simulation** are the two implementations, so the page, the JSON API and the export name the source and not a database. |
| **Interface binding** | Constraining a probe's packet to a named interface, with the semantics of `ping -I <interface> <address>`. |
| **Interval** | The delay between two consecutive probe cycles of the same target. |
| **Local error** | A failure of the host to perform a probe — missing privilege, missing interface, socket error, unresolvable name, malformed answer — as opposed to a target that does not answer. The DNS system resolver is the one case where the platform hides the difference; see [FR-21](./PRODUCT.md). |
| **Monitor** | The long-running subcommand that probes targets and persists transitions. |
| **Bot token** | The secret Telegram issues for a bot, which authorises `sendMessage` and travels in the request path. It is read from the configuration file only, never from a flag or an environment variable, and every rendering of it — `Debug`, `--show-config`, a log line, an error message — reads `<redacted>`. |
| **Chat ID** | The identifier of the Telegram chat or channel a message is posted to; the `chat_id` of a **Telegram channel** declaration. |
| **Notification** | One **message** the tool hands to every **channel** a `notify` list enables, stating a target or the process, the state it reached, the cause when it is a `down`, and the instant. A target notification is created from a recorded state transition and never before it. |
| **Channel** | A configured destination of notifications: a Telegram chat, a webhook endpoint, or a log file. It is declared once, under a unique **channel name**, and enabled by that name in a `notify` list; it has no scope of its own. |
| **Channel name** | The `name` of a **channel** declaration, unique across the configuration; the only way a `notify` list references it. |
| **Enablement** | Naming a **channel** in a `notify` list: the root list enables it for **global events** and as the default for every target, a target's list replaces the root list for that target, an absent key inherits it, and an empty list silences the target. |
| **Output destination** | The real sink that identifies a **channel** and makes it unique: the normalised log filename, the canonical webhook URL, or the Telegram `chat_id`. Two channels may not share one, so one event reaches a destination once (`INV-14`). |
| **Message** | One typed fact published to a channel: its scope (global or target), the event, the target key and label, the state, the cause, and the instant. Each channel renders it in its own form. |
| **Global event** | A **message** about the process rather than a target: start, stop, reload, or channel start. The root `notify` list enables the channels that receive it. |
| **Target event** | A **message** about one target's state, `down` or `up`; it reaches the channels the target's effective `notify` list names. |
| **Webhook channel** | A **channel** whose **output destination** is a URL; it posts one JSON document (schema `1`) per message, optionally with a bearer token that requires `https`. |
| **Webhook token** | The optional bearer secret of a **webhook channel**; read from the file only, printed as `<redacted>`, refused on an `http` URL, and never part of a log line or an error. |
| **Log file channel** | A **channel** that appends one line per message to a file, as the diagnostic text or as one JSON document per line, under its own **logger name** and **level**. |
| **Log format** | The rendering of a **log file channel**: `text` (the diagnostic line) or `jsonl` (one compact JSON document per line). |
| **Logger name** | The name every diagnostic line carries: `monitor` for the process and `notification` for a channel. A **log file channel** writes the name it was given. |
| **Fault rate** | The `--fault-rate` a **simulation** runs with: the faults a day of a target that fails now and then holds. One target in five never fails whatever the number, one in ten flaps with about eight times as many faults, and `0` is a board where nothing goes down. It moves how many faults there are, never the shape of a day. |
| **Seed** | The integer a **simulation** starts from. Two runs of the same seed show the same history; another seed shows another one. It is the only input of the generated past besides the target keys, the calendar and the **fault rate**. |
| **Simulation** | The `simulate` subcommand: it serves the same page over a history generated on demand instead of recorded. It runs no probe, reads and writes no database, and keeps nothing between requests; a whole day of every target is a pure function of the **seed**, the target key and the UTC day, so the same command replays the same history. |
| **Simulated history** | The past a **simulation** presents: whole UTC days ending today, `--history` of them, cut into stretches of `up` and `down` with the failures and latencies of each target's **method**. Nothing exists before its first day and nothing after now, and the day that begins inside a fault opens with that fault. |
| **Stable `up` window** | The interval `stabilize_seconds` during which `up` must hold continuously before a recovery is reported. A `down` inside the window cancels the report; `0` reports the recovery at once. |
| **Probe** | One reachability check of a target, in the transport its method names: an ICMP echo request, a TCP connection, a DNS question, or an HTTP request. |
| **Priority** | The integer a target states in the configuration to place its lane: the targets that state one come first, the lowest number first, and those without one follow by key. It orders the lanes of the page and the list of the JSON API, and it changes nothing else. |
| **Probe cycle** | The evaluation of one target that produces one verdict. |
| **Method** | The transport a target is checked with: `icmp`, `tcp`, `dns`, or `http`. It is part of the target's identity. |
| **Endpoint** | The method plus the parameters that identify what is checked — an address, a host and port, a name and optional server, a URL — and the expectation, such as the HTTP statuses that count as `up`. |
| **Query** | The short-lived subcommand that reads the event database and renders it as a table or JSON. |
| **Reachable (`up`)** | The state of a target whose probe got the answer its method expects within the timeout: an echo reply, a completed TCP handshake, a record of the requested type, or a status inside the expectation. |
| **Reload** | Re-reading the configuration file while monitoring runs and applying a valid change to the interval, timeout, target list, and retention without a restart. |
| **Check** | The `check` subcommand: it verifies the conditions a start checks — the target list, every target's interface, the event database, the optional listening address, the declared channels, the active snapshot, and optionally one real probe per target — as one `ok`/`warn`/`fail` line each. It starts nothing, sends no notification, and writes nothing to the event database; it exits non-zero on a `fail`. |
| **Candidate configuration** | A configuration file named by `monitor --try`. The process starts with it only when it passes the **check** and its bytes differ from the main file, and it becomes the main file only through a **promotion**. An absent candidate is a quiet no-op; a candidate that cannot be used leaves the main file in effect. |
| **Promotion** | The atomic replacement of the main configuration file by a verified **candidate configuration** after every target of the start has probed once without a local failure. The previous file is kept as `<main>.<UTC timestamp>.bak`; a local failure in the first round cancels the promotion and swaps the process back to the main configuration without touching either file. |
| **Active snapshot** | The `<main>.active` file holding the exact bytes of the configuration the last start began from. It is written once after a clean first probe round, never by a reload or a promotion, is created with mode `0600` on Unix, and is the recovery point — not a configuration layer and never read by the monitor. |
| **Retention** | The policy that deletes events older than `retention_days`; a value of `0` disables deletion. |
| **State transition** | A change of a target's verdict between two consecutive probe cycles. |
| **Target** | One endpoint under supervision, with an optional interface, an optional label, and an optional timeout and interval of its own. |
| **Target key** | The canonical string that identifies a target in the database, in links, and in queries: `icmp://1.1.1.1`, `tcp://1.1.1.1:443`, `dns://example.com@9.9.9.9`, `https://example.com/health`. |
| **Target specification** | The command-line form of a target: `[method:]address[,key=value]…`, where a bare address is `icmp`. |
| **Archive** | The gzipped CSV of every event, written by the page and read by `import`; one format, two directions. |
| **Current `up` streak** | The time since the transition that turned a target `up`; it does not exist while the target is `down` or `unknown`, and it reaches back before the shown window when the target has been up that long. |
| **Fault** | One recorded failure of a target: the `down` transition that started it, the cause it states, and — if it happened — the transition back to `up`. Its duration is the distance between the two, or from the transition to now while the target is still down. The page lists the last ten, the CSV has one row per event, and `query events` shows them. |
| **Timeline segment** | An interval of constant state for one target, from one transition to the next or to the edge of the window. |
| **Timeout** | The maximum time a probe waits for its answer; its expiry yields `down`. It is global unless a target states its own. |
| **Up run** | A maximal stretch of `up` of one target inside a window; a run that the window opens or closes in the middle of counts as it appears. |
| **Unknown state** | The state of a target before its first verdict has been recorded. |
| **Unreachable (`down`)** | The state of a target whose probe got a definitive negative answer, or no answer within the timeout: no echo reply, a refusal, a missing name, or a status outside the expectation. |
| **Cause** | The reason a check failed, as a code plus a standard message. Every failure of a probe cycle states one: the verdict it produces, or the local failure that prevents a verdict. |
| **Cause code** | The short uppercase token that identifies a cause, prefixed with the layer that reported it: `ICMP.NO_REPLY`, `TCP.REFUSED`, `TCP.TIMEOUT`, `DNS.NXDOMAIN`, `DNS.NO_RECORD`, `DNS.TIMEOUT`, `DNS.UNRESOLVED`, `HTTP.UNEXPECTED_STATUS`, `HTTP.TLS`, and the codes of the local failures (`TCP.RESOLVE`, `HTTP.MALFORMED`, `PROBE.WRONG_METHOD`, …). It is what the database stores, a table shows, and `query` prints, so a log line can be filtered by it. |
| **Cause message** | The one sentence a cause states, declared in the code as a template and rendered with the details of the occurrence — the host and port, the status and the expectation, the timeout, the text the platform returned. A detail a code does not supply reads `unknown`; a message is never built by hand at a call site. |
| **Verdict** | The outcome of one probe cycle: `up` with the time the check took, or `down` with its cause. A local failure is a `ProbeError`, not a verdict, and is never persisted. |
| **Web page** | The optional read-only HTML view of the timelines and of the per-target detail page, served either by `serve` or by `monitor --listen`. |
