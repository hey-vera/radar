# SPDX-License-Identifier: Apache-2.0
"""Build the pump.fun fixture set from CryptoHouse.

The RPC-based capture (capture_fixtures.py) could only see whatever appeared in
the ~30 slots it sampled, so it found 14 instructions and silently missed the
rare ones -- including `create`, the pre-V2 launch path, which is still live at
~7 per two hours and would have been decoded as Unknown forever.

CryptoHouse holds every Solana instruction ever executed and is free to query, so
"every discriminator this program has emitted in a window" is one GROUP BY. See
ADR 0002.

Names are recovered by brute-forcing sha256("global:<name>")[..8] against a
candidate list rather than read from documentation, because the program is well
ahead of any published reference. A hash match on eight bytes is strong evidence
(a false positive needs a 2^-64 collision); a miss means the name is simply not
in the candidate list, and the instruction is recorded unnamed rather than
guessed at.

Writes crates/radar-decode/tests/fixtures/pumpfun_instructions.json.
"""

import hashlib
import json
import os
import sys
import urllib.parse
import urllib.request

ENDPOINT = "https://crypto-clickhouse.clickhouse.com/"
USER = "crypto"
PUMP_FUN = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
OUT = os.path.join("crates", "radar-decode", "tests", "fixtures", "pumpfun_instructions.json")

# Two-hour windows: the server caps a query at 60s and this table is 1.31
# trillion rows. Several windows spread across days catch instructions that are
# rare in any single one.
WINDOWS = [
    ("2026-08-21 06:00:00", "2026-08-21 08:00:00"),
    ("2026-08-20 18:00:00", "2026-08-20 20:00:00"),
    ("2026-08-19 12:00:00", "2026-08-19 14:00:00"),
]

# Anchor's event-CPI tag is a hardcoded constant, not a name hash: a program
# self-CPIs with this discriminator to emit a structured event. On pump.fun it is
# the highest-volume instruction of all, and it carries the trade details that
# would otherwise have to be reconstructed from account deltas.
ANCHOR_EVENT_CPI = "e445a52e51cb9a1d"

VERBS = [
    "set", "update", "admin_set", "admin_update", "init", "initialize", "close", "claim",
    "collect", "distribute", "sync", "migrate", "create", "extend", "withdraw", "deposit",
    "pause", "unpause", "enable", "disable", "reset", "apply", "register", "revoke",
    "transfer", "accept", "propose", "buy", "sell", "toggle", "finalize", "graduate",
]
NOUNS = [
    "", "params", "creator", "coin_creator", "fee", "fees", "config", "global", "authority",
    "global_authority", "idl_authority", "token_incentives", "incentives",
    "user_volume_accumulator", "volume_accumulator", "bonding_curve", "mayhem_state", "mayhem",
    "metaplex_creator", "fee_config", "admin", "event_authority", "last_withdraw", "pool",
    "curve", "state", "account", "cashback", "treasury", "protocol_fee", "creator_fee",
    "migration", "migrator", "exact_sol_in", "exact_quote_in", "exact_tokens_in",
    "creator_fees", "coin_creator_fees", "protocol_fees", "accumulators", "creators",
    "token_incentive", "volume", "user_volume", "fee_recipient", "fee_recipients",
]
SUFFIXES = ["", "_v2", "_v3", "_v4"]


def anchor_disc(name):
    return hashlib.sha256(f"global:{name}".encode()).digest()[:8].hex()


def build_name_table():
    table = {}
    for v in VERBS:
        for n in NOUNS:
            base = v if not n else f"{v}_{n}"
            for s in SUFFIXES:
                name = base + s
                table.setdefault(anchor_disc(name), name)
    return table


def query(sql):
    url = ENDPOINT + "?" + urllib.parse.urlencode({"user": USER, "query": sql})
    with urllib.request.urlopen(url, timeout=120) as resp:
        body = resp.read().decode()
    rows = [json.loads(line) for line in body.splitlines() if line.strip()]
    if rows and "exception" in rows[0]:
        raise RuntimeError(rows[0]["exception"][:300])
    return rows


def main():
    names = build_name_table()
    print(f"candidate name table: {len(names)} discriminators")

    seen = {}
    for start, end in WINDOWS:
        sql = (
            "SELECT lower(hex(substring(base58Decode(data), 1, 8))) AS disc, "
            "count() AS n, any(data) AS sample_b58, any(tx_signature) AS sig, "
            "any(block_slot) AS slot, min(length(base58Decode(data))) AS min_len, "
            "max(length(base58Decode(data))) AS max_len "
            f"FROM solana.instructions WHERE program_id='{PUMP_FUN}' "
            f"AND block_timestamp >= '{start}' AND block_timestamp < '{end}' "
            "AND length(data) > 10 GROUP BY disc ORDER BY n DESC FORMAT JSONEachRow"
        )
        print(f"  window {start} .. {end}", end=" ", flush=True)
        try:
            rows = query(sql)
        except Exception as e:  # noqa: BLE001
            print(f"-> failed: {e}")
            continue
        print(f"-> {len(rows)} discriminators")
        for r in rows:
            d = r["disc"]
            prev = seen.get(d)
            if prev is None or int(r["n"]) > prev["observed_count"]:
                seen[d] = {
                    "discriminator": d,
                    "anchor_name": names.get(d),
                    "kind": "anchor_event_cpi" if d == ANCHOR_EVENT_CPI else "instruction",
                    "observed_count": int(r["n"]),
                    "sample_data_b58": r["sample_b58"],
                    "min_data_len": int(r["min_len"]),
                    "max_data_len": int(r["max_len"]),
                    "example_signature": r["sig"],
                    "example_slot": int(r["slot"]),
                }

    if not seen:
        print("no rows captured")
        return 1

    named = sum(1 for v in seen.values() if v["anchor_name"])
    unnamed = [d for d, v in seen.items() if not v["anchor_name"] and d != ANCHOR_EVENT_CPI]

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump(
            {
                "source": "CryptoHouse (crypto-clickhouse.clickhouse.com), solana.instructions",
                "program": PUMP_FUN,
                "windows": [{"from": a, "to": b} for a, b in WINDOWS],
                "note": (
                    "Names recovered by brute-forcing sha256('global:'+name)[..8], not read "
                    "from documentation. A null anchor_name means the name is not in the "
                    "candidate list; the instruction is real and must decode to Unknown."
                ),
                "instructions": dict(
                    sorted(seen.items(), key=lambda kv: -kv[1]["observed_count"])
                ),
            },
            f,
            indent=2,
        )
        f.write("\n")

    print(f"\n{len(seen)} distinct discriminators -> {OUT}")
    print(f"  named       : {named}")
    print(f"  anchor event: {1 if ANCHOR_EVENT_CPI in seen else 0}")
    print(f"  unnamed     : {len(unnamed)}  {unnamed}")
    for d, v in sorted(seen.items(), key=lambda kv: -kv[1]["observed_count"]):
        label = v["anchor_name"] or ("<anchor event cpi>" if v["kind"] != "instruction" else "<unnamed>")
        print(f"  {d}  n={v['observed_count']:>8}  {v['min_data_len']:>3}-{v['max_data_len']:<3}b  {label}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
