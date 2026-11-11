# Importing an Uptime Kuma history

Runbook for moving a history out of Uptime Kuma and into a `network-monitor` database,
or for merging a Kuma history into one that already records transitions. The import
command, its options, and the merge semantics are in
[DEVELOPER-GUIDE.md](./DEVELOPER-GUIDE.md#restoring-the-history-after-a-redeploy); this
document covers only the Kuma side.

The conversion is done by [scripts/utils/kuma2nm.py](../scripts/utils/kuma2nm.py). Its
module documentation is the reference for the field mapping and for the caveats; the
steps below are the operation itself.

## 1. Check the Kuma clock

Kuma writes the heartbeat time as a naive local-time string, produced by the clock of the
Kuma process. The official image runs on UTC unless `TZ` is set, and this runbook assumes
that:

```bash
docker -c qubo exec uptime-kuma date   # must print UTC
```

If it does not, convert the timestamps before importing: the converter adds `Z` and
carries no offset on purpose, because a wrong offset corrupts every row silently.

## 2. Export the heartbeats from the Kuma database

Take the rows from Kuma's own SQLite database, not from the CSV the status page offers
for download: the page export is a snapshot of what the browser was shown, and in this
installation it stopped three days before the database did. The database is
`/app/data/kuma.db` inside the Kuma container.

Start by checking what is there:

```bash
docker -c qubo exec -i uptime-kuma sqlite3 -header -column /app/data/kuma.db "
SELECT m.name, h.monitor_id, COUNT(*) heartbeats, MIN(h.time) first, MAX(h.time) last
FROM heartbeat h JOIN monitor m ON h.monitor_id = m.id
GROUP BY h.monitor_id;"
```

Then write the CSV the converter reads, with the same columns Kuma's page would have
written — `id,monitor_name,status,time,message,ping`:

```bash
docker -c qubo exec -i uptime-kuma sqlite3 -header -csv /app/data/kuma.db "
SELECT h.id,
       m.name AS monitor_name,
       h.status,
       h.time,
       COALESCE(h.msg, '') AS message,
       CASE WHEN h.ping IS NULL THEN '' ELSE CAST(h.ping AS TEXT) END AS ping
FROM heartbeat h
JOIN monitor m ON h.monitor_id = m.id
ORDER BY h.time, h.id;" > tmp/import/kuma-heartbeats.csv
```

The count of physical lines is not the count of rows: a ping message spans several lines
and SQLite quotes it as one field, so `wc -l` overstates the heartbeats. Count them with
the converter's report or with a CSV reader.

The database and the page export can disagree in two ways, and the database is the one
to trust:

- **Coverage.** Kuma prunes the heartbeats it keeps, but it keeps every state change, so
  the old months are transitions only while the last days hold every probe. A page export
  taken earlier is both shorter and older.
- **The monitor list.** A monitor that has since been deleted, or repointed, is visible
  in `monitor`; check `SELECT id, name, hostname, active FROM monitor` before converting,
  because the address in the messages is what decides the target.

`heartbeat.important` is `1` on a state change; selecting `WHERE h.important = 1` gives a
much smaller file, and it is the same chain the import would end up with. Keep the full
export unless size is a problem: the importer collapses the repeats anyway (`BR-1`), and
leaving them lets the conversion stay a plain field mapping.

## 3. Look at the file

One row per heartbeat, ordered by id:

```csv
id,monitor_name,status,time,message,ping
101273,"DNS ping",0,"2026-09-01 10:00:00.000","PING 1.1.1.1 (1.1.1.1) 56(84) bytes of data. ...", 
101277,"DNS ping",1,"2026-09-01 10:00:20.000","",6.56
101280,"Gateway ping",0,"2026-09-01 10:05:00.000","PING 192.168.1.1 (192.168.1.1) ... From 192.168.2.1 icmp_seq=1 Destination Net Unreachable ...", 
101282,"Gateway ping",1,"2026-09-01 10:05:20.000","",1.12
101300,"DNS ping",3,"2026-09-01 10:10:00.000","Maintenance: router firmware upgrade",""
101310,"DNS ping",0,"2026-09-02 08:00:00.000","PING 8.8.8.8 (8.8.8.8) ...", 
101312,"DNS ping",1,"2026-09-02 08:00:20.000","",7.4
101314,"DNS ping",1,"2026-09-02 08:00:40.000","",7.2
```

[scripts/utils/kuma-example.csv](../scripts/utils/kuma-example.csv) holds these rows with
the ping messages unabridged, so the command below can be tried on it before touching a
real export. The file shows the four cases that matter:

- the address lives in the `message`, in the `PING <address>` line, and an up heartbeat
  carries no message, so it inherits the address of its monitor;
- the ICMP text can name another host (`From 192.168.2.1`): only the `PING` line counts;
- `status` is `1` for up, `0` for down, and other values are skipped;
- a monitor can be repointed, and its history then splits at that instant.

## 4. Convert

```bash
python3 scripts/utils/kuma2nm.py ~/Downloads/kuma-events.csv \
    --initial "DNS ping=1.1.1.1" --initial "Gateway ping=192.168.1.1" \
    --alias 8.8.8.8=1.1.1.1 \
    --label 1.1.1.1=cloudflare-dns --label 192.168.1.1=gateway \
    -o /tmp/kuma-network-monitor.csv
```

`--initial` seeds the address of a monitor whose first heartbeats carry no message, and
the command stops and asks for it when one is missing. `--alias` records a measured
address under another target: in this history the `DNS ping` monitor moved from `1.1.1.1`
to `8.8.8.8` on 2026-08-17, and the alias keeps the two eras as one `1.1.1.1` lane.
`--skip <address>` leaves an address out instead. Use `--alias` and `--skip` only when
that is the story the reader should see, and say so in the commit message.

On the example file the report and the output are:

```
note: DNS ping moves from 1.1.1.1 to 8.8.8.8 at 2026-09-02 08:00:00.000
note: 8.8.8.8 is recorded as 1.1.1.1
8 rows read, 7 written, 1 with another status, 0 of a skipped address
  1.1.1.1: 5 heartbeats
  192.168.1.1: 2 heartbeats
```

```csv
target,label,interface,state,occurred_at,rtt_us
1.1.1.1,cloudflare-dns,,down,2026-09-01T10:00:00.000Z,
1.1.1.1,cloudflare-dns,,up,2026-09-01T10:00:20.000Z,6560
1.1.1.1,cloudflare-dns,,down,2026-09-02T08:00:00.000Z,
1.1.1.1,cloudflare-dns,,up,2026-09-02T08:00:20.000Z,7400
1.1.1.1,cloudflare-dns,,up,2026-09-02T08:00:40.000Z,7200
192.168.1.1,gateway,,down,2026-09-01T10:05:00.000Z,
192.168.1.1,gateway,,up,2026-09-01T10:05:20.000Z,1120
```

Repeated states stay in the file: the importer drops them in one pass (`BR-1`), which is
why the report of the import is smaller than the number of rows.

The tests of the converter run without any dependency:

```bash
python3 -m unittest discover -s scripts/utils -p 'test_*.py'
```

## 5. Correcting an import already made

A wrong or partial archive is fixed by replacing the window it covers instead of adding
to it. The window belongs to the target: `--replace-window` deletes, for every target the
archive mentions, the events between that target's first and last instant in the file,
then writes the archive. A target's history before and after its own window stays, and a
target the archive does not mention keeps everything.

```bash
cat tmp/import/kuma-network-monitor.csv | docker -c qubo run --rm -i \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  import --replace-window --database /var/lib/network-monitor/events.db -
```

The command reports what it removed first:

```
replaced 4653 events inside the window of each target (2026-07-03T09:38:16.640Z to
2026-11-11T12:20:42.136Z), then imported 32119 events for 2 target(s) ... (0 rows
skipped, 24551 events dropped as repeats of the state before them, plain csv format)
```

The parentheses name the span that holds every window, not one window shared by all: each
target's own window ends at that target's newest row, so the transitions the monitor
recorded for it after that instant survive. The deletion is not reversible: take the
backup the developer guide requires before every import
([back up before you import](./DEVELOPER-GUIDE.md#back-up-before-you-import)) and keep it
until the page shows what you expect.

This scoping is ADR-0020, and it exists because the earlier behaviour destroyed data here:
the window used to be one span for the whole database, the `DNS ping` monitor stopped on
2026-11-07 while `Gateway ping` kept writing until 2026-11-11, and replacing the window
erased four days of native `1.1.1.1` events that the archive never mentioned. A `serve`
binary older than that change still has the old semantics: check `query state` and the
page after a replacement, and if rows went missing, restore them from the backup taken
before it:

```bash
# the native events of the backup, in the import format
sqlite3 backups/events.db "
SELECT target,
       COALESCE((SELECT label FROM targets t WHERE t.target = e.target), ''),
       COALESCE(interface, ''),
       state,
       printf('%s.%03dZ', strftime('%Y-%m-%dT%H:%M:%S', occurred_at_ms / 1000, 'unixepoch'),
              occurred_at_ms % 1000),
       COALESCE(CAST(rtt_us AS TEXT), '')
FROM events e ORDER BY target, occurred_at_ms;" > /tmp/native.csv

{ echo "target,label,interface,state,occurred_at,rtt_us"; cat /tmp/native.csv; } > /tmp/native-import.csv
cat /tmp/native-import.csv | docker -c qubo run --rm -i \
  -v network-monitor-data:/var/lib/network-monitor network-monitor:dev \
  import --database /var/lib/network-monitor/events.db -
```

## 6. Import into the database

The monitor decides a transition from the state it loaded at startup, so the import runs
with the monitor stopped. Back up first, as
[the developer guide requires](./DEVELOPER-GUIDE.md#back-up-before-you-import):
stop the monitor, copy `events.db` aside, keep that file. Then, still with the monitor
stopped:

```bash
# import from the pipe, in a one-off container on the same volume
cat /tmp/kuma-network-monitor.csv | docker -c qubo run --rm -i \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  import --database /var/lib/network-monitor/events.db -

# start again
docker -c qubo start network-monitor
```

The expected report is `imported <rows> events for <n> target(s) ... (<d> events dropped
as repeats of the state before them, plain csv format)`.

## 7. Verify

```bash
docker -c qubo run --rm -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev query state --database /var/lib/network-monitor/events.db
curl -s http://qubo:8642/healthz
```

Then open `http://qubo:8642/target?target=<address>`: the last 30 days now hold the Kuma
days as well, and the totals of the period include them. Check the labels in `query
state` too: a null label means the live monitor has to be started with `label=` (see
[Labels belong to the live configuration](#labels-belong-to-the-live-configuration)).

Re-running an import is harmless in outcome but not free: the rows that repeat a state
are inserted again and then dropped by the normalisation, so a second run of a 32 000-row
file touches as many rows as the first. Use `--replace-window` when the point is to
correct an earlier import, and a plain merge only to append a newer export.

## 8. Rolling back

Put the backup taken before the import back and restart:

```bash
docker -c qubo stop network-monitor
docker -c qubo cp backups/events-<stamp>.db network-monitor:/var/lib/network-monitor/events.db
docker -c qubo start network-monitor
```

## Reference: closing a measurement gap with an `unknown` row

An imported chain carries its last state forward. When a Kuma monitor stopped hours or
days before the live monitor started, that carried state describes a stretch nobody
measured: in this installation a stale `down` turned the 35 hours between the two chains
into a day of down time that never happened.

The import format has no `unknown`, so the marker is a row written once, with the
monitor stopped:

```bash
# stop, then edit the file in place
docker -c qubo stop network-monitor
docker -c qubo cp network-monitor:/var/lib/network-monitor/events.db ./events.db
LAST=$(sqlite3 ./events.db "SELECT MAX(occurred_at_ms) FROM events
        WHERE target='1.1.1.1' AND occurred_at_ms < <first native event>")
sqlite3 ./events.db "INSERT INTO events (target, interface, previous_state, state, occurred_at_ms, rtt_us)
  VALUES ('1.1.1.1', 'eth1', 'down', 'unknown', $LAST + 1, NULL);"
docker -c qubo cp ./events.db network-monitor:/var/lib/network-monitor/events.db

# let the store recompute the chain: a one-row import of an event already stored
# inserts nothing and runs the normalisation
cat normalize.csv | docker -c qubo run --rm -i \
  -v network-monitor-data:/var/lib/network-monitor network-monitor:dev \
  import --database /var/lib/network-monitor/events.db -
docker -c qubo start network-monitor
```

The marker is drawn grey like any `unknown` band, and `up_permille` leaves it out of the
availability instead of counting it as down. In this installation it moved the 30-day
down time from `4d01h` to `2d14h`, all of it difference in unmeasured hours.

This is a repair, not a documented feature: nothing in the binary writes an `unknown`
event. If it becomes routine, the honest fix is to let `import` accept an `unknown` state
and let the Kuma converter emit the marker.

## Labels belong to the live configuration

Every transition rewrites the target row from the configuration the monitor loaded, and
the label is part of that row: a monitor started without `label=` clears a label that the
import set, at the first drop. Give the live monitor the same label it should show —
`--target 1.1.1.1,interface=eth1,label=cloudflare-dns` — and the import only fills the gap
for the period before the monitor started.

## What the import does not undo

- **Labels already set win.** The store fills a target's label only when it is empty, so
  `--label` names new targets and leaves existing ones alone.
- **A measurement gap is not a state.** Where the export has no heartbeat the chain
  continues with the last state it knows. If the first live event of a target repeats
  the imported state, the importer folds it away and the page reports the state from the
  last imported change instead of from the first live probe.
- **Retention still applies.** Import a history you want to keep with `retention_days`
  disabled, or accept that the next retention pass trims it.
