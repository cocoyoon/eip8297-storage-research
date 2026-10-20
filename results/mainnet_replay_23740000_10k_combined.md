# Mainnet replay, 4 key-space parts combined

> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as 4 caches of 1/4 each, one per part, instead of one shared cache. Costs are averaged cell by cell; the verdict is recomputed (a store is named only if it wins by 1.3x or more at every end of both ranges).

Parts: `results/mainnet_replay_23740000_10k_p0.md`, `results/mainnet_replay_23740000_10k_p1.md`, `results/mainnet_replay_23740000_10k_p2.md`, `results/mainnet_replay_23740000_10k_p3.md`

| figure | part 0 | part 1 | part 2 | part 3 | combined |
|---|---|---|---|---|---|
| log store: records written per block | 22907.9 | 14069.2 | 21600.9 | 14174.8 | 18188.2 |
| log store: reads per block with no cache | 22446.1 | 13831.8 | 21036.0 | 13890.0 | 17801.0 |
| page store: merkle pages written per block | 3070.8 | 1817.7 | 2865.9 | 1843.7 | 2399.5 |
| page store: reads per block with no cache | 5399.4 | 3729.8 | 5294.0 | 3733.5 | 4539.2 |
| log store: reads of records untouched since the replay began | 1467.3 | 1077.7 | 1601.3 | 1101.4 | 1311.9 |
| page store: reads of pages untouched since the replay began | 304.4 | 212.9 | 333.4 | 223.2 | 268.5 |
| execution reads of existing keys per block | 978.6 | 1108.4 | 933.4 | 1076.6 | 1024.2 |
| index write amplification | 15.5 | 15.6 | 15.8 | 15.5 | 15.6 |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 18877 | - | - | - | page (log cannot run) |
| 4 | 17962 | - | - | - | page (log cannot run) |
| 8 | 17517 | - | 19995 | - | too close (page by 1.14x to 1.14x) |
| 16 | 16837-17164 | - | 5720-13592 | - | too close (log by 1.24x to 3.00x) |
| 32 | 16280-17085 | - | 5720-13592 | 5720-13592 | too close (log by 1.20x to 2.99x) |
| 64 | 16280-17085 | - | 5720-13592 | 5720-13592 | too close (log by 1.20x to 2.99x) |
| 128 | 16280-17085 | 1388-5324 | 5720-13592 | 5720-13592 | log (3.06x to 12.31x) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 18877 | - | - | - | page (log cannot run) |
| 4 | 17962 | - | - | - | page (log cannot run) |
| 8 | 17517 | - | 16872 | - | too close (log by 1.04x to 1.04x) |
| 16 | 16837-17164 | - | 5720-11870 | - | log (1.42x to 3.00x) |
| 32 | 16280-17085 | - | 5720-11870 | 5720-11870 | log (1.37x to 2.99x) |
| 64 | 16280-17085 | - | 5720-11870 | 5720-11870 | log (1.37x to 2.99x) |
| 128 | 16280-17085 | 1388-3602 | 5720-11870 | 5720-11870 | log (4.52x to 12.31x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 73171 | - | - | - | page (log cannot run) |
| 4 | 70121 | - | - | - | page (log cannot run) |
| 8 | 68636 | - | 54658 | - | too close (log by 1.26x to 1.26x) |
| 16 | 66368-67458 | - | 7075-33314 | - | log (1.99x to 9.53x) |
| 32 | 64512-67197 | - | 7075-33314 | 7075-33314 | log (1.94x to 9.50x) |
| 64 | 64512-67197 | - | 7075-33314 | 7075-33314 | log (1.94x to 9.50x) |
| 128 | 64512-67197 | 1934-15053 | 7075-33314 | 7075-33314 | log (4.29x to 34.75x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 73171 | - | - | - | page (log cannot run) |
| 4 | 70121 | - | - | - | page (log cannot run) |
| 8 | 68636 | - | 44250 | - | log (1.55x to 1.55x) |
| 16 | 66368-67458 | - | 7075-27574 | - | log (2.41x to 9.53x) |
| 32 | 64512-67197 | - | 7075-27574 | 7075-27574 | log (2.34x to 9.50x) |
| 64 | 64512-67197 | - | 7075-27574 | 7075-27574 | log (2.34x to 9.50x) |
| 128 | 64512-67197 | 1934-9313 | 7075-27574 | 7075-27574 | log (6.93x to 34.75x) |
