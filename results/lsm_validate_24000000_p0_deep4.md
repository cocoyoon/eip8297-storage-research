# LSM of records on RocksDB: lsm_validate_24000000_p0_deep4

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

| setting | value |
|---|---|
| write buffer | 256 MB |
| L0 files before compaction into L1 | 2 |
| level base (`max_bytes_for_level_base`, dynamic level bytes) | 4 MB |
| level multiplier | 10 |
| target file size | 2 MB |
| compression | snappy |
| bloom filter | 10 bits per key |
| block cache | 64 MB |
| WAL sync per block | false |
| background jobs | 8 |
| key layout | Path |

## Starting state

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), 23104322 live entries once loaded, 1595.1 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 6s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 7823.7 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 25042572 live entries tracked (27513508 estimated by the engine), 1897.4 MB on disk (75.8 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 1.2s; replay 242s, whole run 242s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723550 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8138562931 | 77.4 | 1.04 | 406928 |
| flush | 1482216590 | 14.1 | 0.19 | 74111 |
| compaction write | 11629655368 | 110.6 | 1.49 | 581483 |
| compaction read | 12983901269 | 123.5 | 1.66 | 649195 |
| user bytes (engine's count) | 8138562931 | 77.42 | - | 406928 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 13111871958 | 124.7 | **1.68** | 655594 |

- write amplification, engine: **1.68** bytes written per user byte, **1.65** entries of 75.8 B (the on-disk entry) per update; compaction reads 1.66 per user byte, 1.63 entries per update. Disk grew by 65.7 MB over the window.
- whole replay, levels filling included: 1.56 per user byte (146378327 updates, 10888.8 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.16** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.03; 25042572 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L3 4 MB, L4 15 MB, L5 162 MB, L6 1629 MB) and an append-only memtable of 2723550 updates merged 2 flushes at a time: **8.74** entries written per update, compaction reads 8.55

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.161 | 1.034 |
| L1 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723550 | 2723550 | true | 0.189 | 0.424 | 0.235 |
| L1 | 52794 | 94725 | true | 0.557 | 2.647 | 2.647 |
| L2 | 197976 | 580484 | true | 0.341 | 4.024 | 4.024 |
| L3 | 2138144 | 14362412 | true | 0.149 | 1.646 | 1.646 |
| L4 | 21500227 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.30 | 0.00 | 0.16 | 105000000 | 0 | 1.00 | buffer (flush) | 0.23 | 0.72 |
| L3 | 1.30 | 1.30 | 0.16 | 22049000 | 377000 | 0.21 | buffer (merge) | 0.20 | 0.82 |
| L4 | 1.10 | 1.30 | 0.14 | 19642000 | 2410000 | 0.16 | L1 | 2.65 | 0.05 |
| L5 | 4.40 | 5.00 | 0.55 | 87000000 | 8274000 | 0.75 | L2 | 4.02 | 0.14 |
| L6 | 4.00 | 4.60 | 0.50 | 74000000 | 11000000 | 0.60 | L3 | 1.65 | 0.31 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.65 | 1.16 | 8.74 | 1.42 | 0.19 |
| bytes written per block by flushes and compactions | 655594 | 467504 | 3481012 | 1.40 | 0.19 |
| entries read back per update by compactions | 1.63 | 1.03 | 8.55 | 1.58 | 0.19 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        2        4
  4        7       15
  5       94      162
  6      892     1629
```

39 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0       6.5       6.6      6.5       0.0   1.0      0.0      7.2    930.41             21.88        75   12.405    228M      0       0.0       0.0
  L3      2/0      3.55 MB   0.9      1.8     1.7      0.1       1.8       1.8      1.7       0.2   1.0    523.7    513.9      3.53              7.88        47    0.075     30M   538K       0.0       0.0
  L4      7/0     14.99 MB   0.9      1.6     0.9      0.6       1.4       1.4      0.8       1.0   1.5    176.3    155.3      9.04              6.37       235    0.038     25M  2974K       0.0       0.0
  L5     94/0    162.02 MB   1.0      6.5     1.7      4.8       5.8       5.9      1.0       0.0   3.4    149.0    133.3     44.90             29.74       841    0.053    112M    11M       0.0       0.0
  L6    892/0      1.59 GB   0.0     10.3     5.5      4.8       6.4       6.8      1.6       0.0   1.2    215.7    134.6     48.82             39.86       516    0.095    163M    61M       0.0       0.0
 Sum    995/0      1.77 GB   0.0     20.2     9.8     10.4      21.9      22.5     11.6       1.2   3.4     19.9     21.7   1036.71            105.73      1714    0.605    561M    77M       0.0       0.0
 Int      0/0      0.00 KB   0.0      0.8     0.3      0.5       0.8       0.8      0.3       0.0   8.6    117.4    118.5      6.97              7.16        87    0.080     20M  1465K       0.0       0.0
```
