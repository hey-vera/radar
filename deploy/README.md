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
**Do them in this order**, and do not reorder them — see the warning under step 3.

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

### 3. The web route — only after DNS resolves

```bash
cat deploy/radar.heyvera.org.caddy >> /home/guardian/claw-net/Caddyfile
sudo cp /home/guardian/claw-net/Caddyfile /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

Both sudo commands are already in the NOPASSWD list, so this step is the one an
agent *can* do unattended — and should not, before DNS exists.

**Why the order matters.** Adding the site block makes Caddy immediately try to
obtain a certificate for `radar.heyvera.org`. If the name does not resolve, ACME
fails and retries, and those failures count against the rate limits of the
**same Let's Encrypt account that holds the certificates for `heyvera.org`,
`cortex.heyvera.org` and `api.heyvera.org`**. A route that does nothing is not
worth putting a shared certificate account near a rate limit for.

The reload itself is safe in isolation: Caddy validates the new config and keeps
the running one if it does not parse, so a malformed block costs a failed reload
rather than an outage. The certificate account is the part that is shared.

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

## Running without the units

Both binaries run fine under `setsid nohup` before the systemd units are
installed, which is how the first deployment was done:

```bash
cd ~/radar
setsid nohup ~/bin/radar-backfill --follow --store ~/radar/data/store     --window-minutes 5 > ~/radar/follow.log 2>&1 < /dev/null &
RADAR_STORE=~/radar/data/store RADAR_BIND=127.0.0.1:8402     setsid nohup ~/bin/radar-serve > ~/radar/serve.log 2>&1 < /dev/null &
```

Measured on `clawguard`: the recorder holds ~10 MB and the server ~7 MB, both at
roughly 0% CPU — neither appears in the top consumers on a box also running
Cortex and Pulse. The systemd units are for restart-on-failure and boot
persistence, not for resource containment.

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


## The signer — only when capital is actually going to be deployed

**Do not install this until Josh decides to trade.** Radar ships with
`Policy::CLOSED`, the risk kernel refuses every proposal, and a signer with no
policy to serve is a key on a box for no reason. Installing it early buys
nothing and adds a secret to defend.

When that changes, the property to preserve is the one the whole design rests on:
**the executor cannot read the key file.** That is why the executor does not spawn
the signer — a child process would inherit the executor's user, and the key would
have to be readable by it.

### 1. A user of its own

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin radar-signer
sudo groupadd -f radar
sudo usermod -a -G radar guardian
```

`radar` is the group the socket is shared through, and `guardian` — which runs
the executor — is the only other member. That group membership is the entire
access-control policy.

### 2. The key, readable by nobody else

Generate it **off this box**, on a machine that is not running other people's
production services, and move it in. A key generated on the host it will run on
has already touched that host's shell history, page cache and any backup that ran
in between.

```bash
scp signer.json guardian-vps-tail:/tmp/signer.json
ssh guardian-vps-tail '
  sudo install -D -m600 -o radar-signer -g radar-signer /tmp/signer.json /etc/radar/signer.json &&
  shred -u /tmp/signer.json'
```

Check it: `sudo -u guardian cat /etc/radar/signer.json` must fail. If it
succeeds, stop — the separation is decorative and the rest of this is theatre.

### 3. The units

```bash
sudo install -D -m644 deploy/radar-signer.socket /etc/systemd/system/radar-signer.socket
sudo install -D -m644 deploy/radar-signer@.service /etc/systemd/system/radar-signer@.service
sudo install -m640 -o radar-signer -g radar-signer deploy/signer.env.example /etc/radar/signer.env
sudo install -m755 dist/radar-signer /usr/local/bin/radar-signer
sudo systemctl daemon-reload
sudo systemctl enable --now radar-signer.socket
```

Fill in `RADAR_SIGNER_PROGRAMS` before enabling. An empty allowlist refuses
everything, which is safe but useless; there is deliberately no permissive
default, because that is the one misconfiguration here with no upper bound on
its cost.

### 4. Verify it refuses

The signer is worth having only if it says no. Ask it to sign something it
should not:

```bash
printf '%s\n' '{"authorization":{"nonce":"probe","mint":"So11111111111111111111111111111111111111112","action":"buy","max_notional":1,"expires_after":0,"needs_operator_signature":false},"transaction":"AAAA","now_slot":999999999}' \
  | sudo -u guardian nc -U /run/radar/signer.sock
```

Expect `{"outcome":"refused",...}`. A `signed` here means the binary on the box
is not the one in this repository.

Then confirm it has no network at all:

```bash
ssh guardian-vps-tail 'sudo ss -tlnp | grep radar-signer'
```

That must print nothing. If it prints anything, something was added that should
not have been.
