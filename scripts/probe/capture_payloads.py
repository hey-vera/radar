# SPDX-License-Identifier: Apache-2.0
"""Capture many real payloads per instruction, to verify argument layouts.

Identifying an instruction is not the same as understanding it. The argument
layouts in radar-decode were derived by staring at one sample each, and one
sample is enough to form a hypothesis and nowhere near enough to trust it: a
field that happens to be zero, or a length that happens to be typical, will
agree with almost any wrong guess.

So: pull a few hundred real payloads per instruction and let the test assert the
layout holds across all of them. A wrong field offset shows up immediately as
implausible values -- a max_sol_cost of 10^15 lamports, a token name that is not
valid UTF-8, a creator pubkey that is all zeroes.

Writes crates/radar-decode/tests/fixtures/pumpfun_payloads.json.
"""

import json
import os
import sys
import urllib.parse
import urllib.request

ENDPOINT = "https://crypto-clickhouse.clickhouse.com/"
USER = "crypto"
PUMP_FUN = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
OUT = os.path.join("crates", "radar-decode", "tests", "fixtures", "pumpfun_payloads.json")

WINDOW = ("2026-08-21 06:00:00", "2026-08-21 08:00:00")
PER_INSTRUCTION = 250

WANTED = {
    "38fc74089edfcd5f": "buy_exact_sol_in",
    "66063d1201daebea": "buy",
    "b817ee6167c5d33d": "buy_v2",
    "c2ab1c46684d5b2f": "buy_exact_quote_in_v2",
    "33e685a4017f83ad": "sell",
    "5df6823ce7e940b2": "sell_v2",
    "d6904cec5f8b31b4": "create_v2",
    "181ec828051c0777": "create",
}


def query(sql):
    url = ENDPOINT + "?" + urllib.parse.urlencode({"user": USER, "query": sql})
    with urllib.request.urlopen(url, timeout=150) as resp:
        body = resp.read().decode()
    rows = [json.loads(line) for line in body.splitlines() if line.strip()]
    if rows and "exception" in rows[0]:
        raise RuntimeError(rows[0]["exception"][:300])
    return rows


def main():
    start, end = WINDOW
    out = {}
    for disc, name in WANTED.items():
        sql = (
            "SELECT data AS b58, tx_signature AS sig, block_slot AS slot, "
            "toString(block_timestamp) AS ts, accounts "
            f"FROM solana.instructions WHERE program_id='{PUMP_FUN}' "
            f"AND block_timestamp >= '{start}' AND block_timestamp < '{end}' "
            f"AND lower(hex(substring(base58Decode(data), 1, 8))) = '{disc}' "
            f"LIMIT {PER_INSTRUCTION} FORMAT JSONEachRow"
        )
        print(f"  {name:<24}", end=" ", flush=True)
        try:
            rows = query(sql)
        except Exception as e:  # noqa: BLE001
            print(f"failed: {e}")
            continue
        print(f"{len(rows)} payloads")
        out[name] = {
            "discriminator": disc,
            "samples": [
                {
                    "data_b58": r["b58"],
                    "signature": r["sig"],
                    "slot": int(r["slot"]),
                    "timestamp": r["ts"],
                    "account_count": len(r["accounts"]),
                }
                for r in rows
            ],
        }

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump(
            {
                "source": "CryptoHouse solana.instructions",
                "program": PUMP_FUN,
                "window": {"from": start, "to": end},
                "note": (
                    "Real payloads, for asserting that argument layouts hold across many "
                    "samples rather than the one they were guessed from."
                ),
                "instructions": out,
            },
            f,
            indent=1,
        )
        f.write("\n")

    total = sum(len(v["samples"]) for v in out.values())
    print(f"\n{total} payloads across {len(out)} instructions -> {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
