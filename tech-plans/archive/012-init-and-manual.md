---
title: Add init and manual subcommands to simplify first use
date: 2026-11-17
status: done
type: feature
impact: medium
area: cli, configuration template, self-documentation, documentation
version_bump: minor
tags: [init, manual, cli, scaffold, markdown, offline, documentation]
depends_on: []
branch_recommended: false
---

## Summary

`network-monitor init <FOLDER>` creates a new folder, or fills an existing empty one, with a
fixed `network-monitor.toml`, a `data/` directory for the SQLite database, and a `logs/`
directory for the event log, then prints the three commands that check and start it. The generated
configuration has one unprivileged TCP target, a commented ICMP example, and an enabled
size-rotated `logfile` channel; it contains no secret.

The command has no options of its own, reads no configuration, opens no database, starts no
probe, and installs no service. An existing non-empty folder is refused, so nothing is overwritten. The
folder is the unit of deployment: the operator edits the TOML and runs the monitor from that
folder. A later `install` verb can reuse the same template for a system service; this plan
does not add one.

The same plan adds `network-monitor manual`, with `man` as a visible alias, which prints the
user guide embedded at build time as Markdown to stdout. It reads no configuration, starts
nothing, and adds no dependency. When stdout is a terminal it prints one hint to stderr; when
piped, stdout is pure Markdown, so `manual | less -R` and `manual | mdcat --ansi | less -R`
work. A closed stdout pipe is a quiet success. The guide is exactly the `docs/USER-GUIDE.md`
of the build, so the binary carries no stale or duplicated manual.

## Why

- A first run needs a configuration file, a predictable database location, and a writable
  directory for a log file. The binary creates the database and its parent, but it does not
  create the configuration or the log directory; the two missing pieces are easy to get wrong,
  and they fail at different moments (config parse, channel open).
- The repository ships `network-monitor.sample.toml`, but it is a reference, not a scaffold:
  its `database = "./tmp/network-monitor.db"` is a reference path, it lists several targets,
  and several of them need `root`/`CAP_NET_RAW`. Copying it is not a first run.
- `Config::resolve` resolves `database` against the process working directory, not against the
  configuration file. A relative path in a config launched from elsewhere therefore puts the
  database somewhere else; the scaffold fixes a convention (`data/events.db`) and prints `cd`
  so the location is predictable. `Store::open_rw` creates the database's parent, so `data/` is
  a convention, not a hard requirement.
- A `logfile` channel creates its file but never its directory (`FR-29`); without `logs/`, the
  first start fails while validating the channel. A “log file ready” in a scaffold must
  therefore be a channel plus its directory, not an empty file the process never opens.
- The diagnostic log is stderr by design and has no `--log-file` (`ADR-0035`). A generated
  `logs/events.log` must be the event log of a `logfile` channel; calling it a diagnostic log
  would promise something the tool does not do.
- An ICMP target without privilege is a local probe error, not a stored `down` (`INV-5`), so a
  scaffold that defaults to ICMP leaves the page `unknown` and the log repeating the error. The
  active target must be a method that needs no privilege.
- `check` verifies a configuration but requires one. `init` is the missing zero-to-check step.
  Without it every user repeats the same `mkdir`/TOML/log setup, and the sample invites a
  copy-paste that is not a clean first run.
- The out-of-scope line of `docs/PRODUCT.md` and the “no installer” NFR do not block this: a
  command that writes into a folder the user names is not an installer, needs no privilege, and
  leaves the binary runnable with nothing installed.

- The product is a single binary for appliances, NAS devices, and containers, where the
  repository and the website may not be present. The user guide is the standalone manual;
  embedding it makes the binary self-documenting offline.
- The guide is 1,009 lines / 54 KB. The release binary is 7.0 MB against a 15 MB footprint
  limit, so the size cost is negligible, and the binary already embeds HTML, CSS, JS, OpenAPI,
  and Swagger UI with `include_str!` (`src/web/templates.rs`, `src/web/api/docs.rs`,
  `src/web/api/openapi.rs`).
- `--help` covers the flags and subcommands but not the configuration keys, operations, or
  troubleshooting. The guide does; a `manual` command is the shortest path to it on a host
  with no checkout.
- A true mdcat-like renderer is a larger feature: the guide has 136 table rows, 30 code
  blocks, 39 lines longer than 120 characters, and a maximum line of 537 characters. A
  half-renderer that ignores table wrapping is worse than raw Markdown; a proper renderer
  needs a parser and renderer dependency, which conflicts with the project's minimal
  dependency principle.
- A curated short manual would duplicate content and drift from the user guide; embedding the
  existing file keeps one source of truth. A native `man network-monitor` page would need a
  second format and an installation step, reopening the installer question the project
  deliberately avoids.

Evidence: `network-monitor.sample.toml` (`database = "./tmp/network-monitor.db"`, several
targets), `src/config.rs` (`DEFAULT_DATABASE`, `Config::resolve`, `FileNotification`),
`src/check.rs` (`check_database` treats a missing database as a warning),
`src/store/mod.rs` (`Store::open_rw` creates the database's parent),
`src/notify/logfile.rs` (opens and creates the file, never its directory),
`docs/USER-GUIDE.md` (“The diagnostic log is not a file of the tool”, 1,009 lines), `Cargo.toml`
(no Markdown crate), `src/cli.rs` (`Command` enum, `disable_help_subcommand = true`),
`src/main.rs` (dispatch), `src/web/templates.rs` and `src/web/api/docs.rs` (`include_str!`), and
`docs/PRODUCT.md` (`INV-5`/`FR-6` on the first probe and local errors, and the single-binary
portability requirement).

## Goals

- `network-monitor init <FOLDER>` exits `0` for a missing or empty folder and creates exactly
  `<FOLDER>/network-monitor.toml`, `<FOLDER>/data/`, and `<FOLDER>/logs/`; it writes nothing
  outside that folder.
- The generated `network-monitor.toml` parses through `config::parse` and resolves through
  `Config::resolve` with no override. It has one active target, `tcp://1.1.1.1:443`, with no
  interface, and the defaults `interval_ms = 5000`, `timeout_ms = 2000`,
  `database = "data/events.db"`, and `retention_days = 0`.
- The generated file declares one enabled `logfile` channel named `events`, at
  `logs/events.log`, with `stabilize_seconds = 0`, `max_bytes = 10485760`, and `max_files = 5`;
  the root `notify` list enables it for global events and every target.
- From the folder, `network-monitor check --config network-monitor.toml` exits `0` (the missing
  database is a warning) and creates `logs/events.log`; `data/events.db` does not exist until
  the monitor starts.
- The active target is not ICMP and has no interface, so the monitor starts without
  `root`/`CAP_NET_RAW`; a host with outbound TCP 443 answers `up`. The ICMP example is present
  only as comments and carries the privilege note.
- `init` refuses a path that is a file, a folder that contains any entry, and any invocation
  with a configuration flag (`--config`, `--interval-ms`, `--timeout-ms`, `--database`,
  `--retention-days`, `--target`, `--show-config`). Every refusal exits non-zero, names the
  reason, and writes nothing.
- `init` never reads `./network-monitor.toml`, never opens the database, never starts a probe,
  and never binds a socket.
- A second `init` on the same folder fails and leaves the first `network-monitor.toml`
  byte-identical.
- The command, the folder layout, and the event-log choice are documented in `PRODUCT`,
  `USER-GUIDE`, `README`, `ARCHITECTURE`, `DEVELOPER-GUIDE`, `GLOSSARY`, `ADR`, and the landing
  page; `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes.
- Existing behaviour is unchanged: no new configuration key and no change to `Config::resolve`,
  `check`, `monitor`, `probe`, `store`, `web`, `notify`, or `import`. The binary remains one
  statically linked executable with no runtime dependency.
- `network-monitor manual` exits `0` and writes the exact bytes of the embedded
  `docs/USER-GUIDE.md` to stdout; `network-monitor man` is a visible alias with byte-identical
  stdout.
- `manual` reads no configuration: it succeeds in a directory containing an invalid
  `network-monitor.toml`, and it never parses `--target`; `--config`, `--show-config`,
  `--interval-ms`, `--timeout-ms`, `--database`, `--retention-days`, or a non-empty `--target`
  exits non-zero with `manual takes no arguments; it does not read a configuration`.
- `manual` creates no file, opens no database, starts no probe, and binds no socket.
- When stdout is a terminal, exactly one hint is written to stderr; when piped, no hint is
  written and stdout is pure Markdown. A closed stdout pipe exits `0`; any other write error
  exits non-zero with a message naming standard output.
- The top-level `--help` lists `manual` and its `man` alias; `manual --help` and `man --help`
  both show the command help.
- The user guide documents the command and the pipelines; `PRODUCT` gains `FR-34`;
  `ARCHITECTURE`, `DEVELOPER-GUIDE`, `GLOSSARY`, `ADR` (`ADR-0037`), `README`, and the landing
  page are updated; the binary grows by about 54 KB and adds no crate.

## Non-goals

- No OS-level service installation, scheduling, systemd/launchd/Windows service, `/etc` or
  `/var/lib` writes, user creation, or `systemctl`. A later `install` verb is a separate plan.
- No options on `init`: no `--target`, `--listen`, `--database`, `--force`, `--dry-run`,
  `--absolute`, `--no-log`, or template customization. The folder is the only argument.
- No update or merge: the generated configuration is user-owned; `init` never overwrites or
  edits it. To start over, remove the folder and run `init` again.
- No pre-created database and no pre-created log file. `init` creates `data/` and `logs/`; the
  monitor creates the database, and `check` or the monitor creates the log file through the
  channel.
- No new configuration key, no `--log-file`, and no change to the diagnostic log on stderr.
- No fake log lines and no generated history; `simulate` remains the demo mode.
- No generated `README`, `.gitignore`, run script, Makefile, unit file, or compose file.
- No changes to `network-monitor.sample.toml`, `docker/config.toml`, `Dockerfile`, or
  `docker-compose.yml`.
- No Windows/macOS-specific code. The implementation uses `std::fs` and `std::path` only; no CI
  matrix exists, so the integration tests run on the host platform.
- No Markdown renderer, ANSI styling, table reflow, wrapping, syntax highlighting, images, or
  OSC 8 hyperlinks; the raw Markdown is the output.
- No pager or subprocess: `less`, `more`, and `mdcat` are optional tools the user pipes to;
  the binary never spawns them.
- No native `man network-monitor` page, no roff, and no installation step.
- No section argument, `--list`, search, or filter; `manual` prints the whole guide.
- No curated duplicate manual and no generated summary.
- No new flag for `manual` and no web route for the guide.
- `init` is unchanged apart from sharing the configuration-flag guard with `manual`; its report
  does not mention `manual`.

## Affected modules

**Source**

- `src/init.rs` *(new)* — the `TEMPLATE` constant, `run`, `Report`, `InitError`, the folder
  checks, the deterministic renderer, and unit tests.
- `src/manual.rs` *(new)* — `USER_GUIDE` with `include_str!("../docs/USER-GUIDE.md")`,
  `write_guide`, `run`, `ManualError`, the terminal hint, broken-pipe handling, and unit
  tests.
- `src/lib.rs` — `pub mod init;` and `pub mod manual;`.
- `src/cli.rs` — `Command::Init { folder: PathBuf }` with a positional `FOLDER`;
  `Command::Manual` with `#[command(visible_alias = "man")]`; a shared
  `Cli::uses_configuration()` helper; parse and help tests for both.
- `src/main.rs` — dispatch `Command::Init` and `Command::Manual` before
  `Cli::overrides`/`load_settings`, reject the configuration flags through the shared guard,
  add `AppError::Init`/`AppError::Manual`, print the init report or the guide, and keep the
  no-subcommand message current.

**Tests**

- `tests/init.rs` *(new)* — end-to-end contracts: folder creation, `check` acceptance, refusal
  cases, no default-config read, path with spaces, second run.
- `tests/manual.rs` *(new)* — output, alias, no configuration read, flag rejection, no files,
  help listing, and a Unix broken-pipe case.

**Documentation / configuration**

- `docs/PRODUCT.md` — the `init` and `manual` bullets in “In scope”, one in “Out of scope”,
  and `FR-33` plus `FR-34` in “Functional behaviour”.
- `docs/USER-GUIDE.md` — a new “From an empty folder” subsection under “Install”; rows in the
  “Command line > Subcommands” table for `init` and `manual`; a manual pipeline paragraph;
  one bullet in “Limits and deliberate choices”; one sentence in “Rotate the logs” about the
  scaffolded event log and the diagnostic log.
- `README.md` — the init block in “Quick start” and one line pointing at `manual`.
- `docs/ARCHITECTURE.md` — section 1 (short-lived command list), section 5 (component table
  and a `### init` plus a `### manual` contract), section 7 (a “Scaffolded folder” bullet and
  one sentence that the binary carries its manual).
- `docs/DEVELOPER-GUIDE.md` — `src/init.rs` and `src/manual.rs` in “Repository layout”; one
  note that `docs/USER-GUIDE.md` is embedded, so `manual` is rebuilt with the binary.
- `docs/GLOSSARY.md` — `Init` and `Manual` terms.
- `docs/ADR.md` — index rows and `ADR-0036` plus `ADR-0037`.
- `site/index.html` — a “From an empty folder” snippet and a “Read the manual” snippet in
  “Quick start”.

No change: `network-monitor.sample.toml`, `docker/config.toml`, `Dockerfile`,
`docker-compose.yml`, `Cargo.toml`, and `Cargo.lock` (no new dependency; the version bump is
applied by `spec-close`).

## Architecture impact

Two short-lived components join the executable beside `check`, `import`, `query`, `serve`, and
`simulate`.

`init` depends only on `config` for the template's parse/resolve validation and on
`std::fs`/`std::path` for the folder; it does not depend on `probe`, `store`, `web`, `notify`,
or `monitor`, and it adds no thread, socket, database connection, or crate.

`manual` depends on `std` and the existing `thiserror` only: `include_str!` for the guide,
`std::io` for stdout/stderr, and `std::io::IsTerminal` for the hint. It depends on no other
component and adds no thread, socket, database connection, or crate.

Data ownership does not change: `init` creates an empty `data/` directory, but `store` remains
the only component that opens or creates the database. No invariant changes: `INV-2` gains no
key, `INV-5` sees no probe, `INV-8` gains no dependency, `INV-10`/`INV-11` see no database or
listener, `INV-13` is untouched, and `INV-15` sees no configuration write outside the new
folder. `manual` is a read-only presentation surface.

`docs/ARCHITECTURE.md` must state the new components, their contracts, and the scaffolded-folder
deployment shape in sections 1, 5, and 7.

## Decision-record impact

A new decision record is required: `ADR-0036 — Scaffold a local setup with an init verb, not an
installer`, with its row in the ADR index. It records:

- `init <FOLDER>` is one-shot and takes no flags; it accepts a missing or empty folder and
  refuses anything else, so it never overwrites an operator's file.
- The template is fixed: one unprivileged TCP target, a commented ICMP example, `data/` and
  `logs/`, and an enabled size-rotated `logfile` channel. The configuration stays the single
  source of truth; `init` is not a configuration UI or updater.
- The event log is a `logfile` channel; the diagnostic log stays on stderr and gains no
  `--log-file`, consistent with `ADR-0035`.
- The folder is run from its own directory; relative paths are not rewritten, and the default
  configuration lookup (`FR-1`) does not change.
- `init` is not an installer: it writes only inside the folder the user names. A later `install`
  may reuse the template for `/etc` and a supervisor, but that is a separate decision.

The record's rejected alternatives are: documentation-only or copying the sample (the sample is
not a clean first run); a full OS installer (a separate decision, and this is not one);
`--target`, `--listen`, or `--force` (they duplicate the configuration file or weaken the
one-shot contract); a diagnostic log file or a `2>>` recipe (rotation belongs to the supervisor,
and `ADR-0035` keeps diagnostics on stderr); a generated run script (no shell script belongs in
the execution path); and defaulting to `.` (the command's purpose is to create a folder, and `.`
can be a non-empty repository).

A second decision record is required: `ADR-0037 — Embed the user guide and print it as Markdown,
without a renderer`, with its row in the ADR index. It records:

- The user guide is embedded with `include_str!("../docs/USER-GUIDE.md")`; the binary and the
  distributed guide are the same file at build time.
- `manual` prints the raw Markdown to stdout; `man` is a visible alias. When stdout is a
  terminal, one hint goes to stderr; otherwise stdout is pure for pipelines.
- The command reads no configuration, starts nothing, and treats a closed stdout pipe as a
  quiet success.
- No renderer, pager, subprocess, native man page, or curated duplicate is added.
- The size cost is about 54 KB, against a 15 MB footprint limit.

Rejected alternatives: an mdcat-like renderer (a parser/renderer dependency plus table and
wrapping code); a curated short manual (duplicates the guide and drifts); a native roff man page
(a second format and an installation step); spawning a pager (an external process and platform
differences); printing the guide only when stdout is a terminal (breaks pipelines).

It does not amend `ADR-0002`'s “no installer”: the binary still runs with nothing installed,
`init` only writes a folder, and `manual` only prints the embedded text. The `init` record
applies `ADR-0035` and does not duplicate the diagnostic log; the `manual` record adds no
runtime dependency and no installer.

## Proposed solution

### Step 1 — Add the `init` module and its fixed template

Create `src/init.rs` with:

- Constants for the layout: `DATA_DIR = "data"`, `LOG_DIR = "logs"`,
  `DATABASE = "data/events.db"`, `EVENT_LOG = "logs/events.log"`, and the `TEMPLATE` string.
  Reuse `config::DEFAULT_CONFIG_FILE` for `network-monitor.toml`. Make the path constants and
  `TEMPLATE` `pub` so the component contract, the tests, and `-D warnings` can name them
  without a dead-code exemption.
- `pub struct Report` with the `folder`, `config`, `data`, and `logs` paths and a `render()`
  that prints the created paths and the next commands. Follow the `check::Report` pattern: the
  module returns the report, `main` prints it.
- `pub enum InitError` with `thiserror`: `NotADirectory { path }`, `NotEmpty { path }`,
  `Read { path, source }`, `Create { path, source }`, `Write { path, source }`, and
  `Template { reason }`. The messages are:
  - `"<path> is not a directory"`
  - `"<path> is not empty; choose a new or empty folder"`
  - `"cannot read <path>: <source>"`
  - `"cannot create <path>: <source>"`
  - `"cannot write <path>: <source>"`
  - `"the generated configuration is invalid: <reason>"`
- `pub fn run(folder: &Path) -> Result<Report, InitError>`:
  1. Validate the template with `config::parse(TEMPLATE, Path::new(config::DEFAULT_CONFIG_FILE))`
     and `Config::resolve(Some(file), Overrides::default())`; map any error to
     `InitError::Template`. This is an internal guard, not a user input.
  2. If the path exists: it must be a directory, and `read_dir` must return no entry; otherwise
     return `NotADirectory`/`NotEmpty`/`Read` before creating anything.
  3. If the path does not exist, `create_dir_all(folder)` so a nested path works.
  4. Create `data/` and `logs/` with `create_dir`.
  5. Write the template to `network-monitor.toml` with `OpenOptions::create_new(true)` so a
     file that appears between the check and the write is never clobbered.
  6. Return the `Report`.
  The steps are ordered so the template is known valid before the folder is touched, and the
  configuration is the last file written: a failure before it leaves a folder without a
  configuration, never a configuration that disagrees with the code.

`data/` is a convention rather than a requirement — `Store::open_rw` would create it on the
first monitor start — but creating it here makes the folder layout complete and keeps the
database beside the configuration. `logs/` is required: the `logfile` channel creates its file
and never its directory. A failure after the directories exist leaves an empty `data/`/`logs/`
and no configuration; the error names the path, and the operator can remove the folder. That is
the only partial state, and it never leaves a file that looks initialized.

The template is one `const &str`, with no interpolation, so it is deterministic and testable.
The paths are relative on purpose: the header tells the operator to run from the folder, and
path rewriting is a non-goal.

```toml
# network-monitor.toml — generated by `network-monitor init`.
# Edit this file; `init` never overwrites it.
# Run from this folder:
#   network-monitor check --config network-monitor.toml
#   network-monitor monitor --config network-monitor.toml
#
# The diagnostic log is written to stderr. The event log is `logs/events.log`,
# enabled as a `logfile` channel below and rotated by size.

interval_ms = 5000
timeout_ms = 2000
database = "data/events.db"
retention_days = 0

notify = ["events"]

[[notification]]
name = "events"
type = "logfile"
filename = "logs/events.log"
stabilize_seconds = 0
max_bytes = 10485760
max_files = 5

# A TCP target needs no privilege, so the setup runs as your user.
[[targets]]
method = "tcp"
host = "1.1.1.1"
port = 443
label = "cloudflare-https"

# ICMP needs root or CAP_NET_RAW. Uncomment and run the monitor with sudo:
# [[targets]]
# method = "icmp"
# ip = "1.1.1.1"
# label = "cloudflare-dns"
```

Unit tests in `src/init.rs`:

- `the_template_resolves`: parse/resolve, then assert one target with `Method::Tcp`, no
  interface, `database = "data/events.db"`, one `ChannelSettings::LogFile` with the expected
  filename, `stabilize == Duration::ZERO`, `max_bytes == 10_485_760`, `max_files == 5`, and the
  global `notify` list containing `events`.
- `the_template_keeps_icmp_out_of_the_active_targets`: no resolved target uses ICMP, while the
  template contains the commented `method = "icmp"` example.
- `a_missing_folder_is_created`, `an_empty_folder_is_filled`, `a_non_empty_folder_is_refused`,
  `a_file_is_not_a_directory`, `a_second_run_is_refused`, and
  `the_report_lists_the_paths_and_commands`.
- The `NotEmpty` and `NotADirectory` tests assert the error variant and that no new
  `network-monitor.toml` was created; the second-run test asserts the existing file is
  byte-identical.

**Verify:** `cargo test --lib init` passes; `cargo test --lib config` still passes.

### Step 2 — Add the `init` subcommand and dispatch it before any configuration

In `src/cli.rs`, add to `Command`:

```text
Init {
    /// Create a minimal, runnable setup in a new folder; configuration flags are rejected
    #[arg(value_name = "FOLDER")]
    folder: PathBuf,
}
```

`FOLDER` is required and positional. Help text states that the folder must not exist or must be
empty, and that the command takes no configuration. The global configuration flags still appear
in `init --help` because clap propagates every `global = true` argument; the runtime guard is
what rejects them. Hiding them would mean moving every flag off `global`, a separate refactor
that this plan does not attempt.

In `src/main.rs`:

- Add `AppError::Init(#[from] init::InitError)`.
- Match `Command::Init` after the `--log-level` handling and before `Cli::overrides`, the
  `--show-config` branch, and `load_settings`. This is what guarantees that `init` never reads
  `./network-monitor.toml` and never parses a `--target`.
- Add a small guard that rejects the configuration flags:
  `--config`, `--show-config`, `--interval-ms`, `--timeout-ms`, `--database`,
  `--retention-days`, and a non-empty `--target` list. The error is one line:
  `"init takes only a folder; it does not read a configuration"`. `--log-level` and
  `--compression` are process-wide flags with no configuration effect and are not rejected.
- On success, `print!("{}", report.render())` and return `ExitCode::SUCCESS`.
- Change the no-subcommand message from the stale `(monitor, query)` list to
  `"error: a subcommand is required; run --help for the list"`.

In `src/lib.rs`, add `pub mod init;`.

CLI tests in `src/cli.rs`: `init` parses with the folder, `init` without a folder is an error,
and `--help` lists the `init` subcommand.

**Verify:** `cargo test --lib cli` and `cargo test --lib init` pass; `cargo run -- init --help`
shows the `FOLDER` argument and the doc text. The global configuration flags also appear in that
output because clap propagates `global = true`; the runtime guard rejects them, and
`init_rejects_configuration_flags` covers that rejection.

### Step 3 — Prove the contract end to end

Add `tests/init.rs`, using `assert_cmd`, `predicates`, and `tempfile` as in `tests/cli.rs`:

- `init_creates_a_folder_that_check_accepts`: run `init <root>/monitor`; assert success, the
  three paths, and no `data/events.db`; then run `check --config network-monitor.toml` with
  `current_dir(<root>/monitor)`; assert success and that `logs/events.log` exists while
  `data/events.db` still does not.
- `init_refuses_a_non_empty_folder`: put a file in the target; run; assert failure, the “not
  empty” message, and no `network-monitor.toml`.
- `init_refuses_a_file`: pass a path that is a regular file; assert “not a directory”.
- `init_refuses_a_second_run`: run once, capture the config bytes, run again; assert failure
  and byte-identical content.
- `init_does_not_read_the_default_configuration`: write an invalid `network-monitor.toml` in the
  process working directory, run `init` on a different new folder; assert success and that the
  invalid file is untouched. This pins the dispatch order.
- `init_rejects_configuration_flags`: one case each for `--config`, `--target`, `--database`,
  and `--show-config`; assert failure and the single error line.
- `init_accepts_an_empty_existing_folder` and `init_accepts_a_path_with_spaces`.

**Verify:** `cargo test --test init` passes.

### Step 4 — Add the embedded guide and the writer

Create `src/manual.rs`:

- `pub const USER_GUIDE: &str = include_str!("../docs/USER-GUIDE.md");`
- `pub enum ManualError` with `thiserror`: `Write { source: std::io::Error }`, displayed as
  `cannot write the manual to standard output: <source>`.
- `pub fn write_guide<W: Write>(out: &mut W) -> Result<(), ManualError>`: `write_all` the
  guide bytes, then `flush`; an `ErrorKind::BrokenPipe` from either call is `Ok(())`; any
  other error becomes `ManualError::Write`.
- `pub fn run() -> Result<(), ManualError>`: obtain `std::io::stdout()`; if
  `stdout.is_terminal()`, write one line to stderr with `writeln!` and ignore a write error:
  `hint: this is Markdown; pipe it to a pager (less, more) or mdcat --ansi`; lock stdout and
  call `write_guide`. The hint is never written to stdout, never written when stdout is not a
  terminal, and a closed stderr cannot turn a successful manual into a panic.
- Unit tests: `USER_GUIDE` starts with `# User Guide` and contains `## Install`,
  `## Configuration`, `## Command line`, and `## Limits and deliberate choices`;
  `write_guide` into a `Vec<u8>` equals `USER_GUIDE.as_bytes()`; a writer returning
  `BrokenPipe` is `Ok`; a writer returning another error is `ManualError::Write`.

The guide is UTF-8 and `include_str!` fails the build if it is not; the file is never read at
run time, so `manual` works when the binary is moved alone to another host. No configuration
file key is added or changed; the only new surface is the command:

```text
network-monitor manual
network-monitor man
network-monitor manual | less -R
network-monitor manual | mdcat --ansi | less -R
```

**Verify:** `cargo test --lib manual` passes.

### Step 5 — Add the `manual` subcommand and share the configuration-flag guard

In `src/cli.rs`:

- Add a unit variant `Manual` with the doc comment
  `/// Print the full user guide as Markdown` and `#[command(visible_alias = "man")]`.
- Extract `pub fn uses_configuration(&self) -> bool` returning true when `config`,
  `show_config`, `interval_ms`, `timeout_ms`, `database`, `retention_days`, or a non-empty
  `targets` is set. Replace the inline guard in the `init` branch from Step 2 with this method.
- CLI tests: `manual` and `man` parse to `Command::Manual`; `manual extra` is an argument
  error; the top-level help lists `manual` and the alias.

In `src/main.rs`:

- Add `AppError::Manual(#[from] manual::ManualError)`.
- Match `Command::Manual` after the `--log-level` handling and before `Cli::overrides`, the
  `--show-config` branch, and `load_settings`. This is what guarantees no configuration read
  and no `--target` parse.
- If `cli.uses_configuration()`, return one `AppError::Invalid`:
  `manual takes no arguments; it does not read a configuration`. `--log-level` and
  `--compression` are process-wide flags with no configuration effect and remain accepted.
- On success, return `ExitCode::SUCCESS`; a `ManualError` is printed as `error: <message>`
  and exits non-zero.

In `src/lib.rs`, add `pub mod manual;`.

**Verify:** `cargo test --lib cli` and `cargo test --lib manual` pass; `cargo run -- manual
--help` shows the command help; `cargo run -- man --help` shows the same; `cargo run --
--help` lists `manual` and the `man` alias.

### Step 6 — Prove the manual contract end to end

Add `tests/manual.rs`, using `assert_cmd`, `predicates`, and `tempfile` as in `tests/cli.rs`:

- `manual_prints_the_embedded_guide`: `manual` exits `0`; stdout starts with `# User Guide`;
  stdout contains `## Command line` and `## Configuration`; stdout equals the bytes of
  `docs/USER-GUIDE.md` read by the test (the `include_str!` guarantee).
- `man_is_an_alias`: `man` and `manual` produce byte-identical stdout and both exit `0`.
- `manual_reads_no_configuration`: write an invalid `network-monitor.toml` in a temporary
  working directory; run `manual` there; assert success and that the invalid file is
  untouched. This pins the dispatch order.
- `manual_rejects_configuration_flags`: one case each for `--config`, `--target`,
  `--database`, `--show-config`; assert failure and the single error line.
- `manual_creates_no_files`: run in an empty temporary directory; assert `read_dir` is still
  empty and no `data/` or `logs/` appears.
- `manual_help_lists_the_command_and_alias`: `manual --help` and `man --help` contain the doc
  text; the top-level `--help` contains `manual` and the `man` alias.
- `#[cfg(unix)] manual_survives_a_closed_pipe`: spawn `manual` with stdout piped, read one
  line, drop the pipe, and wait; assert exit code `0` and no panic text on stderr. The unit
  test in Step 4 covers the same branch with a fake writer, so this is the process-level
  proof.

**Verify:** `cargo test --test manual` passes.

### Step 7 — Document both commands in the product and the user guide

Update `docs/PRODUCT.md`:

- “In scope”: add “A one-shot `init` that scaffolds a minimal, runnable setup in a new folder.”
- “Out of scope”: add “OS-level service installation or scheduling: `init` writes only inside a
  folder; no systemd, launchd, or Windows service is installed.”
- Add `FR-33` after `FR-32`: `init <FOLDER>` creates a minimal setup, takes no configuration
  flag, accepts a missing or empty folder and refuses a file or a non-empty folder; it writes
  `network-monitor.toml` with the defaults, one unprivileged TCP target, a commented ICMP
  example, and an enabled `logfile` channel `events` at `logs/events.log` with the rotation
  keys; it creates `data/` and `logs/`; it validates the generated file before writing it,
  starts no probe, opens no database, binds no socket, reads no configuration, writes nothing
  outside the folder, and prints the next commands. It exits non-zero on a refusal or an I/O
  error. The generated file is an ordinary configuration; no key is new. The first `check` or
  start creates `logs/events.log` through the channel, as `FR-29` states.

Update `docs/USER-GUIDE.md`:

- Add `### From an empty folder` under `## Install`, after `### From source` (the fastest path
  should be near the top). It must be self-contained:

  ```bash
  network-monitor init ./monitor
  cd ./monitor
  network-monitor check --config network-monitor.toml
  network-monitor monitor --config network-monitor.toml --listen 127.0.0.1:8642
  ```

  The guide uses `network-monitor` for the binary on `PATH`; in a source checkout, use
  `./target/release/network-monitor`. Explain what the folder contains, that the active target
  is TCP `1.1.1.1:443` and needs no privilege, that the ICMP example is commented and needs
  `root`/`CAP_NET_RAW`, that `logs/events.log` is the event log of the `events` channel and the
  diagnostic log remains on stderr, that `check` creates the log file, that the relative paths
  resolve against the working directory so the commands must run from the folder, that `init`
  refuses a non-empty folder and never overwrites, and that the page is at `127.0.0.1:8642`
  when `--listen` is given. State that the generated file contains no secret; a token added
  later is read from the file only and should be protected, as the notification section states.
- In “Command line > Subcommands”, add a row:
  `| init <folder> | Create a minimal, runnable setup in a new folder: the configuration, the data directory, and the log directory |`.
- In “Limits and deliberate choices”, add one bullet: the scaffolded event log is a `logfile`
  channel bounded by `max_bytes`/`max_files`; the diagnostic log is stderr and is bounded by the
  supervisor, not by `init`; `init` does not install or supervise a service.
- In “Rotate the logs”, state that the generated `events` channel is an example of the
  `logfile` row and that the diagnostic log never becomes a file of the tool.

The user guide already documents every key the template uses: `interval_ms`, `timeout_ms`,
`database`, `retention_days`, `notify`, `[[notification]]` with `type = "logfile"`,
`filename`, `stabilize_seconds`, `max_bytes`, `max_files`, and `[[targets]]` with
`method = "tcp"`, `host`, `port`, and `label`. No new key row is required. Verify each against
the code and the guide in the step below.

Update `README.md` “Quick start” with one block, after the build command:

```bash
# Scaffold a runnable setup in ./monitor, then check and start it
./target/release/network-monitor init ./monitor
cd ./monitor
../target/release/network-monitor check --config network-monitor.toml
../target/release/network-monitor monitor --config network-monitor.toml \
  --listen 127.0.0.1:8642
```

- Add `FR-34` after `FR-33`: `manual` prints the exact bytes of the user guide embedded at
  build time to stdout, with `man` as a visible alias; it accepts no arguments, reads no
  configuration, starts no probe, opens no database, binds no socket, and creates no file.
  When stdout is a terminal it writes one hint to stderr; otherwise stdout is pure Markdown. A
  closed stdout pipe is a quiet success; any other write error exits non-zero with a message.
  It adds no renderer, pager, subprocess, native man page, or configuration key, and the guide
  always matches the binary of that build.

Update the same `docs/USER-GUIDE.md` “Command line > Subcommands” table with a second row:
`| manual (alias man) | Print the full user guide as Markdown; pipe it to less -R or mdcat --ansi |`.
After the table, add a short paragraph with the two pipelines:

```bash
network-monitor manual | less -R
network-monitor manual | mdcat --ansi | less -R
```

State that `less` and `mdcat` are optional, that `manual` never starts a pager or another
process, that the command reads no configuration, and that `man` is an alias. State that the
guide printed is the guide of that build, not a copy that can drift.

Add one line to the `README.md` quick start, after the init block:

```bash
./target/release/network-monitor manual | less -R
```

**Verify:** `grep -n "init\|manual" docs/PRODUCT.md docs/USER-GUIDE.md README.md` finds the new
sections, including the `init <folder>` and `manual (alias man)` rows of the Subcommands table;
each key in the generated template appears in the user-guide key tables (`grep -n
"max_bytes\|max_files\|stabilize_seconds\|retention_days" docs/USER-GUIDE.md`); the documented
`init` and `check` commands use the same arguments as the integration test, the `monitor` line
is the existing start path covered by the monitor tests, `FR-34` follows `FR-33`, and the two
manual pipelines are exactly the ones the smoke test uses.

### Step 8 — Update the architecture, the developer guide, the glossary, the ADRs, and the landing page

- `docs/ARCHITECTURE.md`:
  - Section 1: add `init` to the short-lived commands list, with “creates a minimal, runnable
    setup in a new folder”.
  - Section 5: add a component-table row:
    `| init | Create a minimal, runnable setup in a new folder: the template configuration and
    its data and log directories | run, Report, InitError, TEMPLATE |`.
  - Add a `### init` contract after `### check`: input one folder path and no configuration;
    output the folder with the three entries or an `InitError`; guarantees: refuses a non-empty
    folder or a file, validates the template before writing, writes nothing outside the folder,
    starts no probe, opens no database, binds no socket, and the event log is a `logfile`
    channel rather than a diagnostic log.
  - Section 7: add a “Scaffolded folder” bullet: the folder is created by `init`, moved or
    mounted as a unit, run from its own directory; relative paths follow the process working
    directory; a service supervisor remains the operator's choice.
- `docs/DEVELOPER-GUIDE.md`: add `src/init.rs` to the repository layout after `src/import.rs`,
  with the one-line description “create a minimal, runnable setup in a new folder”.
- `docs/GLOSSARY.md`: add `**Init**` — “The `init` subcommand: it creates a minimal, runnable
  setup in a new folder — the template configuration and the `data/` and `logs/` directories —
  and prints the next commands. It starts no probe, installs no service, and changes nothing
  outside the folder.”
- `docs/ADR.md`: add the index row and `ADR-0036 — Scaffold a local setup with an init verb, not
  an installer`, with the context, decision, consequences, and rejected alternatives recorded
  in “Decision-record impact”. Status `accepted`, date `2026-11-17`.
- `site/index.html`: add a fourth `.snippet` in the Quick start grid, “From an empty folder”,
  with the same four commands, using `./target/release/network-monitor` as the surrounding
  source snippet does, and one sentence that the active target is TCP and needs no privilege.
  The grid is single-column, so no CSS change is needed.

- `docs/ARCHITECTURE.md`: add a `manual` component-table row
  `| manual | Print the user guide embedded at build time, without reading configuration or touching the database | USER_GUIDE, write_guide, run, ManualError |`;
  add a `### manual` contract after `### init`; add one sentence under “Bare host” in section 7
  that the binary carries its manual.
- `docs/DEVELOPER-GUIDE.md`: add `src/manual.rs` after `src/init.rs` with “print the embedded
  user guide”; note that `docs/USER-GUIDE.md` is embedded, so editing it updates `manual` at the
  next build and there is no second copy.
- `docs/GLOSSARY.md`: add `**Manual**` — “The `manual` subcommand: it prints the user guide
  embedded in the binary as Markdown, with `man` as an alias; it reads no configuration, starts
  nothing, and writes only to standard output (and one hint to standard error when standard
  output is a terminal).”
- `docs/ADR.md`: add the index row and `ADR-0037 — Embed the user guide and print it as
  Markdown, without a renderer`, with the context, decision, consequences, and rejected
  alternatives recorded in “Decision-record impact”; status `accepted`, date `2026-11-17`.
- `site/index.html`: add a fifth `.snippet`, “Read the manual”, with
  `./target/release/network-monitor manual | less -R` and
  `./target/release/network-monitor manual | mdcat --ansi | less -R`, and one sentence that the
  manual is embedded and works offline.

**Verify:** `grep -n "init\|manual\|Manual" docs/ARCHITECTURE.md docs/DEVELOPER-GUIDE.md
docs/GLOSSARY.md docs/ADR.md site/index.html` finds every entry; both ADR index rows match their
section titles and dates; the landing page’s two new snippets use the same sequences of
subcommands as the user guide, with the source-build binary path; preview the page with the
existing `scripts/utils/serve_site.py` helper and open `#quick-start`.

### Step 9 — Run the quality gate and the hands-on smoke tests

Run the exact quality command from `.spec-workflow.yml`:

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Then a manual smoke test in a scratch directory:

```bash
cargo build --release
BIN="$PWD/target/release/network-monitor"
rm -rf /tmp/nm-init-demo
"$BIN" init /tmp/nm-init-demo
cd /tmp/nm-init-demo
"$BIN" check --config network-monitor.toml
"$BIN" monitor --config network-monitor.toml --listen 127.0.0.1:8642
```

The `check` must exit `0`, create `logs/events.log`, and leave `data/events.db` absent. The
monitor must start without `sudo`, record the first TCP transition, create `data/events.db`,
append to `logs/events.log`, and serve the page at `127.0.0.1:8642`; stop it with `SIGINT`.
Then confirm `init /tmp/nm-init-demo` fails with the non-empty message and
`init <new> --target 1.1.1.1` fails with the configuration-flag message.

For `manual`, with the same built binary:

```bash
BIN="$PWD/target/release/network-monitor"
"$BIN" manual > /tmp/nm-manual.md
cmp /tmp/nm-manual.md docs/USER-GUIDE.md
"$BIN" manual | head -n 5
"$BIN" man | head -n 5
"$BIN" manual | mdcat --ansi | less -R
"$BIN" manual --config missing.toml
```

`cmp` must be silent: stdout is exactly the embedded guide. `head` closing the pipe must not
produce a panic, and `manual` must exit `0` (use `set -o pipefail` if the shell supports it).
The `mdcat` pipeline is interactive; it must render the guide with colour and page it with
`less`. The last command must exit non-zero with
`manual takes no arguments; it does not read a configuration`.

**Verify:** the quality command exits `0`; the init commands produce the stated files and the
page; the two init refusals and the manual configuration refusal exit non-zero with the stated
messages; `cmp` is silent and the manual pipelines exit cleanly.

## Security considerations

- The generated configuration has no secret: the only channel is a `logfile` channel, and the
  only target is a public TCP endpoint. The operator may add a Telegram or webhook token later;
  the user guide already states that secrets live in the configuration file only and are
  redacted from every rendering.
- `init` refuses a non-empty folder and writes the configuration with `create_new`, so it cannot
  overwrite an existing file or a secret. It writes only the fixed names inside the folder the
  operator names; no path comes from the configuration or from the network.
- The event log may contain target keys, labels, and causes; the `logfile` channel creates it
  with mode `0600` on Unix (`FR-29`), and `init` does not pre-create it with a weaker mode.
- `init` runs with the operator's privileges and invokes no external command, opens no socket,
  reads no network input, and adds no dependency. It is not a privileged or remote surface.
- The generated folder is writable by the operator; the configuration contains no secret, and
  the new user-guide section states that a token added later is read from the file only and
  should be protected, as the notification section already says.
- `manual` is a compile-time static text surface: it accepts no input beyond the optional
  process-wide flags, has no path, URL, or network input, opens no file, socket, or database,
  and spawns no process. It writes only to stdout and, on a terminal, one hint to stderr.
- A closed stdout pipe is handled as success, so `manual | head` does not panic or produce a
  misleading failure. Other write errors are reported without a stack trace, and a closed
  stderr cannot turn a successful manual into a panic.
- The embedded guide contains no secret; it describes token redaction. It cannot be modified at
  run time; an attacker who can edit the binary can already change anything.

## Verification plan

- **Automated:** `cargo test --lib init`, `cargo test --lib manual`, `cargo test --lib cli`,
  `cargo test --test init`, and `cargo test --test manual`. The tests pin the template
  parse/resolve, the active method, the channel keys, the folder creation, every refusal, the
  dispatch-before-configuration guarantee, the `check` acceptance that creates `logs/events.log`
  without creating the database, the exact embedded guide bytes, the `man` alias, the manual
  flag refusal, the no-files side effect, the help listing, and the broken-pipe branch.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo
  test`.
- **Manual:** the Step 9 smoke tests: `init`, `check`, `monitor --listen`, inspect the page, the
  database, and the event log; stop the monitor; run the two init refusals; compare
  `manual`'s output with `docs/USER-GUIDE.md`; pipe `manual` and `man` to `head`; render with
  `mdcat --ansi | less -R`; run the manual configuration refusal. Optionally repeat on
  macOS/Windows when such a host is available; no CI matrix exists, so this is a manual
  platform check, not an automated gate.
- **Rollback:** delete the created folder; no other path was written. The code rollback is one
  revert of the `init` and `manual` modules, the CLI arms, the tests, and the documentation; no
  database, schema, or configuration file of an existing deployment changes.

## Open questions

None. The interview resolved the design; the user was unavailable for the init defaults and the
manual design was settled in the subsequent conversation. The decisions are recorded here, in
`ADR-0036`, and in `ADR-0037`.

For `init`:

- `init` requires a folder; it does not default to the current directory.
- The event log channel is enabled by default.
- `init` creates only `logs/`; the first `check` or the monitor creates `logs/events.log`.
- The active target is TCP `1.1.1.1:443`; ICMP is a commented example.
- A non-empty folder is refused; there is no `--force`.
- Configuration flags are rejected; `--log-level` and `--compression` are process-wide and are
  not configuration.

For `manual`:

- The command is `manual`, with `man` as a visible alias; it has no arguments.
- The output is the raw Markdown of `docs/USER-GUIDE.md`, embedded at build time; no renderer,
  pager, or subprocess.
- The terminal hint goes to stderr only when stdout is a terminal.
- Configuration flags are rejected with the same shared guard as `init`.
- A closed stdout pipe is a quiet success.
- `init` is unchanged apart from sharing the guard; its report does not mention `manual`.

## Implementation checklist

- [x] Step 1 — `src/init.rs`: template, `run`, `Report`, `InitError`, unit tests
- [x] Step 2 — `Command::Init`, early dispatch, flag rejection, CLI tests
- [x] Step 3 — `tests/init.rs`: end-to-end creation, acceptance, and refusals
- [x] Step 4 — `src/manual.rs`: embedded guide, writer, broken-pipe handling, unit tests
- [x] Step 5 — `Command::Manual` and `man` alias, early dispatch, shared flag guard, CLI tests
- [x] Step 6 — `tests/manual.rs`: output, alias, no configuration read, refusals, no files, closed pipe
- [x] Step 7 — `PRODUCT`, `USER-GUIDE`, `README`
- [x] Step 8 — `ARCHITECTURE`, `DEVELOPER-GUIDE`, `GLOSSARY`, `ADR-0036`, `ADR-0037`, landing page
- [x] Step 9 — quality gate and hands-on smoke tests
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

- **Date**: 2026-11-17
- **Lane**: `spec-auto` (implement and review in one invocation)
- **Entry state**: `status: draft`, no review rounds; baseline branch `main`, HEAD `ddbf563`,
  the plan file the only untracked file.
- **Checkpoints**: none in the plan, so none crossed.

### What changed

| File | Change |
|---|---|
| `src/init.rs` *(new)* | `TEMPLATE` constant, `DATA_DIR`/`LOG_DIR`/`DATABASE`/`EVENT_LOG`, `run` (validate, refuse a file or non-empty folder, create the folder and `data/`/`logs/`, write the configuration last with `create_new`), `Report::render`, `InitError`, 7 unit tests |
| `src/manual.rs` *(new)* | `USER_GUIDE` from `include_str!("../docs/USER-GUIDE.md")`, `write_guide` (a closed pipe is success, any other write error is a `ManualError`), `run` (one hint on stderr only when stdout is a terminal), 4 unit tests |
| `src/lib.rs` | `pub mod init;` and `pub mod manual;` |
| `src/cli.rs` | `Command::Init { folder }`, `Command::Manual` with `visible_alias = "man"`, `Cli::uses_configuration`, 4 unit tests |
| `src/main.rs` | Early dispatch of `init` and `manual` before `Cli::overrides`/`load_settings`; the shared configuration-flag guard with the two exact messages; `AppError::Init`/`AppError::Manual`; the `unreachable!` arm; the no-subcommand message now says “run --help for the list” |
| `tests/init.rs` *(new)* | 8 end-to-end tests: creation plus `check` acceptance, log created and database absent, non-empty/file/second-run refusals, no default-config read, four configuration flags, empty existing folder, path with spaces |
| `tests/manual.rs` *(new)* | 7 end-to-end tests: exact guide bytes, `man` alias byte-identical, no configuration read, four configuration flags, no file created, help and alias listing, closed pipe on Unix |
| `docs/PRODUCT.md` | `init` and `manual` in scope; one out-of-scope bullet for OS service installation and scheduling (merged with the previous Windows/launchd bullet); `FR-33` and `FR-34` |
| `docs/USER-GUIDE.md` | “From an empty folder” under Install; `init`/`manual` rows and the pipelines in Command line; the scaffolded channel in “Rotate the logs”; one bullet in Limits |
| `README.md` | Init block and `manual` line in Quick start |
| `docs/ARCHITECTURE.md` | Section 1 command list; component table and contracts for `init` and `manual`; a “Scaffolded folder” deployment bullet and the manual on a bare host |
| `docs/DEVELOPER-GUIDE.md` | Both modules and both test files in the repository layout; the `include_str!` note |
| `docs/GLOSSARY.md` | `Init` and `Manual` |
| `docs/ADR.md` | `ADR-0036` and `ADR-0037` with index rows |
| `site/index.html` | “From an empty folder” and “Read the manual” snippets |

### Decisions and deviations

- **`FR-33`/`FR-34`, not `FR-32`/`FR-33`.** `FR-32` is already the active snapshot, so the two
  new requirements were appended as `FR-33` and `FR-34`. Plan and documentation were corrected
  to those numbers.
- **The template resolves its logfile path.** `Config::resolve` canonicalizes a `logfile`
  channel's `filename` to an absolute path, so the unit test compares with
  `settings.filename.ends_with(EVENT_LOG)` rather than with the literal relative path; the
  generated file itself keeps the relative path.
- **`read_exact` in the closed-pipe test.** The first version read with `read`, which clippy
  rejects as `unused_io_amount`; the test now reads exactly 64 bytes before dropping the pipe.
- **The configuration is the last write.** A failure after `data/` exists leaves a folder that a
  second `init` refuses; the plan's rollback is to delete the folder. The configuration itself
  is never left behind, and `create_new` cannot overwrite.
- **`--log-level` and `--compression` remain accepted.** They are process-wide, not
  configuration keys, and are not in `Cli::uses_configuration`.
- **The out-of-scope bullet was merged.** The new OS-service bullet repeated the previous
  Windows/launchd bullet, so the two are now one.

### Verification performed

| Command | Result |
|---|---|
| `cargo fmt --check` | passed, no output |
| `cargo clippy --all-targets -- -D warnings` | passed, no warnings |
| `cargo test` | passed: 458 lib (2 ignored, privileged or public network), 44 `cli`, 8 `config_precedence`, 1 `icmp_privileged` (3 ignored), 18 `import`, 8 `init`, 7 `manual`, 6 `monitor_e2e`, 7 `notify`, 5 `notify_channels` (5 ignored), 8 `reload`, 7 `simulate`, 3 `snapshot`, 4 `store`, 60 `web`; 0 failed |
| `cargo test --lib init` / `--lib manual` / `--lib cli` | 12 / 6 / 18 passed |
| `cargo test --test init` / `--test manual` | 8 / 7 passed |
| `cargo package --list` | includes `docs/USER-GUIDE.md`, so `include_str!` builds from a packaged crate |
| `init /tmp/nm-init-demo` | exit `0`; created the configuration (0664) and `data/`/`logs/` (0775); printed the three next commands |
| second `init` / `init --target 1.1.1.1` | exit `1` with the non-empty and configuration-flag messages |
| `check` in the folder | exit `0`; created `logs/events.log` (0600); `data/events.db` absent |
| `monitor` from the folder with `--listen 127.0.0.1:8642` | ran unprivileged, logged `unknown -> up (rtt 1.885 ms)`, page `200` (6333 bytes), created `data/events.db`, appended the `UP` line to `logs/events.log`, exit `0` on `SIGTERM` |
| `manual > /tmp/nm-manual.md` and `cmp` | identical to `docs/USER-GUIDE.md`; exit `0` |
| `manual \| head -n 5`, `man \| head -n 5` with `pipefail` | exit `0`, no panic |
| `manual \| mdcat --ansi \| less -R` | exit `0` (non-TTY; `less` copies the stream) |
| `manual --config missing.toml` | exit `1` with the manual configuration message |
| `--help` | lists `init` and `manual [alias: man]` |

Limitations: interactive paging under a real TTY was not exercised (the smoke test piped
`less`); macOS and Windows were not available, as the plan's optional manual check states.


## Review Notes

- **Round**: 1
- **Review date**: 2026-11-17
- **Disclosure**: this round is a self-review performed in the same `spec-auto` invocation as
  the implementation; there is no independent reviewer. The verdict rests on the diff, the plan
  text, and the commands executed below, not on `## Implementation Notes`.
- **Verdict**: PASS
- **Suggested commit**: `feat(cli): add init and manual subcommands`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check`, `cargo clippy --all-targets
  -- -D warnings`, `cargo test` — 644 passed, 0 failed, 10 ignored, re-run after the user-guide
  repair below)

### Verification matrix

| # | Check | Evidence |
|---|---|---|
| 1 | Goals achieved | 16/16; the matrix below names the evidence for each group |
| 2 | Scope | the change set is `src/init.rs`, `src/manual.rs`, `src/lib.rs`, `src/cli.rs`, `src/main.rs`, the two new test files, and the eight documentation files in `## Affected modules`; `Cargo.toml`, `Cargo.lock`, the sample configuration, Docker files, `NOTES.md`, `HISTORY-IMPORT.md`, and `NOTIFICATIONS.md` are untouched; the plan file is the only untracked non-code file |
| 3 | Quality suite green | `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean; `cargo test` 644 passed, 0 failed, 10 ignored (the pre-existing privileged/public-network tests) |
| 4 | Tests exercise real code | `tests/init.rs` runs the binary and then the real `check`; `tests/manual.rs` runs the binary and compares stdout with the on-disk guide; unit tests cover the template parse/resolve and the broken-pipe branch |
| 5 | No existing test weakened | no test deleted, skipped, or loosened; the only test edits are the two new files and the CLI test module additions |
| 6 | Checklist, questions, notes | checklist 14/14; all three hand-off gates passed; open questions resolved in the plan |
| 7 | Documentation contracts | `PRODUCT` (`FR-33`, `FR-34`), `USER-GUIDE`, `README`, `ARCHITECTURE`, `DEVELOPER-GUIDE`, `GLOSSARY`, `ADR-0036`/`ADR-0037`, and the landing page updated; no stale subcommand list remains |
| 8 | No architecture/decision contradiction | `ADR-0036` and `ADR-0037` are accepted and indexed; no invariant changes; `ADR-0006` remains a historical record, not a current list |
| 9 | No scope creep | no installer, no `/etc`, no service, no `init` option, no renderer, no pager, no new configuration key, no new crate |
| 10 | Edge cases and errors | file, non-empty folder, second run, configuration flags on both commands, path with spaces, invalid default configuration in the working directory, piped and TTY stdout, closed pipe, non-UTF-8 path not applicable on a `PathBuf` |
| 11 | Security | no secret in the template, `create_new`, configuration written last, nothing outside the folder, event log created by the channel with mode `0600`, `manual` has no input or file access |
| 12 | Security plan | not a security plan |
| 13 | User guide | accurate and standalone: the folder flow, both refusals, the partial-folder recovery, the pipelines, the scaffolded event log, and the limits |
| 14 | Verification plan | every automated and manual item executed except interactive paging and the macOS/Windows optional check, recorded under Known limitations |

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | minor | `src/main.rs`, `src/init.rs` | the `init` report is printed with `print!`, so `init … \| true` panics with exit `101` (after the folder is created) instead of ignoring the closed pipe; the other short-lived commands print the same way and no plan goal covers this | recorded, not fixed: a cross-command output helper is future work, not this plan |
| 2 | minor | `tests/manual.rs` | `manual_survives_a_closed_pipe` may complete the 54 KB write before it drops the reader, so it does not always force `EPIPE`; the unit test `a_broken_pipe_is_success` covers the branch deterministically | recorded, not fixed |
| 3 | minor | `docs/USER-GUIDE.md` | a failure after `data/` is created leaves a non-empty folder that a retry refuses, and the guide did not say how to recover | fixed in review: the init section now says to remove the folder and run it again |

### Gap analysis

- Goals: 16/16 achieved; each is covered by a unit test, an integration test, a smoke test, or a
  document diff.
- Scope: in scope; no out-of-scope file changed; no commit or push was made.
- Deviations: `FR-33`/`FR-34` numbering, the canonicalized logfile path in the unit test, and
  `read_exact` in the closed-pipe test are recorded in `## Implementation Notes`.
- Additions: none beyond the plan, apart from the user-guide recovery sentence above.
- Deferred: a shared EPIPE-safe output helper for the short-lived commands; a stronger
  closed-pipe integration test.

### Known limitations

- Interactive paging with `less` under a real terminal was not exercised; `less` copied the
  stream with a non-TTY stdout, and `script` verified the TTY hint on a pseudo-terminal.
- macOS and Windows were not available; the optional manual platform check was not run.
- The `init` report is informational: if stdout is closed, the folder is still complete but the
  process exits `101` (finding 1).

### Recommendations for future work

- Route the short-lived commands through one output helper that treats `BrokenPipe` as success,
  so `check`, `init`, `query`, and `--show-config` share the behaviour `manual` has.
- If the closed-pipe guarantee becomes part of a contract, write the integration test against a
  pipe smaller than the guide instead of relying on the reader's timing.
- A later `install` verb can reuse `TEMPLATE` and `Report` without changing this plan's contract.
