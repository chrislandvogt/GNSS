# RTKLIB-EX engine module

RTKLIB-EX's `rnx2rtkp`, compiled to a single WebAssembly (WASI) module. The web app runs it in engine workers, and the CLI runs the same module with wasmtime (see [ARCHITECTURE.md](../../docs/ARCHITECTURE.md#93-rtklib-ex-module) and decision D-023).

## Pinned versions

| Component | Version |
|---|---|
| RTKLIB-EX | `v2.5.1`, commit `62d4677ed8425a4e2748c6d390b500d1afb493fc` |
| wasi-sdk | 34.0 (clang 23.1.0) |
| Target | `wasm32-wasip1` |

The pins live in `build.sh`. Changing any of them changes the module hash, which is recorded in provenance, so treat an update like any other engine change: re-run the benchmark before releasing it.

## Building

Install [wasi-sdk](https://github.com/WebAssembly/wasi-sdk) 34, then:

```bash
WASI_SDK_PATH=/opt/wasi-sdk ./build.sh            # dist/rnx2rtkp.wasm
WASI_SDK_PATH=/opt/wasi-sdk ./build.sh --native   # also dist/rnx2rtkp-native, for comparisons
```

The script clones RTKLIB-EX at the pinned tag into `.src/` (or uses `RTKLIB_SRC`), checks the commit, and writes the module, its SHA-256 and a `build-info.json` to `dist/`.

The build is reproducible. With wasi-sdk 34.0, `rnx2rtkp.wasm` has this SHA-256 whatever directory it's built in:

```
ff6b24e0d6c7c3b536f14ef70584caab5963800252abf017dd102beb29ab9eaa
```

## How the build differs from upstream

- **The same sources and defines as upstream's `rnx2rtkp` makefile.** RTKLIB-EX itself isn't patched.
- **`wasi_compat.c` supplies `system()`.** WASI has no shell. RTKLIB only calls `system()` to run gzip, tar or crx2rnx on compressed inputs. The stub always fails, so a compressed file is reported as unreadable rather than silently skipped. The engine driver always passes uncompressed RINEX.
- **The stack is 8 MiB and placed first** (`--stack-first`), so a stack overflow traps instead of corrupting memory.
- **The maths library is wasi-libc's, which is derived from musl.** A native build linked against musl gives byte-identical output to the module. Builds linked against glibc don't: their maths functions round some results differently in the last bit, and on kinematic data that changes RTKLIB's fix decisions. So a native build isn't a reference for the module. See the [M0 spike report](../../docs/spikes/m0-engine.md).

## Running

```bash
wasmtime run --dir . dist/rnx2rtkp.wasm -k static.conf -o out.pos rover.obs base.obs nav.rnx
```

The module only sees the directories the host grants it. Input and output paths must be inside them.

## Licence

RTKLIB is © T. Takasu and contributors, under the BSD 2-clause licence. Its licence text must ship with the module (the build copies it to `dist/LICENSE-RTKLIB.txt`), and the web app and CLI include it in their third-party notices.
