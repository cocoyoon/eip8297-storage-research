# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64b4i

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 330102328 ballast puts (4 copies of the starting state, each under its key with one more byte so that it sorts beside the original, never updated); 115521610 live entries once loaded, 6721.9 MB on disk once compacted into the last level (58.2 B per live entry), loaded and compacted in 36s

## Replay

- 50000 blocks applied (the stream's block 0 onward), 265253769 updates in all; measured window: blocks 35000..50000 of the stream, 15000 blocks, 87906152 updates (87323587 puts, 582565 deletes, 0 relocations), 6544.7 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793030813162928 and 1793030902614708 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 119195214 live entries tracked (126832670 estimated by the engine), 7535.8 MB on disk (63.2 B per live entry, against 74.5 B handed in per put: prefix compression and snappy at work); 287558 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids, 4954187 of them first inside the window (0.056 per update: entries new to every level); the window's last flush and compactions settled in 2.6s; replay 510s, whole run 510s
- the engine's memtable held 99.6 B per entry (sampled), so the 64 MB buffer holds about 673490 entries, against the 874937 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 6808033825 | 77.4 | 1.04 | 453869 |
| flush | 1909693076 | 21.7 | 0.29 | 127313 |
| compaction write | 17475401546 | 198.8 | 2.67 | 1165027 |
| compaction read | 19306512614 | 219.6 | 2.95 | 1287101 |
| user bytes (engine's count) | 6808033825 | 77.45 | - | 453869 |
| keys written | 87906152 | 1.00 | - | 5860 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 19385094622 | 220.5 | **2.96** | 1292340 |

- write amplification, engine: **2.96** bytes written per user byte, **3.49** entries of 63.2 B (the on-disk entry) per update; compaction reads 2.95 per user byte, 3.47 entries per update. Disk grew by 32.6 MB over the window.
- whole replay, levels filling included: 2.35 per user byte (265253769 updates, 19740.3 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **3.60** entries written per update (flushes and compactions, the WAL apart), compaction reads 3.40; 119195214 entries, 76.7 B each, buffer 64 MB = 874937 entries, ratio 10; the stream's 265253769 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 631 MB, L6 6544 MB) and an append-only memtable of 673490 updates merged 2 flushes at a time: **9.02** entries written per update, compaction reads 8.74

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
| buffer | 673490 | 673490 | true | 0.276 | 8.020 | 7.744 |
| L1 | 9980598 | 113831666 | true | 0.088 | 0.997 | 0.997 |
| L2 | 103507188 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.80 | 0.00 | 0.35 | 88000000 | 0 | 1.00 | buffer (flush) | 0.33 | 1.04 |
| L3 | 1.50 | 1.80 | 0.29 | 31000000 | 5000000 | 0.30 | buffer (merge) | 7.69 | 0.04 |
| L4 | 3.30 | 3.80 | 0.64 | 64000000 | 9000000 | 0.63 | L1 | 1.00 | 0.64 |
| L5 | 8.50 | 9.10 | 1.64 | 150000000 | 10000000 | 1.59 | L2 | 0.00 | 0.00 |
| L6 | 3.00 | 3.40 | 0.58 | 59000000 | 6000000 | 0.60 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 3.49 | 3.60 | 9.02 | 0.97 | 0.39 |
| bytes written per block by flushes and compactions | 1292340 | 1619922 | 3340922 | 0.80 | 0.39 |
| entries read back per update by compactions | 3.47 | 3.40 | 8.74 | 1.02 | 0.40 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        1       12
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5       11      631
  6      108     6544
```

129 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      1/0     11.82 MB   0.5      0.0     0.0      0.0      28.5      30.4     28.5       0.0   1.0      0.0    269.7    108.28            103.53       430    0.252    677M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      5.3     5.2      0.1       4.3       4.4      4.2       0.0   0.8    275.7    226.8     19.53             18.53       193    0.101     89M    15M       0.0       0.0
  L4      0/0      0.00 KB   0.0     10.9     3.9      6.9       9.4       9.5      2.4       0.3   2.4    264.5    228.2     42.03             40.09       177    0.237    186M    25M       0.0       0.0
  L5     11/0    630.55 MB   1.0     20.9     2.7     18.2      19.4      19.6      1.2       0.1   7.2    270.0    250.5     79.17             76.47        73    1.085    343M    25M       0.0       0.0
  L6    108/0      6.39 GB   0.0     28.7    24.0      4.7      11.1      13.3      6.4       0.0   0.5    490.3    190.1     59.96            125.41        12    4.997    506M   300M       0.0       0.0
 Sum    120/0      7.02 GB   0.0     65.7    35.8     29.9      72.7      77.1     42.8       0.4   2.5    217.7    241.0    308.98            364.03       885    0.349   1803M   366M       0.0       0.0
 Int      0/0      0.00 KB   0.0     65.7    35.8     29.9      72.7      77.1     42.8       0.4   2.5    217.7    241.0    308.98            364.03       885    0.349   1803M   366M       0.0       0.0
```
