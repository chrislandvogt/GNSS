"""Fetch and decompress the RINEX files listed in manifest.json.

For each baseline, downloads the daily RINEX 3 observation files for both
stations and a mixed broadcast navigation file from the GA GNSS Data
Repository, then decompresses them (gzip, then Hatanaka) into work/rinex/.
Files already present are skipped.

Navigation files must be mixed-constellation (RINEX 3 "_MN" files). Some
stations publish only per-constellation files (Galileo-only, GLONASS-only and
so on), which leave RTKLIB without GPS ephemerides. Broadcast ephemerides are
the same whichever station logged them, so the base's mixed file is
preferred, then the rover's, then a fallback reference station's. The choice
is written to work/nav_sources.json.

Requires the `hatanaka` package (pip install hatanaka).
"""

import json
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import hatanaka

HERE = Path(__file__).resolve().parent
RINEX = HERE / "work" / "rinex"
API = "https://data.gnss.ga.gov.au/api/v1/rinexFiles"
NAV_FALLBACKS = ["TID1", "STR1", "SYDN", "MOBS", "PERT"]


def locate(station, date, file_type):
    """URL of the station's daily file, or None. For navigation files, only a
    mixed-constellation ("_MN") file is accepted."""
    url = (
        f"{API}?stationId={station}&startDate={date}&endDate={date}"
        f"&fileType={file_type}&rinexVersion=3&filePeriod=01D"
    )
    try:
        with urllib.request.urlopen(url, timeout=120) as resp:
            files = json.load(resp)
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return None
        raise
    if file_type == "nav":
        files = [f for f in files if "_MN." in f["fileLocation"].split("?")[0]]
    return files[0]["fileLocation"] if files else None


def fetch(station, date, file_type):
    """Download and decompress one file. Returns the decompressed path."""
    out = RINEX / f"{station}_{date}_{file_type}.rnx"
    if out.exists():
        return out
    url = locate(station, date, file_type)
    if url is None:
        return None
    with urllib.request.urlopen(url, timeout=600) as resp:
        raw = resp.read()
    # hatanaka.decompress handles gzip and Hatanaka-compressed RINEX alike.
    out.write_bytes(hatanaka.decompress(raw))
    return out


def main():
    manifest = json.loads((HERE / "manifest.json").read_text())
    date = manifest["date"]
    RINEX.mkdir(parents=True, exist_ok=True)

    stations = sorted({s for b in manifest["baselines"] for s in (b["base"], b["rover"])})
    with ThreadPoolExecutor(max_workers=6) as pool:
        for station, path in zip(stations, pool.map(lambda s: fetch(s, date, "obs"), stations)):
            if path is None:
                raise RuntimeError(f"no observation file for {station} on {date}")
            print(f"{station:5} obs: {path.name} ({path.stat().st_size / 1e6:.1f} MB)")

    nav_sources = {}
    for b in manifest["baselines"]:
        for candidate in [b["base"], b["rover"], *NAV_FALLBACKS]:
            path = fetch(candidate, date, "nav")
            if path is not None:
                nav_sources[f"{b['base']}-{b['rover']}"] = path.name
                break
        else:
            raise RuntimeError(f"no navigation file for {b['base']}-{b['rover']}")
    (HERE / "work" / "nav_sources.json").write_text(json.dumps(nav_sources, indent=2) + "\n")
    print(f"navigation files: {sorted(set(nav_sources.values()))}")


if __name__ == "__main__":
    main()
