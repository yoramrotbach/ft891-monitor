# FT-891 CAT: what 891 Monitor sends

Sources: Yaesu *FT-891 CAT Operation Reference Book*, cross-checked against Hamlib's `newcat` / `ft891` driver (mode table, `IF` length, meter calibration). Model ID: `ID;` → `ID0650;`.

Commands are ASCII, terminated by `;`. Read commands get an answer; set commands get none; `?;` means rejected.

## Connection

On connect the app sends `ID;` and refuses to continue unless the answer is `ID0650;`, then `AI0;` (auto-information off) so every answer belongs to the command that asked for it.

## Polling

| Every cycle (~30/s) | Receive | Transmit |
|---|---|---|
| `TX;` | `TX0` receive, `TX1` CAT TX, `TX2` radio TX | |
| meter | `SM0;` → `SM0nnn` (0–255) and `BY;` (busy) | `RM<sel>;` for the selected meter, plus `RM5;` (PO) |
| `FA;` | VFO-A, 9 digits Hz | |

| ~4/s | |
|---|---|
| `IF;` | `IF` + channel(3) + freq(9) + ±clar(5) + clar on(1) + 0 + mode(1) + VFO/MEM(1) + CTCSS(1) + 00 + shift(1) |
| `OI;` | same layout, VFO-B |
| `MS;` | meter: 0 COMP, 1 ALC, 2 PO, 3 SWR, 4 ID → `RM` 3,4,5,6,7 |
| `ST;` `LK;` | split, lock |
| `RM6;` | SWR (transmit only) |

| ~1/s | |
|---|---|
| `VX;` `PA0;` `RA0;` `NA0;` `NB0;` `NR0;` `BC0;` `PR0;` `FS;` | VOX, preamp (0 = IPO), ATT, NAR, NB, DNR, DNF, PRC, FST |
| `ML0;` `AC;` `GT0;` `PC;` | monitor, tuner, AGC, power |

Mode codes: 1 LSB, 2 USB, 3 CW-U, 4 FM, 5 AM, 6 RTTY-L, 7 CW-L, 8 DATA-L, 9 RTTY-U, A DATA-FM, B FM-N, C DATA-U, D AM-N.

## Set commands allowed

| Command | Validation |
|---|---|
| `AI0;` | only 0 |
| `FA` / `FB` + 9 digits | 30 kHz – 56 MHz |
| `MD0x;` | x in 1–9, B, C, D (A is undefined in the manual) |
| `BU0;` `BD0;` | band up/down |
| `SV;` `AB;` `BA;` | swap, A→B, B→A |
| `ST0/1;` `LK0/1;` `FS0/1;` | split, lock, fast |
| `NB0x;` `NR0x;` `NA0x;` `RA0x;` `PA0x;` | x is 0 or 1 |
| `PC` + 3 digits | 005–100 |

Everything else is refused, including `TX`, `MX`, `AC`, `VX`, `KY`, `KM`, `EX`, `MW`, `MA`, `MC`, `CN`, `CT`, `DA`, `ZI`. See `is_allowed()` in `crates/ft891-cat/src/cat.rs` and its tests.

## Meter calibration

S-meter (raw → dB re S9): 0 → −54, 130 → 0 (S9), 172 → +20, 255 → +60 (Hamlib `FT891_STR_CAL`). Power (raw → W): 50 → 8, 100 → 26, 150 → 54, 200 → 92, 250 → 140 (Hamlib `FT891_RFPOWER_METER_CAL`). The SWR conversion is an approximation.
