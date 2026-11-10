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
docker -c qubo build -t network-monitor:dev .
```

```bash
docker -c qubo run -d --name network-monitor \
  --cap-drop ALL --cap-add NET_RAW --network host \
  --restart unless-stopped \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  monitor --target 1.1.1.1,interface=eth1 --target 192.168.1.1 \
  --database /var/lib/network-monitor/events.db \
  --listen 0.0.0.0:8642
```

`-p 8642:8642` is useless next to `--network host`: Docker ignores it and warns. To
republish the port, drop `--network host` — but then `interface = "eth1"` no longer
refers to a physical interface, so keep host networking for this deployment.

To recreate the container, remove it first, otherwise `run` fails on the taken name:

```bash
docker -c qubo rm -f network-monitor
```

## Switching from B to A

The history is kept, because the compose file pins the volume with `name:
network-monitor-data` — without it Compose would create
`network-monitor_network-monitor-data` and the monitor would start from an empty
database.

The container, on the other hand, must be removed first: the one started by `docker
run` has no Compose labels, so Compose does not recognise it and cannot replace it,
and `create` fails with `Conflict. The container name "/network-monitor" is already
in use`. One transition, then A only:

```bash
docker -c qubo rm -f network-monitor
docker -c qubo compose up -d --build
```

Compose warns `volume "network-monitor-data" already exists but was not created by
Docker Compose` on every `up`: expected, the volume predates the project. It is a
warning, not an error, and it disappears if the volume is ever created by Compose
itself. `external: true` would silence it, but then the volume has to exist on a
fresh host, which is what A is meant to avoid.

## Reading the state, in both modes

The runtime image is `scratch`: no shell, no `sqlite3`, but the binary itself runs.

```bash
docker -c qubo exec network-monitor \
  /network-monitor query state --database /var/lib/network-monitor/events.db
```
