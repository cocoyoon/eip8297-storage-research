# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256b8

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

| setting | value |
|---|---|
| write buffer | 256 MB |
| L0 files before compaction into L1 | 2 |
| level base (`max_bytes_for_level_base`, dynamic level bytes) | 64 MB |
| level multiplier | 10 |
| target file size | 64 MB |
| compression | snappy |
| bloom filter | 10 bits per key |
| block cache | 64 MB |
| WAL sync per block | false |
| background jobs | 8 |
| key layout | Path |

## Starting state

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 14349.7 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 58s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 7823.7 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 209877148 live entries tracked (219855830 estimated by the engine), 15188.2 MB on disk (72.4 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 1.8s; replay 722s, whole run 722s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723513 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8138562931 | 77.4 | 1.04 | 406928 |
| flush | 1519502567 | 14.5 | 0.19 | 75975 |
| compaction write | 5595910776 | 53.2 | 0.72 | 279796 |
| compaction read | 6572371690 | 62.5 | 0.84 | 328619 |
| user bytes (engine's count) | 8138562931 | 77.42 | - | 406928 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 7115413343 | 67.7 | **0.91** | 355771 |

- write amplification, engine: **0.91** bytes written per user byte, **0.94** entries of 72.4 B (the on-disk entry) per update; compaction reads 0.84 per user byte, 0.86 entries per update. Disk grew by 491.4 MB over the window.
- whole replay, levels filling included: 0.80 per user byte (146378327 updates, 10888.8 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.93** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.81; 209877148 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L3 38 MB, L4 82 MB, L5 680 MB, L6 13685 MB) and an append-only memtable of 2723513 updates merged 2 flushes at a time: **4.84** entries written per update, compaction reads 4.65

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.014 | 0.887 |
| L1 | 20987715 | 250780887 | false | 0.084 | 0.921 | 0.921 |
| L2 | 209877148 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723513 | 2723513 | true | 0.189 | 0.511 | 0.322 |
| L1 | 525100 | 2205931 | true | 0.238 | 0.752 | 0.752 |
| L2 | 1133110 | 6203263 | true | 0.183 | 1.697 | 1.697 |
| L3 | 9396519 | 105454370 | true | 0.089 | 1.882 | 1.882 |
| L4 | 189104948 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.40 | 0.00 | 0.18 | 108000000 | 0 | 1.03 | buffer (flush) | 0.23 | 0.82 |
| L3 | 1.90 | 2.30 | 0.25 | 40000000 | 7543000 | 0.31 | buffer (merge) | 0.29 | 0.88 |
| L4 | 1.20 | 1.40 | 0.16 | 22937000 | 2717000 | 0.19 | L1 | 0.75 | 0.21 |
| L5 | 2.10 | 2.40 | 0.28 | 40000000 | 5607000 | 0.33 | L2 | 1.70 | 0.16 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 0.94 | 1.93 | 4.84 | 0.48 | 0.19 |
| bytes written per block by flushes and compactions | 355771 | 779202 | 1841673 | 0.46 | 0.19 |
| entries read back per update by compactions | 0.86 | 1.81 | 4.65 | 0.48 | 0.19 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        1       38
  4        2       82
  5       15      680
  6      211    13685
```

40 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      48.8      49.0     48.8       0.0   1.0      0.0    240.1    207.99            148.25        96    2.167    889M      0       0.0       0.0
  L3      1/0     38.21 MB   0.6      3.1     1.9      1.2       2.5       2.6      1.3       0.0   1.3    396.0    318.8      8.08              9.92        27    0.299     53M    10M       0.0       0.0
  L4      2/0     81.65 MB   0.6      1.8     0.9      0.9       1.6       1.6      0.7       0.4   1.8    267.1    235.0      6.95              5.31        18    0.386     30M  3622K       0.0       0.0
  L5     15/0    679.84 MB   0.5      2.4     0.8      1.6       2.1       2.1      0.5       0.2   2.6    266.2    230.3      9.31              7.07        16    0.582     40M  5607K       0.0       0.0
  L6    211/0     13.36 GB   0.0     46.8    46.8      0.0      13.4      14.1     13.4       0.0   0.3    887.7    253.3     54.03            141.52         1   54.031    731M   524M       0.0       0.0
 Sum    229/0     14.15 GB   0.0     54.2    50.5      3.7      68.3      69.3     64.6       0.6   1.4    193.8    244.4    286.36            312.08       158    1.812   1745M   543M       0.0       0.0
 Int      0/0      0.00 KB   0.0      6.2     2.9      3.3       6.6       6.7      3.3       0.3   4.8    214.8    229.5     29.41             25.88        88    0.334    209M    16M       0.0       0.0
```
