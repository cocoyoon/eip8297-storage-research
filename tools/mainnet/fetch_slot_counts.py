# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Ask the ethpandaops Lab API how many storage slots each contract holds.

The replay has to know how large each written contract's storage is, to
give its subtree the depth it has on mainnet. The API serves one row per
contract per day on which its slot count changed, so for each contract this
takes the latest row on or before the target day.

The endpoint is the one the Lab site itself uses, not a documented product:
requests are sequential and paced, and results are appended to the output
as they arrive so an interrupted run resumes.

Data: ethpandaops Xatu, CC BY 4.0.

usage: uv run tools/mainnet/fetch_slot_counts.py --contracts FILE.contracts.csv \
           --day 2025-12-13 --top 2000 --out slot_counts.csv
"""
import argparse
import csv
import json
import os
import sys
import time
import urllib.parse
import urllib.request

API = "https://lab.ethpandaops.io/api/v1/mainnet/fct_storage_slot_state_by_address_daily"


def latest_on_or_before(address: str, day: str):
    """Walk back through date prefixes until a row on or before `day` turns up."""
    y, m, d = day.split("-")
    prefixes = [f"{y}-{m}-{d[0]}", f"{y}-{m}", y]
    prev_year = str(int(y) - 1)
    prefixes += [prev_year, ""]
    for prefix in prefixes:
        q = {"address_eq": "0x" + address, "order_by": "day_start_date desc", "page_size": "40"}
        if prefix:
            q["day_start_date_starts_with"] = prefix
        url = API + "?" + urllib.parse.urlencode(q)
        for attempt in range(4):
            try:
                # The default urllib agent is refused; identify plainly.
                req = urllib.request.Request(url, headers={"User-Agent": "pbt-storage-research/0.1"})
                with urllib.request.urlopen(req, timeout=30) as r:
                    rows = json.load(r).get("fct_storage_slot_state_by_address_daily", [])
                break
            except Exception as e:  # noqa: BLE001
                if attempt == 3:
                    raise
                time.sleep(2 * (attempt + 1))
        for row in rows:  # newest first
            if row["day_start_date"] <= day:
                return row["day_start_date"], int(row["active_slots"])
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--contracts", required=True)
    ap.add_argument("--day", required=True)
    ap.add_argument("--top", type=int, default=2000)
    ap.add_argument("--out", required=True)
    ap.add_argument("--pause", type=float, default=0.15)
    args = ap.parse_args()

    done = {}
    if os.path.exists(args.out):
        with open(args.out) as f:
            for row in csv.reader(l for l in f if not l.startswith("#")):
                if row and row[0] != "address":
                    done[row[0]] = row
    else:
        with open(args.out, "w") as f:
            f.write(f"# Active storage slots per contract on or before {args.day}.\n")
            f.write("# Source: ethpandaops Lab API, fct_storage_slot_state_by_address_daily (Xatu, CC BY 4.0).\n")
            f.write("address,active_slots,day\n")
    with open(args.contracts) as f:
        wanted = [r["address"] for r in csv.DictReader(f)][: args.top]
    todo = [a for a in wanted if a not in done]
    print(f"[slots] {len(done)} already fetched, {len(todo)} to go", file=sys.stderr)
    with open(args.out, "a") as f:
        for i, a in enumerate(todo, 1):
            got = latest_on_or_before(a, args.day)
            day, n = got if got else ("", 0)
            f.write(f"{a},{n},{day}\n")
            f.flush()
            if i % 100 == 0:
                print(f"[slots] {i}/{len(todo)}", file=sys.stderr)
            time.sleep(args.pause)
    print(f"[slots] wrote {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
