# Mainnet replay, 4 key-space parts combined

> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as 4 caches of 1/4 each, one per part, instead of one shared cache. Costs are averaged cell by cell; the verdict is recomputed (a store is named only if it wins by 1.3x or more at every end of both ranges).

Parts: `results/mainnet_replay_23700000_50k_p0.md`, `results/mainnet_replay_23700000_50k_p1.md`, `results/mainnet_replay_23700000_50k_p2.md`, `results/mainnet_replay_23700000_50k_p3.md`

| figure | part 0 | part 1 | part 2 | part 3 | combined |
|---|---|---|---|---|---|
| log store: records written per block | 22952.4 | 14098.9 | 21642.1 | 14217.9 | 18227.8 |
| log store: reads per block with no cache | 22490.7 | 13861.5 | 21077.2 | 13933.1 | 17840.6 |
| page store: merkle pages written per block | 3079.2 | 1826.4 | 2872.9 | 1850.2 | 2407.2 |
| page store: reads per block with no cache | 5408.7 | 3739.5 | 5301.9 | 3741.0 | 4547.8 |
| log store: reads of records untouched since the replay began | 807.3 | 606.0 | 879.2 | 616.1 | 727.1 |
| page store: reads of pages untouched since the replay began | 127.3 | 104.8 | 127.9 | 110.8 | 117.7 |
| execution reads of existing keys per block | 978.6 | 1108.4 | 933.4 | 1076.6 | 1024.2 |
| index write amplification | 15.3 | 15.9 | 15.6 | 15.4 | 15.6 |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 18937 | - | - | - | page (log cannot run) |
| 4 | 18016 | - | - | - | page (log cannot run) |
| 8 | 17567 | - | 20097 | - | too close (page by 1.14x to 1.14x) |
| 16 | 17160 | - | 5715-10078 | - | log (1.70x to 3.00x) |
| 32 | 16796 | - | 5715-10078 | 5715-10078 | log (1.67x to 2.94x) |
| 64 | 16317-16670 | - | 5715-10078 | 5715-10078 | log (1.62x to 2.92x) |
| 128 | 16317-16670 | 1390-3572 | 5715-10078 | 5715-10078 | log (4.57x to 11.99x) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 18937 | - | - | - | page (log cannot run) |
| 4 | 18016 | - | - | - | page (log cannot run) |
| 8 | 17567 | - | 16951 | - | too close (log by 1.04x to 1.04x) |
| 16 | 17160 | - | 5715-9124 | - | log (1.88x to 3.00x) |
| 32 | 16796 | - | 5715-9124 | 5715-9124 | log (1.84x to 2.94x) |
| 64 | 16317-16670 | - | 5715-9124 | 5715-9124 | log (1.79x to 2.92x) |
| 128 | 16317-16670 | 1390-2618 | 5715-9124 | 5715-9124 | log (6.23x to 11.99x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 73394 | - | - | - | page (log cannot run) |
| 4 | 70326 | - | - | - | page (log cannot run) |
| 8 | 68826 | - | 55007 | - | too close (log by 1.25x to 1.25x) |
| 16 | 67470 | - | 7068-21612 | - | log (3.12x to 9.55x) |
| 32 | 66254 | - | 7068-21612 | 7068-21612 | log (3.07x to 9.37x) |
| 64 | 64661-65838 | - | 7068-21612 | 7068-21612 | log (2.99x to 9.31x) |
| 128 | 64661-65838 | 1936-9207 | 7068-21612 | 7068-21612 | log (7.02x to 34.02x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | log, index on disk (4.25 B) | cheapest |
|---|---|---|---|---|---|
| 1 | - | - | - | - | - |
| 2 | 73394 | - | - | - | page (log cannot run) |
| 4 | 70326 | - | - | - | page (log cannot run) |
| 8 | 68826 | - | 44520 | - | log (1.55x to 1.55x) |
| 16 | 67470 | - | 7068-18430 | - | log (3.66x to 9.55x) |
| 32 | 66254 | - | 7068-18430 | 7068-18430 | log (3.59x to 9.37x) |
| 64 | 64661-65838 | - | 7068-18430 | 7068-18430 | log (3.51x to 9.31x) |
| 128 | 64661-65838 | 1936-6026 | 7068-18430 | 7068-18430 | log (10.73x to 34.02x) |
