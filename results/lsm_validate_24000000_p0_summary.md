## Entries written per update, all runs

| run | buffer | level base | key | ballast | levels below L0 (fed in the window) | engine, entries by the event log | engine, bytes / on-disk entry | model nominal | engine / model, by entries | engine / model, by bytes | model, engine's levels | compactions into the last level | first touches per update |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| geth256b0w | 256 MB | 64 MB | Path | 0, prefix | 3 (3 fed) | 1.56 | 1.39 | 1.23 | 1.27 | 1.13 | 4.39 | 25 | - |
| geth256b0r2 | 256 MB | 64 MB | Path | 0, prefix | 3 (3 fed) | 1.54 | 1.32 | 1.24 | 1.24 | 1.06 | 6.40 | 24 | - |
| geth256b8r2 | 256 MB | 64 MB | Path | 8, prefix | 4 (3 fed) | 1.47 | 1.24 | 1.79 | 0.82 | 0.69 | 5.25 | 0 | - |
| rocks256b8r2 | 256 MB | 256 MB | Path | 8, prefix | 4 (2 fed) | 1.01 | 0.85 | 1.79 | 0.56 | 0.47 | 2.49 | 0 | - |
| geth256b8hr2 | 256 MB | 64 MB | Hash | 8, prefix | 4 (3 fed) | 1.41 | 1.19 | 1.79 | 0.79 | 0.66 | 4.94 | 0 | - |
| geth64b8r2 | 64 MB | 16 MB | Path | 8, prefix | 4 (3 fed) | 2.94 | 2.53 | 3.32 | 0.89 | 0.76 | 4.68 | 0 | - |
| geth64b1w | 64 MB | 16 MB | Path | 1, prefix | 3 (4 fed) | 2.98 | 2.57 | 2.80 | 1.06 | 0.92 | 5.06 | 18 | 0.057 |
| geth64b2w | 64 MB | 16 MB | Path | 2, prefix | 3 (4 fed) | 2.96 | 2.57 | 3.18 | 0.93 | 0.81 | 6.25 | 16 | 0.057 |
| geth64b4w | 64 MB | 16 MB | Path | 4, prefix | 4 (4 fed) | 2.86 | 2.50 | 3.60 | 0.79 | 0.69 | 4.90 | 8 | 0.056 |
| geth64b2i | 64 MB | 16 MB | Path | 2, beside | 4 (4 fed) | 3.22 | 2.92 | 3.18 | 1.01 | 0.92 | 5.07 | 15 | 0.057 |
| geth64b4i | 64 MB | 16 MB | Path | 4, beside | 3 (4 fed) | 3.47 | 3.49 | 3.60 | 0.96 | 0.97 | 9.02 | 7 | 0.056 |
| geth64b2i5 | 64 MB | 2 MB | Path | 2, beside | 2 (4 fed) | 3.53 | 3.28 | 3.18 | 1.11 | 1.03 | 6.06 | 15 | 0.057 |
| geth64b4i5 | 64 MB | 2 MB | Path | 4, beside | 3 (4 fed) | 3.64 | 3.60 | 3.60 | 1.01 | 1.00 | 4.86 | 6 | 0.056 |
| idx64b0 | 64 MB | 16 MB | Hash | 0, prefix | 2 (3 fed) | 7.21 | 7.47 | 8.85 | 0.82 | 0.84 | 36.68 | 94 | - |
| idx64b8 | 64 MB | 16 MB | Hash | 8, prefix | 2 (4 fed) | 10.44 | 10.61 | 15.07 | 0.69 | 0.70 | 11.40 | 63 | - |
| idx64b8g23 | 64 MB | 16 MB | Hash | 8, prefix | 2 (3 fed) | 8.79 | 9.02 | 11.29 | 0.78 | 0.80 | 10.24 | 33 | - |
| idx64b8i | 64 MB | 16 MB | Hash | 8, beside | 2 (3 fed) | 12.49 | 16.33 | 15.07 | 0.83 | 1.08 | 11.83 | 66 | 0.033 |
| idx64b8g23i | 64 MB | 16 MB | Hash | 8, beside | 2 (4 fed) | 9.89 | 13.51 | 11.29 | 0.88 | 1.20 | 10.25 | 36 | 0.044 |

### geth256b0w

buffer 256 MB, L0 trigger 2, level base 64 MB, key Path, ballast 0, prefix; window 173310015 updates (by unix micros); levels at the end: L4 26 MB, L5 138 MB, L6 1728 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.232 | buffer | 0.191 | 0.484 | - |
| L4 | 26 | 0.235 | 0.149 | 0.6 | 0.0 | 0.307 | buffer | 0.191 | 0.484 | 1.23 |
| L5 | 138 | 0.143 | 0.189 | 1.3 | 5.3 | 0.293 | L1 | 0.282 | 1.780 | 0.51 |
| L6 | 1728 | 0.119 | 0.713 | 6.0 | 12.5 | 0.726 | L2 | 0.157 | 2.125 | 0.76 |

Entries written per update over the window: 1.56 by the event log; 1.39 by the byte tickers over the on-disk entry; model nominal 1.23, model with the engine's levels and memtable 4.39.

Compactions into the engine's last level (L6) inside the window: 25.

### geth256b0r2

buffer 256 MB, L0 trigger 2, level base 64 MB, key Path, ballast 0, prefix; window 173310015 updates (by unix micros); levels at the end: L4 15 MB, L5 164 MB, L6 1753 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.232 | buffer | 0.191 | 0.456 | - |
| L4 | 15 | 0.232 | 0.154 | 0.7 | 0.0 | 0.307 | buffer | 0.191 | 0.456 | 1.21 |
| L5 | 164 | 0.130 | 0.200 | 1.5 | 10.9 | 0.293 | L1 | 0.351 | 4.191 | 0.37 |
| L6 | 1753 | 0.120 | 0.707 | 5.9 | 10.7 | 0.707 | L2 | 0.150 | 1.754 | 0.80 |

Entries written per update over the window: 1.54 by the event log; 1.32 by the byte tickers over the on-disk entry; model nominal 1.24, model with the engine's levels and memtable 6.40.

Compactions into the engine's last level (L6) inside the window: 24.

### geth256b8r2

buffer 256 MB, L0 trigger 2, level base 64 MB, key Path, ballast 8, prefix; window 173310015 updates (by unix micros); levels at the end: L3 24 MB, L4 93 MB, L5 1147 MB, L6 13685 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.232 | buffer | 0.191 | 0.479 | - |
| L3 | 24 | 0.232 | 0.137 | 0.6 | 0.0 | 0.296 | buffer | 0.191 | 0.479 | 1.21 |
| L4 | 93 | 0.137 | 0.144 | 1.1 | 3.9 | 0.248 | L1 | 0.291 | 1.417 | 0.47 |
| L5 | 1147 | 0.135 | 0.698 | 5.2 | 12.3 | 0.698 | L2 | 0.179 | 2.390 | 0.75 |

Entries written per update over the window: 1.47 by the event log; 1.24 by the byte tickers over the on-disk entry; model nominal 1.79, model with the engine's levels and memtable 5.25.

Compactions into the engine's last level (L6) inside the window: 0.

### rocks256b8r2

buffer 256 MB, L0 trigger 4, level base 256 MB, key Path, ballast 8, prefix; window 173310015 updates (by unix micros); levels at the end: L0 79 MB, L4 214 MB, L5 1147 MB, L6 13685 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.232 | buffer | 0.161 | 0.651 | - |
| L4 | 214 | 0.232 | 0.313 | 1.3 | 0.0 | 0.417 | buffer | 0.161 | 0.651 | 1.44 |
| L5 | 1147 | 0.109 | 0.357 | 3.3 | 5.4 | 0.357 | L1 | 0.137 | 0.872 | 0.80 |

Entries written per update over the window: 1.01 by the event log; 0.85 by the byte tickers over the on-disk entry; model nominal 1.79, model with the engine's levels and memtable 2.49.

Compactions into the engine's last level (L6) inside the window: 0.

### geth256b8hr2

buffer 256 MB, L0 trigger 2, level base 64 MB, key Hash, ballast 8, prefix; window 173310015 updates (by unix micros); levels at the end: L3 40 MB, L4 97 MB, L5 1266 MB, L6 15614 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.221 | buffer | 0.183 | 0.477 | - |
| L3 | 40 | 0.225 | 0.103 | 0.5 | 0.0 | 0.262 | buffer | 0.183 | 0.477 | 1.23 |
| L4 | 97 | 0.129 | 0.154 | 1.2 | 2.4 | 0.249 | L1 | 0.252 | 0.864 | 0.51 |
| L5 | 1266 | 0.128 | 0.677 | 5.3 | 13.1 | 0.677 | L2 | 0.185 | 2.594 | 0.69 |

Entries written per update over the window: 1.41 by the event log; 1.19 by the byte tickers over the on-disk entry; model nominal 1.79, model with the engine's levels and memtable 4.94.

Compactions into the engine's last level (L6) inside the window: 0.

### geth64b8r2

buffer 64 MB, L0 trigger 2, level base 16 MB, key Path, ballast 8, prefix; window 173310015 updates (by unix micros); levels at the end: L0 10 MB, L4 105 MB, L5 1147 MB, L6 13685 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.337 | buffer | 0.276 | 1.663 | - |
| L3 | 0 | 0.338 | 0.006 | 0.0 | 0.0 | 0.282 | buffer | 0.276 | 1.663 | 1.22 |
| L4 | 105 | 0.276 | 1.265 | 4.6 | 0.0 | 1.401 | L1 | 0.172 | 2.052 | 1.60 |
| L5 | 1147 | 0.139 | 0.921 | 6.6 | 10.9 | 0.921 | L2 | 0.075 | 0.966 | 1.86 |

Entries written per update over the window: 2.94 by the event log; 2.53 by the byte tickers over the on-disk entry; model nominal 3.32, model with the engine's levels and memtable 4.68.

Compactions into the engine's last level (L6) inside the window: 0.

### geth64b1w

buffer 64 MB, L0 trigger 2, level base 16 MB, key Path, ballast 1, prefix; window 173310015 updates (by unix micros); levels at the end: L0 12 MB, L5 308 MB, L6 3234 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.337 | buffer | 0.276 | 3.671 | - |
| L3 | 0 | 0.335 | 0.000 | 0.0 | 0.0 | 0.276 | buffer | 0.276 | 3.671 | 1.21 |
| L4 | 0 | 0.140 | 0.139 | 1.0 | 0.0 | 0.231 | L1 | 0.120 | 1.385 | 1.17 |
| L5 | 308 | 0.231 | 1.719 | 7.4 | 0.0 | 1.815 | - | - | - | - |
| L6 | 3234 | 0.094 | 0.309 | 3.3 | 10.5 | 0.323 | - | - | - | - |

Entries written per update over the window: 2.98 by the event log; 2.57 by the byte tickers over the on-disk entry; model nominal 2.80, model with the engine's levels and memtable 5.06.

Compactions into the engine's last level (L6) inside the window: 18.
First touches inside the window: 0.057 per update (per update, by 5000 blocks: 5k 0.143, 10k 0.092, 15k 0.071, 20k 0.077, 25k 0.061, 30k 0.058, 35k 0.056, 40k 0.063, 45k 0.048).

### geth64b2w

buffer 64 MB, L0 trigger 2, level base 16 MB, key Path, ballast 2, prefix; window 173310015 updates (by unix micros); levels at the end: L0 12 MB, L5 440 MB, L6 4737 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.337 | buffer | 0.276 | 4.993 | - |
| L3 | 0 | 0.335 | 0.000 | 0.0 | 0.0 | 0.276 | buffer | 0.276 | 4.993 | 1.21 |
| L4 | 0 | 0.189 | 0.260 | 1.4 | 0.0 | 0.375 | L1 | 0.107 | 1.257 | 1.77 |
| L5 | 440 | 0.209 | 1.704 | 8.2 | 0.0 | 1.787 | - | - | - | - |
| L6 | 4737 | 0.079 | 0.173 | 2.2 | 10.8 | 0.188 | - | - | - | - |

Entries written per update over the window: 2.96 by the event log; 2.57 by the byte tickers over the on-disk entry; model nominal 3.18, model with the engine's levels and memtable 6.25.

Compactions into the engine's last level (L6) inside the window: 16.
First touches inside the window: 0.057 per update (per update, by 5000 blocks: 5k 0.143, 10k 0.092, 15k 0.071, 20k 0.077, 25k 0.061, 30k 0.058, 35k 0.056, 40k 0.063, 45k 0.048).

### geth64b4w

buffer 64 MB, L0 trigger 2, level base 16 MB, key Path, ballast 4, prefix; window 87906152 updates (by unix micros); levels at the end: L0 12 MB, L4 63 MB, L5 768 MB, L6 7678 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.342 | buffer | 0.276 | 1.235 | - |
| L3 | 0 | 0.339 | 0.013 | 0.0 | 0.0 | 0.287 | buffer | 0.276 | 1.235 | 1.23 |
| L4 | 63 | 0.270 | 0.577 | 2.1 | 0.0 | 0.740 | L1 | 0.205 | 2.707 | 1.32 |
| L5 | 768 | 0.175 | 1.299 | 7.4 | 12.2 | 1.375 | L2 | 0.087 | 0.954 | 2.01 |
| L6 | 7678 | 0.072 | 0.105 | 1.5 | 10.0 | 0.118 | - | - | - | - |

Entries written per update over the window: 2.86 by the event log; 2.50 by the byte tickers over the on-disk entry; model nominal 3.60, model with the engine's levels and memtable 4.90.

Compactions into the engine's last level (L6) inside the window: 8.
First touches inside the window: 0.056 per update (per update, by 5000 blocks: 5k 0.143, 10k 0.092, 15k 0.071, 20k 0.077, 25k 0.061, 30k 0.058, 35k 0.056, 40k 0.063, 45k 0.048).

### geth64b2i

buffer 64 MB, L0 trigger 2, level base 16 MB, key Path, ballast 2, beside; window 173310015 updates (by unix micros); levels at the end: L0 12 MB, L4 39 MB, L5 423 MB, L6 4436 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.337 | buffer | 0.276 | 1.021 | - |
| L3 | 0 | 0.335 | 0.000 | 0.0 | 0.0 | 0.276 | buffer | 0.276 | 1.021 | 1.21 |
| L4 | 39 | 0.186 | 0.244 | 1.3 | 0.0 | 0.359 | L1 | 0.238 | 2.824 | 0.78 |
| L5 | 423 | 0.209 | 1.603 | 7.7 | 10.8 | 1.688 | L2 | 0.106 | 1.221 | 1.97 |
| L6 | 4436 | 0.081 | 0.542 | 6.7 | 10.5 | 0.555 | - | - | - | - |

Entries written per update over the window: 3.22 by the event log; 2.92 by the byte tickers over the on-disk entry; model nominal 3.18, model with the engine's levels and memtable 5.07.

Compactions into the engine's last level (L6) inside the window: 15.
First touches inside the window: 0.057 per update (per update, by 5000 blocks: 5k 0.143, 10k 0.092, 15k 0.071, 20k 0.077, 25k 0.061, 30k 0.058, 35k 0.056, 40k 0.063, 45k 0.048).

### geth64b4i

buffer 64 MB, L0 trigger 2, level base 16 MB, key Path, ballast 4, beside; window 87906152 updates (by unix micros); levels at the end: L0 12 MB, L5 631 MB, L6 6544 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.342 | buffer | 0.276 | 8.020 | - |
| L3 | 0 | 0.339 | 0.008 | 0.0 | 0.0 | 0.284 | buffer | 0.276 | 8.020 | 1.23 |
| L4 | 0 | 0.265 | 0.466 | 1.8 | 0.0 | 0.632 | L1 | 0.088 | 0.997 | 3.01 |
| L5 | 631 | 0.192 | 1.520 | 7.9 | 0.0 | 1.599 | - | - | - | - |
| L6 | 6544 | 0.077 | 0.594 | 7.7 | 10.4 | 0.611 | - | - | - | - |

Entries written per update over the window: 3.47 by the event log; 3.49 by the byte tickers over the on-disk entry; model nominal 3.60, model with the engine's levels and memtable 9.02.

Compactions into the engine's last level (L6) inside the window: 7.
First touches inside the window: 0.056 per update (per update, by 5000 blocks: 5k 0.143, 10k 0.092, 15k 0.071, 20k 0.077, 25k 0.061, 30k 0.058, 35k 0.056, 40k 0.063, 45k 0.048).

### geth64b2i5

buffer 64 MB, L0 trigger 2, level base 2 MB, key Path, ballast 2, beside; window 173310015 updates (by unix micros); levels at the end: L5 378 MB, L6 4439 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.337 | buffer | 0.276 | 4.666 | - |
| L3 | 0 | 0.087 | 0.104 | 1.2 | 0.0 | 0.159 | buffer | 0.276 | 4.666 | 0.32 |
| L4 | 0 | 0.188 | 0.462 | 2.5 | 0.0 | 0.557 | L1 | 0.110 | 1.398 | 1.71 |
| L5 | 378 | 0.215 | 1.771 | 8.3 | 0.0 | 1.857 | - | - | - | - |
| L6 | 4439 | 0.088 | 0.605 | 6.8 | 11.7 | 0.619 | - | - | - | - |

Entries written per update over the window: 3.53 by the event log; 3.28 by the byte tickers over the on-disk entry; model nominal 3.18, model with the engine's levels and memtable 6.06.

Compactions into the engine's last level (L6) inside the window: 15.
First touches inside the window: 0.057 per update (per update, by 5000 blocks: 5k 0.143, 10k 0.092, 15k 0.071, 20k 0.077, 25k 0.061, 30k 0.058, 35k 0.056, 40k 0.063, 45k 0.048).

### geth64b4i5

buffer 64 MB, L0 trigger 2, level base 2 MB, key Path, ballast 4, beside; window 87906152 updates (by unix micros); levels at the end: L4 54 MB, L5 648 MB, L6 6533 MB

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.342 | buffer | 0.276 | 1.240 | - |
| L3 | 0 | 0.101 | 0.152 | 1.5 | 0.0 | 0.211 | buffer | 0.276 | 1.240 | 0.37 |
| L4 | 54 | 0.272 | 0.753 | 2.8 | 0.0 | 0.911 | L1 | 0.205 | 2.659 | 1.33 |
| L5 | 648 | 0.190 | 1.631 | 8.6 | 12.0 | 1.706 | L2 | 0.087 | 0.964 | 2.18 |
| L6 | 6533 | 0.062 | 0.458 | 7.4 | 10.1 | 0.469 | - | - | - | - |

Entries written per update over the window: 3.64 by the event log; 3.60 by the byte tickers over the on-disk entry; model nominal 3.60, model with the engine's levels and memtable 4.86.

Compactions into the engine's last level (L6) inside the window: 6.
First touches inside the window: 0.056 per update (per update, by 5000 blocks: 5k 0.143, 10k 0.092, 15k 0.071, 20k 0.077, 25k 0.061, 30k 0.058, 35k 0.056, 40k 0.063, 45k 0.048).

### idx64b0

buffer 64 MB, L0 trigger 2, level base 16 MB, key Hash, ballast 0, prefix; window 210234080 updates (by unix micros); levels at the end: L5 14 MB, L6 695 MB; index mode 16 B hashed keys, 8 B values, 1.00 relocations per update

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.655 | buffer | 0.614 | 1.341 | - |
| L4 | 0 | 0.115 | 0.097 | 0.8 | 0.0 | 0.202 | buffer | 0.614 | 1.341 | 0.19 |
| L5 | 14 | 0.514 | 0.847 | 1.6 | 0.0 | 1.292 | L1 | 0.698 | 35.339 | 0.74 |
| L6 | 695 | 0.585 | 5.059 | 8.6 | 49.6 | 5.066 | - | - | - | - |

Entries written per update over the window: 7.21 by the event log; 7.47 by the byte tickers over the on-disk entry; model nominal 8.85, model with the engine's levels and memtable 36.68.

Compactions into the engine's last level (L6) inside the window: 94.

### idx64b8

buffer 64 MB, L0 trigger 2, level base 16 MB, key Hash, ballast 8, prefix; window 210234080 updates (by unix micros); levels at the end: L5 522 MB, L6 5831 MB; index mode 16 B hashed keys, 8 B values, 1.00 relocations per update

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.665 | buffer | 0.614 | 4.405 | - |
| L3 | 0 | 0.046 | 0.046 | 1.0 | 0.0 | 0.088 | buffer | 0.614 | 4.405 | 0.07 |
| L4 | 0 | 0.456 | 0.651 | 1.4 | 0.0 | 1.066 | L1 | 0.574 | 6.992 | 0.79 |
| L5 | 522 | 0.625 | 4.307 | 6.9 | 0.0 | 4.831 | - | - | - | - |
| L6 | 5831 | 0.539 | 3.767 | 7.0 | 11.2 | 3.793 | - | - | - | - |

Entries written per update over the window: 10.44 by the event log; 10.61 by the byte tickers over the on-disk entry; model nominal 15.07, model with the engine's levels and memtable 11.40.

Compactions into the engine's last level (L6) inside the window: 63.

### idx64b8g23

buffer 64 MB, L0 trigger 2, level base 16 MB, key Hash, ballast 8, prefix; window 157675560 updates (by unix micros); levels at the end: L5 563 MB, L6 5764 MB; index mode 16 B hashed keys, 8 B values, 0.50 relocations per update

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.539 | buffer | 0.485 | 5.554 | - |
| L4 | 0 | 0.395 | 0.527 | 1.3 | 0.0 | 0.867 | buffer | 0.485 | 5.554 | 0.81 |
| L5 | 563 | 0.495 | 4.214 | 8.5 | 0.0 | 4.599 | L1 | 0.417 | 4.689 | 1.19 |
| L6 | 5764 | 0.384 | 2.764 | 7.2 | 10.2 | 2.785 | - | - | - | - |

Entries written per update over the window: 8.79 by the event log; 9.02 by the byte tickers over the on-disk entry; model nominal 11.29, model with the engine's levels and memtable 10.24.

Compactions into the engine's last level (L6) inside the window: 33.

### idx64b8i

buffer 64 MB, L0 trigger 2, level base 16 MB, key Hash, ballast 8, beside; window 210234080 updates (by unix micros); levels at the end: L5 241 MB, L6 2986 MB; index mode 16 B hashed keys, 8 B values, 1.00 relocations per update

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.665 | buffer | 0.614 | 4.117 | - |
| L4 | 0 | 0.493 | 0.740 | 1.5 | 0.0 | 1.187 | buffer | 0.614 | 4.117 | 0.80 |
| L5 | 241 | 0.626 | 2.159 | 3.4 | 0.0 | 2.717 | L1 | 0.576 | 7.716 | 1.09 |
| L6 | 2986 | 0.575 | 7.891 | 13.7 | 12.4 | 7.918 | - | - | - | - |

Entries written per update over the window: 12.49 by the event log; 16.33 by the byte tickers over the on-disk entry; model nominal 15.07, model with the engine's levels and memtable 11.83.

Compactions into the engine's last level (L6) inside the window: 66.
First touches inside the window: 0.033 per update (per update, by 5000 blocks: 5k 0.071, 10k 0.046, 15k 0.036, 20k 0.039, 25k 0.030, 30k 0.029).

### idx64b8g23i

buffer 64 MB, L0 trigger 2, level base 16 MB, key Hash, ballast 8, beside; window 157675560 updates (by unix micros); levels at the end: L5 270 MB, L6 2804 MB; index mode 16 B hashed keys, 8 B values, 0.50 relocations per update

| level | size MB | moved in per update | rewritten per update | rewritten per moved | size ratio to the level above | written per update | model level | model outflow | model written | engine moved / model outflow |
|---|---|---|---|---|---|---|---|---|---|---|
| L0 (flush) | - | - | - | - | - | 0.539 | buffer | 0.485 | 5.495 | - |
| L3 | 0 | 0.004 | 0.004 | 1.0 | 0.0 | 0.008 | buffer | 0.485 | 5.495 | 0.01 |
| L4 | 0 | 0.417 | 0.765 | 1.8 | 0.0 | 1.118 | L1 | 0.418 | 4.754 | 1.00 |
| L5 | 270 | 0.485 | 1.633 | 3.4 | 0.0 | 2.049 | - | - | - | - |
| L6 | 2804 | 0.424 | 6.151 | 14.5 | 10.4 | 6.175 | - | - | - | - |

Entries written per update over the window: 9.89 by the event log; 13.51 by the byte tickers over the on-disk entry; model nominal 11.29, model with the engine's levels and memtable 10.25.

Compactions into the engine's last level (L6) inside the window: 36.
First touches inside the window: 0.044 per update (per update, by 5000 blocks: 5k 0.095, 10k 0.062, 15k 0.048, 20k 0.051, 25k 0.041, 30k 0.039).
