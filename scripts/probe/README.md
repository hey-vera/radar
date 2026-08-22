<!-- SPDX-License-Identifier: Apache-2.0 -->
# Probes

Throwaway-looking scripts kept on purpose: they produced the numbers in
[docs/research/0004-measured-launch-base-rates.md](../../docs/research/0004-measured-launch-base-rates.md),
and a number without the thing that produced it is a claim rather than a
measurement.

Stdlib only, no dependencies. They hit the free public RPC and pace themselves,
so they cost nothing and can be re-run to check whether a finding still holds.

```bash
python scripts/probe/probe_adr0001.py      # can one getBlock carry a whole launch?
python scripts/probe/probe_launch.py       # anatomy of a single launch slot
python scripts/probe/probe_base_rates.py   # base rates across a window of slots
```

Re-run `probe_base_rates.py` before trusting any percentage in the research doc:
the sample there is 45 blocks on one day, which is enough to set direction and
not enough to build a detector on.
