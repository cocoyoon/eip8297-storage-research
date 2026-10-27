# LSM of index entries on RocksDB: lsm_validate_24000000_p0_idx64b8g23

> MEASURED on RocksDB 11.8.1 (crate rocksdb 0.25, librocksdb-sys 0.19) fed the record stream `/Users/raf/Dev/research/el-storage-data/stream_24000000_50k_p0.bin` as the disk index of a log store sees it: every record update is a put of its 16-byte hashed key with an 8-byte value (the log position), a delete removes the key, and every block adds 0.50 puts per update to live entries chosen at random, ballast copies included (the records the log's garbage collection moves). The model is `pbt_store::lsm::cost_from_curve` on the same stream with that relocation share, with this database's live entries as the full state.

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
| index mode | 16 B hashed keys, 8 B values, 0.50 relocations per update |

## Starting state

- 82525582 puts and 0 deletes handed in (1980.6 MB of keys and values, 24.0 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 5943.2 MB on disk once compacted into the last level (28.6 B per live entry), loaded and compacted in 36s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 219567490 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 157675560 updates (104428836 puts, 688204 deletes, 52558520 relocations), 3778.7 MB of keys and values, 24.0 B per put as a record, 32.0 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793028129732419 and 1793028355640337 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 209877148 live entries tracked (228363387 estimated by the engine), 6633.9 MB on disk (31.6 B per live entry, against 24.0 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 3.4s; replay 1785s, whole run 1785s
- the engine's memtable held 51.5 B per entry (sampled), so the 64 MB buffer holds about 1304102 entries, against the 2097152 entries the model gives a buffer of that size at 32.0 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 4251286140 | 27.0 | 1.13 | 212564 |
| flush | 2959090095 | 18.8 | 0.78 | 147955 |
| compaction write | 41989681353 | 266.3 | 11.11 | 2099484 |
| compaction read | 44797698644 | 284.1 | 11.86 | 2239885 |
| user bytes (engine's count) | 4251286140 | 26.96 | - | 212564 |
| keys written | 157675560 | 1.00 | - | 7884 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 44948771448 | 285.1 | **11.90** | 2247439 |

- write amplification, engine: **11.90** bytes written per user byte, **9.02** entries of 31.6 B (the on-disk entry) per update; compaction reads 11.86 per user byte, 8.99 entries per update. Disk grew by 41.0 MB over the window.
- whole replay, levels filling included: 10.57 per user byte (219567490 updates, 5262.0 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **11.29** entries written per update (flushes and compactions, the WAL apart), compaction reads 10.81; 209877148 entries, 32.0 B each, buffer 64 MB = 2097152 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 563 MB, L6 5764 MB) and an append-only memtable of 1304102 updates merged 2 flushes at a time: **10.24** entries written per update, compaction reads 9.76

Model levels, nominal buffer, the report-time model:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 2097152 | 2906039 | true | 0.481 | 1.444 | 0.963 |
| L1 | 2098771 | 2908477 | true | 0.481 | 5.292 | 5.292 |
| L2 | 20987715 | 33817779 | true | 0.414 | 4.551 | 4.551 |
| L3 | 209877148 | - | - | 0.000 | 0.000 | 0.000 |

Model levels, engine's levels and memtable:

| level | entries | fills over (updates) | inside the stream | outflow per update | written per update | read per update |
|---|---|---|---|---|---|---|
| buffer | 1304102 | 1304102 | true | 0.485 | 5.554 | 5.069 |
| L1 | 17811635 | 28458234 | true | 0.417 | 4.689 | 4.689 |
| L2 | 182355712 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.70 | 0.00 | 0.58 | 159000000 | 0 | 1.01 | buffer (flush) | 0.27 | 2.12 |
| L4 | 4.40 | 4.70 | 0.95 | 146000000 | 7813000 | 0.88 | buffer (merge) | 5.28 | 0.18 |
| L5 | 23.20 | 23.70 | 5.00 | 743000000 | 17113000 | 4.60 | L1 | 4.69 | 1.07 |
| L6 | 11.60 | 13.50 | 2.50 | 496000000 | 57000000 | 2.78 | L2 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 9.02 | 11.29 | 10.24 | 0.80 | 0.88 |
| bytes written per block by flushes and compactions | 2247439 | 2847394 | 2552539 | 0.79 | 0.88 |
| entries read back per update by compactions | 8.99 | 10.81 | 9.76 | 0.83 | 0.92 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        9      563
  6       96     5764
```

120 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      27.3      27.3     27.3       0.0   1.0      0.0    140.4    199.23            160.71       208    0.958    962M      0       0.0       0.0
  L4      0/0      0.00 KB   0.0      6.6     2.8      3.8       6.2       6.2      2.4       1.0   2.2    210.8    198.2     31.99             31.61       122    0.262    203M    11M       0.0       0.0
  L5      9/0    562.79 MB   1.0     29.2     3.4     25.9      28.5      28.5      2.6       0.1   8.5    219.4    213.7    136.43            135.07        81    1.684    912M    22M       0.0       0.0
  L6     96/0      5.63 GB   0.0     38.8    25.6     13.2      18.8      22.4      5.6       0.0   0.7    343.1    166.5    115.88            211.75        38    3.049   1294M   586M       0.0       0.0
 Sum    105/0      6.18 GB   0.0     74.6    31.8     42.9      80.8      84.4     37.9       1.1   3.0    158.1    171.2    483.52            539.13       449    1.077   3372M   620M       0.0       0.0
 Int      0/0      0.00 KB   0.0     74.6    31.8     42.9      61.5      65.1     18.6       1.1   7.7    226.1    186.4    337.99            431.27       415    0.814   2760M   620M       0.0       0.0
```
