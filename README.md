# pbt-storage-replay

Replays Ethereum mainnet state changes into an EIP-8297 partitioned binary tree (PBT) and compares two on-disk layouts for its state under the same DRAM budget:

- **page layout**: NOMT-style 4 KB pages of six tree levels each, rewritten in place, with values in a separate B-tree whose interior nodes stay in memory;
- **log layout**: every changed record appended to a log, compacted, with an index from record to position kept on disk as a leveled merge tree.

Both layouts produce the same state root as the reference PBT in `crates/pbt-ref`. No NOMT or QMDB code is used; only their techniques are modeled.

## Layout of the code

- `crates/pbt-ref`: reference PBT (keys, hashing, tree, rewrite statistics).
- `crates/pbt-store`: the two layouts as IO-counting models, an LRU miss-ratio profiler (`mrc.rs`) and the on-disk index model (`lsm.rs`).
- `crates/pbt-char`: the harness. `bench` replays mainnet diffs and writes a report; `iobench` times one per-block IO mix on a real device.
- `tools/mainnet`: fetch Xatu data, export diffs, fetch per-contract slot counts, combine key-space parts, time IO mixes.
- `results`: the reports and timings cited below.

## Reproduce

Input data is the public [Xatu](https://github.com/ethpandaops/xatu-data) dataset (ethpandaops, CC BY 4.0). No API key is needed.

```bash
cargo build --release

# 1. 50 chunks of 1,000 blocks from block 24,000,000, with execution reads
tools/mainnet/fetch.sh DATA 24000000 50 reads

# 2. export net changes per block
uv run tools/mainnet/export_diffs.py --data DATA --out DIFFS --reads

# 3. replay in four key-space parts (49,000 warm-up blocks, 1,000 measured), then combine
for q in 0 1 2 3; do
  target/release/pbt-char bench --mainnet-diffs DIFFS \
    --mainnet-slot-counts tools/mainnet/data/slot_counts_2025-12-13.csv \
    --resident-levels 2 --blocks 1000 --warmup-blocks 49000 \
    --mainnet-sample-bits 2 --mainnet-sample-part $q \
    --out results --label replay_p$q
done
uv run tools/mainnet/combine_parts.py --out results/replay_combined.md results/replay_p{0,1,2,3}.md

# 4. time each IO mix of the report on a device (Linux O_DIRECT or macOS F_NOCACHE)
uv run tools/mainnet/time_mix.py --report results/replay_combined.md \
  --file /path/on/device/state.dat --size-gb 400 --fill --threads 1,32 --out results/timing.md
```

One part needs 6 to 11 GB of memory and 25 to 65 minutes for 50,000 blocks. The slot-count file was fetched from the public Lab API for 2025-12-13 (`tools/mainnet/fetch_slot_counts.py`).

## Results

| file | what |
|---|---|
| `mainnet_replay_24000000_50k_combined_mix.md` | blocks 24,000,000–24,049,999, model costs and per-block IO mix |
| `mainnet_replay_23700000_50k_combined.md` | blocks 23,700,000–23,749,999, model costs |
| `mainnet_replay_24040000_10k_*`, `mainnet_replay_23740000_10k_combined.md` | same measured blocks with 9,000 warm-up blocks; `execsplit` separates execution-read misses |
| `iobench_24000000_50k_{i4i.xlarge,i7ie.large,macbook}.md` | per-block times of the IO mix on three devices |
| `iobench_primitives_*.txt` | single-operation timings on the same devices |

Costs in the replay reports are from a counting model with assumed device profiles; `iobench` replays those counts on a device and times them. It times the IO a layout would issue, not a complete engine.

## License

MIT, see `LICENSE`.
