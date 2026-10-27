# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256b8h

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Hash.

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
| key layout | Hash |

## Starting state

- 82525582 puts and 0 deletes handed in (5440.5 MB of keys and values, 65.9 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 16372.6 MB on disk once compacted into the last level (78.7 B per live entry), loaded and compacted in 51s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 6383.0 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 209877148 live entries tracked (219940235 estimated by the engine), 17318.8 MB on disk (82.5 B per live entry, against 60.7 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 1.7s; replay 1713s, whole run 1713s
- the engine's memtable held 84.0 B per entry (sampled), so the 256 MB buffer holds about 3193853 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 6697879703 | 63.7 | 1.05 | 334894 |
| flush | 1643931518 | 15.6 | 0.26 | 82197 |
| compaction write | 5739947381 | 54.6 | 0.90 | 286997 |
| compaction read | 6745123335 | 64.2 | 1.06 | 337256 |
| user bytes (engine's count) | 6697879703 | 63.72 | - | 334894 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 7383878899 | 70.2 | **1.16** | 369194 |

- write amplification, engine: **1.16** bytes written per user byte, **0.85** entries of 82.5 B (the on-disk entry) per update; compaction reads 1.06 per user byte, 0.78 entries per update. Disk grew by 598.0 MB over the window.
- whole replay, levels filling included: 1.08 per user byte (146378327 updates, 8924.1 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.93** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.81; 209877148 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L3 49 MB, L4 107 MB, L5 746 MB, L6 15614 MB) and an append-only memtable of 3193853 updates merged 2 flushes at a time: **4.59** entries written per update, compaction reads 4.41

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.014 | 0.887 |
| L1 | 20987715 | 250780887 | false | 0.084 | 0.921 | 0.921 |
| L2 | 209877148 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3193853 | 3193853 | true | 0.181 | 0.491 | 0.309 |
| L1 | 593804 | 2603964 | true | 0.228 | 0.726 | 0.726 |
| L2 | 1296673 | 7428140 | true | 0.175 | 1.392 | 1.392 |
| L3 | 9040357 | 100089282 | true | 0.090 | 1.981 | 1.981 |
| L4 | 189217332 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.60 | 0.00 | 0.18 | 109000000 | 0 | 1.04 | buffer (flush) | 0.22 | 0.85 |
| L3 | 1.70 | 2.10 | 0.20 | 32000000 | 6415000 | 0.24 | buffer (merge) | 0.27 | 0.71 |
| L4 | 1.70 | 2.00 | 0.20 | 30000000 | 3697000 | 0.25 | L1 | 0.73 | 0.27 |
| L5 | 1.90 | 2.10 | 0.22 | 32136000 | 4624000 | 0.26 | L2 | 1.39 | 0.16 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 0.85 | 1.93 | 4.59 | 0.44 | 0.19 |
| bytes written per block by flushes and compactions | 369194 | 779202 | 1990279 | 0.47 | 0.19 |
| entries read back per update by compactions | 0.78 | 1.81 | 4.41 | 0.43 | 0.18 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        1       49
  4        3      107
  5       15      746
  6      241    15614
```

34 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      54.4      54.4     54.4       0.0   1.0      0.0    267.3    208.28            137.46        88    2.367    889M      0       0.0       0.0
  L3      1/0     49.49 MB   0.8      2.9     2.1      0.8       2.3       2.3      1.5       0.0   1.1    600.2    483.1      4.94              7.17        23    0.215     43M  8501K       0.0       0.0
  L4      3/0    106.61 MB   0.7      2.8     1.2      1.5       2.4       2.4      0.9       0.3   2.0    305.7    270.3      9.27              6.22        25    0.371     41M  4841K       0.0       0.0
  L5     15/0    746.25 MB   0.5      2.4     0.9      1.5       2.1       2.1      0.6       0.1   2.3    333.9    289.1      7.47              5.42        16    0.467     36M  4940K       0.0       0.0
  L6    241/0     15.25 GB   0.0     52.3    52.3      0.0      15.2      15.3     15.2       0.0   0.3   1096.2    319.7     48.83            155.05         1   48.833    731M   524M       0.0       0.0
 Sum    260/0     16.13 GB   0.0     60.4    56.5      3.9      76.5      76.5     72.6       0.4   1.4    221.7    281.0    278.78            311.33       153    1.822   1743M   542M       0.0       0.0
 Int      0/0      0.00 KB   0.0     60.4    56.5      3.9      32.2      32.2     28.3       0.4   3.2    520.9    277.8    118.66            209.36       118    1.006   1113M   542M       0.0       0.0
```
