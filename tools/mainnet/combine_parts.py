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
as the report: a store is named only if it wins by TOO_CLOSE or more
whichever end of its own range each store is at. Rows are kept only for
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


def verdict(page, logs):
    running = [l for l in logs if l is not None]
    if page is None:
        return "log (page cannot run)" if running else "-"
    if not running:
        return "page (log cannot run)"
    p_lo, p_hi = page
    safest = min(running, key=lambda r: r[1])
    keenest = min(running, key=lambda r: r[0])
    if p_lo / safest[1] >= TOO_CLOSE:
        return f"log ({p_lo / safest[1]:.2f}x to {p_hi / safest[0]:.2f}x)"
    if keenest[0] / p_hi >= TOO_CLOSE:
        return f"page ({keenest[0] / p_hi:.2f}x to {safest[1] / p_lo:.2f}x)"
    if safest[1] <= p_lo:
        return f"too close (log by {p_lo / safest[1]:.2f}x to {p_hi / safest[0]:.2f}x)"
    if keenest[0] >= p_hi:
        return f"too close (page by {keenest[0] / p_hi:.2f}x to {safest[1] / p_lo:.2f}x)"
    log_best, page_best = p_hi / keenest[0], safest[1] / p_lo
    word = "too close" if max(log_best, page_best) < TOO_CLOSE else "undecided"
    return f"{word} (log by up to {log_best:.2f}x, page by up to {page_best:.2f}x)"


def tables(md):
    """{title: {dram_gb: [cells]}} for every IO cost table."""
    out, title = {}, None
    for line in md.splitlines():
        if line.startswith("### IO cost per block, mainnet replay"):
            title = line[4:]
            out[title] = {}
        elif title and line.startswith("| ") and not line.startswith("| DRAM") and not line.startswith("|---"):
            cols = [c.strip() for c in line.strip("|").split("|")]
            out[title][cols[0]] = [cell(c) for c in cols[1:-1]]
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
        f"verdict is recomputed (a store is named only if it wins by {TOO_CLOSE}x or more at every end of both ranges).\n"
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
        o.append(f"### {title}\n")
        o.append("| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |")
        o.append("|---|---|---|---|---|---|")
        for gb, cells in ts[0][title].items():
            rows = [t[title].get(gb) for t in ts]
            if any(r is None for r in rows):
                continue  # a budget only some parts report
            avg = []
            for c in range(len(cells)):
                col = [r[c] for r in rows]
                if any(x is None for x in col):
                    avg.append(None)
                else:
                    avg.append((sum(x[0] for x in col) / n, sum(x[1] for x in col) / n))
            o.append(f"| {gb} | " + " | ".join(fmt(x) for x in avg) + f" | {verdict(avg[0], avg[1:])} |")
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
