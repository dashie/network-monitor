# Glossary — network-monitor

The domain language of this project. Each term has exactly one definition; code,
documentation, and conversations use these terms and no synonyms. Add a term when it
appears in a requirement, in a component name, or in a public interface.

| Term | Definition |
|---|---|
| **Config file** | The TOML file that declares global defaults and the list of targets; read once at startup. |
| **Default interface** | The interface the operating system routing table selects for a destination when no interface is configured for the probe. |
| **Event** | One persisted row describing a single state transition of a single target. |
| **Event database** | The SQLite file that holds the `events` and `targets` tables and is the only integration point of the tool. |
| **Interface binding** | Constraining a probe's packet to a named interface, with the semantics of `ping -I <interface> <address>`. |
| **Interval** | The delay between two consecutive probe cycles of the same target. |
| **Local error** | A failure of the host to perform a probe — missing privilege, missing interface, socket error, unresolvable name, malformed answer — as opposed to a target that does not answer. The DNS system resolver is the one case where the platform hides the difference; see [FR-21](./PRODUCT.md). |
| **Monitor** | The long-running subcommand that probes targets and persists transitions. |
| **Bot token** | The secret Telegram issues for a bot, which authorises `sendMessage` and travels in the request path. It is read from the configuration file only, never from a flag or an environment variable, and every rendering of it — `Debug`, `--show-config`, a log line, an error message — reads `<redacted>`. |
| **Chat ID** | The identifier of the Telegram chat or channel a message is posted to; `chat_id` in the `[telegram]` section. |
| **Notification** | One message the tool sends to the configured Telegram chat, stating a target, the state it reached, the cause when it is a `down`, and the instant. It is created from a recorded state transition and never before it. |
| **Stable `up` window** | The interval `stabilize_seconds` during which `up` must hold continuously before a recovery is reported. A `down` inside the window cancels the report; `0` reports the recovery at once. |
| **Probe** | One reachability check of a target, in the transport its method names: an ICMP echo request, a TCP connection, a DNS question, or an HTTP request. |
| **Probe cycle** | The evaluation of one target that produces one verdict. |
| **Method** | The transport a target is checked with: `icmp`, `tcp`, `dns`, or `http`. It is part of the target's identity. |
| **Endpoint** | The method plus the parameters that identify what is checked — an address, a host and port, a name and optional server, a URL — and the expectation, such as the HTTP statuses that count as `up`. |
| **Query** | The short-lived subcommand that reads the event database and renders it as a table or JSON. |
| **Reachable (`up`)** | The state of a target whose probe got the answer its method expects within the timeout: an echo reply, a completed TCP handshake, a record of the requested type, or a status inside the expectation. |
| **Reload** | Re-reading the configuration file while monitoring runs and applying a valid change to the interval, timeout, target list, and retention without a restart. |
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
