"""Compare builds of rnx2rtkp epoch by epoch on RTKLIB's own test data.

The GA baselines are processed as static sessions with one final solution,
which can hide small differences between builds. This runs RTKLIB-EX's
bundled test data (the files of its test4) with five option sets, from DGPS
and static to kinematic, writes every epoch, and compares each build with
the first one given: fix status, positions and the printed standard
deviations.

Usage:
  python libm_check.py --data <rtklib>/test/data/rinex \\
    --build wasmtime="wasmtime run --dir . rnx2rtkp.wasm" \\
    --build glibc=/path/to/rnx2rtkp-native \\
    --build glibc+libm-crate="env LD_PRELOAD=/abs/path/liblibm_shim.so /path/to/rnx2rtkp-native"

Commands run in a scratch directory holding the input files, so wasmtime
needs `--dir .` and native paths must be absolute.
"""

import argparse
import math
import shlex
import shutil
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
INPUTS = ["07590920.05o", "30400920.05n", "30400920.05o"]
COMMON = ["-t", "-e", "-r", "-3978241.958", "3382840.234", "3649900.853"]
CASES = {
    "dgps": ["-p", "1"],
    "static": ["-p", "3"],
    "kinematic": [],  # rnx2rtkp's default mode
    "kinematic-backward": ["-b"],
    "kinematic-combined": ["-c"],
}


def epochs(path):
    out = {}
    for line in path.read_text().splitlines():
        if line.startswith("%") or not line.strip():
            continue
        f = line.split()
        # Match epochs on the nominal second: the time tag includes the
        # receiver clock correction, which can itself differ between builds.
        h, m, s = f[1].split(":")
        key = (f[0], round(int(h) * 3600 + int(m) * 60 + float(s)))
        out[key] = {"line": line, "xyz": [float(v) for v in f[2:5]], "q": int(f[5])}
    return out


def compare(ref, other):
    common = sorted(set(ref) & set(other))
    return {
        "epochs": f"{len(ref)}/{len(other)}",
        "differ": sum(1 for t in common if ref[t]["line"] != other[t]["line"]) + len(set(ref) ^ set(other)),
        "q_differ": sum(1 for t in common if ref[t]["q"] != other[t]["q"]),
        "max_mm": max((1000 * math.dist(ref[t]["xyz"], other[t]["xyz"]) for t in common), default=0.0),
        "fixed": f"{sum(1 for e in ref.values() if e['q'] == 1)}/{sum(1 for e in other.values() if e['q'] == 1)}",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--data", required=True, type=Path, help="RTKLIB-EX's test/data/rinex directory")
    parser.add_argument("--build", action="append", required=True, metavar="NAME=COMMAND")
    args = parser.parse_args()
    builds = dict(b.split("=", 1) for b in args.build)

    work = HERE / "work" / "libm-check"
    work.mkdir(parents=True, exist_ok=True)
    for name in INPUTS:
        shutil.copy(args.data / name, work / name)

    results = {}
    for build, command in builds.items():
        for case, options in CASES.items():
            out = f"{build}_{case}.pos".replace("+", "_")
            cmd = shlex.split(command) + COMMON + options + INPUTS + ["-o", out]
            subprocess.run(cmd, cwd=work, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            results[build, case] = epochs(work / out)

    reference, *others = builds
    for build in others:
        print(f"\n{build} compared with {reference}\n")
        print("| Case | Epochs | Epochs that differ | Fix status differs | Largest position difference (mm) | Fixed epochs |")
        print("|---|---|---|---|---|---|")
        for case in CASES:
            c = compare(results[reference, case], results[build, case])
            print(f"| {case} | {c['epochs']} | {c['differ']} | {c['q_differ']} | {c['max_mm']:.1f} | {c['fixed']} |")


if __name__ == "__main__":
    main()
