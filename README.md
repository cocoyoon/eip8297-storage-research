# eip8297-storage-research

Replays Ethereum mainnet state changes into an EIP-8297 partitioned binary tree (PBT) and prices eight ways of laying its state out on disk under the same DRAM budget. Two layouts are built and produce the same state root as the reference PBT in `crates/pbt-ref`; six are models fed by the same replay. One figure is measured on a real engine: a RocksDB was fed the replay's record stream to check the write amplification the models use. No NOMT or QMDB code is used; only their techniques are modeled.

## Layouts

Layouts that write pages:

- **page store, in place** (built): NOMT-style 4 KB pages of six tree levels each, rewritten in place; values in a separate B-tree whose interior nodes stay in memory.
- **page log** (modeled): the same pages appended to a log, found through a per-page index in DRAM, compacted when half the log is garbage.
- **page LSM** (modeled): the same pages as 4 KB values of a leveled merge tree keyed by page id.
- **copy-on-write B+tree** (modeled): records in key order in 4 KB pages; a commit copies every changed leaf and interior page, as the MDBX tables of a reth node do.

Layouts that write records:

- **log** (built): every changed record appended to a log and compacted; the index from record to position is kept on disk as a leveled merge tree with its filters in memory. Reported at two compaction settings, when half or two thirds of the log is garbage.
- **LSM of records, keyed by path** (modeled): every record is one entry of a leveled merge tree keyed by its tree position, as geth (Pebble), Nethermind and Besu (RocksDB) keep their trie; with a record cache or a block cache.
- **LSM of records, keyed by page** (modeled): the same tree with its key laid out as (page level, six-level-aligned path prefix, position in the page), so that one 4 KB block holds the records of one page.

## Layout of the code

- `crates/pbt-ref`: reference PBT (keys, hashing, tree, rewrite statistics).
- `crates/pbt-store`: the page store (`paged.rs`) and the log store (`append.rs`) as IO-counting stores, the copy-on-write B+tree model (`btree.rs`), the merge-tree write model (`lsm.rs`), an LRU miss-ratio profiler (`mrc.rs`) and the full-state shape (`shape.rs`). `kv.rs` (libmdbx) and `split.rs` are controls for synthetic runs and take no part in the mainnet replay.
- `crates/pbt-char`: the harness. `bench` replays mainnet diffs and writes a report; the modeled columns are computed at report time from the two built stores' record and page streams. `iobench` times one per-block IO mix on a real device. `lsm-validate` (feature `rocksdb`) feeds a record stream written by `bench` (`stream.rs`) to RocksDB and sets its flush and compaction bytes beside the model.
- `tools/mainnet`: fetch Xatu data, export diffs, fetch per-contract slot counts, combine key-space parts, time IO mixes, summarize the RocksDB runs.
- `results`: the reports and timings listed below.

## Reproduce

Input data is the public [Xatu](https://github.com/ethpandaops/xatu-data) dataset (ethpandaops, CC BY 4.0). No API key is needed.

```bash
cargo build --release

# 1. 50 chunks of 1,000 blocks from block 24,000,000, with execution reads (about 6 GB)
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

# 4. check the write amplification on RocksDB. Adding --mainnet-record-stream STREAM to the
#    part-0 command above writes every record the replay commits, from the starting state on
#    (about 9 GB). Then:
cargo build --release -p pbt-char --features rocksdb
# the LSM of records at the part's scale: geth-like constants, five levels, the untouched
# state as cold entries beside the originals (--ballast)
target/release/pbt-char lsm-validate --stream STREAM --db DB --label geth64b4i5 --out results \
  --blocks 50000 --measure-from-block 35000 \
  --buffer-mb 64 --l0-trigger 2 --level-base-mb 2 --ballast 4
# the log's index: 16-byte hash keys, 8-byte values, one relocation per update (0.5 at GC 2/3)
target/release/pbt-char lsm-validate --stream STREAM --db DB --label idx64b8i --out results \
  --blocks 30000 --measure-from-block 10000 \
  --buffer-mb 64 --l0-trigger 2 --level-base-mb 16 --ballast 8 --index-mode --relocations 1.0
# one table over several runs; copy each run's RocksDB LOG to LOGS/<prefix>_<label>.LOG first
uv run tools/mainnet/lsm_validate_summary.py --prefix results/lsm_validate_24000000_p0 \
  --logs LOGS --out results/lsm_validate_24000000_p0_summary.md geth64b4i5 idx64b8i

# 5. time each IO mix of a report on a device (Linux O_DIRECT or macOS F_NOCACHE)
uv run tools/mainnet/time_mix.py --report results/replay_combined.md \
  --file /path/on/device/state.dat --size-gb 400 --fill --threads 1,32 --out results/timing.md
```

One part needs 6 to 11 GB of memory and 25 to 45 minutes for 50,000 blocks. The slot-count file was fetched from the public Lab API for 2025-12-13 (`tools/mainnet/fetch_slot_counts.py`). The RocksDB runs are directories of a few GB that `lsm-validate` leaves behind; delete them when done.

## Results

| file | what |
|---|---|
| `mainnet_replay_24000000_50k_combined.md` | blocks 24,000,000–24,049,999: IO cost per block for ten store columns at 1 to 128 GB of DRAM, two device profiles, and the per-block IO mix of each column for timing |
| `mainnet_replay_23700000_50k_combined.md` | blocks 23,700,000–23,749,999, the same |
| `mainnet_replay_*_50k_p{0,1,2,3}.md`, `*_mrc.csv` | the four key-space parts behind each combined report, with the sensitivity tables (write amplification, garbage collection, read pricing) and the cache miss-ratio curves |
| `lsm_validate_24000000_p0_summary.md`, `lsm_validate_24000000_p0_*.md` | RocksDB 11.8 fed part 0's record stream: entries written per update by the engine and by the model, per run and per level |
| `iobench_24000000_50k_{i4i.xlarge,i7ie.large,macbook}.md` | per-block times of the page store's and the log's IO mix on three devices |
| `iobench_24000000_50k_macbook_v2.md` | per-block times of every column's IO mix on the laptop SSD; comparable within this run only |
| `iobench_primitives_*.txt` | single-operation timings on the same devices |
| `mainnet_replay_24040000_10k_*`, `mainnet_replay_23740000_10k_combined.md` | earlier runs of the same measured blocks with 9,000 warm-up blocks: page store and log only, index write amplification not yet scaled by the RocksDB check |

In the replay reports, `page, NOMT value store` and the two `log` columns (index in DRAM, index on disk) are the built stores. `LSM, node cache`, `LSM, block cache`, `B+tree, copy-on-write`, `log, index on disk, GC at 2/3`, `LSM, page-keyed`, `page log, GC at 1/2` and `page LSM` are models computed at report time from the same replay; Pebble, RocksDB and MDBX are not run as the store. The write amplification of the LSM of records and of the log's index is the model's value scaled by what RocksDB wrote for the same stream (1.01 for the records, 0.83 and 0.88 for the index at the two compaction settings); the page LSM's is the model alone.

Costs in the replay reports come from a counting model with assumed device profiles; `iobench` replays those counts on a device and times them. It times the IO a layout would issue, not a complete engine.

In both windows at 16 GB of DRAM for the storage engine, on the NVMe profile, the layouts that write records (the log and the LSM of records) cost 1.3 to 3.2 times less per block than those that write pages (page log, page store) and 6 to 12 times less than the copy-on-write B+tree; the log and the LSM of records are within 1.6x of each other, and neither clears the 1.3x margin the reports use to name a winner. Below 16 GB the record layouts' filters and buffers do not fit; the page log is the cheapest at 4 and 8 GB and the in-place page store at 2 GB.

## License

MIT, see `LICENSE`.
