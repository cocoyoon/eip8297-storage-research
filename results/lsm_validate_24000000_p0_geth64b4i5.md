# LSM of records on RocksDB: lsm_validate_24000000_p0_geth64b4i5

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

- 82525582 puts and 0 deletes handed in (6206.4 MB of keys and values, 75.2 B per put), plus 330102328 ballast puts (4 copies of the starting state, each under its key with one more byte so that it sorts beside the original, never updated); 115521610 live entries once loaded, 6721.9 MB on disk once compacted into the last level (58.2 B per live entry), loaded and compacted in 39s

## Replay

- 50000 blocks applied (the stream's block 0 onward), 265253769 updates in all; measured window: blocks 35000..50000 of the stream, 15000 blocks, 87906152 updates (87323587 puts, 582565 deletes, 0 relocations), 6544.7 MB of keys and values, 68.7 B per put as a record, 76.7 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793030941700753 and 1793031013126858 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 119195214 live entries tracked (127637342 estimated by the engine), 7586.8 MB on disk (63.7 B per live entry, against 74.5 B handed in per put: prefix compression and snappy at work); 287558 deletes dropped as superseded within their own commit; the replayed blocks touched 18614404 distinct ids, 4954187 of them first inside the window (0.056 per update: entries new to every level); the window's last flush and compactions settled in 247.3ms; replay 492s, whole run 492s
- the engine's memtable held 99.6 B per entry (sampled), so the 64 MB buffer holds about 673490 entries, against the 874937 entries the model gives a buffer of that size at 76.7 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 6808033825 | 77.4 | 1.04 | 453869 |
| flush | 1909693069 | 21.7 | 0.29 | 127313 |
| compaction write | 18219403351 | 207.3 | 2.78 | 1214627 |
| compaction read | 19966403686 | 227.1 | 3.05 | 1331094 |
| user bytes (engine's count) | 6808033825 | 77.45 | - | 453869 |
| keys written | 87906152 | 1.00 | - | 5860 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 20129096420 | 229.0 | **3.08** | 1341940 |

- write amplification, engine: **3.08** bytes written per user byte, **3.60** entries of 63.7 B (the on-disk entry) per update; compaction reads 3.05 per user byte, 3.57 entries per update. Disk grew by 112.4 MB over the window.
- whole replay, levels filling included: 2.59 per user byte (265253769 updates, 19740.3 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **3.60** entries written per update (flushes and compactions, the WAL apart), compaction reads 3.40; 119195214 entries, 76.7 B each, buffer 64 MB = 874937 entries, ratio 10; the stream's 265253769 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L4 54 MB, L5 648 MB, L6 6533 MB) and an append-only memtable of 673490 updates merged 2 flushes at a time: **4.86** entries written per update, compaction reads 4.59

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
| buffer | 673490 | 673490 | true | 0.276 | 1.240 | 0.964 |
| L1 | 848383 | 4147388 | true | 0.205 | 2.659 | 2.659 |
| L2 | 10180592 | 117023917 | true | 0.087 | 0.964 | 0.964 |
| L3 | 102638594 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 1.80 | 0.00 | 0.35 | 88000000 | 0 | 1.00 | buffer (flush) | 0.33 | 1.03 |
| L3 | 1.00 | 1.40 | 0.19 | 22000000 | 3724000 | 0.21 | buffer (merge) | 0.91 | 0.21 |
| L4 | 4.70 | 5.30 | 0.90 | 91000000 | 10000000 | 0.92 | L1 | 2.66 | 0.34 |
| L5 | 9.00 | 9.50 | 1.73 | 160000000 | 10000000 | 1.71 | L2 | 0.96 | 1.79 |
| L6 | 2.30 | 2.60 | 0.44 | 46000000 | 5000000 | 0.47 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 3.60 | 3.60 | 4.86 | 1.00 | 0.74 |
| bytes written per block by flushes and compactions | 1341940 | 1619922 | 1814230 | 0.83 | 0.74 |
| entries read back per update by compactions | 3.57 | 3.40 | 4.59 | 1.05 | 0.78 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        1       54
  5       13      648
  6      108     6533
```

129 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      28.5      30.4     28.5       0.0   1.0      0.0    268.9    108.60            103.88       430    0.253    677M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      2.5     1.0      1.4       2.0       2.1      0.6       4.2   2.0    285.7    238.0      8.80              8.53        74    0.119     41M  6955K       0.0       0.0
  L4      1/0     54.23 MB   0.8     17.1     4.4     12.7      15.1      15.3      2.5       0.4   3.4    293.2    259.7     59.57             58.61       301    0.198    291M    32M       0.0       0.0
  L5     13/0    648.33 MB   1.0     22.2     2.7     19.5      20.7      20.9      1.2       0.1   7.6    298.0    277.1     76.34             75.23        74    1.032    369M    26M       0.0       0.0
  L6    108/0      6.38 GB   0.0     28.3    23.9      4.4      10.8      12.8      6.4       0.0   0.5    502.0    191.3     57.82            126.03        13    4.447    498M   299M       0.0       0.0
 Sum    122/0      7.07 GB   0.0     70.1    32.1     38.0      77.1      81.5     39.1       4.6   2.7    230.6    253.9    311.13            372.28       892    0.349   1879M   366M       0.0       0.0
 Int      0/0      0.00 KB   0.0     70.1    32.1     38.0      77.1      81.5     39.1       4.6   2.7    230.6    253.9    311.13            372.28       892    0.349   1879M   366M       0.0       0.0
```
