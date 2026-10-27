# LSM of records on RocksDB: lsm_validate_24000000_p0_rocks256

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), 23104322 live entries once loaded, 1594.4 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 4s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 146378327 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 105117040 updates (104428836 puts, 688204 deletes), 7823.7 MB of keys and values, 68.6 B per put as a record, 76.6 B per entry with the engine's 8 B of overhead
- at the end: 25042572 live entries tracked (28637103 estimated by the engine), 1967.3 MB on disk (78.6 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 301.1ms; replay 191s, whole run 191s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723550 entries, against the 3502253 entries the model gives a buffer of that size at 76.6 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 8138562931 | 77.4 | 1.04 | 406928 |
| flush | 1519473385 | 14.5 | 0.19 | 75974 |
| compaction write | 6265046901 | 59.6 | 0.80 | 313252 |
| compaction read | 7664758402 | 72.9 | 0.98 | 383238 |
| user bytes (engine's count) | 8138562931 | 77.42 | - | 406928 |
| keys written | 105117040 | 1.00 | - | 5256 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 7784520286 | 74.1 | **0.99** | 389226 |

- write amplification, engine: **0.99** bytes written per user byte, **0.94** entries of 78.6 B (the on-disk entry) per update; compaction reads 0.98 per user byte, 0.93 entries per update. Disk grew by 27.9 MB over the window.
- whole replay, levels filling included: 0.82 per user byte (146378327 updates, 10888.8 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.16** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.03; 25042572 entries, 76.6 B each, buffer 256 MB = 3502253 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 195 MB, L6 1622 MB) and an append-only memtable of 2723550 updates merged 4 flushes at a time: **1.94** entries written per update, compaction reads 1.78

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3502253 | 27614587 | true | 0.127 | 1.161 | 1.034 |
| L1 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723550 | 2723550 | true | 0.159 | 0.612 | 0.453 |
| L1 | 2482248 | 17481933 | true | 0.142 | 1.323 | 1.323 |
| L2 | 20647211 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GB | read GB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.40 | 0.00 | 0.17 | 108000000 | 0 | 1.03 | buffer (flush) | 0.23 | 0.75 |
| L5 | 2.80 | 3.60 | 0.34 | 60000000 | 13649000 | 0.44 | buffer (merge) | 0.39 | 0.88 |
| L6 | 3.00 | 3.60 | 0.36 | 58000000 | 10000000 | 0.46 | L1 | 1.32 | 0.27 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 0.94 | 1.16 | 1.94 | 0.81 | 0.49 |
| bytes written per block by flushes and compactions | 389226 | 467504 | 799082 | 0.83 | 0.49 |
| entries read back per update by compactions | 0.93 | 1.03 | 1.78 | 0.90 | 0.52 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        2       59
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        4      195
  6       29     1622
```

40 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      2/0     58.85 MB   0.5      0.0     0.0      0.0       6.5       6.6      6.5       0.0   1.0      0.0    310.9     21.54             21.16        75    0.287    228M      0       0.0       0.0
  L5      4/0    194.95 MB   0.8      4.3     1.9      2.5       3.3       3.4      0.9       0.0   1.8    856.8    659.5      5.18             16.15        13    0.398     72M    17M       0.0       0.0
  L6     29/0      1.58 GB   0.0      8.2     5.3      2.9       4.5       4.8      1.6       0.0   0.9    535.4    294.3     15.76             24.60        12    1.314    129M    58M       0.0       0.0
 Sum     35/0      1.83 GB   0.0     12.6     7.2      5.4      14.4      14.7      9.0       0.0   2.2    303.0    347.2     42.49             61.91       100    0.425    430M    75M       0.0       0.0
 Int      0/0      0.00 KB   0.0     12.6     7.2      5.4      14.4      14.7      9.0       0.0   2.2    303.0    347.2     42.49             61.91       100    0.425    430M    75M       0.0       0.0
```
