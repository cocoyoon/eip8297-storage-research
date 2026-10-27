# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64b2i5

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

| setting | value |
|---|---|
| write buffer | 64 MB |
| L0 files before compaction into L1 | 2 |
| level base (`max_bytes_for_level_base`, dynamic level bytes) | 2 MB |
| level multiplier | 10 |
| target file size | 64 MB |
| compression | snappy |
| bloom filter | 10 bits per key |
| block cache | 64 MB |
| WAL sync per block | false |
| background jobs | 8 |
| key layout | Path |

## Starting state

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 165051164 ballast puts (2 copies of the starting state, each under its key with one more byte so that it sorts beside the original, never updated); 69312966 live entries once loaded, 4477.2 MB on disk once compacted into the last level (64.6 B per live entry), loaded and compacted in 21s

## Replay

- 50000 blocks applied (the stream's block 0 onward), 265253769 updates in all; measured window: blocks 20000..50000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793030358118235 and 1793030521069303 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 72986570 live entries tracked (77489609 estimated by the engine), 5050.7 MB on disk (69.2 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 287558 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids, 9933166 of them first inside the window (0.057 per update: entries new to every level); the window's last flush and compactions settled in 4.4s; replay 388s, whole run 388s
- the engine's memtable held 99.6 B per entry (sampled), so the 64 MB buffer holds about 673484 entries, against the 875233 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 3696910595 | 21.3 | 0.29 | 123230 |
| compaction write | 35692934801 | 205.9 | 2.77 | 1189764 |
| compaction read | 39224172706 | 226.3 | 3.04 | 1307472 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 39389845396 | 227.3 | **3.05** | 1312995 |

- write amplification, engine: **3.05** bytes written per user byte, **3.28** entries of 69.2 B (the on-disk entry) per update; compaction reads 3.04 per user byte, 3.27 entries per update. Disk grew by 76.6 MB over the window.
- whole replay, levels filling included: 2.71 per user byte (265253769 updates, 19740.3 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **3.18** entries written per update (flushes and compactions, the WAL apart), compaction reads 2.98; 72986570 entries, 76.7 B each, buffer 64 MB = 875233 entries, ratio 10; the stream's 265253769 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 378 MB, L6 4439 MB) and an append-only memtable of 673484 updates merged 2 flushes at a time: **6.06** entries written per update, compaction reads 5.79

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 875233 | 4325048 | true | 0.202 | 2.092 | 1.890 |
| L1 | 7298657 | 73586236 | true | 0.099 | 1.091 | 1.091 |
| L2 | 72986570 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 673484 | 673484 | true | 0.276 | 4.666 | 4.390 |
| L1 | 5462367 | 49808089 | true | 0.110 | 1.398 | 1.398 |
| L2 | 64146688 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 3.40 | 0.00 | 0.30 | 173000000 | 0 | 1.00 | buffer (flush) | 0.33 | 0.91 |
| L3 | 1.60 | 1.90 | 0.14 | 32588000 | 5670000 | 0.16 | buffer (merge) | 4.33 | 0.03 |
| L4 | 5.70 | 6.70 | 0.51 | 113000000 | 15494000 | 0.56 | L1 | 1.40 | 0.37 |
| L5 | 19.40 | 20.80 | 1.74 | 344000000 | 21458000 | 1.86 | L2 | 0.00 | 0.00 |
| L6 | 6.50 | 7.30 | 0.58 | 120000000 | 13000000 | 0.62 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 3.28 | 3.18 | 6.06 | 1.03 | 0.54 |
| bytes written per block by flushes and compactions | 1312995 | 1410053 | 2423958 | 0.93 | 0.54 |
| entries read back per update by compactions | 3.27 | 2.98 | 5.79 | 1.10 | 0.57 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        6      378
  6       75     4439
```

254 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      19.5      20.4     19.5       0.0   1.0      0.0    296.1     67.42             65.06       430    0.157    512M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      2.3     1.1      1.3       1.9       2.0      0.7       4.1   1.8    263.0    217.9      9.07              8.40        79    0.115     39M  6775K       0.0       0.0
  L4      0/0      0.00 KB   0.0     10.6     3.2      7.4       9.1       9.2      1.6       1.6   2.9    287.0    245.6     37.75             36.37       238    0.159    179M    25M       0.0       0.0
  L5      6/0    378.16 MB   0.9     28.1     3.2     24.8      26.2      26.5      1.4       0.0   8.1    279.4    260.9    102.88             98.00        70    1.470    464M    31M       0.0       0.0
  L6     75/0      4.33 GB   0.0     22.7    15.3      7.3      11.7      12.7      4.3       0.0   0.8    441.3    227.0     52.59             89.31        18    2.922    387M   189M       0.0       0.0
 Sum     81/0      4.70 GB   0.0     63.6    22.8     40.8      68.3      70.7     27.5       5.8   3.5    241.6    259.5    269.71            297.13       835    0.323   1583M   254M       0.0       0.0
 Int      0/0      0.00 KB   0.0     63.6    22.8     40.8      68.3      70.7     27.5       5.8   3.5    241.6    259.5    269.71            297.13       835    0.323   1583M   254M       0.0       0.0
```
