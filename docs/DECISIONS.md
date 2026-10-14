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
**Status:** Accepted

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
**Status:** Accepted

**Decision:** use RTKLIB-EX (formerly demo5, BSD-2), built from a pinned tag in CI, and run `rnx2rtkp` once per baseline as a separate process.

**Why:**
- It's the most capable maintained open-source baseline engine, and is tuned for modern multi-band receivers.
- Its post-processing keeps state in file-level globals and isn't re-entrant, so separate processes give parallelism and crash isolation.
- Pinned binaries make provenance exact.

**Consequences:**
- We own config generation and output parsing, including the signed covariance terms and our own forward/backward agreement check, which RTKLIB-EX doesn't do in static mode.
- The engine envelope (roughly 20 km or less with sessions of 1 h or more) is published and enforced by checks.

### D-004 Ginan as the optional PPP and long-baseline engine (Phase 2)
**Status:** Accepted

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

### D-012 PDF report generation
**Status:** Open

**Options:**
- **Typst, embedded as a Rust library.** Deterministic, offline and high quality. Embedding takes some integration work.
- **Print-to-PDF from the webview.** Simple, but output varies by platform.

**Decide by:** M12.

### D-013 Field capture platform
**Status:** Open

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
**Status:** Proposed

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
