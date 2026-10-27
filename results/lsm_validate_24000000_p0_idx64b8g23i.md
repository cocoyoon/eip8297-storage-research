# LSM of index entries on RocksDB: lsm_validate_24000000_p0_idx64b8g23i

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

- 82525582 puts and 0 deletes handed in (1980.6 MB of keys and values, 24.0 B per put), plus 660204656 ballast puts (8 copies of the starting state, each under its key with one more byte so that it sorts beside the original, never updated); 207938898 live entries once loaded, 2655.8 MB on disk once compacted into the last level (12.8 B per live entry), loaded and compacted in 31s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 219567490 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 157675560 updates (104428836 puts, 688204 deletes, 52558520 relocations), 3825.4 MB of keys and values, 24.0 B per put as a record, 32.0 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793031406234589 and 1793031583113962 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 209877148 live entries tracked (220509791 estimated by the engine), 3223.6 MB on disk (15.4 B per live entry, against 24.3 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids, 7012532 of them first inside the window (0.044 per update: entries new to every level); the window's last flush and compactions settled in 7.0s; replay 603s, whole run 603s
- the engine's memtable held 51.5 B per entry (sampled), so the 64 MB buffer holds about 1304102 entries, against the 2097152 entries the model gives a buffer of that size at 32.0 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 4298008681 | 27.3 | 1.12 | 214900 |
| flush | 2996253639 | 19.0 | 0.78 | 149813 |
| compaction write | 29721419083 | 188.5 | 7.77 | 1486071 |
| compaction read | 32470448745 | 205.9 | 8.49 | 1623522 |
| user bytes (engine's count) | 4298008681 | 27.26 | - | 214900 |
| keys written | 157675560 | 1.00 | - | 7884 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 32717672722 | 207.5 | **8.55** | 1635884 |

- write amplification, engine: **8.55** bytes written per user byte, **13.51** entries of 15.4 B (the on-disk entry) per update; compaction reads 8.49 per user byte, 13.41 entries per update. Disk grew by 133.5 MB over the window.
- whole replay, levels filling included: 8.07 per user byte (219567490 updates, 5327.0 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **11.29** entries written per update (flushes and compactions, the WAL apart), compaction reads 10.81; 209877148 entries, 32.0 B each, buffer 64 MB = 2097152 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 270 MB, L6 2804 MB) and an append-only memtable of 1304102 updates merged 2 flushes at a time: **10.25** entries written per update, compaction reads 9.76

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
| buffer | 1304102 | 1304102 | true | 0.485 | 5.495 | 5.009 |
| L1 | 17579009 | 28067234 | true | 0.418 | 4.754 | 4.754 |
| L2 | 182561268 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 2.80 | 0.00 | 1.24 | 159000000 | 0 | 1.01 | buffer (flush) | 0.27 | 4.53 |
| L4 | 5.80 | 6.10 | 2.57 | 187000000 | 9875000 | 1.12 | L1 | 4.75 | 0.54 |
| L5 | 9.80 | 10.30 | 4.35 | 334000000 | 10228000 | 2.05 | L2 | 0.00 | 0.00 |
| L6 | 12.30 | 14.10 | 5.45 | 1037000000 | 63000000 | 6.18 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 13.51 | 11.29 | 10.25 | 1.20 | 1.32 |
| bytes written per block by flushes and compactions | 1635884 | 2847394 | 1240962 | 0.57 | 1.32 |
| entries read back per update by compactions | 13.41 | 10.81 | 9.76 | 1.24 | 1.37 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        5      270
  6       43     2804
```

120 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      14.4      20.3     14.4       0.0   1.0      0.0    131.4    111.87            111.20       208    0.538    962M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      0.0     0.0      0.0       0.0       0.0      0.0       3.8   1.8    196.2    182.8      0.24              0.24         1    0.238   1388K    88K       0.0       0.0
  L4      0/0      0.00 KB   0.0      8.1     2.9      5.1       7.6       7.6      2.5       0.9   2.6    197.4    185.8     41.80             41.57       126    0.332    246M    13M       0.0       0.0
  L5      5/0    270.26 MB   1.0     13.9     3.4     10.6      13.2      13.2      2.7       0.0   3.9    193.3    183.9     73.79             73.51        88    0.839    446M    14M       0.0       0.0
  L6     43/0      2.74 GB   0.0     28.0    12.9     15.1      17.8      32.1      2.7       0.0   1.4    131.2     83.4    218.39            296.88        44    4.963   2033M   600M       0.0       0.0
 Sum     48/0      3.00 GB   0.0     50.0    19.3     30.8      53.0      73.2     22.3       4.8   3.7    114.8    121.7    446.09            523.39       467    0.955   3690M   628M       0.0       0.0
 Int      0/0      0.00 KB   0.0      0.8     0.1      0.7       0.7       1.0      0.0       0.0 745674918.0    125.9    115.8      6.14              6.13         2    3.070     43M  1999K       0.0       0.0
```
