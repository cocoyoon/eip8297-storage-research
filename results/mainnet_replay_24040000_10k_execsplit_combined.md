# Mainnet replay, 4 key-space parts combined

> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as 4 caches of 1/4 each, one per part, instead of one shared cache. Costs are averaged cell by cell; the verdict is recomputed (a store is named only if it wins by 1.3x or more at every end of both ranges).

Parts: `p0.md`, `p1.md`, `p2.md`, `p3.md`

| figure | part 0 | part 1 | part 2 | part 3 | combined |
|---|---|---|---|---|---|
| log store: records written per block | 21996.5 | 17248.9 | 21816.6 | 12679.6 | 18435.4 |
| log store: reads per block with no cache | 21529.7 | 15434.9 | 21239.8 | 12178.3 | 17595.7 |
| page store: merkle pages written per block | 2975.5 | 2091.2 | 2863.6 | 1647.9 | 2394.6 |
| page store: reads per block with no cache | 5393.3 | 3948.7 | 5346.9 | 3193.2 | 4470.5 |
| log store: reads of records untouched since the replay began | 1322.1 | 1837.2 | 1459.0 | 1017.7 | 1409.0 |
| page store: reads of pages untouched since the replay began | 304.4 | 435.6 | 313.2 | 212.8 | 316.5 |
| execution reads of existing keys per block | 1013.9 | 858.3 | 981.1 | 866.0 | 929.8 |
| index write amplification | 15.4 | 15.8 | 15.6 | 15.5 | 15.6 |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 19068 | - | - | - | page (log cannot run) |
| 4 | 18152 | - | - | - | page (log cannot run) |
| 8 | 17723 | - | 20952 | - | too close (page by 1.18x to 1.18x) |
| 16 | 17188-17408 | - | 5787-14241 | - | too close (log by 1.21x to 3.01x) |
| 32 | 16376-17326 | - | 5787-14241 | 5787-14241 | too close (log by 1.15x to 2.99x) |
| 64 | 16376-17326 | - | 5787-14241 | 5787-14241 | too close (log by 1.15x to 2.99x) |
| 128 | 16376-17326 | 1426-5653 | 5787-14241 | 5787-14241 | log (2.90x to 12.15x) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 19068 | - | - | - | page (log cannot run) |
| 4 | 18152 | - | - | - | page (log cannot run) |
| 8 | 17723 | - | 17635 | - | too close (log by 1.01x to 1.01x) |
| 16 | 17188-17408 | - | 5787-12392 | - | log (1.39x to 3.01x) |
| 32 | 16376-17326 | - | 5787-12392 | 5787-12392 | log (1.32x to 2.99x) |
| 64 | 16376-17326 | - | 5787-12392 | 5787-12392 | log (1.32x to 2.99x) |
| 128 | 16376-17326 | 1426-3804 | 5787-12392 | 5787-12392 | log (4.31x to 12.15x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 73870 | - | - | - | page (log cannot run) |
| 4 | 70818 | - | - | - | page (log cannot run) |
| 8 | 69388 | - | 57864 | - | too close (log by 1.20x to 1.20x) |
| 16 | 67606-68336 | - | 7313-35494 | - | log (1.90x to 9.34x) |
| 32 | 64897-68062 | - | 7313-35494 | 7313-35494 | log (1.83x to 9.31x) |
| 64 | 64897-68062 | - | 7313-35494 | 7313-35494 | log (1.83x to 9.31x) |
| 128 | 64897-68062 | 2065-16155 | 7313-35494 | 7313-35494 | log (4.02x to 32.96x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 73870 | - | - | - | page (log cannot run) |
| 4 | 70818 | - | - | - | page (log cannot run) |
| 8 | 69388 | - | 46806 | - | log (1.48x to 1.48x) |
| 16 | 67606-68336 | - | 7313-29329 | - | log (2.31x to 9.34x) |
| 32 | 64897-68062 | - | 7313-29329 | 7313-29329 | log (2.21x to 9.31x) |
| 64 | 64897-68062 | - | 7313-29329 | 7313-29329 | log (2.21x to 9.31x) |
| 128 | 64897-68062 | 2065-9990 | 7313-29329 | 7313-29329 | log (6.50x to 32.96x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 592 | 0 | 4026 | 691688 | 0 | 5.00 | 121 |
| 8 | page | 449 | 0 | 4026 | 691688 | 0 | 5.00 | 90 |
| 8 | log, index on disk (1.5 B) | 2740 | 2528 | 0 | 11697039 | 11645140 | 1.00 | 59 |
| 16 | page | 344 | 0 | 4026 | 691688 | 0 | 5.00 | 70 |
| 16 | log, index on disk (1.5 B) | 1622 | 1409 | 0 | 11697039 | 11645140 | 1.00 | 27 |
| 32 | page | 316 | 0 | 4026 | 691688 | 0 | 5.00 | 63 |
| 32 | log, index on disk (1.5 B) | 1622 | 1409 | 0 | 11697039 | 11645140 | 1.00 | 27 |
