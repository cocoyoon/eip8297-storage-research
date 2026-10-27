# LSM of records on RocksDB: lsm_validate_24000000_p0_geth256b0r2

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 0 ballast puts (0 copies of the starting state under other key prefixes, never updated); 23104322 live entries once loaded, 1594.4 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 5s

## Replay

- 100000 blocks applied (the stream's block 0 onward), 530507538 updates in all; measured window: blocks 70000..100000 of the stream, 30000 blocks, 173310015 updates (172136623 puts, 1173392 deletes, 0 relocations), 12900.8 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the stream's blocks were replayed 2 times over, block numbers running on (the same keys in the same order again: a longer run than the stream for levels that take longer than the stream to fill)
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793023283812917 and 1793023445586689 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 26777926 live entries tracked (29382319 estimated by the engine), 2024.9 MB on disk (75.6 B per live entry, against 74.4 B handed in per put: prefix compression and snappy at work); 575116 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids; the window's last flush and compactions settled in 2.0s; replay 550s, whole run 550s
- the engine's memtable held 98.6 B per entry (sampled), so the 256 MB buffer holds about 2723630 entries, against the 3500933 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 13419883745 | 77.4 | 1.04 | 447329 |
| flush | 2536371055 | 14.6 | 0.20 | 84546 |
| compaction write | 14729886928 | 85.0 | 1.14 | 490996 |
| compaction read | 17193978589 | 99.2 | 1.33 | 573133 |
| user bytes (engine's count) | 13419883745 | 77.43 | - | 447329 |
| keys written | 173310015 | 1.00 | - | 5777 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 17266257983 | 99.6 | **1.34** | 575542 |

- write amplification, engine: **1.34** bytes written per user byte, **1.32** entries of 75.6 B (the on-disk entry) per update; compaction reads 1.33 per user byte, 1.31 entries per update. Disk grew by -58.9 MB over the window.
- whole replay, levels filling included: 1.30 per user byte (530507538 updates, 39480.6 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **1.24** entries written per update (flushes and compactions, the WAL apart), compaction reads 1.11; 26777926 entries, 76.7 B each, buffer 256 MB = 3500933 entries, ratio 10; the stream's 530507538 updates, longest measured window 268435456
- write amplification, model with the engine's levels (L4 15 MB, L5 164 MB, L6 1753 MB) and an append-only memtable of 2723630 updates merged 2 flushes at a time: **6.40** entries written per update, compaction reads 6.21

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 3500933 | 27337067 | true | 0.128 | 1.236 | 1.108 |
| L1 | 26777926 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2723630 | 2723630 | true | 0.191 | 0.456 | 0.265 |
| L1 | 198369 | 564853 | true | 0.351 | 4.191 | 4.191 |
| L2 | 2168835 | 14457426 | true | 0.150 | 1.754 | 1.754 |
| L3 | 23182728 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.30 | 0.00 | 0.19 | 176000000 | 0 | 1.02 | buffer (flush) | 0.23 | 0.82 |
| L4 | 3.10 | 3.90 | 0.25 | 67000000 | 14000000 | 0.31 | buffer (merge) | 0.23 | 1.12 |
| L5 | 3.10 | 3.50 | 0.25 | 58000000 | 7000000 | 0.29 | L1 | 4.19 | 0.06 |
| L6 | 7.80 | 9.00 | 0.64 | 144000000 | 21000000 | 0.71 | L2 | 1.75 | 0.36 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 1.32 | 1.24 | 6.40 | 1.07 | 0.21 |
| bytes written per block by flushes and compactions | 575542 | 547348 | 2795887 | 1.05 | 0.21 |
| entries read back per update by compactions | 1.31 | 1.11 | 6.21 | 1.18 | 0.21 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        2       15
  5        3      164
  6       28     1753
```

65 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      11.7      11.8     11.7       0.0   1.0      0.0    238.7     50.29             49.92       216    0.233    613M      0       0.0       0.0
  L4      2/0     14.73 MB   0.2     11.6     7.1      4.5       9.3       9.5      4.8       0.0   1.3    533.9    428.5     22.30             38.71        97    0.230    197M    39M       0.0       0.0
  L5      3/0    163.57 MB   0.9     10.1     4.3      5.8       8.9       9.0      3.1       0.5   2.1    324.1    287.5     31.82             31.52        98    0.325    168M    19M       0.0       0.0
  L6     28/0      1.71 GB   0.0     30.3     8.1     22.1      23.9      25.2      1.7       0.0   2.9    368.0    290.1     84.22             93.46        71    1.186    482M   104M       0.0       0.0
 Sum     33/0      1.89 GB   0.0     52.0    19.5     32.5      53.8      55.6     21.4       0.5   4.6    282.1    292.3    188.63            213.61       482    0.391   1460M   163M       0.0       0.0
 Int      0/0      0.00 KB   0.0     52.0    19.5     32.5      53.8      55.6     21.4       0.5   4.6    282.1    292.3    188.63            213.61       482    0.391   1460M   163M       0.0       0.0
```
