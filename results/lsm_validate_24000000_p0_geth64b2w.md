# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64b2w

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

| setting | value |
|---|---|
| write buffer | 64 MB |
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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 165051164 ballast puts (2 copies of the starting state under other key prefixes, never updated); 69312966 live entries once loaded, 4783.1 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 18s

## Replay

- 50000 blocks applied (the stream's block 0 onward), 265253769 updates in all; measured window: blocks 20000..50000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793029527606143 and 1793029678933319 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 72986570 live entries tracked (78878040 estimated by the engine), 5440.4 MB on disk (74.5 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 287558 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids, 9933166 of them first inside the window (0.057 per update: entries new to every level); the window's last flush and compactions settled in 1.5s; replay 392s, whole run 392s
- the engine's memtable held 99.6 B per entry (sampled), so the 64 MB buffer holds about 673484 entries, against the 875233 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 3696910581 | 21.3 | 0.29 | 123230 |
| compaction write | 29523953548 | 170.4 | 2.29 | 984132 |
| compaction read | 32998177012 | 190.4 | 2.56 | 1099939 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 33220864129 | 191.7 | **2.58** | 1107362 |

- write amplification, engine: **2.58** bytes written per user byte, **2.57** entries of 74.5 B (the on-disk entry) per update; compaction reads 2.56 per user byte, 2.55 entries per update. Disk grew by 127.2 MB over the window.
- whole replay, levels filling included: 2.31 per user byte (265253769 updates, 19740.3 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **3.18** entries written per update (flushes and compactions, the WAL apart), compaction reads 2.98; 72986570 entries, 76.7 B each, buffer 64 MB = 875233 entries, ratio 10; the stream's 265253769 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 440 MB, L6 4737 MB) and an append-only memtable of 673484 updates merged 2 flushes at a time: **6.25** entries written per update, compaction reads 5.97

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 875233 | 4325048 | true | 0.202 | 2.092 | 1.890 |
| L1 | 7298657 | 73586236 | true | 0.099 | 1.091 | 1.091 |
| L2 | 72986570 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 673484 | 673484 | true | 0.276 | 4.993 | 4.717 |
| L1 | 5902918 | 55232291 | true | 0.107 | 1.257 | 1.257 |
| L2 | 63550280 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 3.40 | 0.00 | 0.28 | 173000000 | 0 | 1.00 | buffer (flush) | 0.33 | 0.85 |
| L3 | 2.80 | 3.40 | 0.23 | 58000000 | 9987000 | 0.28 | buffer (merge) | 4.66 | 0.05 |
| L4 | 3.80 | 4.60 | 0.32 | 77000000 | 12015000 | 0.37 | L1 | 1.26 | 0.25 |
| L5 | 18.80 | 20.00 | 1.56 | 332000000 | 20971000 | 1.79 | L2 | 0.00 | 0.00 |
| L6 | 2.20 | 2.80 | 0.18 | 44000000 | 11000000 | 0.19 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 2.57 | 3.18 | 6.25 | 0.81 | 0.41 |
| bytes written per block by flushes and compactions | 1107362 | 1410053 | 2691483 | 0.79 | 0.41 |
| entries read back per update by compactions | 2.55 | 2.98 | 5.97 | 0.86 | 0.43 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        1       12
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        9      440
  6       82     4737
```

254 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      1/0     11.82 MB   0.5      0.0     0.0      0.0      20.8      20.9     20.8       0.0   1.0      0.0    347.3     61.36             60.29       430    0.143    512M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      5.2     5.2      0.0       4.3       4.3      4.3       0.0   0.8    314.3    259.4     16.86             16.53       193    0.087     87M    15M       0.0       0.0
  L4      0/0      0.00 KB   0.0      6.8     2.9      3.9       5.7       5.8      1.8       1.4   2.0    316.3    264.8     22.10             21.84       131    0.169    115M    18M       0.0       0.0
  L5      9/0    439.94 MB   0.9     26.6     3.1     23.5      24.9      25.1      1.3       0.1   8.0    306.3    285.7     89.09             88.26        62    1.437    441M    30M       0.0       0.0
  L6     82/0      4.63 GB   0.0     18.8    16.6      2.2       6.9       7.2      4.6       0.0   0.4    727.8    265.7     26.46             63.49        19    1.393    294M   187M       0.0       0.0
 Sum     92/0      5.07 GB   0.0     57.5    27.7     29.7      62.5      63.4     32.8       1.5   3.0    272.5    296.6    215.86            250.39       835    0.259   1452M   252M       0.0       0.0
 Int      0/0      0.00 KB   0.0     57.5    27.7     29.7      62.5      63.4     32.8       1.5   3.0    272.5    296.6    215.86            250.39       835    0.259   1452M   252M       0.0       0.0
```
