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

## Before the next deploy: two settings that gate startup

Two changes landed after 2026-08-27 that a running instance will not have in its
`/etc/radar/radar.env`, and the first of them **stops the binary from starting**.
Set them before installing, not after — this is the same shape as the mistake
that blanked the interface for a few minutes on 2026-08-27, where the binary
shipped ahead of the CSP it needed.

### `radar-serve` refuses to start until somebody decides who may look

There is no default, and that is the point. Serving to everyone looks like it
works; refusing everyone looks like a broken deploy. Neither should be what
happens because nobody chose.

```
sudo tee -a /etc/radar/radar.env >/dev/null <<'ENV'
RADAR_ACCESS_TEAM=heyvera.cloudflareaccess.com
RADAR_ACCESS_AUD=<the Application Audience tag from the Access application>
ENV
```

The AUD tag is on the Access application's **Overview** page. It is per
application, not per team: using the team name, or another application's tag,
means any token from any application in the team opens Radar — with a signature
that verifies perfectly, which is why the audience check is the one worth
getting right.

Radar verifies the **signature** on `Cf-Access-Jwt-Assertion` against
Cloudflare's published keys. It does **not** read
`Cf-Access-Authenticated-User-Email`: that header is a claim by whoever sent the
request, and "the origin is behind a tunnel" is a network topology rather than
an authentication model.

`/health` and `/x402/` stay reachable without a token — the first so uptime
checks do not become false alarms, the second because the paid surface has its
own paywall.

To keep serving openly, say so in as many words instead:

```
echo 'RADAR_ACCESS=off' | sudo tee -a /etc/radar/radar.env
```

Setting both blocks is refused rather than resolved.

**Verify in both directions after restarting**, because a guard checked only in
the refusing direction is indistinguishable from a server that is simply broken:

```
curl -s -o /dev/null -w '%{http_code}
' http://127.0.0.1:8402/health
curl -s -o /dev/null -w '%{http_code}
' http://127.0.0.1:8402/v1/funnel
```

With Access enforced those are `200` and `403`. Through the browser, signed in,
the interface loads as before. If `/health` is not `200`, the unit did not start
— read `journalctl -u radar-serve -n 20`, which will name the missing variable.

### Reading customers' wallets

`RADAR_PRIVY_APP_ID` and `RADAR_PRIVY_APP_SECRET` are both required for
`radar-serve` to read a customer's wallet address. Either one alone is refused:
an id with no secret is a half-finished deployment, and reporting it as
"unconfigured" would hide the mistake behind a message about a feature nobody
turned on.

This credential authenticates Radar as an **application**. It authorises no
signature — that needs the authorization key, which is in the signer's
environment and nowhere else.

Without it, `radar-serve` starts and says so:

```
  wallets    : off (no RADAR_PRIVY_APP_SECRET; wallets cannot be read)
```

That line is printed separately from the `customers` line on purpose. The two can
disagree, and the disagreement is the interesting state: an instance that
verifies customer tokens but cannot read wallets will sign people in and then
fail every lookup, which an operator should see at start rather than from a
support message.

### Wallet ownership, which is a setting and not code

When customer wallets are created, the **customer must be the owner** and Radar
must be only a **signer**.

Privy separates the two: an owner may update policies, change owners, add signers
and export the key; a signer may only sign, within the policy the owner set.

[ADR 0008](../docs/adr/0008-the-signer-holds-its-own-policy.md) names Privy's
policy engine as the independent backstop against a compromised `radar-serve`.
That holds only while Radar is not the owner — an app-owned wallet lets whoever
holds the application credential rewrite the policy that was meant to bound them.

It is invisible in normal operation, which is exactly why it is written here:
nothing will fail, and the bound will simply not exist.

### The signer's own policy

`RADAR_SIGNER_POLICY` points at a JSON `Policy` file, and the signer **refuses
to start without it**.

That is [ADR 0008](../docs/adr/0008-the-signer-holds-its-own-policy.md). The
signer does not verify that an `Authorization` came from the kernel — there is no
MAC on it, and its nonce is checked against nothing — so an authorisation's
bounds are the caller's *claim* about what was approved. This file is the only
ceiling in that process that a caller does not write.

Its contents are an operator decision about money, which is why it is a file and
not a constant. `Policy::CLOSED` is the correct starting value and refuses
everything:

```json
{"autonomy":"observe","max_position":0,"max_deployed":0,"max_per_creator":0,
 "max_daily_loss":0,"max_round_trip_cost_bps":0,"max_canary":0,
 "max_input_staleness":0,"max_consecutive_failures":0}
```

Changing `autonomy` away from `observe` is a decision about deploying capital.
Make it deliberately, and not as a side effect of getting something to work.

### The signer's customer key

`RADAR_PRIVY_AUTHORIZATION_KEY` belongs to the **signer's** environment and
never to `radar-serve`'s.

That is [ADR 0007](../docs/adr/0007-the-privy-authorization-key-lives-in-the-signer-process.md),
not a preference. A signature made with this key causes a customer's wallet to
move funds, which makes it the same category of object as the local wallet key —
and `radar-serve` is the process with a listener, a model provider, an HTTP
client, an embedded frontend and a paywall. **A deployment that puts this
variable in `radar-serve`'s environment silently discards that decision**, and
nothing will complain, because `radar-serve` simply never reads it.

Absent is a refusal, not a failure to start: an instance with no customers needs
no customer key, and refusing to boot without one would take down the local lane
too. A Privy request with no key configured is refused by name.

### The customer lane, when there are customers

Off unless a Privy application id is configured, and **off is not a degradation**:
with no customer authenticator a customer route requires operator identity, which
is stricter than it will be rather than looser. Rule 8's direction.

```
echo 'RADAR_PRIVY_APP_ID=<the application id from the Privy dashboard>' | sudo tee -a /etc/radar/radar.env
```

The application id is **not a secret** — it ships in the browser bundle — and
token verification needs nothing else. Privy's *app secret* is a different thing,
used for server API calls rather than verification, and it does not belong in
this file until something needs it.

A value that is plainly an unsubstituted placeholder stops the server rather than
being enforced, because that failure presents as "nobody can log in" and sends an
operator to the vendor's dashboard instead of to their own env file. That
happened once already with `RADAR_ACCESS_AUD`.

The startup log says which state it is in, every start:

```
  customers  : verifying Privy tokens for app cmthhkznr0a3u0cl86prxlb7x
  customers  : off — customer routes require operator identity
```

### The reading assistant, when you want it

Off unless a provider *and* a budget are configured, and it holds no credential
of its own. The subscription path spawns the vendor CLI, which owns `auth.json`
and its own refresh; Radar has no code that reads, writes or stores a token.

That is a claim about Radar's source and not about the operating system: a CLI
running as `guardian` has a credential file `guardian` can read. Give it its own
user so the boundary is one the kernel enforces.

```
sudo useradd --system --create-home --home-dir /var/lib/radar-agent      --shell /usr/sbin/nologin radar-agent
sudo install -d -o radar-agent -g radar-agent -m700 /var/lib/radar-agent/.codex
```

Seed the credential once, interactively. This is the only step that needs a
human, and it needs the vendor CLI installed on the box first — check with
`which codex`:

```
sudo -u radar-agent env CODEX_HOME=/var/lib/radar-agent/.codex      codex login --device-auth
```

Then a wrapper that drops to that user, so `radar-serve` never runs the CLI as
itself. **It must pass its arguments through**, because Radar supplies the
subcommand: `exec -` to ask a question and `login --device-auth` to link the
credential.

```
sudo tee /usr/local/bin/radar-codex >/dev/null <<'SH'
#!/bin/sh
exec sudo -n -u radar-agent env CODEX_HOME=/var/lib/radar-agent/.codex codex "$@"
SH
sudo chmod 755 /usr/local/bin/radar-codex
```

`guardian` needs `NOPASSWD` for exactly that one command and nothing else. Radar
clears the child's environment and passes only `PATH`, `HOME`, `CODEX_HOME`,
`LANG`, `LC_ALL` and `TMPDIR`, so nothing else in `/etc/radar/radar.env` reaches
the CLI, and the prompt goes in on stdin rather than as an argument — arguments
are visible in `ps` to every user on the box.

### Linking is a button, not an SSH session

Once the wrapper is in place, **the interface links the credential itself**.
Sign in, press **Link**, and the page shows a verification URL and a short code
to enter in a browser. Neither is a credential — that is what device
authorisation is — so nothing secret crosses the page.

The seeding command below still works and is the fallback when the interface is
not reachable. The button matters most for the case nobody plans for: the
refresh token expires after roughly 14–30 days of inactivity, and re-linking
should be a click rather than a procedure somebody has to remember.

```
sudo -u radar-agent env CODEX_HOME=/var/lib/radar-agent/.codex      codex login --device-auth
```

Only one flow runs at a time. The credential is single-writer, so a second
concurrent login would race the first; asking again while one is open returns
the same code rather than starting another, and an abandoned flow is reclaimed
after fifteen minutes.

Finally, in `/etc/radar/radar.env`:

```
RADAR_MODEL_DAILY_USD=2.00
# Where meter state is written so a restart does not reset the day's spend.
# Required for the agent to run at all: a meter that cannot record what it spent
# cannot enforce a ceiling across a restart. Must be writable by the service
# user, and it is checked at startup rather than at the first write.
RADAR_STATE_DIR=/home/guardian/radar/data/state
RADAR_MODEL_CODEX=/usr/local/bin/radar-codex
```

The startup log says which mode it is in. `radar-serve` prints `access` and
`agent` lines on every start, and an instance serving without a check says so
about itself in its own logs:

```
  access     : verifying heyvera.cloudflareaccess.com tokens
  agent      : on via codex, 3 read-only tool(s), $2.000000/day
```

`radar brief` then reports the agent in **both** directions, from the health body
it already fetches:

```
  [ok  ] agent              codex answering, 3 read-only tool(s), $0.010000 today
  [FAIL] agent              codex refused the last call: the CLI exited with exit code: 1
```

Both were verified by running them — the second by breaking the CLI on purpose
and confirming `radar brief` exits non-zero. A check confirmed only where it
passes is not confirmed.

## Before the deploy that lands the customer lane

**Read this one before installing.** The instance already has
`RADAR_PRIVY_APP_ID` set, and that is the setting that switches customer
authentication on. It has been on.

### What that means today, on the running binary

The guard tries a **customer** token first and falls back to Cloudflare Access:

```rust
if audience.accepts_customer()
    && let Some(config) = state.customer.config()      // Privy configured
    && let Some(token) = customer::token_from(headers) // a bearer token
{ ... return next.run(request).await; }                // served, before Access
```

So Access is the fallback, not the gate. On the deployed binary there is no
admission check between "this token is genuine" and "this identity is one of
ours" — and anyone can sign up to a Privy application. Any verified identity for
that app reaches `/v1/funnel`, `/v1/scoreboard` and `/v1/tokens/*`.

**Nothing has actually been exposed**, and that is worth checking rather than
assuming. The application has no registered users:

```bash
ssh guardian-vps-tail 'set -a; . /etc/radar/radar.env; set +a
  curl -s -u "$RADAR_PRIVY_APP_ID:$RADAR_PRIVY_APP_SECRET"     -H "privy-app-id: $RADAR_PRIVY_APP_ID"     https://auth.privy.io/api/v1/users | head -c 200'
```

On 2026-09-01 that returned zero. The door is unlocked and nobody holds a key.

The harm if somebody did is also bounded: those routes are Radar's own research
record, `/v1/chat` is not mounted because no model provider is configured, and
`/v1/customer/wallet` returns the caller's *own* wallet from their own verified
DID, so it leaks nothing about anyone else.

It is still not what anybody chose, and the fix is below.

### The three settings, and what each does when unset

None of them stops the binary starting. Each fails **closed**, which means an
instance that installs the new binary without them is *more* restrictive than
before, not less — a customer will be refused rather than admitted. That is the
right direction to be wrong in, and it is why the order here is install first,
configure second, rather than the other way round.

| setting | unset means |
|---|---|
| `RADAR_CUSTOMER_ACCESS` | **nobody** may reach the product as a customer |
| `RADAR_CUSTOMER_SALT` | customers cannot be metered, so nothing is spent on them |
| `RADAR_CHAT_PER_CUSTOMER_DAILY` | no customer may spend the model budget |

`RADAR_STATE_DIR` is now read at start for the share meter as well as the model
ledger, and the binary **will not start without it**. It is already set on this
host; check before installing anywhere else:

```bash
ssh guardian-vps-tail 'grep -c RADAR_STATE_DIR /etc/radar/radar.env'
```

### Finding your own DID — and why it has to come second

The allowlist matches the `sub` of a Privy access token, because there is no
email in one. **The identity does not exist until you have logged in once**, so
the order is: deploy closed, log in, read the DID, then allowlist it. Setting the
allowlist first means guessing a string that has not been issued.

Once you have logged in through the interface, the identity is at Privy and can
be read back:

```bash
ssh guardian-vps-tail 'set -a; . /etc/radar/radar.env; set +a
  curl -s -u "$RADAR_PRIVY_APP_ID:$RADAR_PRIVY_APP_SECRET"     -H "privy-app-id: $RADAR_PRIVY_APP_ID"     https://auth.privy.io/api/v1/users' |
  grep -oE '"id":"did:privy:[^"]+"'
```

Two other routes to the same string, for when that one is inconvenient:

- **The Privy dashboard.** Users → your account.
- **From a refusal.** An identity that is not admitted is told its own DID in the
  403 body. This will *not* work for you while you hold a Cloudflare Access
  cookie: the guard falls through to Access, Access passes, and you are served
  rather than refused. Use a browser with no Access session.

### Setting them

Edit on the host — these are secrets and belong in the env file, not in a shell
history or a commit:

```bash
ssh guardian-vps-tail 'head -c 48 /dev/urandom | base64'   # the salt, generated on the box
ssh -t guardian-vps-tail 'sudo -e /etc/radar/radar.env'
```

Add:

```
RADAR_CUSTOMER_ACCESS=closed
RADAR_CUSTOMER_SALT=<the base64 from above>
```

`closed` first, on purpose. It is the state the instance should be in while you
are deploying, and it is what makes the login below safe to do before the
allowlist exists. Once you have your DID, change the one line:

```
RADAR_CUSTOMER_ACCESS=allowlist:did:privy:<your did>
```

Leave `RADAR_CHAT_PER_CUSTOMER_DAILY` out until a model provider is configured —
the chat route is not mounted without one, so a limit on it would be a setting
with nothing to limit.

Then restart and confirm the instance says what it is doing. The start-up banner
reports both:

```bash
ssh guardian-vps-tail 'sudo systemctl restart radar-serve && sleep 2 &&
  journalctl -u radar-serve -n 20 --no-pager | grep -E "admission|chat share|customers"'
```

Expected:

```
  admission  : allowlist of 1
  chat share : closed — no customer may spend the model budget (set RADAR_CHAT_PER_CUSTOMER_DAILY)
  customers  : enforcing for application cmthhk…
```

A line reading `admission  : closed — no customer may reach the product` means
the variable did not take, and **a line reading `open` means the product is
public**. Both are worth reading rather than assuming.

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

### The three binaries do not live in the same place

Checked 2026-09-01, and an earlier version of this section had it wrong in the
way this file exists to prevent — it installed all three to `~/bin` and then
`sha256sum`'d `~/bin`, which "verifies" a `radar-serve` that nothing runs.

Ask rather than assume:

```bash
ssh guardian-vps-tail 'grep -h ExecStart /etc/systemd/system/radar-*.service'
```

| binary | where its unit runs it from | sudo? |
|---|---|---|
| `radar-backfill` | `/home/guardian/bin/` (`radar-follow.service`) | no |
| `radar` | `/home/guardian/bin/` (used by `radar-brief.sh`) | no |
| `radar-serve` | **`/usr/local/bin/`**, root-owned | **yes** |

Install by writing beside the target and renaming, rather than over it. A running
process holds its inode, so a rename never disturbs one mid-write, and the
running binary keeps working until it is restarted.

The two that need no privileges:

```bash
ssh guardian-vps-tail '
  set -e
  for b in radar radar-backfill; do
    install -m755 /tmp/$b ~/bin/$b.new && mv -f ~/bin/$b.new ~/bin/$b
  done
  sha256sum ~/bin/radar ~/bin/radar-backfill'
```

And the one that does. `guardian` has full sudo but **no NOPASSWD entry for
radar**, so this needs an interactive session and cannot be run unattended:

```bash
ssh -t guardian-vps-tail '
  set -e
  sudo install -m755 /tmp/radar-serve /usr/local/bin/radar-serve.new &&
  sudo mv -f /usr/local/bin/radar-serve.new /usr/local/bin/radar-serve &&
  sha256sum /usr/local/bin/radar-serve'
```

**Compare all three against `BUILD-INFO.txt`.** Verifying the download proves the
artifact arrived; only this proves the artifact is what got installed, and the
two came apart in exactly the way described above.

The quickest way to catch the mistake this section used to cause:

```bash
ssh guardian-vps-tail 'readlink /proc/$(pgrep -x radar-serve)/exe'
```

It prints the path, and `(deleted)` after it when the running process is holding
a binary that has since been replaced — which is a deploy that landed and has not
been restarted. If it prints a path under `/home/guardian/bin`, the unit is not
running what you just installed.

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

## The public analyst

`radar-analyst` answers mentions with what Radar measured. It is the one service
here that talks to a third party on a stranger's schedule, so it is a separate
unit: a crash, a rate limit or a revoked token must not touch the recorder.

**It does not read the store.** Per-mint facts come from RPC on demand and the
population figures come from a snapshot file, so this unit and the recorder
share nothing but a disk.

```bash
sudo install -D -m644 deploy/radar-analyst.service /etc/systemd/system/radar-analyst.service
sudo install -m 0640 -o root -g guardian deploy/analyst.env.example /etc/radar/analyst.env
install -m755 dist/radar-analyst ~/bin/radar-analyst
mkdir -p ~/radar/data/analyst
sudo systemctl daemon-reload
sudo systemctl enable --now radar-analyst
```

### The two files it reads, and what happens without them

The analyst reads two published measurements from disk. Neither is optional in
the sense that matters: without them the bot still answers, and says less.

| file | without it |
|---|---|
| `docs/research/data/0024-base-rates.json` | replies carry no population context — a recipient count with no distribution to quote it against |
| `docs/research/data/creator-index.json` | **every reply about a fresh launch says the same thing**, and the venue's own graduation rates go stale with the snapshot |
| `docs/research/data/population.json` | written beside the index by the same job; without it `/v1/public/stats` answers 404 and the site shows its dated fixture |

Both are read relative to the working directory, which the unit sets to
`/home/guardian/radar`, so they live at
`/home/guardian/radar/docs/research/data/`.

The second one is the difference between an account worth following and one that
gets mentioned once. Measured on 2026-09-04: three real launches produced three
**identical** replies without it, because the cost figure is a constant and most
launches sit in the same recipient band. With it, the same command said *"150
tokens launched by this creator, none of which ever filled its curve"* — which
is specific, checkable, and said by nobody else.

Build it, and put it on the timer that keeps it current:

```bash
sudo install -D -m644 deploy/radar-creator-index.service /etc/systemd/system/radar-creator-index.service
sudo install -D -m644 deploy/radar-creator-index.timer   /etc/systemd/system/radar-creator-index.timer
sudo systemctl daemon-reload
sudo systemctl enable --now radar-creator-index.timer

# And once now, rather than waiting six hours for the first run.
sudo systemctl start radar-creator-index
journalctl -u radar-creator-index -n 5 --no-pager
```

It took about a minute over 506,821 launches and produced 116,405 creators in a
13MB file. **Check the count, not the exit status**: a store that cannot be read
produces an empty index rather than an error, and an empty index makes every
reply say "this creator has no record here".

**It ships with `main`.** The count in the file is a measurement, so the file is
not committed — like the store itself, it is built where the data is.

The same pass also measures **the venue's own graduation rates** — how many of
every launch Radar has recorded and measured ever filled their curve, over time
or inside their own block — and writes them into the file as `population`. Those
two figures also exist in `0024-base-rates.json`, where they came from a public
RPC walking 45 slots; here they are counted over the whole recorded population
instead, and refreshed every six hours by this timer rather than by hand.

`radar brief` gains an `index` line reporting the creator count, the population
the replies quote, and how long ago it was rebuilt. **It fails when the rebuild
is more than twelve hours old** — two missed runs. That is the state nothing else
would catch: this unit is a `oneshot` with no `Restart=`, so a build that starts
failing leaves the last good file exactly where it is, and every reply keeps
quoting a frozen population, confidently. On a host with no index at all the line
says so and passes, because that is every workstation checkout.

An index written before this existed has no `population` key and still loads:
absent means *not measured*, and a reply says nothing rather than quoting five
zeroes. So the first run after upgrading is what starts the figures flowing.

**It starts safely with an empty env file, and that is the point.** Every switch
is deny-by-default, and each absence is reported rather than assumed:

| absent | what happens |
|---|---|
| `RADAR_X_BEARER` / `RADAR_X_USER_ID` | **nothing is read and nothing is posted** — the publisher is the dry run |
| the four prices | nothing is answered: an unpriced call cannot be metered |
| `RADAR_ANALYST_DAILY_USD` / `_PER_CALL_USD` | the budget is closed and every call is refused |
| the limits | the admission gate refuses every mention |
| `RADAR_MODEL_*` | the deterministic template ships instead of a voice |
| the base-rate snapshot | replies carry no population context |
| `RADAR_SELF_MINT` | **no token is the analyst's own** and every coin is answered on the same rule — the right state until the token exists. Set to the mint, a price or market-cap fact about it is dropped before the model sees the sheet (ADR 0013 constraint 5). **Set to something that is not an address, the daemon idles and says so**: a misspelt mint must not switch the rule off for the real token |

A daemon that exits on a missing variable looks like a broken deploy. One that
runs and says `unfunded` on every tick is legible, and `radar brief` can see it.

**`RADAR_X_BEARER` and `RADAR_X_USER_ID` make the account read. `RADAR_X_PUBLISH=on`
makes it speak.** They are two switches on purpose.

With the credential alone, the daemon polls real mentions, answers them, and
writes every answer to the log beside the fact sheet it was built from — saying
nothing in public. That is the state to spend the first day in, and the state the
launch gate is read in.

Until 2026-09-05 it was one switch, and pasting a token went straight to a live
account. There was no way to satisfy the gate — a hundred replies read with their
evidence — without publishing the hundred. Two wrong figures were found in the
reply path on 2026-09-04 alone, both by reading real output, so the first hundred
are exactly where the next one turns up.

The daemon prints which of its three states it is in on every start:

```
radar-analyst: no credential, so nothing is read and nothing is posted.
radar-analyst: reading mentions and answering them to the log ONLY -- set RADAR_X_PUBLISH=on to speak in public.
radar-analyst: RADAR_X_PUBLISH=on but there is no signing credential, so every reply will be answered and none delivered. ...
radar-analyst: LIVE -- replies are being posted publicly.
```

### Telegram: the free lane, on a second bot

Design 0009 L5: X is the public record and the contest; Telegram is where the
same question costs nothing to answer, so it is where the volume goes. The
daemon runs both lanes from one process and one env file, with the same parser,
the same gate shape and the same fact path — only the transport differs.

Three things are deliberately separate. **The token** is a different bot from
the alert channel's (`RADAR_TELEGRAM_BOT_TOKEN`; make it in BotFather and never
reuse the alert bot's, or a stranger's message can land in the alert chat).
**The caps** are `RADAR_TELEGRAM_PER_SUMMONER_DAILY`, `RADAR_TELEGRAM_GLOBAL_DAILY`
and `RADAR_TELEGRAM_DEDUPE_SECONDS`, unset meaning zero meaning refuse. **The
log** is `telegram.jsonl` beside `replies.jsonl`, and nothing that scores the
contest reads it — a Telegram answer is kept out of the record by being in a
different file, not by a flag.

The lane has the same two switches as X: the token makes it read and answer
into `telegram.jsonl`; `RADAR_TELEGRAM_PUBLISH=on` makes it reply in the chat.
The daemon prints which state the lane is in on every start, on the line after
the X posture, and `radar brief`'s `analyst` line gains a telegram count once
the file exists.

### Two credentials, because the platform needs two

Reading mentions and posting replies do **not** take the same credential.
`POST /2/tweets` refuses an app-only bearer token and requires user context —
checked against `docs.x.com` on 2026-09-05, after the client had already shipped
sending a bearer to it. Reading would have worked; the first real reply would
have been refused.

| what | credential | portal name |
|---|---|---|
| read mentions | `RADAR_X_BEARER` | Bearer Token |
| identify the account | `RADAR_X_USER_ID` | the numeric id, not the handle |
| post a reply | `RADAR_X_API_KEY` + `RADAR_X_API_SECRET` | API Key, API Key Secret |
| | `RADAR_X_ACCESS_TOKEN` + `RADAR_X_ACCESS_SECRET` | Access Token, Access Token Secret |

OAuth 1.0a rather than OAuth 2.0, deliberately: four static values, no browser
redirect, no two-hour expiry, and no refresh loop whose failure would leave a bot
that has quietly stopped talking.

**Set App permissions to "Read and write" before generating the access token.**
A token minted under "Read" keeps those permissions for life and fails to post
with a 403 that does not say why.

The numeric user id, which `/2/users/me` will not give you from a bearer:

```bash
curl -s -H "Authorization: Bearer $RADAR_X_BEARER" \
  "https://api.x.com/2/users/by/username/CabalHunter"
```

```bash
journalctl -u radar-analyst -f
tail -f ~/radar/data/analyst/replies.jsonl
```

The three files it owns live under `RADAR_ANALYST_DIR`, and the unit grants
write access to that path and nothing else: `replies.jsonl` is the reply log,
`cursor` is the last mention answered, and `ledger.json` is the day's spend. The
ledger is what stops a service under `Restart=always` spending the day's budget
as many times as it can crash, so **do not delete it to "reset" anything**.

`radar brief` gains an `analyst` line reporting how many replies were answered
and how many were actually published. The gap between those two numbers is the
one worth watching: a publisher that is down all night fills the log and answers
nobody.

**That line only *alarms* on a host where `RADAR_ANALYST_DIR` is set.** Setting
it is the claim that the analyst runs here, and the unit above sets it, so
installing the analyst is what arms the check. Everywhere else a missing reply
log is reported in words and graded `ok`, because on a host without the daemon
it is absent forever and a check that fires every fifteen minutes for weeks
would teach you to ignore the channel that carries the recorder's death.

So: if you run `radar brief` by hand and want the analyst held to account, run
it the way the timer does — with the environment file — rather than bare.

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

**A webhook, if one is configured.** `radar-brief.service` already loads
`/etc/radar/alert.env`, and the file does not exist on this box yet.
[`alert.env.example`](alert.env.example) is the template and explains each
setting; the short version is one of these three:

```bash
sudo install -m 0640 -o root -g guardian deploy/alert.env.example /etc/radar/alert.env
sudo nano /etc/radar/alert.env
sudo systemctl restart radar-brief.timer   # picks the file up on the next run
```

| destination | what to set | setup |
|---|---|---|
| **Telegram** — recommended | `RADAR_ALERT_WEBHOOK=https://api.telegram.org/bot<TOKEN>/sendMessage`, `RADAR_ALERT_FORMAT=telegram`, `RADAR_ALERT_CHAT_ID=<id>` | ~5 min in the app: @BotFather `/newbot`, message the bot once, read the id from `getUpdates` |
| ntfy.sh | `RADAR_ALERT_WEBHOOK=https://ntfy.sh/<long-random-topic>` and `RADAR_ALERT_FORMAT=text` | ~1 min, no account — but the topic name is the only thing protecting it |
| Discord | the channel webhook URL, format left unset | a server you own |
| Slack | the incoming-webhook URL, format left unset | a workspace |

**Telegram is the recommendation** and the trade against ntfy is the honest
one: four more minutes of setup, in exchange for a token instead of a guessable
name, and a searchable history for the question that always gets asked after an
outage — *when did this start?* Telegram is also the only one of the four whose
body needs a second value, the chat id; without it the script sends nothing and
says so in the journal, because a `curl` that succeeds while delivering to
nobody is the worst outcome this path can produce.

The body carries **both** `text` and `content`, because Slack reads the first
and Discord the second and each ignores the other. That is not belt and braces:
an earlier version of this script sent only `text`, which Discord answers with
a **400**, and the only trace would have been a single "POST failed" line in
the journal — an alerting path failing in exactly the manner of the outage it
exists to report.

The message has its quotes and backslashes **removed** rather than escaped, so
a Windows path or a quoted token symbol in a check detail cannot produce a
malformed body. The version that escaped them lost the backslashes somewhere
between the heredoc, `sed` and `curl -d`, and posted an empty message field —
which from outside is indistinguishable from a delivered alert. Found by
pointing it at a local listener and reading what actually arrived, which is the
only way this class of bug is ever found.

Unset means no webhook, never a silent pass — the journal line happens
regardless. That is rule 8 applied to alerting: missing configuration must not
turn a failure into a success.

**Why a status page is not a substitute, even though one is planned.** The
failure this exists for is a recorder that exited at 05:37 and was found
thirteen hours later by somebody happening to reground (LEARNINGS 8). A page
has precisely the property that caused that: it waits to be looked at. The
sibling service on this box proves the point at length —
`claw-net/scripts/monitoring-check.sh` has run every five minutes for months
with `SLACK_WEBHOOK_URL`, `ALERT_EMAIL` and `PAGERDUTY_ROUTING_KEY` all unset,
writing to a log nobody reads. A public page is the right surface for *users*
and it is in design 0007; it cannot be the thing that wakes an operator.

**Prove the channel before trusting it**, the same way the timer is proved
above — a store that cannot be healthy must both exit non-zero and arrive:

```bash
mkdir -p /tmp/emptystore && RADAR_STORE=/tmp/emptystore ~/bin/radar-brief.sh; echo "exit=$?"
```

If nothing arrives, `journalctl -t radar-brief -p err -n 5` will say
`alert webhook POST failed` — the script never lets a dead webhook change the
health verdict.

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
