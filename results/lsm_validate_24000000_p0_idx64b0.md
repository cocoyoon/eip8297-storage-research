# LSM of index entries on RocksDB: lsm_validate_24000000_p0_idx64b0

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

- 82525582 puts and 0 deletes handed in (1980.6 MB of keys and values, 24.0 B per put), plus 0 ballast puts (0 copies of the starting state under other key prefixes, never updated); 23104322 live entries once loaded, 673.0 MB on disk once compacted into the last level (29.1 B per live entry), loaded and compacted in 4s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 292756654 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 210234080 updates (104428836 puts, 688204 deletes, 105117040 relocations), 5040.1 MB of keys and values, 24.0 B per put as a record, 32.0 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793028582147929 and 1793028852949438 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 25042572 live entries tracked (25470991 estimated by the engine), 743.9 MB on disk (29.7 B per live entry, against 24.0 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 3.6s; replay 497s, whole run 497s
- the engine's memtable held 51.5 B per entry (sampled), so the 64 MB buffer holds about 1303570 entries, against the 2097152 entries the model gives a buffer of that size at 32.0 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 5670366180 | 27.0 | 1.13 | 283518 |
| flush | 4797149958 | 22.8 | 0.95 | 239857 |
| compaction write | 41872039776 | 199.2 | 8.31 | 2093602 |
| compaction read | 46509682862 | 221.2 | 9.23 | 2325484 |
| user bytes (engine's count) | 5670366180 | 26.97 | - | 283518 |
| keys written | 210234080 | 1.00 | - | 10512 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 46669189734 | 222.0 | **9.26** | 2333459 |

- write amplification, engine: **9.26** bytes written per user byte, **7.47** entries of 29.7 B (the on-disk entry) per update; compaction reads 9.23 per user byte, 7.45 entries per update. Disk grew by -30.7 MB over the window.
- whole replay, levels filling included: 9.15 per user byte (292756654 updates, 7018.5 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **8.85** entries written per update (flushes and compactions, the WAL apart), compaction reads 8.22; 25042572 entries, 32.0 B each, buffer 64 MB = 2097152 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 14 MB, L6 695 MB) and an append-only memtable of 1303570 updates merged 2 flushes at a time: **36.68** entries written per update, compaction reads 36.07

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2097152 | 1669161 | true | 0.628 | 2.007 | 1.378 |
| L1 | 2504257 | 2013345 | true | 0.622 | 6.841 | 6.841 |
| L2 | 25042572 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 1303570 | 1303570 | true | 0.614 | 1.341 | 0.727 |
| L1 | 471303 | 337701 | true | 0.698 | 35.339 | 35.339 |
| L2 | 23396848 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 4.40 | 0.00 | 0.76 | 211000000 | 0 | 1.00 | buffer (flush) | 0.27 | 2.76 |
| L4 | 1.30 | 1.40 | 0.22 | 45000000 | 2300000 | 0.20 | buffer (merge) | 1.07 | 0.21 |
| L5 | 8.80 | 9.30 | 1.51 | 287000000 | 13947000 | 1.30 | L1 | 35.34 | 0.04 |
| L6 | 28.90 | 32.90 | 4.97 | 1187000000 | 121000000 | 5.07 | L2 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 7.47 | 8.85 | 36.68 | 0.84 | 0.20 |
| bytes written per block by flushes and compactions | 2333459 | 2976116 | 11453451 | 0.78 | 0.20 |
| entries read back per update by compactions | 7.45 | 8.22 | 36.07 | 0.91 | 0.21 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        1       14
  6       12      695
```

159 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0       8.8       8.8      8.8       0.0   1.0      0.0    165.2     54.81             54.45       263    0.208    375M      0       0.0       0.0
  L4      0/0      0.00 KB   0.0      1.8     1.0      0.8       1.7       1.7      0.9       5.2   1.8    209.8    198.7      8.91              8.81        35    0.255     56M  2902K       0.0       0.0
  L5      1/0     14.46 MB   0.2     12.6     4.8      7.8      12.0      12.0      4.2       1.3   2.5    220.7    209.4     58.48             57.79       169    0.346    389M    19M       0.0       0.0
  L6     12/0    694.97 MB   0.0     48.0     8.1     39.9      40.6      47.9      0.7       0.0   5.0    204.1    172.6    240.65            248.69       132    1.823   1720M   224M       0.0       0.0
 Sum     13/0    709.42 MB   0.0     62.4    13.9     48.5      63.1      70.4     14.6       6.5   7.1    176.1    178.0    362.86            369.74       599    0.606   2540M   247M       0.0       0.0
 Int      0/0      0.00 KB   0.0     62.4    13.9     48.5      63.1      70.4     14.6       6.5   7.1    176.1    178.0    362.86            369.74       599    0.606   2540M   247M       0.0       0.0
```
