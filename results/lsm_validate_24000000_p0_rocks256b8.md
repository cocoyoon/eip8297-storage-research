# LSM of records on RocksDB: lsm_validate_24000000_p0_rocks256b8

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

| setting | value |
|---|---|
| write buffer | 256 MB |
| L0 files before compaction into L1 | 4 |
| level base (`max_bytes_for_level_base`, dynamic level bytes) | 256 MB |
| level multiplier | 10 |
| target file size | 64 MB |
| compression | snappy |
| bloom filter | 10 bits per key |
| block cache | 64 MB |
| WAL sync per block | false |
| background jobs | 8 |
| key layout | Path |

## Starting state

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 14349.7 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 51s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 7823.7 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 209877148 live entries tracked (220688764 estimated by the engine), 15247.3 MB on disk (72.6 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 301.5ms; replay 770s, whole run 770s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723513 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8138562931 | 77.4 | 1.04 | 406928 |
| flush | 1519502560 | 14.5 | 0.19 | 75975 |
| compaction write | 3609564163 | 34.3 | 0.46 | 180478 |
| compaction read | 4497785247 | 42.8 | 0.57 | 224889 |
| user bytes (engine's count) | 8138562931 | 77.42 | - | 406928 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 5129066723 | 48.8 | **0.66** | 256453 |

- write amplification, engine: **0.66** bytes written per user byte, **0.67** entries of 72.6 B (the on-disk entry) per update; compaction reads 0.57 per user byte, 0.59 entries per update. Disk grew by 552.4 MB over the window.
- whole replay, levels filling included: 0.58 per user byte (146378327 updates, 10888.8 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.93** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.81; 209877148 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 196 MB, L5 601 MB, L6 13685 MB) and an append-only memtable of 2723513 updates merged 4 flushes at a time: **3.41** entries written per update, compaction reads 3.25

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.014 | 0.887 |
| L1 | 20987715 | 250780887 | false | 0.084 | 0.921 | 0.921 |
| L2 | 209877148 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723513 | 2723513 | true | 0.159 | 0.632 | 0.473 |
| L1 | 2697921 | 19527399 | true | 0.138 | 0.562 | 0.562 |
| L2 | 8272708 | 88779104 | true | 0.093 | 2.215 | 2.215 |
| L3 | 188372725 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.40 | 0.00 | 0.18 | 108000000 | 0 | 1.03 | buffer (flush) | 0.23 | 0.81 |
| L4 | 2.70 | 3.50 | 0.35 | 59000000 | 13649000 | 0.43 | buffer (merge) | 0.41 | 0.87 |
| L5 | 0.70 | 0.80 | 0.09 | 13000000 | 1825000 | 0.11 | L1 | 0.56 | 0.16 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 0.67 | 1.93 | 3.41 | 0.35 | 0.20 |
| bytes written per block by flushes and compactions | 256453 | 779202 | 1301621 | 0.33 | 0.20 |
| entries read back per update by compactions | 0.59 | 1.81 | 3.25 | 0.33 | 0.18 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        2       59
  1        0        0
  2        0        0
  3        0        0
  4        4      196
  5       14      601
  6      211    13685
```

40 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      2/0     59.02 MB   0.5      0.0     0.0      0.0      48.8      49.0     48.8       0.0   1.0      0.0    245.5    203.42            146.82        96    2.119    889M      0       0.0       0.0
  L4      4/0    195.72 MB   0.8      4.2     1.9      2.3       3.2       3.3      0.9       0.0   1.7    522.0    398.9      8.28             16.08        13    0.637     71M    17M       0.0       0.0
  L5     14/0    601.27 MB   0.4      0.8     0.4      0.4       0.7       0.7      0.3       0.3   1.7    133.3    115.7      6.04              2.48         7    0.863     13M  1825K       0.0       0.0
  L6    211/0     13.36 GB   0.0     46.8    46.8      0.0      13.4      14.1     13.4       0.0   0.3    999.3    285.1     48.00            141.54         1   47.997    731M   524M       0.0       0.0
 Sum    231/0     14.20 GB   0.0     51.8    49.1      2.7      66.0      67.0     63.3       0.3   1.4    199.8    254.5    265.73            306.92       117    2.271   1705M   542M       0.0       0.0
 Int      0/0      0.00 KB   0.0      3.9     1.7      2.2       4.3       4.3      2.1       0.1   3.5    152.6    169.3     25.97             21.64        51    0.509    159M    14M       0.0       0.0
```
