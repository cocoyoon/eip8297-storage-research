# /// script
# requires-python = ">=3.10"
# ///
"""Time each IO mix of a replay report on a real device.

Reads the "IO mix per block" table of a (combined) replay report and runs
`pbt-char iobench` once per row against FILE, which should be of full-state
size on the device under test (fill it once with --fill). Prints one line
per row and writes them to OUT as a table. Linux (O_DIRECT) or macOS (F_NOCACHE); elsewhere the page
cache is not bypassed and the times are not device times.

usage: uv run tools/mainnet/time_mix.py --report R.md --file F --size-gb N
       [--fill] [--blocks 200] [--threads 1,32] --out OUT.md
"""
import argparse
import re
import subprocess

ap = argparse.ArgumentParser()
ap.add_argument("--report", required=True)
ap.add_argument("--file", required=True)
ap.add_argument("--size-gb", type=int, required=True)
ap.add_argument("--fill", action="store_true")
ap.add_argument("--blocks", type=int, default=200)
ap.add_argument("--threads", default="1,32")
ap.add_argument("--bin", default="target/release/pbt-char")
ap.add_argument("--out", required=True)
a = ap.parse_args()

rows, on = [], False
for line in open(a.report):
    if line.startswith("### IO mix per block"):
        on = True
    elif on and line.startswith("| ") and not line.startswith("| DRAM"):
        c = [x.strip() for x in line.strip().strip("|").split("|")]
        rows.append((c[0], c[1], *[float(x) for x in c[2:]]))
    elif on and line.startswith("###"):
        break
if not rows:
    raise SystemExit("no IO mix table in report")

out = ["| DRAM GB | store | threads | mean ms/block | p50 | p99 | read ms | write ms | page cache bypassed |", "|---|---|---|---|---|---|---|---|---|"]
first = True
for gb, store, r4k, r512, w4k, seqw, seqr, fs, *_ in rows:
    for t in a.threads.split(","):
        cmd = [a.bin, "iobench", "--file", a.file, "--size-gb", str(a.size_gb),
               "--reads", str(round(r4k)), "--read-size", "4096",
               "--reads2", str(round(r512)), "--read-size2", "512",
               "--page-writes", str(round(w4k)), "--seq-write-bytes", str(round(seqw)),
               "--seq-read-bytes", str(round(seqr)), "--fsyncs", str(max(1, round(fs))),
               "--blocks", str(a.blocks), "--threads", t, "--label", f"{gb}GB {store}"]
        if a.fill and first:
            cmd.append("--fill")
        first = False
        line = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout.strip()
        print(line, flush=True)
        f = dict(kv.split("=", 1) for kv in line.split("\t")[1:])
        out.append(f"| {gb} | {store} | {t} | {f['mean_ms']} | {f['p50_ms']} | {f['p99_ms']} | {f['read_ms']} | {f['write_ms']} | {f['direct']} |")
open(a.out, "w").write("\n".join(out) + "\n")
