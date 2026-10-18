# M0 engine spike: RTKLIB-EX as a WebAssembly module

**Status:** complete, 18 October 2026. **Outcome:** [D-023](../DECISIONS.md) is confirmed. The spike also led to [D-024](../DECISIONS.md) (one maths library on every platform).

The scripts, configuration and instructions for reproducing every number here are in [benchmarks/spike-m0](../../benchmarks/spike-m0/README.md).

## Summary

| Question | Answer |
|---|---|
| Can RTKLIB-EX be built as a WebAssembly module? | **Yes.** wasi-sdk builds `rnx2rtkp` from unmodified RTKLIB-EX sources, plus a stub for `system()`. The module is 1.17 MB (522 kB gzipped). The build is reproducible: it gives the same SHA-256 on different machines and paths, including GitHub's runners. |
| Does it give the same answers as native builds? | **Yes.** On 19 GA baselines, every solution file from the module, in wasmtime and in Chromium, is byte-identical to the native builds' output (76 of 76). RTKLIB's own test output is byte-identical in Chromium, Firefox and WebKit. |
| Is it fast enough? | **Yes.** In Chromium, a 2-hour static baseline, with its inputs trimmed to the session, takes 1.2 to 2.3 s: 1.1 to 1.5 times as long as native code. A whole day at 30 s takes 11 to 21 s, 1.2 to 1.4 times native. |
| How much memory does it need? | 90 MB of module memory for a 2-hour session, and 228 MB for a whole day at 30 s. That's comfortable on desktops. Phones will need a smaller worker pool. |

Four findings matter for what comes next:

1. **The maths library decides more than the last digit.** All the GA static solutions agreed. But on RTKLIB's own kinematic test data, a native build linked against glibc and the module, whose maths library derives from musl, make different ambiguity-fixing decisions. Their positions differ by up to 1.3 m. Replacing glibc's maths functions with the Rust `libm` crate makes the native build match the module exactly. So the CLI must run the module, not a native build, as D-023 already says. And the Rust core needs one maths library on every platform: that's D-024.
2. **Bit-identical isn't the same as robust.** The same experiment shows RTKLIB turning last-bit differences into different fixes, including one epoch "fixed" with a ratio of 6.9 that is 166 m wrong. A fix can't be trusted on its ratio alone, which is why the plan checks results independently (forward against backward runs, loop closures, repeat baselines).
3. **Navigation files must be mixed-constellation.** For many stations, GA's archive serves broadcast navigation data as separate per-constellation files. Using one of them silently degrades a solution or kills it. The planner has to handle this ([ARCHITECTURE.md](../ARCHITECTURE.md#13-data-fetch), section 13).
4. **RTKLIB's formal precisions are optimistic.** On baselines over 10 km, forward and backward solutions of the same 2 hours differ by up to 73 mm, while the formal 3D standard deviation is 1 to 3 mm. This was expected, and it's why the plan includes variance component estimation and the forward/backward check.

## The question

D-023 makes the web app the main front end, which rests on one assumption: that RTKLIB-EX can run as a WebAssembly module, in the browser and in the CLI, giving the same answers as a native build at an acceptable speed and memory cost. The spike tests this ([ROADMAP.md](../ROADMAP.md), M0). It aims to:

- build `rnx2rtkp` as a WASI module with wasi-sdk
- run it in a browser worker and in wasmtime, and confirm the outputs are identical
- compare it with native builds on about 20 CORS-pair baselines from GA: solutions, covariances, fix decisions and ratios
- measure its speed and peak memory against the native build
- record the results in D-023, and set the web performance targets

## Method

### Builds

Every build uses RTKLIB-EX v2.5.1 (commit `62d4677`), with the sources and defines of upstream's `rnx2rtkp` makefile (`-DTRACE -DENAGLO -DENAQZS -DENAGAL -DENACMP -DENAIRN -DNFREQ=4 -DNEXOBS=3`), at `-O3`.

| Build | Compiler | C and maths library | Used for |
|---|---|---|---|
| WebAssembly module (`wasm32-wasip1`) | wasi-sdk 34.0 (clang 23.1.0) | wasi-libc, whose maths library comes from musl | Run in wasmtime 49.0.3 (precompiled with `wasmtime compile`), and in Chromium 141 through the web app's engine workers |
| native-gcc | GCC 13.3.0 | glibc 2.39 | Parity and timing |
| native-musl | zig 0.17.0 (clang), statically linked | musl | Parity |
| native-clang | Ubuntu clang 18.1.3 | glibc 2.39 | Finding the cause of differences |

RTKLIB-EX needed one change for WASI. It calls `system()` to decompress compressed inputs, and WASI has no shell, so `wasi_compat.c` supplies a stub that always fails. The engine driver always passes uncompressed files. The module is linked with an 8 MiB stack placed first in memory, so a stack overflow traps instead of corrupting data. Details are in [engine-modules/rtklib-ex](../../engine-modules/rtklib-ex/README.md).

### Data

- **Baselines.** 19 baselines between Australian CORS, from GA's GNSS Data Repository, on 15 June 2025. They range from 0.07 to 49.8 km. `select_pairs.py` picked four per length bin (0.05 to 1, 1 to 5, 5 to 10, 10 to 20 and 20 to 50 km). Only three short pairs had data that day.
- **Observations.** Daily RINEX 3 files at 30 s.
- **Navigation.** One mixed-constellation broadcast navigation file per baseline.

### Processing

The configuration (`spike.conf`) is:

- static mode, L1 and L2, 15° elevation mask
- GPS, Galileo and BeiDou
- broadcast ionosphere and the Saastamoinen troposphere model
- continuous ambiguity resolution with a ratio threshold of 3
- one final solution per run

Antenna types come from the RINEX headers, with calibrations from `igs14.atx`. That file is bundled with RTKLIB-EX; the IGS file server wasn't reachable from the spike's environment. The base position is the approximate position in its RINEX header. So the spike tests consistency between builds, not absolute accuracy. Accuracy is the benchmark's job (M10).

Each baseline was run as four cases:

- a 2-hour session (02:00 to 04:00 GPST) as a combined solution
- the same session forward-only
- the same session backward-only
- the whole day as a combined solution

### Comparison

- **Parity.** Solution files are compared by SHA-256, with the wasmtime run as the reference.
- **Timing.** A separate pass ran on an otherwise idle cloud VM (4 vCPUs of a 2.1 GHz Intel Xeon, 16 GB of RAM, Linux). It used five baselines across the length range, ran each 2-hour case three times and took the median, and ran the whole-day case once.
  - Native and wasmtime times are whole-process wall times, and their memory is peak RSS.
  - Browser times cover the module's run from start to exit, not fetching the inputs. Browser memory is the size of the module's linear memory.

## Results

### Parity

Solution files byte-identical to the module's run in wasmtime:

| Case | Baselines | native-gcc | native-musl | Chromium |
|---|---|---|---|---|
| 2 h, combined | 19 | 19 | 19 | 19 |
| 2 h, forward | 19 | 19 | 19 | 19 |
| 2 h, backward | 19 | 19 | 19 | 19 |
| 24 h, combined | 19 | 19 | 19 | 19 |

- Every run exited cleanly.
- The timing pass repeated the comparison: every build gave the same solution file in every repeat.
- Trimming the inputs to the session (see below) gave the same solution files as whole-day inputs for all five baselines.
- **Other browser engines.** On every pull request, CI runs RTKLIB's test4 (a static session from its test data) through the web app's engine worker. The output was byte-identical to the golden file, which the module also reproduces under wasmtime, in:
  - Chromium 141.0.7390.37
  - Firefox 142.0.1
  - WebKit 26.0

### The maths library

The GA static results hide something that RTKLIB's own test data shows. `libm_check.py` runs the test4 files in five modes, writes every epoch, and compares each build with the module. Here is the GCC build against glibc:

| Mode | Epochs that differ | Fix status differs | Largest position difference | Epochs fixed (module, glibc) |
|---|---|---|---|---|
| DGPS (`-p 1`) | 2 of 120, standard deviations only | 0 | 0 | none |
| Static (`-p 3`) | 3 of 120, the ratio's last digit only | 0 | 0 | 117, 117 |
| Kinematic | 119 of 120, including one epoch glibc doesn't output | 86 | 1.3 m | 116, 31 |
| Kinematic, backward | 118 of 120 | 40 | 166 m | 86, 92 |
| Kinematic, combined | 120 of 120 | 67 | 1.2 m | 44, 83 |

Narrowing down the cause:

- The clang build against glibc gives exactly the same output as GCC's, so the compiler isn't the cause.
- The musl build matches the module exactly in every mode.
- Replacing glibc's `qsort` with musl's, to rule out differences in sort order, changes nothing.
- Replacing glibc's maths functions with the Rust `libm` crate's (a port of musl's maths library, loaded with `LD_PRELOAD` from `libm-shim/`) makes the glibc build's output byte-identical to the module's in every mode.

So the cause is the maths library. glibc and musl implement functions such as `sin`, `atan2` and `pow` differently, and their results can differ in the last bit. In static processing, the difference disappeared from every final solution. RTKLIB's kinematic filter amplifies it instead: a different rounding tips a ratio test one way or the other, and from then on the two runs take different paths. In the backward run, the module reports a "fixed" solution with a ratio of 6.9 and zero standard deviations, 166 m from where every other solution puts the rover at that epoch.

What follows from this:

- **The module is the reference.** It carries its own maths library, so it gives the same answer in every browser and in wasmtime on every operating system. That's the strongest reason for the CLI to run the module rather than a native build, which D-023 already requires.
- **A native build isn't a check on the module.** Differences are expected whenever the maths libraries differ. CI compares the module with itself across hosts.
- **The Rust core has the same exposure.** `f64::sin` and its relatives call glibc on Linux, the system library on macOS and Windows, and a musl port in WebAssembly. D-024 makes the core use the `libm` crate on every target. That's the library that just reproduced the module's arithmetic bit for bit. Clippy enforces it.
- **Robustness needs its own checks.** On this data, RTKLIB's kinematic filter is chaotic: a last-bit change moves the answer by metres. The plan already refuses to trust a fix on its ratio alone (forward/backward agreement, loop closures, repeat baselines, variance component estimation). The 166 m fix with a ratio of 6.9 is a concrete example of why. Static sessions on the GA data were stable.

### Solutions

These are from the module (identical in every build). They're 2-hour sessions unless noted.

| Baseline | km | Fix (Q) | Satellites | Ratio | Formal 3D σ (mm) | Forward and backward Q | Forward vs backward (mm) | 24 h Q | 2 h vs 24 h (mm) |
|---|---|---|---|---|---|---|---|---|---|
| STR1-STR2 | 0.07 | 1 | 33 | 12.3 | 0.8 | 1, 1 | 3.1 | 1 | 0.9 |
| KAT1-KAT2 | 0.13 | 1 | 34 | 4.6 | 0.8 | 1, 1 | 1.7 | 1 | 1.3 |
| HOB2-HOB5 | 0.21 | 1 | 30 | 47.4 | 0.8 | 1, 1 | 1.0 | 1 | 0.4 |
| GLB2-GLBN | 1.19 | 1 | 23 | 6.5 | 1.4 | 1, 1 | 2.0 | 1 | 2.8 |
| EML2-PKVL | 1.49 | 1 | 33 | 5.1 | 1.0 | 1, 1 | 5.0 | 1 | 5.7 |
| NEWE-NWCS | 2.29 | 1 | 22 | 5.3 | 1.4 | 1, 1 | 5.6 | 1 | 5.5 |
| SFLD-SWTC | 3.26 | 1 | 32 | 15.2 | 0.8 | 1, 1 | 3.2 | 1 | 4.7 |
| BNDR-HBRG | 7.94 | 1 | 22 | 6.9 | 1.3 | 1, 1 | 1.4 | 1 | 0.5 |
| CHSM-SYM1 | 9.17 | 1 | 23 | 3.3 | 1.4 | 1, 1 | 6.5 | 1 | 11.2 |
| MTHR-SNGO | 9.59 | 1 | 22 | 17.8 | 1.5 | 1, 1 | 20.0 | 1 | 22.0 |
| CNCD-SYDN | 9.70 | 1 | 25 | 4.3 | 1.4 | 1, 1 | 9.1 | 1 | 16.5 |
| KIRR-WFAL | 12.39 | 1 | 34 | 4.8 | 1.0 | 1, 1 | 10.0 | 1 | 6.0 |
| MENA-PCTN | 13.25 | 1 | 24 | 7.2 | 1.5 | 1, 1 | 8.4 | 1 | 10.2 |
| MTLD-RAYM | 17.59 | 1 | 22 | 10.4 | 1.4 | 1, 1 | 29.1 | 1 | 31.5 |
| KEPK-SUNB | 17.98 | 1 | 28 | 10.4 | 1.2 | 1, 1 | 1.5 | 1 | 10.1 |
| MANY-MURR | 26.10 | 1 | 29 | 3.0 | 2.6 | 2, 1 | 17.8 | 1 | 27.0 |
| RUTH-WDNG | 31.60 | 1 | 22 | 5.1 | 1.5 | 1, 1 | 22.0 | 1 | 30.6 |
| 5MGB-STRH | 33.82 | 1 | 18 | 9.5 | 1.8 | 1, 1 | 21.1 | 1 | 37.1 |
| LCL2-MAFF | 49.80 | 1 | 22 | 15.6 | 2.1 | 1, 2 | 73.1 | 1 | 27.0 |

- **Every baseline fixed**, in both the 2-hour and the whole-day solutions.
- **Short baselines agree closely.** Under 5 km, the forward and backward solutions agree within 6 mm, as do the 2-hour and whole-day solutions.
- **Longer baselines don't.** On five of the eight baselines over 10 km, the forward and backward solutions differ by more than 17 mm. The largest difference is 73 mm, on LCL2-MAFF (50 km), where the backward run only reached a float solution. The 2-hour and whole-day solutions differ by up to 37 mm.
- **The formal precisions are far too small.** On the longer baselines, the formal 3D standard deviations are 1 to 3 mm, while solutions from the same data differ by 10 to 35 times that. RTKLIB's covariances ignore time correlation and unmodelled errors. This is why the adjustment scales them with variance component estimation ([ARCHITECTURE.md](../ARCHITECTURE.md#107-variance-component-estimation), section 10.7).
- **The configuration matters too.** The spike used one configuration for every length. Beyond about 10 km, the driver won't use broadcast ionosphere and a fixed troposphere model (section 9.3), so these differences partly reflect the configuration, not only the engine.
- **RTKLIB rounds its precisions.** It writes standard deviations and covariance terms in metres to four decimals. For sub-millimetre formal precisions, that loses up to about 10% of the value, and small correlations entirely.

### Speed and memory

The range of the five timing baselines' medians (STR1-STR2, SFLD-SWTC, CNCD-SYDN, KEPK-SUNB and RUTH-WDNG):

| Case | Native | wasmtime | Chromium | Chromium vs native | Memory (native, wasmtime, Chromium) |
|---|---|---|---|---|---|
| 2 h session, inputs trimmed to the session | 0.8 to 2.0 s | 1.4 to 2.9 s | 1.2 to 2.3 s | 1.1 to 1.5 times (median 1.3) | 17 to 18 MB, 32 to 34 MB, 90 MB |
| 2 h session, whole-day input files | 1.6 to 2.9 s | 3.4 to 4.7 s | 3.0 to 4.6 s | 1.5 to 2.0 times (median 1.8) | 17 to 18 MB, 32 to 34 MB, 90 MB |
| Whole day (24 h at 30 s) | 9.0 to 17.6 s | 13.8 to 24.4 s | 11.3 to 21.0 s | 1.2 to 1.4 times (median 1.3) | 88 to 96 MB, 170 to 178 MB, 228 MB |

Per baseline, for the case the engine driver will run (a 2-hour session with trimmed inputs):

| Baseline | km | Native (s) | wasmtime (s) | Chromium (s) | Chromium vs native |
|---|---|---|---|---|---|
| STR1-STR2 | 0.07 | 1.58 | 2.43 | 2.10 | 1.3 |
| SFLD-SWTC | 3.26 | 2.04 | 2.87 | 2.33 | 1.1 |
| CNCD-SYDN | 9.70 | 1.01 | 1.50 | 1.31 | 1.3 |
| KEPK-SUNB | 17.98 | 1.01 | 1.72 | 1.52 | 1.5 |
| RUTH-WDNG | 31.60 | 0.84 | 1.37 | 1.22 | 1.5 |

- **Chromium ran the module faster than wasmtime**, typically by 10 to 15%. wasmtime's times include starting the process.
- **Trimming the inputs to the session roughly halves the run time in Chromium**, without changing the solution. RTKLIB parses every line of its input files even when it's given a time window. Parsing costs more in WebAssembly than natively (1.8 times native with whole-day files, against 1.3 times with trimmed ones), so trimming matters more in the browser. The core will trim the data in its own worker before the engine sees it (ARCHITECTURE.md, section 9.3).
- **Memory.**
  - A 2-hour session uses 90 MB of module memory whether or not the inputs are trimmed, because RTKLIB keeps only the epochs inside its time window. A whole day uses 228 MB.
  - Linear memory is reserved address space. Physical use is lower: under wasmtime, the peak RSS was 33 MB and 178 MB.
  - Eight workers running 2-hour sessions need under 1 GB, which is fine on a desktop. Phones need a smaller pool.
- **The antenna file doesn't matter.** Cutting the 18 MB ANTEX file down to the antennas in use changed run times by under 10% and memory by about 1 MB, so the driver can pass the whole file. (Removing the calibrations entirely changes the solution, so they're needed.)
- **Start-up.** In Chromium on the same machine, the engine was ready (downloaded, hash-checked and compiled) 1.3 s after navigation on a first visit throttled to 25 Mbit/s, and 0.1 s after navigation when loaded from the service worker's cache. The test server doesn't compress files, so a real host should be faster.

### Web performance targets

Section 20 of ARCHITECTURE.md now sets the engine targets from these measurements:

- **50 baselines (2-hour sessions at 30 s).** Under 30 s natively and under 1 minute in the web app. One Chromium worker needs about 1.6 s per baseline, so 50 baselines take about 80 s on one worker and roughly 15 s across six or seven. That leaves room for trimming and for larger satellite counts.
- **A whole-day baseline at 30 s.** Under 20 s natively and under 30 s in the web app.
- **Module memory.** Under 128 MB for a 2-hour session and under 256 MB for a whole day at 30 s.

The core's own speed in WebAssembly, for the adjustment and RINEX ingest targets, isn't measured yet. M2 and M6 will check those.

## Decision

**D-023 is confirmed.** None of its revisit conditions came up:

- the module builds
- its results on the GA baselines are identical to the native builds'
- it runs in 1.1 to 1.5 times native time in Chromium

Follow-ups, all made in the same change as this report:

- **D-024:** one maths library on every platform, enforced by clippy (CONVENTIONS.md, section 8).
- **ARCHITECTURE.md:**
  - section 4, determinism
  - section 9.3, notes for the engine driver
  - section 13, navigation files
  - section 14.1, the worker pool and memory
  - section 20, performance targets

**For M8 (the engine driver):**

- Trim the inputs to the session in the core.
- Use mixed-constellation navigation data.
- Run forward and backward solutions and compare them.
- Decide whether to patch RTKLIB's output precision in our build. Doing so changes the module hash, which is fine, since the hash is recorded either way.
- Size the worker pool by memory as well as cores.

## Limitations

- **One day of data**, from Australian CORS only: geodetic receivers and antennas, logging at 30 s. Low-cost receivers and 1 Hz data weren't tested.
- **One configuration for every baseline length.**
- **No accuracy assessment.** Base coordinates came from RINEX headers. The M10 benchmark assesses accuracy.
- **`igs14.atx`** rather than `igs20.atx`.
- **Narrow timing.** Times come from one 4-vCPU cloud VM and one browser engine, Chromium. Firefox and WebKit were checked for identical output, not for speed.
- **Browser memory** is the size of the module's linear memory, not physical use.
- **Parity holds for this build of the module.** Any change to RTKLIB-EX, wasi-sdk or the flags changes the module hash. CI then checks the new module against the golden output in every browser engine.

## Reproducing it

The steps are in [benchmarks/spike-m0/README.md](../../benchmarks/spike-m0/README.md). Results land in `benchmarks/spike-m0/work/results/`, which isn't committed:

- `runs.jsonl`, the parity runs
- `timing.jsonl`, the timing pass
- `summary.json`, written by `compare.py`
- the output of `libm_check.py`
