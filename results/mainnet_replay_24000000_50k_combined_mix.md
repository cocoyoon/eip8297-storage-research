# Mainnet replay, 4 key-space parts combined

> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as 4 caches of 1/4 each, one per part, instead of one shared cache. Costs are averaged cell by cell; the verdict is recomputed (a store is named only if it wins by 1.3x or more at every end of both ranges).

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
| index write amplification | 15.3 | 15.8 | 15.5 | 15.4 | 15.5 |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 19130 | - | - | - | page (log cannot run) |
| 4 | 18208 | - | - | - | page (log cannot run) |
| 8 | 17772 | - | 21091 | - | too close (page by 1.19x to 1.19x) |
| 16 | 17438 | - | 7524-10906 | - | log (1.60x to 2.32x) |
| 32 | 17092 | - | 5766-10847 | 5766-10847 | log (1.58x to 2.96x) |
| 64 | 16410-16902 | - | 5766-10847 | 5766-10847 | log (1.51x to 2.93x) |
| 128 | 16410-16902 | 1427-3966 | 5766-10847 | 5766-10847 | log (4.14x to 11.85x) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 19130 | - | - | - | page (log cannot run) |
| 4 | 18208 | - | - | - | page (log cannot run) |
| 8 | 17772 | - | 17739 | - | too close (log by 1.00x to 1.00x) |
| 16 | 17438 | - | 7140-9782 | - | log (1.78x to 2.44x) |
| 32 | 17092 | - | 5766-9736 | 5766-9736 | log (1.76x to 2.96x) |
| 64 | 16410-16902 | - | 5766-9736 | 5766-9736 | log (1.69x to 2.93x) |
| 128 | 16410-16902 | 1427-2856 | 5766-9736 | 5766-9736 | log (5.75x to 11.85x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 74097 | - | - | - | page (log cannot run) |
| 4 | 71025 | - | - | - | page (log cannot run) |
| 8 | 69572 | - | 58366 | - | too close (log by 1.19x to 1.19x) |
| 16 | 68460 | - | 13144-24418 | - | log (2.80x to 5.21x) |
| 32 | 67306 | - | 7284-24220 | 7284-24220 | log (2.78x to 9.24x) |
| 64 | 65032-66674 | - | 7284-24220 | 7284-24220 | log (2.69x to 9.15x) |
| 128 | 65032-66674 | 2062-10529 | 7284-24220 | 7284-24220 | log (6.18x to 32.34x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 74097 | - | - | - | page (log cannot run) |
| 4 | 71025 | - | - | - | page (log cannot run) |
| 8 | 69572 | - | 47192 | - | log (1.47x to 1.47x) |
| 16 | 68460 | - | 11862-20670 | - | log (3.31x to 5.77x) |
| 32 | 67306 | - | 7284-20515 | 7284-20515 | log (3.28x to 9.24x) |
| 64 | 65032-66674 | - | 7284-20515 | 7284-20515 | log (3.17x to 9.15x) |
| 128 | 65032-66674 | 2062-6824 | 7284-20515 | 7284-20515 | log (9.53x to 32.34x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs |
|---|---|---|---|---|---|---|---|
| 4 | page | 599 | 0 | 4035 | 693199 | 0 | 5.00 |
| 8 | page | 454 | 0 | 4035 | 693199 | 0 | 5.00 |
| 8 | log, index on disk (1.5 B) | 2766 | 2554 | 0 | 11657809 | 11605891 | 1.00 |
| 16 | page | 343 | 0 | 4035 | 693199 | 0 | 5.00 |
| 16 | log, index on disk (1.5 B) | 1068 | 857 | 0 | 11657809 | 11605891 | 1.00 |
| 32 | page | 227 | 0 | 4035 | 693199 | 0 | 5.00 |
| 32 | log, index on disk (1.5 B) | 1058 | 847 | 0 | 11657809 | 11605891 | 1.00 |
