"""Select CORS-pair baselines for the M0 engine spike.

Picks Australian public CORS pairs in five baseline-length bins, checks that
both stations have a daily RINEX 3 observation file on the chosen date in the
GA GNSS Data Repository, and writes the selection to manifest.json.

The selection is deterministic: pairs are ordered by a hash of their IDs, so
re-running with the same site list and date gives the same manifest.

Usage:
    python select_pairs.py [--date 2025-06-15] [--per-bin 4]
"""

import argparse
import hashlib
import json
import math
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORK = HERE / "work"
SITES_URL = "https://gws.geodesy.ga.gov.au/corsSites?size=5000"
API = "https://data.gnss.ga.gov.au/api/v1/rinexFiles"

# Baseline length bins in km, matching the short end of VALIDATION.md D1.
BINS = [(0.05, 1.0), (1.0, 5.0), (5.0, 10.0), (10.0, 20.0), (20.0, 50.0)]

# Rough bounds of mainland Australia and Tasmania.
LAT_RANGE = (-45.0, -9.0)
LON_RANGE = (112.0, 155.0)


def get_json(url):
    with urllib.request.urlopen(url, timeout=120) as resp:
        return json.load(resp)


def load_sites():
    cache = WORK / "corsSites.json"
    if not cache.exists():
        WORK.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(SITES_URL, timeout=300) as resp:
            cache.write_bytes(resp.read())
    data = json.loads(cache.read_text())
    return data["_embedded"]["corsSites"]


def ecef(lat_deg, lon_deg, h):
    """GRS80 geodetic to ECEF, metres. Good enough for choosing pairs."""
    a = 6378137.0
    f = 1 / 298.257222101
    e2 = f * (2 - f)
    lat, lon = math.radians(lat_deg), math.radians(lon_deg)
    n = a / math.sqrt(1 - e2 * math.sin(lat) ** 2)
    return (
        (n + h) * math.cos(lat) * math.cos(lon),
        (n + h) * math.cos(lat) * math.sin(lon),
        (n * (1 - e2) + h) * math.sin(lat),
    )


def has_daily_obs(station, date, cache):
    if station not in cache:
        url = (
            f"{API}?stationId={station}&startDate={date}&endDate={date}"
            "&fileType=obs&rinexVersion=3&filePeriod=01D"
        )
        try:
            cache[station] = len(get_json(url)) > 0
        except Exception:  # noqa: BLE001 - any failure means "not usable"
            cache[station] = False
    return cache[station]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--date", default="2025-06-15")
    parser.add_argument("--per-bin", type=int, default=4)
    args = parser.parse_args()

    sites = [
        s
        for s in load_sites()
        if s.get("siteStatus") == "PUBLIC" and s.get("approximatePosition")
    ]
    pos = {}
    for s in sites:
        lat, lon, h = s["approximatePosition"]["coordinates"]
        if LAT_RANGE[0] <= lat <= LAT_RANGE[1] and LON_RANGE[0] <= lon <= LON_RANGE[1]:
            pos[s["fourCharacterId"]] = ecef(lat, lon, h)
    ids = sorted(pos)
    print(f"{len(ids)} public Australian sites with positions")

    pairs = {b: [] for b in BINS}
    for i, a in enumerate(ids):
        for b in ids[i + 1 :]:
            km = math.dist(pos[a], pos[b]) / 1000
            for lo, hi in BINS:
                if lo <= km < hi:
                    pairs[(lo, hi)].append((a, b, km))

    available = {}
    used = set()
    selected = []
    for lo, hi in BINS:
        order = sorted(
            pairs[(lo, hi)],
            key=lambda p: hashlib.sha256(f"{p[0]}-{p[1]}".encode()).hexdigest(),
        )
        picked = 0
        for a, b, km in order:
            if picked == args.per_bin:
                break
            if a in used or b in used:
                continue
            if has_daily_obs(a, args.date, available) and has_daily_obs(b, args.date, available):
                selected.append(
                    {"base": a, "rover": b, "length_km": round(km, 3), "bin_km": [lo, hi]}
                )
                used.update((a, b))
                picked += 1
        print(f"bin {lo}-{hi} km: {picked} of {len(order)} candidate pairs selected")

    manifest = {
        "spike": "M0 engine spike (D-023)",
        "date": args.date,
        "source": "GA GNSS Data Repository, daily 30 s RINEX 3",
        "baselines": selected,
    }
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {len(selected)} baselines to manifest.json")


if __name__ == "__main__":
    main()
