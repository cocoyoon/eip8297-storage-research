# /// script
# requires-python = ">=3.10"
# ///
"""Per-level flows of a RocksDB run from its event log.

Reads the `EVENT_LOG_v1` lines of a RocksDB LOG written by `pbt-char
lsm-validate` and, for every compaction after the starting state's manual
compaction, splits its input into the entries that came down from the level
above (moved) and the entries of the output level it rewrote. Flushes are
listed as the entries written to L0. With `--updates N` (the replay's
updates, from the validator's report) every figure is also given per update.

With `--since-micros A --until-micros B` (unix micros, as the validator's
report prints for its measured window) or `--since S` / `--until S` (seconds
after the LOG's first line) only the flushes and compactions finishing inside
that window count, so the figures can be read per update of the window.

usage: uv run tools/mainnet/lsm_log_flows.py LOG [--updates N] [--since-micros A --until-micros B] [--out OUT.md]
"""
import argparse
import datetime as dt
import json
from collections import defaultdict


def stamp(line):
    try:
        return dt.datetime.strptime(line[:26], "%Y/%m/%d-%H:%M:%S.%f")
    except ValueError:
        return None


def parse(log, since=None, until=None, since_micros=None, until_micros=None):
    """Flushes and compactions of `log` inside the window.

    Returns (flushes, per, manual_job): flushes as [(job, bytes, entries)],
    per as {output level: dict(n, moved_b, moved_e, rewritten_b, rewritten_e,
    in_e, out_e, out_b)}, both after the starting state's manual compaction.
    """
    t0 = None

    def inside(line, ev):
        if since_micros is not None or until_micros is not None:
            t = ev.get("time_micros")
            if t is None:
                return False
            return (since_micros is None or t >= since_micros) and (until_micros is None or t < until_micros)
        if since is None and until is None:
            return True
        t = stamp(line)
        if t is None or t0 is None:
            return False
        s = (t - t0).total_seconds()
        return (since is None or s >= since) and (until is None or s < until)

    files = {}  # file number -> (bytes, entries, job)
    started = {}  # job -> {level: [file numbers]}
    manual_job = None
    flush_jobs = set()
    rows = []
    for line in open(log, errors="replace"):
        if t0 is None:
            t0 = stamp(line)
        i = line.find("EVENT_LOG_v1 ")
        if i < 0:
            continue
        try:
            ev = json.loads(line[i + len("EVENT_LOG_v1 "):])
        except json.JSONDecodeError:
            continue
        kind = ev.get("event")
        if kind == "table_file_creation":
            tp = ev.get("table_properties", {})
            files[ev["file_number"]] = (ev.get("file_size", 0), tp.get("num_entries", 0), ev.get("job"))
        elif kind == "compaction_started":
            inputs = {int(k[7:]): v for k, v in ev.items() if k.startswith("files_L")}
            started[ev["job"]] = inputs
            if ev.get("compaction_reason") == "ManualCompaction":
                manual_job = ev["job"]
        elif kind == "compaction_finished":
            if inside(line, ev):
                rows.append((ev["job"], ev["output_level"], started.get(ev["job"], {}), ev.get("num_input_records", 0), ev.get("num_output_records", 0), ev.get("total_output_size", 0)))
        elif kind == "flush_finished":
            if inside(line, ev):
                flush_jobs.add(ev["job"])

    flushes = [(job, b, e) for num, (b, e, job) in files.items() if job in flush_jobs and (manual_job is None or job > manual_job)]
    per = defaultdict(lambda: dict(n=0, moved_b=0, moved_e=0, rewritten_b=0, rewritten_e=0, in_e=0, out_e=0, out_b=0))
    for job, lvl, inputs, in_e, out_e, out_b in rows:
        if manual_job is not None and job <= manual_job:
            continue
        p = per[lvl]
        p["n"] += 1
        for l, nums in inputs.items():
            for num in nums:
                b, e, _ = files.get(num, (0, 0, None))
                if l < lvl:
                    p["moved_b"] += b
                    p["moved_e"] += e
                else:
                    p["rewritten_b"] += b
                    p["rewritten_e"] += e
        p["in_e"] += in_e
        p["out_e"] += out_e
        p["out_b"] += out_b
    return flushes, dict(per), manual_job


def table(flushes, per, manual_job, updates=0.0, window=""):
    fb = sum(b for _, b, _ in flushes)
    fe = sum(e for _, _, e in flushes)
    u = updates if updates > 0 else None

    def pu(x):
        return f"{x / u:.3f}" if u else "-"

    out = []
    out.append("| level written into | compactions | entries moved down into it | entries of it rewritten | rewritten per moved | entries out | MB out | moved per update | rewritten per update | out per update |")
    out.append("|---|---|---|---|---|---|---|---|---|---|")
    out.append(f"| L0 (flushes) | {len(flushes)} | {fe} | 0 | - | {fe} | {fb / 1e6:.0f} | - | - | {pu(fe)} |")
    for lvl in sorted(per):
        p = per[lvl]
        ratio = p["rewritten_e"] / p["moved_e"] if p["moved_e"] else 0.0
        out.append(f"| L{lvl} | {p['n']} | {p['moved_e']} | {p['rewritten_e']} | {ratio:.1f} | {p['out_e']} | {p['out_b'] / 1e6:.0f} | {pu(p['moved_e'])} | {pu(p['rewritten_e'])} | {pu(p['out_e'])} |")
    total_out = fe + sum(p["out_e"] for p in per.values())
    out.append(f"\nEntries written per update, flushes and compactions: {pu(total_out)} ({total_out} entries); after job {manual_job} (the starting state's manual compaction){window}.")
    return "\n".join(out) + "\n"


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("log")
    ap.add_argument("--updates", type=float, default=0.0)
    ap.add_argument("--out")
    ap.add_argument("--since", type=float, default=None, help="seconds after the LOG's first line")
    ap.add_argument("--until", type=float, default=None, help="seconds after the LOG's first line")
    ap.add_argument("--since-micros", type=int, default=None, help="unix micros, as the validator prints when the window opens")
    ap.add_argument("--until-micros", type=int, default=None, help="unix micros, as the validator prints when the window closes")
    a = ap.parse_args()
    if a.since_micros is not None or a.until_micros is not None:
        win = f", finishing between unix micros {a.since_micros} and {a.until_micros}"
    elif a.since is not None or a.until is not None:
        win = f", finishing {a.since or 0:.0f}..{a.until if a.until is not None else float('inf'):.0f} s after the LOG's first line"
    else:
        win = ""
    flushes, per, manual_job = parse(a.log, a.since, a.until, a.since_micros, a.until_micros)
    text = table(flushes, per, manual_job, a.updates, win)
    print(text)
    if a.out:
        open(a.out, "w").write(text)
