"""Make the RINEX test fixtures, and their Hatanaka-compressed copies.

Two kinds:
- synthetic files exercising what real files rarely show at once: events
  (with header records), satellites leaving and returning, blank fields,
  loss-of-lock flags, a cycle slip that restarts a difference arc, clock
  offsets starting and stopping, values under 1 and negative values, and
  (RINEX 2) more than 12 satellites in an epoch;
- excerpts of real files: the first epochs of a daily file.

Each is compressed with the reference RNX2CRX and restored with CRX2RNX.
The restored file (`*.crx2rnx.*`) is what our decompressor must reproduce
byte for byte. The tools come with the `hatanaka` Python package (2.8.0):

    python make_fixtures.py <dir with rnx2crx and crx2rnx> [SWTC.rnx MOBS.20o]

Deterministic: running it again gives the same files.
"""

import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
# A RINEX 2 epoch line: yy mm dd hh mm ss.sssssss, two spaces, a flag.
EPOCH2 = re.compile(r"^ [ \d]\d [ \d]\d [ \d]\d [ \d]\d [ \d]\d [ \d]\d\.\d{7}  [0-6]")


def obs(value, lli=" ", ssi=" "):
    return " " * 16 if value is None else f"{value:14.3f}{lli}{ssi}"


def header3():
    lines = [
        ("     3.04           OBSERVATION DATA    M                   ", "RINEX VERSION / TYPE"),
        ("make_fixtures.py    GNSS project        20260101 000000 UTC ", "PGM / RUN BY / DATE"),
        ("SYNTH3                                                      ", "MARKER NAME"),
        ("1234                LEICA GR30          4.82                ", "REC # / TYPE / VERS"),
        ("5678                LEIAS10         NONE                    ", "ANT # / TYPE"),
        (f"{-4052052.0:14.4f}{4212836.0:14.4f}{-2545105.0:14.4f}{'':18}", "APPROX POSITION XYZ"),
        ("        1.5320        0.0000        0.0000                  ", "ANTENNA: DELTA H/E/N"),
        ("G    4 C1C L1C D1C S1C                                      ", "SYS / # / OBS TYPES"),
        ("R    2 C1C L1C                                              ", "SYS / # / OBS TYPES"),
        ("E    4 C1C L1C C5Q L5Q                                      ", "SYS / # / OBS TYPES"),
        ("    30.000                                                  ", "INTERVAL"),
        ("  2026     1     1     0     0    0.0000000     GPS         ", "TIME OF FIRST OBS"),
        ("    18                                                      ", "LEAP SECONDS"),
        ("                                                            ", "END OF HEADER"),
    ]
    return [f"{a}{b}" for a, b in lines]


def synthetic3():
    out = header3()
    types = {"G": 4, "R": 2, "E": 4}
    for k in range(16):
        t = k * 30
        if k == 6:
            out.append(f"> 2026 01 01 00 {t // 60:02d}{t % 60:11.7f}  3  2")
            out.append(f"{'TRIG2':<60}MARKER NAME")
            out.append(f"{'        1.6010        0.0000        0.0000':<60}ANTENNA: DELTA H/E/N")
        if k == 10:
            out.append(f">{'':30}5  1")
            out.append(f"{'external event without a time':<60}COMMENT")
        sats = ["G01", "G02", "R03", "E04"]
        if 7 <= k <= 9:
            sats.remove("G02")
        if k >= 7:
            sats.append("G05")
        clock = f"{-0.000123456 + 1e-9 * k * k:15.12f}" if k < 5 or k > 11 else ""
        out.append(f"> 2026 01 01 00 {t // 60:02d}{t % 60:11.7f}  0{len(sats):3d}      {clock}".rstrip())
        for s in sats:
            n = int(s[1:])
            fields = []
            for j in range(types[s[0]]):
                base = 20_000_000 + 1_000_000 * n + 7_000 * j
                value = base + 1.234 * k + 0.0101 * k * k
                lli, ssi = " ", str(1 + (n + j) % 9)
                if s == "G02" and j == 1 and k == 3:
                    lli = "1"
                if s == "G01" and j == 1 and k >= 12:
                    value += 1_000_000.0  # a cycle slip: restarts the arc
                if s == "E04" and j == 2 and k in (4, 5):
                    value = None
                if s == "G01" and j == 3:
                    value = 0.5 if k % 2 else -0.25
                if s == "R03" and j == 0:
                    value = -value
                fields.append(obs(value, lli, ssi))
            out.append((s + "".join(fields)).rstrip())
    return out


def header2():
    lines = [
        ("     2.11           OBSERVATION DATA    M (MIXED)           ", "RINEX VERSION / TYPE"),
        ("SYNTH2                                                      ", "MARKER NAME"),
        ("                    TRM59800.00     SCIS                    ", "ANT # / TYPE"),
        ("     6    L1    L2    C1    P2    S1    S2                  ", "# / TYPES OF OBSERV"),
        ("  2026     1     1     0     0    0.0000000     GPS         ", "TIME OF FIRST OBS"),
        ("                                                            ", "END OF HEADER"),
    ]
    return [f"{a}{b}" for a, b in lines]


def synthetic2():
    out = header2()
    for k in range(8):
        t = k * 30
        if k == 4:
            out.append(f" 26  1  1  0 {t // 60:2d}{t % 60:11.7f}  2  1")
            out.append(f"{'antenna starts moving':<60}COMMENT")
        sats = [f"G{n:02d}" for n in range(1, 13)] + ["R01", "R02"]
        if k in (2, 3):
            sats.remove("G07")
        clock = f"{0.000012345 * (k + 1):12.9f}" if k != 5 else ""
        first = "".join(sats[:12])
        out.append(f" 26  1  1  0 {t // 60:2d}{t % 60:11.7f}  0{len(sats):3d}{first:<36}{clock}".rstrip())
        for i in range(12, len(sats), 12):
            out.append(" " * 32 + "".join(sats[i:i + 12]))
        for s in sats:
            n = int(s[1:]) + (30 if s[0] == "R" else 0)
            values = []
            for j in range(6):
                v = 21_000_000 + 100_000 * n + 3_000 * j + 2.5 * k
                if j >= 4:
                    v = 40 + n % 10 + 0.25 * k
                if s == "G03" and j == 3 and k == 6:
                    v = None
                values.append(obs(v, "1" if (s == "G04" and j == 0 and k == 5) else " ", " "))
            for i in range(0, 6, 5):
                out.append("".join(values[i:i + 5]).rstrip())
    return out


def excerpt(path, epochs, v2=False):
    out, body, seen = [], False, 0
    for line in Path(path).read_text(encoding="latin-1").splitlines():
        if body:
            is_epoch = line.startswith(">") if not v2 else bool(EPOCH2.match(line))
            if is_epoch:
                seen += 1
                if seen > epochs:
                    break
        out.append(line)
        body = body or line[60:73] == "END OF HEADER"
    return out


def write(name, lines, tools):
    rnx = HERE / name
    rnx.write_text("\n".join(lines) + "\n", encoding="latin-1")
    crx = rnx.with_name(rnx.name + ".crx")
    with open(crx, "wb") as f:
        subprocess.run([str(tools / "rnx2crx"), "-", ], stdin=open(rnx, "rb"), stdout=f, check=True)
    restored = rnx.with_name(rnx.name + ".crx2rnx")
    with open(restored, "wb") as f:
        subprocess.run([str(tools / "crx2rnx"), "-"], stdin=open(crx, "rb"), stdout=f, check=True)
    print(f"{name}: {rnx.stat().st_size} bytes, compressed {crx.stat().st_size}")


def main():
    tools = Path(sys.argv[1])
    write("synthetic3.rnx", synthetic3(), tools)
    write("synthetic2.11o", synthetic2(), tools)
    if len(sys.argv) > 3:
        write("SWTC00AUS_20251660000.rnx", excerpt(sys.argv[2], 6), tools)
        write("MOBS0050.20o", excerpt(sys.argv[3], 6, v2=True), tools)


if __name__ == "__main__":
    main()
