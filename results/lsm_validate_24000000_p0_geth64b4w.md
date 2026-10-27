# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64b4w

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 330102328 ballast puts (4 copies of the starting state under other key prefixes, never updated); 115521610 live entries once loaded, 7971.9 MB on disk once compacted into the last level (69.0 B per live entry), loaded and compacted in 29s

## Replay

- 50000 blocks applied (the stream's block 0 onward), 265253769 updates in all; measured window: blocks 35000..50000 of the stream, 15000 blocks, 87906152 updates (87323587 puts, 582565 deletes, 0 relocations), 6544.7 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793029728233528 and 1793029803373951 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 119195214 live entries tracked (129041554 estimated by the engine), 8933.8 MB on disk (75.0 B per live entry, against 74.5 B handed in per put: prefix compression and snappy at work); 287558 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids, 4954187 of them first inside the window (0.056 per update: entries new to every level); the window's last flush and compactions settled in 676.2ms; replay 514s, whole run 514s
- the engine's memtable held 99.6 B per entry (sampled), so the 64 MB buffer holds about 673490 entries, against the 874937 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 6808033825 | 77.4 | 1.04 | 453869 |
| flush | 1909693069 | 21.7 | 0.29 | 127313 |
| compaction write | 14557465084 | 165.6 | 2.22 | 970498 |
| compaction read | 16341954779 | 185.9 | 2.50 | 1089464 |
| user bytes (engine's count) | 6808033825 | 77.45 | - | 453869 |
| keys written | 87906152 | 1.00 | - | 5860 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 16467158153 | 187.3 | **2.52** | 1097811 |

- write amplification, engine: **2.52** bytes written per user byte, **2.50** entries of 75.0 B (the on-disk entry) per update; compaction reads 2.50 per user byte, 2.48 entries per update. Disk grew by 72.7 MB over the window.
- whole replay, levels filling included: 2.07 per user byte (265253769 updates, 19740.3 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **3.60** entries written per update (flushes and compactions, the WAL apart), compaction reads 3.40; 119195214 entries, 76.7 B each, buffer 64 MB = 874937 entries, ratio 10; the stream's 265253769 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 63 MB, L5 768 MB, L6 7678 MB) and an append-only memtable of 673490 updates merged 2 flushes at a time: **4.90** entries written per update, compaction reads 4.62

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 874937 | 4323082 | true | 0.202 | 0.680 | 0.478 |
| L1 | 1191952 | 6546765 | true | 0.182 | 2.003 | 2.003 |
| L2 | 11919521 | 142425613 | false | 0.084 | 0.921 | 0.921 |
| L3 | 119195214 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 673490 | 673490 | true | 0.276 | 1.235 | 0.958 |
| L1 | 840552 | 4095520 | true | 0.205 | 2.707 | 2.707 |
| L2 | 10246726 | 118084997 | true | 0.087 | 0.954 | 0.954 |
| L3 | 102440575 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.80 | 0.00 | 0.29 | 88000000 | 0 | 1.00 | buffer (flush) | 0.33 | 0.88 |
| L3 | 1.50 | 1.90 | 0.24 | 31000000 | 5000000 | 0.30 | buffer (merge) | 0.90 | 0.27 |
| L4 | 3.80 | 4.40 | 0.62 | 74000000 | 9000000 | 0.74 | L1 | 2.71 | 0.23 |
| L5 | 7.70 | 8.10 | 1.25 | 129000000 | 9000000 | 1.37 | L2 | 0.95 | 1.31 |
| L6 | 0.60 | 1.00 | 0.10 | 16000000 | 5000000 | 0.13 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 2.50 | 3.60 | 4.90 | 0.69 | 0.51 |
| bytes written per block by flushes and compactions | 1097811 | 1619922 | 2150518 | 0.68 | 0.51 |
| entries read back per update by compactions | 2.48 | 3.40 | 4.62 | 0.73 | 0.54 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        1       12
  1        0        0
  2        0        0
  3        0        0
  4        1       63
  5       14      768
  6      127     7678
```

129 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      1/0     11.82 MB   0.5      0.0     0.0      0.0      31.2      31.4     31.2       0.0   1.0      0.0    328.7     97.26             95.63       430    0.226    677M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      5.3     5.2      0.1       4.3       4.4      4.2       0.0   0.8    322.6    265.2     16.69             16.35       193    0.086     89M    15M       0.0       0.0
  L4      1/0     62.78 MB   0.8     13.3     4.2      9.1      11.6      11.8      2.5       0.1   2.8    308.0    269.3     44.07             43.53       186    0.237    228M    28M       0.0       0.0
  L5     14/0    767.58 MB   1.0     17.4     2.4     15.0      16.1      16.2      1.1       0.1   6.6    318.5    294.1     55.99             55.22        62    0.903    281M    22M       0.0       0.0
  L6    127/0      7.50 GB   0.0     27.2    26.5      0.7       8.2       8.7      7.5       0.0   0.3    907.0    273.8     30.67             96.17        10    3.067    426M   297M       0.0       0.0
 Sum    143/0      8.32 GB   0.0     63.1    38.2     24.9      71.4      72.4     46.5       0.2   2.3    264.1    298.9    244.69            306.91       881    0.278   1703M   364M       0.0       0.0
 Int      0/0      0.00 KB   0.0     63.1    38.2     24.9      71.4      72.4     46.5       0.2   2.3    264.1    298.9    244.69            306.91       881    0.278   1703M   364M       0.0       0.0
```
