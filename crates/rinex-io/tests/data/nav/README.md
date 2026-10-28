# Navigation test files

Made by `make_fixtures.py`, which also documents how; `rtklib_satpos.c` is
the RTKLIB program it runs for the expected positions.

| File | What it is | Source and licence |
|---|---|---|
| `HOB200AUS_R_20251660000_excerpt_MN.rnx` | RINEX 3.04, mixed: GPS, GLONASS, Galileo (I/NAV and F/NAV), BeiDou (GEO, IGSO and MEO), QZSS and SBAS satellites over the first three hours of 15 June 2025 at HOB2 (Hobart) | Geoscience Australia, GNSS Data Repository, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| `HOB200AUS_R_20251660000_excerpt_MN4.rnx` | The same as RINEX 4.01, with records the reader must skip (CNAV, STO, ION) | As above, rewritten by `make_fixtures.py` |
| `mobs0050_excerpt.20n`, `mobs0050_excerpt.20g` | RINEX 2.11, GPS and GLONASS, the first three hours of 5 January 2020 at MOBS (Melbourne) | As above |
| `rtklib.txt` | RTKLIB 2.5.1's positions and clocks from each file at 00:00:01 and every 15 minutes to 02:45:01 GPS time | Made by `make_fixtures.py` with RTKLIB (BSD-2-Clause); Apache-2.0 |
| `ginan.txt` | Precise GPS orbits at the quarter hours to 03:00 on 15 June 2025: GA's Ginan rapid solution `GAG0EXPRAP_20251660000_01D_05M_ORB.SP3` (experimental), in metres | Geoscience Australia, GNSS products, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |

Geoscience Australia's [copyright notice](https://www.ga.gov.au/copyright)
puts its material under CC BY 4.0. The repository's pages still show the
older CC BY 3.0 AU badge. Either way, attribution is what's asked.

Where we differ from RTKLIB, on purpose:

- At a Galileo ephemeris's exact reference time, we use it; RTKLIB uses
  the one before. The test times are a second past the quarter hours to
  stay clear of this.
- RTKLIB uses a GPS, QZSS, BeiDou or NavIC ephemeris for up to a second past
  its validity; we don't. The test allows for that second.
- Where the newest Galileo ephemeris is only in F/NAV, we use it; RTKLIB
  set to I/NAV doesn't. The excerpt has no such case.
- BeiDou-3's GEO satellites (C59 to C62) are beyond this RTKLIB build's
  satellites, so they aren't in the excerpt; C01 tests the GEO case.
