# Mainnet replay, 4 key-space parts combined

> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as 4 caches of 1/4 each, one per part, instead of one shared cache. Costs are averaged cell by cell; the verdict is recomputed (a family of stores is named only if it beats every other family by 1.3x or more whichever end of its own range each is at).

Parts: `results/mainnet_replay_24000000_50k_p0.md`, `results/mainnet_replay_24000000_50k_p1.md`, `results/mainnet_replay_24000000_50k_p2.md`, `results/mainnet_replay_24000000_50k_p3.md`

| figure | part 0 | part 1 | part 2 | part 3 | combined |
|---|---|---|---|---|---|
| log store: records written per block | 22034.4 | 17275.9 | 21856.8 | 12729.2 | 18474.1 |
| log store: reads per block with no cache | 21567.6 | 15462.0 | 21279.9 | 12227.8 | 17634.3 |
| page store: merkle pages written per block | 2978.9 | 2096.7 | 2867.9 | 1658.9 | 2400.6 |
| page store: reads per block with no cache | 5397.8 | 3954.9 | 5352.1 | 3205.6 | 4477.6 |
| log store: reads of records untouched since the replay began | 786.8 | 1132.3 | 849.8 | 618.1 | 846.8 |
| page store: reads of pages untouched since the replay began | 148.4 | 247.8 | 141.8 | 119.0 | 164.2 |
| execution reads of existing keys per block | 1013.9 | 858.3 | 981.1 | 866.0 | 929.8 |
| index write amplification | 12.7 | 13.2 | 12.9 | 12.8 | 12.9 |
| LSM of records: write amplification per record update | 4.6 | 7.4 | 5.0 | 4.9 | 5.5 |
| LSM of records: the model's write amplification per record update | 4.5 | 7.4 | 5.0 | 4.9 | 5.5 |
| index: the model's write amplification per index update | 15.3 | 15.8 | 15.5 | 15.4 | 15.5 |
| LSM of records: bytes per entry | 81.0 | 82.0 | 81.0 | 81.0 | 81.2 |
| B+tree: pages rewritten per block | 18151.8 | 14218.8 | 18058.1 | 11086.5 | 15378.8 |
| B+tree: leaf-page reads per block with no cache | 10261.5 | 7611.1 | 10220.6 | 5886.7 | 8495.0 |
| page log: pages appended per block | 4059.5 | 2837.3 | 3903.1 | 2221.7 | 3255.4 |
| page LSM: write amplification per page update | 8.3 | 9.4 | 8.6 | 7.9 | 8.6 |
| log store: relocations per block at g = 2/3 | 10887.2 | 7871.0 | 10750.9 | 6225.1 | 8933.5 |
| page store: elided-page touches per block | 1080.2 | 1151.7 | 1206.5 | 834.7 | 1068.3 |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 76806 | - | - | - | 57172 | page LSM (1.34x to 1.34x over B+tree) |
| 2 | 19130 | - | - | - | - | 70896 | - | - | - | 55937 | page (2.92x to 2.92x over page LSM) |
| 4 | 18208 | - | - | - | - | 67864 | - | - | 13964 | 55304 | page log (1.30x to 1.30x over page) |
| 8 | 17772 | - | 20433 | - | - | 65794 | 18788 | - | 13208 | 54946 | page log (1.35x to 1.35x over page) |
| 16 | 17438 | - | 6866-10248 | 5854-6908 | 7359 | 64378 | 5221-8603 | 5602 | 12834 | 54624 | undecided (LSM by up to 1.54x, log by up to 1.07x) |
| 32 | 17092 | - | 5108-10189 | 4246-6786 | 6073 | 63294 | 3463-8544 | 5067 | 12462 | 54286 | undecided (LSM by up to 2.01x, log by up to 1.46x) |
| 64 | 16410-16902 | - | 5108-10189 | 4246-6786 | 5284 | 62431-62568 | 3463-8544 | 4246-4739 | 11759-12252 | 53611-54103 | undecided (LSM by up to 2.01x, log by up to 1.37x) |
| 128 | 16410-16902 | 1427-3966 | 5108-10189 | 4246-6786 | 4246-5034 | 61555-62354 | 3463-8544 | 4246-4739 | 11759-12252 | 53611-54103 | too close (log by 1.07x to 3.32x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 76806 | - | - | - | 57172 | page LSM (1.34x to 1.34x over B+tree) |
| 2 | 19130 | - | - | - | - | 70896 | - | - | - | 55937 | page (2.92x to 2.92x over page LSM) |
| 4 | 18208 | - | - | - | - | 67864 | - | - | 13964 | 55304 | page log (1.30x to 1.30x over page) |
| 8 | 17772 | - | 17080 | - | - | 65794 | 15436 | - | 13208 | 54946 | too close (page log by 1.17x to 1.17x over log) |
| 16 | 17438 | - | 6482-9124 | 5854-6908 | 7359 | 64378 | 4836-7479 | 5602 | 12834 | 54624 | undecided (LSM by up to 1.34x, log by up to 1.16x) |
| 32 | 17092 | - | 5108-9078 | 4246-6786 | 6073 | 63294 | 3463-7432 | 5067 | 12462 | 54286 | undecided (LSM by up to 1.75x, log by up to 1.46x) |
| 64 | 16410-16902 | - | 5108-9078 | 4246-6786 | 5284 | 62431-62568 | 3463-7432 | 4246-4739 | 11759-12252 | 53611-54103 | undecided (LSM by up to 1.75x, log by up to 1.37x) |
| 128 | 16410-16902 | 1427-2856 | 5108-9078 | 4246-6786 | 4246-5034 | 61555-62354 | 3463-7432 | 4246-4739 | 11759-12252 | 53611-54103 | log (1.49x to 3.32x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 297018 | - | - | - | 65520 | page LSM (4.53x to 4.53x over B+tree) |
| 2 | 74097 | - | - | - | - | 277315 | - | - | - | 61406 | too close (page LSM by 1.21x to 1.21x over page) |
| 4 | 71025 | - | - | - | - | 267211 | - | - | 19189 | 59297 | page log (3.09x to 3.09x over page LSM) |
| 8 | 69572 | - | 57708 | - | - | 260310 | 56063 | - | 16669 | 58100 | page log (3.36x to 3.36x over log) |
| 16 | 68460 | - | 12486-23760 | 10241-13753 | 15260 | 255588 | 10841-22115 | 9400 | 15420 | 57028 | too close (LSM by 1.15x to 2.35x over log) |
| 32 | 67306 | - | 6627-23562 | 4881-13348 | 10972 | 251978 | 4982-21916 | 7618 | 14181 | 55902 | undecided (LSM by up to 4.49x, log by up to 1.53x) |
| 64 | 65032-66674 | - | 6627-23562 | 4881-13348 | 8340 | 249101-249557 | 4982-21916 | 4881-6523 | 11839-13482 | 53651-55293 | undecided (LSM by up to 4.49x, log by up to 1.31x) |
| 128 | 65032-66674 | 2062-10529 | 6627-23562 | 4881-13348 | 4881-7510 | 246181-248843 | 4982-21916 | 4881-6523 | 11839-13482 | 53651-55293 | undecided (LSM by up to 2.16x, log by up to 3.16x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 297018 | - | - | - | 65520 | page LSM (4.53x to 4.53x over B+tree) |
| 2 | 74097 | - | - | - | - | 277315 | - | - | - | 61406 | too close (page LSM by 1.21x to 1.21x over page) |
| 4 | 71025 | - | - | - | - | 267211 | - | - | 19189 | 59297 | page log (3.09x to 3.09x over page LSM) |
| 8 | 69572 | - | 46534 | - | - | 260310 | 44889 | - | 16669 | 58100 | page log (2.69x to 2.69x over log) |
| 16 | 68460 | - | 11204-20012 | 10241-13753 | 15260 | 255588 | 9559-18366 | 9400 | 15420 | 57028 | too close (LSM by 1.02x to 1.95x over log) |
| 32 | 67306 | - | 6627-19857 | 4881-13348 | 10972 | 251978 | 4982-18212 | 7618 | 14181 | 55902 | undecided (LSM by up to 3.73x, log by up to 1.53x) |
| 64 | 65032-66674 | - | 6627-19857 | 4881-13348 | 8340 | 249101-249557 | 4982-18212 | 4881-6523 | 11839-13482 | 53651-55293 | undecided (LSM by up to 3.73x, log by up to 1.31x) |
| 128 | 65032-66674 | 2062-6824 | 6627-19857 | 4881-13348 | 4881-7510 | 246181-248843 | 4982-18212 | 4881-6523 | 11839-13482 | 53651-55293 | undecided (LSM by up to 1.40x, log by up to 3.16x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 599 | 0 | 4035 | 693199 | 0 | 5.00 | 123 |
| 4 | B+tree, copy-on-write | 2103 | 0 | 15379 | 0 | 0 | 2.00 | 248 |
| 4 | page log, GC at 1/2 | 735 | 0 | 0 | 26668262 | 26668262 | 2.00 | 148 |
| 4 | page LSM | 564 | 0 | 0 | 127898202 | 114511985 | 1.00 | 115 |
| 8 | page | 454 | 0 | 4035 | 693199 | 0 | 5.00 | 91 |
| 8 | log, index on disk (1.5 B) | 2766 | 2554 | 0 | 10160452 | 10108534 | 1.00 | 60 |
| 8 | B+tree, copy-on-write | 1413 | 0 | 15379 | 0 | 0 | 2.00 | 166 |
| 8 | log, index on disk, GC at 2/3 | 2766 | 2554 | 0 | 6416543 | 6364625 | 1.00 | 60 |
| 8 | page log, GC at 1/2 | 483 | 0 | 0 | 26668262 | 26668262 | 2.00 | 98 |
| 8 | page LSM | 445 | 0 | 0 | 127898202 | 114511985 | 1.00 | 90 |
| 16 | page | 343 | 0 | 4035 | 693199 | 0 | 5.00 | 70 |
| 16 | log, index on disk (1.5 B) | 1068 | 857 | 0 | 10160452 | 10108534 | 1.00 | 16 |
| 16 | LSM, node cache | 972 | 0 | 0 | 9704884 | 8199774 | 1.00 | 16 |
| 16 | LSM, block cache | 1123 | 0 | 0 | 9704884 | 8199774 | 1.00 | 111 |
| 16 | B+tree, copy-on-write | 941 | 0 | 15379 | 0 | 0 | 2.00 | 116 |
| 16 | log, index on disk, GC at 2/3 | 1068 | 857 | 0 | 6416543 | 6364625 | 1.00 | 16 |
| 16 | LSM, page-keyed | 537 | 0 | 0 | 9704884 | 8199774 | 1.00 | 91 |
| 16 | page log, GC at 1/2 | 358 | 0 | 0 | 26668262 | 26668262 | 2.00 | 73 |
| 16 | page LSM | 338 | 0 | 0 | 127898202 | 114511985 | 1.00 | 69 |
| 32 | page | 227 | 0 | 4035 | 693199 | 0 | 5.00 | 49 |
| 32 | log, index on disk (1.5 B) | 1058 | 847 | 0 | 10160452 | 10108534 | 1.00 | 16 |
| 32 | LSM, node cache | 932 | 0 | 0 | 9704884 | 8199774 | 1.00 | 16 |
| 32 | LSM, block cache | 694 | 0 | 0 | 9704884 | 8199774 | 1.00 | 72 |
| 32 | B+tree, copy-on-write | 580 | 0 | 15379 | 0 | 0 | 2.00 | 72 |
| 32 | log, index on disk, GC at 2/3 | 1058 | 847 | 0 | 6416543 | 6364625 | 1.00 | 16 |
| 32 | LSM, page-keyed | 359 | 0 | 0 | 9704884 | 8199774 | 1.00 | 57 |
| 32 | page log, GC at 1/2 | 234 | 0 | 0 | 26668262 | 26668262 | 2.00 | 50 |
| 32 | page LSM | 225 | 0 | 0 | 127898202 | 114511985 | 1.00 | 48 |
