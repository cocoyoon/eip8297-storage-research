# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64b1w

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 82525582 ballast puts (1 copies of the starting state under other key prefixes, never updated); 46208644 live entries once loaded, 3188.8 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 11s

## Replay

- 50000 blocks applied (the stream's block 0 onward), 265253769 updates in all; measured window: blocks 20000..50000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793029861447549 and 1793029991219625 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 49882248 live entries tracked (54030387 estimated by the engine), 3726.9 MB on disk (74.7 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 287558 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids, 9933166 of them first inside the window (0.057 per update: entries new to every level); the window's last flush and compactions settled in 879.0ms; replay 312s, whole run 312s
- the engine's memtable held 99.6 B per entry (sampled), so the 64 MB buffer holds about 673503 entries, against the 875233 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 3696968461 | 21.3 | 0.29 | 123232 |
| compaction write | 29622505811 | 170.9 | 2.30 | 987417 |
| compaction read | 33070507819 | 190.8 | 2.56 | 1102350 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 33319474272 | 192.3 | **2.58** | 1110649 |

- write amplification, engine: **2.58** bytes written per user byte, **2.57** entries of 74.7 B (the on-disk entry) per update; compaction reads 2.56 per user byte, 2.55 entries per update. Disk grew by 161.6 MB over the window.
- whole replay, levels filling included: 2.41 per user byte (265253769 updates, 19740.3 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **2.80** entries written per update (flushes and compactions, the WAL apart), compaction reads 2.60; 49882248 entries, 76.7 B each, buffer 64 MB = 875233 entries, ratio 10; the stream's 265253769 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 308 MB, L6 3234 MB) and an append-only memtable of 673503 updates merged 2 flushes at a time: **5.06** entries written per update, compaction reads 4.78

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 875233 | 4325048 | true | 0.202 | 1.558 | 1.356 |
| L1 | 4988225 | 44131125 | true | 0.113 | 1.243 | 1.243 |
| L2 | 49882248 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 673503 | 673503 | true | 0.276 | 3.671 | 3.395 |
| L1 | 4122337 | 34228987 | true | 0.120 | 1.385 | 1.385 |
| L2 | 43284539 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 3.40 | 0.00 | 0.28 | 174000000 | 0 | 1.00 | buffer (flush) | 0.33 | 0.84 |
| L3 | 2.80 | 3.40 | 0.23 | 58000000 | 9987000 | 0.28 | buffer (merge) | 3.34 | 0.07 |
| L4 | 2.40 | 2.90 | 0.20 | 48000000 | 7968000 | 0.23 | L1 | 1.38 | 0.14 |
| L5 | 18.90 | 20.30 | 1.57 | 338000000 | 24000000 | 1.81 | L2 | 0.00 | 0.00 |
| L6 | 3.60 | 4.40 | 0.30 | 70000000 | 14000000 | 0.32 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 2.57 | 2.80 | 5.06 | 0.92 | 0.51 |
| bytes written per block by flushes and compactions | 1110649 | 1240897 | 2182242 | 0.90 | 0.51 |
| entries read back per update by compactions | 2.55 | 2.60 | 4.78 | 0.98 | 0.53 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        1       12
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        6      308
  6       57     3234
```

254 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      1/0     11.82 MB   0.5      0.0     0.0      0.0      15.6      15.7     15.6       0.0   1.0      0.0    348.1     45.92             45.28       430    0.107    430M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      5.2     5.2      0.0       4.3       4.3      4.3       0.0   0.8    347.7    287.0     15.23             15.00       193    0.079     87M    15M       0.0       0.0
  L4      0/0      0.00 KB   0.0      4.3     2.1      2.1       3.6       3.6      1.4       2.1   1.7    351.4    291.1     12.49             12.40        97    0.129     72M    12M       0.0       0.0
  L5      6/0    308.32 MB   1.0     28.7     3.5     25.1      26.7      27.0      1.5       0.0   7.6    338.3    314.7     86.73             86.29        96    0.903    479M    34M       0.0       0.0
  L6     57/0      3.16 GB   0.0     16.2    11.7      4.5       7.7       8.1      3.2       0.0   0.7    621.4    294.7     26.65             50.66        24    1.110    254M   134M       0.0       0.0
 Sum     64/0      3.47 GB   0.0     54.3    22.5     31.8      57.8      58.7     26.0       2.2   3.7    297.2    316.2    187.02            209.63       840    0.223   1324M   196M       0.0       0.0
 Int      0/0      0.00 KB   0.0     54.3    22.5     31.8      57.8      58.7     26.0       2.2   3.7    297.2    316.2    187.02            209.63       840    0.223   1324M   196M       0.0       0.0
```
