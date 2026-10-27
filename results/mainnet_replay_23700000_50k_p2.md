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
| paged-pos(res=2) | 0.198 | 718.2 | 706.1 | 5455.4 | 367.1 | 3.0 | 2 | 18624 | 0 |
| append-readback(res=12) | 0.745 | 0.0 | 5269.3 | 5455.4 | 367.1 | 1.0 | 522 | 0 | 0 |
| btree-cow(res=2) | 0.000 | 4525.3 | 6573.8 | 5455.4 | 367.1 | 2.0 | 1 | 603674 | 0 |

`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens
inside libmdbx and is invisible to these counters, so its seq/rand columns are
zero by construction and a modeled cost would be meaningless. It is compared
separately on real wall-clock and real on-disk size.


## Read pricing sensitivity (nvme)

A per-IO read price ignores transfer size; a page store moves 4 KB per hop
while a log store moves one sector. Both prices shown.

| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |
|---|---|---|---|
| paged-pos(res=2) | 5103.1 | 5103.1 | 2824.6 |
| append-readback(res=12) | 16431.7 | 9515.8 | 2634.7 |
| btree-cow(res=2) | 37863.4 | 37863.4 | 26295.3 |

## Modeled cost, profile `nvme-optimistic`

Units: one sequential 4KB write = 1. rand write 4x, rand read 3x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 5103.1 | 1.00x | 2 |
| append-readback(res=12) | 16431.7 | 3.22x | 522 |
| btree-cow(res=2) | 37863.4 | 7.42x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Modeled cost, profile `commodity-ssd`

Units: one sequential 4KB write = 1. rand write 16x, rand read 10x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 18788.0 | 1.00x | 2 |
| append-readback(res=12) | 54141.3 | 2.88x | 522 |
| btree-cow(res=2) | 138266.0 | 7.36x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Real-IO comparison (wall clock, on-disk)

| backend | commit wall ms/blk | disk MB | basis |
|---|---|---|---|
| paged-pos(res=2) | 10.820 | 18624 | real WAL write+fsync; page IO counter-modeled |
| append-readback(res=12) | 3.544 | 0 | counter-modeled |
| btree-cow(res=2) | 9.708 | 603674 | counter-modeled |

## Backend detail

- paged-pos(res=2): 4767863 stored pages, mean occupancy 7.1/126 slots (5.6%), WAL 12880 MB
  - elision: 18575801 page writes avoided, 0 read fallbacks
  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: [4142927, 447648, 122451, 13920, 10089, 30828]
- append-readback(res=12): live 2768 MB, amortized compaction reclaim 21918 MB, live records 35589136, live branches 17802148, idx_branches 17802148, idx_leaves 17786988, write-path RMW reads 291778123 (sibling-dirty 22300119), uncharged displaced-leaf splits 26917, dram bytes 548348624
- btree-cow MODELED: 33 entries per 4 KB leaf page (96 B mean entry at fill 0.8), fanout 60, height 6, pages per level [151964987, 2532750, 42213, 704, 12, 1] (633.0 GB)
- btree-cow: resident leaf pages 396 (1.6 MB); records created 37555543, rewritten 315828202, deleted 2004524, leaf changes 29973419
- btree-cow, measured window: leaf pages copied 2313567, interior pages copied 2205698, freelist pages 5028; key walks 369938, prefix hits 5144563, fallback walks 16728, memo hits 307968

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
- SAMPLED: only accounts and contracts whose address hash starts with the 2 bits of part 2 are replayed, one key-space subtree in 4. Every figure below is scaled to the whole: per-block counts by 4, cache sizes and DRAM by 4, the full state and the index buffer taken at 1/4 for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.
- window: 1000 measured blocks after 49000 warm-up blocks; 50000 blocks available from 23700000
- full state stood in for: 334563311 accounts, 1438571749 storage slots, 399723913 code chunks = 626855571 leaves, 1253711142 records. Storage contracts: 25000000 (ASSUMED). An untouched contract is taken to hold 57 slots, the mean; real sizes are heavy-tailed.
- replay built from: 3767386 account changes (343177 creations) and 9628290 slot changes; 1908825 keys in the starting state; stand-ins: 5806120 beside account paths, 156046 beside contract buckets, 8363574 inside contracts' storage. 1840 of 17449 contracts written in the storage zone have a fetched slot count, covering 97.8% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes 2476170 distinct slots there in 2205497 stems (1.12 per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.
- path length, account zone: 467.3 leaves written per block, 32.5 branch records above each on average, 40 at most
- path length, storage zone: 619.3 leaves written per block, 45.5 branch records above each on average, 56 at most
- execution reads, per block: 933.4 of existing keys and 535.0 of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.
- measured, log store, per block: 21642.1 records written, 1.50 MB appended by the commit; with no cache 21077.2 reads, of which 933.4 are execution reads, 962.3 fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; 43213.0 index updates, 63.8 deleted branches to mark dead, 107.7 displaced leaves
- measured, page store, per block: 2872.9 merkle pages written; with no cache 5301.9 reads, of which 2824.6 are merkle pages and the rest value-store leaf pages (execution reads included); 1009.0 value-store leaf pages and 937.1 bottom branch nodes rewritten, 18.82 MB held by one commit
- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at 88 B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.
- fixed DRAM at full-state size: log store with its index in DRAM 71.9 GB (15.4 B per record); with its index on disk 7.6 GB or 20.4 GB (1.5 or 4.25 B per record, plus a 256 MB write buffer and 298 MB of page pointers, all ASSUMED); page store 1.02 GB (0.80 GB of resident bottom branch nodes over 74462908 value-store leaf pages, 0.14 GB of bucket map, ASSUMED, and one commit's pages).
- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, 7.30 GB of records for the log store and 48.99 GB of pages for the page store. A replay of 50000 blocks cannot tell further; a longer one narrows the range.
- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over 75 GB of 16-byte entries, a 256 MB write buffer, levels growing by 10x down to one that holds every entry. How much each level absorbs is MEASURED from the 252910407 index updates of the replay (20233 per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The 1.0 relocations per update are taken never to be absorbed. Write amplification 13.0 per index update (208 B written and read): the model gives 15.6, and a real RocksDB fed this stream as the index sees it wrote 0.83 of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); 15 for the assumed engine before. That costs 3956 per block on `nvme-optimistic` and 3956 on `commodity-ssd`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 4.194e6 | 169 | yes | 0.621 | 3.10 |
  | L1 | 1.254e7 | 529 | yes | 0.591 | 6.50 |
  | L2 | 1.254e8 | 5691 | yes | 0.550 | 6.05 |
  | L3 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a 256 MB write buffer and levels growing by 10x down to one holding all 5014844568 records. An entry is 81 B, the mean record of this replay's write stream (72.7 B, its 4 B log header standing in for the varint lengths) plus 8 B of sequence number and type; prefix compression is not modeled. Updates are the 21821.5 records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification 5.7 per record update (461 B written and read back by compaction, plus one WAL copy of each update), the model's 5.7 scaled by 1.01, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against 13.0 for the index entries above; that costs 4417 per block on `nvme-optimistic` and 4417 on `commodity-ssd`. Fixed DRAM 8.89 GB: 1.85 B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at 97 B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 8.317e5 | 164 | yes | 0.251 | 0.88 |
  | L1 | 1.254e6 | 281 | yes | 0.221 | 2.43 |
  | L2 | 1.254e7 | 5716 | yes | 0.108 | 1.19 |
  | L3 | 1.254e8 | 59122 | no: longest window's share used | 0.105 | 1.15 |
  | L4 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, 33 per leaf page (96 B mean entry at fill 0.8, ASSUMED), 60 children per interior page (ASSUMED), height 6 over 151964987 leaf pages (633 GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM (0.002 GB). The page map is static: no page splits or merges over the replay.
- measured, B+tree, per block: 18101.2 pages rewritten copy-on-write; with no cache 10130.6 leaf-page reads and 16164.7 interior-page reads, execution reads included; 9254.3 leaf pages and 8822.8 interior pages copied, 70.71 MB held by one commit, 2 fsyncs
- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM (2.22 GB: 16 B per page over 148925816 pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: 3881.9 pages appended per block, each superseding one; space up to 2x the live pages.
- page LSM, MODELED from the page store's run: the same pages as 4112 B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the 44820108 page writes of the replay (3586 per block, warm-up included). page write amplification 9.2 per page update (37 KB written and read back by compaction per page written, plus one WAL copy).

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 1.632e4 | 9 | yes | 0.497 | 2.13 |
  | L1 | 3.723e4 | 27 | yes | 0.384 | 4.23 |
  | L2 | 3.723e5 | 574 | yes | 0.181 | 1.99 |
  | L3 | 3.723e6 | 13700 | no: longest window's share used | 0.076 | 0.83 |
  | L4 | 3.723e7 | - | - | 0.000 | 0.00 |

- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations 10695.7 per block at g = 2/3 (21391.5 at g = 1/2), index write amplification 10.2 (13.0 at g = 1/2), compaction reading 2.22 MB and rewriting 0.74 MB per block (2.96 and 1.48 at g = 1/2); space up to 3x the live records instead of 2x.
- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves (107.7 per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the 1158.1 elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.
- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified.
- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name.
- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by 1.3x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.

### Reads (write path and execution) per block against cache size, exact range only

Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.

| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |
|---|---|---|---|---|---|---|
| 1/64 | 1275112 | 0.114 | 5367.2 | 199896 | 0.766 | 1390.1 |
| 1/32 | 2550224 | 0.228 | 4108.7 | 399792 | 1.531 | 1076.1 |
| 1/16 | 5100452 | 0.456 | 3190.2 | 799588 | 3.062 | 847.0 |
| 1/8 | 10200908 | 0.912 | 2451.4 | 1599176 | 6.124 | 661.2 |
| 1/4 | 20401820 | 1.824 | 1778.8 | 3198356 | 12.248 | 484.0 |
| 1/2 | 40803644 | 3.648 | 1262.8 | 6396712 | 24.497 | 277.3 |
| 1/1 | 81607288 | 7.296 | 883.3 | 12793424 | 48.994 | 128.7 |

With no cache at all: 21077.2 reads per block for the log store, 5301.9 for the page store. Of the log store's, 879.2 are of records untouched since the replay began; of the page store's, 127.9.

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 91931 | - | - | - | 73245 | too close (page LSM by 1.26x to 1.26x over B+tree) |
| 2 | 23405 | - | - | - | - | 84726 | - | - | - | 71563 | page (3.06x to 3.06x over page LSM) |
| 4 | 22158 | - | - | - | - | 80285 | - | - | 17114 | 70731 | too close (page log by 1.29x to 1.29x over page) |
| 8 | 21482 | - | 25482 | - | - | 77535 | 23522 | - | 16059 | 70132 | page log (1.34x to 1.34x over page) |
| 14.85 | 20943 | - | 11482 | 8120 | 9432 | 75889 | 9522 | 7197 | 15445 | 69613 | LSM (1.32x to 1.32x over log) |
| 16 | 20870 | - | 6182-11457 | 7889 | 9104 | 75723 | 4222-9498 | 7065 | 15371 | 69538 | undecided (LSM by up to 1.34x, log by up to 1.67x) |
| 27.69 | 20351 | - | 6182-11457 | 5190-7828 | 7520 | 74589 | 4222-9498 | 6230 | 14812 | 69040 | undecided (LSM by up to 1.83x, log by up to 1.48x) |
| 32 | 20238 | - | 6182-11457 | 5190-7828 | 7218 | 74266 | 4222-9498 | 6078 | 14691 | 68926 | undecided (LSM by up to 1.83x, log by up to 1.44x) |
| 64 | 19595-19979 | - | 6182-11457 | 5190-7828 | 6043 | 73248 | 4222-9498 | 5190-5574 | 14015-14399 | 68297-68681 | undecided (LSM by up to 1.83x, log by up to 1.32x) |
| 128 | 19595-19979 | 1712-4350 | 6182-11457 | 5190-7828 | 5190-5777 | 72445-73040 | 4222-9498 | 5190-5574 | 14015-14399 | 68297-68681 | too close (log by 1.19x to 3.26x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 91931 | - | - | - | 73245 | too close (page LSM by 1.26x to 1.26x over B+tree) |
| 2 | 23405 | - | - | - | - | 84726 | - | - | - | 71563 | page (3.06x to 3.06x over page LSM) |
| 4 | 22158 | - | - | - | - | 80285 | - | - | 17114 | 70731 | too close (page log by 1.29x to 1.29x over page) |
| 8 | 21482 | - | 21260 | - | - | 77535 | 19300 | - | 16059 | 70132 | too close (page log by 1.20x to 1.20x over log) |
| 14.85 | 20943 | - | 10323 | 8120 | 9432 | 75889 | 8363 | 7197 | 15445 | 69613 | too close (LSM by 1.16x to 1.16x over log) |
| 16 | 20870 | - | 6182-10303 | 7889 | 9104 | 75723 | 4222-8344 | 7065 | 15371 | 69538 | undecided (LSM by up to 1.18x, log by up to 1.67x) |
| 27.69 | 20351 | - | 6182-10303 | 5190-7828 | 7520 | 74589 | 4222-8344 | 6230 | 14812 | 69040 | undecided (LSM by up to 1.61x, log by up to 1.48x) |
| 32 | 20238 | - | 6182-10303 | 5190-7828 | 7218 | 74266 | 4222-8344 | 6078 | 14691 | 68926 | undecided (LSM by up to 1.61x, log by up to 1.44x) |
| 64 | 19595-19979 | - | 6182-10303 | 5190-7828 | 6043 | 73248 | 4222-8344 | 5190-5574 | 14015-14399 | 68297-68681 | undecided (LSM by up to 1.61x, log by up to 1.32x) |
| 128 | 19595-19979 | 1712-3196 | 6182-10303 | 5190-7828 | 5190-5777 | 72445-73040 | 4222-8344 | 5190-5574 | 14015-14399 | 68297-68681 | log (1.62x to 3.26x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 354693 | - | - | - | 84832 | page LSM (4.18x to 4.18x over B+tree) |
| 2 | 90373 | - | - | - | - | 330678 | - | - | - | 79224 | too close (page LSM by 1.14x to 1.14x over page) |
| 4 | 86216 | - | - | - | - | 315874 | - | - | 24424 | 76450 | page log (3.13x to 3.13x over page LSM) |
| 8 | 83962 | - | 72508 | - | - | 306706 | 70548 | - | 20909 | 74455 | page log (3.37x to 3.37x over log) |
| 14.85 | 82164 | - | 25842 | 15750 | 20124 | 301219 | 23882 | 12674 | 18862 | 72724 | LSM (1.49x to 1.49x over page log) |
| 16 | 81922 | - | 8176-25760 | 14980 | 19032 | 300667 | 6216-23800 | 12234 | 18616 | 72474 | undecided (LSM by up to 1.95x, log by up to 1.97x) |
| 27.69 | 80192 | - | 8176-25760 | 5984-14776 | 13750 | 296886 | 6216-23800 | 9452 | 16751 | 70813 | undecided (LSM by up to 3.98x, log by up to 1.52x) |
| 32 | 79815 | - | 8176-25760 | 5984-14776 | 12744 | 295810 | 6216-23800 | 8946 | 16348 | 70434 | undecided (LSM by up to 3.98x, log by up to 1.44x) |
| 64 | 77672-78951 | - | 8176-25760 | 5984-14776 | 8826 | 292415 | 6216-23800 | 5984-7263 | 14095-15374 | 68337-69616 | undecided (LSM by up to 3.98x, log by up to 1.17x) |
| 128 | 77672-78951 | 2506-11298 | 8176-25760 | 5984-14776 | 5984-7940 | 289739-291724 | 6216-23800 | 5984-7263 | 14095-15374 | 68337-69616 | undecided (LSM by up to 1.89x, log by up to 2.90x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 354693 | - | - | - | 84832 | page LSM (4.18x to 4.18x over B+tree) |
| 2 | 90373 | - | - | - | - | 330678 | - | - | - | 79224 | too close (page LSM by 1.14x to 1.14x over page) |
| 4 | 86216 | - | - | - | - | 315874 | - | - | 24424 | 76450 | page log (3.13x to 3.13x over page LSM) |
| 8 | 83962 | - | 58435 | - | - | 306706 | 56475 | - | 20909 | 74455 | page log (2.70x to 2.70x over log) |
| 14.85 | 82164 | - | 21978 | 15750 | 20124 | 301219 | 20018 | 12674 | 18862 | 72724 | LSM (1.49x to 1.49x over page log) |
| 16 | 81922 | - | 8176-21913 | 14980 | 19032 | 300667 | 6216-19954 | 12234 | 18616 | 72474 | undecided (LSM by up to 1.63x, log by up to 1.97x) |
| 27.69 | 80192 | - | 8176-21913 | 5984-14776 | 13750 | 296886 | 6216-19954 | 9452 | 16751 | 70813 | undecided (LSM by up to 3.33x, log by up to 1.52x) |
| 32 | 79815 | - | 8176-21913 | 5984-14776 | 12744 | 295810 | 6216-19954 | 8946 | 16348 | 70434 | undecided (LSM by up to 3.33x, log by up to 1.44x) |
| 64 | 77672-78951 | - | 8176-21913 | 5984-14776 | 8826 | 292415 | 6216-19954 | 5984-7263 | 14095-15374 | 68337-69616 | undecided (LSM by up to 3.33x, log by up to 1.17x) |
| 128 | 77672-78951 | 2506-7452 | 8176-21913 | 5984-14776 | 5984-7940 | 289739-291724 | 6216-19954 | 5984-7263 | 14095-15374 | 68337-69616 | undecided (LSM by up to 1.25x, log by up to 2.90x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 854 | 0 | 4823 | 830167 | 0 | 5.00 | 186 |
| 4 | B+tree, copy-on-write | 2614 | 0 | 18101 | 0 | 0 | 2.00 | 337 |
| 4 | page log, GC at 1/2 | 1033 | 0 | 0 | 31800787 | 31800787 | 2.00 | 224 |
| 4 | page LSM | 811 | 0 | 0 | 162462165 | 146499661 | 1.00 | 178 |
| 8 | page | 629 | 0 | 4823 | 830167 | 0 | 5.00 | 139 |
| 8 | log, index on disk (1.5 B) | 3496 | 3217 | 0 | 12126264 | 12105258 | 1.00 | 79 |
| 8 | B+tree, copy-on-write | 1697 | 0 | 18101 | 0 | 0 | 2.00 | 215 |
| 8 | log, index on disk, GC at 2/3 | 3496 | 3217 | 0 | 7666570 | 7645564 | 1.00 | 79 |
| 8 | page log, GC at 1/2 | 681 | 0 | 0 | 31800787 | 31800787 | 2.00 | 151 |
| 8 | page LSM | 612 | 0 | 0 | 162462165 | 146499661 | 1.00 | 135 |
| 16 | page | 425 | 0 | 4823 | 830167 | 0 | 5.00 | 84 |
| 16 | log, index on disk (1.5 B) | 1158 | 879 | 0 | 12126264 | 12105258 | 1.00 | 21 |
| 16 | LSM, node cache | 1007 | 0 | 0 | 11811886 | 10051140 | 1.00 | 21 |
| 16 | LSM, block cache | 1413 | 0 | 0 | 11811886 | 10051140 | 1.00 | 168 |
| 16 | B+tree, copy-on-write | 1093 | 0 | 18101 | 0 | 0 | 2.00 | 144 |
| 16 | log, index on disk, GC at 2/3 | 1158 | 879 | 0 | 7666570 | 7645564 | 1.00 | 21 |
| 16 | LSM, page-keyed | 733 | 0 | 0 | 11811886 | 10051140 | 1.00 | 138 |
| 16 | page log, GC at 1/2 | 452 | 0 | 0 | 31800787 | 31800787 | 2.00 | 91 |
| 16 | page LSM | 414 | 0 | 0 | 162462165 | 146499661 | 1.00 | 81 |
| 32 | page | 214 | 0 | 4823 | 830167 | 0 | 5.00 | 31 |
| 32 | log, index on disk (1.5 B) | 1158 | 879 | 0 | 12126264 | 12105258 | 1.00 | 21 |
| 32 | LSM, node cache | 987 | 0 | 0 | 11811886 | 10051140 | 1.00 | 21 |
| 32 | LSM, block cache | 784 | 0 | 0 | 11811886 | 10051140 | 1.00 | 85 |
| 32 | B+tree, copy-on-write | 607 | 0 | 18101 | 0 | 0 | 2.00 | 74 |
| 32 | log, index on disk, GC at 2/3 | 1158 | 879 | 0 | 7666570 | 7645564 | 1.00 | 21 |
| 32 | LSM, page-keyed | 404 | 0 | 0 | 11811886 | 10051140 | 1.00 | 52 |
| 32 | page log, GC at 1/2 | 225 | 0 | 0 | 31800787 | 31800787 | 2.00 | 33 |
| 32 | page LSM | 210 | 0 | 0 | 162462165 | 146499661 | 1.00 | 30 |

### Sensitivity to the index engine's write amplification, 16 GB

The log store with its index on disk (1.5 B per record in DRAM), IO cost per block. 13.0 is the amplification the tables above use, the model's 15.6 scaled by the RocksDB measurement.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 13.0 | 6182-11457 | 6182-10303 | 8176-25760 | 8176-21913 |
| 15.6 | 6981-12256 | 6981-11102 | 8975-26559 | 8975-22712 |
| 5.0 | 3746-9021 | 3746-7867 | 5739-23324 | 5739-19477 |
| 30.0 | 11342-16617 | 11342-15463 | 13335-30920 | 13335-27073 |
| page store, for comparison | 20870 | 20870 | 81922 | 81922 |

### Sensitivity to the record engine's write amplification, 16 GB

The LSM of records, IO cost per block. 5.7 is the amplification the tables above use, the model's 5.7 scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 5.7, node cache | 7889 | 7889 | 14980 | 14980 |
| 5.7, block cache | 9104 | 9104 | 19032 | 19032 |
| 5.7, page-keyed | 7065 | 7065 | 12234 | 12234 |
| 5.0, node cache | 7341 | 7341 | 14432 | 14432 |
| 5.0, block cache | 8556 | 8556 | 18484 | 18484 |
| 5.0, page-keyed | 6517 | 6517 | 11686 | 11686 |
| 15.0, node cache | 15078 | 15078 | 22170 | 22170 |
| 15.0, block cache | 16294 | 16294 | 26221 | 26221 |
| 15.0, page-keyed | 14254 | 14254 | 19423 | 19423 |
| 30.0, node cache | 26685 | 26685 | 33776 | 33776 |
| 30.0, block cache | 27900 | 27900 | 37828 | 37828 |
| 30.0, page-keyed | 25861 | 25861 | 31030 | 31030 |

### Sensitivity to garbage collection, 16 GB

The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.

| g | store | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes | space |
|---|---|---|---|---|---|---|
| 0.50 | log, index on disk (index amplification 13.0) | 6182-11457 | 6182-10303 | 8176-25760 | 8176-21913 | 2.0x |
| 0.50 | page log | 15371 | 15371 | 18616 | 18616 | 2.0x |
| 0.67 | log, index on disk (index amplification 10.2) | 4222-9498 | 4222-8344 | 6216-23800 | 6216-19954 | 3.0x |
| 0.67 | page log | 11878 | 11878 | 15122 | 15122 | 3.0x |
| 0.80 | log, index on disk (index amplification 7.3) | 3116-8391 | 3116-7237 | 5109-22694 | 5109-18847 | 5.0x |
| 0.80 | page log | 10131 | 10131 | 13376 | 13376 | 5.0x |

