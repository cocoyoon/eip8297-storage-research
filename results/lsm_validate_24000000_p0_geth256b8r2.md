# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256b8r2

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin`; the model is `pbt_store::lsm::cost_from_curve` on the same stream, with this database's live entries as the full state. Values are pseudo-random bytes of the record's size less its key; keys are tree positions laid out as Path.

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
| key layout | Path |

## Starting state

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 14349.7 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 83s

## Replay

- 100000 blocks applied (the stream's block 0 onward), 530507538 updates in all; measured window: blocks 70000..100000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the stream's blocks were replayed 2 times over, block numbers running on (the same keys in the same order again: a longer run than the stream for levels that take longer than the stream to fill)
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793024500018975 and 1793024665597285 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 211612502 live entries tracked (226208071 estimated by the engine), 15674.6 MB on disk (74.1 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 575116 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids; the window's last flush and compactions settled in 2.0s; replay 1220s, whole run 1220s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723351 entries, against the 3500933 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 2535893425 | 14.6 | 0.20 | 84530 |
| compaction write | 13386917421 | 77.2 | 1.04 | 446231 |
| compaction read | 15878716499 | 91.6 | 1.23 | 529291 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 15922810846 | 91.9 | **1.23** | 530760 |

- write amplification, engine: **1.23** bytes written per user byte, **1.24** entries of 74.1 B (the on-disk entry) per update; compaction reads 1.23 per user byte, 1.24 entries per update. Disk grew by -99.3 MB over the window.
- whole replay, levels filling included: 1.09 per user byte (530507538 updates, 39480.6 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.79** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.67; 211612502 entries, 76.7 B each, buffer 256 MB = 3500933 entries, ratio 10; the stream's 530507538 updates, longest measured window 268435456
- write amplification, model with the engine's levels (L3 24 MB, L4 93 MB, L5 1147 MB, L6 13685 MB) and an append-only memtable of 2723351 updates merged 2 flushes at a time: **5.25** entries written per update, compaction reads 5.06

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3500933 | 27337067 | true | 0.128 | 1.030 | 0.902 |
| L1 | 21161250 | 304778362 | false | 0.069 | 0.764 | 0.764 |
| L2 | 211612502 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723351 | 2723351 | true | 0.191 | 0.479 | 0.288 |
| L1 | 324009 | 1114486 | true | 0.291 | 1.417 | 1.417 |
| L2 | 1255534 | 7004411 | true | 0.179 | 2.390 | 2.390 |
| L3 | 15484913 | 207218970 | true | 0.075 | 0.966 | 0.966 |
| L4 | 184752432 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.40 | 0.00 | 0.20 | 175000000 | 0 | 1.01 | buffer (flush) | 0.23 | 0.88 |
| L3 | 3.10 | 3.80 | 0.26 | 64000000 | 12000000 | 0.30 | buffer (merge) | 0.25 | 1.04 |
| L4 | 2.50 | 2.90 | 0.21 | 49000000 | 6000000 | 0.25 | L1 | 1.42 | 0.15 |
| L5 | 7.20 | 8.50 | 0.60 | 144000000 | 23000000 | 0.70 | L2 | 2.39 | 0.25 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.24 | 1.79 | 5.25 | 0.69 | 0.24 |
| bytes written per block by flushes and compactions | 530760 | 794644 | 2247660 | 0.67 | 0.24 |
| entries read back per update by compactions | 1.24 | 1.67 | 5.06 | 0.74 | 0.24 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        1       24
  4        4       93
  5       24     1147
  6      211    13685
```

65 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      54.0      54.2     54.0       0.0   1.0      0.0    257.1    214.89            205.21       237    0.907   1273M      0       0.0       0.0
  L3      1/0     23.56 MB   0.4     11.5     7.1      4.4       9.3       9.4      4.9       0.0   1.3    544.7    438.8     21.59             37.52        97    0.223    192M    37M       0.0       0.0
  L4      4/0     92.91 MB   0.7      8.1     3.9      4.2       7.1       7.2      3.0       1.0   1.8    318.1    281.7     25.92             25.65        85    0.305    138M    16M       0.0       0.0
  L5     24/0      1.12 GB   0.8     19.2     3.7     15.5      16.5      16.6      0.9       0.2   4.5    336.0    288.2     58.49             57.95        76    0.770    324M    46M       0.0       0.0
  L6    211/0     13.36 GB   0.0     46.8    46.8      0.0      13.4      14.1     13.4       0.0   0.3    601.0    171.5     79.81            251.89         1   79.812    731M   524M       0.0       0.0
 Sum    240/0     14.60 GB   0.0     85.6    61.5     24.1     100.2     101.5     76.1       1.2   1.9    218.7    256.0    400.70            578.23       496    0.808   2661M   624M       0.0       0.0
 Int      0/0      0.00 KB   0.0      2.4     0.7      1.7       2.4       2.4      0.7       0.0   7.6    306.8    298.0      8.08              8.58        21    0.385     62M  6404K       0.0       0.0
```
