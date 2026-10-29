#!/usr/bin/env bash
# Build RTKLIB-EX's rnx2rtkp (the engine) and convbin (raw receiver data to
# RINEX) as WASI modules, and optionally natively.
#
# Usage:
#   WASI_SDK_PATH=/opt/wasi-sdk ./build.sh            # WebAssembly module only
#   WASI_SDK_PATH=/opt/wasi-sdk ./build.sh --native   # also a native build with $CC
#
# Environment:
#   WASI_SDK_PATH  wasi-sdk install (default /opt/wasi-sdk); must be version $WASI_SDK_VERSION
#   RTKLIB_SRC     RTKLIB-EX checkout (default .src/rtklib, cloned at the pinned tag if missing)
#   OUT            output directory (default dist/)
#   CC             native C compiler for --native (default cc)
#
# Outputs, in $OUT:
#   rnx2rtkp.wasm          the engine module
#   convbin.wasm           the converter module
#   *.wasm.sha256          their checksums
#   build-info.json        what was built, from what, with which flags
#   *-native               with --native
#   LICENSE-RTKLIB.txt     RTKLIB's BSD-2 licence, which must ship with the module
set -euo pipefail

RTKLIB_REPO=https://github.com/rtklibexplorer/rtklib
RTKLIB_TAG=v2.5.1
RTKLIB_COMMIT=62d4677ed8425a4e2748c6d390b500d1afb493fc
WASI_SDK_VERSION=34.0

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WASI_SDK_PATH="${WASI_SDK_PATH:-/opt/wasi-sdk}"
RTKLIB_SRC="${RTKLIB_SRC:-$HERE/.src/rtklib}"
OUT="${OUT:-$HERE/dist}"
CC="${CC:-cc}"

native=0
for arg in "$@"; do
  case "$arg" in
    --native) native=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

# Same source lists and defines as upstream's makefiles,
# app/consapp/rnx2rtkp/gcc/makefile and app/consapp/convbin/gcc/makefile.
RNX2RTKP_SOURCES=(
  app/consapp/rnx2rtkp/rnx2rtkp.c
  src/rtkcmn.c src/trace.c src/rinex.c src/rtkpos.c src/postpos.c src/solution.c
  src/lambda.c src/geoid.c src/sbas.c src/preceph.c src/pntpos.c src/ephemeris.c
  src/options.c src/ppp.c src/ppp_ar.c src/rtcm.c src/rtcm2.c src/rtcm3.c
  src/rtcm3e.c src/ionex.c src/tides.c src/sofa.c
)
CONVBIN_SOURCES=(
  app/consapp/convbin/convbin.c
  src/rtkcmn.c src/trace.c src/rinex.c src/sbas.c src/preceph.c src/rcvraw.c src/convrnx.c
  src/rtcm.c src/rtcm2.c src/rtcm3.c src/rtcm3e.c src/pntpos.c src/ephemeris.c src/ionex.c
  src/sofa.c
  src/rcv/novatel.c src/rcv/ublox.c src/rcv/crescent.c src/rcv/skytraq.c src/rcv/javad.c
  src/rcv/nvs.c src/rcv/binex.c src/rcv/rt17.c src/rcv/septentrio.c src/rcv/swiftnav.c
  src/rcv/unicore.c
)
DEFINES=(-DTRACE -DENAGLO -DENAQZS -DENAGAL -DENACMP -DENAIRN -DNFREQ=4 -DNEXOBS=3)
# -w: this is third-party code; we don't patch its warnings.
# -ffile-prefix-map: keeps build paths out of the module, so builds are reproducible.
CFLAGS=(-std=c99 -O3 -w -I src "-ffile-prefix-map=$RTKLIB_SRC=rtklib" "${DEFINES[@]}")

if [ ! -d "$RTKLIB_SRC/.git" ]; then
  git clone --depth 1 --branch "$RTKLIB_TAG" "$RTKLIB_REPO" "$RTKLIB_SRC"
fi
actual="$(git -C "$RTKLIB_SRC" rev-parse HEAD)"
if [ "$actual" != "$RTKLIB_COMMIT" ]; then
  echo "RTKLIB-EX checkout is at $actual, expected $RTKLIB_COMMIT ($RTKLIB_TAG)" >&2
  exit 1
fi

clang_wasi="$WASI_SDK_PATH/bin/clang"
if [ ! -x "$clang_wasi" ]; then
  echo "wasi-sdk not found at $WASI_SDK_PATH (set WASI_SDK_PATH)" >&2
  exit 1
fi
sdk_version="$(cat "$WASI_SDK_PATH/VERSION" 2>/dev/null | head -1 || true)"
if [ "$sdk_version" != "$WASI_SDK_VERSION" ]; then
  echo "warning: wasi-sdk version is '$sdk_version', pinned $WASI_SDK_VERSION; the module hash will differ" >&2
fi

mkdir -p "$OUT"
cd "$RTKLIB_SRC"

# --stack-first puts the stack below the data, so a stack overflow traps
# instead of silently overwriting globals. 8 MiB matches a typical native stack.
WASM_LDFLAGS=(-Wl,--stack-first -Wl,-z,stack-size=8388608)
build() { # name, sources...
  local name="$1"; shift
  "$clang_wasi" --target=wasm32-wasip1 --sysroot="$WASI_SDK_PATH/share/wasi-sysroot" \
    "${CFLAGS[@]}" "$@" "$HERE/wasi_compat.c" -lm "${WASM_LDFLAGS[@]}" \
    -o "$OUT/$name.wasm"
  if [ "$native" = 1 ]; then
    "$CC" "${CFLAGS[@]}" "$@" -lm -o "$OUT/$name-native"
  fi
  sha256sum "$OUT/$name.wasm" | cut -d' ' -f1
}
digest="$(build rnx2rtkp "${RNX2RTKP_SOURCES[@]}")"
echo "$digest  rnx2rtkp.wasm" > "$OUT/rnx2rtkp.wasm.sha256"
convbin_digest="$(build convbin "${CONVBIN_SOURCES[@]}")"
echo "$convbin_digest  convbin.wasm" > "$OUT/convbin.wasm.sha256"
cp license.txt "$OUT/LICENSE-RTKLIB.txt"

cat > "$OUT/build-info.json" <<EOF
{
  "module": "rnx2rtkp.wasm",
  "sha256": "$digest",
  "modules": { "rnx2rtkp.wasm": "$digest", "convbin.wasm": "$convbin_digest" },
  "rtklib": { "repo": "$RTKLIB_REPO", "tag": "$RTKLIB_TAG", "commit": "$RTKLIB_COMMIT" },
  "wasi_sdk": "$sdk_version",
  "clang": "$("$clang_wasi" --version | head -1)",
  "target": "wasm32-wasip1",
  "cflags": "${CFLAGS[*]}",
  "ldflags": "${WASM_LDFLAGS[*]}"
}
EOF

echo "rnx2rtkp.wasm  sha256 $digest"
echo "convbin.wasm   sha256 $convbin_digest"
