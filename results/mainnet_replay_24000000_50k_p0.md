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
| paged-pos(res=2) | 0.204 | 744.7 | 732.7 | 5562.6 | 346.6 | 3.0 | 2 | 16956 | 0 |
| append-readback(res=12) | 0.758 | 0.0 | 5391.9 | 5562.6 | 346.6 | 1.0 | 451 | 0 | 0 |
| btree-cow(res=2) | 0.000 | 4537.9 | 6468.8 | 5562.6 | 346.6 | 2.0 | 1 | 603674 | 0 |

`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens
inside libmdbx and is invisible to these counters, so its seq/rand columns are
zero by construction and a modeled cost would be meaningless. It is compared
separately on real wall-clock and real on-disk size.


## Read pricing sensitivity (nvme)

A per-IO read price ignores transfer size; a page store moves 4 KB per hop
while a log store moves one sector. Both prices shown.

| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |
|---|---|---|---|
| paged-pos(res=2) | 5290.1 | 5290.1 | 2930.8 |
| append-readback(res=12) | 16769.7 | 9692.8 | 2696.0 |
| btree-cow(res=2) | 37599.0 | 37599.0 | 25875.2 |

## Modeled cost, profile `nvme-optimistic`

Units: one sequential 4KB write = 1. rand write 4x, rand read 3x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 5290.1 | 1.00x | 2 |
| append-readback(res=12) | 16769.7 | 3.17x | 451 |
| btree-cow(res=2) | 37599.0 | 7.11x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Modeled cost, profile `commodity-ssd`

Units: one sequential 4KB write = 1. rand write 16x, rand read 10x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 19478.8 | 1.00x | 2 |
| append-readback(res=12) | 55229.9 | 2.84x | 451 |
| btree-cow(res=2) | 137418.2 | 7.05x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Real-IO comparison (wall clock, on-disk)

| backend | commit wall ms/blk | disk MB | basis |
|---|---|---|---|
| paged-pos(res=2) | 11.068 | 16956 | real WAL write+fsync; page IO counter-modeled |
| append-readback(res=12) | 5.776 | 0 | counter-modeled |
| btree-cow(res=2) | 9.972 | 603674 | counter-modeled |

## Backend detail

- paged-pos(res=2): 4340834 stored pages, mean occupancy 6.7/126 slots (5.3%), WAL 12800 MB
  - elision: 16479078 page writes avoided, 0 read fallbacks
  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: [3739693, 495878, 65022, 7286, 6489, 26466]
- append-readback(res=12): live 2344 MB, amortized compaction reclaim 21754 MB, live records 30706604, live branches 15364005, idx_branches 15364005, idx_leaves 15342599, write-path RMW reads 288767262 (sibling-dirty 20601961), uncharged displaced-leaf splits 21055, dram bytes 473156958
- btree-cow MODELED: 33 entries per 4 KB leaf page (96 B mean entry at fill 0.8), fanout 60, height 6, pages per level [151964987, 2532750, 42213, 704, 12, 1] (633.0 GB)
- btree-cow: resident leaf pages 394 (1.6 MB); records created 32729210, rewritten 313292511, deleted 2045188, leaf changes 28296500
- btree-cow, measured window: leaf pages copied 2361557, interior pages copied 2170360, freelist pages 5033; key walks 365445, prefix hits 5231507, fallback walks 21991, memo hits 312259

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
- SAMPLED: only accounts and contracts whose address hash starts with the 2 bits of part 0 are replayed, one key-space subtree in 4. Every figure below is scaled to the whole: per-block counts by 4, cache sizes and DRAM by 4, the full state and the index buffer taken at 1/4 for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.
- window: 1000 measured blocks after 49000 warm-up blocks; 50000 blocks available from 24000000
- full state stood in for: 334563311 accounts, 1438571749 storage slots, 399723913 code chunks = 626855571 leaves, 1253711142 records. Storage contracts: 25000000 (ASSUMED). An untouched contract is taken to hold 57 slots, the mean; real sizes are heavy-tailed.
- replay built from: 3777580 account changes (486173 creations) and 10464043 slot changes; 1353746 keys in the starting state; stand-ins: 6322960 beside account paths, 174584 beside contract buckets, 5717413 inside contracts' storage. 1897 of 20044 contracts written in the storage zone have a fetched slot count, covering 96.4% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes 1930325 distinct slots there in 1707379 stems (1.13 per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.
- path length, account zone: 510.5 leaves written per block, 32.4 branch records above each on average, 39 at most
- path length, storage zone: 617.9 leaves written per block, 46.3 branch records above each on average, 54 at most
- execution reads, per block: 1013.9 of existing keys and 372.6 of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.
- measured, log store, per block: 22034.4 records written, 1.53 MB appended by the commit; with no cache 21567.6 reads, of which 1013.9 are execution reads, 1002.1 fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; 44024.9 index updates, 75.1 deleted branches to mark dead, 84.2 displaced leaves
- measured, page store, per block: 2978.9 merkle pages written; with no cache 5397.8 reads, of which 2930.8 are merkle pages and the rest value-store leaf pages (execution reads included); 1080.6 value-store leaf pages and 965.2 bottom branch nodes rewritten, 19.63 MB held by one commit
- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at 88 B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.
- fixed DRAM at full-state size: log store with its index in DRAM 71.9 GB (15.4 B per record); with its index on disk 7.6 GB or 20.4 GB (1.5 or 4.25 B per record, plus a 256 MB write buffer and 298 MB of page pointers, all ASSUMED); page store 1.02 GB (0.80 GB of resident bottom branch nodes over 74462908 value-store leaf pages, 0.14 GB of bucket map, ASSUMED, and one commit's pages).
- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, 6.35 GB of records for the log store and 45.84 GB of pages for the page store. A replay of 50000 blocks cannot tell further; a longer one narrows the range.
- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over 75 GB of 16-byte entries, a 256 MB write buffer, levels growing by 10x down to one that holds every entry. How much each level absorbs is MEASURED from the 265541327 index updates of the replay (21243 per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The 1.0 relocations per update are taken never to be absorbed. Write amplification 12.7 per index update (203 B written and read): the model gives 15.3, and a real RocksDB fed this stream as the index sees it wrote 0.83 of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); 15 for the assumed engine before. That costs 3932 per block on `nvme-optimistic` and 3932 on `commodity-ssd`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 4.194e6 | 166 | yes | 0.603 | 3.01 |
  | L1 | 1.254e7 | 518 | yes | 0.576 | 6.33 |
  | L2 | 1.254e8 | 5537 | yes | 0.539 | 5.93 |
  | L3 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a 256 MB write buffer and levels growing by 10x down to one holding all 5014844568 records. An entry is 81 B, the mean record of this replay's write stream (72.6 B, its 4 B log header standing in for the varint lengths) plus 8 B of sequence number and type; prefix compression is not modeled. Updates are the 22250.4 records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification 4.6 per record update (370 B written and read back by compaction, plus one WAL copy of each update), the model's 4.5 scaled by 1.01, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against 12.7 for the index entries above; that costs 3621 per block on `nvme-optimistic` and 3621 on `commodity-ssd`. Fixed DRAM 8.89 GB: 1.85 B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at 97 B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 8.323e5 | 190 | yes | 0.206 | 0.72 |
  | L1 | 1.254e6 | 329 | yes | 0.179 | 1.97 |
  | L2 | 1.254e7 | 7021 | no: longest window's share used | 0.084 | 0.92 |
  | L3 | 1.254e8 | 70210 | no: longest window's share used | 0.084 | 0.92 |
  | L4 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, 33 per leaf page (96 B mean entry at fill 0.8, ASSUMED), 60 children per interior page (ASSUMED), height 6 over 151964987 leaf pages (633 GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM (0.002 GB). The page map is static: no page splits or merges over the replay.
- measured, B+tree, per block: 18151.8 pages rewritten copy-on-write; with no cache 10261.5 leaf-page reads and 15613.7 interior-page reads, execution reads included; 9446.2 leaf pages and 8681.4 interior pages copied, 70.91 MB held by one commit, 2 fsyncs
- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM (2.22 GB: 16 B per page over 148925816 pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: 4059.5 pages appended per block, each superseding one; space up to 2x the live pages.
- page LSM, MODELED from the page store's run: the same pages as 4112 B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the 48253229 page writes of the replay (3860 per block, warm-up included). page write amplification 8.3 per page update (33 KB written and read back by compaction per page written, plus one WAL copy).

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 1.632e4 | 9 | yes | 0.487 | 2.08 |
  | L1 | 3.723e4 | 28 | yes | 0.350 | 3.85 |
  | L2 | 3.723e5 | 668 | yes | 0.144 | 1.59 |
  | L3 | 3.723e6 | 14412 | no: longest window's share used | 0.067 | 0.74 |
  | L4 | 3.723e7 | - | - | 0.000 | 0.00 |

- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations 10887.2 per block at g = 2/3 (21774.5 at g = 1/2), index write amplification 9.8 (12.7 at g = 1/2), compaction reading 2.26 MB and rewriting 0.75 MB per block (3.01 and 1.51 at g = 1/2); space up to 3x the live records instead of 2x.
- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves (84.2 per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the 1080.2 elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.
- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified.
- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name.
- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by 1.3x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.

### Reads (write path and execution) per block against cache size, exact range only

Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.

| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |
|---|---|---|---|---|---|---|
| 1/64 | 1109120 | 0.099 | 4813.8 | 187040 | 0.716 | 1208.6 |
| 1/32 | 2218240 | 0.198 | 3626.5 | 374084 | 1.433 | 876.8 |
| 1/16 | 4436484 | 0.397 | 2741.6 | 748172 | 2.865 | 637.4 |
| 1/8 | 8872968 | 0.793 | 2046.5 | 1496348 | 5.730 | 480.8 |
| 1/4 | 17745936 | 1.587 | 1578.0 | 2992696 | 11.461 | 380.5 |
| 1/2 | 35491872 | 3.173 | 1117.7 | 5985396 | 22.922 | 265.1 |
| 1/1 | 70983744 | 6.346 | 790.3 | 11970792 | 45.843 | 149.4 |

With no cache at all: 21567.6 reads per block for the log store, 5397.8 for the page store. Of the log store's, 786.8 are of records untouched since the replay began; of the page store's, 148.4.

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 90509 | - | - | - | 68923 | page LSM (1.31x to 1.31x over B+tree) |
| 2 | 23581 | - | - | - | - | 82678 | - | - | - | 67208 | page (2.85x to 2.85x over page LSM) |
| 4 | 22302 | - | - | - | - | 78913 | - | - | 17071 | 66377 | page log (1.31x to 1.31x over page) |
| 8 | 21770 | - | 21691 | - | - | 76590 | 19694 | - | 16099 | 65946 | too close (page log by 1.22x to 1.22x over log) |
| 13.90 | 21509 | - | 10818 | 7029 | 7854 | 75319 | 8821 | 5855 | 15790 | 65694 | LSM (1.51x to 1.51x over log) |
| 16 | 21410 | - | 6076-10797 | 4331-6692 | 7257 | 75076 | 4079-8800 | 5672 | 15707 | 65597 | undecided (LSM by up to 2.03x, log by up to 1.39x) |
| 26.74 | 21156 | - | 6076-10797 | 4331-6692 | 6151 | 74315 | 4079-8800 | 5233 | 15411 | 65352 | undecided (LSM by up to 2.03x, log by up to 1.28x) |
| 32 | 21065 | - | 6076-10797 | 4331-6692 | 5874 | 74051 | 4079-8800 | 5122 | 15321 | 65262 | undecided (LSM by up to 2.03x, log by up to 1.26x) |
| 64 | 20423-20868 | - | 6076-10797 | 4331-6692 | 5104 | 73388 | 4079-8800 | 4331-4777 | 14654-15099 | 64628-65073 | undecided (LSM by up to 2.03x, log by up to 1.17x) |
| 128 | 20423-20868 | 1666-4026 | 6076-10797 | 4331-6692 | 4331-4967 | 72647-73294 | 4079-8800 | 4331-4777 | 14654-15099 | 64628-65073 | too close (log by 1.08x to 2.87x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 90509 | - | - | - | 68923 | page LSM (1.31x to 1.31x over B+tree) |
| 2 | 23581 | - | - | - | - | 82678 | - | - | - | 67208 | page (2.85x to 2.85x over page LSM) |
| 4 | 22302 | - | - | - | - | 78913 | - | - | 17071 | 66377 | page log (1.31x to 1.31x over page) |
| 8 | 21770 | - | 18275 | - | - | 76590 | 16278 | - | 16099 | 65946 | too close (page log by 1.01x to 1.01x over log) |
| 13.90 | 21509 | - | 9781 | 7029 | 7854 | 75319 | 7784 | 5855 | 15790 | 65694 | LSM (1.33x to 1.33x over log) |
| 16 | 21410 | - | 6076-9764 | 4331-6692 | 7257 | 75076 | 4079-7767 | 5672 | 15707 | 65597 | undecided (LSM by up to 1.79x, log by up to 1.39x) |
| 26.74 | 21156 | - | 6076-9764 | 4331-6692 | 6151 | 74315 | 4079-7767 | 5233 | 15411 | 65352 | undecided (LSM by up to 1.79x, log by up to 1.28x) |
| 32 | 21065 | - | 6076-9764 | 4331-6692 | 5874 | 74051 | 4079-7767 | 5122 | 15321 | 65262 | undecided (LSM by up to 1.79x, log by up to 1.26x) |
| 64 | 20423-20868 | - | 6076-9764 | 4331-6692 | 5104 | 73388 | 4079-7767 | 4331-4777 | 14654-15099 | 64628-65073 | undecided (LSM by up to 1.79x, log by up to 1.17x) |
| 128 | 20423-20868 | 1666-2994 | 6076-9764 | 4331-6692 | 4331-4967 | 72647-73294 | 4079-7767 | 4331-4777 | 14654-15099 | 64628-65073 | log (1.45x to 2.87x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 350087 | - | - | - | 78984 | page LSM (4.43x to 4.43x over B+tree) |
| 2 | 91492 | - | - | - | - | 323983 | - | - | - | 73268 | too close (page LSM by 1.25x to 1.25x over page) |
| 4 | 87231 | - | - | - | - | 311436 | - | - | 22792 | 70498 | page log (3.09x to 3.09x over page LSM) |
| 8 | 85458 | - | 59871 | - | - | 303691 | 57874 | - | 19552 | 69060 | page log (2.96x to 2.96x over log) |
| 13.90 | 84587 | - | 23627 | 13954 | 16704 | 299455 | 21630 | 10039 | 18521 | 68222 | LSM (1.84x to 1.84x over page log) |
| 16 | 84257 | - | 7821-23556 | 4961-12828 | 14713 | 298645 | 5824-21559 | 9430 | 18243 | 67899 | undecided (LSM by up to 4.35x, log by up to 1.62x) |
| 26.74 | 83409 | - | 7821-23556 | 4961-12828 | 11025 | 296107 | 5824-21559 | 7968 | 17258 | 67081 | undecided (LSM by up to 4.35x, log by up to 1.37x) |
| 32 | 83107 | - | 7821-23556 | 4961-12828 | 10104 | 295230 | 5824-21559 | 7598 | 16956 | 66780 | undecided (LSM by up to 4.35x, log by up to 1.30x) |
| 64 | 80967-82451 | - | 7821-23556 | 4961-12828 | 7535 | 293018 | 5824-21559 | 4961-6445 | 14734-16218 | 64668-66152 | undecided (LSM by up to 4.35x, log by up to 1.11x) |
| 128 | 80967-82451 | 2296-10163 | 7821-23556 | 4961-12828 | 4961-7079 | 290549-292703 | 5824-21559 | 4961-6445 | 14734-16218 | 64668-66152 | undecided (LSM by up to 2.05x, log by up to 2.81x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 350087 | - | - | - | 78984 | page LSM (4.43x to 4.43x over B+tree) |
| 2 | 91492 | - | - | - | - | 323983 | - | - | - | 73268 | too close (page LSM by 1.25x to 1.25x over page) |
| 4 | 87231 | - | - | - | - | 311436 | - | - | 22792 | 70498 | page log (3.09x to 3.09x over page LSM) |
| 8 | 85458 | - | 48485 | - | - | 303691 | 46488 | - | 19552 | 69060 | page log (2.38x to 2.38x over log) |
| 13.90 | 84587 | - | 20169 | 13954 | 16704 | 299455 | 18172 | 10039 | 18521 | 68222 | LSM (1.81x to 1.81x over log) |
| 16 | 84257 | - | 7821-20114 | 4961-12828 | 14713 | 298645 | 5824-18117 | 9430 | 18243 | 67899 | undecided (LSM by up to 3.65x, log by up to 1.62x) |
| 26.74 | 83409 | - | 7821-20114 | 4961-12828 | 11025 | 296107 | 5824-18117 | 7968 | 17258 | 67081 | undecided (LSM by up to 3.65x, log by up to 1.37x) |
| 32 | 83107 | - | 7821-20114 | 4961-12828 | 10104 | 295230 | 5824-18117 | 7598 | 16956 | 66780 | undecided (LSM by up to 3.65x, log by up to 1.30x) |
| 64 | 80967-82451 | - | 7821-20114 | 4961-12828 | 7535 | 293018 | 5824-18117 | 4961-6445 | 14734-16218 | 64668-66152 | undecided (LSM by up to 3.65x, log by up to 1.11x) |
| 128 | 80967-82451 | 2296-6721 | 7821-20114 | 4961-12828 | 4961-7079 | 290549-292703 | 5824-18117 | 4961-6445 | 14734-16218 | 64668-66152 | undecided (LSM by up to 1.35x, log by up to 2.81x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 626 | 0 | 5029 | 854240 | 0 | 5.00 | 93 |
| 4 | B+tree, copy-on-write | 2089 | 0 | 18152 | 0 | 0 | 2.00 | 201 |
| 4 | page log, GC at 1/2 | 806 | 0 | 0 | 33255096 | 33255096 | 2.00 | 125 |
| 4 | page LSM | 583 | 0 | 0 | 154438195 | 137745696 | 1.00 | 87 |
| 8 | page | 449 | 0 | 5029 | 854240 | 0 | 5.00 | 61 |
| 8 | log, index on disk (1.5 B) | 2846 | 2602 | 0 | 12127696 | 12107057 | 1.00 | 61 |
| 8 | B+tree, copy-on-write | 1314 | 0 | 18152 | 0 | 0 | 2.00 | 112 |
| 8 | log, index on disk, GC at 2/3 | 2846 | 2602 | 0 | 7583338 | 7562699 | 1.00 | 61 |
| 8 | page log, GC at 1/2 | 482 | 0 | 0 | 33255096 | 33255096 | 2.00 | 68 |
| 8 | page LSM | 439 | 0 | 0 | 154438195 | 137745696 | 1.00 | 59 |
| 16 | page | 329 | 0 | 5029 | 854240 | 0 | 5.00 | 40 |
| 16 | log, index on disk (1.5 B) | 1030 | 787 | 0 | 12127696 | 12107057 | 1.00 | 13 |
| 16 | LSM, node cache | 871 | 0 | 0 | 10032965 | 8238908 | 1.00 | 13 |
| 16 | LSM, block cache | 1059 | 0 | 0 | 10032965 | 8238908 | 1.00 | 80 |
| 16 | B+tree, copy-on-write | 810 | 0 | 18152 | 0 | 0 | 2.00 | 65 |
| 16 | log, index on disk, GC at 2/3 | 1030 | 787 | 0 | 7583338 | 7562699 | 1.00 | 13 |
| 16 | LSM, page-keyed | 531 | 0 | 0 | 10032965 | 8238908 | 1.00 | 61 |
| 16 | page log, GC at 1/2 | 351 | 0 | 0 | 33255096 | 33255096 | 2.00 | 44 |
| 16 | page LSM | 323 | 0 | 0 | 154438195 | 137745696 | 1.00 | 39 |
| 32 | page | 214 | 0 | 5029 | 854240 | 0 | 5.00 | 22 |
| 32 | log, index on disk (1.5 B) | 1030 | 787 | 0 | 12127696 | 12107057 | 1.00 | 13 |
| 32 | LSM, node cache | 871 | 0 | 0 | 10032965 | 8238908 | 1.00 | 13 |
| 32 | LSM, block cache | 598 | 0 | 0 | 10032965 | 8238908 | 1.00 | 38 |
| 32 | B+tree, copy-on-write | 468 | 0 | 18152 | 0 | 0 | 2.00 | 34 |
| 32 | log, index on disk, GC at 2/3 | 1030 | 787 | 0 | 7583338 | 7562699 | 1.00 | 13 |
| 32 | LSM, page-keyed | 348 | 0 | 0 | 10032965 | 8238908 | 1.00 | 29 |
| 32 | page log, GC at 1/2 | 222 | 0 | 0 | 33255096 | 33255096 | 2.00 | 23 |
| 32 | page LSM | 211 | 0 | 0 | 154438195 | 137745696 | 1.00 | 21 |

### Sensitivity to the index engine's write amplification, 16 GB

The log store with its index on disk (1.5 B per record in DRAM), IO cost per block. 12.7 is the amplification the tables above use, the model's 15.3 scaled by the RocksDB measurement.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 12.7 | 6076-10797 | 6076-9764 | 7821-23556 | 7821-20114 |
| 15.3 | 6869-11590 | 6869-10557 | 8614-24350 | 8614-20908 |
| 5.0 | 3692-8412 | 3692-7380 | 5437-21172 | 5437-17730 |
| 30.0 | 11431-16151 | 11431-15119 | 13176-28911 | 13176-25469 |
| page store, for comparison | 21410 | 21410 | 84257 | 84257 |

### Sensitivity to the record engine's write amplification, 16 GB

The LSM of records, IO cost per block. 4.6 is the amplification the tables above use, the model's 4.5 scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 4.6, node cache | 4331-6692 | 4331-6692 | 4961-12828 | 4961-12828 |
| 4.6, block cache | 7257 | 7257 | 14713 | 14713 |
| 4.6, page-keyed | 5672 | 5672 | 9430 | 9430 |
| 5.0, node cache | 4653-7013 | 4653-7013 | 5282-13150 | 5282-13150 |
| 5.0, block cache | 7578 | 7578 | 15035 | 15035 |
| 5.0, page-keyed | 5993 | 5993 | 9751 | 9751 |
| 15.0, node cache | 12537-14897 | 12537-14897 | 13166-21034 | 13166-21034 |
| 15.0, block cache | 15462 | 15462 | 22919 | 22919 |
| 15.0, page-keyed | 13877 | 13877 | 17635 | 17635 |
| 30.0, node cache | 24363-26723 | 24363-26723 | 24992-32860 | 24992-32860 |
| 30.0, block cache | 27288 | 27288 | 34745 | 34745 |
| 30.0, page-keyed | 25703 | 25703 | 29461 | 29461 |

### Sensitivity to garbage collection, 16 GB

The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.

| g | store | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes | space |
|---|---|---|---|---|---|---|
| 0.50 | log, index on disk (index amplification 12.7) | 6076-10797 | 6076-9764 | 7821-23556 | 7821-20114 | 2.0x |
| 0.50 | page log | 15707 | 15707 | 18243 | 18243 | 2.0x |
| 0.67 | log, index on disk (index amplification 9.8) | 4079-8800 | 4079-7767 | 5824-21559 | 5824-18117 | 3.0x |
| 0.67 | page log | 12053 | 12053 | 14590 | 14590 | 3.0x |
| 0.80 | log, index on disk (index amplification 6.8) | 2955-7675 | 2955-6642 | 4700-20435 | 4700-16993 | 5.0x |
| 0.80 | page log | 10227 | 10227 | 12763 | 12763 | 5.0x |

