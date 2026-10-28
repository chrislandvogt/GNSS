"""Broadcast satellite positions against precise orbits in an SP3 file:
the 3D difference by system, for healthy satellites, and by the
ephemeris's age.

    cargo run --release -p gnss-rinex-io --example satpos -- DATE NAV... > brdc.txt
    python compare_sp3.py ORBITS.SP3 brdc.txt

DATE is the SP3 file's day (YYYY-MM-DD). Precise orbits are for the
satellite's centre of mass and broadcast ones (except GLONASS's) for its
antenna, so they differ by up to a couple of metres even when both are right.
"""

import collections
import datetime
import math
import sys

GPS_EPOCH = datetime.datetime(1980, 1, 6)


def seconds_of_week(when):
    return (when - GPS_EPOCH).total_seconds() % 604800


def precise(path):
    out, t = {}, None
    for line in open(path):
        if line.startswith("*"):
            f = line.split()
            when = datetime.datetime(*(int(x) for x in f[1:6])) + datetime.timedelta(
                seconds=float(f[6]))
            t = round(seconds_of_week(when))
        elif line.startswith("P") and t is not None:
            xyz = [float(line[4 + 14 * i:18 + 14 * i]) for i in range(3)]
            # Missing positions are written as zeros.
            if abs(xyz[0]) > 1:
                out[(line[1:4].replace(" ", "0"), t)] = [v * 1000 for v in xyz]
    return out


def main(sp3_path, brdc_path):
    sp3 = precise(sp3_path)
    by_system = collections.defaultdict(list)
    by_age = collections.defaultdict(list)
    for line in open(brdc_path):
        f = line.split()
        if f[6] != "0":
            continue
        key = (f[0], round(float(f[1])))
        if key not in sp3:
            continue
        d = math.dist([float(v) for v in f[2:5]], sp3[key])
        by_system[f[0][0]].append(d)
        by_age[(f[0][0], int(abs(float(f[7])) // 1800))].append(d)
    print("system  n      RMS (m)  median  95%     max")
    for s, d in sorted(by_system.items()):
        d.sort()
        n = len(d)
        rms = math.sqrt(sum(x * x for x in d) / n)
        print(f"{s}       {n:<6} {rms:<8.2f} {d[n // 2]:<7.2f} {d[int(n * .95)]:<7.2f} {d[-1]:.2f}")
    print("\nsystem  age (h)    n      median  95%     max")
    for (s, age), d in sorted(by_age.items()):
        d.sort()
        n = len(d)
        print(f"{s}       {age / 2:.1f}-{(age + 1) / 2:.1f}    {n:<6} {d[n // 2]:<7.2f} "
              f"{d[int(n * .95)]:<7.2f} {d[-1]:.2f}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
