"""Refresh the IGS antenna files in this folder.

Downloads, from https://files.igs.org/pub/station/general/:
- rcvr_ant.json: the IGS antenna, radome and receiver codes;
- antenna.gra: each antenna's reference point (ARP), north reference point
  and dimensions, with drawings;
- igs20.atx: the IGS antenna calibrations. It's 60 MB, so only the list
  of antenna and radome pairs it calibrates is kept, in calibrated.txt.

and records where each came from, when, and its SHA-256 in sources.toml.
The IGS asks users to cite it (see README.md).

    python update.py
"""

import datetime
import hashlib
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
BASE = "https://files.igs.org/pub/station/general/"


def fetch(name):
    with urllib.request.urlopen(BASE + name, timeout=600) as resp:
        return resp.read()


def calibrated(atx):
    """The antenna and radome pairs with type-mean receiver calibrations:
    TYPE / SERIAL NO records with a blank serial (satellites carry one)."""
    pairs = set()
    for line in atx.decode("latin-1").splitlines():
        if line[60:76] == "TYPE / SERIAL NO" and not line[20:40].strip():
            antenna, radome = line[:16].strip(), line[16:20].strip()
            if antenna:
                pairs.add(f"{antenna:<16}{radome}")
    return sorted(pairs)


def main():
    today = datetime.date.today().isoformat()
    sources = ["# Where the IGS files here came from. Written by update.py.\n"]
    for name in ["rcvr_ant.json", "antenna.gra"]:
        data = fetch(name)
        (HERE / name).write_bytes(data)
        sources.append(
            f'\n[[file]]\nname = "{name}"\nurl = "{BASE}{name}"\n'
            f'fetched = "{today}"\nsha256 = "{hashlib.sha256(data).hexdigest()}"\n'
        )
    atx = fetch("igs20.atx")
    pairs = calibrated(atx)
    (HERE / "calibrated.txt").write_text(
        "# Antenna and radome pairs with an IGS type-mean calibration in igs20.atx\n"
        "# (columns 1-16 antenna, 17-20 radome). Written by update.py.\n"
        + "".join(p + "\n" for p in pairs)
    )
    sources.append(
        f'\n[[file]]\nname = "calibrated.txt"\nfrom = "{BASE}igs20.atx"\n'
        f'fetched = "{today}"\nsha256_of_source = "{hashlib.sha256(atx).hexdigest()}"\n'
        f"pairs = {len(pairs)}\n"
    )
    (HERE / "sources.toml").write_text("".join(sources))
    print(f"rcvr_ant.json, antenna.gra and {len(pairs)} calibrated pairs, as of {today}")


if __name__ == "__main__":
    main()
