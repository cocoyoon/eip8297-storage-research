# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), 23104322 live entries once loaded, 1594.4 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 4s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 7823.7 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 25042572 live entries tracked (27753483 estimated by the engine), 1914.7 MB on disk (76.5 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 912.6ms; replay 182s, whole run 182s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723550 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8138562931 | 77.4 | 1.04 | 406928 |
| flush | 1519473397 | 14.5 | 0.19 | 75974 |
| compaction write | 8236294585 | 78.4 | 1.05 | 411815 |
| compaction read | 9531587495 | 90.7 | 1.22 | 476579 |
| user bytes (engine's count) | 8138562931 | 77.42 | - | 406928 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 9755767982 | 92.8 | **1.25** | 487788 |

- write amplification, engine: **1.25** bytes written per user byte, **1.21** entries of 76.5 B (the on-disk entry) per update; compaction reads 1.22 per user byte, 1.19 entries per update. Disk grew by 165.2 MB over the window.
- whole replay, levels filling included: 1.18 per user byte (146378327 updates, 10888.8 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.16** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.03; 25042572 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 56 MB, L5 143 MB, L6 1627 MB) and an append-only memtable of 2723550 updates merged 2 flushes at a time: **3.22** entries written per update, compaction reads 3.03

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.161 | 1.034 |
| L1 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723550 | 2723550 | true | 0.189 | 0.549 | 0.360 |
| L1 | 732445 | 3456054 | true | 0.212 | 0.753 | 0.753 |
| L2 | 1870350 | 12046158 | true | 0.155 | 1.922 | 1.922 |
| L3 | 21280133 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.40 | 0.00 | 0.17 | 108000000 | 0 | 1.03 | buffer (flush) | 0.23 | 0.77 |
| L4 | 1.90 | 2.40 | 0.24 | 40000000 | 7864000 | 0.31 | buffer (merge) | 0.32 | 0.73 |
| L5 | 1.60 | 1.80 | 0.20 | 32000000 | 3849000 | 0.27 | L1 | 0.75 | 0.26 |
| L6 | 4.10 | 4.70 | 0.51 | 75000000 | 10000000 | 0.62 | L2 | 1.92 | 0.27 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.21 | 1.16 | 3.22 | 1.05 | 0.38 |
| bytes written per block by flushes and compactions | 487788 | 467504 | 1295355 | 1.04 | 0.38 |
| entries read back per update by compactions | 1.19 | 1.03 | 3.03 | 1.15 | 0.39 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        1       56
  5        4      143
  6       30     1627
```

40 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0       6.5       6.6      6.5       0.0   1.0      0.0    263.8     25.39             20.88        75    0.339    228M      0       0.0       0.0
  L4      1/0     55.92 MB   0.9      3.1     1.9      1.1       2.5       2.5      1.3       0.0   1.3    549.4    443.7      5.73              9.65        27    0.212     51M  9996K       0.0       0.0
  L5      4/0    143.08 MB   0.9      2.5     1.1      1.4       2.2       2.2      0.8       0.2   2.0    341.2    299.6      7.56              7.50        26    0.291     43M  5389K       0.0       0.0
  L6     30/0      1.59 GB   0.0     10.7     5.4      5.2       6.8       7.2      1.6       0.0   1.3    510.1    325.8     21.43             30.31        19    1.128    168M    61M       0.0       0.0
 Sum     35/0      1.78 GB   0.0     16.3     8.5      7.8      18.1      18.6     10.3       0.2   2.8    277.2    307.5     60.11             68.34       147    0.409    492M    76M       0.0       0.0
 Int      0/0      0.00 KB   0.0     16.3     8.5      7.8      18.1      18.6     10.3       0.2   2.8    277.2    307.5     60.11             68.34       147    0.409    492M    76M       0.0       0.0
```
