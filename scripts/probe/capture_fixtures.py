# SPDX-License-Identifier: Apache-2.0
"""Capture (instruction data, instruction name) pairs from real mainnet traffic.

radar-decode matches Anchor discriminators -- the first eight bytes of
sha256("global:<snake_case_name>") -- because matching on log-line names is what
produced LEARNINGS entry 3. But a discriminator table written from guessed names
is just the same mistake one layer down.

So: capture the raw instruction bytes alongside the name the program itself
logged, and let the decoder's tests assert that our computed discriminators map
the real bytes to the right name. Ground truth from the chain, not from prose.

Writes crates/radar-decode/tests/fixtures/pumpfun_instructions.json.
"""

import base64
import hashlib
import json
import os
import sys
import time
import urllib.request
from collections import OrderedDict

RPC = "https://api.mainnet-beta.solana.com"
PUMP_FUN = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
OUT = os.path.join("crates", "radar-decode", "tests", "fixtures", "pumpfun_instructions.json")
SLOTS = 30

B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58decode(s):
    num = 0
    for ch in s:
        num = num * 58 + B58.index(ch)
    body = num.to_bytes((num.bit_length() + 7) // 8, "big") if num else b""
    pad = len(s) - len(s.lstrip("1"))
    return b"\0" * pad + body


def camel_to_snake(name):
    out = []
    for i, ch in enumerate(name):
        if ch.isupper() and i and not (name[i - 1].isupper() and (i + 1 >= len(name) or name[i + 1].isupper())):
            out.append("_")
        out.append(ch.lower())
    return "".join(out)


def anchor_disc(name):
    return hashlib.sha256(f"global:{name}".encode()).digest()[:8]


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(RPC, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read())


def main():
    out, = (rpc("getSlot", [{"commitment": "finalized"}]),)
    tip = out["result"]
    print(f"capturing pump.fun instruction fixtures from {SLOTS} slots below {tip}")

    captured = OrderedDict()
    for offset in range(2, 2 + SLOTS):
        slot = tip - offset
        time.sleep(0.3)
        try:
            r = rpc("getBlock", [slot, {
                "encoding": "json", "transactionDetails": "full",
                "maxSupportedTransactionVersion": 0, "rewards": False,
            }])
        except Exception:  # noqa: BLE001
            continue
        if "error" in r or not r.get("result"):
            continue

        for tx in r["result"]["transactions"]:
            msg = tx["transaction"]["message"]
            keys = msg.get("accountKeys", [])
            if PUMP_FUN not in keys:
                continue
            logs = tx.get("meta", {}).get("logMessages") or []

            # Only top-level instructions can be paired with a log line without
            # reconstructing the full CPI tree, and a wrong pairing is worse than
            # no fixture. Take transactions with exactly one pump.fun top-level
            # instruction and exactly one pump.fun "Instruction:" log directly
            # after its invoke line.
            pf_ix = [ix for ix in msg["instructions"] if keys[ix["programIdIndex"]] == PUMP_FUN]
            if len(pf_ix) != 1:
                continue

            name = None
            for i, line in enumerate(logs):
                if line.startswith(f"Program {PUMP_FUN} invoke [1]"):
                    for follow in logs[i + 1:i + 3]:
                        if "Instruction: " in follow:
                            name = follow.split("Instruction: ", 1)[1].strip()
                            break
                    break
            if not name:
                continue

            data = b58decode(pf_ix[0]["data"])
            if len(data) < 8:
                continue
            disc = data[:8]
            snake = camel_to_snake(name)
            key = name
            if key in captured:
                continue
            captured[key] = {
                "logged_name": name,
                "snake_case": snake,
                "discriminator": list(disc),
                "computed_matches": list(anchor_disc(snake)) == list(disc),
                "instruction_data_b64": base64.b64encode(data).decode(),
                "data_len": len(data),
                "slot": slot,
                "signature": tx["transaction"]["signatures"][0],
            }
            print(f"  {name:<24} disc={disc.hex()} len={len(data):<4} "
                  f"anchor_match={captured[key]['computed_matches']}")

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump({"captured_from": "solana mainnet-beta", "program": PUMP_FUN,
                   "instructions": captured}, f, indent=2)
        f.write("\n")

    matched = sum(1 for v in captured.values() if v["computed_matches"])
    print(f"\n{len(captured)} distinct instructions captured -> {OUT}")
    print(f"{matched}/{len(captured)} match sha256('global:'+snake_case)[..8]")
    if matched < len(captured):
        print("\nnon-matching (the naming convention is not what we assumed):")
        for k, v in captured.items():
            if not v["computed_matches"]:
                print(f"  {k:<24} logged disc {bytes(v['discriminator']).hex()} "
                      f"vs computed {anchor_disc(v['snake_case']).hex()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
