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
| **Local error** | A failure of the host to perform a probe — missing privilege, missing interface, socket error — as opposed to a target that does not answer. |
| **Monitor** | The long-running subcommand that probes targets and persists transitions. |
| **Probe** | One ICMP echo request sent to a target, plus the bounded wait for its matching reply. |
| **Probe cycle** | The evaluation of one target that produces one verdict. |
| **Query** | The short-lived subcommand that reads the event database and renders it as a table or JSON. |
| **Reachable (`up`)** | The state of a target whose probe received a matching echo reply within the timeout. |
| **Reload** | Re-reading the configuration file while monitoring runs and applying a valid change to the interval, timeout, target list, and retention without a restart. |
| **Retention** | The policy that deletes events older than `retention_days`; a value of `0` disables deletion. |
| **State transition** | A change of a target's verdict between two consecutive probe cycles. |
| **Target** | One IP address under supervision, with an optional interface and an optional label. |
| **Target specification** | The command-line form of a target: `IP[,interface=NAME][,label=NAME]`. |
| **Archive** | The gzipped CSV of every event, written by the page and read by `import`; one format, two directions. |
| **Down episode** | One `down` transition and, if it happened, the transition back to `up`; its duration is the distance between the two, or from the transition to now while the target is still down. |
| **Timeline segment** | An interval of constant state for one target, from one transition to the next or to the edge of the window. |
| **Timeout** | The maximum time a probe waits for a matching echo reply; its expiry yields `down`. |
| **Unknown state** | The state of a target before its first verdict has been recorded. |
| **Unreachable (`down`)** | The state of a target whose probe received no matching echo reply within the timeout. |
| **Verdict** | The outcome of one probe cycle: `up` with a round-trip time, or `down`. A local failure is a `ProbeError`, not a verdict. |
| **Web page** | The optional read-only HTML view of the timelines and of the per-target detail page, served either by `serve` or by `monitor --listen`. |
