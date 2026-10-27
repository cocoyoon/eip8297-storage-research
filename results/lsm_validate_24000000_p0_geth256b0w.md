# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256b0w

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 0 ballast puts (0 copies of the starting state under other key prefixes, never updated); 23104322 live entries once loaded, 1594.4 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 4s

## Replay

- 50000 blocks applied (the stream's block 0 onward), 265253769 updates in all; measured window: blocks 20000..50000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793022749070099 and 1793022894820947 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 26777926 live entries tracked (28887942 estimated by the engine), 1983.1 MB on disk (74.1 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 287558 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids; the window's last flush and compactions settled in 2.2s; replay 268s, whole run 268s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723318 entries, against the 3500933 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 2535531957 | 14.6 | 0.20 | 84518 |
| compaction write | 15257505652 | 88.0 | 1.18 | 508584 |
| compaction read | 17578026340 | 101.4 | 1.36 | 585934 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 17793037609 | 102.7 | **1.38** | 593101 |

- write amplification, engine: **1.38** bytes written per user byte, **1.39** entries of 74.1 B (the on-disk entry) per update; compaction reads 1.36 per user byte, 1.37 entries per update. Disk grew by 112.3 MB over the window.
- whole replay, levels filling included: 1.29 per user byte (265253769 updates, 19740.3 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.23** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.10; 26777926 entries, 76.7 B each, buffer 256 MB = 3500933 entries, ratio 10; the stream's 265253769 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 26 MB, L5 138 MB, L6 1728 MB) and an append-only memtable of 2723318 updates merged 2 flushes at a time: **4.39** entries written per update, compaction reads 4.20

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3500933 | 27492111 | true | 0.127 | 1.229 | 1.101 |
| L1 | 26777926 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723318 | 2723318 | true | 0.191 | 0.484 | 0.293 |
| L1 | 351086 | 1244457 | true | 0.282 | 1.780 | 1.780 |
| L2 | 1863454 | 11859213 | true | 0.157 | 2.125 | 2.125 |
| L3 | 23333686 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.40 | 0.00 | 0.20 | 175000000 | 0 | 1.01 | buffer (flush) | 0.23 | 0.88 |
| L4 | 3.20 | 4.00 | 0.27 | 67000000 | 12362000 | 0.32 | buffer (merge) | 0.26 | 1.05 |
| L5 | 3.00 | 3.40 | 0.25 | 57000000 | 6712000 | 0.29 | L1 | 1.78 | 0.14 |
| L6 | 8.00 | 9.10 | 0.67 | 144000000 | 18000000 | 0.73 | L2 | 2.12 | 0.31 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.39 | 1.23 | 4.39 | 1.13 | 0.32 |
| bytes written per block by flushes and compactions | 593101 | 544261 | 1877274 | 1.09 | 0.32 |
| entries read back per update by compactions | 1.37 | 1.10 | 4.20 | 1.24 | 0.33 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        1       26
  5        3      138
  6       28     1728
```

65 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0       8.2       8.2      8.2       0.0   1.0      0.0    295.8     28.26             28.19       119    0.238    347M      0       0.0       0.0
  L4      1/0     26.10 MB   0.4      5.7     3.6      2.2       4.6       4.7      2.4       0.0   1.3    549.3    441.6     10.64             17.96        49    0.217     96M    18M       0.0       0.0
  L5      3/0    137.58 MB   0.8      4.9     2.1      2.8       4.3       4.4      1.6       0.3   2.0    351.0    310.9     14.27             14.15        49    0.291     81M  9626K       0.0       0.0
  L6     28/0      1.69 GB   0.0     17.4     6.3     11.1      12.8      13.5      1.7       0.0   2.0    445.0    326.4     40.06             49.04        37    1.083    274M    74M       0.0       0.0
 Sum     32/0      1.85 GB   0.0     28.0    12.0     16.0      29.9      30.7     13.9       0.3   3.7    307.6    327.9     93.24            109.34       254    0.367    800M   103M       0.0       0.0
 Int      0/0      0.00 KB   0.0     28.0    12.0     16.0      29.9      30.7     13.9       0.3   3.7    307.6    327.9     93.24            109.34       254    0.367    800M   103M       0.0       0.0
```
