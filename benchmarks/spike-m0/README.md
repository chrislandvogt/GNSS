# M0 engine spike

This tests the assumption behind decision D-023: that RTKLIB-EX can run as a WebAssembly module, in the browser and in the CLI, and give the same answers as a native build. The results are in [docs/spikes/m0-engine.md](../../docs/spikes/m0-engine.md).

These scripts are spike tooling: plain Python and Node, kept so the results can be reproduced. The benchmark proper (M10) replaces them with `gnss bench`.

## What it does

1. **`select_pairs.py`** picks CORS-pair baselines from GA's public site list, four per length bin between 0.05 and 50 km, and checks that each station has data on the chosen day. It writes `manifest.json`.
2. **`fetch.py`** downloads each station's daily RINEX 3 observations, plus a mixed-constellation navigation file, from the GA GNSS Data Repository. It then decompresses them into `work/rinex/`.
3. **`run.py`** runs every baseline through each build given to it, natively and under wasmtime. It records the SHA-256 of each solution file, the wall time and the peak memory in `work/results/runs.jsonl`.
4. **`browser/run_browser.mjs`** runs the same cases through the web app's engine workers in headless Chromium, under the production Content Security Policy.
5. **`trim_rinex.py`** trims observations to the 2-hour session window, as the engine driver will (ARCHITECTURE.md 9.3), for the timing pass.
6. **`compare.py`** summarises parity, solution quality and timings, per baseline.
7. **`libm_check.py`** runs RTKLIB's own test data through several builds in five modes, writes every epoch, and compares them with the first build. **`libm-shim/`** is a small library that replaces a glibc build's maths functions with the `libm` crate's, to show whether differences come from the maths library.

Each baseline is run as four cases: a 2-hour session (02:00 to 04:00 GPST) as combined, forward-only and backward-only solutions, plus the whole day as a combined solution. The configuration is `spike.conf`.

## Reproducing it

You'll need Python 3 with `pip install hatanaka`, Node.js 22, wasmtime, wasi-sdk 34 and a C compiler.

```bash
# Build the engine module (and a native build to compare against).
WASI_SDK_PATH=/opt/wasi-sdk ../../engine-modules/rtklib-ex/build.sh --native
wasmtime compile ../../engine-modules/rtklib-ex/dist/rnx2rtkp.wasm -o rnx2rtkp.cwasm

python select_pairs.py --date 2025-06-15 --per-bin 4
python fetch.py
python run.py --antex <rtklib>/data/ant/igs14.atx \
  --build native=../../engine-modules/rtklib-ex/dist/rnx2rtkp-native \
  --build wasmtime="wasmtime run --allow-precompiled --dir . $PWD/rnx2rtkp.cwasm"

# The browser leg: build the web app first.
(cd ../../apps/web/ui && npm ci && npm run build)
node browser/run_browser.mjs

python compare.py

# The timing pass, on an otherwise idle machine: five baselines, the 2-hour case
# three times with whole-day and with trimmed inputs, and the whole day once.
# Results are appended, so remove work/results/timing.jsonl first.
python trim_rinex.py
PAIRS=STR1-STR2,SFLD-SWTC,CNCD-SYDN,KEPK-SUNB,RUTH-WDNG
BUILDS=(--build native=... --build wasmtime=...)   # as above
python run.py --log timing.jsonl --only $PAIRS --cases 2h-combined --repeat 3 --antex <atx> "${BUILDS[@]}"
python run.py --log timing.jsonl --only $PAIRS --cases 2h-combined --repeat 3 --trimmed --antex <atx> "${BUILDS[@]}"
python run.py --log timing.jsonl --only $PAIRS --cases 24h-combined --antex <atx> "${BUILDS[@]}"
node browser/run_browser.mjs --log timing.jsonl --only $PAIRS --cases 2h-combined --repeat 3
node browser/run_browser.mjs --log timing.jsonl --only $PAIRS --cases 2h-combined --repeat 3 --trimmed
node browser/run_browser.mjs --log timing.jsonl --only $PAIRS --cases 24h-combined
python compare.py

# The maths library check, on RTKLIB's own test data.
cargo build --release --manifest-path libm-shim/Cargo.toml
NATIVE=$PWD/../../engine-modules/rtklib-ex/dist/rnx2rtkp-native
python libm_check.py --data <rtklib>/test/data/rinex \
  --build wasmtime="wasmtime run --dir . $PWD/../../engine-modules/rtklib-ex/dist/rnx2rtkp.wasm" \
  --build glibc=$NATIVE \
  --build glibc+libm-crate="env LD_PRELOAD=$PWD/libm-shim/target/release/liblibm_shim.so $NATIVE"
```

The native build from `build.sh --native` uses the system's C compiler, so on most Linux systems it links glibc. A musl build for comparison can be made with `zig cc -target x86_64-linux-musl` and the same flags.

Downloaded data and results go in `work/`, which isn't committed. The data is about 1.3 GB.

## Data

Observations come from the GA GNSS Data Repository, which is published under Creative Commons Attribution (GA's pages cite both CC BY 3.0 AU and CC BY 4.0; see VALIDATION.md). The manifest lists stations and dates, not data.
