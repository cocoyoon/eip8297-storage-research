# LSM of records on RocksDB: lsm_validate_24000000_p0_model64

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

| setting | value |
|---|---|
| write buffer | 64 MB |
| L0 files before compaction into L1 | 1 |
| level base (`max_bytes_for_level_base`, dynamic level bytes) | 640 MB |
| level multiplier | 10 |
| target file size | 64 MB |
| compression | snappy |
| bloom filter | 10 bits per key |
| block cache | 64 MB |
| WAL sync per block | false |
| background jobs | 8 |
| key layout | Path |

## Starting state

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), 23104322 live entries once loaded, 1594.4 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 6s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 7823.7 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 25042572 live entries tracked (32484033 estimated by the engine), 2278.9 MB on disk (91.0 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 2.3s; replay 225s, whole run 225s
- the engine's memtable held 99.7 B per entry (sampled), so the 64 MB buffer holds about 673352 entries, against the 875563 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8138562931 | 77.4 | 1.04 | 406928 |
| flush | 2168210458 | 20.6 | 0.28 | 108411 |
| compaction write | 67482530577 | 642.0 | 8.63 | 3374127 |
| compaction read | 67980359783 | 646.7 | 8.69 | 3399018 |
| user bytes (engine's count) | 8138562931 | 77.42 | - | 406928 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 69650741035 | 662.6 | **8.90** | 3482537 |

- write amplification, engine: **8.90** bytes written per user byte, **7.28** entries of 91.0 B (the on-disk entry) per update; compaction reads 8.69 per user byte, 7.11 entries per update. Disk grew by 360.8 MB over the window.
- whole replay, levels filling included: 7.47 per user byte (146378327 updates, 10888.8 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **2.53** entries written per update (flushes and compactions, the WAL apart), compaction reads 2.33; 25042572 entries, 76.6 B each, buffer 64 MB = 875563 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 633 MB, L6 1540 MB) and an append-only memtable of 673352 updates merged 1 flushes at a time: **11.33** entries written per update, compaction reads 11.00

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 875563 | 4395012 | true | 0.199 | 0.968 | 0.769 |
| L1 | 2504257 | 17688091 | true | 0.142 | 1.557 | 1.557 |
| L2 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 673352 | 673352 | true | 0.328 | 10.986 | 10.658 |
| L1 | 6955984 | 70237888 | true | 0.099 | 0.340 | 0.340 |
| L2 | 16922931 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.50 | 0.60 | 0.26 | 116000000 | 1721000 | 1.09 | buffer (flush) | 0.33 | 0.80 |
| L5 | 62.20 | 63.70 | 6.50 | 1061000000 | 24766000 | 9.86 | buffer (merge) | 10.66 | 0.61 |
| L6 | 0.20 | 0.30 | 0.02 | 6000000 | 2000000 | 0.04 | L1 | 0.34 | 0.06 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 7.28 | 2.53 | 11.33 | 2.88 | 0.64 |
| bytes written per block by flushes and compactions | 3482537 | 1017418 | 5416848 | 3.42 | 0.64 |
| entries read back per update by compactions | 7.11 | 2.33 | 11.00 | 3.05 | 0.65 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5       12      633
  6       25     1540
```

178 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.6     0.0      0.6       8.5       8.6      7.9       0.0   1.1     15.5    222.9     39.06             24.98       280    0.140    239M  1721K       0.0       0.0
  L5     12/0    633.04 MB   1.0     74.2     2.7     71.5      72.2      73.0      0.7       0.0  26.8    548.3    533.6    138.57            273.63       187    0.741   1235M    33M       0.0       0.0
  L6     25/0      1.50 GB   0.0      5.5     5.3      0.2       1.7       1.9      1.5       0.0   0.3    837.4    262.7      6.78             15.98         3    2.261     87M    60M       0.0       0.0
 Sum     37/0      2.12 GB   0.0     80.3     8.0     72.3      82.5      83.4     10.1       0.0  10.3    446.0    457.8    184.42            314.59       470    0.392   1562M    95M       0.0       0.0
 Int      0/0      0.00 KB   0.0     80.3     8.0     72.3      82.5      83.4     10.1       0.0  10.3    446.0    457.8    184.42            314.59       470    0.392   1562M    95M       0.0       0.0
```
