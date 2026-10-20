# Mainnet replay, 4 key-space parts combined

> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as 4 caches of 1/4 each, one per part, instead of one shared cache. Costs are averaged cell by cell; the verdict is recomputed (a store is named only if it wins by 1.3x or more at every end of both ranges).

Parts: `results/mainnet_replay_24040000_10k_p0.md`, `results/mainnet_replay_24040000_10k_p1.md`, `results/mainnet_replay_24040000_10k_p2.md`, `results/mainnet_replay_24040000_10k_p3.md`

| figure | part 0 | part 1 | part 2 | part 3 | combined |
|---|---|---|---|---|---|
| log store: records written per block | 21987.4 | 17235.7 | 21807.3 | 12670.4 | 18425.2 |
| log store: reads per block with no cache | 21520.6 | 15421.8 | 21230.5 | 12169.1 | 17585.5 |
| page store: merkle pages written per block | 2974.4 | 2089.7 | 2862.5 | 1646.8 | 2393.3 |
| page store: reads per block with no cache | 5391.7 | 3946.3 | 5345.1 | 3191.5 | 4468.6 |
| log store: reads of records untouched since the replay began | 1318.1 | 1830.0 | 1454.6 | 1013.3 | 1404.0 |
| page store: reads of pages untouched since the replay began | 303.3 | 433.6 | 311.9 | 211.5 | 315.1 |
| execution reads of existing keys per block | 1013.9 | 858.3 | 981.1 | 866.0 | 929.8 |
| index write amplification | 15.4 | 15.8 | 15.6 | 15.5 | 15.6 |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 19049 | - | - | - | page (log cannot run) |
| 4 | 18135 | - | - | - | page (log cannot run) |
| 8 | 17708 | - | 20888 | - | too close (page by 1.18x to 1.18x) |
| 16 | 17159-17394 | - | 5783-14207 | - | too close (log by 1.21x to 3.01x) |
| 32 | 16368-17313 | - | 5783-14207 | 5783-14207 | too close (log by 1.15x to 2.99x) |
| 64 | 16368-17313 | - | 5783-14207 | 5783-14207 | too close (log by 1.15x to 2.99x) |
| 128 | 16368-17313 | 1425-5637 | 5783-14207 | 5783-14207 | log (2.90x to 12.15x) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 19049 | - | - | - | page (log cannot run) |
| 4 | 18135 | - | - | - | page (log cannot run) |
| 8 | 17708 | - | 17584 | - | too close (log by 1.01x to 1.01x) |
| 16 | 17159-17394 | - | 5783-12364 | - | log (1.39x to 3.01x) |
| 32 | 16368-17313 | - | 5783-12364 | 5783-12364 | log (1.32x to 2.99x) |
| 64 | 16368-17313 | - | 5783-12364 | 5783-12364 | log (1.32x to 2.99x) |
| 128 | 16368-17313 | 1425-3794 | 5783-12364 | 5783-12364 | log (4.31x to 12.15x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 73801 | - | - | - | page (log cannot run) |
| 4 | 70755 | - | - | - | page (log cannot run) |
| 8 | 69332 | - | 57662 | - | too close (log by 1.20x to 1.20x) |
| 16 | 67502-68284 | - | 7308-35388 | - | log (1.91x to 9.34x) |
| 32 | 64865-68016 | - | 7308-35388 | 7308-35388 | log (1.83x to 9.31x) |
| 64 | 64865-68016 | - | 7308-35388 | 7308-35388 | log (1.83x to 9.31x) |
| 128 | 64865-68016 | 2064-16104 | 7308-35388 | 7308-35388 | log (4.03x to 32.95x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 73801 | - | - | - | page (log cannot run) |
| 4 | 70755 | - | - | - | page (log cannot run) |
| 8 | 69332 | - | 46647 | - | log (1.49x to 1.49x) |
| 16 | 67502-68284 | - | 7308-29246 | - | log (2.31x to 9.34x) |
| 32 | 64865-68016 | - | 7308-29246 | 7308-29246 | log (2.22x to 9.31x) |
| 64 | 64865-68016 | - | 7308-29246 | 7308-29246 | log (2.22x to 9.31x) |
| 128 | 64865-68016 | 2064-9962 | 7308-29246 | 7308-29246 | log (6.51x to 32.95x) |
