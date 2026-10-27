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
| paged-pos(res=2) | 0.145 | 524.2 | 512.2 | 4330.5 | 291.4 | 3.0 | 2 | 22186 | 0 |
| append-readback(res=12) | 0.576 | 0.0 | 3865.5 | 4330.5 | 291.4 | 1.0 | 587 | 0 | 0 |
| btree-cow(res=2) | 0.000 | 3554.7 | 5157.5 | 4330.5 | 291.4 | 2.0 | 1 | 603674 | 0 |

`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens
inside libmdbx and is invisible to these counters, so its seq/rand columns are
zero by construction and a modeled cost would be meaningless. It is compared
separately on real wall-clock and real on-disk size.


## Read pricing sensitivity (nvme)

A per-IO read price ignores transfer size; a page store moves 4 KB per hop
while a log store moves one sector. Both prices shown.

| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |
|---|---|---|---|
| paged-pos(res=2) | 3731.3 | 3731.3 | 2048.7 |
| append-readback(res=12) | 12169.0 | 7095.5 | 1932.8 |
| btree-cow(res=2) | 29732.1 | 29732.1 | 20630.0 |

## Modeled cost, profile `nvme-optimistic`

Units: one sequential 4KB write = 1. rand write 4x, rand read 3x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 3731.3 | 1.00x | 2 |
| append-readback(res=12) | 12169.0 | 3.26x | 587 |
| btree-cow(res=2) | 29732.1 | 7.97x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Modeled cost, profile `commodity-ssd`

Units: one sequential 4KB write = 1. rand write 16x, rand read 10x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 13729.5 | 1.00x | 2 |
| append-readback(res=12) | 40148.7 | 2.92x | 587 |
| btree-cow(res=2) | 108573.3 | 7.91x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Real-IO comparison (wall clock, on-disk)

| backend | commit wall ms/blk | disk MB | basis |
|---|---|---|---|
| paged-pos(res=2) | 13.264 | 22186 | real WAL write+fsync; page IO counter-modeled |
| append-readback(res=12) | 14.582 | 0 | counter-modeled |
| btree-cow(res=2) | 14.646 | 603674 | counter-modeled |

## Backend detail

- paged-pos(res=2): 5679656 stored pages, mean occupancy 6.7/126 slots (5.4%), WAL 9909 MB
  - elision: 17073838 page writes avoided, 0 read fallbacks
  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: [5056937, 426462, 152266, 14760, 4008, 25223]
- append-readback(res=12): live 3137 MB, amortized compaction reclaim 16676 MB, live records 39983163, live branches 20042227, idx_branches 20042227, idx_leaves 19940936, write-path RMW reads 219507788 (sibling-dirty 18102303), uncharged displaced-leaf splits 31053, dram bytes 616016126
- btree-cow MODELED: 33 entries per 4 KB leaf page (96 B mean entry at fill 0.8), fanout 60, height 6, pages per level [151964987, 2532750, 42213, 704, 12, 1] (633.0 GB)
- btree-cow: resident leaf pages 399 (1.6 MB); records created 40969945, rewritten 240660800, deleted 1089008, leaf changes 28777809
- btree-cow, measured window: leaf pages copied 1752008, interior pages copied 1797645, freelist pages 4052; key walks 435265, prefix hits 3949447, fallback walks 26450, memo hits 237192

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
- SAMPLED: only accounts and contracts whose address hash starts with the 2 bits of part 1 are replayed, one key-space subtree in 4. Every figure below is scaled to the whole: per-block counts by 4, cache sizes and DRAM by 4, the full state and the index buffer taken at 1/4 for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.
- window: 1000 measured blocks after 49000 warm-up blocks; 50000 blocks available from 24000000
- full state stood in for: 334563311 accounts, 1438571749 storage slots, 399723913 code chunks = 626855571 leaves, 1253711142 records. Storage contracts: 25000000 (ASSUMED). An untouched contract is taken to hold 57 slots, the mean; real sizes are heavy-tailed.
- replay built from: 3884851 account changes (484744 creations) and 9267072 slot changes; 1851504 keys in the starting state; stand-ins: 6313802 beside account paths, 174924 beside contract buckets, 6800911 inside contracts' storage. 1866 of 20005 contracts written in the storage zone have a fetched slot count, covering 95.7% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes 5247763 distinct slots there in 1755649 stems (2.99 per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.
- path length, account zone: 514.8 leaves written per block, 32.4 branch records above each on average, 40 at most
- path length, storage zone: 880.5 leaves written per block, 49.2 branch records above each on average, 57 at most
- execution reads, per block: 858.3 of existing keys and 307.4 of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.
- measured, log store, per block: 17275.9 records written, 1.22 MB appended by the commit; with no cache 15462.0 reads, of which 858.3 are execution reads, 628.9 fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; 33063.9 index updates, 15.7 deleted branches to mark dead, 124.2 displaced leaves
- measured, page store, per block: 2096.7 merkle pages written; with no cache 3954.9 reads, of which 2048.7 are merkle pages and the rest value-store leaf pages (execution reads included); 740.6 value-store leaf pages and 670.1 bottom branch nodes rewritten, 13.70 MB held by one commit
- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at 87 B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.
- fixed DRAM at full-state size: log store with its index in DRAM 71.9 GB (15.4 B per record); with its index on disk 7.6 GB or 20.4 GB (1.5 or 4.25 B per record, plus a 256 MB write buffer and 298 MB of page pointers, all ASSUMED); page store 1.00 GB (0.80 GB of resident bottom branch nodes over 74462908 value-store leaf pages, 0.14 GB of bucket map, ASSUMED, and one commit's pages).
- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, 9.00 GB of records for the log store and 51.56 GB of pages for the page store. A replay of 50000 blocks cannot tell further; a longer one narrows the range.
- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over 75 GB of 16-byte entries, a 256 MB write buffer, levels growing by 10x down to one that holds every entry. How much each level absorbs is MEASURED from the 187913241 index updates of the replay (15033 per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The 0.9 relocations per update are taken never to be absorbed. Write amplification 13.2 per index update (212 B written and read): the model gives 15.8, and a real RocksDB fed this stream as the index sees it wrote 0.84 of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); 15 for the assumed engine before. That costs 3075 per block on `nvme-optimistic` and 3075 on `commodity-ssd`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 4.194e6 | 235 | yes | 0.622 | 3.10 |
  | L1 | 1.254e7 | 734 | yes | 0.595 | 6.55 |
  | L2 | 1.254e8 | 7857 | yes | 0.556 | 6.12 |
  | L3 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a 256 MB write buffer and levels growing by 10x down to one holding all 5014844568 records. An entry is 82 B, the mean record of this replay's write stream (73.8 B, its 4 B log header standing in for the varint lengths) plus 8 B of sequence number and type; prefix compression is not modeled. Updates are the 17322.0 records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification 7.4 per record update (608 B written and read back by compaction, plus one WAL copy of each update), the model's 7.4 scaled by 1.01, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against 13.2 for the index entries above; that costs 4629 per block on `nvme-optimistic` and 4629 on `commodity-ssd`. Fixed DRAM 8.89 GB: 1.85 B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at 98 B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 8.202e5 | 188 | yes | 0.290 | 1.02 |
  | L1 | 1.254e6 | 318 | yes | 0.263 | 2.89 |
  | L2 | 1.254e7 | 5079 | yes | 0.164 | 1.81 |
  | L3 | 1.254e8 | 55837 | no: longest window's share used | 0.149 | 1.64 |
  | L4 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, 33 per leaf page (96 B mean entry at fill 0.8, ASSUMED), 60 children per interior page (ASSUMED), height 6 over 151964987 leaf pages (633 GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM (0.002 GB). The page map is static: no page splits or merges over the replay.
- measured, B+tree, per block: 14218.8 pages rewritten copy-on-write; with no cache 7611.1 leaf-page reads and 13018.9 interior-page reads, execution reads included; 7008.0 leaf pages and 7190.6 interior pages copied, 55.54 MB held by one commit, 2 fsyncs
- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM (2.22 GB: 16 B per page over 148925816 pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: 2837.3 pages appended per block, each superseding one; space up to 2x the live pages.
- page LSM, MODELED from the page store's run: the same pages as 4112 B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the 31508169 page writes of the replay (2521 per block, warm-up included). page write amplification 9.4 per page update (38 KB written and read back by compaction per page written, plus one WAL copy).

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 1.632e4 | 14 | yes | 0.458 | 1.96 |
  | L1 | 3.723e4 | 41 | yes | 0.363 | 3.99 |
  | L2 | 3.723e5 | 769 | yes | 0.192 | 2.11 |
  | L3 | 3.723e6 | 12351 | no: longest window's share used | 0.120 | 1.32 |
  | L4 | 3.723e7 | - | - | 0.000 | 0.00 |

- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations 7871.0 per block at g = 2/3 (15742.0 at g = 1/2), index write amplification 10.6 (13.2 at g = 1/2), compaction reading 1.63 MB and rewriting 0.54 MB per block (2.17 and 1.09 at g = 1/2); space up to 3x the live records instead of 2x.
- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves (124.2 per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the 1151.7 elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.
- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified.
- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name.
- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by 1.3x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.

### Reads (write path and execution) per block against cache size, exact range only

Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.

| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |
|---|---|---|---|---|---|---|
| 1/64 | 1589228 | 0.141 | 4441.3 | 210376 | 0.806 | 973.5 |
| 1/32 | 3178460 | 0.281 | 3620.0 | 420756 | 1.611 | 789.3 |
| 1/16 | 6356920 | 0.562 | 2917.0 | 841512 | 3.223 | 643.3 |
| 1/8 | 12713844 | 1.125 | 2428.5 | 1683028 | 6.445 | 551.2 |
| 1/4 | 25427688 | 2.250 | 1928.8 | 3366060 | 12.891 | 468.4 |
| 1/2 | 50855380 | 4.499 | 1515.6 | 6732124 | 25.781 | 360.2 |
| 1/1 | 101710760 | 8.999 | 1139.2 | 13464252 | 51.563 | 249.4 |

With no cache at all: 15462.0 reads per block for the log store, 3954.9 for the page store. Of the log store's, 1132.3 are of records untouched since the replay began; of the page store's, 247.8.

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 24725 | - | - | - | - | 72270 | - | - | - | 54369 | page (2.20x to 2.20x over page LSM) |
| 2 | 17038 | - | - | - | - | 67613 | - | - | - | 53407 | page (3.13x to 3.13x over page LSM) |
| 4 | 16263 | - | - | - | - | 64857 | - | - | 12581 | 52875 | too close (page log by 1.29x to 1.29x over page) |
| 8 | 15923 | - | 23568 | - | - | 62668 | 22113 | - | 11940 | 52596 | page log (1.33x to 1.33x over page) |
| 16 | 15620 | - | 11954 | 9185 | 9471 | 61113 | 10499 | 6993 | 11623 | 52300 | LSM (1.50x to 1.50x over log) |
| 16.55 | 15607 | - | 11758 | 9068 | 9380 | 61045 | 10303 | 6970 | 11601 | 52287 | LSM (1.48x to 1.48x over log) |
| 29.40 | 15330 | - | 4922-11716 | 5368-8765 | 8256 | 59817 | 3467-10261 | 6564 | 11310 | 52015 | undecided (LSM by up to 1.91x, log by up to 1.89x) |
| 32 | 15291 | - | 4922-11716 | 5368-8765 | 8125 | 59659 | 3467-10261 | 6501 | 11270 | 51977 | undecided (LSM by up to 1.91x, log by up to 1.87x) |
| 64 | 14294-15037 | - | 4922-11716 | 5368-8765 | 7217 | 58749 | 3467-10261 | 5368-6111 | 10254-10998 | 50986-51729 | undecided (LSM by up to 1.91x, log by up to 1.76x) |
| 128 | 14294-15037 | 1428-4824 | 4922-11716 | 5368-8765 | 5368-6690 | 56915-58248 | 3467-10261 | 5368-6111 | 10254-10998 | 50986-51729 | too close (log by 1.11x to 4.28x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 24725 | - | - | - | - | 72270 | - | - | - | 54369 | page (2.20x to 2.20x over page LSM) |
| 2 | 17038 | - | - | - | - | 67613 | - | - | - | 53407 | page (3.13x to 3.13x over page LSM) |
| 4 | 16263 | - | - | - | - | 64857 | - | - | 12581 | 52875 | too close (page log by 1.29x to 1.29x over page) |
| 8 | 15923 | - | 19489 | - | - | 62668 | 18034 | - | 11940 | 52596 | page log (1.33x to 1.33x over page) |
| 16 | 15620 | - | 10416 | 9185 | 9471 | 61113 | 8961 | 6993 | 11623 | 52300 | too close (LSM by 1.28x to 1.28x over log) |
| 16.55 | 15607 | - | 10262 | 9068 | 9380 | 61045 | 8807 | 6970 | 11601 | 52287 | too close (LSM by 1.26x to 1.26x over log) |
| 29.40 | 15330 | - | 4922-10230 | 5368-8765 | 8256 | 59817 | 3467-8775 | 6564 | 11310 | 52015 | undecided (LSM by up to 1.63x, log by up to 1.89x) |
| 32 | 15291 | - | 4922-10230 | 5368-8765 | 8125 | 59659 | 3467-8775 | 6501 | 11270 | 51977 | undecided (LSM by up to 1.63x, log by up to 1.87x) |
| 64 | 14294-15037 | - | 4922-10230 | 5368-8765 | 7217 | 58749 | 3467-8775 | 5368-6111 | 10254-10998 | 50986-51729 | undecided (LSM by up to 1.63x, log by up to 1.76x) |
| 128 | 14294-15037 | 1428-3338 | 4922-10230 | 5368-8765 | 5368-6690 | 56915-58248 | 3467-8775 | 5368-6111 | 10254-10998 | 50986-51729 | log (1.61x to 4.28x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 91401 | - | - | - | - | 278805 | - | - | - | 62302 | page LSM (1.47x to 1.47x over page) |
| 2 | 65779 | - | - | - | - | 263280 | - | - | - | 59095 | too close (page LSM by 1.11x to 1.11x over page) |
| 4 | 63194 | - | - | - | - | 254092 | - | - | 18090 | 57324 | page log (3.17x to 3.17x over page LSM) |
| 8 | 62060 | - | 68964 | - | - | 246797 | 67509 | - | 15954 | 56391 | page log (3.53x to 3.53x over page LSM) |
| 16 | 61050 | - | 30250 | 19000 | 19956 | 241615 | 28795 | 11693 | 14898 | 55408 | too close (LSM by 1.27x to 1.27x over page log) |
| 16.55 | 61007 | - | 29595 | 18610 | 19652 | 241388 | 28140 | 11619 | 14822 | 55363 | too close (LSM by 1.28x to 1.28x over page log) |
| 29.40 | 60085 | - | 6812-29457 | 6277-17600 | 15904 | 237294 | 5357-28002 | 10265 | 13855 | 54455 | undecided (LSM by up to 4.46x, log by up to 1.92x) |
| 32 | 59955 | - | 6812-29457 | 6277-17600 | 15469 | 236766 | 5357-28002 | 10055 | 13719 | 54331 | undecided (LSM by up to 4.46x, log by up to 1.88x) |
| 64 | 56631-59108 | - | 6812-29457 | 6277-17600 | 12442 | 233733 | 5357-28002 | 6277-8755 | 10334-12812 | 51026-53504 | undecided (LSM by up to 4.46x, log by up to 1.63x) |
| 128 | 56631-59108 | 2337-13660 | 6812-29457 | 6277-17600 | 6277-10684 | 227621-232064 | 5357-28002 | 6277-8755 | 10334-12812 | 51026-53504 | undecided (LSM by up to 2.18x, log by up to 3.75x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 91401 | - | - | - | - | 278805 | - | - | - | 62302 | page LSM (1.47x to 1.47x over page) |
| 2 | 65779 | - | - | - | - | 263280 | - | - | - | 59095 | too close (page LSM by 1.11x to 1.11x over page) |
| 4 | 63194 | - | - | - | - | 254092 | - | - | 18090 | 57324 | page log (3.17x to 3.17x over page LSM) |
| 8 | 62060 | - | 55368 | - | - | 246797 | 53913 | - | 15954 | 56391 | page log (3.38x to 3.38x over log) |
| 16 | 61050 | - | 25123 | 19000 | 19956 | 241615 | 23667 | 11693 | 14898 | 55408 | too close (LSM by 1.27x to 1.27x over page log) |
| 16.55 | 61007 | - | 24612 | 18610 | 19652 | 241388 | 23156 | 11619 | 14822 | 55363 | too close (LSM by 1.28x to 1.28x over page log) |
| 29.40 | 60085 | - | 6812-24503 | 6277-17600 | 15904 | 237294 | 5357-23048 | 10265 | 13855 | 54455 | undecided (LSM by up to 3.67x, log by up to 1.92x) |
| 32 | 59955 | - | 6812-24503 | 6277-17600 | 15469 | 236766 | 5357-23048 | 10055 | 13719 | 54331 | undecided (LSM by up to 3.67x, log by up to 1.88x) |
| 64 | 56631-59108 | - | 6812-24503 | 6277-17600 | 12442 | 233733 | 5357-23048 | 6277-8755 | 10334-12812 | 51026-53504 | undecided (LSM by up to 3.67x, log by up to 1.63x) |
| 128 | 56631-59108 | 2337-8706 | 6812-24503 | 6277-17600 | 6277-10684 | 227621-232064 | 5357-23048 | 6277-8755 | 10334-12812 | 51026-53504 | undecided (log by up to 3.75x, LSM by up to 1.39x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 656 | 0 | 3511 | 608086 | 0 | 5.00 | 181 |
| 4 | B+tree, copy-on-write | 2647 | 0 | 14219 | 0 | 0 | 2.00 | 377 |
| 4 | page log, GC at 1/2 | 776 | 0 | 0 | 23243096 | 23243096 | 2.00 | 198 |
| 4 | page LSM | 630 | 0 | 0 | 121161190 | 109494245 | 1.00 | 177 |
| 8 | page | 543 | 0 | 3511 | 608086 | 0 | 5.00 | 162 |
| 8 | log, index on disk (1.5 B) | 3372 | 3108 | 0 | 9412491 | 9276989 | 1.00 | 60 |
| 8 | B+tree, copy-on-write | 1918 | 0 | 14219 | 0 | 0 | 2.00 | 296 |
| 8 | log, index on disk, GC at 2/3 | 3372 | 3108 | 0 | 6101516 | 5966014 | 1.00 | 60 |
| 8 | page log, GC at 1/2 | 562 | 0 | 0 | 23243096 | 23243096 | 2.00 | 165 |
| 8 | page LSM | 537 | 0 | 0 | 121161190 | 109494245 | 1.00 | 161 |
| 16 | page | 442 | 0 | 3511 | 608086 | 0 | 5.00 | 150 |
| 16 | log, index on disk (1.5 B) | 1436 | 1172 | 0 | 9412491 | 9276989 | 1.00 | 21 |
| 16 | LSM, node cache | 1396 | 0 | 0 | 11951340 | 10534028 | 1.00 | 22 |
| 16 | LSM, block cache | 1492 | 0 | 0 | 11951340 | 10534028 | 1.00 | 177 |
| 16 | B+tree, copy-on-write | 1399 | 0 | 14219 | 0 | 0 | 2.00 | 245 |
| 16 | log, index on disk, GC at 2/3 | 1436 | 1172 | 0 | 6101516 | 5966014 | 1.00 | 21 |
| 16 | LSM, page-keyed | 666 | 0 | 0 | 11951340 | 10534028 | 1.00 | 162 |
| 16 | page log, GC at 1/2 | 456 | 0 | 0 | 23243096 | 23243096 | 2.00 | 152 |
| 16 | page LSM | 438 | 0 | 0 | 121161190 | 109494245 | 1.00 | 150 |
| 32 | page | 332 | 0 | 3511 | 608086 | 0 | 5.00 | 137 |
| 32 | log, index on disk (1.5 B) | 1396 | 1132 | 0 | 9412491 | 9276989 | 1.00 | 21 |
| 32 | LSM, node cache | 1256 | 0 | 0 | 11951340 | 10534028 | 1.00 | 21 |
| 32 | LSM, block cache | 1043 | 0 | 0 | 11951340 | 10534028 | 1.00 | 151 |
| 32 | B+tree, copy-on-write | 915 | 0 | 14219 | 0 | 0 | 2.00 | 164 |
| 32 | log, index on disk, GC at 2/3 | 1396 | 1132 | 0 | 6101516 | 5966014 | 1.00 | 21 |
| 32 | LSM, page-keyed | 502 | 0 | 0 | 11951340 | 10534028 | 1.00 | 142 |
| 32 | page log, GC at 1/2 | 338 | 0 | 0 | 23243096 | 23243096 | 2.00 | 137 |
| 32 | page LSM | 330 | 0 | 0 | 121161190 | 109494245 | 1.00 | 136 |

### Sensitivity to the index engine's write amplification, 16 GB

The log store with its index on disk (1.5 B per record in DRAM), IO cost per block. 13.2 is the amplification the tables above use, the model's 15.8 scaled by the RocksDB measurement.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 13.2 | 11954 | 10416 | 30250 | 25123 |
| 15.8 | 12543 | 11005 | 30839 | 25712 |
| 5.0 | 10041 | 8503 | 28337 | 23210 |
| 30.0 | 15853 | 14315 | 34149 | 29022 |
| page store, for comparison | 15620 | 15620 | 61050 | 61050 |

### Sensitivity to the record engine's write amplification, 16 GB

The LSM of records, IO cost per block. 7.4 is the amplification the tables above use, the model's 7.4 scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 7.4, node cache | 9185 | 9185 | 19000 | 19000 |
| 7.4, block cache | 9471 | 9471 | 19956 | 19956 |
| 7.4, page-keyed | 6993 | 6993 | 11693 | 11693 |
| 5.0, node cache | 7670 | 7670 | 17485 | 17485 |
| 5.0, block cache | 7956 | 7956 | 18441 | 18441 |
| 5.0, page-keyed | 5478 | 5478 | 10178 | 10178 |
| 15.0, node cache | 13898 | 13898 | 23713 | 23713 |
| 15.0, block cache | 14185 | 14185 | 24670 | 24670 |
| 15.0, page-keyed | 11706 | 11706 | 16406 | 16406 |
| 30.0, node cache | 23241 | 23241 | 33056 | 33056 |
| 30.0, block cache | 23528 | 23528 | 34012 | 34012 |
| 30.0, page-keyed | 21049 | 21049 | 25749 | 25749 |

### Sensitivity to garbage collection, 16 GB

The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.

| g | store | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes | space |
|---|---|---|---|---|---|---|
| 0.50 | log, index on disk (index amplification 13.2) | 11954 | 10416 | 30250 | 25123 | 2.0x |
| 0.50 | page log | 11623 | 11623 | 14898 | 14898 | 2.0x |
| 0.67 | log, index on disk (index amplification 10.6) | 10499 | 8961 | 28795 | 23667 | 3.0x |
| 0.67 | page log | 9070 | 9070 | 12344 | 12344 | 3.0x |
| 0.80 | log, index on disk (index amplification 7.9) | 9689 | 8151 | 27985 | 22858 | 5.0x |
| 0.80 | page log | 7793 | 7793 | 11067 | 11067 | 5.0x |

