# SPDX-License-Identifier: Apache-2.0
"""Measure base rates across many pump.fun launches.

Probe 2 looked at one launch and overturned two assumptions, so the next question
is which of those was the sample and which is the population. This walks a window
of slots, finds every Create, and reports the distribution.

Corrections carried over from probe 2:
  - pump.fun is on V2 instructions (CreateV2, BuyV2). Matching on bare "Buy"
    misses the dev buy entirely, which is the single most load-bearing piece of
    Tier-A coordination evidence. Match on prefix.
  - There is an InitializeMayhemState instruction that is not in any 2024-era
    reference. Decoders have to be built from what the chain actually does.
"""

import json
import sys
import time
import urllib.request
from collections import Counter

RPC = "https://api.mainnet-beta.solana.com"
PUMP_FUN = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
JITO_TIP_ACCOUNTS = {
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
}
SLOTS_TO_SCAN = 45


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(RPC, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        raw = resp.read()
    return json.loads(raw), len(raw)


def instr_names(tx):
    return [
        line.split("Instruction: ", 1)[1].strip()
        for line in (tx.get("meta", {}).get("logMessages") or [])
        if "Instruction: " in line
    ]


def main():
    out, _ = rpc("getSlot", [{"commitment": "finalized"}])
    tip = out["result"]
    print(f"scanning {SLOTS_TO_SCAN} slots back from {tip}\n")

    launches = []
    all_instr = Counter()
    total_bytes = total_txs = blocks = 0
    pumpfun_tx_total = 0

    for offset in range(2, 2 + SLOTS_TO_SCAN):
        slot = tip - offset
        time.sleep(0.3)
        try:
            out, nbytes = rpc("getBlock", [slot, {
                "encoding": "json", "transactionDetails": "full",
                "maxSupportedTransactionVersion": 0, "rewards": False,
            }])
        except Exception:  # noqa: BLE001
            continue
        if "error" in out or not out.get("result"):
            continue

        blocks += 1
        total_bytes += nbytes
        txs = out["result"]["transactions"]
        total_txs += len(txs)

        pump = []
        for i, tx in enumerate(txs):
            keys = tx["transaction"]["message"].get("accountKeys", [])
            if PUMP_FUN in keys:
                names = instr_names(tx)
                all_instr.update(names)
                pump.append((i, tx, names, bool(JITO_TIP_ACCOUNTS.intersection(keys))))
        pumpfun_tx_total += len(pump)

        for idx, tx, names, tipped in pump:
            if not any(n.startswith("Create") for n in names):
                continue
            # Same-slot buys other than the one inside the create itself.
            same_slot_buys = [
                (i, t) for i, t, n, _ in pump
                if i != idx and any(x.startswith("Buy") for x in n)
            ]
            near = [i for i, _ in same_slot_buys if abs(i - idx) <= 30]
            launches.append({
                "slot": slot,
                "idx": idx,
                "dev_buy_in_create": any(n.startswith("Buy") for n in names),
                "create_variant": next(n for n in names if n.startswith("Create")),
                "tipped": tipped,
                "same_slot_buys": len(same_slot_buys),
                "buys_within_30_idx": len(near),
                "instr_count": len(names),
            })

    print("=" * 74)
    print("BLOCK ECONOMICS")
    print("=" * 74)
    print(f"  blocks fetched           : {blocks}")
    print(f"  mean txs per block       : {total_txs/max(blocks,1):,.0f}")
    print(f"  mean block size          : {total_bytes/max(blocks,1)/1_048_576:.2f} MiB")
    print(f"  total bandwidth          : {total_bytes/1_048_576:.1f} MiB for {blocks} slots")
    print(f"  pump.fun txs seen        : {pumpfun_tx_total}")
    print(f"  cost as getBlock         : ${blocks * 0.001:.3f}")
    print(f"  cost as parsed txs       : ${pumpfun_tx_total * 0.05:,.2f}")
    if blocks:
        print(f"  measured ratio           : {(pumpfun_tx_total*0.05)/(blocks*0.001):,.0f}x")
        # Solana runs ~2.5 slots/sec.
        daily_gib = (total_bytes / blocks) * 2.5 * 86400 / 1_073_741_824
        print(f"  every-slot polling would cost {daily_gib:,.0f} GiB/day of bandwidth")

    print("\n" + "=" * 74)
    print(f"LAUNCH BASE RATES  (n={len(launches)})")
    print("=" * 74)
    if launches:
        n = len(launches)
        dev = sum(1 for x in launches if x["dev_buy_in_create"])
        tipped = sum(1 for x in launches if x["tipped"])
        with_buys = sum(1 for x in launches if x["same_slot_buys"] > 0)
        near = sum(1 for x in launches if x["buys_within_30_idx"] > 0)
        print(f"  dev buy inside the create tx : {dev}/{n}  ({100*dev/n:.0f}%)")
        print(f"  create pays a jito tip       : {tipped}/{n}  ({100*tipped/n:.0f}%)")
        print(f"  has any other same-slot buy  : {with_buys}/{n}  ({100*with_buys/n:.0f}%)")
        print(f"  has a buy within 30 indices  : {near}/{n}  ({100*near/n:.0f}%)")
        print(f"  create variants seen         : {Counter(x['create_variant'] for x in launches).most_common()}")
        print(f"  mean instructions per create : "
              f"{sum(x['instr_count'] for x in launches)/n:.1f}")
        print(f"  launches per slot            : {n/max(blocks,1):.2f}")
    else:
        print("  no launches in window")

    print("\n" + "=" * 74)
    print("PUMP.FUN INSTRUCTION MIX  (what a decoder must actually handle)")
    print("=" * 74)
    for name, count in all_instr.most_common(22):
        print(f"  {count:>6}  {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
