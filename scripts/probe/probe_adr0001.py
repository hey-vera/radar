# SPDX-License-Identifier: Apache-2.0
"""Probe the assumptions ADR 0001 rests on, against real mainnet data.

The claim: one getBlock call ($0.001) returns the entire launch picture -- the
create transaction, the dev buy, and every same-slot coordinated buy -- where
buying those transactions parsed would cost $0.05 each.

Four things could falsify it:
  1. A cheap/free RPC may refuse getBlock entirely.
  2. A pump.fun launch block may be so large that "one call" hides a bandwidth
     or latency problem the price does not show.
  3. pump.fun launches may not actually have same-slot buys, which would make
     the coordination-detection story weaker than claimed.
  4. Jito tip transfers may not be identifiable in the same block.

Uses only the stdlib. Public RPC, so requests are paced.
"""

import json
import sys
import time
import urllib.request

RPC = "https://api.mainnet-beta.solana.com"
PUMP_FUN = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"

# The eight Jito tip accounts, fixed and enumerable. A transfer to one of these
# in the same slot as a launch is Tier-A coordination evidence.
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

_calls = 0


def rpc(method, params):
    global _calls
    _calls += 1
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(
        RPC, data=body, headers={"Content-Type": "application/json"}
    )
    started = time.time()
    with urllib.request.urlopen(req, timeout=60) as resp:
        raw = resp.read()
    elapsed = time.time() - started
    return json.loads(raw), len(raw), elapsed


def main():
    print("=" * 72)
    print("ADR 0001 probe -- can one getBlock carry a whole launch?")
    print("=" * 72)

    # 1. Does a free public RPC serve getBlock at all?
    out, _, _ = rpc("getSlot", [{"commitment": "finalized"}])
    if "error" in out:
        print(f"FAIL: getSlot refused: {out['error']}")
        return 1
    tip = out["result"]
    print(f"\n[1] Free public RPC reachable. Finalized slot: {tip}")

    # 2. Find a recent pump.fun create. Walk back from the tip; the public node
    #    keeps only a limited window, so stay close to it.
    print("\n[2] Searching recent slots for a pump.fun launch...")
    launch = None
    for offset in range(2, 40):
        slot = tip - offset
        time.sleep(0.4)  # public RPC is rate limited
        out, nbytes, elapsed = rpc(
            "getBlock",
            [slot, {
                "encoding": "json",
                "transactionDetails": "full",
                "maxSupportedTransactionVersion": 0,
                "rewards": False,
            }],
        )
        if "error" in out:
            code = out["error"].get("code")
            if code in (-32004, -32007, -32009):  # not available / skipped / cleaned up
                continue
            print(f"    slot {slot}: error {out['error']}")
            continue

        block = out.get("result")
        if not block:
            continue

        txs = block.get("transactions", [])
        pump_txs = []
        tip_payers = 0
        for i, tx in enumerate(txs):
            keys = tx["transaction"]["message"].get("accountKeys", [])
            if PUMP_FUN in keys:
                pump_txs.append(i)
            if JITO_TIP_ACCOUNTS.intersection(keys):
                tip_payers += 1

        print(
            f"    slot {slot}: {len(txs):>4} txs, {nbytes/1_048_576:>6.2f} MiB, "
            f"{elapsed:>5.2f}s, pump.fun txs: {len(pump_txs):>3}, jito-tip txs: {tip_payers:>3}"
        )
        if pump_txs and launch is None:
            launch = (slot, block, nbytes, elapsed, pump_txs, tip_payers)
        if launch and offset > 8:
            break

    if not launch:
        print("\nNo pump.fun activity found in the sampled window.")
        return 1

    slot, block, nbytes, elapsed, pump_txs, tip_payers = launch
    txs = block["transactions"]

    print("\n" + "=" * 72)
    print(f"[3] Anatomy of slot {slot}")
    print("=" * 72)
    print(f"    transactions in block : {len(txs)}")
    print(f"    response size         : {nbytes/1_048_576:.2f} MiB")
    print(f"    fetch latency         : {elapsed:.2f}s")
    print(f"    pump.fun transactions : {len(pump_txs)}")
    print(f"    jito tip transactions : {tip_payers}")
    print(f"    cost as getBlock      : $0.001")
    print(f"    cost as parsed txs    : ${len(pump_txs) * 0.05:.2f}  ({len(pump_txs)} x $0.05)")
    if pump_txs:
        ratio = (len(pump_txs) * 0.05) / 0.001
        print(f"    ratio                 : {ratio:,.0f}x")

    # 4. Are the pump.fun transactions contiguous? Contiguous indices plus a tip
    #    transfer in the same slot is what a bundle looks like from outside.
    print("\n[4] Transaction-index clustering (bundle shape)")
    runs, start, prev = [], pump_txs[0], pump_txs[0]
    for idx in pump_txs[1:]:
        if idx == prev + 1:
            prev = idx
        else:
            runs.append((start, prev))
            start = prev = idx
    runs.append((start, prev))
    contiguous = [r for r in runs if r[1] > r[0]]
    print(f"    index runs            : {len(runs)}")
    print(f"    contiguous runs (>1)  : {len(contiguous)}")
    if contiguous:
        longest = max(contiguous, key=lambda r: r[1] - r[0])
        print(f"    longest run           : indices {longest[0]}-{longest[1]} "
              f"({longest[1] - longest[0] + 1} txs back to back)")

    # 5. What do the instructions look like? This is what radar-decode must read.
    print("\n[5] First pump.fun transaction, for decoder fixtures")
    tx = txs[pump_txs[0]]
    msg = tx["transaction"]["message"]
    keys = msg["accountKeys"]
    sig = tx["transaction"]["signatures"][0]
    print(f"    signature             : {sig}")
    print(f"    account keys          : {len(keys)}")
    print(f"    instructions          : {len(msg['instructions'])}")
    for ins in msg["instructions"]:
        pid = keys[ins["programIdIndex"]]
        data = ins.get("data", "")
        marker = "  <-- pump.fun" if pid == PUMP_FUN else ""
        print(f"      program {pid[:8]}... data[{len(data)} b58 chars]{marker}")
    print(f"    log lines             : {len(tx.get('meta', {}).get('logMessages') or [])}")
    for line in (tx.get("meta", {}).get("logMessages") or [])[:6]:
        print(f"      {line[:100]}")

    print(f"\n[done] {_calls} RPC calls made.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
