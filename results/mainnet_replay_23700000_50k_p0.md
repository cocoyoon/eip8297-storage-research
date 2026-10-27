# PBT Phase-2: Storage Backends Under a Fixed Commitment

> Commitment is PBT for every row: each backend receives the identical
> record batch the tree emits per block, so any difference is attributable
> to physical organization alone. `kv-mdbx` is real libmdbx (reads included);
> `paged` models NOMT's bitbox as read from source (4KB pages, rootless
> depth-6 subtrie = 126 node slots, in-place hash-addressed buckets, WAL of
> node-granularity redo diffs); `append` does real sequential writes with
> counter-modeled compaction. DRAM-resident paged levels: 2.

## Per-block IO (post-genesis, 1000 blocks)

| backend | seq MB/blk | rand 4K writes/blk | page reads/blk | records/blk | lookups/blk | fsyncs/blk | DRAM MB | disk MB | compactions |
|---|---|---|---|---|---|---|---|---|---|
| paged-pos(res=2) | 0.211 | 769.8 | 757.5 | 5785.8 | 318.7 | 3.0 | 2 | 15939 | 0 |
| append-readback(res=12) | 0.791 | 0.0 | 5622.7 | 5785.8 | 318.7 | 1.0 | 431 | 0 | 0 |
| btree-cow(res=2) | 0.000 | 4697.7 | 6455.4 | 5785.8 | 318.7 | 2.0 | 1 | 603674 | 0 |

`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens
inside libmdbx and is invisible to these counters, so its seq/rand columns are
zero by construction and a modeled cost would be meaningless. It is compared
separately on real wall-clock and real on-disk size.


## Read pricing sensitivity (nvme)

A per-IO read price ignores transfer size; a page store moves 4 KB per hop
while a log store moves one sector. Both prices shown.

| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |
|---|---|---|---|
| paged-pos(res=2) | 5466.9 | 5466.9 | 3030.2 |
| append-readback(res=12) | 17467.3 | 10087.5 | 2811.3 |
| btree-cow(res=2) | 38197.7 | 38197.7 | 25821.5 |

## Modeled cost, profile `nvme-optimistic`

Units: one sequential 4KB write = 1. rand write 4x, rand read 3x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 5466.9 | 1.00x | 2 |
| append-readback(res=12) | 17467.3 | 3.20x | 431 |
| btree-cow(res=2) | 38197.7 | 6.99x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Modeled cost, profile `commodity-ssd`

Units: one sequential 4KB write = 1. rand write 16x, rand read 10x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 20130.3 | 1.00x | 2 |
| append-readback(res=12) | 57512.7 | 2.86x | 431 |
| btree-cow(res=2) | 139840.0 | 6.95x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Real-IO comparison (wall clock, on-disk)

| backend | commit wall ms/blk | disk MB | basis |
|---|---|---|---|
| paged-pos(res=2) | 10.732 | 15939 | real WAL write+fsync; page IO counter-modeled |
| append-readback(res=12) | 3.733 | 0 | counter-modeled |
| btree-cow(res=2) | 10.376 | 603674 | counter-modeled |

## Backend detail

- paged-pos(res=2): 4080505 stored pages, mean occupancy 6.8/126 slots (5.4%), WAL 12286 MB
  - elision: 16013661 page writes avoided, 0 read fallbacks
  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: [3523719, 452683, 62222, 10014, 5366, 26501]
- append-readback(res=12): live 2247 MB, amortized compaction reclaim 20925 MB, live records 29337757, live branches 14678846, idx_branches 14678846, idx_leaves 14658911, write-path RMW reads 277450236 (sibling-dirty 19629924), uncharged displaced-leaf splits 24687, dram bytes 452077331
- btree-cow MODELED: 33 entries per 4 KB leaf page (96 B mean entry at fill 0.8), fanout 60, height 6, pages per level [151964987, 2532750, 42213, 704, 12, 1] (633.0 GB)
- btree-cow: resident leaf pages 397 (1.6 MB); records created 31091625, rewritten 301590257, deleted 1774732, leaf changes 26990316
- btree-cow, measured window: leaf pages copied 2442287, interior pages copied 2249224, freelist pages 5187; key walks 342301, prefix hits 5456482, fallback walks 15552, memo hits 305705

## Mainnet replay under a full node's DRAM

> Nature: MAINNET write stream, replayed on a tree whose untouched state is
> MODELED, costed with the COUNTER model. Which keys each block writes, and
> whether each write creates, rewrites or deletes, is real. The rest of the
> state is not loaded: beside every written path, wherever the full state has
> keys on the other side of a branch, a stand-in is inserted, so paths have
> the depth they would have on mainnet. Where the full state has keys is
> taken from its size and EIP-8297's hash placement, not from the keys
> themselves. The page side is NOMT as built; the log side with its index on
> disk is an ASSUMED engine. Two existing-engine baselines, an LSM of records
> and a copy-on-write B+tree, are MODELED from the same replay. Execution reads of balances, nonces and storage are replayed from the
> same dataset; code reads and delegations are left out.

- workload: mainnet blocks 23700000 to 23749999 replayed from state diffs (Xatu, CC BY 4.0); untouched state modeled as 334563311 accounts, 1438571749 storage slots in 25000000 contracts
- SAMPLED: only accounts and contracts whose address hash starts with the 2 bits of part 0 are replayed, one key-space subtree in 4. Every figure below is scaled to the whole: per-block counts by 4, cache sizes and DRAM by 4, the full state and the index buffer taken at 1/4 for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.
- window: 1000 measured blocks after 49000 warm-up blocks; 50000 blocks available from 23700000
- full state stood in for: 334563311 accounts, 1438571749 storage slots, 399723913 code chunks = 626855571 leaves, 1253711142 records. Storage contracts: 25000000 (ASSUMED). An untouched contract is taken to hold 57 slots, the mean; real sizes are heavy-tailed.
- replay built from: 3563648 account changes (343965 creations) and 9628324 slot changes; 1536684 keys in the starting state; stand-ins: 5803955 beside account paths, 153873 beside contract buckets, 5959866 inside contracts' storage. 1831 of 17209 contracts written in the storage zone have a fetched slot count, covering 97.8% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes 1694654 distinct slots there in 1564382 stems (1.08 per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.
- path length, account zone: 463.1 leaves written per block, 32.6 branch records above each on average, 40 at most
- path length, storage zone: 696.6 leaves written per block, 46.6 branch records above each on average, 54 at most
- execution reads, per block: 978.6 of existing keys and 296.2 of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.
- measured, log store, per block: 22952.4 records written, 1.59 MB appended by the commit; with no cache 22490.7 reads, of which 978.6 are execution reads, 1063.2 fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; 45898.2 index updates, 67.6 deleted branches to mark dead, 98.7 displaced leaves
- measured, page store, per block: 3079.2 merkle pages written; with no cache 5408.7 reads, of which 3030.2 are merkle pages and the rest value-store leaf pages (execution reads included); 1103.8 value-store leaf pages and 1009.3 bottom branch nodes rewritten, 20.28 MB held by one commit
- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at 88 B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.
- fixed DRAM at full-state size: log store with its index in DRAM 71.9 GB (15.4 B per record); with its index on disk 7.6 GB or 20.4 GB (1.5 or 4.25 B per record, plus a 256 MB write buffer and 298 MB of page pointers, all ASSUMED); page store 1.02 GB (0.80 GB of resident bottom branch nodes over 74462908 value-store leaf pages, 0.14 GB of bucket map, ASSUMED, and one commit's pages).
- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, 5.92 GB of records for the log store and 44.34 GB of pages for the page store. A replay of 50000 blocks cannot tell further; a longer one narrows the range.
- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over 75 GB of 16-byte entries, a 256 MB write buffer, levels growing by 10x down to one that holds every entry. How much each level absorbs is MEASURED from the 252457207 index updates of the replay (20197 per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The 1.0 relocations per update are taken never to be absorbed. Write amplification 12.7 per index update (204 B written and read): the model gives 15.3, and a real RocksDB fed this stream as the index sees it wrote 0.83 of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); 15 for the assumed engine before. That costs 4109 per block on `nvme-optimistic` and 4109 on `commodity-ssd`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 4.194e6 | 173 | yes | 0.604 | 3.02 |
  | L1 | 1.254e7 | 542 | yes | 0.577 | 6.35 |
  | L2 | 1.254e8 | 5792 | yes | 0.540 | 5.94 |
  | L3 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a 256 MB write buffer and levels growing by 10x down to one holding all 5014844568 records. An entry is 81 B, the mean record of this replay's write stream (72.6 B, its 4 B log header standing in for the varint lengths) plus 8 B of sequence number and type; prefix compression is not modeled. Updates are the 23143.2 records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification 4.6 per record update (375 B written and read back by compaction, plus one WAL copy of each update), the model's 4.6 scaled by 1.01, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against 12.7 for the index entries above; that costs 3812 per block on `nvme-optimistic` and 3812 on `commodity-ssd`. Fixed DRAM 8.89 GB: 1.85 B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at 97 B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 8.323e5 | 198 | yes | 0.208 | 0.73 |
  | L1 | 1.254e6 | 343 | yes | 0.181 | 1.99 |
  | L2 | 1.254e7 | 7244 | no: longest window's share used | 0.086 | 0.94 |
  | L3 | 1.254e8 | 72444 | no: longest window's share used | 0.086 | 0.94 |
  | L4 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, 33 per leaf page (96 B mean entry at fill 0.8, ASSUMED), 60 children per interior page (ASSUMED), height 6 over 151964987 leaf pages (633 GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM (0.002 GB). The page map is static: no page splits or merges over the replay.
- measured, B+tree, per block: 18790.8 pages rewritten copy-on-write; with no cache 10450.8 leaf-page reads and 15370.7 interior-page reads, execution reads included; 9769.1 leaf pages and 8996.9 interior pages copied, 73.40 MB held by one commit, 2 fsyncs
- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM (2.22 GB: 16 B per page over 148925816 pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: 4183.0 pages appended per block, each superseding one; space up to 2x the live pages.
- page LSM, MODELED from the page store's run: the same pages as 4112 B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the 45632003 page writes of the replay (3651 per block, warm-up included). page write amplification 8.3 per page update (33 KB written and read back by compaction per page written, plus one WAL copy).

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 1.632e4 | 9 | yes | 0.477 | 2.04 |
  | L1 | 3.723e4 | 29 | yes | 0.350 | 3.85 |
  | L2 | 3.723e5 | 687 | yes | 0.148 | 1.63 |
  | L3 | 3.723e6 | 14871 | no: longest window's share used | 0.069 | 0.75 |
  | L4 | 3.723e7 | - | - | 0.000 | 0.00 |

- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations 11377.5 per block at g = 2/3 (22755.0 at g = 1/2), index write amplification 9.8 (12.7 at g = 1/2), compaction reading 2.36 MB and rewriting 0.79 MB per block (3.15 and 1.57 at g = 1/2); space up to 3x the live records instead of 2x.
- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves (98.7 per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the 1114.0 elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.
- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified.
- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name.
- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by 1.3x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.

### Reads (write path and execution) per block against cache size, exact range only

Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.

| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |
|---|---|---|---|---|---|---|
| 1/64 | 1034968 | 0.093 | 5402.5 | 180916 | 0.693 | 1251.9 |
| 1/32 | 2069936 | 0.185 | 4000.6 | 361836 | 1.386 | 928.7 |
| 1/16 | 4139876 | 0.370 | 3061.9 | 723672 | 2.771 | 695.4 |
| 1/8 | 8279752 | 0.740 | 2325.5 | 1447344 | 5.543 | 528.3 |
| 1/4 | 16559508 | 1.481 | 1729.8 | 2894688 | 11.085 | 398.6 |
| 1/2 | 33119020 | 2.961 | 1175.8 | 5789380 | 22.171 | 244.5 |
| 1/1 | 66238040 | 5.922 | 811.0 | 11578764 | 44.342 | 128.1 |

With no cache at all: 22490.7 reads per block for the log store, 5408.7 for the page store. Of the log store's, 807.3 are of records untouched since the replay began; of the page store's, 127.3.

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 93497 | - | - | - | 71147 | page LSM (1.31x to 1.31x over B+tree) |
| 2 | 24349 | - | - | - | - | 85934 | - | - | - | 69493 | page (2.85x to 2.85x over page LSM) |
| 4 | 23134 | - | - | - | - | 81714 | - | - | 17646 | 68703 | page log (1.31x to 1.31x over page) |
| 8 | 22554 | - | 23515 | - | - | 79264 | 21431 | - | 16667 | 68203 | too close (page log by 1.29x to 1.29x over log) |
| 13.48 | 22225 | - | 11245 | 7341 | 8413 | 78009 | 9161 | 6300 | 16291 | 67887 | LSM (1.45x to 1.45x over log) |
| 16 | 22075 | - | 6379-11223 | 4583-7005 | 7673 | 77683 | 4295-9139 | 6026 | 16132 | 67742 | undecided (LSM by up to 1.99x, log by up to 1.40x) |
| 26.32 | 21765 | - | 6379-11223 | 4583-7005 | 6434 | 76791 | 4295-9139 | 5466 | 15790 | 67445 | undecided (LSM by up to 1.99x, log by up to 1.27x) |
| 32 | 21646 | - | 6379-11223 | 4583-7005 | 6028 | 76510 | 4295-9139 | 5295 | 15664 | 67328 | undecided (LSM by up to 1.99x, log by up to 1.23x) |
| 64 | 21102-21484 | - | 6379-11223 | 4583-7005 | 5205 | 75804 | 4295-9139 | 4583-4965 | 15099-15481 | 66793-67175 | undecided (LSM by up to 1.99x, log by up to 1.16x) |
| 128 | 21102-21484 | 1771-4193 | 6379-11223 | 4583-7005 | 4583-5108 | 75203-75738 | 4295-9139 | 4583-4965 | 15099-15481 | 66793-67175 | too close (log by 1.09x to 2.80x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 93497 | - | - | - | 71147 | page LSM (1.31x to 1.31x over B+tree) |
| 2 | 24349 | - | - | - | - | 85934 | - | - | - | 69493 | page (2.85x to 2.85x over page LSM) |
| 4 | 23134 | - | - | - | - | 81714 | - | - | 17646 | 68703 | page log (1.31x to 1.31x over page) |
| 8 | 22554 | - | 19767 | - | - | 79264 | 17683 | - | 16667 | 68203 | too close (page log by 1.06x to 1.06x over log) |
| 13.48 | 22225 | - | 10180 | 7341 | 8413 | 78009 | 8096 | 6300 | 16291 | 67887 | too close (LSM by 1.29x to 1.29x over log) |
| 16 | 22075 | - | 6379-10163 | 4583-7005 | 7673 | 77683 | 4295-8079 | 6026 | 16132 | 67742 | undecided (LSM by up to 1.76x, log by up to 1.40x) |
| 26.32 | 21765 | - | 6379-10163 | 4583-7005 | 6434 | 76791 | 4295-8079 | 5466 | 15790 | 67445 | undecided (LSM by up to 1.76x, log by up to 1.27x) |
| 32 | 21646 | - | 6379-10163 | 4583-7005 | 6028 | 76510 | 4295-8079 | 5295 | 15664 | 67328 | undecided (LSM by up to 1.76x, log by up to 1.23x) |
| 64 | 21102-21484 | - | 6379-10163 | 4583-7005 | 5205 | 75804 | 4295-8079 | 4583-4965 | 15099-15481 | 66793-67175 | undecided (LSM by up to 1.76x, log by up to 1.16x) |
| 128 | 21102-21484 | 1771-3133 | 6379-10163 | 4583-7005 | 4583-5108 | 75203-75738 | 4295-8079 | 4583-4965 | 15099-15481 | 66793-67175 | log (1.46x to 2.80x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 361751 | - | - | - | 81347 | page LSM (4.45x to 4.45x over B+tree) |
| 2 | 94483 | - | - | - | - | 336543 | - | - | - | 75831 | too close (page LSM by 1.25x to 1.25x over page) |
| 4 | 90433 | - | - | - | - | 322474 | - | - | 23670 | 73200 | page log (3.09x to 3.09x over page LSM) |
| 8 | 88498 | - | 65395 | - | - | 314310 | 63311 | - | 20405 | 71533 | page log (3.10x to 3.10x over log) |
| 13.48 | 87401 | - | 24494 | 14506 | 18079 | 310127 | 22410 | 11037 | 19151 | 70480 | LSM (1.74x to 1.74x over page log) |
| 16 | 86902 | - | 8275-24421 | 5315-13388 | 15612 | 309040 | 6191-22337 | 10122 | 18624 | 69996 | undecided (LSM by up to 4.20x, log by up to 1.63x) |
| 26.32 | 85869 | - | 8275-24421 | 5315-13388 | 11482 | 306067 | 6191-22337 | 8258 | 17484 | 69005 | undecided (LSM by up to 4.20x, log by up to 1.33x) |
| 32 | 85471 | - | 8275-24421 | 5315-13388 | 10129 | 305129 | 6191-22337 | 7686 | 17063 | 68614 | undecided (LSM by up to 4.20x, log by up to 1.24x) |
| 64 | 83657-84930 | - | 8275-24421 | 5315-13388 | 7388 | 302775 | 6191-22337 | 5315-6588 | 15179-16452 | 66833-68107 | undecided (LSM by up to 4.20x, log by up to 1.06x) |
| 128 | 83657-84930 | 2502-10575 | 8275-24421 | 5315-13388 | 5315-7065 | 300773-302555 | 6191-22337 | 5315-6588 | 15179-16452 | 66833-68107 | undecided (LSM by up to 1.99x, log by up to 2.63x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 361751 | - | - | - | 81347 | page LSM (4.45x to 4.45x over B+tree) |
| 2 | 94483 | - | - | - | - | 336543 | - | - | - | 75831 | too close (page LSM by 1.25x to 1.25x over page) |
| 4 | 90433 | - | - | - | - | 322474 | - | - | 23670 | 73200 | page log (3.09x to 3.09x over page LSM) |
| 8 | 88498 | - | 52900 | - | - | 314310 | 50816 | - | 20405 | 71533 | page log (2.49x to 2.49x over log) |
| 13.48 | 87401 | - | 20946 | 14506 | 18079 | 310127 | 18862 | 11037 | 19151 | 70480 | LSM (1.71x to 1.71x over log) |
| 16 | 86902 | - | 8275-20889 | 5315-13388 | 15612 | 309040 | 6191-18805 | 10122 | 18624 | 69996 | undecided (LSM by up to 3.54x, log by up to 1.63x) |
| 26.32 | 85869 | - | 8275-20889 | 5315-13388 | 11482 | 306067 | 6191-18805 | 8258 | 17484 | 69005 | undecided (LSM by up to 3.54x, log by up to 1.33x) |
| 32 | 85471 | - | 8275-20889 | 5315-13388 | 10129 | 305129 | 6191-18805 | 7686 | 17063 | 68614 | undecided (LSM by up to 3.54x, log by up to 1.24x) |
| 64 | 83657-84930 | - | 8275-20889 | 5315-13388 | 7388 | 302775 | 6191-18805 | 5315-6588 | 15179-16452 | 66833-68107 | undecided (LSM by up to 3.54x, log by up to 1.06x) |
| 128 | 83657-84930 | 2502-7043 | 8275-20889 | 5315-13388 | 5315-7065 | 300773-302555 | 6191-18805 | 5315-6588 | 15179-16452 | 66833-68107 | undecided (LSM by up to 1.33x, log by up to 2.63x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 678 | 0 | 5196 | 886981 | 0 | 5.00 | 81 |
| 4 | B+tree, copy-on-write | 2170 | 0 | 18791 | 0 | 0 | 2.00 | 196 |
| 4 | page log, GC at 1/2 | 849 | 0 | 0 | 34267070 | 34267070 | 2.00 | 114 |
| 4 | page LSM | 637 | 0 | 0 | 159591189 | 142390726 | 1.00 | 75 |
| 8 | page | 484 | 0 | 5196 | 886981 | 0 | 5.00 | 50 |
| 8 | log, index on disk (1.5 B) | 3121 | 2856 | 0 | 12667547 | 12651296 | 1.00 | 79 |
| 8 | B+tree, copy-on-write | 1354 | 0 | 18791 | 0 | 0 | 2.00 | 105 |
| 8 | log, index on disk, GC at 2/3 | 3121 | 2856 | 0 | 7925248 | 7908997 | 1.00 | 79 |
| 8 | page log, GC at 1/2 | 523 | 0 | 0 | 34267070 | 34267070 | 2.00 | 56 |
| 8 | page LSM | 470 | 0 | 0 | 159591189 | 142390726 | 1.00 | 48 |
| 16 | page | 324 | 0 | 5196 | 886981 | 0 | 5.00 | 31 |
| 16 | log, index on disk (1.5 B) | 1072 | 807 | 0 | 12667547 | 12651296 | 1.00 | 21 |
| 16 | LSM, node cache | 906 | 0 | 0 | 10539433 | 8673351 | 1.00 | 21 |
| 16 | LSM, block cache | 1128 | 0 | 0 | 10539433 | 8673351 | 1.00 | 71 |
| 16 | B+tree, copy-on-write | 827 | 0 | 18791 | 0 | 0 | 2.00 | 58 |
| 16 | log, index on disk, GC at 2/3 | 1072 | 807 | 0 | 7925248 | 7908997 | 1.00 | 21 |
| 16 | LSM, page-keyed | 580 | 0 | 0 | 10539433 | 8673351 | 1.00 | 50 |
| 16 | page log, GC at 1/2 | 345 | 0 | 0 | 34267070 | 34267070 | 2.00 | 33 |
| 16 | page LSM | 316 | 0 | 0 | 159591189 | 142390726 | 1.00 | 31 |
| 32 | page | 181 | 0 | 5196 | 886981 | 0 | 5.00 | 16 |
| 32 | log, index on disk (1.5 B) | 1072 | 807 | 0 | 12667547 | 12651296 | 1.00 | 21 |
| 32 | LSM, node cache | 906 | 0 | 0 | 10539433 | 8673351 | 1.00 | 21 |
| 32 | LSM, block cache | 580 | 0 | 0 | 10539433 | 8673351 | 1.00 | 31 |
| 32 | B+tree, copy-on-write | 436 | 0 | 18791 | 0 | 0 | 2.00 | 30 |
| 32 | log, index on disk, GC at 2/3 | 1072 | 807 | 0 | 7925248 | 7908997 | 1.00 | 21 |
| 32 | LSM, page-keyed | 336 | 0 | 0 | 10539433 | 8673351 | 1.00 | 22 |
| 32 | page log, GC at 1/2 | 188 | 0 | 0 | 34267070 | 34267070 | 2.00 | 16 |
| 32 | page LSM | 178 | 0 | 0 | 159591189 | 142390726 | 1.00 | 15 |

### Sensitivity to the index engine's write amplification, 16 GB

The log store with its index on disk (1.5 B per record in DRAM), IO cost per block. 12.7 is the amplification the tables above use, the model's 15.3 scaled by the RocksDB measurement.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 12.7 | 6379-11223 | 6379-10163 | 8275-24421 | 8275-20889 |
| 15.3 | 7211-12054 | 7211-10995 | 9107-25253 | 9107-21721 |
| 5.0 | 3884-8728 | 3884-7668 | 5780-21926 | 5780-18394 |
| 30.0 | 11952-16796 | 11952-15736 | 13848-29994 | 13848-26462 |
| page store, for comparison | 22075 | 22075 | 86902 | 86902 |

### Sensitivity to the record engine's write amplification, 16 GB

The LSM of records, IO cost per block. 4.6 is the amplification the tables above use, the model's 4.6 scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 4.6, node cache | 4583-7005 | 4583-7005 | 5315-13388 | 5315-13388 |
| 4.6, block cache | 7673 | 7673 | 15612 | 15612 |
| 4.6, page-keyed | 6026 | 6026 | 10122 | 10122 |
| 5.0, node cache | 4872-7294 | 4872-7294 | 5603-13676 | 5603-13676 |
| 5.0, block cache | 7961 | 7961 | 15901 | 15901 |
| 5.0, page-keyed | 6314 | 6314 | 10411 | 10411 |
| 15.0, node cache | 13073-15495 | 13073-15495 | 13804-21877 | 13804-21877 |
| 15.0, block cache | 16162 | 16162 | 24101 | 24101 |
| 15.0, page-keyed | 14515 | 14515 | 18612 | 18612 |
| 30.0, node cache | 25374-27795 | 25374-27795 | 26105-34178 | 26105-34178 |
| 30.0, block cache | 28463 | 28463 | 36402 | 36402 |
| 30.0, page-keyed | 26816 | 26816 | 30912 | 30912 |

### Sensitivity to garbage collection, 16 GB

The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.

| g | store | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes | space |
|---|---|---|---|---|---|---|
| 0.50 | log, index on disk (index amplification 12.7) | 6379-11223 | 6379-10163 | 8275-24421 | 8275-20889 | 2.0x |
| 0.50 | page log | 16132 | 16132 | 18624 | 18624 | 2.0x |
| 0.67 | log, index on disk (index amplification 9.8) | 4295-9139 | 4295-8079 | 6191-22337 | 6191-18805 | 3.0x |
| 0.67 | page log | 12368 | 12368 | 14859 | 14859 | 3.0x |
| 0.80 | log, index on disk (index amplification 6.9) | 3120-7964 | 3120-6905 | 5016-21163 | 5016-17631 | 5.0x |
| 0.80 | page log | 10485 | 10485 | 12977 | 12977 | 5.0x |

