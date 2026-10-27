# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64b8r2

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 14349.7 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 77s

## Replay

- 100000 blocks applied (the stream's block 0 onward), 530507538 updates in all; measured window: blocks 70000..100000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the stream's blocks were replayed 2 times over, block numbers running on (the same keys in the same order again: a longer run than the stream for levels that take longer than the stream to fill)
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793024464885582 and 1793024606363142 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 211612502 live entries tracked (226112516 estimated by the engine), 15673.3 MB on disk (74.1 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 575116 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids; the window's last flush and compactions settled in 245.0ms; replay 1147s, whole run 1147s
- the engine's memtable held 99.6 B per entry (sampled), so the 64 MB buffer holds about 673541 entries, against the 875233 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 3696128514 | 21.3 | 0.29 | 123204 |
| compaction write | 28841331913 | 166.4 | 2.24 | 961378 |
| compaction read | 32480004353 | 187.4 | 2.52 | 1082667 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 32537460427 | 187.7 | **2.52** | 1084582 |

- write amplification, engine: **2.52** bytes written per user byte, **2.53** entries of 74.1 B (the on-disk entry) per update; compaction reads 2.52 per user byte, 2.53 entries per update. Disk grew by -40.9 MB over the window.
- whole replay, levels filling included: 2.31 per user byte (530507538 updates, 39480.6 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **3.32** entries written per update (flushes and compactions, the WAL apart), compaction reads 3.12; 211612502 entries, 76.7 B each, buffer 64 MB = 875233 entries, ratio 10; the stream's 530507538 updates, longest measured window 268435456
- write amplification, model with the engine's levels (L4 105 MB, L5 1147 MB, L6 13685 MB) and an append-only memtable of 673541 updates merged 2 flushes at a time: **4.68** entries written per update, compaction reads 4.40

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 875233 | 4323319 | true | 0.202 | 0.894 | 0.692 |
| L1 | 2116125 | 13994780 | true | 0.151 | 1.663 | 1.663 |
| L2 | 21161250 | 304778362 | false | 0.069 | 0.764 | 0.764 |
| L3 | 211612502 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 673541 | 673541 | true | 0.276 | 1.663 | 1.387 |
| L1 | 1417655 | 8239518 | true | 0.172 | 2.052 | 2.052 |
| L2 | 15486199 | 207242989 | true | 0.075 | 0.966 | 0.966 |
| L3 | 184767765 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 3.40 | 0.00 | 0.28 | 174000000 | 0 | 1.00 | buffer (flush) | 0.33 | 0.85 |
| L3 | 2.80 | 3.50 | 0.23 | 60000000 | 11000000 | 0.28 | buffer (merge) | 1.33 | 0.18 |
| L4 | 14.60 | 16.10 | 1.22 | 267000000 | 24000000 | 1.40 | L1 | 2.05 | 0.60 |
| L5 | 9.40 | 10.80 | 0.79 | 184000000 | 24000000 | 0.92 | L2 | 0.97 | 0.81 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 2.53 | 3.32 | 4.68 | 0.76 | 0.54 |
| bytes written per block by flushes and compactions | 1084582 | 1471225 | 2002784 | 0.74 | 0.54 |
| entries read back per update by compactions | 2.53 | 3.12 | 4.40 | 0.81 | 0.57 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        1       10
  1        0        0
  2        0        0
  3        0        0
  4        2      105
  5       24     1147
  6      211    13685
```

254 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      1/0     10.50 MB   0.5      0.0     0.0      0.0      57.2      57.5     57.2       0.0   1.0      0.0    262.0    223.72            212.45       818    0.273   1273M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0     10.5    10.4      0.1       8.6       8.8      8.5       0.0   0.8    339.4    279.3     31.71             31.12       387    0.082    178M    31M       0.0       0.0
  L4      2/0    104.83 MB   0.8     47.9     8.5     39.4      43.6      44.2      4.2       0.0   5.1    329.0    299.6    149.12            147.61       380    0.392    796M    72M       0.0       0.0
  L5     24/0      1.12 GB   0.8     25.1     4.0     21.1      22.1      22.3      1.0       0.1   5.5    329.8    290.1     78.04             77.28        80    0.976    429M    52M       0.0       0.0
  L6    211/0     13.36 GB   0.0     46.8    46.8      0.0      13.4      14.1     13.4       0.0   0.3    654.0    186.6     73.33            239.09         1   73.334    731M   524M       0.0       0.0
 Sum    238/0     14.60 GB   0.0    130.4    69.7     60.7     145.0     146.9     84.3       0.2   2.5    240.2    267.1    555.92            707.54      1666    0.334   3409M   680M       0.0       0.0
 Int      0/0      0.00 KB   0.0    130.4    69.7     60.7     100.6     102.3     39.9       0.2   7.8    344.4    265.6    387.75            549.71      1627    0.238   2707M   680M       0.0       0.0
```
