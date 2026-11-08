# Notes

```bash
docker -c qubo build -t network-monitor:dev .
```

```bash
docker -c qubo exec network-monitor \        
  /network-monitor query state --database /var/lib/network-monitor/events.db
```

```bash
docker -c qubo run -d --name network-monitor \
  --cap-drop ALL --cap-add NET_RAW --network host \
  --restart unless-stopped \
  -p 8642:8642 \
  -v network-monitor-data:/var/lib/network-monitor \
  network-monitor:dev \
  monitor --target 1.1.1.1,interface=eth1 --target 192.168.1.1 \
  --database /var/lib/network-monitor/events.db \
  --listen 0.0.0.0:8642
```

```bash
docker -c qubo rm -f network-monitor
```
