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
| paged-pos(res=2) | 0.127 | 462.5 | 450.5 | 3566.8 | 327.9 | 3.0 | 2 | 15096 | 0 |
| append-readback(res=12) | 0.489 | 0.0 | 3483.3 | 3566.8 | 327.9 | 1.0 | 374 | 0 | 0 |
| btree-cow(res=2) | 0.000 | 3071.3 | 4885.6 | 3566.8 | 327.9 | 2.0 | 1 | 603674 | 0 |

`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens
inside libmdbx and is invisible to these counters, so its seq/rand columns are
zero by construction and a modeled cost would be meaningless. It is compared
separately on real wall-clock and real on-disk size.


## Read pricing sensitivity (nvme)

A per-IO read price ignores transfer size; a page store moves 4 KB per hop
while a log store moves one sector. Both prices shown.

| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |
|---|---|---|---|
| paged-pos(res=2) | 3295.2 | 3295.2 | 1802.1 |
| append-readback(res=12) | 10881.9 | 6310.1 | 1741.6 |
| btree-cow(res=2) | 26982.7 | 26982.7 | 19542.5 |

## Modeled cost, profile `nvme-optimistic`

Units: one sequential 4KB write = 1. rand write 4x, rand read 3x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 3295.2 | 1.00x | 2 |
| append-readback(res=12) | 10881.9 | 3.30x | 374 |
| btree-cow(res=2) | 26982.7 | 8.19x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Modeled cost, profile `commodity-ssd`

Units: one sequential 4KB write = 1. rand write 16x, rand read 10x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 12122.4 | 1.00x | 2 |
| append-readback(res=12) | 35866.8 | 2.96x | 374 |
| btree-cow(res=2) | 98119.6 | 8.09x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Real-IO comparison (wall clock, on-disk)

| backend | commit wall ms/blk | disk MB | basis |
|---|---|---|---|
| paged-pos(res=2) | 10.173 | 15096 | real WAL write+fsync; page IO counter-modeled |
| append-readback(res=12) | 2.629 | 0 | counter-modeled |
| btree-cow(res=2) | 5.019 | 603674 | counter-modeled |

## Backend detail

- paged-pos(res=2): 3864680 stored pages, mean occupancy 6.3/126 slots (5.0%), WAL 8468 MB
  - elision: 14840162 page writes avoided, 0 read fallbacks
  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: [3472125, 297852, 46441, 14851, 7590, 25821]
- append-readback(res=12): live 1931 MB, amortized compaction reclaim 14664 MB, live records 25495264, live branches 12760672, idx_branches 12760672, idx_leaves 12734592, write-path RMW reads 190181104 (sibling-dirty 13630143), uncharged displaced-leaf splits 10601, dram bytes 392902459
- btree-cow MODELED: 33 entries per 4 KB leaf page (96 B mean entry at fill 0.8), fanout 60, height 6, pages per level [151964987, 2532750, 42213, 704, 12, 1] (633.0 GB)
- btree-cow: resident leaf pages 392 (1.6 MB); records created 26257285, rewritten 211566973, deleted 789522, leaf changes 21110463
- btree-cow, measured window: leaf pages copied 1511209, interior pages copied 1555495, freelist pages 3560; key walks 218067, prefix hits 3361702, fallback walks 10226, memo hits 314905

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
- SAMPLED: only accounts and contracts whose address hash starts with the 2 bits of part 3 are replayed, one key-space subtree in 4. Every figure below is scaled to the whole: per-block counts by 4, cache sizes and DRAM by 4, the full state and the index buffer taken at 1/4 for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.
- window: 1000 measured blocks after 49000 warm-up blocks; 50000 blocks available from 23700000
- full state stood in for: 334563311 accounts, 1438571749 storage slots, 399723913 code chunks = 626855571 leaves, 1253711142 records. Storage contracts: 25000000 (ASSUMED). An untouched contract is taken to hold 57 slots, the mean; real sizes are heavy-tailed.
- replay built from: 3670781 account changes (343678 creations) and 5831438 slot changes; 1326459 keys in the starting state; stand-ins: 5809681 beside account paths, 155894 beside contract buckets, 3972531 inside contracts' storage. 1860 of 17456 contracts written in the storage zone have a fetched slot count, covering 95.6% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes 1425950 distinct slots there in 1212472 stems (1.18 per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.
- path length, account zone: 468.2 leaves written per block, 32.5 branch records above each on average, 40 at most
- path length, storage zone: 286.5 leaves written per block, 41.3 branch records above each on average, 53 at most
- execution reads, per block: 1076.6 of existing keys and 234.9 of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.
- measured, log store, per block: 14217.9 records written, 0.99 MB appended by the commit; with no cache 13933.1 reads, of which 1076.6 are execution reads, 660.5 fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; 28295.0 index updates, 17.0 deleted branches to mark dead, 42.4 displaced leaves
- measured, page store, per block: 1850.2 merkle pages written; with no cache 3741.0 reads, of which 1802.1 are merkle pages and the rest value-store leaf pages (execution reads included); 627.3 value-store leaf pages and 552.6 bottom branch nodes rewritten, 11.84 MB held by one commit
- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at 88 B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.
- fixed DRAM at full-state size: log store with its index in DRAM 71.9 GB (15.4 B per record); with its index on disk 7.6 GB or 20.4 GB (1.5 or 4.25 B per record, plus a 256 MB write buffer and 298 MB of page pointers, all ASSUMED); page store 0.99 GB (0.80 GB of resident bottom branch nodes over 74462908 value-store leaf pages, 0.14 GB of bucket map, ASSUMED, and one commit's pages).
- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, 5.25 GB of records for the log store and 34.73 GB of pages for the page store. A replay of 50000 blocks cannot tell further; a longer one narrows the range.
- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over 75 GB of 16-byte entries, a 256 MB write buffer, levels growing by 10x down to one that holds every entry. How much each level absorbs is MEASURED from the 172588190 index updates of the replay (13807 per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The 1.0 relocations per update are taken never to be absorbed. Write amplification 12.8 per index update (205 B written and read): the model gives 15.4, and a real RocksDB fed this stream as the index sees it wrote 0.83 of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); 15 for the assumed engine before. That costs 2555 per block on `nvme-optimistic` and 2555 on `commodity-ssd`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 4.194e6 | 252 | yes | 0.608 | 3.03 |
  | L1 | 1.254e7 | 786 | yes | 0.583 | 6.41 |
  | L2 | 1.254e8 | 8396 | yes | 0.545 | 6.00 |
  | L3 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a 256 MB write buffer and levels growing by 10x down to one holding all 5014844568 records. An entry is 81 B, the mean record of this replay's write stream (72.7 B, its 4 B log header standing in for the varint lengths) plus 8 B of sequence number and type; prefix compression is not modeled. Updates are the 14267.1 records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification 5.0 per record update (406 B written and read back by compaction, plus one WAL copy of each update), the model's 5.0 scaled by 1.01, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against 12.8 for the index entries above; that costs 2547 per block on `nvme-optimistic` and 2547 on `commodity-ssd`. Fixed DRAM 8.89 GB: 1.85 B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at 97 B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 8.312e5 | 276 | yes | 0.218 | 0.76 |
  | L1 | 1.254e6 | 470 | yes | 0.193 | 2.13 |
  | L2 | 1.254e7 | 9537 | yes | 0.095 | 1.05 |
  | L3 | 1.254e8 | 95821 | no: longest window's share used | 0.095 | 1.04 |
  | L4 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, 33 per leaf page (96 B mean entry at fill 0.8, ASSUMED), 60 children per interior page (ASSUMED), height 6 over 151964987 leaf pages (633 GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM (0.001 GB). The page map is static: no page splits or merges over the replay.
- measured, B+tree, per block: 12285.1 pages rewritten copy-on-write; with no cache 6762.7 leaf-page reads and 12779.8 interior-page reads, execution reads included; 6044.8 leaf pages and 6222.0 interior pages copied, 47.99 MB held by one commit, 2 fsyncs
- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM (2.22 GB: 16 B per page over 148925816 pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: 2477.5 pages appended per block, each superseding one; space up to 2x the live pages.
- page LSM, MODELED from the page store's run: the same pages as 4112 B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the 29983299 page writes of the replay (2399 per block, warm-up included). page write amplification 8.2 per page update (33 KB written and read back by compaction per page written, plus one WAL copy).

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 1.632e4 | 16 | yes | 0.437 | 1.87 |
  | L1 | 3.723e4 | 46 | yes | 0.336 | 3.70 |
  | L2 | 3.723e5 | 1041 | yes | 0.149 | 1.64 |
  | L3 | 3.723e6 | 16960 | no: longest window's share used | 0.092 | 1.01 |
  | L4 | 3.723e7 | - | - | 0.000 | 0.00 |

- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations 7013.9 per block at g = 2/3 (14027.9 at g = 1/2), index write amplification 10.0 (12.8 at g = 1/2), compaction reading 1.46 MB and rewriting 0.49 MB per block (1.94 and 0.97 at g = 1/2); space up to 3x the live records instead of 2x.
- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves (42.4 per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the 895.3 elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.
- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified.
- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name.
- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by 1.3x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.

### Reads (write path and execution) per block against cache size, exact range only

Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.

| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |
|---|---|---|---|---|---|---|
| 1/64 | 918324 | 0.082 | 3392.1 | 141684 | 0.543 | 768.1 |
| 1/32 | 1836652 | 0.164 | 2573.0 | 283372 | 1.085 | 574.3 |
| 1/16 | 3673304 | 0.328 | 2013.3 | 566744 | 2.170 | 434.2 |
| 1/8 | 7346608 | 0.657 | 1595.8 | 1133488 | 4.341 | 334.4 |
| 1/4 | 14693216 | 1.314 | 1168.7 | 2266976 | 8.682 | 256.8 |
| 1/2 | 29386432 | 2.627 | 865.2 | 4533956 | 17.363 | 178.2 |
| 1/1 | 58772868 | 5.255 | 618.4 | 9067912 | 34.726 | 111.4 |

With no cache at all: 13933.1 reads per block for the log store, 3741.0 for the page store. Of the log store's, 616.1 are of records untouched since the replay began; of the page store's, 110.8.

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 21049 | - | - | - | - | 58868 | - | - | - | 41709 | page (1.98x to 1.98x over page LSM) |
| 2 | 14136 | - | - | - | - | 55353 | - | - | - | 40823 | page (2.89x to 2.89x over page LSM) |
| 4 | 13516 | - | - | - | - | 53163 | - | - | 10384 | 40391 | page log (1.30x to 1.30x over page) |
| 8 | 13223 | - | 14609 | - | - | 51788 | 13326 | - | 9873 | 40141 | page log (1.34x to 1.34x over page) |
| 12.81 | 13021 | - | 7490 | 5118 | 5546 | 51108 | 6207 | 4015 | 9645 | 39953 | LSM (1.55x to 1.55x over log) |
| 16 | 12942 | - | 3780-7476 | 2975-4824 | 4948 | 50844 | 2497-6194 | 3827 | 9570 | 39874 | undecided (LSM by up to 2.08x, log by up to 1.53x) |
| 25.65 | 12789 | - | 3780-7476 | 2975-4824 | 4217 | 50282 | 2497-6194 | 3519 | 9399 | 39727 | undecided (LSM by up to 2.08x, log by up to 1.41x) |
| 32 | 12730 | - | 3780-7476 | 2975-4824 | 3990 | 50085 | 2497-6194 | 3419 | 9333 | 39668 | undecided (LSM by up to 2.08x, log by up to 1.37x) |
| 64 | 12366-12699 | - | 3780-7476 | 2975-4824 | 3467 | 49180-49667 | 2497-6194 | 2975-3308 | 8959-9291 | 39309-39641 | undecided (LSM by up to 2.08x, log by up to 1.32x) |
| 128 | 12366-12699 | 1046-2895 | 3780-7476 | 2975-4824 | 2975-3454 | 49180-49667 | 2497-6194 | 2975-3308 | 8959-9291 | 39309-39641 | too close (log by 1.03x to 3.16x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 21049 | - | - | - | - | 58868 | - | - | - | 41709 | page (1.98x to 1.98x over page LSM) |
| 2 | 14136 | - | - | - | - | 55353 | - | - | - | 40823 | page (2.89x to 2.89x over page LSM) |
| 4 | 13516 | - | - | - | - | 53163 | - | - | 10384 | 40391 | page log (1.30x to 1.30x over page) |
| 8 | 13223 | - | 12240 | - | - | 51788 | 10957 | - | 9873 | 40141 | too close (page log by 1.11x to 1.11x over log) |
| 12.81 | 13021 | - | 6678 | 5118 | 5546 | 51108 | 5395 | 4015 | 9645 | 39953 | LSM (1.34x to 1.34x over log) |
| 16 | 12942 | - | 3780-6668 | 2975-4824 | 4948 | 50844 | 2497-5385 | 3827 | 9570 | 39874 | undecided (LSM by up to 1.81x, log by up to 1.53x) |
| 25.65 | 12789 | - | 3780-6668 | 2975-4824 | 4217 | 50282 | 2497-5385 | 3519 | 9399 | 39727 | undecided (LSM by up to 1.81x, log by up to 1.41x) |
| 32 | 12730 | - | 3780-6668 | 2975-4824 | 3990 | 50085 | 2497-5385 | 3419 | 9333 | 39668 | undecided (LSM by up to 1.81x, log by up to 1.37x) |
| 64 | 12366-12699 | - | 3780-6668 | 2975-4824 | 3467 | 49180-49667 | 2497-5385 | 2975-3308 | 8959-9291 | 39309-39641 | undecided (LSM by up to 1.81x, log by up to 1.32x) |
| 128 | 12366-12699 | 1046-2086 | 3780-6668 | 2975-4824 | 2975-3454 | 49180-49667 | 2497-5385 | 2975-3308 | 8959-9291 | 39309-39641 | log (1.43x to 3.16x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 77918 | - | - | - | - | 228974 | - | - | - | 47350 | page LSM (1.65x to 1.65x over page) |
| 2 | 54874 | - | - | - | - | 217256 | - | - | - | 44398 | too close (page LSM by 1.24x to 1.24x over page) |
| 4 | 52807 | - | - | - | - | 209958 | - | - | 13789 | 42957 | page log (3.12x to 3.12x over page LSM) |
| 8 | 51830 | - | 40629 | - | - | 205373 | 39346 | - | 12086 | 42123 | page log (3.26x to 3.26x over log) |
| 12.81 | 51158 | - | 16900 | 10454 | 11881 | 203107 | 15617 | 6778 | 11327 | 41495 | LSM (1.67x to 1.67x over page log) |
| 16 | 50894 | - | 4532-16855 | 3312-9473 | 9890 | 202227 | 3250-15572 | 6153 | 11077 | 41234 | undecided (LSM by up to 4.70x, log by up to 1.89x) |
| 25.65 | 50386 | - | 4532-16855 | 3312-9473 | 7452 | 200352 | 3250-15572 | 5125 | 10505 | 40741 | undecided (LSM by up to 4.70x, log by up to 1.58x) |
| 32 | 50187 | - | 4532-16855 | 3312-9473 | 6696 | 199695 | 3250-15572 | 4791 | 10285 | 40546 | undecided (LSM by up to 4.70x, log by up to 1.47x) |
| 64 | 48975-50083 | - | 4532-16855 | 3312-9473 | 4951 | 196681-198302 | 3250-15572 | 3312-4420 | 9039-10147 | 39349-40457 | undecided (LSM by up to 4.70x, log by up to 1.36x) |
| 128 | 48975-50083 | 1383-7544 | 4532-16855 | 3312-9473 | 3312-4909 | 196681-198302 | 3250-15572 | 3312-4420 | 9039-10147 | 39349-40457 | undecided (LSM by up to 2.28x, log by up to 3.20x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 77918 | - | - | - | - | 228974 | - | - | - | 47350 | page LSM (1.65x to 1.65x over page) |
| 2 | 54874 | - | - | - | - | 217256 | - | - | - | 44398 | too close (page LSM by 1.24x to 1.24x over page) |
| 4 | 52807 | - | - | - | - | 209958 | - | - | 13789 | 42957 | page log (3.12x to 3.12x over page LSM) |
| 8 | 51830 | - | 32733 | - | - | 205373 | 31450 | - | 12086 | 42123 | page log (2.60x to 2.60x over log) |
| 12.81 | 51158 | - | 14195 | 10454 | 11881 | 203107 | 12912 | 6778 | 11327 | 41495 | LSM (1.67x to 1.67x over page log) |
| 16 | 50894 | - | 4532-14160 | 3312-9473 | 9890 | 202227 | 3250-12877 | 6153 | 11077 | 41234 | undecided (LSM by up to 3.89x, log by up to 1.89x) |
| 25.65 | 50386 | - | 4532-14160 | 3312-9473 | 7452 | 200352 | 3250-12877 | 5125 | 10505 | 40741 | undecided (LSM by up to 3.89x, log by up to 1.58x) |
| 32 | 50187 | - | 4532-14160 | 3312-9473 | 6696 | 199695 | 3250-12877 | 4791 | 10285 | 40546 | undecided (LSM by up to 3.89x, log by up to 1.47x) |
| 64 | 48975-50083 | - | 4532-14160 | 3312-9473 | 4951 | 196681-198302 | 3250-12877 | 3312-4420 | 9039-10147 | 39349-40457 | undecided (LSM by up to 3.89x, log by up to 1.36x) |
| 128 | 48975-50083 | 1383-4849 | 4532-14160 | 3312-9473 | 3312-4909 | 196681-198302 | 3250-12877 | 3312-4420 | 9039-10147 | 39349-40457 | undecided (LSM by up to 1.46x, log by up to 3.20x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 383 | 0 | 3034 | 532409 | 0 | 5.00 | 56 |
| 4 | B+tree, copy-on-write | 1328 | 0 | 12285 | 0 | 0 | 2.00 | 122 |
| 4 | page log, GC at 1/2 | 475 | 0 | 0 | 20295680 | 20295680 | 2.00 | 72 |
| 4 | page LSM | 361 | 0 | 0 | 93931648 | 83744168 | 1.00 | 51 |
| 8 | page | 286 | 0 | 3034 | 532409 | 0 | 5.00 | 36 |
| 8 | log, index on disk (1.5 B) | 1907 | 1805 | 0 | 7866844 | 7851090 | 1.00 | 56 |
| 8 | B+tree, copy-on-write | 869 | 0 | 12285 | 0 | 0 | 2.00 | 73 |
| 8 | log, index on disk, GC at 2/3 | 1907 | 1805 | 0 | 4947908 | 4932154 | 1.00 | 56 |
| 8 | page log, GC at 1/2 | 305 | 0 | 0 | 20295680 | 20295680 | 2.00 | 40 |
| 8 | page LSM | 277 | 0 | 0 | 93931648 | 83744168 | 1.00 | 35 |
| 16 | page | 192 | 0 | 3034 | 532409 | 0 | 5.00 | 21 |
| 16 | log, index on disk (1.5 B) | 718 | 616 | 0 | 7866844 | 7851090 | 1.00 | 16 |
| 16 | LSM, node cache | 659 | 0 | 0 | 6947195 | 5795297 | 1.00 | 16 |
| 16 | LSM, block cache | 700 | 0 | 0 | 6947195 | 5795297 | 1.00 | 49 |
| 16 | B+tree, copy-on-write | 555 | 0 | 12285 | 0 | 0 | 2.00 | 42 |
| 16 | log, index on disk, GC at 2/3 | 718 | 616 | 0 | 4947908 | 4932154 | 1.00 | 16 |
| 16 | LSM, page-keyed | 326 | 0 | 0 | 6947195 | 5795297 | 1.00 | 36 |
| 16 | page log, GC at 1/2 | 204 | 0 | 0 | 20295680 | 20295680 | 2.00 | 23 |
| 16 | page LSM | 189 | 0 | 0 | 93931648 | 83744168 | 1.00 | 21 |
| 32 | page | 121 | 0 | 3034 | 532409 | 0 | 5.00 | 12 |
| 32 | log, index on disk (1.5 B) | 718 | 616 | 0 | 7866844 | 7851090 | 1.00 | 16 |
| 32 | LSM, node cache | 659 | 0 | 0 | 6947195 | 5795297 | 1.00 | 16 |
| 32 | LSM, block cache | 381 | 0 | 0 | 6947195 | 5795297 | 1.00 | 23 |
| 32 | B+tree, copy-on-write | 301 | 0 | 12285 | 0 | 0 | 2.00 | 21 |
| 32 | log, index on disk, GC at 2/3 | 718 | 616 | 0 | 4947908 | 4932154 | 1.00 | 16 |
| 32 | LSM, page-keyed | 190 | 0 | 0 | 6947195 | 5795297 | 1.00 | 14 |
| 32 | page log, GC at 1/2 | 125 | 0 | 0 | 20295680 | 20295680 | 2.00 | 12 |
| 32 | page LSM | 120 | 0 | 0 | 93931648 | 83744168 | 1.00 | 11 |

### Sensitivity to the index engine's write amplification, 16 GB

The log store with its index on disk (1.5 B per record in DRAM), IO cost per block. 12.8 is the amplification the tables above use, the model's 15.4 scaled by the RocksDB measurement.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 12.8 | 3780-7476 | 3780-6668 | 4532-16855 | 4532-14160 |
| 15.4 | 4297-7994 | 4297-7185 | 5050-17372 | 5050-14677 |
| 5.0 | 2219-5916 | 2219-5107 | 2972-15295 | 2972-12599 |
| 30.0 | 7193-10890 | 7193-10081 | 7946-20269 | 7946-17573 |
| page store, for comparison | 12942 | 12942 | 50894 | 50894 |

### Sensitivity to the record engine's write amplification, 16 GB

The LSM of records, IO cost per block. 5.0 is the amplification the tables above use, the model's 5.0 scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 5.0, node cache | 2975-4824 | 2975-4824 | 3312-9473 | 3312-9473 |
| 5.0, block cache | 4948 | 4948 | 9890 | 9890 |
| 5.0, page-keyed | 3827 | 3827 | 6153 | 6153 |
| 5.0, node cache | 2959-4808 | 2959-4808 | 3296-9458 | 3296-9458 |
| 5.0, block cache | 4933 | 4933 | 9874 | 9874 |
| 5.0, page-keyed | 3812 | 3812 | 6137 | 6137 |
| 15.0, node cache | 8022-9870 | 8022-9870 | 8358-14520 | 8358-14520 |
| 15.0, block cache | 9995 | 9995 | 14936 | 14936 |
| 15.0, page-keyed | 8874 | 8874 | 11199 | 11199 |
| 30.0, node cache | 15615-17463 | 15615-17463 | 15951-22113 | 15951-22113 |
| 30.0, block cache | 17588 | 17588 | 22529 | 22529 |
| 30.0, page-keyed | 16467 | 16467 | 18792 | 18792 |

### Sensitivity to garbage collection, 16 GB

The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.

| g | store | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes | space |
|---|---|---|---|---|---|---|
| 0.50 | log, index on disk (index amplification 12.8) | 3780-7476 | 3780-6668 | 4532-16855 | 4532-14160 | 2.0x |
| 0.50 | page log | 9570 | 9570 | 11077 | 11077 | 2.0x |
| 0.67 | log, index on disk (index amplification 10.0) | 2497-6194 | 2497-5385 | 3250-15572 | 3250-12877 | 3.0x |
| 0.67 | page log | 7341 | 7341 | 8847 | 8847 | 3.0x |
| 0.80 | log, index on disk (index amplification 7.1) | 1773-5470 | 1773-4661 | 2526-14848 | 2526-12153 | 5.0x |
| 0.80 | page log | 6226 | 6226 | 7732 | 7732 | 5.0x |

