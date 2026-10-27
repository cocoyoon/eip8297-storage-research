# /// script
# requires-python = ">=3.10"
# ///
"""One table over several `pbt-char lsm-validate` runs.

For each run label it reads `results/<prefix>_<label>.md` (the settings, the
window's updates and its unix-micros bounds, the engine's levels at the end,
the model's per-level outflow with the engine's levels) and the engine's
event log `<logs>/<prefix>_<label>.LOG` (per level, the entries moved down
into it and the entries of it rewritten inside the window, through
`lsm_log_flows.parse`), and prints per run a per-level table and over all
runs a totals table: entries written per update by the event log, by the
engine's byte tickers over the on-disk entry, and by the two models. The
event log's count is the one to set against the model's amplification, which
counts entries; the byte figure divides by the mean on-disk entry at the end,
which cold ballast beside the originals shrinks (its keys compress away), so
it overcounts in those runs.

Where the run's own log (`<prefix>_<label>.log`) carries first touches (keys
the replayed blocks touch for the first time: entries new to every level,
which sink to the last one), the window's first touches per update and their
course over the run are printed, and one more level of the same size ratio is
extrapolated below the engine's last with `--extrapolate`: it receives the window's first touches
and rewrites per moved entry what the engine's last level did. That stands
for the one level mainnet's per-part state has beyond these runs.

The ballast column tells the two designs apart: `prefix` copies sit under
other first bytes, in key regions a compaction over the stream's keys never
overlaps, so those runs' deeper levels rewrite less than a state of that size
would (their last-level figures understate); `beside` copies sit under the
original's key with one more byte, interleaved as the untouched state is.
Trivial moves (a file handed down a level unchanged, no write) are not
compactions and do not appear in the per-level flows; the written figures
are complete without them.

usage: uv run tools/mainnet/lsm_validate_summary.py --prefix results/lsm_validate_24000000_p0 --logs DIR [--out OUT.md] LABEL...
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lsm_log_flows import parse  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("labels", nargs="+")
ap.add_argument("--prefix", required=True)
ap.add_argument("--logs", required=True)
ap.add_argument("--out")
ap.add_argument("--extrapolate", action="store_true", help="add one more level below the engine's last, fed by the window's first touches")
a = ap.parse_args()


def setting(md, name):
    m = re.search(r"^\| " + re.escape(name) + r"[^|]*\| ([^|]+) \|", md, re.M)
    return m.group(1).strip() if m else "-"


out = []
totals = []
for label in a.labels:
    md_path = f"{a.prefix}_{label}.md"
    md = open(md_path).read()
    updates = int(re.search(r"measured window: .*?, (\d+) updates \(", md).group(1))
    m = re.search(r"between unix micros (\d+) and (\d+)", md)
    if m:
        since_us, until_us = int(m.group(1)), int(m.group(2))
        flushes, per, _ = parse(f"{a.logs}/{os.path.basename(a.prefix)}_{label}.LOG", since_micros=since_us, until_micros=until_us)
        win = "by unix micros"
    else:
        # Older reports: seconds from the run's own log, the window's first and last block lines.
        log = open(f"{a.prefix}_{label}.log").read()
        mf = int(re.search(r"measured window: blocks (\d+)\.\.", md).group(1))
        s = float(re.search(rf"block {mf} \(([\d.]+)s\)", log).group(1))
        e = max(float(x) for x in re.findall(r"block \d+ \(([\d.]+)s\)", log)) + 2.0
        flushes, per, _ = parse(f"{a.logs}/{os.path.basename(a.prefix)}_{label}.LOG", since=s, until=e)
        win = f"by seconds {s:.0f}..{e:.0f}"
    # The engine's levels at the end.
    sizes = {}
    for lvl, files, mb in re.findall(r"^\s+(\d+)\s+(\d+)\s+(\d+)\s*$", md, re.M):
        if int(mb) > 0:
            sizes[int(lvl)] = int(mb)
    # The model's levels with the engine's levels: outflow per update, in order (buffer, L1, ...).
    sec = md.split("Model levels, engine's levels and memtable:")[1].split("Engine per level")[0]
    model = [(r[0], float(r[1]), float(r[4]), float(r[5])) for r in re.findall(r"^\| (\w+) \| (\d+) \| ([\d-]+) \| (\w+) \| ([\d.]+) \| ([\d.]+) \| ([\d.]+) \|", sec, re.M)]
    side = re.search(r"^\| entries written per update \| ([\d.]+) \| ([\d.]+) \| ([\d.]+) \|", md, re.M)
    fe = sum(e for _, _, e in flushes)
    bm = re.search(r"plus (\d+) ballast puts \((\d+) copies", md)
    ballast = (bm.group(2) + (", beside" if "with one more byte" in md else ", prefix")) if bm else "0"
    # First touches: the window's rate from the report, their course from the run's log.
    ftm = re.search(r"(\d+) of them first inside the window \(([\d.]+) per update", md)
    ft_rate = float(ftm.group(2)) if ftm else None
    log_path = f"{a.prefix}_{label}.log"
    course = []
    if os.path.exists(log_path):
        rows = [(int(b), int(u), int(d)) for b, u, d in re.findall(r"block (\d+) \([\d.]+s\): (\d+) updates, .*? first touches \d+ \((\d+) in the last 1000 blocks\)", open(log_path).read())]
        prev_u, acc_d, acc_u = 0, 0, 0
        for b, u, d in rows:
            acc_d += d
            acc_u += u - prev_u
            prev_u = u
            if b % 5000 == 0:
                course.append((b, acc_d / max(acc_u, 1)))
                acc_d, acc_u = 0, 0
    out.append(f"\n### {label}\n")
    out.append(f"buffer {setting(md, 'write buffer')}, L0 trigger {setting(md, 'L0 files')}, level base {setting(md, 'level base')}, key {setting(md, 'key layout')}, ballast {ballast}; window {updates} updates ({win}); levels at the end: " + ", ".join(f"L{l} {mb} MB" for l, mb in sorted(sizes.items())) + ("; index mode " + setting(md, 'index mode') if 'index mode' in md else "") + "\n")
    out.append("| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |")
    out.append("|---|---|---|---|---|---|---|---|---|---|---|")
    out.append(f"| L0 (flush) | - | - | - | - | - | {fe / updates:.3f} | buffer | {model[0][2] if model else 0:.3f} | {model[0][3] if model else 0:.3f} | - |")
    levels = sorted(per)
    prev = None
    total_e = fe
    for i, lvl in enumerate(levels):
        p = per[lvl]
        mv, rw = p["moved_e"] / updates, p["rewritten_e"] / updates
        ratio = p["rewritten_e"] / p["moved_e"] if p["moved_e"] else 0.0
        size_ratio = sizes.get(lvl, 0) / sizes[prev] if prev in sizes and sizes[prev] else 0.0
        total_e += p["out_e"]
        ml = model[i] if i < len(model) else None
        mo = f"{ml[2]:.3f}" if ml else "-"
        mw = f"{ml[3]:.3f}" if ml else "-"
        em = f"{mv / ml[2]:.2f}" if ml and ml[2] > 0 else "-"
        out.append(f"| L{lvl} | {sizes.get(lvl, 0)} | {mv:.3f} | {rw:.3f} | {ratio:.1f} | {size_ratio:.1f} | {p['out_e'] / updates:.3f} | {ml[0] if ml else '-'} | {mo} | {mw} | {em} |")
        prev = lvl
    out.append(f"\nEntries written per update over the window: {total_e / updates:.2f} by the event log; {side.group(1) if side else '-'} by the byte tickers over the on-disk entry; model nominal {side.group(2) if side else '-'}, model with the engine's levels and memtable {side.group(3) if side else '-'}.")
    # The engine's last level is the deepest one holding data at the end;
    # a run whose window never compacted into it understates.
    last = max(sizes) if sizes else None
    last_n = per[last]["n"] if last is not None and last in per else 0
    out.append(f"\nCompactions into the engine's last level (L{last}) inside the window: {last_n}." if last is not None else "")
    extra = None
    if ft_rate is not None:
        out.append(f"First touches inside the window: {ft_rate:.3f} per update" + (" (per update, by 5000 blocks: " + ", ".join(f"{b // 1000}k {r:.3f}" for b, r in course) + ")" if course else "") + ".")
        if a.extrapolate and last is not None and last_n > 0 and per[last]["moved_e"]:
            rpm = per[last]["rewritten_e"] / per[last]["moved_e"]
            extra = ft_rate * (1.0 + rpm)
            by_bytes = float(side.group(1)) * (1.0 + extra / (total_e / updates)) if side else 0.0
            out.append(f"One more level below, as mainnet's per-part state has (MODELED from the engine's figures: it receives the window's first touches, {ft_rate:.3f} per update, and rewrites {rpm:.1f} per moved entry as L{last} did): +{extra:.2f}, so {total_e / updates + extra:.2f} entries per update by the event log, {by_bytes:.2f} by bytes; an upper bound, since the level above it at mainnet keeps keys longer than this stream is.")
    totals.append((label, setting(md, 'write buffer'), setting(md, 'level base'), setting(md, 'key layout'), ballast, f"{len(sizes)} ({len(levels)} fed)", total_e / updates, side.group(1) if side else '-', side.group(2) if side else '-', side.group(3) if side else '-', last_n, f"{ft_rate:.3f}" if ft_rate is not None else "-", f"{total_e / updates + extra:.2f}" if extra is not None else "-"))

head = ["## Entries written per update, all runs\n", "| run | buffer | level base | key | ballast | levels below L0 (fed in the window) | engine, entries by the event log | engine, bytes / on-disk entry | model nominal | engine / model, by entries | engine / model, by bytes | model, engine's levels | compactions into the last level | first touches per update |" + (" with one more level |" if a.extrapolate else ""), "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|" + ("---|" if a.extrapolate else "")]
for t in totals:
    by_e = f"{t[6] / float(t[8]):.2f}" if t[8] != '-' and float(t[8]) > 0 else "-"
    by_b = f"{float(t[7]) / float(t[8]):.2f}" if t[7] != '-' and t[8] != '-' and float(t[8]) > 0 else "-"
    head.append(f"| {t[0]} | {t[1]} | {t[2]} | {t[3]} | {t[4]} | {t[5]} | {t[6]:.2f} | {t[7]} | {t[8]} | {by_e} | {by_b} | {t[9]} | {t[10]} | {t[11]} |" + (f" {t[12]} |" if a.extrapolate else ""))
text = "\n".join(head + out) + "\n"
print(text)
if a.out:
    open(a.out, "w").write(text)
