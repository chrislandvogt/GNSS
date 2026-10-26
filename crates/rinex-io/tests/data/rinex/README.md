# RINEX test files

Made by `make_fixtures.py`, which also documents how. For each file `X`:

- `X` is the RINEX file;
- `X.crx` is it compressed by the reference RNX2CRX 4.1.0;
- `X.crx2rnx` is that restored by the reference CRX2RNX 4.1.0, which our
  decompressor must reproduce byte for byte. It differs from `X` only where
  CRX2RNX drops the zero before a decimal point (`-.250` for `-0.250`).

| File | What it is | Source and licence |
|---|---|---|
| `synthetic3.rnx` | RINEX 3.04, synthetic: events with header records, satellites leaving and returning, blank fields, loss-of-lock flags, a cycle slip, clock offsets starting and stopping | Made by `make_fixtures.py`; Apache-2.0, like the rest of the repository |
| `synthetic2.11o` | RINEX 2.11, synthetic: 14 satellites an epoch (continuation lines), an event, blank fields, a clock offset dropping out | As above |
| `SWTC00AUS_20251660000.rnx` | The first 6 epochs of SWTC (Swan Hill, Victoria) on 15 June 2025, RINEX 3.03 | Geoscience Australia, GNSS Data Repository, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| `MOBS0050.20o` | The first 6 epochs of MOBS (Melbourne) on 5 January 2020, RINEX 2.11 | Geoscience Australia, GNSS Data Repository, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
