# Mainnet replay, 4 key-space parts combined

> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as 4 caches of 1/4 each, one per part, instead of one shared cache. Costs are averaged cell by cell; the verdict is recomputed (a store is named only if it wins by 1.3x or more at every end of both ranges).

Parts: `results/mainnet_replay_24000000_50k_p0.md`, `results/mainnet_replay_24000000_50k_p1.md`, `results/mainnet_replay_24000000_50k_p2.md`, `results/mainnet_replay_24000000_50k_p3.md`

| figure | part 0 | part 1 | part 2 | part 3 | combined |
|---|---|---|---|---|---|
| log store: records written per block | 22026.9 | 17265.2 | 21849.4 | 12722.0 | 18465.9 |
| log store: reads per block with no cache | 21560.1 | 15451.3 | 21272.6 | 12220.6 | 17626.1 |
| page store: merkle pages written per block | 2977.9 | 2095.2 | 2867.0 | 1657.9 | 2399.5 |
| page store: reads per block with no cache | 5396.4 | 3953.0 | 5350.7 | 3204.2 | 4476.1 |
| log store: reads of records untouched since the replay began | 784.1 | 1127.9 | 847.4 | 615.6 | 843.8 |
| page store: reads of pages untouched since the replay began | 147.8 | 246.7 | 141.2 | 118.2 | 163.5 |
| execution reads of existing keys per block | 1013.9 | 858.3 | 981.1 | 866.0 | 929.8 |
| index write amplification | 15.3 | 15.8 | 15.5 | 15.4 | 15.5 |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 19114 | - | - | - | page (log cannot run) |
| 4 | 18193 | - | - | - | page (log cannot run) |
| 8 | 17758 | - | 21045 | - | too close (page by 1.19x to 1.19x) |
| 16 | 17425 | - | 7513-10884 | - | log (1.60x to 2.32x) |
| 32 | 17081 | - | 5763-10826 | 5763-10826 | log (1.58x to 2.96x) |
| 64 | 16404-16894 | - | 5763-10826 | 5763-10826 | log (1.52x to 2.93x) |
| 128 | 16404-16894 | 1426-3957 | 5763-10826 | 5763-10826 | log (4.15x to 11.84x) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 19114 | - | - | - | page (log cannot run) |
| 4 | 18193 | - | - | - | page (log cannot run) |
| 8 | 17758 | - | 17702 | - | too close (log by 1.00x to 1.00x) |
| 16 | 17425 | - | 7130-9764 | - | log (1.78x to 2.44x) |
| 32 | 17081 | - | 5763-9718 | 5763-9718 | log (1.76x to 2.96x) |
| 64 | 16404-16894 | - | 5763-9718 | 5763-9718 | log (1.69x to 2.93x) |
| 128 | 16404-16894 | 1426-2850 | 5763-9718 | 5763-9718 | log (5.76x to 11.84x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 74040 | - | - | - | page (log cannot run) |
| 4 | 70972 | - | - | - | page (log cannot run) |
| 8 | 69523 | - | 58220 | - | too close (log by 1.19x to 1.19x) |
| 16 | 68413 | - | 13114-24350 | - | log (2.81x to 5.22x) |
| 32 | 67264 | - | 7282-24156 | 7282-24156 | log (2.78x to 9.24x) |
| 64 | 65006-66641 | - | 7282-24156 | 7282-24156 | log (2.69x to 9.15x) |
| 128 | 65006-66641 | 2061-10498 | 7282-24156 | 7282-24156 | log (6.19x to 32.33x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 74040 | - | - | - | page (log cannot run) |
| 4 | 70972 | - | - | - | page (log cannot run) |
| 8 | 69523 | - | 47077 | - | log (1.48x to 1.48x) |
| 16 | 68413 | - | 11838-20616 | - | log (3.32x to 5.78x) |
| 32 | 67264 | - | 7282-20465 | 7282-20465 | log (3.29x to 9.24x) |
| 64 | 65006-66641 | - | 7282-20465 | 7282-20465 | log (3.18x to 9.15x) |
| 128 | 65006-66641 | 2061-6807 | 7282-20465 | 7282-20465 | log (9.55x to 32.33x) |
