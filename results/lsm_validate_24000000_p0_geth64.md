# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

| setting | value |
|---|---|
| write buffer | 64 MB |
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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), 23104322 live entries once loaded, 1594.4 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 6s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 7823.7 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 25042572 live entries tracked (27783611 estimated by the engine), 1914.6 MB on disk (76.5 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 420.9ms; replay 191s, whole run 191s
- the engine's memtable held 99.7 B per entry (sampled), so the 64 MB buffer holds about 673352 entries, against the 875563 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8138562931 | 77.4 | 1.04 | 406928 |
| flush | 2168210458 | 20.6 | 0.28 | 108411 |
| compaction write | 12961557052 | 123.3 | 1.66 | 648078 |
| compaction read | 14959626384 | 142.3 | 1.91 | 747981 |
| user bytes (engine's count) | 8138562931 | 77.42 | - | 406928 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 15129767510 | 143.9 | **1.93** | 756488 |

- write amplification, engine: **1.93** bytes written per user byte, **1.88** entries of 76.5 B (the on-disk entry) per update; compaction reads 1.91 per user byte, 1.86 entries per update. Disk grew by 91.0 MB over the window.
- whole replay, levels filling included: 1.81 per user byte (146378327 updates, 10888.8 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **2.53** entries written per update (flushes and compactions, the WAL apart), compaction reads 2.33; 25042572 entries, 76.6 B each, buffer 64 MB = 875563 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 38 MB, L5 161 MB, L6 1627 MB) and an append-only memtable of 673352 updates merged 2 flushes at a time: **3.90** entries written per update, compaction reads 3.63

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 875563 | 4395012 | true | 0.199 | 0.968 | 0.769 |
| L1 | 2504257 | 17688091 | true | 0.142 | 1.557 | 1.557 |
| L2 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 673352 | 673352 | true | 0.272 | 0.968 | 0.697 |
| L1 | 497039 | 2047764 | true | 0.243 | 1.271 | 1.271 |
| L2 | 2105874 | 14078198 | true | 0.150 | 1.661 | 1.661 |
| L3 | 21281101 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.00 | 0.00 | 0.25 | 105000000 | 0 | 1.00 | buffer (flush) | 0.33 | 0.76 |
| L4 | 4.20 | 5.30 | 0.52 | 92000000 | 16332000 | 0.72 | buffer (merge) | 0.64 | 0.82 |
| L5 | 2.50 | 3.00 | 0.31 | 47000000 | 5200000 | 0.40 | L1 | 1.27 | 0.24 |
| L6 | 5.30 | 5.90 | 0.66 | 94000000 | 11000000 | 0.79 | L2 | 1.66 | 0.40 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.88 | 2.53 | 3.90 | 0.75 | 0.48 |
| bytes written per block by flushes and compactions | 756488 | 1017418 | 1567358 | 0.74 | 0.48 |
| entries read back per update by compactions | 1.86 | 2.33 | 3.63 | 0.80 | 0.51 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        1       38
  5        4      161
  6       28     1627
```

155 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0       8.0       8.1      8.0       0.0   1.0      0.0    242.7     33.81             23.50       257    0.132    228M      0       0.0       0.0
  L4      1/0     38.41 MB   0.6      7.1     2.8      4.2       5.7       5.8      1.5       0.0   2.0    314.1    255.4     23.01             22.86       107    0.215    123M    22M       0.0       0.0
  L5      4/0    160.56 MB   1.0      3.7     1.3      2.3       3.2       3.3      0.9       0.1   2.4    277.7    246.8     13.48             10.96        38    0.355     59M  6704K       0.0       0.0
  L6     28/0      1.59 GB   0.0     12.5     6.1      6.4       8.0       8.5      1.6       0.0   1.3    398.2    254.7     32.15             38.03        18    1.786    198M    71M       0.0       0.0
 Sum     33/0      1.78 GB   0.0     23.2    10.3     13.0      25.0      25.7     12.0       0.1   3.1    232.1    249.9    102.45             95.35       420    0.244    610M   101M       0.0       0.0
 Int      0/0      0.00 KB   0.0     23.2    10.3     13.0      25.0      25.7     12.0       0.1   3.1    232.1    249.9    102.45             95.35       420    0.244    610M   101M       0.0       0.0
```
