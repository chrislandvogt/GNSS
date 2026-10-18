# M0 engine spike

This tests the assumption behind decision D-023: that RTKLIB-EX can run as a WebAssembly module, in the browser and in the CLI, and give the same answers as a native build. The results are in [docs/spikes/m0-engine.md](../../docs/spikes/m0-engine.md).

These scripts are spike tooling: plain Python and Node, kept so the results can be reproduced. The benchmark proper (M10) replaces them with `gnss bench`.

## What it does

1. **`select_pairs.py`** picks CORS-pair baselines from GA's public site list, four per length bin between 0.05 and 50 km, and checks that each station has data on the chosen day. It writes `manifest.json`.
2. **`fetch.py`** downloads each station's daily RINEX 3 observations, plus a mixed-constellation navigation file, from the GA GNSS Data Repository. It then decompresses them into `work/rinex/`.
3. **`run.py`** runs every baseline through each build given to it, natively and under wasmtime. It records the SHA-256 of each solution file, the wall time and the peak memory in `work/results/runs.jsonl`.
4. **`browser/run_browser.mjs`** runs the same cases through the web app's engine workers in headless Chromium, under the production Content Security Policy.
5. **`trim_rinex.py`** trims observations to the 2-hour session window, as the engine driver will (ARCHITECTURE.md 9.3), for the timing pass.
6. **`compare.py`** summarises parity, solution quality and timings.

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
```

Downloaded data and results go in `work/`, which isn't committed. The data is about 1.3 GB.

## Data

Observations come from the GA GNSS Data Repository, which is published under Creative Commons Attribution (GA's pages cite both CC BY 3.0 AU and CC BY 4.0; see VALIDATION.md). The manifest lists stations and dates, not data.
