# LSM of index entries on RocksDB: lsm_validate_24000000_p0_idx64b8i

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin` as the disk index of a log store sees it: every record update is a put of its 16-byte hashed key with an 8-byte value (the log position), a delete removes the key, and every block adds 1.00 puts per update to live entries chosen at random, ballast copies included (the records the log's garbage collection moves). The model is `pbt_store::lsm::cost_from_curve` on the same stream with that relocation share, with this database's live entries as the full state.

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
| key layout | Hash |
| index mode | 16 B hashed keys, 8 B values, 1.00 relocations per update |

## Starting state

- 82525582 puts and 0 deletes handed in (1980.6 MB of keys and values, 24.0 B per put), plus 660204656 ballast puts (8 copies of the starting state, each under its key with one more byte so that it sorts beside the original, never updated); 207938898 live entries once loaded, 2655.8 MB on disk once compacted into the last level (12.8 B per live entry), loaded and compacted in 38s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 292756654 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 210234080 updates (104428836 puts, 688204 deletes, 105117040 relocations), 5133.5 MB of keys and values, 24.0 B per put as a record, 32.0 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793030638145078 and 1793030980251796 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 209877148 live entries tracked (222096083 estimated by the engine), 3384.5 MB on disk (16.1 B per live entry, against 24.4 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids, 7012532 of them first inside the window (0.033 per update: entries new to every level); the window's last flush and compactions settled in 10.6s; replay 946s, whole run 946s
- the engine's memtable held 51.5 B per entry (sampled), so the 64 MB buffer holds about 1303776 entries, against the 2097152 entries the model gives a buffer of that size at 32.0 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 5763801960 | 27.4 | 1.12 | 288190 |
| flush | 4937419740 | 23.5 | 0.96 | 246871 |
| compaction write | 50435530733 | 239.9 | 9.82 | 2521777 |
| compaction read | 54994972349 | 261.6 | 10.71 | 2749749 |
| user bytes (engine's count) | 5763801960 | 27.42 | - | 288190 |
| keys written | 210234080 | 1.00 | - | 10512 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 55372950473 | 263.4 | **10.79** | 2768648 |

- write amplification, engine: **10.79** bytes written per user byte, **16.33** entries of 16.1 B (the on-disk entry) per update; compaction reads 10.71 per user byte, 16.22 entries per update. Disk grew by 185.9 MB over the window.
- whole replay, levels filling included: 10.41 per user byte (292756654 updates, 7148.6 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **15.07** entries written per update (flushes and compactions, the WAL apart), compaction reads 14.44; 209877148 entries, 32.0 B each, buffer 64 MB = 2097152 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 241 MB, L6 2986 MB) and an append-only memtable of 1303776 updates merged 2 flushes at a time: **11.83** entries written per update, compaction reads 11.22

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2097152 | 1669161 | true | 0.628 | 1.885 | 1.257 |
| L1 | 2098771 | 1670525 | true | 0.628 | 6.910 | 6.910 |
| L2 | 20987715 | 18407178 | true | 0.570 | 6.271 | 6.271 |
| L3 | 209877148 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 1303776 | 1303776 | true | 0.614 | 4.117 | 3.503 |
| L1 | 14944770 | 12966641 | true | 0.576 | 7.716 | 7.716 |
| L2 | 185166315 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 4.60 | 0.00 | 1.46 | 211000000 | 0 | 1.00 | buffer (flush) | 0.27 | 5.32 |
| L4 | 8.20 | 8.50 | 2.60 | 259000000 | 9267000 | 1.19 | buffer (merge) | 3.84 | 0.68 |
| L5 | 17.40 | 18.10 | 5.51 | 586000000 | 13665000 | 2.72 | L1 | 7.72 | 0.71 |
| L6 | 21.90 | 25.10 | 6.94 | 1779000000 | 115000000 | 7.91 | L2 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 16.33 | 15.07 | 11.83 | 1.08 | 1.38 |
| bytes written per block by flushes and compactions | 2768648 | 5067855 | 2005842 | 0.55 | 1.38 |
| entries read back per update by compactions | 16.22 | 14.44 | 11.22 | 1.12 | 1.45 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        4      241
  6       47     2986
```

159 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      16.9      22.8     16.9       0.0   1.0      0.0    124.7    138.57            134.91       263    0.527   1035M      0       0.0       0.0
  L4      0/0      0.00 KB   0.0     10.9     4.4      6.5      10.5      10.5      3.9       2.0   2.4    169.5    162.0     66.05             63.94       153    0.432    333M    12M       0.0       0.0
  L5      4/0    241.45 MB   0.8     25.1     5.9     19.3      24.1      24.1      4.8       0.0   4.1    168.1    161.1    153.10            148.34       144    1.063    808M    19M       0.0       0.0
  L6     47/0      2.92 GB   0.0     43.1    15.1     28.0      30.9      53.9      2.9       0.0   2.0    104.2     74.7    423.29            512.54        84    5.039   3079M   671M       0.0       0.0
 Sum     51/0      3.15 GB   0.0     79.1    25.4     53.7      82.3     111.2     28.6       2.0   4.9    103.8    107.9    781.01            859.73       644    1.213   5256M   703M       0.0       0.0
 Int      0/0      0.00 KB   0.0     52.0    11.3     40.6      52.2      67.6     11.6       1.2  11.2    109.6    110.0    485.77            474.93       451    1.077   2843M   139M       0.0       0.0
```
