# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64b2i

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 165051164 ballast puts (2 copies of the starting state, each under its key with one more byte so that it sorts beside the original, never updated); 69312966 live entries once loaded, 4477.2 MB on disk once compacted into the last level (64.6 B per live entry), loaded and compacted in 19s

## Replay

- 50000 blocks applied (the stream's block 0 onward), 265253769 updates in all; measured window: blocks 20000..50000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793030236620046 and 1793030392388406 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 72986570 live entries tracked (78984143 estimated by the engine), 5147.9 MB on disk (70.5 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 287558 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids, 9933166 of them first inside the window (0.057 per update: entries new to every level); the window's last flush and compactions settled in 46.1ms; replay 362s, whole run 362s
- the engine's memtable held 99.6 B per entry (sampled), so the 64 MB buffer holds about 673484 entries, against the 875233 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 3696910595 | 21.3 | 0.29 | 123230 |
| compaction write | 32015728800 | 184.7 | 2.48 | 1067191 |
| compaction read | 35447429274 | 204.5 | 2.75 | 1181581 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 35712639395 | 206.1 | **2.77** | 1190421 |

- write amplification, engine: **2.77** bytes written per user byte, **2.92** entries of 70.5 B (the on-disk entry) per update; compaction reads 2.75 per user byte, 2.90 entries per update. Disk grew by 158.9 MB over the window.
- whole replay, levels filling included: 2.49 per user byte (265253769 updates, 19740.3 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **3.18** entries written per update (flushes and compactions, the WAL apart), compaction reads 2.98; 72986570 entries, 76.7 B each, buffer 64 MB = 875233 entries, ratio 10; the stream's 265253769 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 39 MB, L5 423 MB, L6 4436 MB) and an append-only memtable of 673484 updates merged 2 flushes at a time: **5.07** entries written per update, compaction reads 4.79

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 875233 | 4325048 | true | 0.202 | 2.092 | 1.890 |
| L1 | 7298657 | 73586236 | true | 0.099 | 1.091 | 1.091 |
| L2 | 72986570 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 673484 | 673484 | true | 0.276 | 1.021 | 0.745 |
| L1 | 552940 | 2319878 | true | 0.238 | 2.824 | 2.824 |
| L2 | 5997270 | 56411953 | true | 0.106 | 1.221 | 1.221 |
| L3 | 62893355 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 3.40 | 0.00 | 0.30 | 173000000 | 0 | 1.00 | buffer (flush) | 0.33 | 0.89 |
| L3 | 2.80 | 3.40 | 0.25 | 58000000 | 9987000 | 0.28 | buffer (merge) | 0.69 | 0.36 |
| L4 | 3.70 | 4.40 | 0.33 | 75000000 | 11934000 | 0.36 | L1 | 2.82 | 0.12 |
| L5 | 17.90 | 19.10 | 1.57 | 314000000 | 21269000 | 1.69 | L2 | 1.22 | 1.29 |
| L6 | 5.80 | 6.60 | 0.51 | 108000000 | 12000000 | 0.55 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 2.92 | 3.18 | 5.07 | 0.92 | 0.58 |
| bytes written per block by flushes and compactions | 1190421 | 1410053 | 2064095 | 0.84 | 0.58 |
| entries read back per update by compactions | 2.90 | 2.98 | 4.79 | 0.97 | 0.61 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        1       12
  1        0        0
  2        0        0
  3        0        0
  4        1       39
  5        7      423
  6       76     4436
```

254 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      1/0     11.82 MB   0.5      0.0     0.0      0.0      19.5      20.4     19.5       0.0   1.0      0.0    331.2     60.26             58.93       430    0.140    512M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      5.2     5.2      0.0       4.3       4.3      4.3       0.0   0.8    293.0    241.8     18.08             17.33       193    0.094     87M    15M       0.0       0.0
  L4      1/0     39.00 MB   0.9      6.7     2.9      3.8       5.6       5.7      1.8       1.4   1.9    295.7    247.2     23.10             22.49       130    0.178    113M    18M       0.0       0.0
  L5      7/0    422.55 MB   1.0     25.5     3.1     22.4      23.8      24.0      1.3       0.1   7.7    285.8    266.1     91.45             88.07        63    1.452    419M    30M       0.0       0.0
  L6     76/0      4.33 GB   0.0     22.1    15.3      6.8      11.1      12.1      4.3       0.0   0.7    465.7    234.7     48.51             83.80        18    2.695    375M   188M       0.0       0.0
 Sum     85/0      4.79 GB   0.0     59.4    26.4     33.0      64.2      66.4     31.2       1.5   3.3    252.1    272.4    241.41            270.62       834    0.289   1508M   252M       0.0       0.0
 Int      0/0      0.00 KB   0.0     59.4    26.4     33.0      64.2      66.4     31.2       1.5   3.3    252.1    272.4    241.41            270.62       834    0.289   1508M   252M       0.0       0.0
```
