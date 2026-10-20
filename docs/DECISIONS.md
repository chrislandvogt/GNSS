# Decision log

This log records significant decisions, why they were made and what follows from them. Statuses:

| Status | Meaning |
|---|---|
| Accepted | In force |
| Proposed | Recommended but not yet confirmed |
| Open | Still to be decided |
| Superseded | Replaced by a later decision, which is linked |

Add new entries at the end with the next number. Never renumber an entry. Supersede it instead. Decisions that need more space than a few lines get a full ADR in `docs/adr/` (template added in M0), linked from here.

---

### D-001 Rust for the core
**Status:** Accepted

**Decision:** all domain logic is in Rust: model, datums, adjustment, formats, pipeline and engine drivers.

**Why:**
- A single installer with no runtime to install. Surveyors won't manage Python environments.
- Memory-safe parsing of untrusted receiver data.
- Performance for large networks and batch work.
- It compiles to Python wheels (PyO3) and WASM when needed.

**Consequences:** a smaller contributor pool than Python. This is mitigated by Python bindings (M17), so scripting users never need Rust.

### D-002 Desktop UI: Tauri 2 with Svelte, MapLibre GL and uPlot
**Status:** Superseded by D-023. The Svelte, MapLibre GL and uPlot stack carries over; the Tauri shell doesn't.

**Decision:** Tauri 2 shell, Svelte 5 + TypeScript UI, MapLibre GL JS maps, uPlot charts, and typed IPC generated with tauri-specta. All logic stays in Rust; the TypeScript is presentation only.

**Why:**
- The hard parts of this UI are maps, plots, large tables and printable reports, and the web stack is well ahead on all four.
- One codebase gives Windows, macOS, Linux and mobile.
- Sidecar support suits our engine model.

**Alternatives:**
- Leptos inside Tauri, if a Rust-only UI becomes a hard requirement. Same architecture, smaller ecosystem.
- egui, which is great for engineering tools but weaker on maps and forms.

**Consequences:** two languages in the repo. The rule that front ends contain no domain logic keeps the TypeScript thin.

### D-003 RTKLIB-EX as the v1 baseline engine, run as a sidecar process
**Status:** Accepted for the choice of engine. The sidecar execution model is superseded by D-023: RTKLIB-EX runs as a WebAssembly module.

**Decision:** use RTKLIB-EX (formerly demo5, BSD-2), built from a pinned tag in CI, and run `rnx2rtkp` once per baseline as a separate process.

**Why:**
- It's the most capable maintained open-source baseline engine, and is tuned for modern multi-band receivers.
- Its post-processing keeps state in file-level globals and isn't re-entrant, so separate processes give parallelism and crash isolation.
- Pinned binaries make provenance exact.

**Consequences:**
- We own config generation and output parsing, including the signed covariance terms and our own forward/backward agreement check, which RTKLIB-EX doesn't do in static mode.
- The engine envelope (roughly 20 km or less with sessions of 1 h or more) is published and enforced by checks.

### D-004 Ginan as the optional PPP and long-baseline engine (Phase 2)
**Status:** Accepted. Per D-023, Ginan runs natively (CLI and server) unless a WebAssembly build proves practical.

**Decision:** integrate Geoscience Australia's Ginan (Apache-2.0) for single-site PPP and long baselines, run as a sidecar.

**Why:** it's geodetic-grade, open and Australian-maintained. It also gives an offline equivalent of AUSPOS for users who can't or shouldn't use online services.

### D-005 No linking of copyleft code
**Status:** Accepted

**Decision:** no GPL, AGPL or LGPL code is linked into our binaries. Copyleft tools may run only as separate processes. MPL-2.0 is allowed, because its file-level copyleft is compatible with our licence. cargo-deny enforces this in CI.

**Why:** it keeps the project permissively licensed, which matters for adoption by agencies and companies.

### D-006 Don't depend on the `gnss-rtk` crate
**Status:** Accepted (revisit if its licence or features change)

**Decision:** don't use `gnss-rtk`.

**Why:** it's AGPL-3, which would pull the whole app to AGPL (D-005). We also couldn't find integer ambiguity resolution in it, so it isn't a survey-grade baseline engine.

### D-007 Use the nav-solutions `rinex` crate behind our own interface
**Status:** Accepted (confirm the licence of each pinned version)

**Decision:** parse RINEX with the `rinex` crate (MPL-2.0 in recent releases; earlier GeoRust-era releases were Apache-2.0/MIT), wrapped behind a trait in `rinex-io`.

**Why:** it's mature Rust RINEX support, including Hatanaka compression. The wrapper lets us swap or fork it if needed.

### D-008 Licence: Apache-2.0
**Status:** Proposed. Confirm in M0.

**Decision:** license the project under Apache-2.0.

**Why:**
- It's permissive, which suits agencies and companies.
- It includes an explicit patent grant, which matters in a field with active GNSS patents.
- It's compatible with Ginan and with MPL-2.0 dependencies.

**Alternative:** dual MIT/Apache-2.0, following Rust convention. This adds GPLv2 compatibility, which we're unlikely to need.

### D-009 Project format: a folder with immutable raw data
**Status:** Accepted

**Decision:** a project is a folder containing:
- TOML files, one per occupation
- an immutable `raw/` directory with hashes
- overlays for corrections
- regenerable results
- an append-only provenance record

**Why:** it's Git-friendly, auditable, hand-editable and easy to archive. It also makes reproducibility structural.

### D-010 Native adjustment, with DynAdjust as the oracle
**Status:** Accepted

**Decision:** implement the least squares adjustment natively in Rust. Use DynAdjust as the reference implementation in tests and as an export target, not as a runtime dependency.

**Why:**
- The adjustment is core to the product, needs deep integration (explanations, map overlays, incremental re-runs) and is well-defined maths.
- An independent oracle gives strong validation.

### D-011 Geodesy implementation
**Status:** Proposed

**Decision:**
- Implement the analytic transformations natively: conversions, Helmert 7 and 14, plate motion and Transverse Mercator.
- Implement a native NTv2 and GeoTIFF grid reader.
- Use PROJ as a test oracle, and optionally as a backend for other jurisdictions.

**Why:** it avoids building PROJ on every platform, keeps the core pure Rust and easy to audit, and the native code is validated against PROJ and GeodePy anyway.

**Progress (M1):** both parts are built as proposed, so confirming this decision needs no further work.
- The analytic transformations match PROJ to the last bit or to nanometres.
- The grid reader matches PROJ to under a micrometre on ICSM's GDA94 to GDA2020 grid and on AUSGeoid2020.
- GeoTIFF decoding uses the `tiff` crate (MIT), wrapped behind the reader's own types.

### D-012 PDF report generation
**Status:** Open

**Options:**
- **Typst, embedded as a Rust library.** Deterministic, offline and high quality. Embedding takes some integration work.
- **Print-to-PDF from the webview.** Simple, but output varies by platform.

**Decide by:** M12.

### D-013 Field capture platform
**Status:** Superseded by D-023. Field capture is a mode of the web app.

**Options:**
- **Tauri mobile app.** Native capabilities and the shared codebase, but app store distribution.
- **PWA.** No install friction and works offline, but limited device APIs on iOS.

**Decide by:** the start of M14.

### D-014 Project name
**Status:** Open

**Decision needed:** a short, memorable name that means something to surveyors. One suggestion is "Trig", after trig stations. Check crates.io, PyPI, domains and trademarks before adopting.

**Decide by:** M0, before any crates are published.

### D-015 No telemetry
**Status:** Accepted

**Decision:** the software collects no usage data. Adoption is measured through downloads, pilots and opt-in surveys.

**Why:** many target users work with sensitive locations or in air-gapped environments, and trust is the product.

### D-016 Supported platforms
**Status:** Superseded by D-023. The web app supports current browsers; this list now applies to the native CLI.

**Decision:**
- **Windows 10 and 11 x64** is the primary platform; most surveyors work there.
- **macOS:** arm64, plus x64 while practical.
- **Linux:** x64 (AppImage, .deb) and arm64.

### D-017 Default adjustment frame strategy for Australia
**Status:** Proposed (validate with GA)

**Decision:**
1. Process in the orbit frame (IGS20) at the observation epoch.
2. Adjust in ATRF2014 at the mean observation epoch, with GDA2020 control brought to that epoch through the plate motion model.
3. Output GDA2020, AHD via AUSGeoid2020 (or AVWS) and MGA2020.

The tool shows the difference from adjusting directly in GDA2020, so the shortcut can be used when it's provably negligible.

**Why:** rigorous handling of epochs, while staying transparent about when it matters.

### D-018 RINEX first
**Status:** Accepted

**Decision:** support RINEX 2.11, 3.0x and 4.0x, including Hatanaka compression. Don't read proprietary raw formats such as Trimble T02/T04 or Leica MDB. Open receiver formats are handled through conversion (D-020).

**Why:** proprietary formats are undocumented and change. Every vendor exports RINEX.

### D-019 GLONASS ambiguity resolution off by default in v1
**Status:** Proposed

**Decision:** GLONASS observations are used as float or excluded, and GLONASS integer ambiguity resolution is off by default.

**Why:** inter-frequency biases between different receiver brands make GLONASS ambiguity resolution unreliable on mixed-brand baselines.

**Revisit:** with benchmark evidence (M10), and possibly enable it for same-brand baselines.

### D-020 Decoding open receiver formats
**Status:** Open

**Options:**
- **Call `convbin` from RTKLIB-EX** for UBX, SBF, Unicore, NovAtel and RTCM3. Fast to ship.
- **Native Rust decoders.** Fuzzable and better integrated, but more work.

**Decide by:** M6.

### D-021 Contributions use DCO sign-off
**Status:** Proposed

**Decision:** contributors certify the Developer Certificate of Origin with `Signed-off-by` lines, enforced by a CI check. No CLA.

**Why:** it's lightweight and widely understood, and doesn't deter contributors from agencies and universities.

### D-022 Ship the second-opinion adjustment first
**Status:** Accepted

**Decision:** Phase 0 delivers vector import, adjustment and reporting before any baseline processing.

**Why:**
- It's useful immediately to anyone who already has vectors.
- It doesn't require users to trust our engine.
- The adjustment's correctness can be validated against an oracle.
- It builds the foundation the rest depends on.

### D-023 Browser-first, local-first web app, with a native CLI
**Status:** Accepted. Confirmed by the M0 engine spike ([report](spikes/m0-engine.md)).

**Decision:**
- The main front end is a web app that runs entirely in the user's browser. The Rust core is compiled to WebAssembly and runs in a web worker. The site is static files: there's no application server, and project data never leaves the user's machine unless they export it or configure a destination.
- Projects stay folders on the user's disk (D-009), opened through the File System Access API in Chrome and Edge. Firefox and Safari keep projects in browser storage, with zip import and export.
- RTKLIB-EX is compiled to a WebAssembly (WASI) module. The web app runs it in a pool of web workers, with a fresh instance for every run. The native CLI runs the same module with wasmtime.
- The native CLI stays. It's the same core compiled for Windows, macOS and Linux. It runs the benchmark, CI, scripting and batch work, prepares data packs, and serves the web app for offline and air-gapped use. The Python bindings and the server build on the native crates as before.
- The web app works offline once loaded, and can be installed as an app where the browser supports it.
- Each release is deployed at its own permanent, versioned URL, and published as a downloadable bundle.
- Data sources that browsers can't reach directly, because they don't send cross-origin headers, are fetched through an optional relay or brought in with an offline data pack.

**Why:**
- Nothing to install. Trying the tool takes a link, and it runs on locked-down agency machines where users can't install software.
- It removes the installers, app code signing, the updater and the per-platform sidecar builds, which were a large share of the work for a single maintainer.
- It stays local-first. Untrusted files are parsed inside WebAssembly, inside the browser's sandbox, which isolates them better than a native process does.
- WebAssembly arithmetic is fully specified, so the same build gives bit-identical results on every operating system and browser.
- There's one engine artifact everywhere, so CI benchmarks the exact RTKLIB-EX module that users run.
- One code base covers the office, the field (phones and tablets) and agency self-hosting.
- The UI stack from D-002 (Svelte, MapLibre GL and uPlot) carries over unchanged.

**Alternatives:**
- **Keep the Tauri desktop app (D-002).** Native speed and full file access, but the installers, signing and per-platform builds remain.
- **Hosted processing (software as a service).** The simplest client, but survey data would leave the user's machine, offline use would be lost, and the project would have to run paid servers with accounts. It survives only as an optional, self-hosted agency server (M18).
- **Wrap the web app in Tauri later.** Still possible, since the UI would be the same, if browser limits turn out to matter. Not planned.

**Consequences:**
- Supersedes D-002, D-013 and D-016, and the sidecar execution model in D-003. Ginan (D-004) runs natively, in the CLI and server, unless a WebAssembly build proves practical; the web app reaches it through a local CLI or an agency server. If `convbin` is chosen (D-020), it also runs as a WebAssembly module.
- D-005's "separate process" boundary carries over to engine modules: they're built separately and talk to our code only through files and arguments. Confirm this reading before shipping any copyleft module.
- Editing a project folder in place needs Chrome or Edge. Other browsers keep projects in browser storage, which the browser can evict, so the app requests persistent storage and treats the folder or an exported zip as the durable copy.
- WebAssembly is slower than native code, and an instance's memory is limited to 4 GB at most. So the core trims and decimates RINEX before the engine sees it, and very large jobs can go to the CLI. The core's maths runs single-threaded in the browser, because shared-memory threads currently need a nightly Rust toolchain and cross-origin isolation headers. The web performance targets are set from the spike's measurements.
- Some sources need the relay. GA's Web API and archive downloads send cross-origin headers; `files.igs.org` and the S3 host of the AUSGeoid2020 grid don't (all checked 2026-10-15). The relay sees which files are requested, which hints at where and when a survey happened, so it's off by default, self-hostable, and built into `gnss serve`.
- The static host sees page requests, like any website, but never project data. There are no analytics scripts (D-015).
- Users get a new version when they reload. Old versions stay at their URLs and as bundles, so a project can be re-run on the exact build that produced it.

**Revisit:** if the M0 engine spike shows that RTKLIB-EX can't be built as a WebAssembly module, or that its results or speed are unacceptable compared with the native build. The fallback is to run it natively through the CLI, with the web app calling a local `gnss serve`, and to reconsider a desktop wrapper.

**Spike results (M0, October 2026):** none of the revisit conditions came up. Details are in the [spike report](spikes/m0-engine.md).
- **It builds.** RTKLIB-EX builds as a WASI module with wasi-sdk, needing only a stub for `system()`. The module is 1.17 MB (522 kB gzipped), and the build is reproducible.
- **The results are identical.** On 19 GA CORS-pair baselines (0.07 to 50 km, four cases each), all 76 solution files from the module, in wasmtime and in Chromium, are byte-identical to those of native glibc and musl builds. RTKLIB's test output is byte-identical in Chromium, Firefox and WebKit.
- **Speed.** In Chromium, a 2-hour session with trimmed inputs takes 1.2 to 2.3 s, 1.1 to 1.5 times native. A whole day at 30 s takes 1.2 to 1.4 times native. Untrimmed inputs cost 1.5 to 2.0 times native, so the core trims inputs before every run.
- **Memory.** A 2-hour session uses 90 MB of module memory, and a whole day at 30 s uses 228 MB.
- **A finding that changes the plan.** On RTKLIB's kinematic test data, glibc builds and the module make different fix decisions, and positions end up metres apart. The cause is the maths library. That confirms running the module in the CLI rather than a native build, and led to D-024.

### D-024 One maths library on every platform
**Status:** Proposed

**Decision:**
- The Rust core computes transcendental functions with the `libm` crate, a Rust port of musl's maths library, on every target. That covers the trigonometric and hyperbolic functions and their inverses, `atan2`, `exp`, logarithms, powers, `cbrt` and `hypot`.
- The core never calls the matching `f64` methods. Clippy's `disallowed-methods` rejects them (`clippy.toml`).
- Dependencies that do maths for us are configured to use the same library: nalgebra, for example, with its `libm-force` feature.
- Operations that IEEE 754 defines exactly stay as they are: arithmetic, `sqrt`, `mul_add`, `abs` and rounding.
- Golden numeric tests compare results exactly, and run both natively and in the browser in CI.

**Why:**
- **Last bits change decisions.** The M0 spike found that native RTKLIB-EX builds linked against glibc, and the WebAssembly module with its musl-derived maths library, make different ambiguity-fixing decisions on kinematic data. Their positions end up metres apart. Replacing glibc's maths functions with the `libm` crate's made the native build match the module bit for bit ([spike report](spikes/m0-engine.md)). Our own code has decisions too, such as outlier tests against critical values and convergence checks, and a last-bit difference can tip any of them.
- **Rust's `f64` methods depend on the platform.** `f64::sin` and its relatives call glibc on Linux, the system library on macOS and Windows, and a musl port in WebAssembly. The same core would otherwise give slightly different numbers in the CLI on each operating system and in the browser.
- **It makes reproducibility exact.** With one maths library, the CLI and the web app give bit-identical results for the same inputs, so "re-run the project and get the same answer" can be tested exactly. A difference between the CLI and the browser then always means a bug.
- **The core and the engines share their arithmetic.** The `libm` crate is the library that reproduced the engine module's results in the spike.

**Alternatives:**
- **Accept platform differences within a tolerance** (the earlier plan: 1e-6 m). It's simpler, but a decision near a threshold can still flip, and agreement within a tolerance is harder to test and explain than identity.
- **Correctly rounded maths libraries.** These would agree everywhere by definition, but there's no mature pure-Rust one covering every function we need yet.

**Consequences:**
- Code calls `libm::sin(x)`, not `x.sin()`. Clippy says so.
- The `libm` crate can be slightly slower than a platform's tuned routines. That's negligible next to the rest of the work.
- Determinism has more to check:
  - Parallel code must not change the order of floating-point operations.
  - Any dependency with CPU-specific kernels, such as a matrix multiply that uses fused multiply-add where the processor has it, must be configured to avoid them or kept out of numeric paths.
  - The cross-target golden tests catch what slips through.

**Revisit:** if a dependency we need can't be made deterministic. Then the affected results get a documented tolerance instead.

### D-025 Our own linear algebra and distributions in the adjustment
**Status:** Proposed

**Decision:**
- The adjustment solves its normal equations with its own block-sparse factorisation: `L D Lᵀ` in 3×3 blocks, in minimum-degree order with ties to the lowest mark, and the Takahashi recurrence for the selected inverse. It doesn't use faer.
- nalgebra is built without its `std` feature, which would hand dynamic matrix products to matrixmultiply.
- The distributions behind the critical values (normal, χ², non-central χ² and Student's t) are our own, computed with `libm`. It doesn't use statrs.

**Why:**
- **Test decisions sit on the last bits (D-024).** General-purpose linear algebra libraries choose SIMD and FMA kernels by CPU at run time. faer has its own dispatch, and nalgebra with `std` uses matrixmultiply for products larger than about 5×5. Different kernels sum in different orders, so results would differ between machines, and between native code and the browser.
- **statrs uses the platform's maths library** through the `f64` methods, which D-024 rules out, and its quantiles would set the critical values.
- **The structure is simple.** A GNSS network's normal equations are 3×3 blocks with a sparse pattern. A block factorisation is a few hundred lines, and the selected inverse gives every block the statistics need at about the cost of the factorisation.
- **It's fast enough by a wide margin.** 1,000 marks and 3,000 vectors adjust in 17 ms natively and 38 ms in Chromium, against targets of 2 s and 6 s. 10,000 marks and 30,000 vectors take 0.3 s natively.

**Alternatives:**
- **faer, or another sparse solver, with a tolerance on results.** Faster for very large networks, but it gives up bit-identical results, and a test near its critical value could flip between the CLI and the web app.
- **A dense path for small networks**, as first planned. The sparse path is exact and fast at every size, so it isn't needed.

**Consequences:**
- We maintain the numerical code. It's checked against dense nalgebra solutions, a textbook dense adjustment, SciPy's distributions and the cross-platform golden hash.
- The ordering is a simple greedy minimum degree. Much larger networks may want a better one, which stays deterministic as long as it's our own code.

**Revisit:** if networks far beyond 10,000 marks become common and the solver becomes the bottleneck.
