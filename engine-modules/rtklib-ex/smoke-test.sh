#!/usr/bin/env bash
# Smoke-test an rnx2rtkp WebAssembly module against RTKLIB's bundled test data.
#
# Runs one static relative-positioning case from upstream's rnx2rtkp makefile
# (test4) under wasmtime and checks the solution file's SHA-256 against the
# golden value. A module built from the pinned sources with the pinned
# wasi-sdk must reproduce it exactly.
#
# Usage: ./smoke-test.sh [path/to/rnx2rtkp.wasm]
# Environment: RTKLIB_SRC (default .src/rtklib), WASMTIME (default wasmtime)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODULE="$(realpath "${1:-$HERE/dist/rnx2rtkp.wasm}")"
RTKLIB_SRC="${RTKLIB_SRC:-$HERE/.src/rtklib}"
WASMTIME="${WASMTIME:-wasmtime}"
GOLDEN="$(awk '$2 == "test4.pos" { print $1 }' "$HERE/golden.sha256")"

cd "$RTKLIB_SRC/test/data/rinex"
out="$(mktemp)"
err="$(mktemp)"
trap 'rm -f "$out" "$err"' EXIT
# RTKLIB writes progress to stderr; show it only if something goes wrong.
if ! "$WASMTIME" run --dir . "$MODULE" -t -p 3 -e -r -3978241.958 3382840.234 3649900.853 \
  07590920.05o 30400920.05n 30400920.05o > "$out" 2> "$err"; then
  cat "$err" >&2
  echo "smoke test FAILED: the module exited with an error" >&2
  exit 1
fi
actual="$(sha256sum "$out" | cut -d' ' -f1)"

if [ "$actual" != "$GOLDEN" ]; then
  echo "smoke test FAILED: test4.pos sha256 $actual, expected $GOLDEN" >&2
  exit 1
fi
echo "smoke test passed: test4.pos sha256 $actual"
