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
| paged-pos(res=2) | 0.114 | 414.7 | 402.7 | 3189.9 | 258.0 | 3.0 | 2 | 15166 | 0 |
| append-readback(res=12) | 0.436 | 0.0 | 3057.0 | 3189.9 | 258.0 | 1.0 | 363 | 0 | 0 |
| btree-cow(res=2) | 0.000 | 2771.6 | 4177.6 | 3189.9 | 258.0 | 2.0 | 1 | 603674 | 0 |

`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens
inside libmdbx and is invisible to these counters, so its seq/rand columns are
zero by construction and a modeled cost would be meaningless. It is compared
separately on real wall-clock and real on-disk size.


## Read pricing sensitivity (nvme)

A per-IO read price ignores transfer size; a page store moves 4 KB per hop
while a log store moves one sector. Both prices shown.

| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |
|---|---|---|---|
| paged-pos(res=2) | 2957.2 | 2957.2 | 1610.9 |
| append-readback(res=12) | 9572.6 | 5560.3 | 1528.5 |
| btree-cow(res=2) | 23660.2 | 23660.2 | 16710.5 |

## Modeled cost, profile `nvme-optimistic`

Units: one sequential 4KB write = 1. rand write 4x, rand read 3x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 2957.2 | 1.00x | 2 |
| append-readback(res=12) | 9572.6 | 3.24x | 363 |
| btree-cow(res=2) | 23660.2 | 8.00x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Modeled cost, profile `commodity-ssd`

Units: one sequential 4KB write = 1. rand write 16x, rand read 10x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 10876.1 | 1.00x | 2 |
| append-readback(res=12) | 31556.6 | 2.90x | 363 |
| btree-cow(res=2) | 86245.4 | 7.93x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Real-IO comparison (wall clock, on-disk)

| backend | commit wall ms/blk | disk MB | basis |
|---|---|---|---|
| paged-pos(res=2) | 10.035 | 15166 | real WAL write+fsync; page IO counter-modeled |
| append-readback(res=12) | 3.164 | 0 | counter-modeled |
| btree-cow(res=2) | 4.871 | 603674 | counter-modeled |

## Backend detail

- paged-pos(res=2): 3882597 stored pages, mean occupancy 6.1/126 slots (4.8%), WAL 8030 MB
  - elision: 14033884 page writes avoided, 0 read fallbacks
  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: [3480392, 315970, 48545, 8310, 5335, 24045]
- append-readback(res=12): live 1850 MB, amortized compaction reclaim 13816 MB, live records 24738851, live branches 12373966, idx_branches 12373966, idx_leaves 12364885, write-path RMW reads 179144822 (sibling-dirty 12392473), uncharged displaced-leaf splits 7121, dram bytes 381253427
- btree-cow MODELED: 33 entries per 4 KB leaf page (96 B mean entry at fill 0.8), fanout 60, height 6, pages per level [151964987, 2532750, 42213, 704, 12, 1] (633.0 GB)
- btree-cow: resident leaf pages 396 (1.6 MB); records created 25196618, rewritten 199713777, deleted 468144, leaf changes 19985618
- btree-cow, measured window: leaf pages copied 1351171, interior pages copied 1416168, freelist pages 3282; key walks 202141, prefix hits 2995992, fallback walks 15980, memo hits 249729

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
- SAMPLED: only accounts and contracts whose address hash starts with the 2 bits of part 3 are replayed, one key-space subtree in 4. Every figure below is scaled to the whole: per-block counts by 4, cache sizes and DRAM by 4, the full state and the index buffer taken at 1/4 for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.
- window: 1000 measured blocks after 49000 warm-up blocks; 50000 blocks available from 24000000
- full state stood in for: 334563311 accounts, 1438571749 storage slots, 399723913 code chunks = 626855571 leaves, 1253711142 records. Storage contracts: 25000000 (ASSUMED). An untouched contract is taken to hold 57 slots, the mean; real sizes are heavy-tailed.
- replay built from: 3910702 account changes (486184 creations) and 4977034 slot changes; 1137767 keys in the starting state; stand-ins: 6322190 beside account paths, 174934 beside contract buckets, 2976806 inside contracts' storage. 1907 of 20026 contracts written in the storage zone have a fetched slot count, covering 92.8% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes 1185539 distinct slots there in 978429 stems (1.21 per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.
- path length, account zone: 490.1 leaves written per block, 32.3 branch records above each on average, 40 at most
- path length, storage zone: 206.4 leaves written per block, 40.1 branch records above each on average, 53 at most
- execution reads, per block: 866.0 of existing keys and 165.9 of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.
- measured, log store, per block: 12729.2 records written, 0.88 MB appended by the commit; with no cache 12227.8 reads, of which 866.0 are execution reads, 557.4 fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; 25209.9 index updates, 10.7 deleted branches to mark dead, 28.5 displaced leaves
- measured, page store, per block: 1658.9 merkle pages written; with no cache 3205.6 reads, of which 1610.9 are merkle pages and the rest value-store leaf pages (execution reads included); 562.8 value-store leaf pages and 501.0 bottom branch nodes rewritten, 10.64 MB held by one commit
- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at 88 B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.
- fixed DRAM at full-state size: log store with its index in DRAM 71.9 GB (15.4 B per record); with its index on disk 7.6 GB or 20.4 GB (1.5 or 4.25 B per record, plus a 256 MB write buffer and 298 MB of page pointers, all ASSUMED); page store 0.98 GB (0.80 GB of resident bottom branch nodes over 74462908 value-store leaf pages, 0.14 GB of bucket map, ASSUMED, and one commit's pages).
- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, 5.19 GB of records for the log store and 34.33 GB of pages for the page store. A replay of 50000 blocks cannot tell further; a longer one narrows the range.
- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over 75 GB of 16-byte entries, a 256 MB write buffer, levels growing by 10x down to one that holds every entry. How much each level absorbs is MEASURED from the 163823229 index updates of the replay (13106 per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The 1.0 relocations per update are taken never to be absorbed. Write amplification 12.8 per index update (205 B written and read): the model gives 15.4, and a real RocksDB fed this stream as the index sees it wrote 0.83 of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); 15 for the assumed engine before. That costs 2267 per block on `nvme-optimistic` and 2267 on `commodity-ssd`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 4.194e6 | 268 | yes | 0.604 | 3.01 |
  | L1 | 1.254e7 | 835 | yes | 0.580 | 6.38 |
  | L2 | 1.254e8 | 8914 | yes | 0.543 | 5.97 |
  | L3 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a 256 MB write buffer and levels growing by 10x down to one holding all 5014844568 records. An entry is 81 B, the mean record of this replay's write stream (72.7 B, its 4 B log header standing in for the varint lengths) plus 8 B of sequence number and type; prefix compression is not modeled. Updates are the 12759.6 records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification 4.9 per record update (400 B written and read back by compaction, plus one WAL copy of each update), the model's 4.9 scaled by 1.01, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against 12.8 for the index entries above; that costs 2241 per block on `nvme-optimistic` and 2241 on `commodity-ssd`. Fixed DRAM 8.89 GB: 1.85 B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at 97 B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 8.311e5 | 298 | yes | 0.213 | 0.75 |
  | L1 | 1.254e6 | 506 | yes | 0.189 | 2.08 |
  | L2 | 1.254e7 | 10128 | yes | 0.094 | 1.04 |
  | L3 | 1.254e8 | 101552 | no: longest window's share used | 0.094 | 1.04 |
  | L4 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, 33 per leaf page (96 B mean entry at fill 0.8, ASSUMED), 60 children per interior page (ASSUMED), height 6 over 151964987 leaf pages (633 GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM (0.002 GB). The page map is static: no page splits or merges over the replay.
- measured, B+tree, per block: 11086.5 pages rewritten copy-on-write; with no cache 5886.7 leaf-page reads and 10823.9 interior-page reads, execution reads included; 5404.7 leaf pages and 5664.7 interior pages copied, 43.31 MB held by one commit, 2 fsyncs
- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM (2.22 GB: 16 B per page over 148925816 pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: 2221.7 pages appended per block, each superseding one; space up to 2x the live pages.
- page LSM, MODELED from the page store's run: the same pages as 4112 B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the 28516028 page writes of the replay (2281 per block, warm-up included). page write amplification 7.9 per page update (32 KB written and read back by compaction per page written, plus one WAL copy).

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 1.632e4 | 17 | yes | 0.425 | 1.82 |
  | L1 | 3.723e4 | 50 | yes | 0.324 | 3.56 |
  | L2 | 3.723e5 | 1137 | yes | 0.144 | 1.58 |
  | L3 | 3.723e6 | 19022 | no: longest window's share used | 0.086 | 0.94 |
  | L4 | 3.723e7 | - | - | 0.000 | 0.00 |

- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations 6225.1 per block at g = 2/3 (12450.3 at g = 1/2), index write amplification 9.9 (12.8 at g = 1/2), compaction reading 1.29 MB and rewriting 0.43 MB per block (1.72 and 0.86 at g = 1/2); space up to 3x the live records instead of 2x.
- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves (28.5 per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the 834.7 elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.
- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified.
- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name.
- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by 1.3x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.

### Reads (write path and execution) per block against cache size, exact range only

Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.

| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |
|---|---|---|---|---|---|---|
| 1/64 | 907560 | 0.081 | 3152.1 | 140060 | 0.536 | 697.0 |
| 1/32 | 1815120 | 0.162 | 2426.2 | 280124 | 1.073 | 523.3 |
| 1/16 | 3630244 | 0.325 | 1843.6 | 560248 | 2.146 | 390.3 |
| 1/8 | 7260492 | 0.649 | 1451.6 | 1120500 | 4.291 | 303.4 |
| 1/4 | 14520984 | 1.298 | 1118.7 | 2241000 | 8.582 | 248.5 |
| 1/2 | 29041972 | 2.597 | 854.8 | 4482004 | 17.164 | 183.9 |
| 1/1 | 58083948 | 5.193 | 620.7 | 8964012 | 34.329 | 119.7 |

With no cache at all: 12227.8 reads per block for the log store, 3205.6 for the page store. Of the log store's, 618.1 are of records untouched since the replay began; of the page store's, 119.0.

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 17886 | - | - | - | - | 52990 | - | - | - | 36155 | page (2.02x to 2.02x over page LSM) |
| 2 | 12733 | - | - | - | - | 50071 | - | - | - | 35362 | page (2.78x to 2.78x over page LSM) |
| 4 | 12147 | - | - | - | - | 48185 | - | - | 9333 | 34951 | page log (1.30x to 1.30x over page) |
| 8 | 11921 | - | 13154 | - | - | 46813 | 12014 | - | 8881 | 34762 | page log (1.34x to 1.34x over page) |
| 12.75 | 11766 | - | 7014 | 4771 | 5027 | 46197 | 5874 | 3537 | 8711 | 34616 | LSM (1.66x to 1.66x over log) |
| 16 | 11716 | - | 3290-6999 | 2598-4452 | 4412 | 45963 | 2150-5859 | 3391 | 8650 | 34566 | undecided (LSM by up to 2.26x, log by up to 1.58x) |
| 25.59 | 11582 | - | 3290-6999 | 2598-4452 | 3844 | 45488 | 2150-5859 | 3158 | 8511 | 34433 | undecided (LSM by up to 2.26x, log by up to 1.47x) |
| 32 | 11514 | - | 3290-6999 | 2598-4452 | 3617 | 45337 | 2150-5859 | 3075 | 8441 | 34367 | undecided (LSM by up to 2.26x, log by up to 1.43x) |
| 64 | 11123-11480 | - | 3290-6999 | 2598-4452 | 3150 | 44386-44933 | 2150-5859 | 2598-2955 | 8038-8395 | 33981-34338 | undecided (LSM by up to 2.26x, log by up to 1.37x) |
| 128 | 11123-11480 | 905-2759 | 3290-6999 | 2598-4452 | 2598-3137 | 44386-44933 | 2150-5859 | 2598-2955 | 8038-8395 | 33981-34338 | undecided (log by up to 3.26x, LSM by up to 1.06x) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 17886 | - | - | - | - | 52990 | - | - | - | 36155 | page (2.02x to 2.02x over page LSM) |
| 2 | 12733 | - | - | - | - | 50071 | - | - | - | 35362 | page (2.78x to 2.78x over page LSM) |
| 4 | 12147 | - | - | - | - | 48185 | - | - | 9333 | 34951 | page log (1.30x to 1.30x over page) |
| 8 | 11921 | - | 10996 | - | - | 46813 | 9857 | - | 8881 | 34762 | too close (page log by 1.11x to 1.11x over log) |
| 12.75 | 11766 | - | 6199 | 4771 | 5027 | 46197 | 5060 | 3537 | 8711 | 34616 | LSM (1.43x to 1.43x over log) |
| 16 | 11716 | - | 3290-6187 | 2598-4452 | 4412 | 45963 | 2150-5048 | 3391 | 8650 | 34566 | undecided (LSM by up to 1.94x, log by up to 1.58x) |
| 25.59 | 11582 | - | 3290-6187 | 2598-4452 | 3844 | 45488 | 2150-5048 | 3158 | 8511 | 34433 | undecided (LSM by up to 1.94x, log by up to 1.47x) |
| 32 | 11514 | - | 3290-6187 | 2598-4452 | 3617 | 45337 | 2150-5048 | 3075 | 8441 | 34367 | undecided (LSM by up to 1.94x, log by up to 1.43x) |
| 64 | 11123-11480 | - | 3290-6187 | 2598-4452 | 3150 | 44386-44933 | 2150-5048 | 2598-2955 | 8038-8395 | 33981-34338 | undecided (LSM by up to 1.94x, log by up to 1.37x) |
| 128 | 11123-11480 | 905-1948 | 3290-6187 | 2598-4452 | 2598-3137 | 44386-44933 | 2150-5048 | 2598-2955 | 8038-8395 | 33981-34338 | log (1.33x to 3.26x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 66587 | - | - | - | - | 206183 | - | - | - | 41266 | page LSM (1.61x to 1.61x over page) |
| 2 | 49409 | - | - | - | - | 196454 | - | - | - | 38624 | too close (page LSM by 1.28x to 1.28x over page) |
| 4 | 47456 | - | - | - | - | 190167 | - | - | 12433 | 37255 | page log (3.00x to 3.00x over page LSM) |
| 8 | 46703 | - | 36684 | - | - | 185593 | 35544 | - | 10929 | 36625 | page log (3.25x to 3.25x over log) |
| 12.75 | 46185 | - | 16217 | 10080 | 10935 | 183542 | 15078 | 5969 | 10361 | 36136 | LSM (1.74x to 1.74x over page log) |
| 16 | 46020 | - | 3804-16166 | 2837-9018 | 8885 | 182760 | 2664-15027 | 5481 | 10156 | 35969 | undecided (LSM by up to 5.30x, log by up to 2.06x) |
| 25.59 | 45574 | - | 3804-16166 | 2837-9018 | 6990 | 181177 | 2664-15027 | 4704 | 9693 | 35529 | undecided (LSM by up to 5.30x, log by up to 1.77x) |
| 32 | 45346 | - | 3804-16166 | 2837-9018 | 6235 | 180675 | 2664-15027 | 4429 | 9462 | 35309 | undecided (LSM by up to 5.30x, log by up to 1.66x) |
| 64 | 44044-45234 | - | 3804-16166 | 2837-9018 | 4678 | 177504-179329 | 2664-15027 | 2837-4027 | 8118-9308 | 34021-35211 | undecided (LSM by up to 5.30x, log by up to 1.51x) |
| 128 | 44044-45234 | 1144-7326 | 3804-16166 | 2837-9018 | 2837-4636 | 177504-179329 | 2664-15027 | 2837-4027 | 8118-9308 | 34021-35211 | undecided (LSM by up to 2.58x, log by up to 3.52x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 66587 | - | - | - | - | 206183 | - | - | - | 41266 | page LSM (1.61x to 1.61x over page) |
| 2 | 49409 | - | - | - | - | 196454 | - | - | - | 38624 | too close (page LSM by 1.28x to 1.28x over page) |
| 4 | 47456 | - | - | - | - | 190167 | - | - | 12433 | 37255 | page log (3.00x to 3.00x over page LSM) |
| 8 | 46703 | - | 29491 | - | - | 185593 | 28352 | - | 10929 | 36625 | page log (2.59x to 2.59x over log) |
| 12.75 | 46185 | - | 13502 | 10080 | 10935 | 183542 | 12362 | 5969 | 10361 | 36136 | LSM (1.74x to 1.74x over page log) |
| 16 | 46020 | - | 3804-13462 | 2837-9018 | 8885 | 182760 | 2664-12322 | 5481 | 10156 | 35969 | undecided (LSM by up to 4.34x, log by up to 2.06x) |
| 25.59 | 45574 | - | 3804-13462 | 2837-9018 | 6990 | 181177 | 2664-12322 | 4704 | 9693 | 35529 | undecided (LSM by up to 4.34x, log by up to 1.77x) |
| 32 | 45346 | - | 3804-13462 | 2837-9018 | 6235 | 180675 | 2664-12322 | 4429 | 9462 | 35309 | undecided (LSM by up to 4.34x, log by up to 1.66x) |
| 64 | 44044-45234 | - | 3804-13462 | 2837-9018 | 4678 | 177504-179329 | 2664-12322 | 2837-4027 | 8118-9308 | 34021-35211 | undecided (LSM by up to 4.34x, log by up to 1.51x) |
| 128 | 44044-45234 | 1144-4621 | 3804-13462 | 2837-9018 | 2837-4636 | 177504-179329 | 2664-12322 | 2837-4027 | 8118-9308 | 34021-35211 | undecided (LSM by up to 1.63x, log by up to 3.52x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 341 | 0 | 2727 | 476057 | 0 | 5.00 | 40 |
| 4 | B+tree, copy-on-write | 1266 | 0 | 11086 | 0 | 0 | 2.00 | 100 |
| 4 | page log, GC at 1/2 | 431 | 0 | 0 | 18200560 | 18200560 | 2.00 | 56 |
| 4 | page LSM | 323 | 0 | 0 | 81340958 | 72205130 | 1.00 | 38 |
| 8 | page | 266 | 0 | 2727 | 476057 | 0 | 5.00 | 27 |
| 8 | log, index on disk (1.5 B) | 1712 | 1644 | 0 | 6988576 | 6966110 | 1.00 | 52 |
| 8 | B+tree, copy-on-write | 809 | 0 | 11086 | 0 | 0 | 2.00 | 55 |
| 8 | log, index on disk, GC at 2/3 | 1712 | 1644 | 0 | 4395404 | 4372937 | 1.00 | 52 |
| 8 | page log, GC at 1/2 | 281 | 0 | 0 | 18200560 | 18200560 | 2.00 | 30 |
| 8 | page LSM | 260 | 0 | 0 | 81340958 | 72205130 | 1.00 | 27 |
| 16 | page | 198 | 0 | 2727 | 476057 | 0 | 5.00 | 17 |
| 16 | log, index on disk (1.5 B) | 686 | 618 | 0 | 6988576 | 6966110 | 1.00 | 14 |
| 16 | LSM, node cache | 647 | 0 | 0 | 6129543 | 5099206 | 1.00 | 14 |
| 16 | LSM, block cache | 633 | 0 | 0 | 6129543 | 5099206 | 1.00 | 38 |
| 16 | B+tree, copy-on-write | 526 | 0 | 11086 | 0 | 0 | 2.00 | 33 |
| 16 | log, index on disk, GC at 2/3 | 686 | 618 | 0 | 4395404 | 4372937 | 1.00 | 14 |
| 16 | LSM, page-keyed | 293 | 0 | 0 | 6129543 | 5099206 | 1.00 | 27 |
| 16 | page log, GC at 1/2 | 204 | 0 | 0 | 18200560 | 18200560 | 2.00 | 18 |
| 16 | page LSM | 195 | 0 | 0 | 81340958 | 72205130 | 1.00 | 17 |
| 32 | page | 130 | 0 | 2727 | 476057 | 0 | 5.00 | 9 |
| 32 | log, index on disk (1.5 B) | 686 | 618 | 0 | 6988576 | 6966110 | 1.00 | 14 |
| 32 | LSM, node cache | 647 | 0 | 0 | 6129543 | 5099206 | 1.00 | 14 |
| 32 | LSM, block cache | 368 | 0 | 0 | 6129543 | 5099206 | 1.00 | 19 |
| 32 | B+tree, copy-on-write | 317 | 0 | 11086 | 0 | 0 | 2.00 | 18 |
| 32 | log, index on disk, GC at 2/3 | 686 | 618 | 0 | 4395404 | 4372937 | 1.00 | 14 |
| 32 | LSM, page-keyed | 188 | 0 | 0 | 6129543 | 5099206 | 1.00 | 13 |
| 32 | page log, GC at 1/2 | 134 | 0 | 0 | 18200560 | 18200560 | 2.00 | 10 |
| 32 | page LSM | 129 | 0 | 0 | 81340958 | 72205130 | 1.00 | 9 |

### Sensitivity to the index engine's write amplification, 16 GB

The log store with its index on disk (1.5 B per record in DRAM), IO cost per block. 12.8 is the amplification the tables above use, the model's 15.4 scaled by the RocksDB measurement.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 12.8 | 3290-6999 | 3290-6187 | 3804-16166 | 3804-13462 |
| 15.4 | 3746-7455 | 3746-6644 | 4260-16622 | 4260-13918 |
| 5.0 | 1909-5618 | 1909-4806 | 2423-14785 | 2423-12081 |
| 30.0 | 6340-10049 | 6340-9238 | 6854-19217 | 6854-16512 |
| page store, for comparison | 11716 | 11716 | 46020 | 46020 |

### Sensitivity to the record engine's write amplification, 16 GB

The LSM of records, IO cost per block. 4.9 is the amplification the tables above use, the model's 4.9 scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 4.9, node cache | 2598-4452 | 2598-4452 | 2837-9018 | 2837-9018 |
| 4.9, block cache | 4412 | 4412 | 8885 | 8885 |
| 4.9, page-keyed | 3391 | 3391 | 5481 | 5481 |
| 5.0, node cache | 2621-4475 | 2621-4475 | 2860-9042 | 2860-9042 |
| 5.0, block cache | 4435 | 4435 | 8908 | 8908 |
| 5.0, page-keyed | 3414 | 3414 | 5504 | 5504 |
| 15.0, node cache | 7149-9003 | 7149-9003 | 7388-13569 | 7388-13569 |
| 15.0, block cache | 8963 | 8963 | 13436 | 13436 |
| 15.0, page-keyed | 7942 | 7942 | 10032 | 10032 |
| 30.0, node cache | 13941-15795 | 13941-15795 | 14180-20361 | 14180-20361 |
| 30.0, block cache | 15755 | 15755 | 20228 | 20228 |
| 30.0, page-keyed | 14734 | 14734 | 16824 | 16824 |

### Sensitivity to garbage collection, 16 GB

The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.

| g | store | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes | space |
|---|---|---|---|---|---|---|
| 0.50 | log, index on disk (index amplification 12.8) | 3290-6999 | 3290-6187 | 3804-16166 | 3804-13462 | 2.0x |
| 0.50 | page log | 8650 | 8650 | 10156 | 10156 | 2.0x |
| 0.67 | log, index on disk (index amplification 9.9) | 2150-5859 | 2150-5048 | 2664-15027 | 2664-12322 | 3.0x |
| 0.67 | page log | 6650 | 6650 | 8156 | 8156 | 3.0x |
| 0.80 | log, index on disk (index amplification 7.0) | 1508-5217 | 1508-4406 | 2022-14384 | 2022-11680 | 5.0x |
| 0.80 | page log | 5650 | 5650 | 7157 | 7157 | 5.0x |

