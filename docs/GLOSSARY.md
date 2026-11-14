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
| **Down episode** | One `down` transition and, if it happened, the transition back to `up`; its duration is the distance between the two, or from the transition to now while the target is still down. |
| **Timeline segment** | An interval of constant state for one target, from one transition to the next or to the edge of the window. |
| **Timeout** | The maximum time a probe waits for its answer; its expiry yields `down`. It is global unless a target states its own. |
| **Up run** | A maximal stretch of `up` of one target inside a window; a run that the window opens or closes in the middle of counts as it appears. |
| **Unknown state** | The state of a target before its first verdict has been recorded. |
| **Unreachable (`down`)** | The state of a target whose probe got a definitive negative answer, or no answer within the timeout: no echo reply, a refusal, a missing name, or a status outside the expectation. |
| **Verdict** | The outcome of one probe cycle: `up` with the time the check took, or `down`. A local failure is a `ProbeError`, not a verdict. |
| **Web page** | The optional read-only HTML view of the timelines and of the per-target detail page, served either by `serve` or by `monitor --listen`. |
