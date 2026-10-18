"""Trim the spike's RINEX 3 observation files to the 2-hour session window.

The engine driver trims observations to the processing window before running
the engine (ARCHITECTURE.md 9.3), because RTKLIB parses a whole file even
when told to process part of it. This script does the same for the spike, so
timings reflect what the product will do.

For each pair in work/runs/, writes trim/rover.obs and trim/base.obs holding
the header and the epochs in [02:00, 04:00] GPST (inclusive, matching
RTKLIB's -ts/-te), and links the other inputs alongside. The file names are
the same as the untrimmed run's, so the solution files can be compared byte
for byte.
"""

import json
import os
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUNS = HERE / "work" / "runs"
START = "02:00:00"
END = "04:00:00"


def epoch_time(line):
    """Parse a RINEX 3 epoch line ('> 2025 06 15 02 00  0.0000000  0 33')."""
    year, month, day, hour, minute = (int(v) for v in line[1:].split()[:5])
    second = float(line[1:].split()[5])
    return datetime(year, month, day, hour, minute), second


def trim(src, dst, start, end):
    keep = False
    with open(src, encoding="ascii", errors="replace") as fin, open(dst, "w", encoding="ascii") as fout:
        for line in fin:
            fout.write(line)
            if "END OF HEADER" in line[60:]:
                break
        for line in fin:
            if line.startswith(">"):
                when, second = epoch_time(line)
                stamp = when.replace(second=int(second))
                keep = start <= stamp <= end
            if keep:
                fout.write(line)


def link(src, dst):
    if not dst.exists():
        os.link(src, dst)


def main():
    manifest = json.loads((HERE / "manifest.json").read_text())
    day = datetime.strptime(manifest["date"], "%Y-%m-%d")
    start = datetime.combine(day, datetime.strptime(START, "%H:%M:%S").time())
    end = datetime.combine(day, datetime.strptime(END, "%H:%M:%S").time())
    for rundir in sorted(p for p in RUNS.iterdir() if p.is_dir()):
        out = rundir / "trim"
        out.mkdir(exist_ok=True)
        for name in ("rover.obs", "base.obs"):
            if not (out / name).exists():
                trim(rundir / name, out / name, start, end)
        for name in ("nav.rnx", "igs14.atx", "2h-combined.conf"):
            link(rundir / name, out / name)
        sizes = {n: (out / n).stat().st_size / 1e6 for n in ("rover.obs", "base.obs")}
        print(f"{rundir.name:10} rover {sizes['rover.obs']:5.1f} MB  base {sizes['base.obs']:5.1f} MB")


if __name__ == "__main__":
    main()
