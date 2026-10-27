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
| paged-pos(res=2) | 0.199 | 717.0 | 704.9 | 5509.0 | 374.3 | 3.0 | 2 | 19313 | 0 |
| append-readback(res=12) | 0.751 | 0.0 | 5320.0 | 5509.0 | 374.3 | 1.0 | 534 | 0 | 0 |
| btree-cow(res=2) | 0.000 | 4514.5 | 6614.2 | 5509.0 | 374.3 | 2.0 | 1 | 603674 | 0 |

`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens
inside libmdbx and is invisible to these counters, so its seq/rand columns are
zero by construction and a modeled cost would be meaningless. It is compared
separately on real wall-clock and real on-disk size.


## Read pricing sensitivity (nvme)

A per-IO read price ignores transfer size; a page store moves 4 KB per hop
while a log store moves one sector. Both prices shown.

| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |
|---|---|---|---|
| paged-pos(res=2) | 5094.7 | 5094.7 | 2819.7 |
| append-readback(res=12) | 16592.1 | 9609.6 | 2660.0 |
| btree-cow(res=2) | 37941.5 | 37941.5 | 26456.9 |

## Modeled cost, profile `nvme-optimistic`

Units: one sequential 4KB write = 1. rand write 4x, rand read 3x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 5094.7 | 1.00x | 2 |
| append-readback(res=12) | 16592.1 | 3.26x | 534 |
| btree-cow(res=2) | 37941.5 | 7.45x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Modeled cost, profile `commodity-ssd`

Units: one sequential 4KB write = 1. rand write 16x, rand read 10x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 18756.0 | 1.00x | 2 |
| append-readback(res=12) | 54674.4 | 2.92x | 534 |
| btree-cow(res=2) | 138497.6 | 7.38x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Real-IO comparison (wall clock, on-disk)

| backend | commit wall ms/blk | disk MB | basis |
|---|---|---|---|
| paged-pos(res=2) | 11.035 | 19313 | real WAL write+fsync; page IO counter-modeled |
| append-readback(res=12) | 3.642 | 0 | counter-modeled |
| btree-cow(res=2) | 9.953 | 603674 | counter-modeled |

## Backend detail

- paged-pos(res=2): 4944323 stored pages, mean occupancy 7.0/126 slots (5.6%), WAL 14062 MB
  - elision: 19320953 page writes avoided, 0 read fallbacks
  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: [4271292, 468907, 152435, 15544, 6471, 29674]
- append-readback(res=12): live 2817 MB, amortized compaction reclaim 23924 MB, live records 36405936, live branches 18210448, idx_branches 18210448, idx_leaves 18195488, write-path RMW reads 319069533 (sibling-dirty 24051772), uncharged displaced-leaf splits 25765, dram bytes 560927401
- btree-cow MODELED: 33 entries per 4 KB leaf page (96 B mean entry at fill 0.8), fanout 60, height 6, pages per level [151964987, 2532750, 42213, 704, 12, 1] (633.0 GB)
- btree-cow: resident leaf pages 396 (1.6 MB); records created 38865667, rewritten 344352163, deleted 2513946, leaf changes 32008749
- btree-cow, measured window: leaf pages copied 2320814, interior pages copied 2187695, freelist pages 5013; key walks 363217, prefix hits 5176074, fallback walks 23420, memo hits 343969

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

- workload: mainnet blocks 24000000 to 24049999 replayed from state diffs (Xatu, CC BY 4.0); untouched state modeled as 334563311 accounts, 1438571749 storage slots in 25000000 contracts
- SAMPLED: only accounts and contracts whose address hash starts with the 2 bits of part 2 are replayed, one key-space subtree in 4. Every figure below is scaled to the whole: per-block counts by 4, cache sizes and DRAM by 4, the full state and the index buffer taken at 1/4 for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.
- window: 1000 measured blocks after 49000 warm-up blocks; 50000 blocks available from 24000000
- full state stood in for: 334563311 accounts, 1438571749 storage slots, 399723913 code chunks = 626855571 leaves, 1253711142 records. Storage contracts: 25000000 (ASSUMED). An untouched contract is taken to hold 57 slots, the mean; real sizes are heavy-tailed.
- replay built from: 4021002 account changes (486463 creations) and 11399117 slot changes; 1423238 keys in the starting state; stand-ins: 6329318 beside account paths, 175764 beside contract buckets, 8173846 inside contracts' storage. 1865 of 20105 contracts written in the storage zone have a fetched slot count, covering 97.2% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes 2379572 distinct slots there in 2128261 stems (1.12 per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.
- path length, account zone: 524.9 leaves written per block, 32.3 branch records above each on average, 41 at most
- path length, storage zone: 623.5 leaves written per block, 45.7 branch records above each on average, 57 at most
- execution reads, per block: 981.1 of existing keys and 516.0 of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.
- measured, log store, per block: 21856.8 records written, 1.52 MB appended by the commit; with no cache 21279.9 reads, of which 981.1 are execution reads, 971.8 fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; 43537.7 index updates, 63.2 deleted branches to mark dead, 103.1 displaced leaves
- measured, page store, per block: 2867.9 merkle pages written; with no cache 5352.1 reads, of which 2819.7 are merkle pages and the rest value-store leaf pages (execution reads included); 1035.2 value-store leaf pages and 966.8 bottom branch nodes rewritten, 19.02 MB held by one commit
- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at 88 B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.
- fixed DRAM at full-state size: log store with its index in DRAM 71.9 GB (15.4 B per record); with its index on disk 7.6 GB or 20.4 GB (1.5 or 4.25 B per record, plus a 256 MB write buffer and 298 MB of page pointers, all ASSUMED); page store 1.02 GB (0.80 GB of resident bottom branch nodes over 74462908 value-store leaf pages, 0.14 GB of bucket map, ASSUMED, and one commit's pages).
- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, 7.50 GB of records for the log store and 50.14 GB of pages for the page store. A replay of 50000 blocks cannot tell further; a longer one narrows the range.
- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over 75 GB of 16-byte entries, a 256 MB write buffer, levels growing by 10x down to one that holds every entry. How much each level absorbs is MEASURED from the 284318900 index updates of the replay (22746 per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The 1.0 relocations per update are taken never to be absorbed. Write amplification 12.9 per index update (206 B written and read): the model gives 15.5, and a real RocksDB fed this stream as the index sees it wrote 0.83 of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); 15 for the assumed engine before. That costs 3939 per block on `nvme-optimistic` and 3939 on `commodity-ssd`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 4.194e6 | 153 | yes | 0.612 | 3.05 |
  | L1 | 1.254e7 | 478 | yes | 0.584 | 6.42 |
  | L2 | 1.254e8 | 5127 | yes | 0.544 | 5.98 |
  | L3 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a 256 MB write buffer and levels growing by 10x down to one holding all 5014844568 records. An entry is 81 B, the mean record of this replay's write stream (72.7 B, its 4 B log header standing in for the varint lengths) plus 8 B of sequence number and type; prefix compression is not modeled. Updates are the 22035.9 records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification 5.0 per record update (405 B written and read back by compaction, plus one WAL copy of each update), the model's 5.0 scaled by 1.01, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against 12.9 for the index entries above; that costs 3923 per block on `nvme-optimistic` and 3923 on `commodity-ssd`. Fixed DRAM 8.89 GB: 1.85 B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at 97 B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 8.314e5 | 159 | yes | 0.230 | 0.81 |
  | L1 | 1.254e6 | 272 | yes | 0.202 | 2.23 |
  | L2 | 1.254e7 | 5713 | yes | 0.096 | 1.06 |
  | L3 | 1.254e8 | 69481 | no: longest window's share used | 0.079 | 0.87 |
  | L4 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, 33 per leaf page (96 B mean entry at fill 0.8, ASSUMED), 60 children per interior page (ASSUMED), height 6 over 151964987 leaf pages (633 GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM (0.002 GB). The page map is static: no page splits or merges over the replay.
- measured, B+tree, per block: 18058.1 pages rewritten copy-on-write; with no cache 10220.6 leaf-page reads and 16236.3 interior-page reads, execution reads included; 9283.3 leaf pages and 8750.8 interior pages copied, 70.54 MB held by one commit, 2 fsyncs
- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM (2.22 GB: 16 B per page over 148925816 pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: 3903.1 pages appended per block, each superseding one; space up to 2x the live pages.
- page LSM, MODELED from the page store's run: the same pages as 4112 B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the 50577746 page writes of the replay (4046 per block, warm-up included). page write amplification 8.6 per page update (35 KB written and read back by compaction per page written, plus one WAL copy).

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 1.632e4 | 8 | yes | 0.479 | 2.05 |
  | L1 | 3.723e4 | 26 | yes | 0.359 | 3.95 |
  | L2 | 3.723e5 | 548 | yes | 0.168 | 1.85 |
  | L3 | 3.723e6 | 12791 | no: longest window's share used | 0.072 | 0.79 |
  | L4 | 3.723e7 | - | - | 0.000 | 0.00 |

- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations 10750.9 per block at g = 2/3 (21501.8 at g = 1/2), index write amplification 9.9 (12.9 at g = 1/2), compaction reading 2.23 MB and rewriting 0.74 MB per block (2.98 and 1.49 at g = 1/2); space up to 3x the live records instead of 2x.
- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves (103.1 per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the 1206.5 elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.
- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified.
- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name.
- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by 1.3x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.

### Reads (write path and execution) per block against cache size, exact range only

Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.

| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |
|---|---|---|---|---|---|---|
| 1/64 | 1310248 | 0.117 | 4667.0 | 204568 | 0.783 | 1214.8 |
| 1/32 | 2620500 | 0.234 | 3648.1 | 409136 | 1.567 | 953.7 |
| 1/16 | 5241004 | 0.469 | 2811.0 | 818276 | 3.134 | 761.2 |
| 1/8 | 10482012 | 0.937 | 2175.8 | 1636556 | 6.267 | 585.8 |
| 1/4 | 20964024 | 1.874 | 1676.1 | 3273116 | 12.535 | 439.2 |
| 1/2 | 41928052 | 3.749 | 1214.7 | 6546236 | 25.069 | 279.5 |
| 1/1 | 83856104 | 7.497 | 853.2 | 13092476 | 50.139 | 142.7 |

With no cache at all: 21279.9 reads per block for the log store, 5352.1 for the page store. Of the log store's, 849.8 are of records untouched since the replay began; of the page store's, 141.8.

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 91457 | - | - | - | 69240 | page LSM (1.32x to 1.32x over B+tree) |
| 2 | 23166 | - | - | - | - | 83220 | - | - | - | 67772 | page (2.93x to 2.93x over page LSM) |
| 4 | 22119 | - | - | - | - | 79503 | - | - | 16872 | 67014 | page log (1.31x to 1.31x over page) |
| 8 | 21474 | - | 23319 | - | - | 77106 | 21329 | - | 15912 | 66479 | page log (1.34x to 1.34x over log) |
| 15.05 | 21048 | - | 11264 | 7519 | 8557 | 75483 | 9275 | 6456 | 15398 | 66075 | LSM (1.44x to 1.44x over log) |
| 16 | 21007 | - | 6145-11244 | 7302 | 8297 | 75358 | 4156-9255 | 6350 | 15355 | 66032 | undecided (LSM by up to 1.46x, log by up to 1.53x) |
| 27.90 | 20595 | - | 6145-11244 | 4686-7236 | 6910 | 74371 | 4156-9255 | 5698 | 14918 | 65630 | undecided (LSM by up to 1.97x, log by up to 1.37x) |
| 32 | 20498 | - | 6145-11244 | 4686-7236 | 6675 | 74130 | 4156-9255 | 5569 | 14816 | 65538 | undecided (LSM by up to 1.97x, log by up to 1.34x) |
| 64 | 19799-20225 | - | 6145-11244 | 4686-7236 | 5663 | 73202 | 4156-9255 | 4686-5112 | 14091-14517 | 64848-65273 | undecided (LSM by up to 1.97x, log by up to 1.23x) |
| 128 | 19799-20225 | 1708-4257 | 6145-11244 | 4686-7236 | 4686-5344 | 72272-72941 | 4156-9255 | 4686-5112 | 14091-14517 | 64848-65273 | too close (log by 1.10x to 2.99x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 91457 | - | - | - | 69240 | page LSM (1.32x to 1.32x over B+tree) |
| 2 | 23166 | - | - | - | - | 83220 | - | - | - | 67772 | page (2.93x to 2.93x over page LSM) |
| 4 | 22119 | - | - | - | - | 79503 | - | - | 16872 | 67014 | page log (1.31x to 1.31x over page) |
| 8 | 21474 | - | 19562 | - | - | 77106 | 17573 | - | 15912 | 66479 | too close (page log by 1.10x to 1.10x over log) |
| 15.05 | 21048 | - | 10145 | 7519 | 8557 | 75483 | 8155 | 6456 | 15398 | 66075 | too close (LSM by 1.26x to 1.26x over log) |
| 16 | 21007 | - | 6145-10129 | 7302 | 8297 | 75358 | 4156-8139 | 6350 | 15355 | 66032 | undecided (LSM by up to 1.28x, log by up to 1.53x) |
| 27.90 | 20595 | - | 6145-10129 | 4686-7236 | 6910 | 74371 | 4156-8139 | 5698 | 14918 | 65630 | undecided (LSM by up to 1.74x, log by up to 1.37x) |
| 32 | 20498 | - | 6145-10129 | 4686-7236 | 6675 | 74130 | 4156-8139 | 5569 | 14816 | 65538 | undecided (LSM by up to 1.74x, log by up to 1.34x) |
| 64 | 19799-20225 | - | 6145-10129 | 4686-7236 | 5663 | 73202 | 4156-8139 | 4686-5112 | 14091-14517 | 64848-65273 | undecided (LSM by up to 1.74x, log by up to 1.23x) |
| 128 | 19799-20225 | 1708-3142 | 6145-10129 | 4686-7236 | 4686-5344 | 72272-72941 | 4156-8139 | 4686-5112 | 14091-14517 | 64848-65273 | log (1.49x to 2.99x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 352998 | - | - | - | 79528 | page LSM (4.44x to 4.44x over B+tree) |
| 2 | 89707 | - | - | - | - | 325542 | - | - | - | 74636 | too close (page LSM by 1.20x to 1.20x over page) |
| 4 | 86219 | - | - | - | - | 313150 | - | - | 23441 | 72110 | page log (3.08x to 3.08x over page LSM) |
| 8 | 84068 | - | 65315 | - | - | 305160 | 63325 | - | 20241 | 70325 | page log (3.13x to 3.13x over log) |
| 15.05 | 82650 | - | 25134 | 14889 | 18348 | 299753 | 23144 | 11346 | 18527 | 68977 | LSM (1.63x to 1.63x over page log) |
| 16 | 82511 | - | 8070-25067 | 14166 | 17484 | 299334 | 6081-23078 | 10995 | 18385 | 68835 | undecided (LSM by up to 2.10x, log by up to 1.81x) |
| 27.90 | 81140 | - | 8070-25067 | 5448-13946 | 12861 | 296044 | 6081-23078 | 8820 | 16929 | 67495 | undecided (LSM by up to 4.24x, log by up to 1.45x) |
| 32 | 80815 | - | 8070-25067 | 5448-13946 | 12078 | 295240 | 6081-23078 | 8389 | 16587 | 67188 | undecided (LSM by up to 4.24x, log by up to 1.38x) |
| 64 | 78486-79905 | - | 8070-25067 | 5448-13946 | 8703 | 292149 | 6081-23078 | 5448-6866 | 14171-15590 | 64888-66306 | undecided (LSM by up to 4.24x, log by up to 1.13x) |
| 128 | 78486-79905 | 2469-10967 | 8070-25067 | 5448-13946 | 5448-7641 | 289049-291277 | 6081-23078 | 5448-6866 | 14171-15590 | 64888-66306 | undecided (LSM by up to 2.01x, log by up to 2.78x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 352998 | - | - | - | 79528 | page LSM (4.44x to 4.44x over B+tree) |
| 2 | 89707 | - | - | - | - | 325542 | - | - | - | 74636 | too close (page LSM by 1.20x to 1.20x over page) |
| 4 | 86219 | - | - | - | - | 313150 | - | - | 23441 | 72110 | page log (3.08x to 3.08x over page LSM) |
| 8 | 84068 | - | 52793 | - | - | 305160 | 50803 | - | 20241 | 70325 | page log (2.51x to 2.51x over log) |
| 15.05 | 82650 | - | 21401 | 14889 | 18348 | 299753 | 19412 | 11346 | 18527 | 68977 | LSM (1.63x to 1.63x over page log) |
| 16 | 82511 | - | 8070-21349 | 14166 | 17484 | 299334 | 6081-19360 | 10995 | 18385 | 68835 | undecided (LSM by up to 1.76x, log by up to 1.81x) |
| 27.90 | 81140 | - | 8070-21349 | 5448-13946 | 12861 | 296044 | 6081-19360 | 8820 | 16929 | 67495 | undecided (LSM by up to 3.55x, log by up to 1.45x) |
| 32 | 80815 | - | 8070-21349 | 5448-13946 | 12078 | 295240 | 6081-19360 | 8389 | 16587 | 67188 | undecided (LSM by up to 3.55x, log by up to 1.38x) |
| 64 | 78486-79905 | - | 8070-21349 | 5448-13946 | 8703 | 292149 | 6081-19360 | 5448-6866 | 14171-15590 | 64888-66306 | undecided (LSM by up to 3.55x, log by up to 1.13x) |
| 128 | 78486-79905 | 2469-7249 | 8070-21349 | 5448-13946 | 5448-7641 | 289049-291277 | 6081-19360 | 5448-6866 | 14171-15590 | 64888-66306 | undecided (LSM by up to 1.33x, log by up to 2.78x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 773 | 0 | 4874 | 834413 | 0 | 5.00 | 177 |
| 4 | B+tree, copy-on-write | 2410 | 0 | 18058 | 0 | 0 | 2.00 | 312 |
| 4 | page log, GC at 1/2 | 927 | 0 | 0 | 31974294 | 31974294 | 2.00 | 211 |
| 4 | page LSM | 722 | 0 | 0 | 154652464 | 138602868 | 1.00 | 157 |
| 8 | page | 558 | 0 | 4874 | 834413 | 0 | 5.00 | 115 |
| 8 | log, index on disk (1.5 B) | 3132 | 2862 | 0 | 12113044 | 12083980 | 1.00 | 65 |
| 8 | B+tree, copy-on-write | 1611 | 0 | 18058 | 0 | 0 | 2.00 | 202 |
| 8 | log, index on disk, GC at 2/3 | 3132 | 2862 | 0 | 7585914 | 7556851 | 1.00 | 65 |
| 8 | page log, GC at 1/2 | 607 | 0 | 0 | 31974294 | 31974294 | 2.00 | 127 |
| 8 | page LSM | 544 | 0 | 0 | 154652464 | 138602868 | 1.00 | 112 |
| 16 | page | 402 | 0 | 4874 | 834413 | 0 | 5.00 | 72 |
| 16 | log, index on disk (1.5 B) | 1119 | 850 | 0 | 12113044 | 12083980 | 1.00 | 14 |
| 16 | LSM, node cache | 975 | 0 | 0 | 10705690 | 8926955 | 1.00 | 14 |
| 16 | LSM, block cache | 1307 | 0 | 0 | 10705690 | 8926955 | 1.00 | 149 |
| 16 | B+tree, copy-on-write | 1028 | 0 | 18058 | 0 | 0 | 2.00 | 123 |
| 16 | log, index on disk, GC at 2/3 | 1119 | 850 | 0 | 7585914 | 7556851 | 1.00 | 14 |
| 16 | LSM, page-keyed | 658 | 0 | 0 | 10705690 | 8926955 | 1.00 | 114 |
| 16 | page log, GC at 1/2 | 421 | 0 | 0 | 31974294 | 31974294 | 2.00 | 77 |
| 16 | page LSM | 395 | 0 | 0 | 154652464 | 138602868 | 1.00 | 70 |
| 32 | page | 233 | 0 | 4874 | 834413 | 0 | 5.00 | 28 |
| 32 | log, index on disk (1.5 B) | 1119 | 850 | 0 | 12113044 | 12083980 | 1.00 | 14 |
| 32 | LSM, node cache | 953 | 0 | 0 | 10705690 | 8926955 | 1.00 | 14 |
| 32 | LSM, block cache | 766 | 0 | 0 | 10705690 | 8926955 | 1.00 | 78 |
| 32 | B+tree, copy-on-write | 619 | 0 | 18058 | 0 | 0 | 2.00 | 71 |
| 32 | log, index on disk, GC at 2/3 | 1119 | 850 | 0 | 7585914 | 7556851 | 1.00 | 14 |
| 32 | LSM, page-keyed | 397 | 0 | 0 | 10705690 | 8926955 | 1.00 | 43 |
| 32 | page log, GC at 1/2 | 242 | 0 | 0 | 31974294 | 31974294 | 2.00 | 30 |
| 32 | page LSM | 230 | 0 | 0 | 154652464 | 138602868 | 1.00 | 27 |

### Sensitivity to the index engine's write amplification, 16 GB

The log store with its index on disk (1.5 B per record in DRAM), IO cost per block. 12.9 is the amplification the tables above use, the model's 15.5 scaled by the RocksDB measurement.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 12.9 | 6145-11244 | 6145-10129 | 8070-25067 | 8070-21349 |
| 15.5 | 6938-12037 | 6938-10922 | 8863-25860 | 8863-22142 |
| 5.0 | 3737-8836 | 3737-7721 | 5662-22659 | 5662-18941 |
| 30.0 | 11390-16489 | 11390-15374 | 13315-30312 | 13315-26594 |
| page store, for comparison | 21007 | 21007 | 82511 | 82511 |

### Sensitivity to the record engine's write amplification, 16 GB

The LSM of records, IO cost per block. 5.0 is the amplification the tables above use, the model's 5.0 scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 5.0, node cache | 7302 | 7302 | 14166 | 14166 |
| 5.0, block cache | 8297 | 8297 | 17484 | 17484 |
| 5.0, page-keyed | 6350 | 6350 | 10995 | 10995 |
| 5.0, node cache | 7287 | 7287 | 14151 | 14151 |
| 5.0, block cache | 8283 | 8283 | 17470 | 17470 |
| 5.0, page-keyed | 6336 | 6336 | 10980 | 10980 |
| 15.0, node cache | 15104 | 15104 | 21968 | 21968 |
| 15.0, block cache | 16099 | 16099 | 25286 | 25286 |
| 15.0, page-keyed | 14153 | 14153 | 18797 | 18797 |
| 30.0, node cache | 26829 | 26829 | 33693 | 33693 |
| 30.0, block cache | 27824 | 27824 | 37011 | 37011 |
| 30.0, page-keyed | 25878 | 25878 | 30522 | 30522 |

### Sensitivity to garbage collection, 16 GB

The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.

| g | store | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes | space |
|---|---|---|---|---|---|---|
| 0.50 | log, index on disk (index amplification 12.9) | 6145-11244 | 6145-10129 | 8070-25067 | 8070-21349 | 2.0x |
| 0.50 | page log | 15355 | 15355 | 18385 | 18385 | 2.0x |
| 0.67 | log, index on disk (index amplification 9.9) | 4156-9255 | 4156-8139 | 6081-23078 | 6081-19360 | 3.0x |
| 0.67 | page log | 11843 | 11843 | 14872 | 14872 | 3.0x |
| 0.80 | log, index on disk (index amplification 7.0) | 3039-8138 | 3039-7022 | 4964-21960 | 4964-18242 | 5.0x |
| 0.80 | page log | 10086 | 10086 | 13116 | 13116 | 5.0x |

