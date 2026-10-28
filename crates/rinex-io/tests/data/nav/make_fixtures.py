"""Make the navigation-file test fixtures and their expected positions.

From real daily navigation files, keeps a few satellites of each system over
the first three hours of the day (with the records before it that are still
valid), and writes:

- `HOB200AUS_R_20251660000_excerpt_MN.rnx`: RINEX 3.04, every system;
- `HOB200AUS_R_20251660000_excerpt_MN4.rnx`: the same as RINEX 4.01, each
  record behind its `> EPH` line, with records our reader must skip (a GPS
  CNAV message, a system time offset and an ionosphere model) and the extra
  line RINEX 4 gives a GLONASS record;
- `mobs0050_excerpt.20n` and `mobs0050_excerpt.20g`: RINEX 2.11, GPS and
  GLONASS;
- `rtklib.txt`: RTKLIB's positions from each at 00:00:01 and every 15
  minutes to 02:45:01 (one second past the quarter hours, where RTKLIB and
  we take different Galileo ephemerides: it doesn't use one at its exact
  reference time);
- `ginan.txt`: Geoscience Australia's precise GPS orbits (Ginan rapid) for
  the same satellites at the quarter hours, for checking the broadcast
  orbits' accuracy.

    python make_fixtures.py RTKLIB_SATPOS HOB2_NAV MOBS_20N MOBS_20G SP3

`RTKLIB_SATPOS` is `rtklib_satpos.c` built as it says. The inputs are GA's
`HOB200AUS_R_20251660000_01D_MN.rnx`, `mobs0050.20n`, `mobs0050.20g` and
`GAG0EXPRAP_20251660000_01D_05M_ORB.SP3` (decompressed). Deterministic.
"""

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
HOB2_SATS = {"G04", "G05", "G07", "R05", "R15", "E07", "E30", "C01", "C06", "C24",
             "J02", "J07", "S28", "S37"}
MOBS_SATS = {" 1", " 3", "11", "22", " 6", " 7", "15"}
TIMES = [1 + 900 * k for k in range(12)]


def records(lines, starts):
    """The header, then each record's lines."""
    end = next(i for i, l in enumerate(lines) if l[60:73] == "END OF HEADER")
    out, current = [], None
    for line in lines[end + 1:]:
        if starts(line):
            current = [line]
            out.append(current)
        elif current is not None:
            current.append(line)
    return lines[: end + 1], out


def keep(record, sat, epoch, sats):
    """Within 22:00 the day before to 03:00, for a chosen satellite. SBAS
    records are frequent and short-lived: only those just before a test
    time."""
    if sat not in sats:
        return False
    if not ("2025 06 14 22" <= epoch <= "2025 06 15 03" or "20  1  4 22" <= epoch <= "20  1  5  3"):
        return False
    if sat.startswith("S"):
        h, m, s = int(epoch[11:13]), int(epoch[14:16]), int(epoch[17:19])
        t = h * 3600 + m * 60 + s
        return any(0 <= x - t <= 360 for x in TIMES) and epoch.startswith("2025 06 15")
    return True


def write(path, lines):
    path.write_text("".join(f"{l}\n" for l in lines), newline="\n")


def oracle(satpos, path, date):
    y, m, d = date
    out = subprocess.run(
        [satpos, str(path), str(y), str(m), str(d), "900", str(len(TIMES)), "1"],
        check=True, capture_output=True, text=True,
    ).stdout
    rows = []
    for line in out.splitlines():
        f = line.split()
        if f[0][0].isdigit():
            # RTKLIB names SBAS satellites by PRN; RINEX 3 by PRN - 100.
            f[0] = f"S{int(f[0]) - 100:02d}"
        rows.append(f"{path.name} {' '.join(f)}")
    return rows


def orbit_line(values):
    """Four spaces, then fields of 19 columns: `-0.123456789012E+00`."""
    return "    " + "".join(f"{v:19.12E}" for v in values)


def rinex4(header, recs):
    out = [f"{'4.01':>9}{'':11}{'N: GNSS NAV DATA':20}{'M':20}RINEX VERSION / TYPE"]
    out += [l for l in header[1:] if l[60:].strip() in ("PGM / RUN BY / DATE", "LEAP SECONDS",
                                                        "END OF HEADER")]
    for rec in recs:
        sat = rec[0][:3]
        system, prn = sat[0], int(sat[1:])
        if system == "E":
            source = int(float(rec[5][23:42].replace("D", "E")))
            kind = "FNAV" if source & (1 << 8) else "INAV"
        elif system == "C":
            kind = "D2" if prn <= 5 or prn >= 59 else "D1"
        else:
            kind = {"G": "LNAV", "J": "LNAV", "R": "FDMA", "S": "SBAS"}[system]
        out.append(f"> EPH {sat} {kind}")
        out += rec
        if system == "R":
            # RINEX 4's status flags, L1/L2 delay difference, URAI, health.
            out.append(orbit_line([0.0] * 4))
        if sat == "G05":
            # A CNAV message: a different layout, to be skipped.
            out.append("> EPH G05 CNAV")
            out += [rec[0]] + [orbit_line([1.0] * 4)] * 8
    out.append("> STO G01 LNAV")
    out.append("    2025 06 15 00 00 00 GPUT                          ")
    out.append(orbit_line([0.0, 2.79396772385e-09, 3.5527136788e-15, 0.0]))
    out.append("> ION G01 LNAV")
    out.append("    2025 06 15 00 00 00" + orbit_line([1.39698386192e-08, 2.23517417908e-08,
                                                     -1.19209289551e-07])[4:])
    out.append(orbit_line([-1.19209289551e-07, 110592.0, 163840.0, -65536.0]))
    out.append(orbit_line([-524288.0, 0.0]))
    return out


def main():
    satpos, hob2, mobs_n, mobs_g, sp3 = sys.argv[1:6]
    expected = []

    lines = Path(hob2).read_text().splitlines()
    header, recs = records(lines, lambda l: l[:1] not in (" ", ""))
    recs = [r for r in recs if keep(r, r[0][:3].replace(" ", "0"), r[0][4:23], HOB2_SATS)]
    for r in recs:
        r[0] = r[0][0] + r[0][1:3].replace(" ", "0") + r[0][3:]
    out3 = HERE / "HOB200AUS_R_20251660000_excerpt_MN.rnx"
    write(out3, header + [l for r in recs for l in r])
    write(HERE / "HOB200AUS_R_20251660000_excerpt_MN4.rnx", rinex4(header, recs))
    expected += oracle(satpos, out3, (2025, 6, 15))

    for path, name in [(mobs_n, "mobs0050_excerpt.20n"), (mobs_g, "mobs0050_excerpt.20g")]:
        lines = Path(path).read_text().splitlines()
        header, recs = records(lines, lambda l: l[1:2].isdigit())
        recs = [r for r in recs if keep(r, r[0][:2], r[0][3:14], MOBS_SATS)]
        out = HERE / name
        write(out, header + [l for r in recs for l in r])
        expected += oracle(satpos, out, (2020, 1, 5))
    write(HERE / "rtklib.txt", expected)

    gps = sorted(s for s in HOB2_SATS if s.startswith("G"))
    rows, t = [], None
    for line in Path(sp3).read_text().splitlines():
        if line.startswith("*"):
            f = line.split()
            t = int(f[4]) * 3600 + int(f[5]) * 60 + int(float(f[6]))
        elif line.startswith("P") and line[1:4] in gps and t is not None and t <= 3 * 3600 \
                and t % 900 == 0:
            f = line.split()
            # GPS seconds of week (the day is a Sunday) and metres.
            rows.append(f"{f[0][1:]} {t} " + " ".join(f"{float(x) * 1000:.3f}" for x in f[1:4]))
    write(HERE / "ginan.txt", rows)


if __name__ == "__main__":
    main()
