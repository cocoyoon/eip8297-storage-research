# LSM of records on RocksDB: lsm_validate_24000000_p0_deep256

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

| setting | value |
|---|---|
| write buffer | 256 MB |
| L0 files before compaction into L1 | 2 |
| level base (`max_bytes_for_level_base`, dynamic level bytes) | 16 MB |
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
- at the end: 25042572 live entries tracked (26567172 estimated by the engine), 1830.9 MB on disk (73.1 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 2.8s; replay 211s, whole run 211s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723550 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8138562931 | 77.4 | 1.04 | 406928 |
| flush | 1519473397 | 14.5 | 0.19 | 75974 |
| compaction write | 12641041182 | 120.3 | 1.62 | 632052 |
| compaction read | 14029005212 | 133.5 | 1.79 | 701450 |
| user bytes (engine's count) | 8138562931 | 77.42 | - | 406928 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 14160514579 | 134.7 | **1.81** | 708026 |

- write amplification, engine: **1.81** bytes written per user byte, **1.84** entries of 73.1 B (the on-disk entry) per update; compaction reads 1.79 per user byte, 1.83 entries per update. Disk grew by 96.3 MB over the window.
- whole replay, levels filling included: 1.69 per user byte (146378327 updates, 10888.8 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.16** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.03; 25042572 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 110 MB, L6 1636 MB) and an append-only memtable of 2723550 updates merged 2 flushes at a time: **3.33** entries written per update, compaction reads 3.14

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.161 | 1.034 |
| L1 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723550 | 2723550 | true | 0.189 | 0.690 | 0.502 |
| L1 | 1504585 | 9049828 | true | 0.166 | 2.639 | 2.639 |
| L2 | 22377288 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.40 | 0.00 | 0.18 | 108000000 | 0 | 1.03 | buffer (flush) | 0.23 | 0.81 |
| L5 | 5.70 | 6.40 | 0.74 | 109000000 | 10597000 | 0.94 | buffer (merge) | 0.46 | 1.60 |
| L6 | 6.00 | 6.70 | 0.78 | 106000000 | 12000000 | 0.89 | L1 | 2.64 | 0.30 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.84 | 1.16 | 3.33 | 1.59 | 0.55 |
| bytes written per block by flushes and compactions | 708026 | 467504 | 1279296 | 1.51 | 0.55 |
| entries read back per update by compactions | 1.83 | 1.03 | 3.14 | 1.77 | 0.58 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        2      110
  6       28     1636
```

40 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0       6.5       6.6      6.5       0.0   1.0      0.0    269.6     24.85             22.66        75    0.331    228M      0       0.0       0.0
  L5      2/0    110.10 MB   0.7      8.3     1.9      6.4       7.4       7.5      1.1       0.0   3.9    265.2    238.2     31.91             24.89        53    0.602    140M    14M       0.0       0.0
  L6     28/0      1.60 GB   0.0     13.3     5.6      7.7       9.3       9.8      1.6       0.0   1.7    388.3    271.4     34.99             38.64        19    1.841    208M    63M       0.0       0.0
 Sum     30/0      1.71 GB   0.0     21.5     7.5     14.0      23.2      23.9      9.2       0.0   3.6    240.3    259.3     91.74             86.18       147    0.624    577M    77M       0.0       0.0
 Int      0/0      0.00 KB   0.0     21.5     7.5     14.0      23.2      23.9      9.2       0.0   3.6    240.3    259.3     91.74             86.18       147    0.624    577M    77M       0.0       0.0
```
