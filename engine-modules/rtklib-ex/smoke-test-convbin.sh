#!/usr/bin/env bash
# Smoke-test a convbin WebAssembly module against RTKLIB's bundled raw data.
#
# Converts one file in each format RTKLIB ships a sample of (RTCM 3, u-blox,
# NovAtel, Javad, Hemisphere) under wasmtime, the way the web app does, and
# checks each RINEX file's SHA-256 against the golden value. The
# `PGM / RUN BY / DATE` record holds the time of conversion, so it's left out
# of the hash.
#
# Usage: ./smoke-test-convbin.sh [path/to/convbin.wasm]
# Environment: RTKLIB_SRC (default .src/rtklib), WASMTIME (default wasmtime)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODULE="$(realpath "${1:-$HERE/dist/convbin.wasm}")"
RTKLIB_SRC="${RTKLIB_SRC:-$HERE/.src/rtklib}"
WASMTIME="${WASMTIME:-wasmtime}"

# File, then convbin's options for it. RTCM 3 carries no week number, so it
# needs an approximate time.
CASES=(
  "GMSD7_20121014.rtcm3|-tr 2012/10/14 00:00:00"
  "testglo.rtcm3|-tr 2012/10/12 23:00:00"
  "ubx_20080526.ubx|"
  "oemv_200911218.gps|-r nov"
  "javad_20110115.jps|"
  "cres_20080526.bin|"
)

cd "$RTKLIB_SRC/test/data/rcvraw"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
failed=0
for case in "${CASES[@]}"; do
  file="${case%%|*}"
  read -r -a options <<< "${case#*|}"
  stem="${file%.*}"
  if ! "$WASMTIME" run --dir . --dir "$out::/out" "$MODULE" "${options[@]}" -os \
    -o "/out/$stem.obs" -n "/out/$stem.nav" "$file" > /dev/null 2> "$out/stderr"; then
    cat "$out/stderr" >&2
    echo "convbin smoke test FAILED: the module exited with an error on $file" >&2
    exit 1
  fi
  for kind in obs nav; do
    name="$stem.$kind"
    actual="$(grep -v 'PGM / RUN BY / DATE' "$out/$name" | sha256sum | cut -d' ' -f1)"
    expected="$(awk -v n="$name" '$2 == n { print $1 }' "$HERE/golden-convbin.sha256")"
    if [ "$actual" != "$expected" ]; then
      echo "FAILED: $name sha256 $actual, expected ${expected:-(none)}" >&2
      failed=1
    else
      echo "ok: $name"
    fi
  done
done
if [ "$failed" = 1 ]; then
  echo "convbin smoke test FAILED" >&2
  exit 1
fi
echo "convbin smoke test passed: ${#CASES[@]} files in 5 formats"
