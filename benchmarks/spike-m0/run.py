"""Run the spike baselines through one or more rnx2rtkp builds.

Each build is given as NAME=COMMAND, for example:

    python run.py \
        --build native-gcc=/path/to/rnx2rtkp-gcc \
        --build wasmtime="wasmtime run --allow-precompiled --dir . /path/to/rnx2rtkp.cwasm"

For every baseline in manifest.json the runner makes a working directory
under work/runs/ holding hard links to the inputs under fixed names
(rover.obs, base.obs, nav.rnx, igs14.atx). Every build runs from that
directory with the same relative paths, so the solution files' headers are
identical and their hashes can be compared directly.

Cases per baseline:
    2h-combined, 2h-forward, 2h-backward   02:00-04:00 GPST, a typical session
    24h-combined                            the whole day, the memory stress case

Results are appended to work/results/runs.jsonl (or --log): wall time, peak
resident memory of the process, exit status and the SHA-256 of the solution
file. For timing, --repeat runs each case several times, and --trimmed uses
the session-trimmed inputs made by trim_rinex.py (work/runs/<pair>/trim/).
"""

import argparse
import hashlib
import json
import os
import shlex
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORK = HERE / "work"

CASES = {
    "2h-combined": ("combined", ("02:00:00", "04:00:00")),
    "2h-forward": ("forward", ("02:00:00", "04:00:00")),
    "2h-backward": ("backward", ("02:00:00", "04:00:00")),
    "24h-combined": ("combined", None),
}


def link(src, dst):
    if not dst.exists():
        os.link(src, dst)


def prepare(baseline, date, nav_sources, antex):
    pair = f"{baseline['base']}-{baseline['rover']}"
    rundir = WORK / "runs" / pair
    rundir.mkdir(parents=True, exist_ok=True)
    rinex = WORK / "rinex"
    link(rinex / f"{baseline['rover']}_{date}_obs.rnx", rundir / "rover.obs")
    link(rinex / f"{baseline['base']}_{date}_obs.rnx", rundir / "base.obs")
    link(rinex / nav_sources[pair], rundir / "nav.rnx")
    link(antex, rundir / "igs14.atx")
    template = (HERE / "spike.conf").read_text()
    for case, (soltype, _) in CASES.items():
        conf = template.replace("pos1-soltype       =combined", f"pos1-soltype       ={soltype}")
        (rundir / f"{case}.conf").write_text(conf)
    return pair, rundir


def run_one(cmd, rundir):
    """Run a command, returning (exit status, wall seconds, peak RSS in MB, stderr)."""
    start = time.perf_counter()
    proc = subprocess.Popen(cmd, cwd=rundir, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    stderr = proc.stderr.read()
    _, status, rusage = os.wait4(proc.pid, 0)
    wall = time.perf_counter() - start
    proc.returncode = os.waitstatus_to_exitcode(status)
    return proc.returncode, wall, rusage.ru_maxrss / 1024, stderr.decode(errors="replace")


def run_case(baseline, case, build, command, workdir, date):
    """Run one case with one build in workdir, returning its result record."""
    _, window = CASES[case]
    (workdir / build).mkdir(exist_ok=True)
    out = f"{build}/{case}.pos"
    cmd = shlex.split(command) + ["-k", f"{case}.conf"]
    if window:
        cmd += ["-ts", date, window[0], "-te", date, window[1]]
    cmd += ["-o", out, "rover.obs", "base.obs", "nav.rnx"]
    code, wall, rss, stderr = run_one(cmd, workdir)
    solution = workdir / out
    return {
        "length_km": baseline["length_km"],
        "case": case,
        "build": build,
        "exit": code,
        "wall_s": round(wall, 3),
        "peak_rss_mb": round(rss, 1),
        "sha256": hashlib.sha256(solution.read_bytes()).hexdigest() if solution.exists() else None,
        "stderr_tail": stderr[-300:],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--build", action="append", required=True, help="NAME=COMMAND")
    parser.add_argument("--antex", required=True, type=Path, help="ANTEX file to use")
    parser.add_argument("--only", help="comma-separated pair names to run")
    parser.add_argument("--cases", default=",".join(CASES), help="comma-separated cases")
    parser.add_argument("--log", default="runs.jsonl", help="results file in work/results/")
    parser.add_argument("--repeat", type=int, default=1, help="runs per case and build")
    parser.add_argument("--trimmed", action="store_true", help="use trim_rinex.py's inputs")
    args = parser.parse_args()

    builds = dict(spec.split("=", 1) for spec in args.build)
    manifest = json.loads((HERE / "manifest.json").read_text())
    nav_sources = json.loads((WORK / "nav_sources.json").read_text())
    date = manifest["date"].replace("-", "/")
    cases = args.cases.split(",")
    only = set(args.only.split(",")) if args.only else None

    results = WORK / "results"
    results.mkdir(parents=True, exist_ok=True)
    with (results / args.log).open("a") as log:
        for baseline in manifest["baselines"]:
            pair, rundir = prepare(baseline, manifest["date"], nav_sources, args.antex)
            if only and pair not in only:
                continue
            workdir = rundir / "trim" if args.trimmed else rundir
            for case in cases:
                for rep in range(args.repeat):
                    for name, command in builds.items():
                        record = run_case(baseline, case, name, command, workdir, date)
                        record.update(pair=pair, rep=rep)
                        if args.trimmed:
                            record["case"] += "-trimmed"
                        log.write(json.dumps(record) + "\n")
                        log.flush()
                        print(
                            f"{pair:10} {record['case']:13} {name:12} exit={record['exit']} "
                            f"{record['wall_s']:7.2f}s {record['peak_rss_mb']:7.1f} MB "
                            f"{record['sha256'] and record['sha256'][:12]}"
                        )


if __name__ == "__main__":
    main()
