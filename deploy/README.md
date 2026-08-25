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

## Verifying the deployment (checked 2026-08-25)

Every claim here is checkable with one command, and the command is given rather
than the answer.

**An earlier version of this section asserted a state instead.** It described
`radar-serve` running as a hand-started process, was correct on the day it was
written, and stayed in the file after the host was fixed — telling the next
operator that *First install* step 2 "has never been run" when it had, and that
the deploy procedure "fails today" when it works. A runbook that states a remote
host's state has a half-life. One that states how to ask does not.

```bash
ssh guardian-vps-tail 'systemctl list-unit-files "radar*" --no-pager'
```

Expected, and true on 2026-08-25:

| unit | state |
|---|---|
| `radar-serve.service` | enabled, active |
| `radar-follow.service` | enabled, active |
| `radar-brief.timer` | enabled, active |
| `radar-brief.service` | static — the timer starts it |
| `/etc/systemd/system/radar-hosted.service` | disabled, and **not Radar** — see below |

`radar-serve` should be in the system slice with its sandbox applied:

```bash
ssh guardian-vps-tail 'systemctl show radar-serve \
  -p ControlGroup -p Restart -p MemoryMax -p ProtectSystem -p NoNewPrivileges'
```

```
ControlGroup=/system.slice/radar-serve.service
Restart=always
MemoryMax=805306368
ProtectSystem=strict
NoNewPrivileges=yes
```

**A `ControlGroup` under `/user.slice/user-1000.slice/session-NNNNNN.scope` is
the failure this checks for**: a process started by hand over SSH and reparented
to init. It serves correctly and has none of the sandbox above, does not survive
a reboot, and `loginctl terminate-session` kills it. If you see that, run step 2
of *First install* — it is the situation the unit exists to remove.

- **Name collision, do not be fooled by it.** `/etc/systemd/system/` also holds
  `/etc/systemd/system/radar-hosted.service`, which is disabled and is **not Radar** — it is an
  unrelated Node service for `radar.claw-net.org` (`npx tsx hosted/server.ts`).
  Enabling or restarting it does nothing useful and will not start the API.

`radar brief`'s `serving` check is what stands between a dead API and nobody
noticing. It runs on `radar-brief.timer` every fifteen minutes.

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

**Run these from a workstation that can reach `guardian-vps-tail`, not from the
box.** The name is a Tailscale name and does not resolve on `clawguard` itself,
so running this there fails at the `scp` with `Temporary failure in name
resolution` — and then, if the steps are pasted separately, the `install` reads
whatever `/tmp` already held from a previous deploy and succeeds. That happened on
2026-08-24: a fix was "deployed" and the running binary was two commits old, with
nothing in the output saying so.

```bash
gh run download --repo hey-vera/radar --name radar-linux-x86_64 --dir ./dist
head -2 ./dist/BUILD-INFO.txt   # confirm this is the commit you meant to ship
sha256sum -c <(awk 'NF==2 {print $1"  ./dist/"$2}' dist/BUILD-INFO.txt)

scp dist/radar-serve dist/radar dist/radar-backfill guardian-vps-tail:/tmp/
```

Check the commit line before shipping. `gh run download` takes the most recent
*completed* run, which during a merge is often the previous one.

Install by writing beside the target and renaming, rather than over it. A running
process holds its inode, so a rename never disturbs one mid-write, and the
running binary keeps working until it is restarted:

```bash
ssh guardian-vps-tail '
  set -e
  for b in radar radar-backfill radar-serve; do
    install -m755 /tmp/$b ~/bin/$b.new && mv -f ~/bin/$b.new ~/bin/$b
  done
  sha256sum ~/bin/radar ~/bin/radar-backfill ~/bin/radar-serve'
```

**Compare that output against `BUILD-INFO.txt`.** Verifying the download proves
the artifact arrived; only this proves the artifact is what got installed, and
the two came apart in exactly the way described above.

Then restart, and ask the service what it thinks it is running:

```bash
ssh guardian-vps-tail '
  sudo systemctl restart radar-serve radar-follow &&
  sleep 3 && curl -sS localhost:8402/health'
```

**This `sudo` prompts for a password.** `guardian`'s NOPASSWD sudoers list
covers `claw-net-node`, Caddy, Cortex and Pulse — it has no radar entry, so the
restart needs an interactive session and cannot be run unattended. That is the
right boundary today; see the optional NOPASSWD line under *First install* step 2
if it becomes tedious. An agent that cannot restart a production service on its
own is a feature, not a gap.

The `curl` is the point. A deploy that does not end by asking the service what it
thinks it is running has not been verified, and `/health` reports the version,
the instrument count and the store watermark.

**A running process keeps the old code until it is restarted.** `readlink
/proc/<pid>/exe` prints `(deleted)` when a process is running a binary that has
since been replaced — which is the quickest way to tell a deploy that landed from
one that only looks like it did.

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

## Running without the units — and why not to

Both binaries run fine under `setsid nohup`, which is how the first deployment
was done. **Nothing supervises them.** On 2026-08-24 the recorder exited on a
single bad query at 05:37 and was still down thirteen hours later, found by
someone reading the process list for an unrelated reason. `radar-follow.service`
sets `Restart=always` with `RestartSec=30`, and would have turned that outage
into a thirty-second gap.

Treat this section as what to do for ten minutes of debugging, not as a way to
run the recorder. If you find yourself here for anything longer, install the
unit instead — it is written, it is in this directory, and the only reason it was
not already running is that installing it needs an interactive `sudo`:

```bash
cd ~/radar
setsid nohup ~/bin/radar-backfill --follow --store ~/radar/data/store     --window-minutes 5 > ~/radar/follow.log 2>&1 < /dev/null &
RADAR_STORE=~/radar/data/store RADAR_BIND=127.0.0.1:8402     setsid nohup ~/bin/radar-serve > ~/radar/serve.log 2>&1 < /dev/null &
```

Measured on `clawguard`: the recorder holds ~10 MB and the server ~7 MB, both at
roughly 0% CPU — neither appears in the top consumers on a box also running
Cortex and Pulse. The systemd units are for restart-on-failure and boot
persistence, not for resource containment.

## Knowing when it stopped

`radar-follow.service` restarts a recorder that dies. It cannot help with a
recorder that is running and not advancing, and it cannot tell anyone either way.

`radar brief` decides what unhealthy means and says so through its exit status.
`radar-brief.timer` runs it every fifteen minutes, and `radar-brief.sh` delivers
the answer:

```bash
sudo install -D -m644 deploy/radar-brief.service /etc/systemd/system/radar-brief.service
sudo install -D -m644 deploy/radar-brief.timer   /etc/systemd/system/radar-brief.timer
install  -m755 deploy/radar-brief.sh ~/bin/radar-brief.sh
sudo systemctl daemon-reload
sudo systemctl enable --now radar-brief.timer

# Prove it before trusting it: a store that cannot be healthy must exit non-zero.
mkdir -p /tmp/emptystore && RADAR_STORE=/tmp/emptystore ~/bin/radar-brief.sh; echo "exit=$?"
journalctl -t radar-brief -p err -n 5 --no-pager
```

Fifteen minutes is chosen against the thresholds: the check warns at twenty
minutes of ingestion lag and fails at sixty, so a stopped recorder is noticed
inside the first failing window rather than an hour later.

### Where a failure goes

**The journal, always.** `journalctl -t radar-brief -p err` shows one line per
failing check. This needs no configuration and cannot be switched off.

**A webhook, if one is configured.** Put it in `/etc/radar/alert.env`:

```
RADAR_ALERT_WEBHOOK=https://hooks.slack.com/services/T.../B.../...
```

Unset means no webhook, never a silent pass — the journal line happens
regardless. That is rule 8 applied to alerting: missing configuration must not
turn a failure into a success.

**As of 2026-08-24 no channel is configured on this box, and that is worth
knowing.** `claw-net/scripts/monitoring-check.sh` has been running every five
minutes for months with `SLACK_WEBHOOK_URL`, `ALERT_EMAIL` and
`PAGERDUTY_ROUTING_KEY` all unset, writing to a log nobody reads. A monitor
without a channel is a monitor that only works when someone is already
suspicious — which is how a thirteen-hour recorder outage was found by accident
rather than reported.

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
