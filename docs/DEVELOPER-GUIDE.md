# Developer Guide — network-monitor

## Tech stack

- **Language:** Rust, stable toolchain, edition 2021, minimum supported version 1.75.
- **Compile-time crates:** `clap` (derive) for the command line, `serde` and `toml` for the configuration, `rusqlite` with the `bundled` feature for SQLite, `socket2` for socket options, `libc` on Unix and `windows-sys` on Windows for interface binding, `thiserror` for typed errors.
- **Test crates:** `assert_cmd`, `predicates`, `tempfile`.
- **Storage:** SQLite 3 compiled into the binary, WAL journal mode.
- **Release artifacts:** one static executable per platform — `x86_64-unknown-linux-musl`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
- **Packaging:** a multi-stage Docker image for Linux, deployed to QNAP through Container Station.

## Prerequisites

- Rust 1.75 or newer through `rustup` (`rustup toolchain install stable --component rustfmt,clippy`).
- The musl target and a musl linker for static Linux builds: `rustup target add x86_64-unknown-linux-musl`, plus `musl-gcc` (Debian/Ubuntu: `apt install musl-tools`).
- Docker 24 or newer, only to build and test the container image.
- Optional: the `sqlite3` command-line client for ad-hoc inspection of the database.
- Raw socket privileges for manual runs: `sudo` on macOS and Linux, an elevated shell on Windows. The test suite needs none.

## Setup

1. Install the toolchain: `rustup toolchain install stable --component rustfmt,clippy`.
2. Install the musl target: `rustup target add x86_64-unknown-linux-musl`, and install `musl-gcc` with your package manager.
3. Fetch the dependencies: `cargo fetch`.
4. Build: `cargo build --release`.
5. Run the suite: `cargo test`.
6. Optional, container: `docker build -t network-monitor:dev .`.
7. Optional, manual run: create `network-monitor.toml` from the example in [README.md](../README.md#quick-start), then `sudo cargo run -- monitor --config ./network-monitor.toml`.

Verify the setup before starting work: the build command below must succeed on a
clean checkout.

## Repository layout

```text
network-monitor/
├── Cargo.toml            crate metadata, dependencies, MSRV
├── Cargo.lock            pinned dependency graph, committed
├── Dockerfile            multi-stage static build for the container image
├── network-monitor.toml  example configuration, kept in sync with the defaults
├── src/
│   ├── main.rs           entry point, subcommand dispatch, exit codes
│   ├── cli.rs            command-line surface, one flag per configuration key
│   ├── config.rs         defaults, TOML loading, precedence, validation
│   ├── monitor/
│   │   ├── mod.rs        interval loop per target, shutdown handling
│   │   └── state.rs      transition decision from the previous state and the verdict
│   ├── probe/
│   │   ├── mod.rs        Probe contract, Verdict, ProbeError
│   │   ├── icmp.rs       raw ICMP echo request and reply matching
│   │   ├── interface.rs  platform interface binding (Linux, macOS, Windows)
│   │   └── fake.rs       deterministic prober for tests
│   ├── store/
│   │   ├── mod.rs        connection, write transaction, retention, read queries
│   │   └── schema.rs     DDL, schema version, migrations
│   └── query/
│       ├── mod.rs        filters and read path
│       └── render.rs     table and JSON rendering
├── tests/
│   ├── config_precedence.rs  every key, file and flag, in both directions
│   ├── monitor_state.rs      transitions, no duplicates, local errors
│   ├── store.rs              schema, concurrent read, retention
│   └── cli.rs                exit codes and query output
├── docs/                 product, architecture, decisions, glossary, developer guide
├── .github/              commit and pull request instruction contracts
└── .vscode/              editor settings that register those contracts
```

## Build

```bash
cargo build --release
cargo build --release --target x86_64-unknown-linux-musl
```

## Run

```bash
cargo run -- --help
cargo run -- monitor --config ./network-monitor.toml
cargo run -- query state --config ./network-monitor.toml
cargo run -- query events --target 1.1.1.1 --since 24h --config ./network-monitor.toml
cargo run -- --show-config --config ./network-monitor.toml
```

## Test

```bash
cargo test
cargo test --test monitor_state
cargo test --test store
```

<!-- skill:block:devguide-test-rules section="## Test" -->
Fixed rules that apply to every change:

- Every change ships with the tests of the part it changes, in the same commit. A
  change without tests is not done.
- New behaviour needs a test that fails without the change and passes with it; a
  defect fix needs a regression test that reproduces the defect.
- Test each module independently: substitute the other modules at their contract
  boundary instead of starting them. A test that needs the whole system to run is a
  signal that the module is not decoupled enough.
- Never delete or weaken a test to make the build green; fix the cause.
<!-- skill:end:devguide-test-rules -->

## Lint and format

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## Extending the project

### Adding a configuration option

1. Add the field to `Config` with its default expressed in `Default`.
2. Add the matching `clap` argument in `cli.rs`; there is no option without a flag (INV-2).
3. Apply the merge in `config.rs`, which is the only place that knows the precedence order.
4. Document the key in `network-monitor.toml`, in the README example, and in [PRODUCT.md](./PRODUCT.md) when it changes behaviour.
5. Test three cases: value from the file only, value from the flag only, flag overriding the file.

### Adding a probe type

1. Define the type in `probe/`, implement the `Probe` contract, and return the existing `Verdict` variants; do not add a variant that means "local failure".
2. Keep the platform-specific socket work in `probe/interface.rs` so the rest of the code stays portable.
3. Select it in the composition step, from configuration, without touching `monitor`.
4. Add a fake-driven test of the decision logic and one manual acceptance test with real privileges.

### Changing the schema

1. Append a migration step in `store/schema.rs` and bump `schema_meta.version`; never edit an applied migration.
2. Extend the read path in `store` and the rendering in `query` in the same change.
3. Update the data model in [ARCHITECTURE.md](./ARCHITECTURE.md) and add an ADR when the change is hard to reverse.
4. Test the migration against a fixture database of the previous version.

### Adding a query output

1. Add the filter to `QueryFilter` and the rendering to `query/render.rs`.
2. Keep stdout for data and stderr for diagnostics, so the output stays pipeable.
3. Test the exact output of the new form, including the empty result.

### Adding a test

1. Put the test next to the module for unit-level checks and in `tests/` for cross-module contracts.
2. Never open a raw socket in a test: use `probe::fake`, and a temporary database from `tempfile`.
3. Keep the test deterministic: inject the clock or assert on monotonic ordering instead of wall time.

<!-- skill:block:devguide-design-rules section="## Extending the project" -->
### Design rules

Fixed rules for adding anything to this project:

- Place the new code in the module that owns the responsibility. If no module owns
  it, add a module with one responsibility — do not grow an existing module for an
  unrelated concern.
- Depend on the other modules' public contracts only; never reach into their
  internals, and never introduce a dependency cycle.
- Start with the simplest implementation that satisfies the requirement (KISS). No
  speculative abstraction, extension point, or configuration for a need that is not
  there yet.
- Add the tests of the new code in the same change, at the module boundary, with the
  other modules replaced by fakes.
<!-- skill:end:devguide-design-rules -->

<!-- skill:block:devguide-git-standards -->
## Git Standards

These rules are mandatory for every contributor, human or AI. They define how work
enters this repository.

Automated agents never commit or push: they prepare the change and the commit
message, and only the user approves and saves it in Git (see
[AGENTS.md](../AGENTS.md#git-user-approved-changes-only)).

### Commit messages

Every commit message follows Conventional Commits and stays concise: one logical
change per commit, an imperative subject of at most 72 characters, no trailing
period, a body only when the reason is not obvious.

The complete contract — format, allowed types and scopes, subject and body rules,
footers, breaking changes, and accepted and rejected examples — lives in
[.github/copilot-commit-message-instructions.md](../.github/copilot-commit-message-instructions.md).
That file is the single source of truth for message format: it is what an assistant
reads when it drafts a commit message, and it is registered for that purpose in
[.vscode/settings.json](../.vscode/settings.json). Read it before writing a message;
do not restate it here.

### Branches and integration

- `main` is always releasable; never commit directly to it.
- Branch from `main` with `feat/<slug>`, `fix/<slug>`, `docs/<slug>`, or
  `chore/<slug>`; keep branches short-lived and rebase on `main` before review.
- One pull request per concern. Squash on merge so `main` keeps one commit per
  change, with the pull request title in Conventional Commits form.
- Long-running work uses feature flags or stacked pull requests, not long-lived
  branches.

### Pull requests

The title follows the commit message format, because a squash merge turns it into the
commit subject. The description states the problem, the approach, the verification, and
the deliberate exclusions. The contract is in
[.github/copilot-pull-request-instructions.md](../.github/copilot-pull-request-instructions.md).

### Language

Every artefact is written in English: commit messages, branch names, code,
identifiers, comments, tests, documentation, issue text, and user-facing strings.
<!-- skill:end:devguide-git-standards -->
