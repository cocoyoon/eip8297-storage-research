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
| paged-pos(res=2) | 0.125 | 456.6 | 444.5 | 3543.1 | 335.6 | 3.0 | 2 | 18644 | 0 |
| append-readback(res=12) | 0.486 | 0.0 | 3465.4 | 3543.1 | 335.6 | 1.0 | 489 | 0 | 0 |
| btree-cow(res=2) | 0.000 | 3038.1 | 4899.4 | 3543.1 | 335.6 | 2.0 | 1 | 603674 | 0 |

`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens
inside libmdbx and is invisible to these counters, so its seq/rand columns are
zero by construction and a modeled cost would be meaningless. It is compared
separately on real wall-clock and real on-disk size.


## Read pricing sensitivity (nvme)

A per-IO read price ignores transfer size; a page store moves 4 KB per hop
while a log store moves one sector. Both prices shown.

| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |
|---|---|---|---|
| paged-pos(res=2) | 3253.0 | 3253.0 | 1778.1 |
| append-readback(res=12) | 10884.0 | 6335.7 | 1732.7 |
| btree-cow(res=2) | 26891.5 | 26891.5 | 19597.7 |

## Modeled cost, profile `nvme-optimistic`

Units: one sequential 4KB write = 1. rand write 4x, rand read 3x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 3253.0 | 1.00x | 2 |
| append-readback(res=12) | 10884.0 | 3.35x | 489 |
| btree-cow(res=2) | 26891.5 | 8.27x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Modeled cost, profile `commodity-ssd`

Units: one sequential 4KB write = 1. rand write 16x, rand read 10x.

| backend | cost/block | vs best | DRAM MB |
|---|---|---|---|
| paged-pos(res=2) | 11966.9 | 1.00x | 2 |
| append-readback(res=12) | 35915.2 | 3.00x | 489 |
| btree-cow(res=2) | 97727.0 | 8.17x | 1 |
- DRAM ceiling 64 MB: no configuration fits
- DRAM ceiling 256 MB: no configuration fits
- DRAM ceiling 1024 MB: no configuration fits

## Real-IO comparison (wall clock, on-disk)

| backend | commit wall ms/blk | disk MB | basis |
|---|---|---|---|
| paged-pos(res=2) | 11.410 | 18644 | real WAL write+fsync; page IO counter-modeled |
| append-readback(res=12) | 5.566 | 0 | counter-modeled |
| btree-cow(res=2) | 6.150 | 603674 | counter-modeled |

## Backend detail

- paged-pos(res=2): 4772916 stored pages, mean occupancy 6.6/126 slots (5.3%), WAL 9350 MB
  - elision: 16166957 page writes avoided, 0 read fallbacks
  - occupancy buckets [<10%,<25%,<50%,<75%,<100%,full]: [4238134, 404486, 89962, 10160, 3688, 26486]
- append-readback(res=12): live 2589 MB, amortized compaction reclaim 15950 MB, live records 33284198, live branches 16668423, idx_branches 16668423, idx_leaves 16615775, write-path RMW reads 208436567 (sibling-dirty 16408975), uncharged displaced-leaf splits 9938, dram bytes 512852332
- btree-cow MODELED: 33 entries per 4 KB leaf page (96 B mean entry at fill 0.8), fanout 60, height 6, pages per level [151964987, 2532750, 42213, 704, 12, 1] (633.0 GB)
- btree-cow: resident leaf pages 392 (1.6 MB); records created 34122333, rewritten 230107234, deleted 892236, leaf changes 25385046
- btree-cow, measured window: leaf pages copied 1497921, interior pages copied 1535645, freelist pages 3532; key walks 223751, prefix hits 3337589, fallback walks 10007, memo hits 317347

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
- SAMPLED: only accounts and contracts whose address hash starts with the 2 bits of part 1 are replayed, one key-space subtree in 4. Every figure below is scaled to the whole: per-block counts by 4, cache sizes and DRAM by 4, the full state and the index buffer taken at 1/4 for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.
- window: 1000 measured blocks after 49000 warm-up blocks; 50000 blocks available from 23700000
- full state stood in for: 334563311 accounts, 1438571749 storage slots, 399723913 code chunks = 626855571 leaves, 1253711142 records. Storage contracts: 25000000 (ASSUMED). An untouched contract is taken to hold 57 slots, the mean; real sizes are heavy-tailed.
- replay built from: 3672467 account changes (343927 creations) and 7543347 slot changes; 1941090 keys in the starting state; stand-ins: 5812641 beside account paths, 156174 beside contract buckets, 5915399 inside contracts' storage. 1834 of 17484 contracts written in the storage zone have a fetched slot count, covering 97.0% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes 3418994 distinct slots there in 1454665 stems (2.35 per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.
- path length, account zone: 460.5 leaves written per block, 32.5 branch records above each on average, 40 at most
- path length, storage zone: 284.7 leaves written per block, 41.3 branch records above each on average, 56 at most
- execution reads, per block: 1108.4 of existing keys and 234.1 of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.
- measured, log store, per block: 14098.9 records written, 0.98 MB appended by the commit; with no cache 13861.5 reads, of which 1108.4 are execution reads, 668.9 fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; 28112.9 index updates, 24.8 deleted branches to mark dead, 39.8 displaced leaves
- measured, page store, per block: 1826.4 merkle pages written; with no cache 3739.5 reads, of which 1778.1 are merkle pages and the rest value-store leaf pages (execution reads included); 618.9 value-store leaf pages and 545.2 bottom branch nodes rewritten, 11.68 MB held by one commit
- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at 88 B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.
- fixed DRAM at full-state size: log store with its index in DRAM 71.9 GB (15.4 B per record); with its index on disk 7.6 GB or 20.4 GB (1.5 or 4.25 B per record, plus a 256 MB write buffer and 298 MB of page pointers, all ASSUMED); page store 0.99 GB (0.80 GB of resident bottom branch nodes over 74462908 value-store leaf pages, 0.14 GB of bucket map, ASSUMED, and one commit's pages).
- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, 7.41 GB of records for the log store and 45.48 GB of pages for the page store. A replay of 50000 blocks cannot tell further; a longer one narrows the range.
- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over 75 GB of 16-byte entries, a 256 MB write buffer, levels growing by 10x down to one that holds every entry. How much each level absorbs is MEASURED from the 179601197 index updates of the replay (14368 per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The 1.0 relocations per update are taken never to be absorbed. Write amplification 13.2 per index update (212 B written and read): the model gives 15.9, and a real RocksDB fed this stream as the index sees it wrote 0.83 of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); 15 for the assumed engine before. That costs 2616 per block on `nvme-optimistic` and 2616 on `commodity-ssd`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 4.194e6 | 236 | yes | 0.623 | 3.11 |
  | L1 | 1.254e7 | 735 | yes | 0.598 | 6.58 |
  | L2 | 1.254e8 | 7775 | yes | 0.566 | 6.22 |
  | L3 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a 256 MB write buffer and levels growing by 10x down to one holding all 5014844568 records. An entry is 81 B, the mean record of this replay's write stream (72.7 B, its 4 B log header standing in for the varint lengths) plus 8 B of sequence number and type; prefix compression is not modeled. Updates are the 14172.3 records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification 6.6 per record update (530 B written and read back by compaction, plus one WAL copy of each update), the model's 6.5 scaled by 1.01, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against 13.2 for the index entries above; that costs 3298 per block on `nvme-optimistic` and 3298 on `commodity-ssd`. Fixed DRAM 8.89 GB: 1.85 B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at 97 B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 8.311e5 | 227 | yes | 0.255 | 0.90 |
  | L1 | 1.254e6 | 380 | yes | 0.230 | 2.53 |
  | L2 | 1.254e7 | 6045 | yes | 0.144 | 1.59 |
  | L3 | 1.254e8 | 64788 | no: longest window's share used | 0.135 | 1.48 |
  | L4 | 1.254e9 | - | - | 0.000 | 0.00 |

- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, 33 per leaf page (96 B mean entry at fill 0.8, ASSUMED), 60 children per interior page (ASSUMED), height 6 over 151964987 leaf pages (633 GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM (0.001 GB). The page map is static: no page splits or merges over the replay.
- measured, B+tree, per block: 12152.4 pages rewritten copy-on-write; with no cache 6742.9 leaf-page reads and 12854.9 interior-page reads, execution reads included; 5991.7 leaf pages and 6142.6 interior pages copied, 47.47 MB held by one commit, 2 fsyncs
- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM (2.22 GB: 16 B per page over 148925816 pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: 2445.2 pages appended per block, each superseding one; space up to 2x the live pages.
- page LSM, MODELED from the page store's run: the same pages as 4112 B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the 30518067 page writes of the replay (2441 per block, warm-up included). page write amplification 9.0 per page update (36 KB written and read back by compaction per page written, plus one WAL copy).

  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |
  |---|---|---|---|---|---|
  | write buffer | 1.632e4 | 15 | yes | 0.443 | 1.90 |
  | L1 | 3.723e4 | 44 | yes | 0.347 | 3.82 |
  | L2 | 3.723e5 | 884 | yes | 0.173 | 1.90 |
  | L3 | 3.723e6 | 12457 | no: longest window's share used | 0.122 | 1.35 |
  | L4 | 3.723e7 | - | - | 0.000 | 0.00 |

- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations 6970.3 per block at g = 2/3 (13940.6 at g = 1/2), index write amplification 10.6 (13.2 at g = 1/2), compaction reading 1.45 MB and rewriting 0.48 MB per block (1.93 and 0.97 at g = 1/2); space up to 3x the live records instead of 2x.
- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves (39.8 per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the 911.5 elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.
- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified.
- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name.
- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by 1.3x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.

### Reads (write path and execution) per block against cache size, exact range only

Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.

| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |
|---|---|---|---|---|---|---|
| 1/64 | 1294296 | 0.116 | 2834.4 | 185540 | 0.711 | 650.0 |
| 1/32 | 2588596 | 0.231 | 2176.4 | 371080 | 1.421 | 476.3 |
| 1/16 | 5177196 | 0.463 | 1692.1 | 742164 | 2.842 | 359.1 |
| 1/8 | 10354392 | 0.926 | 1366.2 | 1484332 | 5.684 | 282.9 |
| 1/4 | 20708784 | 1.852 | 1000.4 | 2968664 | 11.369 | 210.8 |
| 1/2 | 41417568 | 3.703 | 706.9 | 5937332 | 22.738 | 142.3 |
| 1/1 | 82835136 | 7.406 | 607.1 | 11874664 | 45.475 | 105.1 |

With no cache at all: 13861.5 reads per block for the log store, 3739.5 for the page store. Of the log store's, 606.0 are of records untouched since the replay began; of the page store's, 104.8.

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 20457 | - | - | - | - | 58179 | - | - | - | 44351 | page (2.17x to 2.17x over page LSM) |
| 2 | 13858 | - | - | - | - | 54727 | - | - | - | 43477 | page (3.14x to 3.14x over page LSM) |
| 4 | 13258 | - | - | - | - | 52565 | - | - | 10156 | 43073 | page log (1.31x to 1.31x over page) |
| 8 | 13008 | - | 14104 | - | - | 51195 | 12836 | - | 9690 | 42859 | page log (1.32x to 1.32x over log) |
| 14.96 | 12780 | - | 7485 | 5610 | 5726 | 50395 | 6216 | 4551 | 9443 | 42636 | LSM (1.37x to 1.37x over log) |
| 16 | 12752 | - | 3842-7478 | 5559 | 5590 | 50341 | 2574-6210 | 4516 | 9422 | 42613 | undecided (LSM by up to 1.38x, log by up to 1.75x) |
| 27.80 | 12594 | - | 3842-7478 | 3717-5534 | 4869 | 49735 | 2574-6210 | 4198 | 9240 | 42462 | undecided (LSM by up to 1.67x, log by up to 1.63x) |
| 32 | 12568 | - | 3842-7478 | 3717-5534 | 4732 | 49608 | 2574-6210 | 4139 | 9211 | 42436 | undecided (LSM by up to 1.67x, log by up to 1.61x) |
| 64 | 12206-12520 | - | 3842-7478 | 3717-5534 | 4273 | 49208 | 2574-6210 | 3717-4031 | 8843-9157 | 42076-42390 | undecided (LSM by up to 1.67x, log by up to 1.57x) |
| 128 | 12206-12520 | 1033-2850 | 3842-7478 | 3717-5534 | 3717-4197 | 48650-49137 | 2574-6210 | 3717-4031 | 8843-9157 | 42076-42390 | log (1.30x to 3.90x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 20457 | - | - | - | - | 58179 | - | - | - | 44351 | page (2.17x to 2.17x over page LSM) |
| 2 | 13858 | - | - | - | - | 54727 | - | - | - | 43477 | page (3.14x to 3.14x over page LSM) |
| 4 | 13258 | - | - | - | - | 52565 | - | - | 10156 | 43073 | page log (1.31x to 1.31x over page) |
| 8 | 13008 | - | 11859 | - | - | 51195 | 10591 | - | 9690 | 42859 | too close (page log by 1.09x to 1.09x over log) |
| 14.96 | 12780 | - | 6688 | 5610 | 5726 | 50395 | 5420 | 4551 | 9443 | 42636 | too close (LSM by 1.19x to 1.19x over log) |
| 16 | 12752 | - | 3842-6683 | 5559 | 5590 | 50341 | 2574-5415 | 4516 | 9422 | 42613 | undecided (LSM by up to 1.20x, log by up to 1.75x) |
| 27.80 | 12594 | - | 3842-6683 | 3717-5534 | 4869 | 49735 | 2574-5415 | 4198 | 9240 | 42462 | undecided (LSM by up to 1.46x, log by up to 1.63x) |
| 32 | 12568 | - | 3842-6683 | 3717-5534 | 4732 | 49608 | 2574-5415 | 4139 | 9211 | 42436 | undecided (LSM by up to 1.46x, log by up to 1.61x) |
| 64 | 12206-12520 | - | 3842-6683 | 3717-5534 | 4273 | 49208 | 2574-5415 | 3717-4031 | 8843-9157 | 42076-42390 | undecided (LSM by up to 1.46x, log by up to 1.57x) |
| 128 | 12206-12520 | 1033-2055 | 3842-6683 | 3717-5534 | 3717-4197 | 48650-49137 | 2574-5415 | 3717-4031 | 8843-9157 | 42076-42390 | log (1.81x to 3.90x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 75842 | - | - | - | - | 226323 | - | - | - | 49702 | page LSM (1.53x to 1.53x over page) |
| 2 | 53847 | - | - | - | - | 214817 | - | - | - | 46787 | too close (page LSM by 1.15x to 1.15x over page) |
| 4 | 51847 | - | - | - | - | 207611 | - | - | 13301 | 45440 | page log (3.42x to 3.42x over page LSM) |
| 8 | 51013 | - | 38818 | - | - | 203044 | 37550 | - | 11745 | 44725 | page log (3.20x to 3.20x over log) |
| 14.96 | 50253 | - | 16754 | 10347 | 10732 | 200378 | 15485 | 6818 | 10925 | 43985 | LSM (1.60x to 1.60x over page log) |
| 16 | 50160 | - | 4613-16732 | 10177 | 10279 | 200197 | 3344-15464 | 6699 | 10852 | 43906 | undecided (LSM by up to 2.31x, log by up to 2.00x) |
| 27.80 | 49634 | - | 4613-16732 | 4035-10095 | 7876 | 198177 | 3344-15464 | 5639 | 10246 | 43403 | undecided (LSM by up to 3.83x, log by up to 1.69x) |
| 32 | 49544 | - | 4613-16732 | 4035-10095 | 7419 | 197754 | 3344-15464 | 5442 | 10148 | 43317 | undecided (LSM by up to 3.83x, log by up to 1.63x) |
| 64 | 48339-49387 | - | 4613-16732 | 4035-10095 | 5888 | 196421 | 3344-15464 | 4035-5083 | 8923-9971 | 42116-43164 | undecided (LSM by up to 3.83x, log by up to 1.52x) |
| 128 | 48339-49387 | 1351-7411 | 4613-16732 | 4035-10095 | 4035-5636 | 194558-196184 | 3344-15464 | 4035-5083 | 8923-9971 | 42116-43164 | undecided (LSM by up to 1.84x, log by up to 3.76x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 75842 | - | - | - | - | 226323 | - | - | - | 49702 | page LSM (1.53x to 1.53x over page) |
| 2 | 53847 | - | - | - | - | 214817 | - | - | - | 46787 | too close (page LSM by 1.15x to 1.15x over page) |
| 4 | 51847 | - | - | - | - | 207611 | - | - | 13301 | 45440 | page log (3.42x to 3.42x over page LSM) |
| 8 | 51013 | - | 31336 | - | - | 203044 | 30068 | - | 11745 | 44725 | page log (2.56x to 2.56x over log) |
| 14.96 | 50253 | - | 14098 | 10347 | 10732 | 200378 | 12830 | 6818 | 10925 | 43985 | LSM (1.60x to 1.60x over page log) |
| 16 | 50160 | - | 4613-14081 | 10177 | 10279 | 200197 | 3344-12813 | 6699 | 10852 | 43906 | undecided (LSM by up to 1.91x, log by up to 2.00x) |
| 27.80 | 49634 | - | 4613-14081 | 4035-10095 | 7876 | 198177 | 3344-12813 | 5639 | 10246 | 43403 | undecided (LSM by up to 3.18x, log by up to 1.69x) |
| 32 | 49544 | - | 4613-14081 | 4035-10095 | 7419 | 197754 | 3344-12813 | 5442 | 10148 | 43317 | undecided (LSM by up to 3.18x, log by up to 1.63x) |
| 64 | 48339-49387 | - | 4613-14081 | 4035-10095 | 5888 | 196421 | 3344-12813 | 4035-5083 | 8923-9971 | 42116-43164 | undecided (LSM by up to 3.18x, log by up to 1.52x) |
| 128 | 48339-49387 | 1351-4759 | 4613-14081 | 4035-10095 | 4035-5636 | 194558-196184 | 3344-12813 | 4035-5083 | 8923-9971 | 42116-43164 | undecided (log by up to 3.76x, LSM by up to 1.18x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 351 | 0 | 2994 | 525259 | 0 | 5.00 | 55 |
| 4 | B+tree, copy-on-write | 1305 | 0 | 12152 | 0 | 0 | 2.00 | 134 |
| 4 | page log, GC at 1/2 | 438 | 0 | 0 | 20031341 | 20031341 | 2.00 | 71 |
| 4 | page LSM | 332 | 0 | 0 | 100168707 | 90113913 | 1.00 | 51 |
| 8 | page | 267 | 0 | 2994 | 525259 | 0 | 5.00 | 37 |
| 8 | log, index on disk (1.5 B) | 1815 | 1710 | 0 | 7991354 | 7978548 | 1.00 | 64 |
| 8 | B+tree, copy-on-write | 849 | 0 | 12152 | 0 | 0 | 2.00 | 83 |
| 8 | log, index on disk, GC at 2/3 | 1815 | 1710 | 0 | 5105488 | 5092682 | 1.00 | 64 |
| 8 | page log, GC at 1/2 | 282 | 0 | 0 | 20031341 | 20031341 | 2.00 | 40 |
| 8 | page LSM | 261 | 0 | 0 | 100168707 | 90113913 | 1.00 | 36 |
| 16 | page | 182 | 0 | 2994 | 525259 | 0 | 5.00 | 24 |
| 16 | log, index on disk (1.5 B) | 710 | 606 | 0 | 7991354 | 7978548 | 1.00 | 25 |
| 16 | LSM, node cache | 654 | 0 | 0 | 8648838 | 7504535 | 1.00 | 25 |
| 16 | LSM, block cache | 664 | 0 | 0 | 8648838 | 7504535 | 1.00 | 51 |
| 16 | B+tree, copy-on-write | 564 | 0 | 12152 | 0 | 0 | 2.00 | 51 |
| 16 | log, index on disk, GC at 2/3 | 710 | 606 | 0 | 5105488 | 5092682 | 1.00 | 25 |
| 16 | LSM, page-keyed | 306 | 0 | 0 | 8648838 | 7504535 | 1.00 | 37 |
| 16 | page log, GC at 1/2 | 193 | 0 | 0 | 20031341 | 20031341 | 2.00 | 25 |
| 16 | page LSM | 179 | 0 | 0 | 100168707 | 90113913 | 1.00 | 23 |
| 32 | page | 121 | 0 | 2994 | 525259 | 0 | 5.00 | 16 |
| 32 | log, index on disk (1.5 B) | 710 | 606 | 0 | 7991354 | 7978548 | 1.00 | 25 |
| 32 | LSM, node cache | 646 | 0 | 0 | 8648838 | 7504535 | 1.00 | 25 |
| 32 | LSM, block cache | 378 | 0 | 0 | 8648838 | 7504535 | 1.00 | 26 |
| 32 | B+tree, copy-on-write | 320 | 0 | 12152 | 0 | 0 | 2.00 | 29 |
| 32 | log, index on disk, GC at 2/3 | 710 | 606 | 0 | 5105488 | 5092682 | 1.00 | 25 |
| 32 | LSM, page-keyed | 180 | 0 | 0 | 8648838 | 7504535 | 1.00 | 18 |
| 32 | page log, GC at 1/2 | 123 | 0 | 0 | 20031341 | 20031341 | 2.00 | 16 |
| 32 | page LSM | 120 | 0 | 0 | 100168707 | 90113913 | 1.00 | 16 |

### Sensitivity to the index engine's write amplification, 16 GB

The log store with its index on disk (1.5 B per record in DRAM), IO cost per block. 13.2 is the amplification the tables above use, the model's 15.9 scaled by the RocksDB measurement.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 13.2 | 3842-7478 | 3842-6683 | 4613-16732 | 4613-14081 |
| 15.9 | 4372-8008 | 4372-7212 | 5142-17262 | 5142-14610 |
| 5.0 | 2215-5850 | 2215-5055 | 2985-15104 | 2985-12453 |
| 30.0 | 7156-10792 | 7156-9997 | 7927-20046 | 7927-17395 |
| page store, for comparison | 12752 | 12752 | 50160 | 50160 |

### Sensitivity to the record engine's write amplification, 16 GB

The LSM of records, IO cost per block. 6.6 is the amplification the tables above use, the model's 6.5 scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.

| write amplification | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes |
|---|---|---|---|---|
| 6.6, node cache | 5559 | 5559 | 10177 | 10177 |
| 6.6, block cache | 5590 | 5590 | 10279 | 10279 |
| 6.6, page-keyed | 4516 | 4516 | 6699 | 6699 |
| 5.0, node cache | 4775 | 4775 | 9393 | 9393 |
| 5.0, block cache | 4806 | 4806 | 9496 | 9496 |
| 5.0, page-keyed | 3732 | 3732 | 5916 | 5916 |
| 15.0, node cache | 9804 | 9804 | 14422 | 14422 |
| 15.0, block cache | 9835 | 9835 | 14525 | 14525 |
| 15.0, page-keyed | 8761 | 8761 | 10944 | 10944 |
| 30.0, node cache | 17347 | 17347 | 21965 | 21965 |
| 30.0, block cache | 17378 | 17378 | 22068 | 22068 |
| 30.0, page-keyed | 16304 | 16304 | 18487 | 18487 |

### Sensitivity to garbage collection, 16 GB

The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.

| g | store | `nvme-optimistic` per IO | `nvme-optimistic` by bytes | `commodity-ssd` per IO | `commodity-ssd` by bytes | space |
|---|---|---|---|---|---|---|
| 0.50 | log, index on disk (index amplification 13.2) | 3842-7478 | 3842-6683 | 4613-16732 | 4613-14081 | 2.0x |
| 0.50 | page log | 9422 | 9422 | 10852 | 10852 | 2.0x |
| 0.67 | log, index on disk (index amplification 10.6) | 2574-6210 | 2574-5415 | 3344-15464 | 3344-12813 | 3.0x |
| 0.67 | page log | 7221 | 7221 | 8651 | 8651 | 3.0x |
| 0.80 | log, index on disk (index amplification 7.8) | 1856-5491 | 1856-4696 | 2626-14745 | 2626-12094 | 5.0x |
| 0.80 | page log | 6121 | 6121 | 7551 | 7551 | 5.0x |

