# Notes

Two ways to bring the container up to date on a Docker context. A is the declared
deployment, B is the same thing by hand.

## A. Compose (self-contained, the default)

[docker-compose.yml](./docker-compose.yml) holds the whole definition — image build,
capabilities, host networking, targets, volume — so nothing has to exist on the daemon
beforehand: no configuration file, no volume, no image.

```bash
docker -c qubo compose up -d --build     # build on the NAS, recreate, start
docker -c qubo compose logs -f           # follow the monitor
docker -c qubo compose down              # stop and remove; the volume stays
```

That first command is also the update of an already running project; the variants
(`--force-recreate`, `build` alone, `--pull always`) are in
[DEVELOPER-GUIDE.md](./docs/DEVELOPER-GUIDE.md#rebuilding-a-project-that-is-already-running).

## B. By hand, with docker run

Targets on the command line, so no configuration file is mounted: with a remote context
a bind mount is resolved on the daemon host, not on this machine.

```bash
docker -c qubo build -t uptime-monitor:dev .
```

```bash
docker -c qubo run -d --name uptime-monitor \
  --cap-drop ALL --cap-add NET_RAW --network host \
  --restart unless-stopped \
  --log-opt max-size=10m --log-opt max-file=3 \
  -v uptime-monitor-data:/var/lib/uptime-monitor \
  uptime-monitor:dev \
  monitor --target 1.1.1.1,interface=eth1 --target 192.168.1.1 \
  --database /var/lib/uptime-monitor/events.db \
  --listen 0.0.0.0:8642
```

`--log-opt max-size=10m --log-opt max-file=3` bounds the engine's copy of stdout and
stderr, which the `json-file` driver otherwise keeps for ever. The tool writes no log file
of its own; a `logfile` channel is bounded by its own `max_bytes` and `max_files`.

`-p 8642:8642` is useless next to `--network host`: Docker ignores it and warns. To
republish the port, drop `--network host` — but then `interface = "eth1"` no longer
refers to a physical interface, so keep host networking for this deployment.

To recreate the container, remove it first, otherwise `run` fails on the taken name:

```bash
docker -c qubo rm -f uptime-monitor
```

## Migrating a deployment that carries the old name

A deployment from 0.12 keeps its history in the `network-monitor-data` volume and its
configuration under `/etc/network-monitor`. The database schema is unchanged, so the
history survives a copy. Stop the old container, copy the volume once, then start the
new definition:

```bash
docker -c qubo rm -f network-monitor              # stop the old container; the volume stays
docker -c qubo volume create uptime-monitor-data
docker -c qubo run --rm -v network-monitor-data:/from:ro \
  -v uptime-monitor-data:/to alpine cp -a /from/. /to/
docker -c qubo compose up -d --build              # new names, same history
```

Keep `network-monitor-data` until a `query state` against the new volume answers the
history you expect, then remove it. A configuration file is an ordinary file: rename it
and its `database` path, or keep the explicit paths — only the defaults changed.

## Switching from B to A

The history is kept, because the compose file pins the volume with `name:
uptime-monitor-data` — without it Compose would create
`uptime-monitor_uptime-monitor-data` and the monitor would start from an empty
database.

The container, on the other hand, must be removed first: the one started by `docker
run` has no Compose labels, so Compose does not recognise it and cannot replace it,
and `create` fails with `Conflict. The container name "/uptime-monitor" is already
in use`. One transition, then A only:

```bash
docker -c qubo rm -f uptime-monitor
docker -c qubo compose up -d --build
```

Compose warns `volume "uptime-monitor-data" already exists but was not created by
Docker Compose` on every `up`: expected, the volume predates the project. It is a
warning, not an error, and it disappears if the volume is ever created by Compose
itself. `external: true` would silence it, but then the volume has to exist on a
fresh host, which is what A is meant to avoid.

## Reading the state, in both modes

The runtime image is `scratch`: no shell, no `sqlite3`, but the binary itself runs.

```bash
docker -c qubo exec uptime-monitor \
  /uptime-monitor query state --database /var/lib/uptime-monitor/events.db
```

## Notification log files in the container

A `logfile` notification channel writes to a path inside the container. The declared
volume is mounted at `/var/lib/uptime-monitor`, so a file there survives a recreate:

```toml
[[notification]]
name = "audit"
type = "logfile"
filename = "/var/lib/uptime-monitor/audit.log"
```

Any other directory needs its own mount and must exist before the monitor starts: a channel
that cannot open the file stops the process.
