# SPDX-License-Identifier: Apache-2.0
"""Find a real pump.fun launch slot and test the coordination claims against it.

Probe 1 sampled general pump.fun activity and found mostly ordinary trades. The
claim in ADR 0001 and plan section 6 is specifically about the *launch* slot:
that it holds the create, the dev buy, and every same-slot coordinated buy, and
that contiguous transaction indices plus a Jito tip transfer look like a bundle.

That is a different population from a random slot, so it needs its own test.
"""

import json
import sys
import time
import urllib.request

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


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(RPC, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        raw = resp.read()
    return json.loads(raw), len(raw)


def instruction_names(tx):
    """pump.fun is an Anchor program, so it logs its instruction names."""
    return [
        line.split("Instruction: ", 1)[1].strip()
        for line in (tx.get("meta", {}).get("logMessages") or [])
        if "Instruction: " in line
    ]


def main():
    out, _ = rpc("getSlot", [{"commitment": "finalized"}])
    tip = out["result"]
    print(f"finalized slot {tip}\nhunting for a pump.fun Create...\n")

    for offset in range(2, 60):
        slot = tip - offset
        time.sleep(0.35)
        try:
            out, nbytes = rpc("getBlock", [slot, {
                "encoding": "json", "transactionDetails": "full",
                "maxSupportedTransactionVersion": 0, "rewards": False,
            }])
        except Exception as e:  # noqa: BLE001 - probe, not production
            print(f"  slot {slot}: {e}")
            continue
        if "error" in out or not out.get("result"):
            continue

        txs = out["result"]["transactions"]
        creates, buys, sells, others = [], [], [], []
        for i, tx in enumerate(txs):
            keys = tx["transaction"]["message"].get("accountKeys", [])
            if PUMP_FUN not in keys:
                continue
            names = instruction_names(tx)
            if any(n.startswith("Create") for n in names):
                creates.append((i, tx, names))
            elif "Buy" in names:
                buys.append((i, tx))
            elif "Sell" in names:
                sells.append((i, tx))
            else:
                others.append((i, tx, names))

        if not creates:
            continue

        print("=" * 72)
        print(f"LAUNCH SLOT {slot}  ({len(txs)} txs, {nbytes/1_048_576:.2f} MiB)")
        print("=" * 72)
        print(f"  pump.fun Create      : {len(creates)}")
        print(f"  pump.fun Buy         : {len(buys)}")
        print(f"  pump.fun Sell        : {len(sells)}")
        print(f"  pump.fun other       : {len(others)}")

        idx, ctx, names = creates[0]
        keys = ctx["transaction"]["message"]["accountKeys"]
        print(f"\n  --- the create, at transaction index {idx} ---")
        print(f"  signature   : {ctx['transaction']['signatures'][0]}")
        print(f"  instructions in this one tx: {names}")
        print(f"  pays a jito tip: {bool(JITO_TIP_ACCOUNTS.intersection(keys))}")

        # Is the dev buy inside the create transaction itself? That is Tier-A
        # direct evidence and needs no inference at all.
        bundled_buy = any(n == "Buy" for n in names)
        print(f"  dev buy inside the create tx: {bundled_buy}   <-- Tier-A direct evidence")

        # How close are the other pump.fun txs to the create in index order?
        near = sorted(
            [(abs(i - idx), i, kind)
             for kind, lst in (("buy", buys), ("sell", sells))
             for i, _ in lst],
        )[:12]
        print(f"\n  --- nearest other pump.fun txs by index distance from {idx} ---")
        for dist, i, kind in near:
            t = dict(buys + sells).get(i)
            tipped = bool(JITO_TIP_ACCOUNTS.intersection(
                t["transaction"]["message"]["accountKeys"])) if t else False
            print(f"    idx {i:>5}  distance {dist:>5}  {kind:<5} jito-tip={tipped}")

        tipped_near = sum(
            1 for dist, i, _ in near
            if dist <= 30 and JITO_TIP_ACCOUNTS.intersection(
                dict(buys + sells)[i]["transaction"]["message"]["accountKeys"])
        )
        print(f"\n  txs within 30 indices of the create that pay a jito tip: {tipped_near}")
        return 0

    print("no Create found in the sampled window")
    return 1


if __name__ == "__main__":
    sys.exit(main())
