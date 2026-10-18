"""Summarise the spike: build parity, solution quality and timings.

Reads work/results/runs.jsonl (parity runs) and, if present,
work/results/timing.jsonl (the clean timing pass), and the solution files
under work/runs/. Prints Markdown tables and writes work/results/summary.json.
"""

import json
import math
import statistics
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
RESULTS = HERE / "work" / "results"
RUNS = HERE / "work" / "runs"
CASES = ["2h-combined", "2h-forward", "2h-backward", "24h-combined"]
REFERENCE = "wasmtime"


def signed_square(v):
    """RTKLIB writes off-diagonal covariances as sign(c)*sqrt(|c|)."""
    return math.copysign(v * v, v)


def read_solution(path):
    """The static solution line of an RTKLIB .pos file in XYZ format."""
    lines = [l for l in path.read_text().splitlines() if l and not l.startswith("%")]
    if not lines:
        return None
    f = lines[-1].split()
    sdx, sdy, sdz = (float(v) for v in f[7:10])
    sdxy, sdyz, sdzx = (signed_square(float(v)) for v in f[10:13])
    return {
        "xyz": [float(v) for v in f[2:5]],
        "q": int(f[5]),
        "ns": int(f[6]),
        "cov": [[sdx**2, sdxy, sdzx], [sdxy, sdy**2, sdyz], [sdzx, sdyz, sdz**2]],
        "ratio": float(f[14]),
    }


def load(path):
    return [json.loads(l) for l in path.read_text().splitlines()] if path.exists() else []


def parity(rows):
    by = defaultdict(dict)
    for r in rows:
        by[(r["pair"], r["case"])][r["build"]] = r
    builds = sorted({r["build"] for r in rows})
    table = {}
    for case in CASES:
        keys = [k for k in by if k[1] == case]
        table[case] = {"cases": len(keys)}
        for b in builds:
            if b == REFERENCE:
                continue
            both = [k for k in keys if b in by[k] and REFERENCE in by[k]]
            same = sum(1 for k in both if by[k][b]["sha256"] == by[k][REFERENCE]["sha256"])
            table[case][b] = f"{same}/{len(both)}"
    failures = [(r["pair"], r["case"], r["build"], r["exit"]) for r in rows if r["exit"] != 0]
    return builds, table, failures


def quality(manifest):
    rows = []
    for b in manifest["baselines"]:
        pair = f"{b['base']}-{b['rover']}"
        d = RUNS / pair / REFERENCE
        sol = {c: read_solution(d / f"{c}.pos") for c in CASES if (d / f"{c}.pos").exists()}
        if "2h-combined" not in sol or sol["2h-combined"] is None:
            continue
        c2 = sol["2h-combined"]
        row = {
            "pair": pair,
            "length_km": b["length_km"],
            "q_2h": c2["q"],
            "ns_2h": c2["ns"],
            "ratio_2h": c2["ratio"],
            "sigma3d_mm_2h": 1000 * math.sqrt(sum(c2["cov"][i][i] for i in range(3))),
        }
        fw, bw = sol.get("2h-forward"), sol.get("2h-backward")
        if fw and bw:
            row["fwd_bwd_mm"] = 1000 * math.dist(fw["xyz"], bw["xyz"])
            row["q_fwd_bwd"] = f"{fw['q']}/{bw['q']}"
        if sol.get("24h-combined"):
            c24 = sol["24h-combined"]
            row["q_24h"] = c24["q"]
            row["ratio_24h"] = c24["ratio"]
            row["diff_2h_24h_mm"] = 1000 * math.dist(c2["xyz"], c24["xyz"])
        rows.append(row)
    return rows


def timing(rows):
    groups = defaultdict(list)
    for r in rows:
        if r["exit"] == 0:
            groups[(r["case"], r["build"])].append(r)
    out = {}
    for (case, build), rs in sorted(groups.items()):
        mem = [r.get("peak_rss_mb") or r.get("module_memory_mb") for r in rs]
        out[f"{case}|{build}"] = {
            "runs": len(rs),
            "median_s": statistics.median(r["wall_s"] for r in rs),
            "max_mem_mb": max(m for m in mem if m is not None),
        }
    return out


def main():
    manifest = json.loads((HERE / "manifest.json").read_text())
    builds, par, failures = parity(load(RESULTS / "runs.jsonl"))
    qual = quality(manifest)
    tim = timing(load(RESULTS / "timing.jsonl"))

    others = [b for b in builds if b != REFERENCE]
    print(f"## Parity: solution files byte-identical to {REFERENCE}\n")
    print("| Case | Baselines | " + " | ".join(others) + " |")
    print("|---|---|" + "---|" * len(others))
    for case in CASES:
        print(f"| {case} | {par[case]['cases']} | " + " | ".join(par[case].get(b, "-") for b in others) + " |")
    print(f"\nRuns that exited with an error: {failures or 'none'}\n")

    print("## Solutions (2 h sessions unless noted)\n")
    print("| Baseline | km | Q | Sats | Ratio | 3D σ (mm) | Fwd/bwd Q | Fwd−bwd (mm) | 24 h Q | 2 h − 24 h (mm) |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for r in qual:
        print(
            f"| {r['pair']} | {r['length_km']:.2f} | {r['q_2h']} | {r['ns_2h']} | {r['ratio_2h']:.1f} | "
            f"{r['sigma3d_mm_2h']:.1f} | {r.get('q_fwd_bwd', '-')} | {r.get('fwd_bwd_mm', float('nan')):.1f} | "
            f"{r.get('q_24h', '-')} | {r.get('diff_2h_24h_mm', float('nan')):.1f} |"
        )

    if tim:
        print("\n## Timing (clean pass, medians)\n")
        print("| Case | Build | Runs | Median (s) | Peak memory (MB) |")
        print("|---|---|---|---|---|")
        for key, t in tim.items():
            case, build = key.split("|")
            print(f"| {case} | {build} | {t['runs']} | {t['median_s']:.2f} | {t['max_mem_mb']:.0f} |")

    (RESULTS / "summary.json").write_text(
        json.dumps({"parity": par, "failures": failures, "solutions": qual, "timing": tim}, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
