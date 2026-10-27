# /// script
# requires-python = ">=3.10"
# ///
"""Combine the reports of a replay run in key-space parts into one.

A replay with --mainnet-sample-bits B --mainnet-sample-part q covers the
accounts and contracts whose address hash starts with the B bits of q, and
its report scales every figure to the whole. Run every part q of 2^B and
the parts together cover each key exactly once: their scaled figures
average to the whole. The one approximation is the cache: a budget D is
split into 2^B caches of D / 2^B, one per part, instead of one shared LRU.

Per-block counts and IO costs are averaged cell by cell (each end of a
`lo-hi` range on its own) and the verdict is recomputed with the same rule
as the report: the cost columns fall into families (the text before the
first comma: page, log, LSM, B+tree), each represented by its variant with
the best worst case, and a family is named only if it beats every other
family by TOO_CLOSE or more whichever end of its own range each is at. The
columns are read from the parts' own table headers. Rows are kept only for
DRAM budgets every part reports.

usage: uv run tools/mainnet/combine_parts.py --out FILE PART.md ...
"""
import argparse
import re
import sys

TOO_CLOSE = 1.3


def cell(s):
    s = s.strip()
    if s == "-":
        return None
    if "-" in s:
        a, b = s.split("-")
        return float(a), float(b)
    return float(s), float(s)


def fmt(x):
    if x is None:
        return "-"
    lo, hi = x
    return f"{hi:.0f}" if abs(hi - lo) < 0.5 else f"{lo:.0f}-{hi:.0f}"


def family(name):
    """'page, NOMT value store' -> 'page': the text before the first comma."""
    return name.split(",")[0].strip()


def verdict(cells, names):
    """The same rule as `bench.rs::verdict`: W is the family holding the
    smallest hi (its `safest` variant), R the other family nearest it by lo
    (its `keenest` variant); W wins by at least keenest_R.lo / safest_W.hi."""
    fams, order = {}, []  # family -> [safest, keenest]
    for span, name in zip(cells, names):
        if span is None:
            continue
        f = family(name)
        if f not in fams:
            fams[f] = [span, span]
            order.append(f)
        else:
            if span[1] < fams[f][0][1]:
                fams[f][0] = span
            if span[0] < fams[f][1][0]:
                fams[f][1] = span
    if not fams:
        return "-"
    if len(fams) == 1:
        return f"{order[0]} (others cannot run)"
    w = min(order, key=lambda f: fams[f][0][1])  # min keeps the first on ties
    r = min((f for f in order if f != w), key=lambda f: fams[f][1][0])
    (ws, wk), (rs, rk) = fams[w], fams[r]
    r_min, r_max = rk[0] / ws[1], rs[1] / ws[0]
    if r_min >= TOO_CLOSE:
        return f"{w} ({r_min:.2f}x to {r_max:.2f}x over {r})"
    if ws[1] <= rk[0]:
        return f"too close ({w} by {r_min:.2f}x to {r_max:.2f}x over {r})"
    a, b = rs[1] / wk[0], ws[1] / rk[0]
    word = "too close" if max(a, b) < TOO_CLOSE else "undecided"
    return f"{word} ({w} by up to {a:.2f}x, {r} by up to {b:.2f}x)"


def tables(md):
    """{title: (columns, {dram_gb: [cells]})} for every IO cost table. The
    columns are the store names between `DRAM GB` and `cheapest`."""
    out, title = {}, None
    for line in md.splitlines():
        if line.startswith("### IO cost per block, mainnet replay"):
            title = line[4:]
            out[title] = (None, {})
        elif title and line.startswith("| DRAM"):
            cols = [c.strip() for c in line.strip("|").split("|")]
            out[title] = (cols[1:-1], out[title][1])
        elif title and line.startswith("| ") and not line.startswith("|---"):
            cols = [c.strip() for c in line.strip("|").split("|")]
            out[title][1][cols[0]] = [cell(c) for c in cols[1:-1]]
        elif title and line.startswith("###"):
            title = None
    return out


MIX_HEADER = "| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |"


def mix_table(md):
    """{(dram_gb, store): [numbers]} from the IO mix table, or {} if absent."""
    out, on = {}, False
    for line in md.splitlines():
        if line.startswith("### IO mix per block"):
            on = True
        elif on and line.startswith("| ") and not line.startswith("| DRAM"):
            cols = [c.strip() for c in line.strip("|").split("|")]
            out[(cols[0], cols[1])] = [float(c) for c in cols[2:]]
        elif on and line.startswith("###"):
            break
    return out


FIGURES = [
    ("log store: records written per block", r"measured, log store, per block: ([\d.]+) records written"),
    ("log store: reads per block with no cache", r"With no cache at all: ([\d.]+) reads per block for the log store"),
    ("page store: merkle pages written per block", r"measured, page store, per block: ([\d.]+) merkle pages written"),
    ("page store: reads per block with no cache", r"reads per block for the log store, ([\d.]+) for the page store"),
    ("log store: reads of records untouched since the replay began", r"Of the log store's, ([\d.]+) are of records untouched"),
    ("page store: reads of pages untouched since the replay began", r"of the page store's, ([\d.]+)\."),
    ("execution reads of existing keys per block", r"execution reads, per block: ([\d.]+) of existing keys"),
    ("index write amplification", r"Write amplification ([\d.]+) per index update"),
    ("LSM of records: write amplification per record update", r"Record write amplification ([\d.]+) per record update"),
    ("LSM of records: the model's write amplification per record update", r"plus one WAL copy of each update\), the model's ([\d.]+) scaled by"),
    ("index: the model's write amplification per index update", r"the model gives ([\d.]+), and a real RocksDB fed this stream"),
    ("LSM of records: bytes per entry", r"An entry is ([\d.]+) B, the mean record"),
    ("B+tree: pages rewritten per block", r"measured, B\+tree, per block: ([\d.]+) pages rewritten"),
    ("B+tree: leaf-page reads per block with no cache", r"pages rewritten copy-on-write; with no cache ([\d.]+) leaf-page reads"),
    ("page log: pages appended per block", r"page log: ([\d.]+) pages appended per block"),
    ("page LSM: write amplification per page update", r"page write amplification ([\d.]+) per page update"),
    ("log store: relocations per block at g = 2/3", r"relocations ([\d.]+) per block at g = 2/3"),
    ("page store: elided-page touches per block", r"the ([\d.]+) elided-page touches per block"),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("parts", nargs="+")
    args = ap.parse_args()
    mds = [open(p).read() for p in args.parts]
    for p, md in zip(args.parts, mds):
        if "- SAMPLED:" not in md:
            sys.exit(f"{p} is not a sampled replay report")
    n = len(mds)
    o = [f"# Mainnet replay, {n} key-space parts combined\n"]
    o.append(
        "> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; "
        "together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as "
        f"{n} caches of 1/{n} each, one per part, instead of one shared cache. Costs are averaged cell by cell; the "
        f"verdict is recomputed (a family of stores is named only if it beats every other family by {TOO_CLOSE}x or more whichever end of its own range each is at).\n"
    )
    o.append("Parts: " + ", ".join(f"`{p}`" for p in args.parts) + "\n")
    o.append("| figure | " + " | ".join(f"part {i}" for i in range(n)) + " | combined |")
    o.append("|---|" + "---|" * (n + 1))
    for name, rx in FIGURES:
        vals = []
        for md in mds:
            m = re.search(rx, md)
            vals.append(float(m.group(1)) if m else None)
        if any(v is None for v in vals):
            continue
        o.append(f"| {name} | " + " | ".join(f"{v:.1f}" for v in vals) + f" | {sum(vals) / n:.1f} |")
    o.append("")
    ts = [tables(md) for md in mds]
    for title in ts[0]:
        header, first = ts[0][title]
        if header is None or any(t[title][0] != header for t in ts):
            sys.exit(f"parts disagree on the columns of {title!r}")
        o.append(f"### {title}\n")
        o.append("| DRAM GB | " + " | ".join(header) + " | cheapest |")
        o.append("|" + "---|" * (len(header) + 2))
        for gb, cells in first.items():
            rows = [t[title][1].get(gb) for t in ts]
            if any(r is None for r in rows):
                continue  # a budget only some parts report
            avg = []
            for c in range(len(cells)):
                col = [r[c] for r in rows]
                if any(x is None for x in col):
                    avg.append(None)
                else:
                    avg.append((sum(x[0] for x in col) / n, sum(x[1] for x in col) / n))
            o.append(f"| {gb} | " + " | ".join(fmt(x) for x in avg) + f" | {verdict(avg, header)} |")
        o.append("")
    # The IO mix for timed IO: plain numbers, averaged cell by cell.
    mixes = [mix_table(md) for md in mds]
    if all(mixes):
        o.append("### IO mix per block, for timed IO (`hi` bound)\n")
        o.append(MIX_HEADER)
        o.append("|---|" * 0 + "|---" * len(MIX_HEADER.strip("|").split("|")) + "|")
        for key in mixes[0]:
            rows = [m.get(key) for m in mixes]
            if any(r is None for r in rows):
                continue
            avg = [sum(r[c] for r in rows) / n for c in range(len(rows[0]))]
            o.append(f"| {key[0]} | {key[1]} | " + " | ".join(f"{v:.2f}" if c == 5 else f"{v:.0f}" for c, v in enumerate(avg)) + " |")
        o.append("")
    with open(args.out, "w") as f:
        f.write("\n".join(o))
    print(f"[combine] {n} parts -> {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
