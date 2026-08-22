<!-- SPDX-License-Identifier: Apache-2.0 -->
# Deploying Radar

Radar runs on the shared `clawguard` VPS alongside Cortex and Pulse. That shapes
every choice here: it is a 2-core box with 3.8 GiB of RAM running other people's
production services, so Radar is memory-capped, sandboxed, and installed without
touching anything already there.

There is no Rust toolchain on the server and there should not be one — building
arrow and parquet on two cores would take the box down. Binaries are built by the
`release-linux` workflow and downloaded from it.

## What CI does not do, on purpose

The workflow **builds** and **does not ship**. Giving a GitHub Actions job an SSH
key to a host running Cortex and Pulse would put those services one workflow
compromise away from a stranger, and the benefit is saving one `scp`. The
artifact is downloaded and placed by whoever already has that access.

## First install

Three steps need a human, because each one changes something Radar does not own.

### 1. DNS — `radar.heyvera.org`

Add an `A`/`CNAME` record pointing at the same origin as `cortex.heyvera.org`.
Caddy will obtain the certificate automatically once the record resolves.

### 2. The service unit and its environment

```bash
sudo install -D -m644 deploy/radar-serve.service /etc/systemd/system/radar-serve.service
sudo install -d -m755 /etc/radar
sudo install -m640 -g guardian deploy/radar.env.example /etc/radar/radar.env
sudo systemctl daemon-reload
sudo systemctl enable --now radar-serve
```

`guardian`'s NOPASSWD sudoers list covers `daemon-reload` and the Caddy steps but
not installing a new unit, so this one needs an interactive `sudo`. That is the
right boundary — an automated agent should not be able to add a service to a
production host on its own.

Optionally add a NOPASSWD line so later upgrades need no password:

```
guardian ALL=(root) NOPASSWD: /usr/bin/systemctl restart radar-serve, \
    /usr/bin/systemctl status radar-serve, /usr/bin/install -m755 /tmp/radar-serve /usr/local/bin/radar-serve
```

### 3. The web route

```bash
cat deploy/radar.heyvera.org.caddy >> /home/guardian/claw-net/Caddyfile
sudo cp /home/guardian/claw-net/Caddyfile /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

Both sudo commands are already in the NOPASSWD list.

## Every deploy after that

```bash
gh run download --repo hey-vera/radar --name radar-linux-x86_64 --dir ./dist
sha256sum -c <(awk 'NF==2 {print $1"  ./dist/"$2}' dist/BUILD-INFO.txt)

scp dist/radar-serve dist/radar dist/radar-backfill guardian-vps-tail:/tmp/
ssh guardian-vps-tail '
  sudo install -m755 /tmp/radar-serve /usr/local/bin/radar-serve &&
  install -m755 /tmp/radar /tmp/radar-backfill ~/bin/ &&
  sudo systemctl restart radar-serve &&
  sleep 2 && curl -sS localhost:8402/health'
```

The `curl` is the point. A deploy that does not end by asking the service what it
thinks it is running has not been verified, and `/health` reports the version,
the instrument count and the store watermark.

## Filling the store

The store starts empty, and an empty store answers "cannot answer as of any
slot" rather than pretending to know nothing happened — so a fresh install serves
`503` until it is backfilled.

```bash
ssh guardian-vps-tail '
  mkdir -p ~/radar/data/store &&
  ~/bin/radar-backfill --from "2026-08-01 00:00:00" --to "2026-08-22 00:00:00" \
      --store ~/radar/data/store --window-minutes 30'
```

At roughly 24,000 launches a day and a thousand-row cap per query this is a few
thousand queries and several hours. It paces itself deliberately — Radar is a
guest on a free public endpoint (ADR 0002) — so run it under `tmux` or `nohup`
rather than a session that will disconnect.

## Keeping it current

`--follow` picks up where the store left off and keeps going, staying five
minutes behind the chain. It is the same extraction path as a one-off backfill,
so history and live data remain one code path and a replay still means something.

```bash
sudo install -D -m644 deploy/radar-follow.service /etc/systemd/system/radar-follow.service
sudo systemctl daemon-reload
sudo systemctl enable --now radar-follow
journalctl -u radar-follow -f
```

It is a separate unit from `radar-serve` because the recorder writes and the
server reads, and a crash in one should not take the other down. The store is
append-only, so both may hold it at once.

## Disk

Recorded events run about 1 GiB a month. The box had 48 GiB free at the time of
writing, and Radar is capped to its own directory by the unit's `ReadWritePaths`.
Check before a large backfill:

```bash
ssh guardian-vps-tail 'df -h / | tail -1; du -sh ~/radar/data 2>/dev/null'
```

## Rolling back

Binaries are plain files and the store is append-only, so a rollback is a copy
and a restart. Nothing in the store needs migrating backwards — a partition
written by a newer version is still readable by an older one unless a column was
added, and columns are only ever added.

```bash
ssh guardian-vps-tail 'sudo install -m755 /tmp/radar-serve.previous /usr/local/bin/radar-serve && sudo systemctl restart radar-serve'
```
