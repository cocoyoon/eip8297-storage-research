# Mainnet replay, 4 key-space parts combined

> Each part replays the accounts and contracts of one address-hash subtree and scales its figures to the whole; together the parts cover every key once, so their figures average to the whole. A DRAM budget is taken as 4 caches of 1/4 each, one per part, instead of one shared cache. Costs are averaged cell by cell; the verdict is recomputed (a family of stores is named only if it beats every other family by 1.3x or more whichever end of its own range each is at).

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
| index write amplification | 12.7 | 13.2 | 13.0 | 12.8 | 12.9 |
| LSM of records: write amplification per record update | 4.6 | 6.6 | 5.7 | 5.0 | 5.5 |
| LSM of records: the model's write amplification per record update | 4.6 | 6.5 | 5.7 | 5.0 | 5.5 |
| index: the model's write amplification per index update | 15.3 | 15.9 | 15.6 | 15.4 | 15.6 |
| LSM of records: bytes per entry | 81.0 | 81.0 | 81.0 | 81.0 | 81.0 |
| B+tree: pages rewritten per block | 18790.8 | 12152.4 | 18101.2 | 12285.1 | 15332.4 |
| B+tree: leaf-page reads per block with no cache | 10450.8 | 6742.9 | 10130.6 | 6762.7 | 8521.8 |
| page log: pages appended per block | 4183.0 | 2445.2 | 3881.9 | 2477.5 | 3246.9 |
| page LSM: write amplification per page update | 8.3 | 9.0 | 9.2 | 8.2 | 8.7 |
| log store: relocations per block at g = 2/3 | 11377.5 | 6970.3 | 10695.7 | 7013.9 | 9014.4 |
| page store: elided-page touches per block | 1114.0 | 911.5 | 1158.1 | 895.3 | 1019.7 |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 75619 | - | - | - | 57613 | page LSM (1.31x to 1.31x over B+tree) |
| 2 | 18937 | - | - | - | - | 70185 | - | - | - | 56339 | page (2.98x to 2.98x over page LSM) |
| 4 | 18016 | - | - | - | - | 66932 | - | - | 13825 | 55724 | page log (1.30x to 1.30x over page) |
| 8 | 17567 | - | 19428 | - | - | 64946 | 17779 | - | 13072 | 55334 | page log (1.34x to 1.34x over page) |
| 16 | 17160 | - | 5046-9408 | 5252-6319 | 6829 | 63648 | 3397-7760 | 5358 | 12624 | 54942 | undecided (LSM by up to 1.48x, log by up to 1.58x) |
| 32 | 16796 | - | 5046-9408 | 4116-6298 | 5492 | 62617 | 3397-7760 | 4733 | 12225 | 54590 | undecided (LSM by up to 1.89x, log by up to 1.39x) |
| 64 | 16317-16670 | - | 5046-9408 | 4116-6298 | 4747 | 61860-61982 | 3397-7760 | 4116-4470 | 11729-12082 | 54119-54472 | undecided (LSM by up to 1.89x, log by up to 1.32x) |
| 128 | 16317-16670 | 1390-3572 | 5046-9408 | 4116-6298 | 4116-4634 | 61370-61896 | 3397-7760 | 4116-4470 | 11729-12082 | 54119-54472 | too close (log by 1.15x to 3.21x over LSM) |

### IO cost per block, mainnet replay, profile `nvme-optimistic`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 75619 | - | - | - | 57613 | page LSM (1.31x to 1.31x over B+tree) |
| 2 | 18937 | - | - | - | - | 70185 | - | - | - | 56339 | page (2.98x to 2.98x over page LSM) |
| 4 | 18016 | - | - | - | - | 66932 | - | - | 13825 | 55724 | page log (1.30x to 1.30x over page) |
| 8 | 17567 | - | 16282 | - | - | 64946 | 14633 | - | 13072 | 55334 | too close (page log by 1.12x to 1.12x over log) |
| 16 | 17160 | - | 5046-8454 | 5252-6319 | 6829 | 63648 | 3397-6806 | 5358 | 12624 | 54942 | undecided (LSM by up to 1.30x, log by up to 1.58x) |
| 32 | 16796 | - | 5046-8454 | 4116-6298 | 5492 | 62617 | 3397-6806 | 4733 | 12225 | 54590 | undecided (LSM by up to 1.65x, log by up to 1.39x) |
| 64 | 16317-16670 | - | 5046-8454 | 4116-6298 | 4747 | 61860-61982 | 3397-6806 | 4116-4470 | 11729-12082 | 54119-54472 | undecided (LSM by up to 1.65x, log by up to 1.32x) |
| 128 | 16317-16670 | 1390-2618 | 5046-8454 | 4116-6298 | 4116-4634 | 61370-61896 | 3397-6806 | 4116-4470 | 11729-12082 | 54119-54472 | log (1.57x to 3.21x over LSM) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced per IO

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 292935 | - | - | - | 65808 | page LSM (4.45x to 4.45x over B+tree) |
| 2 | 73394 | - | - | - | - | 274824 | - | - | - | 61560 | too close (page LSM by 1.19x to 1.19x over page) |
| 4 | 70326 | - | - | - | - | 263979 | - | - | 18796 | 59512 | page log (3.17x to 3.17x over page LSM) |
| 8 | 68826 | - | 54338 | - | - | 257358 | 52689 | - | 16286 | 58209 | page log (3.24x to 3.24x over log) |
| 16 | 67470 | - | 6399-20942 | 8446-12004 | 13703 | 253033 | 4750-19293 | 8802 | 14792 | 56902 | undecided (LSM by up to 2.28x, log by up to 1.85x) |
| 32 | 66254 | - | 6399-20942 | 4662-11933 | 9247 | 249597 | 4750-19293 | 6716 | 13461 | 55728 | undecided (LSM by up to 4.14x, log by up to 1.41x) |
| 64 | 64661-65838 | - | 6399-20942 | 4662-11933 | 6763 | 247073-247478 | 4750-19293 | 4662-5838 | 11809-12986 | 54159-55336 | undecided (LSM by up to 4.14x, log by up to 1.23x) |
| 128 | 64661-65838 | 1936-9207 | 6399-20942 | 4662-11933 | 4662-6388 | 245438-247191 | 4750-19293 | 4662-5838 | 11809-12986 | 54159-55336 | undecided (LSM by up to 1.98x, log by up to 3.02x) |

### IO cost per block, mainnet replay, profile `commodity-ssd`, reads priced by bytes

| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk (1.5 B) | LSM, node cache | LSM, block cache | B+tree, copy-on-write | log, index on disk, GC at 2/3 | LSM, page-keyed | page log, GC at 1/2 | page LSM | cheapest |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | - | - | - | - | - | 292935 | - | - | - | 65808 | page LSM (4.45x to 4.45x over B+tree) |
| 2 | 73394 | - | - | - | - | 274824 | - | - | - | 61560 | too close (page LSM by 1.19x to 1.19x over page) |
| 4 | 70326 | - | - | - | - | 263979 | - | - | 18796 | 59512 | page log (3.17x to 3.17x over page LSM) |
| 8 | 68826 | - | 43851 | - | - | 257358 | 42202 | - | 16286 | 58209 | page log (2.59x to 2.59x over log) |
| 16 | 67470 | - | 6399-17761 | 8446-12004 | 13703 | 253033 | 4750-16112 | 8802 | 14792 | 56902 | undecided (LSM by up to 1.91x, log by up to 1.85x) |
| 32 | 66254 | - | 6399-17761 | 4662-11933 | 9247 | 249597 | 4750-16112 | 6716 | 13461 | 55728 | undecided (LSM by up to 3.46x, log by up to 1.41x) |
| 64 | 64661-65838 | - | 6399-17761 | 4662-11933 | 6763 | 247073-247478 | 4750-16112 | 4662-5838 | 11809-12986 | 54159-55336 | undecided (LSM by up to 3.46x, log by up to 1.23x) |
| 128 | 64661-65838 | 1936-6026 | 6399-17761 | 4662-11933 | 4662-6388 | 245438-247191 | 4750-16112 | 4662-5838 | 11809-12986 | 54159-55336 | undecided (LSM by up to 1.29x, log by up to 3.02x) |

### IO mix per block, for timed IO (`hi` bound)

| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |
|---|---|---|---|---|---|---|---|---|
| 4 | page | 566 | 0 | 4012 | 693704 | 0 | 5.00 | 94 |
| 4 | B+tree, copy-on-write | 1854 | 0 | 15332 | 0 | 0 | 2.00 | 197 |
| 4 | page log, GC at 1/2 | 699 | 0 | 0 | 26598720 | 26598720 | 2.00 | 120 |
| 4 | page LSM | 535 | 0 | 0 | 129038427 | 115687117 | 1.00 | 89 |
| 8 | page | 416 | 0 | 4012 | 693704 | 0 | 5.00 | 66 |
| 8 | log, index on disk (1.5 B) | 2585 | 2397 | 0 | 10163002 | 10146548 | 1.00 | 70 |
| 8 | B+tree, copy-on-write | 1192 | 0 | 15332 | 0 | 0 | 2.00 | 119 |
| 8 | log, index on disk, GC at 2/3 | 2585 | 2397 | 0 | 6411304 | 6394849 | 1.00 | 70 |
| 8 | page log, GC at 1/2 | 448 | 0 | 0 | 26598720 | 26598720 | 2.00 | 72 |
| 8 | page LSM | 405 | 0 | 0 | 129038427 | 115687117 | 1.00 | 64 |
| 16 | page | 281 | 0 | 4012 | 693704 | 0 | 5.00 | 40 |
| 16 | log, index on disk (1.5 B) | 914 | 727 | 0 | 10163002 | 10146548 | 1.00 | 21 |
| 16 | LSM, node cache | 806 | 0 | 0 | 9486838 | 8006081 | 1.00 | 21 |
| 16 | LSM, block cache | 976 | 0 | 0 | 9486838 | 8006081 | 1.00 | 85 |
| 16 | B+tree, copy-on-write | 760 | 0 | 15332 | 0 | 0 | 2.00 | 74 |
| 16 | log, index on disk, GC at 2/3 | 914 | 727 | 0 | 6411304 | 6394849 | 1.00 | 21 |
| 16 | LSM, page-keyed | 486 | 0 | 0 | 9486838 | 8006081 | 1.00 | 65 |
| 16 | page log, GC at 1/2 | 298 | 0 | 0 | 26598720 | 26598720 | 2.00 | 43 |
| 16 | page LSM | 274 | 0 | 0 | 129038427 | 115687117 | 1.00 | 39 |
| 32 | page | 159 | 0 | 4012 | 693704 | 0 | 5.00 | 19 |
| 32 | log, index on disk (1.5 B) | 914 | 727 | 0 | 10163002 | 10146548 | 1.00 | 21 |
| 32 | LSM, node cache | 800 | 0 | 0 | 9486838 | 8006081 | 1.00 | 21 |
| 32 | LSM, block cache | 531 | 0 | 0 | 9486838 | 8006081 | 1.00 | 41 |
| 32 | B+tree, copy-on-write | 416 | 0 | 15332 | 0 | 0 | 2.00 | 38 |
| 32 | log, index on disk, GC at 2/3 | 914 | 727 | 0 | 6411304 | 6394849 | 1.00 | 21 |
| 32 | LSM, page-keyed | 278 | 0 | 0 | 9486838 | 8006081 | 1.00 | 26 |
| 32 | page log, GC at 1/2 | 165 | 0 | 0 | 26598720 | 26598720 | 2.00 | 19 |
| 32 | page LSM | 157 | 0 | 0 | 129038427 | 115687117 | 1.00 | 18 |
