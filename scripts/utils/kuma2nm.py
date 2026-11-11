#!/usr/bin/env python3
"""Convert an Uptime Kuma heartbeat export into the `network-monitor import` format.

# What it does

`network-monitor import` reads the CSV the web page offers for download: one row per
state change with the columns

    target,label,interface,state,occurred_at,rtt_us

Uptime Kuma exports its heartbeats in another shape:

    id,monitor_name,status,time,message,ping

This script rewrites the second into the first, so a Kuma history can be loaded into a
`network-monitor` database with the command it already has. It is a migration aid that
runs once per export and stays outside the binary; nothing in `src/` depends on it.

# When to use it

* Moving an existing Uptime Kuma installation to `network-monitor` and keeping the
  history.
* Merging a Kuma history into a database that already records transitions. The importer
  drops an event that repeats the state before it, so re-running the same file is safe.

When the history comes from `network-monitor` itself, do not use this script: the page
offers the file in the right format already.

`kuma-example.csv`, next to this file, is the small export the runbook converts, and
`docs/UPTIME-KUMA-IMPORT.md` is the step-by-step operation: check the Kuma clock, export
the heartbeats from Kuma's own database with the SQL the runbook gives, convert, back up,
import with the monitor stopped, verify, roll back. The CSV the Kuma status page offers
for download is a snapshot of what the browser was shown and can stop days before the
database does, so prefer the SQL export.

# How the fields map

Kuma `message` (`PING 1.1.1.1 (1.1.1.1) ...`) becomes `target`: the address after
`PING `. A heartbeat without a message keeps the last address named for that monitor, so
the heartbeat interval does not have to repeat it.

Kuma `status` becomes `state`: `1` is `up`, `0` is `down`; `2` (pending) and `3`
(maintenance) are skipped and counted.

Kuma `time` becomes `occurred_at`: `YYYY-MM-DD HH:MM:SS.mmm` read as UTC and written as
`...T...Z`.

Kuma `ping` becomes `rtt_us`: milliseconds to microseconds, rounded; empty when Kuma
recorded none.

`monitor_name` is not imported — Kuma names a monitor, `network-monitor` tracks an
address. `label` comes from `--label ADDRESS=LABEL` and is empty otherwise. `interface`
is always empty: Kuma does not record which interface it probed.

Every heartbeat is written, including the ones that repeat the previous state: the
importer collapses them (`BR-1`) in one pass, and letting it decide keeps this script
from needing the state machine the store already has.

# Time zone

Kuma stores the heartbeat time as a naive local-time string, produced by the Kuma
process clock. The official image runs on UTC unless `TZ` is set, so the default here is
to add `Z` and change nothing else. Check the installation before importing:

    docker exec uptime-kuma date

If it prints a time other than UTC, convert the timestamps first: this script does not
carry an offset, on purpose — a wrong offset corrupts every row silently.

# Caveats worth reading before the import

* **A monitor can be repointed.** Kuma keeps the monitor name when the address changes,
  so the history of one name splits at that instant into two targets. The script follows
  the address in the message and prints a `note:` line for every switch it sees.
* **`--alias` changes what the data claims.** Recording one address under another (for
  example a DNS monitor that moved from `1.1.1.1` to `8.8.8.8`, kept as a single
  `1.1.1.1` lane) makes one continuous chain out of two different endpoints. Use it only
  when that is the story the reader should see, and say so in the commit message.
* **A gap in the export is not a state.** When Kuma stopped collecting, the export has no
  row, and the imported chain continues with the last state it knows. On a database that
  is already recording, the two chains join at the first live event; if that event
  repeats the imported state, the importer folds it away and the page reports the state
  as older than the first live probe.
* **Labels never overwrite.** The store sets a target's label with `COALESCE`, so a label
  the live database already holds wins over the one given here.
* **Exit status** is `0` on success and `2` on an unreadable input, so a shell chain
  stops instead of importing an empty file.

# Technical notes

* Standard library only, Python 3.9 or newer; no dependency to install on the NAS or on
  a workstation.
* The whole file is read and sorted by `time` before anything is written, because the
  output must be ordered by target and instant while a Kuma export is ordered by id. 64k
  heartbeats occupy a few tens of megabytes; this is not a streaming converter.
* Nothing is guessed: a monitor whose first heartbeats carry no `PING` message needs
  `--initial MONITOR=ADDRESS`, and the script stops with that instruction instead of
  inventing a target.
* The report goes to standard error and the CSV to standard output (or `--output`), so
  the command composes with a pipe:

      python3 scripts/utils/kuma2nm.py export.csv -o /tmp/events.csv
      cat /tmp/events.csv | docker run --rm -i ... network-monitor import -

* Tests: `python3 -m unittest discover -s scripts/utils -p 'test_*.py'`.

# Example: the migration this script was written for

    python3 scripts/utils/kuma2nm.py ~/Downloads/kuma-events.csv \\
        --initial "DNS ping=1.1.1.1" --initial "Gateway ping=192.168.1.1" \\
        --alias 8.8.8.8=1.1.1.1 \\
        --label 1.1.1.1=cloudflare-dns --label 192.168.1.1=gateway \\
        -o /tmp/kuma-network-monitor.csv
"""

from __future__ import annotations

import argparse
import csv
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Dict, List, Optional, Tuple

HEADER = "target,label,interface,state,occurred_at,rtt_us"

#: Kuma status codes that map onto a state `network-monitor` knows.
STATES = {"1": "up", "0": "down"}


def parse_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Convert an Uptime Kuma heartbeat export into the network-monitor "
        "import format.",
        epilog="The module documentation states the mapping and the caveats.",
    )
    parser.add_argument("source", type=Path, help="the Uptime Kuma CSV")
    parser.add_argument(
        "-o", "--output", type=Path, help="write the CSV here instead of standard output"
    )
    parser.add_argument(
        "--initial",
        action="append",
        default=[],
        metavar="MONITOR=ADDRESS",
        help="address of a monitor before its first down heartbeat; repeatable",
    )
    parser.add_argument(
        "--alias",
        action="append",
        default=[],
        metavar="ADDRESS=TARGET",
        help="record a measured address under another target; repeatable",
    )
    parser.add_argument(
        "--label",
        action="append",
        default=[],
        metavar="ADDRESS=LABEL",
        help="label of an address, as the page shows it; repeatable",
    )
    parser.add_argument(
        "--skip",
        action="append",
        default=[],
        metavar="ADDRESS",
        help="drop the heartbeats of this address; repeatable",
    )
    return parser.parse_args(argv)


def pairs(values: List[str], option: str) -> Dict[str, str]:
    """Read the repeated `KEY=VALUE` options, failing loudly on a malformed one."""
    result: Dict[str, str] = {}
    for value in values:
        key, separator, rest = value.partition("=")
        if not separator or not key.strip() or not rest.strip():
            raise ValueError("%s needs KEY=VALUE, not %r" % (option, value))
        result[key.strip()] = rest.strip()
    return result


def address_of(message: str) -> Optional[str]:
    """The address a ping message names: `PING 1.1.1.1 (1.1.1.1) 56(84) bytes ...`."""
    marker = message.find("PING ")
    if marker < 0:
        return None
    rest = message[marker + len("PING ") :].strip()
    address = rest.split(" ", 1)[0].split("(", 1)[0].strip()
    parts = address.split(".")
    if len(parts) != 4 or not all(part.isdigit() for part in parts):
        return None
    return address


def instant(text: str) -> str:
    """`2026-07-05 16:23:47.590` as an RFC 3339 instant in UTC."""
    date, _, clock = text.strip().partition(" ")
    if not date or not clock:
        raise ValueError("unreadable time %r" % text)
    if "." not in clock:
        clock = clock + ".000"
    return "%sT%sZ" % (date, clock)


def micros(ping: str) -> str:
    """Kuma's ping in milliseconds, as the microseconds the store keeps."""
    if not ping.strip():
        return ""
    try:
        return str(int((Decimal(ping.strip()) * 1000).to_integral_value()))
    except InvalidOperation:
        raise ValueError("unreadable ping %r" % ping)


def convert(
    rows: List[Dict[str, str]],
    initial: Dict[str, str],
    aliases: Dict[str, str],
    labels: Dict[str, str],
    skipped: set,
) -> Tuple[List[Tuple[str, str, str]], Dict[str, int], Dict[str, int]]:
    """Rewrite the heartbeat rows.

    Returns the lines as `(target, instant, line)`, the counters, and the heartbeat count
    per written target. `initial` seeds the address of a monitor, `aliases` maps a
    measured address onto the target it is recorded as, and `skipped` holds the measured
    addresses to leave out.
    """
    targets: Dict[str, str] = dict(initial)
    output: List[Tuple[str, str, str]] = []
    counts = {"rows": 0, "kept": 0, "skipped_status": 0, "skipped_address": 0}
    per_target: Dict[str, int] = {}

    for row in rows:
        counts["rows"] += 1
        monitor = row["monitor_name"]

        address = address_of(row["message"])
        if address is not None and address != targets.get(monitor):
            if monitor in targets:
                note(
                    "%s moves from %s to %s at %s"
                    % (monitor, targets[monitor], address, row["time"])
                )
            targets[monitor] = address

        measured = targets.get(monitor)
        if measured is None:
            raise LookupError(
                '%s has no address before %s: pass --initial "%s=<address>"'
                % (monitor, row["time"], monitor)
            )
        target = aliases.get(measured, measured)

        state = STATES.get(row["status"].strip())
        if state is None:
            counts["skipped_status"] += 1
            continue
        if measured in skipped or target in skipped:
            counts["skipped_address"] += 1
            continue

        at = instant(row["time"])
        output.append(
            (
                target,
                at,
                "%s,%s,,%s,%s,%s"
                % (target, labels.get(target, ""), state, at, micros(row["ping"])),
            )
        )
        counts["kept"] += 1
        per_target[target] = per_target.get(target, 0) + 1

    return output, counts, per_target


def note(text: str) -> None:
    print("note: " + text, file=sys.stderr)


def main(argv: Optional[List[str]] = None) -> int:
    args = parse_args(argv)
    try:
        initial = pairs(args.initial, "--initial")
        aliases = pairs(args.alias, "--alias")
        labels = pairs(args.label, "--label")
        skipped = set(args.skip)
    except ValueError as error:
        print(error, file=sys.stderr)
        return 2

    try:
        with args.source.open(newline="", encoding="utf-8") as handle:
            rows = list(csv.DictReader(handle))
    except OSError as error:
        print("cannot read %s: %s" % (args.source, error), file=sys.stderr)
        return 2
    if not rows:
        print("%s holds no row" % args.source, file=sys.stderr)
        return 2

    # Kuma orders by id; the output wants the instants, and so does the report.
    rows.sort(key=lambda row: row["time"])

    try:
        output, counts, per_target = convert(rows, initial, aliases, labels, skipped)
    except (LookupError, ValueError) as error:
        print(error.args[0] if error.args else error, file=sys.stderr)
        return 2

    output.sort(key=lambda entry: (entry[0], entry[1]))
    text = "\n".join([HEADER] + [line for _, _, line in output]) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)

    for measured, target in sorted(aliases.items()):
        if measured != target:
            note("%s is recorded as %s" % (measured, target))
    print(
        "%d rows read, %d written, %d with another status, %d of a skipped address"
        % (
            counts["rows"],
            counts["kept"],
            counts["skipped_status"],
            counts["skipped_address"],
        ),
        file=sys.stderr,
    )
    for target, count in sorted(per_target.items()):
        print("  %s: %d heartbeats" % (target, count), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
