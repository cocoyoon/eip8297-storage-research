# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256b8hr2

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

- 82525582 puts and 0 deletes handed in (5440.5 MB of keys and values, 65.9 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 16372.6 MB on disk once compacted into the last level (78.7 B per live entry), loaded and compacted in 51s

## Replay

- 100000 blocks applied (the stream's block 0 onward), 530507538 updates in all; measured window: blocks 70000..100000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 10540.0 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the stream's blocks were replayed 2 times over, block numbers running on (the same keys in the same order again: a longer run than the stream for levels that take longer than the stream to fill)
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793028360463286 and 1793028692412217 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 211612502 live entries tracked (226298443 estimated by the engine), 17844.0 MB on disk (84.3 B per live entry, against 60.8 B handed in per put: prefix compression and snappy at work); 575116 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids; the window's last flush and compactions settled in 1.5s; replay 2720s, whole run 2720s
- the engine's memtable held 84.0 B per entry (sampled), so the 256 MB buffer holds about 3196772 entries, against the 3500933 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 11059129514 | 63.8 | 1.05 | 368638 |
| flush | 2724780893 | 15.7 | 0.26 | 90826 |
| compaction write | 14662688402 | 84.6 | 1.39 | 488756 |
| compaction read | 17366949665 | 100.2 | 1.65 | 578898 |
| user bytes (engine's count) | 11059129514 | 63.81 | - | 368638 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 17387469295 | 100.3 | **1.65** | 579582 |

- write amplification, engine: **1.65** bytes written per user byte, **1.19** entries of 84.3 B (the on-disk entry) per update; compaction reads 1.65 per user byte, 1.19 entries per update. Disk grew by -73.9 MB over the window.
- whole replay, levels filling included: 1.44 per user byte (530507538 updates, 32313.1 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.79** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.67; 211612502 entries, 76.7 B each, buffer 256 MB = 3500933 entries, ratio 10; the stream's 530507538 updates, longest measured window 268435456
- write amplification, model with the engine's levels (L3 40 MB, L4 97 MB, L5 1266 MB, L6 15614 MB) and an append-only memtable of 3196772 updates merged 2 flushes at a time: **4.94** entries written per update, compaction reads 4.76

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3500933 | 27337067 | true | 0.128 | 1.030 | 0.902 |
| L1 | 21161250 | 304778362 | false | 0.069 | 0.764 | 0.764 |
| L2 | 211612502 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3196772 | 3196772 | true | 0.183 | 0.477 | 0.293 |
| L1 | 474362 | 1881390 | true | 0.252 | 0.864 | 0.864 |
| L2 | 1150327 | 6230837 | true | 0.185 | 2.594 | 2.594 |
| L3 | 15013543 | 198462874 | true | 0.076 | 1.009 | 1.009 |
| L4 | 185167034 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.60 | 0.00 | 0.19 | 174000000 | 0 | 1.00 | buffer (flush) | 0.22 | 0.87 |
| L3 | 3.00 | 3.80 | 0.22 | 57000000 | 11000000 | 0.27 | buffer (merge) | 0.26 | 0.86 |
| L4 | 2.80 | 3.20 | 0.21 | 49000000 | 6000000 | 0.25 | L1 | 0.86 | 0.24 |
| L5 | 7.80 | 9.30 | 0.57 | 140000000 | 23000000 | 0.68 | L2 | 2.59 | 0.22 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.19 | 1.79 | 4.94 | 0.66 | 0.24 |
| bytes written per block by flushes and compactions | 579582 | 794644 | 2407979 | 0.73 | 0.24 |
| entries read back per update by compactions | 1.19 | 1.67 | 4.76 | 0.71 | 0.25 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        1       40
  4        4       97
  5       21     1266
  6      241    15614
```

55 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      60.0      60.0     60.0       0.0   1.0      0.0    222.3    276.28            210.78       208    1.328   1273M      0       0.0       0.0
  L3      1/0     40.43 MB   0.6     11.0     7.7      3.3       8.8       8.8      5.5       0.0   1.2    723.0    581.0     15.60             29.72        83    0.188    166M    32M       0.0       0.0
  L4      4/0     96.90 MB   0.6     10.2     4.7      5.6       9.0       9.0      3.5       0.8   1.9    409.4    362.6     25.55             24.29       105    0.243    154M    17M       0.0       0.0
  L5     21/0      1.24 GB   0.8     20.7     4.1     16.6      17.7      17.7      1.1       0.1   4.3    432.7    370.1     48.93             47.79        78    0.627    311M    45M       0.0       0.0
  L6    241/0     15.25 GB   0.0     52.3    52.3      0.0      15.2      15.3     15.2       0.0   0.3   1099.0    320.6     48.71            155.52         1   48.707    731M   524M       0.0       0.0
 Sum    267/0     16.62 GB   0.0     94.2    68.7     25.5     110.8     110.8     85.3       1.0   1.8    232.3    273.3    415.05            468.09       475    0.874   2636M   619M       0.0       0.0
 Int      0/0      0.00 KB   0.0     16.0     5.4     10.6      15.9      15.9      5.3       0.3   6.5    306.9    304.7     53.50             57.74       142    0.377    409M    38M       0.0       0.0
```
