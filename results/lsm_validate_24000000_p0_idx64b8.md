# LSM of index entries on RocksDB: lsm_validate_24000000_p0_idx64b8

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

- 82525582 puts and 0 deletes handed in (1980.6 MB of keys and values, 24.0 B per put), plus 660204656 ballast puts (8 copies of the starting state under other key prefixes, never updated); 207938898 live entries once loaded, 5943.2 MB on disk once compacted into the last level (28.6 B per live entry), loaded and compacted in 44s

## Replay

- 30000 blocks applied (the stream's block 0 onward), 292756654 updates in all; measured window: blocks 10000..30000 of the stream, 20000 blocks, 210234080 updates (104428836 puts, 688204 deletes, 105117040 relocations), 5040.1 MB of keys and values, 24.0 B per put as a record, 32.0 B per entry with the engine's 8 B of overhead
- the window's flushes and compactions in the engine's event log (`LOG`) are those finishing between unix micros 1793026251222569 and 1793026570712243 (`tools/mainnet/lsm_log_flows.py --since-micros A --until-micros B`)
- at the end: 209877148 live entries tracked (229692744 estimated by the engine), 6660.7 MB on disk (31.7 B per live entry, against 24.0 B handed in per put: prefix compression and snappy at work); 156940 deletes dropped as superseded within their own commit; the replayed blocks touched 11916983 distinct ids; the window's last flush and compactions settled in 5.3s; replay 1964s, whole run 1964s
- the engine's memtable held 51.5 B per entry (sampled), so the 64 MB buffer holds about 1303776 entries, against the 2097152 entries the model gives a buffer of that size at 32.0 B per entry

### Engine, measured window

| figure | bytes | per update | per user byte | per block |
|---|---|---|---|---|
| WAL | 5670366180 | 27.0 | 1.13 | 283518 |
| flush | 4868494267 | 23.2 | 0.97 | 243425 |
| compaction write | 65931252316 | 313.6 | 13.08 | 3296563 |
| compaction read | 70582515412 | 335.7 | 14.00 | 3529126 |
| user bytes (engine's count) | 5670366180 | 26.97 | - | 283518 |
| keys written | 210234080 | 1.00 | - | 10512 |
| stall micros | 0 | 0.00 | - | 0 |
| **flush + compaction write** | 70799746583 | 336.8 | **14.05** | 3539987 |

- write amplification, engine: **14.05** bytes written per user byte, **10.61** entries of 31.7 B (the on-disk entry) per update; compaction reads 14.00 per user byte, 10.58 entries per update. Disk grew by -0.4 MB over the window.
- whole replay, levels filling included: 13.10 per user byte (292756654 updates, 7018.5 MB).

### Model, same stream and constants

- write amplification, model at the nominal buffer: **15.07** entries written per update (flushes and compactions, the WAL apart), compaction reads 14.44; 209877148 entries, 32.0 B each, buffer 64 MB = 2097152 entries, ratio 10; the stream's 146378327 updates, longest measured window 134217728
- write amplification, model with the engine's levels (L5 522 MB, L6 5831 MB) and an append-only memtable of 1303776 updates merged 2 flushes at a time: **11.40** entries written per update, compaction reads 10.78

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
| buffer | 1303776 | 1303776 | true | 0.614 | 4.405 | 3.791 |
| L1 | 16448220 | 14315403 | true | 0.574 | 6.992 | 6.992 |
| L2 | 183734812 | - | - | 0.000 | 0.000 | 0.000 |

Engine per level over the window (a merge is booked at the level it writes into; the model's level above it is the one that sends):

| engine level | written GiB | read GiB | entries written per update (bytes / on-disk entry) | keys in | keys dropped | keys out per update | model level sending | model written per update | engine / model |
|---|---|---|---|---|---|---|---|---|---|
| L0 | 4.50 | 0.00 | 0.72 | 211000000 | 0 | 1.00 | buffer (flush) | 0.27 | 2.64 |
| L3 | 0.60 | 0.60 | 0.10 | 19000000 | 864000 | 0.09 | buffer (merge) | 4.13 | 0.02 |
| L4 | 7.30 | 7.60 | 1.17 | 233000000 | 8685000 | 1.07 | L1 | 6.99 | 0.17 |
| L5 | 32.50 | 33.20 | 5.23 | 1037000000 | 20991000 | 4.83 | L2 | 0.00 | 0.00 |
| L6 | 21.30 | 24.70 | 3.43 | 906000000 | 107000000 | 3.80 | L3 | 0.00 | 0.00 |

### Side by side

The report-time model prices an entry at its record bytes plus 8; the engine writes the on-disk entry. Both are shown.

| | engine | model, nominal buffer, record bytes | model, memtable capacity, on-disk bytes | engine / nominal | engine / like-for-like |
|---|---|---|---|---|---|
| entries written per update | 10.61 | 15.07 | 11.40 | 0.70 | 0.93 |
| bytes written per block by flushes and compactions | 3539987 | 5067855 | 3801963 | 0.70 | 0.93 |
| entries read back per update by compactions | 10.58 | 14.44 | 10.78 | 0.73 | 0.98 |

### Engine levels at the end

```
Level Files Size(MB)
--------------------
  0        0        0
  1        0        0
  2        0        0
  3        0        0
  4        0        0
  5        9      522
  6       95     5831
```

159 flushes in the window. The engine's compaction table, cumulative over the whole run (starting state included):

```
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) WPreComp(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      0/0      0.00 KB   0.0      0.0     0.0      0.0      29.8      29.8     29.8       0.0   1.0      0.0    148.1    205.97            171.09       263    0.783   1035M      0       0.0       0.0
  L3      0/0      0.00 KB   0.0      0.6     0.3      0.3       0.6       0.6      0.3       6.0   1.9    166.9    159.2      3.84              3.30        11    0.349     19M   864K       0.0       0.0
  L4      0/0      0.00 KB   0.0     10.5     4.4      6.1      10.1      10.1      4.0       1.9   2.3    173.3    166.6     61.98             52.28       153    0.405    323M    12M       0.0       0.0
  L5      9/0    521.53 MB   0.9     43.5     5.8     37.8      42.5      42.6      4.8       0.1   7.4    182.8    178.7    243.82            207.44       135    1.806   1357M    28M       0.0       0.0
  L6     95/0      5.69 GB   0.0     53.8    27.9     26.0      31.7      37.6      5.7       0.0   1.1    245.1    144.2    224.84            297.14        77    2.920   1844M   654M       0.0       0.0
 Sum    104/0      6.20 GB   0.0    108.5    38.3     70.2     114.7     120.7     44.5       8.0   3.8    150.0    158.6    740.44            731.25       639    1.159   4580M   696M       0.0       0.0
 Int      0/0      0.00 KB   0.0     36.4     6.2     30.3      36.4      38.7      6.2       3.1  14.7    183.5    183.4    203.41            196.23       243    0.837   1325M    75M       0.0       0.0
```
