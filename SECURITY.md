<!-- SPDX-License-Identifier: Apache-2.0 -->
# Security

## Reporting a vulnerability

Report privately through
[GitHub's advisory form](https://github.com/hey-vera/radar/security/advisories/new),
which is visible only to the maintainers until an advisory is published. Please
do not open a public issue for anything exploitable.

There is no bounty programme and no service-level commitment. What you will get
is an acknowledgement, a considered reply, and credit in the advisory unless you
would rather not have it.

## What is worth reporting

Radar is a research recorder today and a system that will hold a Solana signing
key tomorrow. That shapes what matters.

**Most valuable:**

- Anything that could make the signer authorise a transaction it did not fully
  read. Its guarantee is stated absolutely — *every account it authorises is one
  it read in the bytes it signed* — so a counterexample is the most serious class
  of bug in this repository. See
  [ADR 0003](docs/adr/0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md).
- Any path from a reasoning layer to the signer. Only the deterministic risk
  kernel may turn a proposal into an authorization
  ([AGENTS.md](AGENTS.md) rule 1).
- A decoder that can be made to panic, hang, or mis-parse attacker-supplied
  bytes. `radar-decode` and the signer's transaction decoder both read data an
  attacker chooses.
- Anything that would let untrusted content — token metadata, memos, social copy
  — reach a position where it is treated as an instruction (rule 4).
- Supply-chain findings: a dependency, a pinned action, or a build step that
  could put code into the released binary.

**Also worth reporting, lower severity:** a way to make the recorder skip chain
without saying so. A missing slot range is indistinguishable from a quiet market,
which is the failure this project is organised against.

**Not vulnerabilities:** losing money on a trade; a strategy that performs badly;
a threshold you disagree with. Those are research questions, and
[`docs/research/`](docs/research/) is where they are argued.

## Scope

This repository, its released binaries, and its GitHub Actions workflows.

The deployment described in [`deploy/README.md`](deploy/README.md) runs on a host
shared with unrelated services. Please do not test against `radar.heyvera.org`;
report what you have found and it will be reproduced locally.

## What is already true

Stated so you can skip the ground already covered:

- `unsafe_code = "forbid"` across the workspace.
- The shipped policy is `Policy::CLOSED`, which refuses every proposal. No
  capital is deployed and no key is installed.
- The signer is a separate process with no network, no listener, and no method
  that signs arbitrary bytes. It refuses address lookup tables so that every
  account it authorises is one it read.
- No crate that binds a socket depends on the signer crate. Enforced by
  `nothing_that_listens_on_a_network_depends_on_the_signer_crate`, after that
  boundary was found broken.
- `cargo deny` runs on every pull request for advisories, licences and sources.
- Every GitHub Action is pinned to a full commit SHA.

## What is not

Also stated plainly, because a security policy that lists only strengths is a
marketing document:

- **There is no threat model document yet.** It is planned and not written.
- **There is no spend meter in the running system.** `radar-provider` implements
  one and nothing depends on it, so there is no daily ceiling on paid calls today
  ([AGENTS.md](AGENTS.md) rule 8 records this).
- **Property and fuzz testing are absent.** The decoders are tested on chosen
  inputs and a small deterministic byte sweep, which is not the same thing.
- **The public server has no authentication.** Everything it serves is intended
  to be public; the paid surface is metered by x402 rather than by identity.
