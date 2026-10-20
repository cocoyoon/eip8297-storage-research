# /// script
# requires-python = ">=3.10"
# dependencies = ["pyarrow>=15"]
# ///
"""Write locality of mainnet state, at the level of state keys.

Reads state diffs fetched by fetch.sh and reports how many distinct keys a
block writes, how soon a written key is written again, and how many distinct
keys are written within a horizon of T blocks.

What this is NOT: there is no tree here, no PBT key derivation and no cost
model. A "key" is a storage slot (address, slot) or an account (address)
whose balance or nonce changed. It answers one question the synthetic
generator cannot: how concentrated mainnet writes are in time.

Counting rules, the same ones the characterization harness uses:
- A block is the unit. Several changes to one key inside a block are one
  write, classified by the value before the block and the value after it.
- A key whose value after the block equals its value before is NOT a write.
- Creation, rewrite and deletion are counted separately.

usage: uv run tools/mainnet/working_set.py --data DIR --out results --label NAME
"""
import argparse
import collections
import hashlib
import os
import sys

import pyarrow.parquet as pq

ZERO_WORD = b"0x" + b"0" * 64
HORIZONS = [1, 3, 10, 30, 100, 300, 1000, 3000, 7200, 21600, 50400]
SECONDS_PER_BLOCK = 12


def kid(*parts: bytes) -> bytes:
    """Compact key identity. 12 bytes: collisions are negligible at the
    tens of millions of keys a run sees."""
    return hashlib.blake2b(b"|".join(parts), digest_size=12).digest()


def read(path, columns):
    t = pq.read_table(path, columns=columns)
    return [t.column(c).to_pylist() for c in columns]


class Existence:
    """What the whole sample says about whether an account existed.

    A balance diff alone cannot tell a first funding from a refill: the
    nonce is absent from a block's diffs unless it moved. The sample as a
    whole often can. Nonces only grow, so an address whose FIRST nonce diff
    starts above zero had a nonzero nonce throughout, and one whose first
    nonce diff starts at zero had none before that block. An address with a
    storage write is a contract. Where the sample says nothing, a balance
    that leaves zero is counted as a creation: an account with no balance,
    no nonce and no code does not exist.
    """

    def __init__(self, data, chunks):
        self.first_nonce = {}  # address -> (block, nonce before it)
        self.contracts = set()
        self.alive = set()  # accounts the sample has already shown to exist
        self.unknown = 0  # creations called on absence of evidence
        for chunk in chunks:
            cols = ["block_number", "transaction_index", "internal_index", "address", "from_value"]
            for b, ti, ii, a, fv in sorted(zip(*read(f"{data}/nonce_diffs/{chunk}.parquet", cols)), key=lambda r: r[:3]):
                self.first_nonce.setdefault(a, (b, fv))
            self.contracts.update(read(f"{data}/storage_diffs/{chunk}.parquet", ["address"])[0])

    def existed(self, address, block):
        """Whether `address` existed before `block`, in which its balance
        left zero and its nonce did not move."""
        if address in self.contracts or address in self.alive:
            return True
        seen = self.first_nonce.get(address)
        if seen is None:
            self.unknown += 1
            return False
        first_block, before = seen
        return before > 0 or block > first_block


def chunk_blocks(data, chunk, existence=None):
    """Per-block net effect for one 1000-block chunk.

    Returns {block: (slots, accts)} where slots maps key id ->
    (kind, address, low_slot, slot) and accts maps key id -> (kind, address).
    kind is one of 'create', 'rewrite', 'delete'. Net-zero keys are dropped.
    `existence` carries what the rest of the sample knows about accounts;
    call the chunks in block order when passing it.
    """
    # storage: first value before and last value after, per (block, key)
    first, last, meta = {}, {}, {}
    cols = ["block_number", "transaction_index", "internal_index", "address", "slot", "from_value", "to_value"]
    rows = zip(*read(f"{data}/storage_diffs/{chunk}.parquet", cols))
    for b, ti, ii, a, s, fv, tv in sorted(rows, key=lambda r: (r[0], r[1], r[2])):
        k = (b, kid(a, s))
        if k not in first:
            first[k] = fv
            meta[k] = (a, int(s, 16) < 64, s)
        last[k] = tv
    out = collections.defaultdict(lambda: ({}, {}))
    for (b, k), fv in first.items():
        tv = last[(b, k)]
        if fv == tv:
            continue  # net-zero inside the block
        kind = "create" if fv == ZERO_WORD else "delete" if tv == ZERO_WORD else "rewrite"
        out[b][0][k] = (kind, *meta[(b, k)])

    # accounts: balance and nonce, same net rule. An account exists when it
    # has a balance, a nonce or code. A field absent from a block's diffs
    # did not move in it, so its value there is unknown:
    # - balance 0 -> X with the nonce unmoved is a creation only if the
    #   nonce was zero and there is no code, which `existence` judges from
    #   the whole sample (without it, such a change is called a rewrite);
    # - balance X -> 0 with the nonce unmoved is never a deletion: an
    #   account that pays out without sending a transaction has code.
    bal_first, bal_last, non_first, non_last = {}, {}, {}, {}
    cols = ["block_number", "transaction_index", "internal_index", "address", "from_value", "to_value"]
    for name, f, l in (("balance_diffs", bal_first, bal_last), ("nonce_diffs", non_first, non_last)):
        rows = zip(*read(f"{data}/{name}/{chunk}.parquet", cols))
        for b, ti, ii, a, fv, tv in sorted(rows, key=lambda r: (r[0], r[1], r[2])):
            k = (b, a)
            if k not in f:
                f[k] = fv
            l[k] = tv
    zero_bal = bytes(32)
    for k in sorted(set(bal_first) | set(non_first)):
        b, a = k
        bf, bl = bal_first.get(k), bal_last.get(k)
        nf, nl = non_first.get(k), non_last.get(k)
        if bf == bl and nf == nl:
            continue
        if nf is None:
            before_empty = bf == zero_bal and existence is not None and not existence.existed(a, b)
            after_empty = False
        else:
            # The nonce moved. Leaving zero with the balance at zero is a
            # creation. Leaving zero with the balance unmoved is a contract
            # deployed without value: a creation too, unless the sample
            # already showed the address holding a balance.
            unfunded = bf is None and existence is not None and a not in existence.alive
            before_empty = nf == 0 and (bf == zero_bal or unfunded)
            after_empty = nl == 0 and bl == zero_bal
        kind = "create" if before_empty and not after_empty else "delete" if after_empty and not before_empty else "rewrite"
        if existence is not None:
            if kind == "create" and a in existence.alive:
                kind = "rewrite"
            if kind == "delete":
                existence.alive.discard(a)
            else:
                existence.alive.add(a)
        out[b][1][kid(a)] = (kind, a)

    cols = ["block_number", "n_code_bytes", "code_hash"]
    contracts = list(zip(*read(f"{data}/contracts/{chunk}.parquet", cols)))
    return out, contracts


class Locality:
    """Backward write gaps for one class of key, in blocks."""

    def __init__(self, first_block):
        self.first_block = first_block
        self.last = {}  # key -> block of its latest write
        self.writes = 0
        self.kinds = collections.Counter()
        # per horizon: writes that had a chance to hit, and those that did
        self.eligible = collections.Counter()
        self.hits = collections.Counter()
        # sum over consecutive writes of min(gap, T): see working_set()
        self.covered = collections.Counter()

    def write(self, key, block, kind):
        self.writes += 1
        self.kinds[kind] += 1
        prev = self.last.get(key)
        gap = None if prev is None else block - prev
        elapsed = block - self.first_block
        for t in HORIZONS:
            # Only blocks at least T into the sample can tell a miss from a
            # key whose earlier write fell before the sample began.
            if elapsed >= t:
                self.eligible[t] += 1
                if gap is not None and gap <= t:
                    self.hits[t] += 1
            if gap is not None:
                self.covered[t] += min(gap, t)
        self.last[key] = block

    def working_set(self, t, end_block, n_blocks):
        """Mean number of distinct keys written in a window of t blocks.

        Each write keeps its key inside the window for min(gap to the next
        write of that key, t) blocks; the final write of a key for
        min(blocks left in the sample, t). Dividing the total by the number
        of blocks gives the mean window content. Trustworthy only while t
        is well under the sample length."""
        tail = sum(min(end_block - b + 1, t) for b in self.last.values())
        return (self.covered[t] + tail) / n_blocks


def human(blocks):
    s = blocks * SECONDS_PER_BLOCK
    if s < 3600:
        return f"{s / 60:.0f} min"
    if s < 86400:
        return f"{s / 3600:.1f} h"
    return f"{s / 86400:.1f} d"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--out", default="results")
    ap.add_argument("--label", required=True)
    args = ap.parse_args()

    chunks = sorted(int(f.split(".")[0]) for f in os.listdir(f"{args.data}/storage_diffs") if f.endswith(".parquet"))
    if not chunks:
        sys.exit("no chunks found")
    for a, b in zip(chunks, chunks[1:]):
        if b - a != 1000:
            sys.exit(f"chunks are not contiguous: {a} then {b}")
    first_block, end_block = chunks[0], chunks[-1] + 999
    n_blocks = end_block - first_block + 1

    slots, accts = Locality(first_block), Locality(first_block)
    per_block_slots, per_block_accts = [], []
    per_contract = collections.Counter()
    header_slots = 0
    n_contracts = code_bytes = 0
    code_hashes = set()

    existence = Existence(args.data, chunks)
    for chunk in chunks:
        blocks, contracts = chunk_blocks(args.data, chunk, existence)
        for b in range(chunk, chunk + 1000):
            s, a = blocks.get(b, ({}, {}))
            per_block_slots.append(len(s))
            per_block_accts.append(len(a))
            for k, (kind, addr, low, _slot) in s.items():
                slots.write(k, b, kind)
                per_contract[addr] += 1
                header_slots += low
            for k, (kind, _addr) in a.items():
                accts.write(k, b, kind)
        for _, n, h in contracts:
            n_contracts += 1
            code_bytes += n or 0
            code_hashes.add(h)
        print(f"[working_set] chunk {chunk} done", file=sys.stderr)

    def stats(x):
        x = sorted(x)
        return f"{sum(x) / len(x):.0f} | {x[len(x) // 2]} | {x[int(len(x) * 0.9)]} | {x[-1]}"

    md = []
    w = md.append
    w("# Mainnet write locality, by state key\n")
    w("> Nature: MAINNET data, key level only. No tree, no PBT key derivation, no cost\n> model. A key is a storage slot or an account whose balance or nonce changed.\n> Source: Xatu public dataset (ethpandaops, CC BY 4.0), state diffs extracted\n> with cryo. A block is the unit: a key changed several times in one block is\n> one write, and a key whose value ends the block where it began is not a write.\n")
    w(f"- blocks {first_block} to {end_block} ({n_blocks} blocks, about {human(n_blocks)})")
    w(f"- reproduce: `tools/mainnet/fetch.sh DIR {first_block} {len(chunks)}` then `uv run tools/mainnet/working_set.py --data DIR --out results --label {args.label}`\n")

    w("## Writes per block\n")
    w("| key class | mean | median | p90 | max |")
    w("|---|---|---|---|---|")
    w(f"| storage slots | {stats(per_block_slots)} |")
    w(f"| accounts | {stats(per_block_accts)} |")
    w(f"| both | {stats([a + b for a, b in zip(per_block_slots, per_block_accts)])} |\n")

    w("## Creation, rewrite, deletion\n")
    w("Counted separately: only a rewrite or a deletion leaves a dead record behind.\n")
    w("| key class | writes | created | rewritten | deleted |")
    w("|---|---|---|---|---|")
    for name, loc in (("storage slots", slots), ("accounts", accts)):
        n = max(loc.writes, 1)
        w(f"| {name} | {loc.writes} | {100 * loc.kinds['create'] / n:.1f}% | {100 * loc.kinds['rewrite'] / n:.1f}% | {100 * loc.kinds['delete'] / n:.1f}% |")
    w(f"\nAn account counts as created when it had no balance, no nonce and no code before the block. Where a block's diffs show the balance leaving zero and no nonce, the rest of the sample decides: an address with a storage write or a nonzero nonce already existed. For {existence.unknown} such changes ({100 * existence.unknown / max(accts.writes, 1):.1f}% of account writes) the sample says nothing and the change is counted as a creation. A balance reaching zero with the nonce unmoved is not a deletion: only an account with code pays out without sending a transaction.\n")

    w("## How soon a written key is written again\n")
    w("`rewritten within T`: of the writes made at least T blocks into the sample, the share whose key was last written no more than T blocks earlier. `distinct in T`: the mean number of distinct keys written in a window of T blocks, which is what a cache has to hold to serve that share. Rows whose T exceeds half the sample are left out.\n")
    w("| T blocks | about | slots rewritten within T | accounts rewritten within T | distinct slots in T | distinct accounts in T |")
    w("|---|---|---|---|---|---|")
    for t in HORIZONS:
        if t > n_blocks // 2:
            continue
        def share(loc):
            return f"{100 * loc.hits[t] / loc.eligible[t]:.1f}%" if loc.eligible[t] else "-"
        w(f"| {t} | {human(t)} | {share(slots)} | {share(accts)} | {slots.working_set(t, end_block, n_blocks):.0f} | {accts.working_set(t, end_block, n_blocks):.0f} |")
    w(f"\nDistinct keys over the whole sample: {len(slots.last)} storage slots, {len(accts.last)} accounts.\n")

    w("## Concentration\n")
    total = sum(per_contract.values())
    top = per_contract.most_common()
    w(f"{len(top)} contracts had a storage write. Share of storage-slot writes taken by the busiest:\n")
    w("| contracts | share of slot writes |")
    w("|---|---|")
    for k in (1, 10, 100, 1000):
        w(f"| top {k} | {100 * sum(c for _, c in top[:k]) / max(total, 1):.1f}% |")
    w(f"\nSlot numbers below 64, which EIP-8297 keeps in the account's own header stem: {100 * header_slots / max(total, 1):.1f}% of slot writes.\n")

    w("## Code\n")
    w(f"{n_contracts} contracts created, {len(code_hashes)} distinct code hashes, {code_bytes} code bytes in total. That is {n_contracts / n_blocks:.1f} creations per block, of which {100 * (1 - len(code_hashes) / max(n_contracts, 1)):.1f}% reuse code already seen in this sample.\n")

    os.makedirs(args.out, exist_ok=True)
    path = f"{args.out}/{args.label}.md"
    with open(path, "w") as f:
        f.write("\n".join(md))
    print(f"[working_set] wrote {path}", file=sys.stderr)


if __name__ == "__main__":
    main()
