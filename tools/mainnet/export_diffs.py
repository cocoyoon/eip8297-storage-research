# /// script
# requires-python = ">=3.10"
# dependencies = ["pyarrow>=15"]
# ///
"""Export mainnet state diffs as the per-block net changes a replay needs.

Same counting rules as working_set.py (it supplies them): a block is the
unit, a key that ends the block where it began is dropped, and every change
is a creation, a rewrite or a deletion. Whether an account existed before
a block is judged from the whole sample (working_set.Existence), so a file
exported from more blocks can classify an early change differently.

Output, one text file:
    # pbt-char mainnet diffs v1
    B <block number>
    A <c|r|d> <address, 40 hex>
    S <c|r|d> <address, 40 hex> <slot, 64 hex>
    R A <address, 40 hex>                 (with --reads: read, exists)
    R S <address, 40 hex> <slot, 64 hex>
    Z A <address, 40 hex>                 (with --reads: read, absent)
    Z S <address, 40 hex> <slot, 64 hex>
Values are not exported. The replay only needs to know that a key changed
and how; it writes a fresh value of its own.

With --reads, each block also lists the keys its execution read but did
not change: distinct per block (a client reads a key from disk once per
block at most), minus the keys the block changes (those are charged as
the read before the write). A read that returned zero met a key that does
not exist (an empty slot, or an account with no balance and no nonce);
those are listed as Z lines.

Also writes <out>.contracts.csv: every contract with a storage write, its
write count and distinct slots in the sample, busiest first. That is the
list fetch_slot_counts.py asks the state-size API about.

usage: uv run tools/mainnet/export_diffs.py --data DIR --out FILE
"""
import argparse
import collections
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import working_set as ws  # noqa: E402

KIND = {"create": "c", "rewrite": "r", "delete": "d"}


def hex40(a: bytes) -> str:
    s = a.decode()
    assert s.startswith("0x") and len(s) == 42, s
    return s[2:]


def hex64(v: bytes) -> str:
    s = v.decode()
    assert s.startswith("0x") and len(s) == 66, s
    return s[2:]


def is_zero(v) -> bool:
    if isinstance(v, int):
        return v == 0
    if len(v) == 32:
        return not any(v)  # raw 32-byte word (balances)
    assert v[:2] == b"0x", v
    return int(v, 16) == 0


def chunk_reads(data, chunk):
    """{block: (accounts, slots, absent accounts, absent slots)}: keys read
    with a nonzero value somewhere in the block, and keys read only as zero."""
    out = collections.defaultdict(lambda: (set(), set(), set(), set()))
    present_s, zero_s = collections.defaultdict(set), collections.defaultdict(set)
    rows = zip(*ws.read(f"{data}/storage_reads/{chunk}.parquet", ["block_number", "contract_address", "slot", "value"]))
    for b, a, sl, v in rows:
        (zero_s if is_zero(v) else present_s)[b].add((a, sl))
    present_a, zero_a = collections.defaultdict(set), collections.defaultdict(set)
    for name, col in (("balance_reads", "balance"), ("nonce_reads", "nonce")):
        rows = zip(*ws.read(f"{data}/{name}/{chunk}.parquet", ["block_number", "address", col]))
        for b, a, v in rows:
            (zero_a if is_zero(v) else present_a)[b].add(a)
    for b in set(present_s) | set(zero_s) | set(present_a) | set(zero_a):
        out[b][0].update(present_a[b])
        out[b][1].update(present_s[b])
        out[b][2].update(zero_a[b] - present_a[b])
        out[b][3].update(zero_s[b] - present_s[b])
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--reads", action="store_true", help="also list execution reads (fetch.sh ... reads)")
    args = ap.parse_args()
    n_read = collections.Counter()

    chunks = sorted(int(f.split(".")[0]) for f in os.listdir(f"{args.data}/storage_diffs") if f.endswith(".parquet"))
    for a, b in zip(chunks, chunks[1:]):
        if b - a != 1000:
            sys.exit(f"chunks are not contiguous: {a} then {b}")
    writes = collections.Counter()
    distinct = collections.defaultdict(set)
    n_a = n_s = 0
    with open(args.out, "w") as f:
        f.write("# pbt-char mainnet diffs v1\n")
        existence = ws.Existence(args.data, chunks)
        for chunk in chunks:
            blocks, _ = ws.chunk_blocks(args.data, chunk, existence)
            reads = chunk_reads(args.data, chunk) if args.reads else {}
            for b in range(chunk, chunk + 1000):
                slots, accts = blocks.get(b, ({}, {}))
                f.write(f"B {b}\n")
                # Sorted so the file is the same on every run.
                for kind, addr in sorted((k, hex40(a)) for k, a in accts.values()):
                    f.write(f"A {KIND[kind]} {addr}\n")
                    n_a += 1
                for kind, addr, slot in sorted((k, hex40(a), hex64(s)) for k, a, _, s in slots.values()):
                    f.write(f"S {KIND[kind]} {addr} {slot}\n")
                    writes[addr] += 1
                    distinct[addr].add(slot)
                    n_s += 1
                if b in reads:
                    ra, rs, za, zs = reads[b]
                    changed_a = {a for _, a in accts.values()}
                    changed_s = {(a, sl) for _, a, _, sl in slots.values()}
                    for tag, ka, ks in (("R", ra, rs), ("Z", za, zs)):
                        for addr in sorted(hex40(a) for a in ka - changed_a):
                            f.write(f"{tag} A {addr}\n")
                            n_read[tag + "A"] += 1
                        for addr, slot in sorted((hex40(a), hex64(sl)) for a, sl in ks - changed_s):
                            f.write(f"{tag} S {addr} {slot}\n")
                            n_read[tag + "S"] += 1
            print(f"[export] chunk {chunk} done", file=sys.stderr)
    with open(args.out + ".contracts.csv", "w") as f:
        f.write("address,slot_writes,distinct_slots\n")
        for addr, n in writes.most_common():
            f.write(f"{addr},{n},{len(distinct[addr])}\n")
    print(f"[export] {len(chunks) * 1000} blocks, {n_a} account changes, {n_s} slot changes, {len(writes)} contracts -> {args.out}", file=sys.stderr)
    if args.reads:
        print(f"[export] reads: {n_read['RA']} accounts, {n_read['RS']} slots; read as absent: {n_read['ZA']} accounts, {n_read['ZS']} slots", file=sys.stderr)


if __name__ == "__main__":
    main()
