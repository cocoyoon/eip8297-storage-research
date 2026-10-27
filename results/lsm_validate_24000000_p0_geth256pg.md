# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256pg

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Page.

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
| key layout | Page |

## Starting state

- 82525582 puts and 0 deletes handed in (6371.5 MB of keys and values, 77.2 B per put), 23104322 live entries once loaded, 1629.9 MB on disk once compacted into the last level (70.5 B per live entry), loaded and compacted in 4s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 8033.9 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 25042572 live entries tracked (27977758 estimated by the engine), 1954.3 MB on disk (78.0 B per live entry, against 76.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 314.7ms; replay 203s, whole run 203s
- the engine's memtable held 100.4 B per entry (sampled), so the 256 MB buffer holds about 2673282 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8348797011 | 79.4 | 1.04 | 417440 |
| flush | 1482851539 | 14.1 | 0.18 | 74143 |
| compaction write | 9081635381 | 86.4 | 1.13 | 454082 |
| compaction read | 10426887840 | 99.2 | 1.30 | 521344 |
| user bytes (engine's count) | 8348797011 | 79.42 | - | 417440 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 10564486920 | 100.5 | **1.31** | 528224 |

- write amplification, engine: **1.31** bytes written per user byte, **1.29** entries of 78.0 B (the on-disk entry) per update; compaction reads 1.30 per user byte, 1.27 entries per update. Disk grew by 78.3 MB over the window.
- whole replay, levels filling included: 1.19 per user byte (146378327 updates, 11181.6 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.16** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.03; 25042572 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 48 MB, L5 133 MB, L6 1661 MB) and an append-only memtable of 2673282 updates merged 2 flushes at a time: **3.54** entries written per update, compaction reads 3.35

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.161 | 1.034 |
| L1 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2673282 | 2673282 | true | 0.190 | 0.531 | 0.342 |
| L1 | 615084 | 2730641 | true | 0.225 | 0.849 | 0.849 |
| L2 | 1704295 | 10660560 | true | 0.160 | 2.156 | 2.156 |
| L3 | 21284462 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.30 | 0.00 | 0.16 | 106000000 | 0 | 1.01 | buffer (flush) | 0.23 | 0.70 |
| L4 | 1.90 | 2.30 | 0.23 | 39000000 | 7365000 | 0.30 | buffer (merge) | 0.30 | 0.76 |
| L5 | 1.60 | 1.90 | 0.20 | 31835000 | 3786000 | 0.27 | L1 | 0.85 | 0.23 |
| L6 | 5.00 | 5.50 | 0.61 | 87000000 | 11000000 | 0.72 | L2 | 2.16 | 0.28 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.29 | 1.16 | 3.54 | 1.11 | 0.36 |
| bytes written per block by flushes and compactions | 528224 | 467504 | 1450718 | 1.13 | 0.36 |
| entries read back per update by compactions | 1.27 | 1.03 | 3.35 | 1.23 | 0.38 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        1       22
  1        0        0
  2        0        0
  3        0        0
  4        1       48
  5        3      133
  6       28     1661
```

40 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      1/0     22.19 MB   0.5      0.0     0.0      0.0       6.4       6.5      6.4       0.0   1.0      0.0    297.5     22.18             20.74        76    0.292    228M      0       0.0       0.0
  L4      1/0     47.86 MB   0.7      3.1     1.9      1.2       2.5       2.5      1.3       0.0   1.3    481.8    388.4      6.59              9.51        27    0.244     52M    10M       0.0       0.0
  L5      3/0    133.14 MB   0.8      2.3     1.0      1.3       2.0       2.1      0.7       0.3   2.1    291.3    257.4      8.12              6.66        22    0.369     39M  4680K       0.0       0.0
  L6     28/0      1.62 GB   0.0     11.2     5.4      5.8       7.5       7.6      1.6       0.0   1.4    441.5    293.7     26.01             31.00        20    1.301    176M    61M       0.0       0.0
 Sum     33/0      1.82 GB   0.0     16.6     8.3      8.4      18.4      18.6     10.1       0.3   2.9    270.6    300.3     62.91             67.91       145    0.434    498M    76M       0.0       0.0
 Int      0/0      0.00 KB   0.0     16.6     8.3      8.4      18.4      18.6     10.1       0.3   2.9    270.6    300.3     62.91             67.91       145    0.434    498M    76M       0.0       0.0
```
