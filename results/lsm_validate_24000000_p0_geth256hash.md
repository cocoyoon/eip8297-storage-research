# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256hash

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

- 82525582 puts and 0 deletes handed in (5440.5 MB of keys and values, 65.9 B per put), 23104322 live entries once loaded, 1830.5 MB on disk once compacted into the last level (79.2 B per live entry), loaded and compacted in 6s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 6383.0 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 25042572 live entries tracked (27577797 estimated by the engine), 2164.1 MB on disk (86.4 B per live entry, against 60.7 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 1.1s; replay 388s, whole run 388s
- the engine's memtable held 84.0 B per entry (sampled), so the 256 MB buffer holds about 3194145 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 6697879703 | 63.7 | 1.05 | 334894 |
| flush | 1644019570 | 15.6 | 0.26 | 82201 |
| compaction write | 9169293377 | 87.2 | 1.44 | 458465 |
| compaction read | 10623193901 | 101.1 | 1.66 | 531160 |
| user bytes (engine's count) | 6697879703 | 63.72 | - | 334894 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 10813312947 | 102.9 | **1.69** | 540666 |

- write amplification, engine: **1.69** bytes written per user byte, **1.19** entries of 86.4 B (the on-disk entry) per update; compaction reads 1.66 per user byte, 1.17 entries per update. Disk grew by 137.5 MB over the window.
- whole replay, levels filling included: 1.59 per user byte (146378327 updates, 8924.1 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.16** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.03; 25042572 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 19 MB, L5 184 MB, L6 1861 MB) and an append-only memtable of 3194145 updates merged 2 flushes at a time: **5.59** entries written per update, compaction reads 5.41

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.161 | 1.034 |
| L1 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3194145 | 3194145 | true | 0.181 | 0.432 | 0.251 |
| L1 | 219862 | 670333 | true | 0.328 | 3.504 | 3.504 |
| L2 | 2129187 | 14283389 | true | 0.149 | 1.657 | 1.657 |
| L3 | 21534878 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.50 | 0.00 | 0.17 | 108000000 | 0 | 1.03 | buffer (flush) | 0.22 | 0.76 |
| L4 | 1.80 | 2.30 | 0.20 | 34000000 | 6744000 | 0.26 | buffer (merge) | 0.22 | 0.92 |
| L5 | 2.10 | 2.40 | 0.23 | 36000000 | 4267000 | 0.30 | L1 | 3.50 | 0.07 |
| L6 | 4.60 | 5.30 | 0.51 | 74000000 | 10000000 | 0.61 | L2 | 1.66 | 0.31 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.19 | 1.16 | 5.59 | 1.03 | 0.21 |
| bytes written per block by flushes and compactions | 540666 | 467504 | 2540380 | 1.16 | 0.21 |
| entries read back per update by compactions | 1.17 | 1.03 | 5.41 | 1.13 | 0.22 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        2       19
  5        5      184
  6       32     1861
```

34 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0       7.2       7.2      7.2       0.0   1.0      0.0    189.4     39.18             28.30        67    0.585    228M      0       0.0       0.0
  L4      2/0     19.38 MB   0.3      3.0     2.1      0.9       2.4       2.4      1.5       0.0   1.1    548.3    441.5      5.58              7.33        23    0.242     45M  8763K       0.0       0.0
  L5      5/0    183.61 MB   1.0      3.1     1.3      1.8       2.7       2.7      0.9       0.2   2.1    301.2    265.7     10.55              6.88        27    0.391     46M  5521K       0.0       0.0
  L6     32/0      1.82 GB   0.0     11.9     6.1      5.8       7.6       7.6      1.8       0.0   1.3    445.7    285.6     27.41             28.97        19    1.442    165M    61M       0.0       0.0
 Sum     39/0      2.02 GB   0.0     18.0     9.5      8.5      20.0      20.0     11.5       0.2   2.8    223.0    248.0     82.71             71.48       136    0.608    486M    75M       0.0       0.0
 Int      0/0      0.00 KB   0.0     18.0     9.5      8.5      20.0      20.0     11.5       0.2   2.8    223.0    248.0     82.71             71.48       136    0.608    486M    75M       0.0       0.0
```
