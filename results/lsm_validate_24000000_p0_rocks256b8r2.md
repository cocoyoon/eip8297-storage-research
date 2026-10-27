# LSM of records on RocksDB: lsm_validate_24000000_p0_rocks256b8r2

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 14349.7 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 56s

## Replay

- 100000 blocks applied (the stream's block 0 onward), 530507538 updates in all; measured window: blocks 70000..100000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the stream's blocks were replayed 2 times over, block numbers running on (the same keys in the same order again: a longer run than the stream for levels that take longer than the stream to fill)
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793025707825947 and 1793025971450609 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 211612502 live entries tracked (228836421 estimated by the engine), 15860.9 MB on disk (75.0 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 575116 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids; the window's last flush and compactions settled in 542.4ms; replay 1305s, whole run 1305s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723351 entries, against the 3500933 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 2535893425 | 14.6 | 0.20 | 84530 |
| compaction write | 8516357615 | 49.1 | 0.66 | 283879 |
| compaction read | 10929865811 | 63.1 | 0.85 | 364329 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 11052251040 | 63.8 | **0.86** | 368408 |

- write amplification, engine: **0.86** bytes written per user byte, **0.85** entries of 75.0 B (the on-disk entry) per update; compaction reads 0.85 per user byte, 0.84 entries per update. Disk grew by -27.4 MB over the window.
- whole replay, levels filling included: 0.75 per user byte (530507538 updates, 39480.6 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.79** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.67; 211612502 entries, 76.7 B each, buffer 256 MB = 3500933 entries, ratio 10; the stream's 530507538 updates, longest measured window 268435456
- write amplification, model with the engine's levels (L4 214 MB, L5 1147 MB, L6 13685 MB) and an append-only memtable of 2723351 updates merged 4 flushes at a time: **2.49** entries written per update, compaction reads 2.33

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3500933 | 27337067 | true | 0.128 | 1.030 | 0.902 |
| L1 | 21161250 | 304778362 | false | 0.069 | 0.764 | 0.764 |
| L2 | 211612502 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723351 | 2723351 | true | 0.161 | 0.651 | 0.491 |
| L1 | 2855139 | 20830925 | true | 0.137 | 0.872 | 0.872 |
| L2 | 15303013 | 203827301 | true | 0.075 | 0.971 | 0.971 |
| L3 | 182582152 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.40 | 0.00 | 0.20 | 175000000 | 0 | 1.01 | buffer (flush) | 0.23 | 0.87 |
| L4 | 4.30 | 5.70 | 0.36 | 95000000 | 22000000 | 0.42 | buffer (merge) | 0.42 | 0.84 |
| L5 | 3.60 | 4.70 | 0.30 | 81000000 | 19000000 | 0.36 | L1 | 0.87 | 0.34 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 0.85 | 1.79 | 2.49 | 0.47 | 0.34 |
| bytes written per block by flushes and compactions | 368408 | 794644 | 1079859 | 0.46 | 0.34 |
| entries read back per update by compactions | 0.84 | 1.67 | 2.33 | 0.51 | 0.36 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        3       79
  1        0        0
  2        0        0
  3        0        0
  4        5      214
  5       24     1147
  6      211    13685
```

65 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      3/0     79.50 MB   0.8      0.0     0.0      0.0      54.0      54.2     54.0       0.0   1.0      0.0    266.3    207.46            189.45       237    0.875   1273M      0       0.0       0.0
  L4      5/0    214.40 MB   0.8     16.7     7.0      9.6      12.8      13.0      3.2       0.0   1.8    556.6    429.5     30.64             62.84        48    0.638    281M    65M       0.0       0.0
  L5     24/0      1.12 GB   0.8      9.6     2.7      6.9       7.7       7.8      0.8       0.3   2.8    213.1    170.8     46.19             28.56        46    1.004    163M    32M       0.0       0.0
  L6    211/0     13.36 GB   0.0     46.8    46.8      0.0      13.4      14.1     13.4       0.0   0.3    903.5    257.8     53.08            166.07         1   53.084    731M   524M       0.0       0.0
 Sum    243/0     14.77 GB   0.0     73.1    56.6     16.5      87.9      89.1     71.4       0.3   1.6    221.9    266.7    337.37            446.93       332    1.016   2449M   621M       0.0       0.0
 Int      0/0      0.00 KB   0.0      4.4     1.5      2.9       4.4       4.4      1.5       0.0   4.5    194.7    191.6     23.40             19.95        42    0.557    145M    17M       0.0       0.0
```
