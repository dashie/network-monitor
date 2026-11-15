---
title: Support ICMP, TCP, DNS and HTTP targets behind one probe contract
date: 2026-11-14
status: done
type: feature
impact: high
area: probe, config, store identity
version_bump: minor
tags: [probe, tcp, dns, http, tls, config, identity]
depends_on: []
branch_recommended: true
---

## Summary

A target stops meaning "one IP address probed with ICMP" and becomes "one reachability
check": a method plus its parameters, with the common properties `label`, `interface`
(where the method supports it), `timeout_ms`, and `interval_ms`. Four methods ship:
`icmp` (today's behaviour, unchanged), `tcp` (a port answers), `dns` (a name resolves, via
the system resolver or one named server), and `http` (`http://` or `https://` answers with
an expected status). Each method lives in its own file under `src/probe/`, behind one
contract, with its own tests; the monitor, the store, the web page, and the query
subcommands keep working on today's data because the identity of a target stays a single
string in the database — now a prefixed key such as `tcp://1.1.1.1:443`.

## Why

The product can answer "is this IP reachable" only with ICMP, which needs `NET_RAW` and is
blocked on many networks where a TCP port or an HTTPS endpoint is perfectly observable.
Adding the three methods the operator asked for in one ad-hoc way would put four transports
in one module and make the per-method logic untestable; the repository's INV-3 ("the monitor
depends on the probe contract only") and the "one file per responsibility" rule in
[DEVELOPER-GUIDE](../docs/DEVELOPER-GUIDE.md#extending-the-project) both require the split
this plan describes. The cost of not doing it is a probe module that only grows and a
history that cannot record anything but ICMP.

## Goals

- `[[targets]]` accepts `method = "icmp" | "tcp" | "dns" | "http"` (default `icmp`, so every
  existing file keeps working) and the method-specific keys listed in the summary.
- `--target` accepts `method:rest,attr=value`; a bare `1.1.1.1` keeps meaning ICMP.
- `timeout_ms` and `interval_ms` can be given per target and override the global values.
- A target's identity is one string: `icmp://1.1.1.1`, `tcp://1.1.1.1:443`,
  `dns://example.com`, `dns://example.com@9.9.9.9`, `http://…`, `https://…`. It is what the
  `events` and `targets` tables store, what the page links to, and what `query` filters on;
  no schema change and no new column.
- Existing databases and archives keep their history: bare-IP rows are rewritten once to
  `icmp://…` by an idempotent step during a writable open, and `import` accepts both forms.
- Each method is a module with its own tests: `src/probe/{icmp,tcp,dns,http}.rs`, plus the
  shared contract in `src/probe/mod.rs`. `cargo test --lib probe::<method>` runs one method
  with no other module started.
- Verdict semantics are uniform and documented: a definitive answer from the target is a
  verdict (`up`/`down`), a failure of our own stack is a `ProbeError` (INV-5 extended per
  method).
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass at
  every checkpoint.

## Non-goals

- IPv6 targets: ICMP stays IPv4; `tcp`/`dns`/`http` may use IPv4 addresses only in v1.
- ICMP via an unprivileged datagram socket (`ping_group_range`); the raw socket stays.
- HTTP features beyond a single request: no redirects, no authentication, no body matching,
  no proxy, no HTTP/2, no custom CA bundle.
- DNS record types other than `A` and `AAAA`, no DNSSEC, no DoH/DoT, no retry policy.
- Renaming `rtt_us` or changing the CSV columns; the archive format is untouched.
- A method badge or per-method colour on the web pages; lanes keep their current look.
- Changing the Dockerfile or the image defaults (existing `[[targets]]` entries stay valid).
  `docker-compose.yml` is the deployment of the machine that runs it and changes only on the
  user's request; the two website targets added after step 7 are such a request.

## Affected modules

**Source**
- `src/probe/mod.rs` — new `Method`/`Endpoint` value types, the redefined `Probe` trait, the
  `Prober` dispatcher, re-exports.
- `src/probe/icmp.rs` — same algorithm, adapted to the new trait (no timeout state).
- `src/probe/tcp.rs` — *(new)* connect check, optional interface binding.
- `src/probe/dns.rs` — *(new)* hand-written query codec over UDP, system-resolver path.
- `src/probe/http.rs` — *(new)* request/response over plain TCP, TLS via `rustls`.
- `src/probe/fake.rs` — scripted verdicts keyed by endpoint, still privilege-free.
- `src/probe/interface.rs` — expose the fd-level bind for the TCP socket (no behaviour change).
- `src/config.rs` — `Target` with method parameters, per-method validation, `key()`,
  `display()`, per-target timeout and interval, `--show-config` rendering.
- `src/cli.rs` — `--target` grammar with a method prefix; help text.
- `src/monitor/mod.rs` — workers use `Target::key()`, per-target interval/timeout, no
  `set_timeout`, prober built once.
- `src/monitor/state.rs` — *(maybe)* unchanged; only if the transition signature moves.
- `src/main.rs` — validation per method (`interface` refused on dns/http), prober wiring.
- `src/store/mod.rs` — one-time idempotent rewrite of legacy target keys on writable open.
- `src/import.rs` — accept bare-IP and prefixed keys, store the prefixed form.
- `src/web/mod.rs` — percent-encode the target key in generated links and decode it on read.
- `Cargo.toml` — `rustls` (ring provider), `webpki-roots`, `socket2`; version `0.2.0`;
  description without "ICMP only".
- `Cargo.lock` — regenerated.

**Tests**
- `src/probe/tcp.rs` (unit) — *(new)* open port, refused port, error classification, spec
  parsing; one `#[ignore]` test for an unroutable timeout.
- `src/probe/dns.rs` (unit) — *(new)* query encoding, answer parsing, RCODE classification,
  a local UDP server answering success and NXDOMAIN, and a silent server for the timeout.
- `src/probe/http.rs` (unit) — *(new)* request line/headers, status-line and head parsing,
  expected-status matching, a local TCP server for 200, 500 and a malformed response.
- `src/probe/icmp.rs` (unit) — existing packet tests; add one test that the probe honours a
  per-call timeout passed by the caller.
- `src/config.rs` (unit) — parsing and validation per method, defaults, key rendering.
- `tests/config_precedence.rs` — extend the key walk with `method` and the per-target keys.
- `tests/cli.rs` — `--target` grammar, `--show-config` for each method, invalid specs.
- `tests/store.rs` — legacy key rewrite is idempotent and keeps `events`/`targets` consistent.
- `tests/import.rs` — a bare-IP archive imports as `icmp://…`; a prefixed archive round-trips.
- `tests/monitor_e2e.rs` — a scripted fake drives one target of each method through a
  transition; per-target interval and timeout are applied.
- `tests/reload.rs` — a reload that changes one target's method and another's timeout is
  applied without restart.
- `tests/web.rs` — a URL-shaped key survives the link and the `?target=` round trip.
- `tests/probe_network.rs` — *(new, `#[ignore]`)* real TCP, DNS and HTTPS checks against
  public endpoints for the manual acceptance run.

**Documentation / configuration**
- `docs/PRODUCT.md` — new FRs for the four methods, amended FR-2 (target spec), FR-13
  (validation), FR-14 (single binary with TLS roots), FR-18/FR-19 (identity in the archive,
  the "no TLS stack" sentence becomes false).
- `docs/ARCHITECTURE.md` — `probe` component table and section, INV-3 and INV-5 wording,
  container note on embedded roots.
- `docs/GLOSSARY.md` — `Method`, `Endpoint`, redefined `Target`, `Probe`, `Probe cycle`,
  `Verdict`, `Timeout`, `Reachable`/`Unreachable`.
- `docs/ADR.md` — ADR-0023 (methods behind one contract, prefixed identity) and ADR-0024
  (TLS, DNS and socket dependencies without an async runtime); index rows.
- `docs/DEVELOPER-GUIDE.md` — rewrite "Adding a probe type", note the per-method test command.
- `README.md` — description and one example per method.
- `network-monitor.toml` — one commented example per method, per-target timeout and interval.
- `.spec-workflow.yml` — *(new)* created with this plan (workflow profile).

## Architecture impact

The `probe` component becomes a family: one contract, four implementations, one dispatcher.
`config` gains a richer `Target` and the validation that belongs to it; `monitor` loses the
global `set_timeout` path and takes timeout and interval per target; `store` keeps its
schema and gains a data normalisation step. [ARCHITECTURE.md](../docs/ARCHITECTURE.md) needs
its component table, the `probe` section (input, output, guarantees), the data-model wording
for the target key, and the invariant text for INV-3 and INV-5 updated.

## Decision-record impact

Two records:

- **ADR-0023** — one probe contract with an endpoint value type and four methods, and the
  prefixed target key as the stored identity (rejected: a `method` column, a separate table,
  four separate monitors).
- **ADR-0024** — `rustls` with the `ring` provider plus embedded `webpki-roots`, a
  hand-written DNS codec, and `socket2` for interface-bound TCP, instead of an async stack
  (`hickory-resolver` needs Rust 1.88 and a tokio runtime) or a system TLS library.

## Proposed solution

### Step 1 — Rebuild the probe contract around an endpoint

Add to `src/probe/mod.rs`:

- `pub enum Method { Icmp, Tcp, Dns, Http }` and `pub enum Endpoint` with one variant per
  method carrying its parameters (`Icmp { address: Ipv4Addr }`,
  `Tcp { host: String, port: u16 }`,
  `Dns { name: String, resolver: Option<IpAddr>, record: RecordType }`,
  `Http { scheme: Scheme, host: String, port: u16, target: String, expect: StatusMatch }`).
- `impl Display for Endpoint` producing the canonical key, and `Endpoint::method()`.
- The trait becomes the single request boundary:
  `fn probe(&self, endpoint: &Endpoint, interface: Option<&str>, timeout: Duration) -> Result<Verdict, ProbeError>`.
- `pub enum Prober { Icmp(IcmpProbe), Tcp(TcpProbe), Dns(DnsProbe), Http(HttpProbe) }`
  implementing `Probe` by matching the endpoint; `Prober::new()` is the only constructor the
  composition root needs.
- Delete `Probe::set_timeout`; `IcmpProbe` receives the timeout per call and loses its
  atomics. `src/monitor/mod.rs` stops calling `set_timeout` (line 190) and builds the
  endpoint from the target.
- `src/probe/fake.rs` scripts verdicts per endpoint and records the timeout it was given, so
  the monitor tests can assert per-target timeouts without sockets.

ICMP behaviour is unchanged: same packet, same socket-per-call, same classification.

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
(the whole existing suite, including `tests/monitor_e2e.rs` and `tests/icmp_privileged.rs`,
passes with ICMP only); `cargo run -- monitor --config network-monitor.toml` still logs the
same transitions.

### Step 2 — Model the target and its identity

`src/config.rs`:

- `Target { endpoint: Endpoint, interface: Option<String>, label: Option<String>, timeout: Duration, interval: Duration }`
  with `key()` (`endpoint.to_string()`), `display()` (label or key), and `method()`.
- `FileTarget` gains `method` (default `icmp`), `host`, `port`, `name`, `resolver`, `record`,
  `url`, `expect_status`, `timeout_ms`, `interval_ms`. Validation is per method and names the
  offending key: a key that does not belong to the method, a missing required key, a port of
  zero, an unparseable URL, `interface` on `dns`/`http`, a duplicate `key()`.
- `Overrides.targets` keeps `Vec<Target>`; `Target::parse_spec` accepts
  `icmp:1.1.1.1,interface=eth1,label=x,timeout_ms=1500,interval_ms=1000`,
  `tcp:1.1.1.1:443,label=https`, `dns:example.com@9.9.9.9,record=AAAA`,
  `http:https://example.com/health,expect=200,label=site`, and a bare `1.1.1.1` (ICMP).
- `--show-config` renders every target with its method and only that method's keys, so a
  round trip `--show-config > file` is a valid configuration.

`src/store/mod.rs`: add `rewrite_legacy_target_keys()` — one `UPDATE` in a transaction that
prefixes every `target` value without `://` in `events` and `targets` with `icmp://`. It runs
from the writable open (`open_rw`) and is idempotent; the read-only open used by the page is
untouched (INV-10).

`src/monitor/mod.rs`: events are written with `target.key()`, the previous state is looked up
by the same key, and each worker uses `target.interval` / `target.timeout`.

`src/import.rs`: a `target` value without `://` is read as `icmp://…`; the stored row carries
the prefixed form.

**Verify:** `cargo test config store import cli monitor_e2e`; then
`cargo run -- --show-config --target 1.1.1.1 --target tcp:1.1.1.1:443 --target dns:example.com --target http:https://example.com/` prints four well-formed targets; a test builds a
database with bare-IP rows, opens it writable twice, and asserts one rewrite and unchanged
history.

> **🔶 CHECKPOINT CP-1 — contract and identity in place**
> **Stable state:** ICMP-only behaviour with the new contract, identity, config, and CLI; the
> database still readable by the previous binary (keys are opaque text).
> **Verify:** the quality gate above.
>
> The implementer stops here, reports, and waits for explicit confirmation.

### Step 3 — TCP prober

`src/probe/tcp.rs`:

- `TcpProbe` implements `Probe` for `Endpoint::Tcp`.
- No interface: `std::net::TcpStream::connect_timeout` against the first resolved address of
  `(host, port)`.
- Interface given: create the socket with `socket2`, call `interface::bind(fd, name)`, then
  connect with the same timeout and convert to `TcpStream`.
- Classification: a completed handshake is `Up { rtt }`; `ECONNREFUSED`, `ETIMEDOUT`, and a
  timeout expiring are `Down`; a name that does not resolve, `EACCES`, `EADDRNOTAVAIL`, and a
  missing interface are `ProbeError::local`.

**Verify:** `cargo test --lib probe::tcp` (local listener → `Up`, closed port → `Down`, spec
and error mapping covered); `cargo run -- monitor --target tcp:127.0.0.1:22 --interval-ms 1000 --log-level debug`
shows a verdict for an open and a closed port.

### Step 4 — DNS prober

`src/probe/dns.rs`, no new dependency:

- Pure codec: `build_query(name, record, id) -> Vec<u8>` and
  `parse_response(bytes) -> Result<Answer, DnsError>`, handling the header, QNAME encoding
  and decoding (compression pointers in answers), `A` and `AAAA` records, and CNAME chains.
- Without `resolver`: `(name, 0).to_socket_addrs()`. `EAI_NONAME`/not found is `Down`; a
  temporary failure (`EAI_AGAIN`, `EAI_FAIL`, system error) is `ProbeError::local`.
- With `resolver`: one UDP query to `resolver:53` with the target timeout. `NOERROR` with at
  least one answer is `Up { rtt }`; `NXDOMAIN` and `NOERROR` with no answer are `Down`;
  `SERVFAIL`, `REFUSED`, `NOTIMP`, a truncated response, a malformed response, and a socket
  error are `ProbeError::local`.
- Response buffer is bounded (4 KiB) and the parser never loops on pointers.

**Verify:** `cargo test --lib probe::dns` (codec round trip, NXDOMAIN, SERVFAIL, malformed,
a local UDP server answering and one staying silent); manual
`cargo run -- monitor --target dns:example.com@9.9.9.9 --log-level debug`.

### Step 5 — HTTP and HTTPS prober

`src/probe/http.rs`:

- URL parsing at configuration time (`http` and `https`, optional port, path and query,
  no credentials, no fragment, ASCII host); reject anything else with a named key.
- Request: `GET <target> HTTP/1.1`, `Host`, `User-Agent: network-monitor/<version>`,
  `Accept: */*`, `Connection: close`; no body, no redirect following.
- Response: read the head up to `\r\n\r\n` within an 8 KiB cap, parse the status line;
  `expect_status` defaults to `200..=399` and accepts a list of exact codes or a class
  (`2xx`, `4xx`).
- Status inside the expectation is `Up { rtt }`; a status outside it, a refused or timed-out
  connection, and a TLS handshake or certificate failure are `Down`; a local socket error,
  a malformed status line, and a head that exceeds the cap are `ProbeError::local`.
- TLS: `rustls` 0.23 with `default-features = false` and the `ring` provider, roots from
  `webpki-roots`, SNI from the host, ALPN `http/1.1`. Verification cannot be disabled.

**Verify:** `cargo test --lib probe::http` (request/status parsing, local server for 200,
500 and a malformed response); manual
`cargo run -- monitor --target http:https://example.com --log-level debug` and one run with
a wrong `expect=204` that must record `down`.

### Step 6 — Page, query and archive with prefixed keys

- `src/web/mod.rs`: add `encode(value)` next to `decode`, and use it in every generated link
  that carries a target key (`/?…`, `/target?target=…`, `/export/…`), so a key containing
  `?`, `&`, or `#` survives the round trip.
- `src/web/page.rs` and `src/web/detail.rs`: the lane label keeps using `display()` and
  `short()`; nothing else changes.
- `src/web/export.rs` and `src/import.rs`: the CSV columns are unchanged; the `target` value
  is the prefixed key, and import accepts the bare legacy form.
- `src/query/render.rs`: no code change expected; confirm that a filter matches both the key
  and the label (existing `resolve_target`).

**Verify:** `cargo test --test web --test import`; start `serve` on a database seeded with
each method and confirm the detail link for `tcp://…` and `https://…/a?b=1` opens the right
target and the download/import round trip keeps the exact keys.

### Step 7 — Documentation, decisions, examples

- `docs/PRODUCT.md`: one FR per method, amended FR-2/FR-13/FR-14, and the two archive
  sentences that name ICMP only.
- `docs/ARCHITECTURE.md`: component table row and `probe` section with the four contracts,
  INV-3/INV-5 rewording, data-model note on the key, container note on embedded roots.
- `docs/GLOSSARY.md`: `Method`, `Endpoint`, redefined `Target`, `Probe`, `Probe cycle`,
  `Verdict`, `Timeout`, `Reachable`, `Unreachable`.
- `docs/ADR.md`: ADR-0023 and ADR-0024 with the alternatives rejected above, plus the index
  rows.
- `docs/DEVELOPER-GUIDE.md`: "Adding a probe type" rewritten around
  `src/probe/<method>.rs` and `cargo test --lib probe::<method>`.
- `README.md` and `network-monitor.toml`: one example per method; per-target timeout and
  interval.
- `Cargo.toml`: version `0.2.0`, description without "ICMP only".

**Verify:** `cargo run -- --show-config --config network-monitor.toml` accepts the example;
`grep -rn "ICMP" README.md docs/PRODUCT.md` shows only statements that are still true;
`docker compose -f docker-compose.yml config -q` stays valid.

> **🔶 CHECKPOINT CP-4 — all four methods and the documentation in place**
> **Stable state:** every method proven by unit tests and one manual run; the docs describe
> the shipped behaviour.
> **Verify:** the full quality gate plus the manual matrix below.
>
> The implementer stops and waits for the review phase.

## Security considerations

- TLS verification is mandatory: no flag disables it, and the roots are the embedded
  `webpki-roots` set, so an operator behind a TLS-inspecting proxy must rebuild the image.
  That is deliberate and recorded in ADR-0024.
- No redirects, no credentials in URLs (rejected at parse time), and no request body: the
  prober cannot be turned into a general-purpose client or leak a location.
- Every read is bounded: 8 KiB for the HTTP head, 4 KiB for the DNS response, one DNS query
  per cycle, no retry loop. A hostile or broken endpoint can cost at most one timeout per
  interval.
- The DNS parser is hand-written against untrusted bytes: strict bounds checks, no pointer
  loop (a pointer must point strictly backwards), and tests for truncated and malformed
  messages. Fuzz-style cases are in the unit tests.
- `interface` stays limited to `icmp` and `tcp`; the two methods that could otherwise reach a
  resolver through an unexpected path reject it.
- No secret is introduced: the configuration still holds only addresses, names, and URLs.

## Verification plan

- **Automated:** the unit tests in each `src/probe/<method>.rs`; `tests/config_precedence.rs`
  for the key walk; `tests/store.rs` for the idempotent key rewrite; `tests/monitor_e2e.rs`
  for one transition per method with the fake; `tests/web.rs` and `tests/import.rs` for the
  prefixed-key round trip.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- **Manual:** `sudo cargo run -- monitor --target 1.1.1.1 --target tcp:1.1.1.1:443 --target dns:example.com@9.9.9.9 --target http:https://example.com --listen 127.0.0.1:8642`
  and a look at the page; then `cargo test --test probe_network -- --ignored` against real
  endpoints; then the container build (`docker build -t network-monitor:dev .`) to prove the
  static binary still links with `rustls`.
- **Rollback:** revert the commit and rebuild. The database keeps reading: keys are opaque
  text. Caveat to state in the release notes — an older binary writes ICMP targets under the
  bare IP again, so the same target can appear as two lanes until the newer binary is
  restored; a database backup taken before the upgrade removes even that.

## Open questions

- [x] `dns://name` without a server uses `getaddrinfo` rather than reading `/etc/resolv.conf`.
      Decided: yes — the only portable option, and it hides the resolver differences.
- [x] A truncated DNS response is a local error instead of a TCP retry. Decided: local
      error; a retry can come later without breaking the contract.
- [x] `tcp` accepts a host name and resolves it before connecting, a resolution failure
      being a local error. Decided: yes — the same rule as HTTP.
- [x] `expect_status` default is `200..=399`; a list of exact codes and the `2xx` class are
      accepted. Decided: yes, modelled as classes `[2, 3]` plus exact codes so the value
      round-trips through `--show-config`.
- [x] The per-target `interval_ms` applies only to that target's loop, not to the reload
      poll. Decided: yes — the poll stays one second globally.
- [x] Version `0.2.0`. Decided: minor — the configuration surface only grows.

## Implementation checklist

- [x] Step 1 — Probe contract around an endpoint
- [x] Step 2 — Target model, identity, config, CLI, store rewrite
- [x] CP-1 — checkpoint reported and confirmed
- [x] Step 3 — TCP prober
- [x] Step 4 — DNS prober
- [x] Step 5 — HTTP and HTTPS prober
- [x] CP-4 — checkpoint reported and confirmed
- [x] Step 6 — Page, query and archive with prefixed keys
- [x] Step 7 — Documentation, ADRs, examples, version
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

### Step 1 — probe contract

- **Decision:** the parameter types an `Endpoint` carries (`Scheme`, `RecordType`,
  `StatusMatch`, `Method`) live in `probe/mod.rs`, because they are the shape of the
  contract; each per-method module owns only behaviour.
- **Decision:** `Prober` is an enum, not a trait object of four transports: the dispatch is
  static and the monitor keeps holding `Arc<dyn Probe>` exactly as before.
- **Decision:** `Probe::set_timeout` is gone. The timeout is a request parameter, so the
  supervisor no longer mutates the prober on reload; each worker reads `target.timeout`.
- **Deviation (approved):** `tests/icmp_privileged.rs` is not in the affected-modules list
  but calls `probe(...)` directly, so its two call sites were adapted and the requested
  per-call-timeout test was added there as `#[ignore]`. The user approved the file.
- **Not executed:** the manual `sudo cargo run -- monitor …` acceptance run of step 1, since
  a raw socket needs privileges this environment does not grant; the ignored privileged
  tests cover the same path when run as root.

### Step 2 — target model and identity

- **Decision:** `TargetSpec` (what the file or the flag states, with `Option` timeout and
  interval) is separate from `Target` (defaults applied, `Duration` values). The CLI parses
  before the global defaults are known, so a single resolved type could not work.
- **Change of shape:** `StatusMatch` became `{ classes, codes }` instead of the enum written
  in step 1; the enum could not represent the default `200..=399` in the configuration
  form, and `--show-config` must round-trip.
- **Deviation:** `src/probe/http.rs` exists from step 2 with only `parse_url`, because the
  configuration has to validate a URL before the probe exists. Step 5 adds the request and
  the response to the same file; no other module grew.
- **Decision:** the legacy-key rewrite runs only on `Store::open_rw`, never on the read-only
  open the page uses (INV-10). A `serve` alone on an old database therefore shows the bare
  keys until a writable open normalises them.
- **Decision:** `resolve_target` and `find_target` also match `icmp://<needle>`, so a query or
  a page link written with the bare address keeps working.
- **Decision:** `import` canonicalises in `Row::parse`, so the replace-window bookkeeping and
  the stored rows agree on one key.
- **Tests adapted (all forced by the planned change, none weakened):**
  `tests/monitor_e2e.rs` (fixture and fake calls), `tests/config_precedence.rs`,
  `tests/import.rs` (canonical keys in the fixture plus the legacy alias),
  `tests/store.rs` (identity rewrite), `tests/cli.rs` (per-method `--show-config`),
  `tests/reload.rs` (method and timeout change without restart), and the `src/import.rs`
  unit test that asserted a bare key stayed bare.

### Step 3 — TCP prober

- **Decision:** `socket2` provides `connect_timeout` on a socket created by hand, which is
  what makes the interface-bound path possible without per-platform `select` code. The
  platform detail stays in `interface::bind_socket`, the only new function there.
- **Decision:** a refusal, a reset, and a timeout are `Down`; every other socket error is a
  `ProbeError`. The first attempt of this step classified the refusal as success and had to
  be corrected — the intermediate `Unreachable` enum keeps "the target said no" apart from
  "we could not ask".

### Step 4 — DNS prober

- **Deviation:** the system-resolver path treats *any* `getaddrinfo` failure as `Down`, not
  only `EAI_NONAME`. Measured on macOS: `to_socket_addrs` reports both "no such name" and
  "resolver unreachable" as `ErrorKind::Uncategorized` with no OS code, so the two cannot be
  told apart portably. A target that needs the distinction names its resolver and gets the
  RCODE-based answer. Recorded here rather than silently weakening INV-5.
- **Decision:** a truncated answer and a malformed answer are `ProbeError` (the plan's
  choice); only RCODE 0/3 produce `Down`.
- **Decision:** the UDP exchange takes a `SocketAddr`, so the tests can point it at a port of
  their own; the public endpoint still carries only the resolver address and port 53.

### Step 5 — HTTP and HTTPS prober

- **Decision:** TLS uses `rustls` with the `ring` provider and `webpki-roots`, configured once
  in a `OnceLock`; ALPN is `http/1.1` and verification cannot be disabled.
- **Decision:** the handshake is completed explicitly, so a certificate failure is classified
  as `Down` before any read; `InvalidData` therefore belongs with the target's answers.
- **Defect found while verifying:** `StreamOwned::write_all` alone does not send a TLS record;
  the request is flushed explicitly. Without it the probe waited for a reply to a request that
  had never left the process, and the manual HTTPS check failed.
- **Decision:** a socket read timeout surfaces as `WouldBlock` on Unix, so it is classified
  with `TimedOut`.
- **Decision:** the address of an `http` target is resolved with `to_socket_addrs`; a name that
  does not resolve is a local failure (the request never reached the endpoint).
- **Manual verification (executed):** `https://example.com/` answers `up` in 95 ms with the
  embedded roots, and a four-method run (`tcp`, `dns`, `http`, `https`) recorded one `up`
  transition per target with its round-trip time.

### Step 6 — page, query and archive

- **Decision:** one `encode` next to `decode` in `web/mod.rs`, escaping everything outside
  the unreserved set, so a key carrying `?`, `&`, or `#` survives both the lane link and the
  period links. `decode` already maps `+` to a space, so the encoder escapes `+` too.
- **Decision:** the CSV columns are untouched; only the `target` value now carries the scheme,
  and the import already canonicalises a bare address.
- **Discovery (pre-existing, fixed):** `DEVELOPER-GUIDE.md` claimed the import does not accept
  HTTPS because "the binary carries no TLS stack" and that labels and interfaces are not in
  the CSV, which the header has carried since ADR-0015. Both statements were corrected.
- A URL-shaped key was walked through the real page: the lane link is percent-encoded, the
  detail page opens from the encoded query, and the address is shown HTML-escaped.

### Step 7 — documentation, decisions, examples, version

- PRODUCT: FR-2, FR-3, FR-4, FR-13, FR-14, FR-18, FR-19 amended; FR-20 (TCP), FR-21 (DNS),
  FR-22 (HTTP), FR-23 (identity) added.
- ARCHITECTURE: component table, the `probe` contract with its per-method verdict rules, the
  data-model note on the canonical key, INV-3 and INV-5, the container note on embedded roots.
- GLOSSARY: `Method`, `Endpoint`, `Target key` added; `Target`, `Probe`, `Reachable`,
  `Unreachable`, `Timeout`, `Verdict`, `Target specification`, `Local error` reworded.
- ADR-0023 and ADR-0024 written, with the alternatives that were rejected and the consequences
  taken (Rust 1.75 kept, trust store ours, DNS system-resolver limit).
- DEVELOPER-GUIDE: layout tree, "Adding a probe type" rewritten around `probe/<method>.rs`, a
  new crate is stated as a decision that needs an ADR.
- README, `network-monitor.toml` (one example per method, per-target timeout), `Cargo.toml`
  (0.2.0, description).
- `--show-config` of the shipped example parses back to an identical configuration (verified by
  running it twice and diffing).

### Extension after the plan completed — the user manual (2026-11-14)

- **Requested by the user** after step 7 closed: "abbiamo un manuale da distribuire su come
  usare e configurare il prodotto?" — answered no, and the manual was created as an extension
  outside this plan's *Affected modules* (which is why it is recorded here and not in the
  scope list; a formal scope change would need `spec-plan`).
- **Created:** `docs/USER-GUIDE.md` — install (binary, container, QNAP), quick start,
  configuration reference for `icmp`/`tcp`/`dns`/`http`, command-line reference, the web page,
  operations (backup, import, the 0.1 → 0.2 identity rewrite, retention, logs, signals),
  troubleshooting, and the deliberate limits. It links to GLOSSARY/PRODUCT/ADR/DEVELOPER-GUIDE
  instead of redefining anything.
- **Wiring so it cannot drift silently:** a row in the README documentation map, a
  project-documents table in `AGENTS.md` outside the managed block (USER-GUIDE, the
  external-history import runbook, and `NOTES.md`, which was outside the map until now),
  and `docs.user_guide` in `.spec-workflow.yml`, so every later plan and every
  `spec-review` audits it.
- **Verified:** the documented commands were executed against a real database —
  `--show-config` for the four methods, `query events` by key, by label and by bare ICMP
  address, and an import of a legacy bare-address CSV that came back as `icmp://1.1.1.1`; the
  relative links and the two anchors the guide uses resolve. Chosen enforcement is procedural
  (no drift test), by the user's decision.
- **Also corrected:** the README *Status* paragraph still described an ICMP-only feature set.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-14
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(probe): support ICMP, TCP, DNS and HTTP targets` (split: the review
  findings are fixed first; this subject fits the final commit of the plan)
- **Quality gate**: tests ✓ (254 passed, 5 ignored) | lint ✓ (`cargo clippy --all-targets -- -D warnings`) | fmt ✓ (`cargo fmt --check`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/probe/tcp.rs:229` | `a_socket_bound_to_an_interface_still_connects` is racy: the accept thread returns and drops the listener immediately, so under load the client sees `POLLHUP` and the test panics on `.expect(...)`. Reproduced on the lib test binary: 4 failures in 40 runs at `--test-threads=32`; 0 in 60 runs of `probe::tcp` alone. A nondeterministic quality gate cannot back a PASS. Keep the listener (and the accepted stream) alive until the probe returns. | open |
| 2 | major | `src/probe/tcp.rs:110` (`classify`) | The same failure exposes a classification gap: socket2 reports a handshake that completed and was then closed by the peer as `Err` with no errno (`"cannot connect: no error set after POLLHUP"`), and `classify` maps it to `Local`, so a service that accepts and closes at once is never `up` and never produces a verdict, contrary to FR-20. Confirm the outcome with `Socket::peer_addr()` (or `SO_ERROR`) before classifying, and retry `ErrorKind::Interrupted` instead of calling it local. | open |
| 3 | major | `tests/probe_network.rs` | Listed under Affected modules and in the Verification plan as the ignored real-endpoint check, but the file does not exist and no Implementation Note records the drop. The functional evidence was gathered by hand instead (four-method run, HTTPS in the container); either create the file or record the deferral explicitly. | open |
| 4 | minor | `README.md` quick start | The minimal TOML still shows ICMP targets only; the plan asked for one example per method. The four methods are documented in `network-monitor.toml` and `docs/USER-GUIDE.md`, so this is a discoverability gap, not a contradiction. | open |
| 5 | nit | `src/probe/dns.rs:196` | `parse_response` does not compare the response identifier with the query's; unreachable in practice because each probe uses a fresh connected socket, but the codec is public to the module. | open |

### Gap analysis

- Goals: 9/9 achieved. Each method is one module behind the `Probe` contract, probeable in
  isolation (`cargo test --lib probe::<method>`), prefixed keys are the stored identity, legacy
  keys are rewritten once on writable open, per-target timeout/interval apply, TLS verifies
  against embedded roots, and the page/export/import/`query` speak the canonical key.
- Scope: in scope. Every changed file is listed under Affected modules; the two out-of-plan
  files are the approved extension (`docs/USER-GUIDE.md` + the `AGENTS.md` row and
  `.spec-workflow.yml` wiring recorded in Implementation Notes) and the plan itself.
- Deviations: all documented — hand-written DNS instead of hickory (ADR-0024), system-resolver
  `down` for an unresolvable name, URL-shaped keys appended to the `query --target` existing
  prefix list. Missing: the `tests/probe_network.rs` deviation (finding 3).
- Additions: `docs/USER-GUIDE.md` (user-approved), the `AGENTS.md` project-documents row,
  `.spec-workflow.yml`.
- Deferred: the manual `sudo cargo run -- monitor …` acceptance of step 1 (no `NET_RAW` in this
  environment) and the Windows raw-socket path; both recorded.
- Row-by-row: 1 ✓, 2 ✓, 3 ✓, 4 ✓ (16 new DNS tests, 14 HTTP, 5 TCP, plus config/store/web/e2e),
  5 ✓ (no test deleted, skipped, or weakened — the removed assertions are the API migration
  from `Ipv4Addr` to `Endpoint` and from a bare target to a canonical key, each replaced by a
  stronger assertion), 6 ✓ (13/13, no open question, notes present), 7 ✓ (PRODUCT FR-2/13/14/18/19
  amended, FR-20–23 added, ARCHITECTURE component table + INV-3/INV-5, ADR-0023/0024 with index
  rows, GLOSSARY terms, DEVELOPER-GUIDE "Adding a probe type", example TOML, README), 8 ✓ (no
  contradiction; the removed "no TLS stack" claim is replaced, not duplicated), 9 ✓, 10 ✗
  (finding 2, plus findings 1/3), 11 ✓ (no new `unsafe`, TLS verification not optional, no
  secrets, DNS response capped at 4 KiB with a 32-jump pointer guard, HTTP head capped at 8 KiB,
  percent-encoded keys), 12 n/a.

### Known limitations

- The DNS system-resolver path reports an unreachable resolver as `down` (FR-21, INV-5
  documented exception).
- No IPv6 target, no redirect following, no probe history: only transitions reach the database.

### Recommendations for future work

- Add `tests/probe_network.rs` as the ignored acceptance check, so the manual run leaves a
  repeatable artefact.
- Consider a `probe::tcp` test that drives a listener which accepts and closes at once, once
  finding 2 is fixed; it is the production shape that currently hides a verdict.

### Review round 1 — findings fixed (2026-11-14)

- **Finding 1 (racy test).** `src/probe/tcp.rs` gained the helper `serve_until_released`: the
  listener and the accepted stream stay alive until the probe has returned, released through a
  channel, so the peer can no longer close mid-handshake. The same shape was in
  `an_open_port_is_up` and was fixed with it. Evidence: 40 runs of the lib test binary at
  `--test-threads=32` and 20 runs at `--test-threads=64`, 0 failures, against 4 failures in 40
  runs before the fix.
- **Finding 2 (classification).** `connect` now builds one socket2 socket for both cases, so the
  std `TcpStream::connect_timeout` shortcut is gone and the interface-bound and the default path
  report the same way. The renamed `outcome(&socket, &error)` asks the socket first: a peer in
  `peer_addr()` means the handshake completed, which is `Up` even when the platform raises a
  hang-up with no error set. The refusal/timeout whitelist is unchanged, so INV-5 still holds.
  Regression test `a_socket_whose_peer_closed_after_the_handshake_is_up`: it fails when the
  `peer_addr()` check is disabled and passes with it. `docs/PRODUCT.md` FR-20 now states the
  behaviour.
- **Deviation from the review suggestion.** `ErrorKind::Interrupted` is **not** retried. The only
  interruption source is `connect`/`set_nonblocking` on a non-blocking socket, and retrying on
  that same socket is unsafe (its state is unspecified, and a second `connect` answers
  `EALREADY`); a retry needs a fresh socket plus a deterministic way to interrupt a connect,
  which the suite cannot produce, so the branch would ship untested. It stays a local failure —
  no verdict, no state change — as INV-5 requires, and the code comment and
  `a_refusal_is_the_answer_of_the_target_and_the_rest_is_ours` now state it.
- **Finding 3 (missing acceptance test).** `tests/probe_network.rs` created with five
  `#[ignore]` tests: a public port, a name resolved through a given server, a name resolved
  through the system resolver, `NXDOMAIN` as `down`, and HTTPS against the embedded roots.
  Executed: `cargo test --test probe_network -- --ignored` → 5 passed in 0.11 s.
- **Quality suite after the fixes.** `cargo fmt --check` ✓,
  `cargo clippy --all-targets -- -D warnings` ✓, `cargo test` ✓ 255 passed, 10 ignored.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-14
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(probe): support ICMP, TCP, DNS and HTTP targets`
- **Quality gate**: tests ✓ (255 passed, 10 ignored) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check --all-targets && cargo test`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/probe/tcp.rs` | `a_socket_bound_to_an_interface_still_connects` was racy: the acceptor dropped the listener, so under load the client saw a hang-up. Now `serve_until_released` holds listener and stream until the probe returns, in that test and in `an_open_port_is_up`. Re-measured on the current lib binary: 0 failures in 40 runs at `--test-threads=32` and 0 in 20 at `--test-threads=64` (4 in 40 before). | fixed in round 2 |
| 2 | major | `src/probe/tcp.rs` | A handshake that completed and was closed by the peer was classified `Local`, so an accept-and-close service was never `up`. `connect` now builds one socket2 socket for both paths and `outcome` asks the socket first (`peer_addr()`), keeping the refusal/timeout whitelist. New regression test `a_socket_whose_peer_closed_after_the_handshake_is_up` fails when the check is disabled and passes with it; FR-20 documents the behaviour. | fixed in round 2 |
| 3 | major | `tests/probe_network.rs` | The planned ignored acceptance file did not exist. Created with five tests and executed: `cargo test --test probe_network -- --ignored` → 5 passed in 0.11 s; the suite counts them as ignored by default. | fixed in round 2 |
| 4 | major | `docs/USER-GUIDE.md` | Not self-contained: the guide links the repository's internal documents, so it breaks when distributed alone — `GLOSSARY.md` and `PRODUCT.md` (lines 7–8), `DEVELOPER-GUIDE.md#build` (line 59), `DEVELOPER-GUIDE.md#container-and-qnap-deployment` (line 79), `ADR.md` + `PRODUCT.md` (lines 350–351). Replace every link with the statement it stands for: the term note, the two build commands, one sentence on how the configuration reaches the container, and the reason behind the deliberate limits. | open |
| 5 | minor | `docs/USER-GUIDE.md:22` | The opening summary is three paragraphs over seven lines; the guide contract asks for one paragraph of at most three lines. | open |
| 6 | minor | `README.md` quick start | The minimal TOML still shows ICMP targets only, though the plan asked for one example per method; `network-monitor.toml` and the user guide carry all four. | open |
| 7 | nit | `src/probe/dns.rs:196` | `parse_response` does not compare the response identifier with the query's; unreachable in practice, since each probe uses a fresh connected socket. | open |
| 8 | nit | `tech-plans/001-multi-method-targets.md` | Review round 1 suggested retrying `ErrorKind::Interrupted`; the implementer did not, with a recorded reason (a retry needs a fresh socket plus a deterministic way to interrupt a connect, which the suite cannot produce). Accepted as a deviation, not a defect. | closed |

### Gap analysis

- Goals: 9/9 achieved. Each method is one module behind the `Probe` contract and testable alone
  (`cargo test --lib probe::<method>`), prefixed keys are the stored identity, legacy keys are
  rewritten once on writable open, per-target timeout/interval apply, TLS verifies against the
  embedded roots, and the page, the export, the import and `query` all speak the canonical key.
- Scope: in scope. Every changed file is listed under Affected modules; out-of-plan files are the
  approved extension (`docs/USER-GUIDE.md`, its `AGENTS.md` row, `.spec-workflow.yml`) and the
  plan itself.
- Deviations: hand-written DNS instead of hickory (ADR-0024), the system-resolver `down` for an
  unresolvable name, URL-shaped keys appended to the `query --target` prefix list, and the
  `Interrupted` non-retry of finding 8 — all documented in Implementation Notes.
- Additions: `docs/USER-GUIDE.md` (user-approved), the `AGENTS.md` project-documents row,
  `.spec-workflow.yml`, and the five ignored real-endpoint tests.
- Deferred: the manual `sudo cargo run -- monitor …` acceptance of step 1 (this environment grants
  no `NET_RAW`) and the Windows raw-socket path; both recorded.
- Row-by-row: 1 ✓, 2 ✓, 3 ✓, 4 ✓, 5 ✓ (the adapted classification test keeps its assertions and
  the socket check is added, nothing removed), 6 ✓ (13/13, no open question), 7 ✓ (PRODUCT
  FR-20 updated with the TCP behaviour; the user guide needs no change for accuracy, only the
  self-containment of finding 4), 8 ✓, 9 ✓, 10 ✓, 11 ✓ (no new `unsafe`, TLS verification not
  optional, DNS and HTTP parsing bounded), 12 n/a, 13 ✗ (finding 4).

### Known limitations

- The DNS system-resolver path reports an unreachable resolver as `down` (FR-21, INV-5 documented
  exception).
- A connect interrupted by a signal is a local failure, never a verdict; it is not retried.
- No IPv6 target, no redirect following, no probe history: only transitions reach the database.

### Recommendations for future work

- Point the 40-run stress at the binary `cargo test --no-run` names: `target/debug/deps` keeps
  stale `network_monitor-*` executables, and the first one found can be an old build.
- Consider a `probe::tcp` test that drives a listener accepting and closing at once, so the
  production shape of finding 2 stays covered end to end.

## Review Notes

- **Round**: 3
- **Review date**: 2026-11-14
- **Verdict**: PASS
- **Suggested commit**: `feat(probe): support ICMP, TCP, DNS and HTTP targets`
- **Quality gate**: tests ✓ (255 passed, 10 ignored) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check --all-targets && cargo test`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `docs/USER-GUIDE.md` | Round-2 finding 4: the guide linked `GLOSSARY.md`, `PRODUCT.md`, `DEVELOPER-GUIDE.md#build`, `DEVELOPER-GUIDE.md#container-and-qnap-deployment` and `ADR.md`, so it broke when distributed alone. Every link is replaced by the statement it stood for (term note, the two build commands, the `--build-arg CONFIG` / compose `configs` route, and the reason behind the deliberate limits). Evidence: `grep -n "](./" docs/USER-GUIDE.md` finds nothing. | fixed in round 3 |
| 2 | minor | `docs/USER-GUIDE.md` | Round-2 finding 5: the opening summary was three paragraphs over seven lines. It is now one paragraph of exactly three lines (`What the tool does`, lines 23–25), keeping the target, the key and the three states. | fixed in round 3 |
| 3 | minor | `README.md` | Round-2 finding 6: the quick-start TOML showed ICMP only. It now carries one target per method; the block was extracted and parsed (`--show-config`) to five well-formed targets, one of each method plus the second ICMP one. | fixed in round 3 |
| 4 | major (scope) | `docker-compose.yml` | Two `http` targets (`https://www.xthema.it`, `https://www.h7labs.it`, `interval_ms = 30000`) were added at the user's request in this session, although the plan listed any change to that file as a non-goal. Treated as an approved extension: the non-goal is amended, the addition is recorded here. Verified with `docker compose -f docker-compose.yml config -q` ✓, with `--show-config` over the extracted `content` (both targets, interval 30 s) ✓, and with a live run of the two URLs (`unknown -> up`, ~181 ms each) ✓. | approved addition (closed) |
| 5 | nit | `src/probe/dns.rs:196` | `parse_response` still does not compare the response identifier with the query's; unreachable in practice, because each probe uses a fresh connected socket. Reported in rounds 1 and 2, left as a nit. | open |

### Gap analysis

- Goals: 9/9 achieved. Four methods behind one contract, each testable alone (`probe::icmp` 10,
  `probe::tcp` 7 + 1 ignored, `probe::dns` 16, `probe::http` 14 + 1 ignored), prefixed keys as the
  stored identity, the idempotent legacy rewrite, per-target timeout and interval, embedded TLS
  roots, and page/export/import/`query` on the canonical key.
- Scope: in scope. Changed files are the plan's modules plus the approved additions: the
  hand-made user guide (from round 2), the `AGENTS.md` row, `.spec-workflow.yml`, this plan, and —
  decided in this round — `docker-compose.yml`.
- Deviations: hand-written DNS instead of hickory (ADR-0024), the system-resolver `down` for an
  unresolvable name, URL-shaped keys appended to the `query --target` prefix list, and the
  `Interrupted` non-retry. All were already recorded in Implementation Notes.
- Additions: the two website targets in the compose configuration (user request, round 3),
  `tests/probe_network.rs`, `docs/USER-GUIDE.md`.
- Deferred: the manual `sudo cargo run -- monitor …` acceptance of step 1 (this environment
  grants no `NET_RAW`) and the Windows raw-socket path; both recorded.
- Row-by-row: 1 ✓, 2 ✓, 3 ✓, 4 ✓, 5 ✓ (no test deleted, skipped or weakened), 6 ✓ (13/13, no open
  question), 7 ✓ (PRODUCT FR-20–23, ARCHITECTURE INV-3/INV-5, ADR-0023/0024 with index rows,
  GLOSSARY `Target key`, DEVELOPER-GUIDE, README, example TOML, and the guide's configuration
  tables cross-read against `src/config.rs`), 8 ✓, 9 ✓, 10 ✓, 11 ✓ (no new `unsafe`, TLS
  verification not optional, DNS 4 KiB and HTTP 8 KiB caps, percent-encoded keys), 12 n/a,
  13 ✓ (accuracy, completeness, self-containment, three-line summary).

### Known limitations

- The DNS system-resolver path reports an unreachable resolver as `down` (FR-21, documented
  exception).
- A connect interrupted by a signal is a local failure, never a verdict; it is not retried.
- No IPv6 target, no redirect following, no probe history: only transitions reach the database.
- The two website targets in the compose configuration depend on outbound HTTPS from the NAS and
  on the global 2 s timeout; measured round trips were ~181 ms, but a slow path could report a
  false `down`. Raise `timeout_ms` for those targets if that happens.

### Recommendations for future work

- Commit `docker-compose.yml` separately, so the deployment change does not hide inside the
  probe feature: `feat(docker): monitor the two websites from the compose config`.
- Compare the DNS response identifier with the query's (finding 5), the last open nit.
- Consider a `probe::tcp` test that drives a listener accepting and closing at once, so the
  production shape of round-1 finding 2 stays covered end to end.
