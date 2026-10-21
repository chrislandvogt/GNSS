# Roadmap

This is the work plan: four phases broken into milestones, each with tasks, exit criteria, dependencies and a size. The reasons behind it are in [PLAN.md](PLAN.md), and the technical detail in [ARCHITECTURE.md](ARCHITECTURE.md).

## How to read this

- **Phases** group milestones by release: Phase 0 leads to v0.1, Phase 1 to v1.0, Phase 2 to v1.x, and Phase 3 to v2.0.
- **Milestones** (M0 to M24) are shippable increments. Each one has a goal, tasks, exit criteria and dependencies.
- **Sizes** are rough effort for one experienced developer:

  | Size | Effort |
  |---|---|
  | S | Days |
  | M | 2 to 4 weeks |
  | L | 1 to 3 months |
  | XL | More than 3 months |

- **Exit criteria** are the definition of done. A milestone isn't done until they're met, whatever the task list says.
- Task boxes are meant to be ticked here, or turned into GitHub issues under matching milestones.

## Start here: the first ten tasks

1. [ ] Pick the project name and check crates.io, PyPI, domains and trademarks (D-014, M0).
2. [x] Add LICENSE (Apache-2.0) and CONTRIBUTING with DCO sign-off (D-008, M0).
3. [x] Create the Cargo workspace and the CI matrix, with native and WebAssembly builds and cargo-deny (M0).
4. [x] Run the engine spike: build RTKLIB-EX as a WebAssembly module and compare it with the native build. This confirms or revisits D-023 (M0).
5. [x] Write the conventions doc covering units, frames, time, errors, numerics and host access (M0).
6. [x] Implement the `datum` core types so every position carries a frame and an epoch, then ECEF, geodetic and ENU conversions plus Helmert transformations, with property tests (M1).
7. [ ] Build the synthetic network generator, which the M2 tests depend on (M2).
8. [ ] Implement a vector-only adjustment with minimal constraints and the global test (M2).
9. [ ] Install DynAdjust locally and produce the first oracle outputs (M2, M4).
10. [ ] Contact GA (the Ginan and DynAdjust teams) and one university about the benchmark (Community).

## Dependency graph

```mermaid
flowchart LR
  subgraph P0["Phase 0 - v0.1"]
    M0[M0 Bootstrap] --> M1[M1 Datum core]
    M0 --> M2[M2 Adjustment]
    M1 --> M2
    M2 --> M3[M3 Formats and CLI]
    M3 --> M4[M4 v0.1 release]
  end
  subgraph P1["Phase 1 - v1.0"]
    M4 --> M5[M5 Project and pipeline]
    M5 --> M6[M6 RINEX and QC]
    M5 --> M7[M7 Data fetch]
    M6 --> M8[M8 RTKLIB-EX]
    M7 --> M8
    M6 --> M9[M9 Blunder checks]
    M8 --> M10[M10 Benchmark v1]
    M5 --> M11[M11 Web app]
    M8 --> M11
    M9 --> M11
    M3 --> M12[M12 Reporting]
    M10 --> M13[M13 v1.0 hardening]
    M11 --> M13
    M12 --> M13
  end
  subgraph P2["Phase 2 - v1.x"]
    M13 --> M14[M14 Field capture]
    M14 --> M15[M15 In-field check]
    M8 --> M15
    M7 --> M16[M16 Ginan]
    M5 --> M17[M17 Python and ArcGIS]
    M5 --> M18[M18 Server and MCP]
    M1 --> M19[M19 Jurisdictions]
  end
  subgraph P3["Phase 3 - v2.0"]
    M10 --> M20[M20 Rust engine]
    M20 --> M21[M21 Multi-baseline]
    M20 --> M22[M22 Dual-engine checks]
    M2 --> M23[M23 Terrestrial obs]
    M8 --> M24[M24 Monitoring]
  end
```

The critical path to v1.0 is M0 → M1 → M2 → M3 → M4 → M5 → M6 → M8 → M10/M11 → M13. Validation work (oracles, then the benchmark) runs alongside from M2 onwards.

---

## Phase 0: Foundations and the second-opinion adjustment (v0.1)

The goal is a tool that rigorously adjusts imported GNSS vectors and reports the results, in the browser and the CLI. It's useful from day one to anyone who already has vectors, and it doesn't require anyone to trust our baseline processing.

### M0 Bootstrap [M]

**Goal:** a repo people can build, test and contribute to, and evidence that the browser-first architecture works (D-023). The engine spike makes this milestone M rather than S.

- [ ] Choose the project name and check for conflicts (D-014).
- [x] Add LICENSE and NOTICE (Apache-2.0, D-008). The maintainer confirms the licence and copyright line before merging.
- [x] Create the Cargo workspace with empty crates per [ARCHITECTURE.md](ARCHITECTURE.md#2-repository-layout). Pin the stable toolchain in `rust-toolchain.toml`, with the `wasm32-unknown-unknown` target, and use edition 2024.
- [x] Set up CI in GitHub Actions on Windows, macOS and Linux: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, plus a WebAssembly build of the core with its tests run in a headless browser.
- [x] Engine spike (D-023). Results: [docs/spikes/m0-engine.md](spikes/m0-engine.md).
  - [x] Build RTKLIB-EX's `rnx2rtkp` as a WASI module with wasi-sdk, trying Emscripten if wasi-sdk proves impractical.
  - [x] Run it in a browser worker and in wasmtime, and confirm the output hashes are identical.
  - [x] Compare it with the native build on about 20 CORS-pair baselines from GA: solutions, covariances, fix decisions and ratios.
  - [x] Measure its speed and peak memory against the native build.
  - [x] Record the results in D-023, and set the web performance targets ([ARCHITECTURE.md](ARCHITECTURE.md#20-performance-targets)).
- [x] Web skeleton: a Vite and Svelte page that loads the core in a worker, opens a project folder (or uses OPFS where folders aren't available), and works offline through a service worker.
- [ ] Deploy the web skeleton at a versioned URL on a static host that can set response headers. Needs the maintainer to choose a host and create the account (Cloudflare Pages or Netlify; CI already builds the deployable site).
- [x] Add cargo-deny for licences, advisories, bans and sources. Deny GPL, AGPL and LGPL. Allow-list MPL-2.0 (D-005). Apply the same licence policy to the web app's JavaScript packages.
- [x] Write CONTRIBUTING.md (DCO sign-off), CODE_OF_CONDUCT.md (Contributor Covenant), SECURITY.md, and issue and PR templates. The maintainer adds the Code of Conduct contact and enables private vulnerability reporting.
- [x] Write `docs/CONVENTIONS.md`: units, frames, epochs, time scales, covariance storage, error handling, logging, numerics, host access and determinism.
- [x] Add an ADR template for decisions too big for the log.
- [ ] Set up a GitHub project board with milestones M0 to M13 and labels (`area/*`, `type/*`, `good first issue`). `scripts/setup-github.ps1` creates the milestones and labels; the board itself is created on GitHub.

**Exit criteria:**
- CI is green on all three operating systems and for the WebAssembly build, with the empty crates.
- The engine spike's results are recorded, and D-023 is confirmed or revisited.
- The web skeleton is deployed.
- The licence, contribution docs and board are in place.

### M1 Datum and geodesy core [M]

**Goal:** frames, epochs and transformations that are correct by construction.

- [x] Time: adopt hifitime. Handle GPST, UTC and TAI conversions with leap seconds. Compute decimal-year epochs in one documented function (`CoordinateEpoch::from_time`).
- [x] Core types:
  - [x] `FrameId` with a registry covering ITRF2020, ITRF2014, IGS20, ATRF2014, GDA2020, GDA94, and WGS84 realisations as aliases with a stated tolerance.
  - [x] `CoordinateEpoch`, and `Position { frame, epoch, xyz, cov }`.
  - [x] Geodetic and ENU views.
  - [x] Enforce the static-frame and dynamic-frame rules.
- [x] Ellipsoids (GRS80, WGS84): ECEF to geodetic and back, and local ENU rotations, each with covariance propagation.
- [x] Helmert transformations:
  - [x] 7-parameter and 14-parameter (time-dependent), with covariance propagation.
  - [x] IERS ITRF2020 to ITRF2014 parameters.
- [x] Plate motion: ITRF2014 and ATRF2014 to GDA2020 using the Australian Plate Motion Model from the GDA2020 Technical Manual.
- [x] GDA94 to GDA2020: the 7-parameter conformal transformation and the conformal-plus-distortion grid (ICSM's NTv2 file or PROJ's GeoTIFF), with guidance on which to use when.
- [x] Grids:
  - [x] NTv2 and geoid grid interpolation, from NTv2 and PROJ's GeoTIFF grids, read in slices.
  - [x] AUSGeoid2020 to convert ellipsoidal heights to AHD, including the model's uncertainty values.
  - [x] Decide between our own grid reader and PROJ (D-011): our own reader, as D-011 proposes. The decision awaits confirmation.
- [x] Projections: MGA2020 and MGA94 (Transverse Mercator via the Krüger series), point scale factor, grid convergence and combined scale factor.
- [x] Australian jurisdiction pack v0 (`packs/au`): frames, transformations, and grid references with URLs, SHA-256 hashes and whether browsers can download them directly.
- [x] Tests:
  - [x] Property tests for round trips and covariance propagation (deterministic pseudo-random cases, so the same ones run natively and in the browser).
  - [x] Comparisons against GeodePy and PROJ on a published point set (`crates/datum/tests`), plus DynAdjust's coordinates for the 109 national CORS.
  - [x] GDA2020 Technical Manual worked examples.
  - [x] A golden hash of results, checked natively on every OS and in the browser (D-024).
  - [x] The same for the grid transformations, plus fuzzing of the grid readers.
- [x] User doc: "How datums and epochs work in this tool" ([guide](user/datums-and-epochs.md)).

**Exit criteria:**
- Analytic transformations agree with GeodePy and PROJ within 0.1 mm, and grid transformations within 1 mm.
- Every public position type carries a frame and an epoch.

**Depends on:** M0.

### M2 Adjustment engine [L]

**Goal:** a rigorous, explainable 3D least squares adjustment of GNSS observations.

- [x] Observation types:
  - [x] GNSS baseline: a 3D vector with a 3×3 covariance.
  - [x] GNSS baseline cluster: n vectors with a full 3n×3n covariance.
  - [x] Point cluster: positions with covariance, for CORS, AUSPOS or control constraints.
- [x] Stochastic model:
  - [x] Input covariances, an a priori variance factor, and group scale factors.
  - [x] Centring and height-measurement errors as occupation-level nuisance parameters with priors, so vectors that share a set-up are correctly correlated.
- [x] Datum definition:
  - [x] Minimally constrained, by fixing one station or using inner constraints.
  - [x] Constrained, with fixed or weighted control.
  - [x] An explicit frame and epoch for the adjustment.
- [x] Solver:
  - [x] Sparse normal equations in 3×3 blocks with a fill-reducing (minimum-degree) ordering and our own block `L D Lᵀ` factorisation, which gives the same bits on every platform (D-025). It's fast enough at every size, so there's no separate dense path.
  - [x] Parameter covariance by selected inverse (Takahashi) at every size: each mark's block and every pair that shares an observation. Other pairs, for RU, cost three solves each.
- [x] Diagnostics: detect disconnected sub-networks, an undefined datum and near-singular geometry, with errors that name the marks involved.
- [x] Statistics:
  - [x] Residuals, the a posteriori variance factor and the global test (two-tailed χ²).
  - [x] Redundancy numbers and standardised residuals.
  - [x] w-tests in local ENU per component, plus a 3-DoF test per vector, with the tau test as an option.
- [x] Reliability:
  - [x] Minimal detectable bias (MDB) per observation, using Baarda's B-method with configurable α₀ and β₀.
  - [x] External reliability: the effect of an undetected MDB on coordinates (the observation's own marks).
- [x] Variance component estimation by group (session, engine, receiver type), iterated, with convergence reporting.
- [x] Uncertainties:
  - [x] Standard and 95% error ellipses and ellipsoids.
  - [x] SP1 v2.2 positional uncertainty (PU) and relative uncertainty (RU) between any pair, following the ICSM *Guideline for Adjustment and Evaluation of Survey Control*, scaled by the a priori or a posteriori variance factor as the project chooses.
- [x] Loop closures (fundamental cycles from a spanning tree) and repeat-baseline comparisons, with misclosure tests and ppm.
- [x] Outlier workflow:
  - [x] Rank the suspects.
  - [x] Exclude or re-weight with an audit trail.
  - [x] Nothing happens automatically without a record.
- [x] Results model: adjusted coordinates with covariance, residuals and statistics, ready for reporting. Each adjusted mark is also a datum-crate `Position` in the network's frame and epoch, with its covariance, so it can be transformed or projected.
- [x] Tests:
  - [x] Synthetic networks with known truth (`gnss_adjust::synthetic`), and a textbook dense adjustment as an oracle in every datum.
  - [x] Blunder injection, measuring detection power against MDB theory.
  - [x] Invariance: test statistics must not depend on which minimal constraint is chosen.
  - [x] A golden hash of results, checked natively on every OS and in the browser (D-024).
  - [x] DynAdjust oracle comparisons (`tests/dynadjust.rs`, made by `tests/oracle/dynadjust.py`).

**Exit criteria:**
- The adjustment matches DynAdjust on the oracle networks within 0.1 mm for coordinates and 0.1% for the variance factor, and flags the same outliers.
- Detection power on synthetic blunders is consistent with theory.
- A network of 1,000 marks and 3,000 vectors adjusts in under 2 s natively on a laptop, and within the web target in the browser ([ARCHITECTURE.md](ARCHITECTURE.md#20-performance-targets)).

**Depends on:** M0, M1.

### M3 Formats and CLI [M]

**Goal:** get vectors in and results out, in the browser and the CLI, before the full app exists.

- [x] Import vectors:
  - [x] DynaML: GNSS baselines (type G) and baseline clusters (type X).
  - [x] STAR*NET GPS vector files.
  - [x] TDEF (Trimble Data Exchange Format), which is what Trimble Business Center actually exports, found while writing the export recipes. It's checked on two real TBC exports.
  - [x] RTKLIB `.pos` files: the static single solution, recovering the signed covariance terms.
  - [x] A documented generic CSV vector format.
- [x] Import control:
  - [x] DynaML stations and point clusters (type Y).
  - [x] CSV with frame, epoch and uncertainty.
  - [x] AUSPOS SINEX: coordinates with covariance.
- [x] Document export recipes from TBC, Infinity, MAGNET, CHC Geomatics Office and Emlid Studio: which of our formats each can produce, and how (`docs/user/export-recipes.md`). Only TBC (through TDEF) and Emlid Studio (RTKLIB) are read directly; the others go through STAR*NET, PynAdjust or CSV.
- [x] Export:
  - [x] DynaML, for DynAdjust cross-checks and agency submission.
  - [x] STAR*NET.
  - [x] CSV.
  - [x] GeoPackage: marks, vectors and error ellipses.
  - [x] LandXML control points.
- [x] CLI commands: `init`, `import`, `adjust`, `report`, `export`, and `exclude` for the outlier workflow.
  - [x] A `--json` output mode.
  - [x] A non-zero exit code when statistical tests fail, so the CLI can be used in scripts and CI.
- [x] A minimal web page: load vectors and control, run the adjustment and view the report, all in the browser. It builds on the M0 web skeleton; the full app comes in M11.
- [x] Check that GeoPackage export works in the browser. The writer writes the SQLite file format itself (D-026), so it needs no SQLite build, and the export tests run in headless Chromium.
- [x] HTML report v0:
  - [x] Network summary, statistics and outlier table.
  - [x] Coordinates with PU, the RU table and loop closures.
  - [x] Provenance.
- [x] Round-trip tests for every format, and fuzz targets for every parser.

**Exit criteria:** someone with vectors from a commercial package (exported in a supported format or CSV) can run a second-opinion adjustment and get a report, in the browser or with the CLI.

**Depends on:** M2.

### M4 v0.1 release [S]

**Goal:** the second-opinion tool in real users' hands.

- [ ] Oracle suite in CI: DynAdjust outputs stored as golden files, plus a manual job to regenerate them.
- [ ] Documentation: a user guide for the second-opinion workflow in the browser and the CLI, and a worked example on public data.
- [ ] Deploy the web page at a versioned URL, and publish its static bundle with checksums.
- [ ] Release CLI binaries for Windows, macOS and Linux, with checksums and an SBOM. Build with cargo-dist or a plain Actions matrix, and generate the SBOM with cargo-cyclonedx.
- [ ] Announce to pilot users and partners, and collect feedback as issues.

**Exit criteria:** v0.1 is tagged, and at least three external users have run it on real data.

**Depends on:** M3.

---

## Phase 1: Full static workflow (v0.3, v0.5 beta, v1.0)

The goal is to go from RINEX and field logs to an adjusted, reported control survey in a web app, with blunder checks and a published benchmark.

### M5 Project model and pipeline [L]

**Goal:** an occupation-first project folder and a cached, reproducible build graph.

- [ ] Domain model:
  - [ ] Mark, Receiver and Antenna (IGS code, radome, serial, calibration source).
  - [ ] HeightMeasurement (method, value, measure-up point, measured by, photos).
  - [ ] Occupation, Session (derived), Baseline, Observation and AdjustmentConfig.
- [ ] Project folder format per [ARCHITECTURE.md](ARCHITECTURE.md#6-project-format):
  - [ ] TOML, with one file per occupation.
  - [ ] An immutable `raw/` directory with hashes.
  - [ ] Overlays for corrections.
  - [ ] JSON Schema for every file.
  - [ ] Schema versioning and migrations.
- [ ] Antenna geometry catalogue (`data/antennas`): ARP, measure-up points, and the radius and offsets needed for slant-height conversion, with sources cited. This information isn't in ANTEX. Document how to contribute to it.
- [ ] Pipeline:
  - [ ] A DAG of steps: ingest → QC → baseline → vector set → adjustment → evaluation → report.
  - [ ] Node keys = hash(inputs, settings, tool versions), with a content-addressed cache.
  - [ ] Parallel execution, cancellation and progress events, with an executor for each host: tokio and rayon natively, and workers in the browser.
- [ ] Storage interface (`store`): the file system natively; the File System Access API and OPFS in the browser, with zip import and export.
- [ ] Provenance:
  - [ ] An append-only JSONL record of every run: inputs, settings, versions, outputs and timing.
  - [ ] A `verify` command that re-runs everything and compares hashes.
- [ ] Command API (`api` crate): typed requests and responses for every operation, plus a job model for long-running work.

**Exit criteria:**
- Changing one antenna height re-runs only the dependent nodes.
- `verify` reproduces a project bit for bit with the same build: the web build on any operating system, and the native CLI on the same platform.

**Depends on:** M4.

### M6 RINEX ingest and QC [M]

**Goal:** trustworthy observation data and the metadata to check it.

- [ ] Integrate the `rinex` crate (nav-solutions; pin the version and confirm its licence) for RINEX 2.11, 3.0x and 4.0x, including Hatanaka compression. Wrap it behind our own trait so the parser can be swapped.
- [ ] Header extraction: marker, receiver, antenna and radome, antenna deltas (H/E/N), approximate position, interval, time span and observation types.
- [ ] Occupation detection: split and merge files, and detect gaps and multiple occupations within one file.
- [ ] Header reconciliation against field logs (this feeds M9). Write corrected RINEX on export, and never overwrite raw files.
- [ ] QC metrics:
  - [ ] Observations per satellite and signal, and completeness.
  - [ ] Cycle slips (geometry-free and Melbourne-Wübbena).
  - [ ] Multipath (code minus carrier).
  - [ ] SNR statistics, elevation coverage and data gaps.
  - [ ] Check all of these against Anubis and teqc-style reference outputs.
- [ ] Read large files in slices rather than whole, so a 24 h, 1 Hz file ingests in the browser within its target.
- [ ] Open receiver formats (UBX, SBF, Unicore, NovAtel, RTCM3): decide between running `convbin` from RTKLIB-EX, as another WebAssembly module, and our own Rust decoders (D-020).

**Exit criteria:**
- The benchmark corpus ingests with QC metrics within tolerance of the reference tools.
- Fuzzed malformed files never crash the app.

**Depends on:** M5.

### M7 Data fetch [M]

**Goal:** no more data chasing.

- [ ] Source registry, driven by the jurisdiction pack:
  - [ ] GA GNSS Data Repository (Web API, S3 and SFTP) for CORS RINEX and site logs.
  - [ ] State CORS networks, where they're openly available.
  - [ ] IGS product mirrors that don't need a login.
  - [ ] ANTEX (igs20.atx).
- [ ] Nearest-CORS suggestions ranked by distance and by data availability in the occupation window.
- [ ] Products: broadcast navigation, plus precise orbits and clocks (final, rapid or ultra-rapid, chosen by latency). Record product versions in provenance.
- [ ] Downloads:
  - [ ] A cache with checksums: in the user's cache directory natively, and in OPFS in the web app.
  - [ ] Resumable downloads and proxy support.
  - [ ] Offline data packs: export and import a bundle for air-gapped machines.
- [ ] Browser access ([details](ARCHITECTURE.md#13-data-fetch)):
  - [ ] Record for each source whether the web app can download from it directly. GA's Web API and archive downloads can; `files.igs.org` and the AUSGeoid2020 grid host can't (checked 2026-10-15).
  - [ ] Relay (`apps/relay`): optional and off by default, stateless, limited to pack-declared hosts, and with no logs. Self-hostable, and built into `gnss serve`. The app explains what the relay can see before it's first used.
  - [ ] Confirm whether ANTEX and the AUSGeoid2020 grid may be redistributed with the app, so the Australian v1 workflow needs no relay.
- [ ] Use site logs to check CORS antenna and receiver history against RINEX headers.
- [ ] Respect data licences, and carry attribution (for example CC BY for GA data) into reports.

**Exit criteria:**
- Given a set of occupations, one command or one click fetches everything needed.
- It works behind a proxy, and fully offline with a data pack.
- The Australian v1 workflow works in the browser without the relay.

**Depends on:** M5.

### M8 RTKLIB-EX baseline processing [L]

**Goal:** reliable static baselines with honest quality information.

- [ ] Module build:
  - [x] Pin an RTKLIB-EX release (done in M0: v2.5.1).
  - [x] Build `rnx2rtkp` from source in CI as a single WASI module, using the build from the M0 spike (done in M0: `engine-modules/rtklib-ex`).
  - [ ] Publish its checksum, and record the version and hash in provenance.
- [ ] Config generation from project settings ([details](ARCHITECTURE.md#93-rtklib-ex-module)):
  - [ ] Static mode with a combined solution.
  - [ ] Frequencies and constellations.
  - [ ] Elevation and SNR masks.
  - [ ] Ionosphere and troposphere options chosen by baseline length.
  - [ ] Ambiguity resolution mode and thresholds.
  - [ ] Antenna types, deltas and ANTEX.
  - [ ] The base position from known coordinates.
- [ ] Module hosts: a pool of engine workers in the browser and of wasmtime instances natively, with a fresh instance per run, timeouts, stderr capture and trap handling. Size the browser pool by memory as well as cores (about 90 MB per 2-hour run, 228 MB for a whole day at 30 s).
- [ ] Before each run, trim each RINEX file to the overlap window and decimate it to the processing interval. This keeps runs within WebAssembly's memory limit and roughly halves their time in the browser (M0 spike).
- [ ] Check that the navigation data covers every constellation being processed, and refuse to run otherwise. In the M0 spike, per-constellation navigation files silently degraded solutions.
- [ ] Output parsing:
  - [ ] The final solution and its covariance, including the signed off-diagonal terms.
  - [ ] Q flag, ratio and satellite count.
  - [ ] Residuals from `.stat` for the inspector.
  - [ ] Decide whether to patch RTKLIB's output precision. It writes standard deviations and covariances to 0.1 mm, which loses information on short baselines.
- [ ] Forward/backward agreement check. RTKLIB-EX doesn't validate this in static mode, so the driver runs forward-only and backward-only solutions and compares them.
- [ ] Baseline selection:
  - [ ] Independent baselines per session, using a minimum spanning tree by length or radial from CORS.
  - [ ] Exclude trivial baselines.
  - [ ] Allow user overrides.
- [ ] Base position seeding: use the best-known coordinates (control, CORS, PPP or AUSPOS, or an iterated solution). Warn when the base is poorly known.
- [ ] Retry strategies: documented variations (elevation mask, satellite exclusion, time window) when a baseline fails to fix. Record every attempt in provenance.
- [ ] Baseline quality model:
  - [ ] Inputs: fix status, ratio, percentage fixed, residual RMS, data overlap and length.
  - [ ] Classification as good, suspect or failed, with reasons.
- [ ] CLI `process` command. With this, v0.3 is end to end in the CLI.

**Exit criteria:**
- The CORS-pair benchmark subset processes at the expected fix rates.
- Results feed the adjustment with covariances.
- A failed baseline is explained, not just flagged.
- The web app and the CLI give identical engine outputs for the same inputs.

**Depends on:** M6, M7.

### M9 Blunder checks [M]

**Goal:** catch the errors that actually wreck surveys.

- [ ] Rule framework: each check has an ID, severity, explanation, evidence, a suggested fix and links to the affected objects. Suppressions are recorded with a reason, who made them and when.
- [ ] Antenna height:
  - [ ] Field log against the RINEX header.
  - [ ] Slant-to-vertical conversion.
  - [ ] Implausible values.
  - [ ] The same mark with different heights across sessions.
- [ ] Antenna type:
  - [ ] Field log against the header.
  - [ ] Validity of the IGS code and radome.
  - [ ] Antennas missing from ANTEX, with the expected bias.
  - [ ] Mixed antenna types on a baseline without calibrations.
- [ ] Mark identity:
  - [ ] The same ID with positions metres apart (a swap or typo).
  - [ ] Different IDs within a few centimetres (probably the same mark).
  - [ ] Approximate position against known coordinates.
- [ ] Session and timing:
  - [ ] Insufficient overlap.
  - [ ] Occupations too short for the baseline length.
  - [ ] Clock and time-system anomalies.
  - [ ] Duplicate files.
- [ ] Network:
  - [ ] Weak geometry (MDB above the project spec).
  - [ ] Marks connected by only one vector.
  - [ ] No redundancy.
  - [ ] Control not connected.
- [ ] Blunder corpus tests: inject each error type into benchmark projects, and measure detection and false-alarm rates.

**Exit criteria:**
- Detection rates are published for each blunder type.
- False alarms are low enough that pilot users don't switch the checks off.

**Depends on:** M6. The network checks also depend on M2.

### M10 Engine benchmark v1 [L]

**Goal:** public evidence of how well the engine works, and where it stops working.

- [ ] Dataset manifests for CORS-pair baselines across a grid of length × session window × conditions ([VALIDATION.md](VALIDATION.md#3-datasets)).
- [ ] Truth propagation: published coordinates and velocities brought to the observation epoch, excluding equipment changes and discontinuities.
- [ ] Runner: process, compare with truth, and compute fix rate, wrong-fix rate, error distributions and covariance realism. It runs in the CLI, which uses the same engine modules as the web app, so the results apply to both.
- [ ] Nightly CI job with a data cache. Publish a results dashboard on GitHub Pages, with regression diffs between runs.
- [ ] Call for low-cost receiver datasets, with a contribution guide, a CC BY 4.0 licence and a metadata template.
- [ ] Derive the minimum occupation needed for a 95% fix rate in each length bin. This feeds the guidance in M9 and M15.

**Exit criteria:**
- The first public benchmark report is out.
- v1.0 release gates are agreed with at least one partner.

**Depends on:** M8.

### M11 Web app [XL]

**Goal:** the full workflow in an app surveyors want to use, with nothing to install.

- [ ] Web app shell: the Svelte and TypeScript UI on the main thread, the core in a worker, and typed messages generated from the command API (specta).
- [ ] Project explorer and import:
  - [ ] Open a project folder (Chrome, Edge), or import and export a zip (Firefox, Safari).
  - [ ] Drag and drop RINEX, field logs and vectors.
- [ ] Map (MapLibre GL):
  - [ ] Marks, occupations and baselines coloured by quality.
  - [ ] Error ellipses, loop closures and CORS.
  - [ ] Configurable online basemaps, and offline PMTiles basemaps.
- [ ] Timeline: occupations per receiver across sessions as a Gantt chart. Overlaps show the candidate baselines.
- [ ] Baseline inspector:
  - [ ] Residuals by satellite (uPlot).
  - [ ] A sky plot with slips and SNR.
  - [ ] Solution summary and retry history.
- [ ] Issues panel: every blunder check, with fixes and suppressions.
- [ ] Adjustment dashboard:
  - [ ] Global test and w-test ranking.
  - [ ] Reliability (MDB) on the map.
  - [ ] Variance component factors.
  - [ ] PU and RU against the project spec.
- [ ] Datum panel: show the working for every transformation.
- [ ] Jobs: progress, cancel and logs.
- [ ] Offline and installable: a service worker per version, an install prompt, persistent storage and a storage usage display.
- [ ] Accessibility (keyboard navigation, contrast, screen reader labels), with light, dark and high-contrast outdoor themes.
- [ ] End-to-end tests with Playwright in Chromium, Firefox and WebKit.
- [ ] Usability sessions with at least five surveyors, then iterate. The v0.5 beta ships partway through this milestone.

**Exit criteria:** pilot users complete the reference project end to end in the browser without help.

**Depends on:** M5, M8, M9.

### M12 Reporting and standards [M]

**Goal:** reports that clients and agencies accept.

- [ ] Standards evaluation (SP1 v2.2): PU and RU at 95% confidence, compared against the project specification. Support jurisdiction-specific measures through packs, such as Local Uncertainty where it's still published (for example in SCIMS).
- [ ] Control survey report template:
  - [ ] Summary, equipment and sessions.
  - [ ] Processing settings and QA statistics.
  - [ ] Outliers and actions taken.
  - [ ] Adjusted coordinates (GDA2020, MGA2020 and AHD) with PU, plus the RU table.
  - [ ] Loop closures and the datum working.
  - [ ] Provenance and data attributions.
- [ ] Output HTML as the primary format, plus PDF (D-012).
- [ ] Exports: DynaML for agency submission, LandXML, GeoPackage, CSV, and the corrected RINEX set.
- [ ] Review the templates with a state agency and two practising surveyors.

**Exit criteria:** for a pilot project, a client or Surveyor-General's office accepts the report as is.

**Depends on:** M3. It's finalised alongside M11.

### M13 v1.0 hardening [L]

**Goal:** production quality.

- [ ] Web release: an immutable deployment at a versioned URL, and a release bundle with checksums, an SBOM and a Sigstore signature.
- [ ] `gnss serve`, for offline and air-gapped use of the bundle.
- [ ] Version handling: announce new versions in the app, switch only on reload and never mid-job, and offer to open a project in the version that last processed it.
- [ ] Sign the CLI binaries where practical: SignPath for Windows, notarisation for macOS.
- [ ] Security:
  - [ ] Fuzzing in CI.
  - [ ] SBOM, cargo-deny and cargo-audit, plus licence and advisory checks for JavaScript packages.
  - [ ] A threat model doc and a strict Content Security Policy.
  - [ ] Confirm there's no telemetry and no analytics scripts.
- [ ] Performance targets met, native and web ([ARCHITECTURE.md](ARCHITECTURE.md#20-performance-targets)).
- [ ] Documentation:
  - [ ] User guide.
  - [ ] Concepts (datums, statistics, reliability).
  - [ ] Troubleshooting.
  - [ ] Worked examples.
- [ ] Benchmark release gates met, with the release report published.
- [ ] Sign-off from at least three pilot organisations.

**Exit criteria:** v1.0 is tagged.

**Depends on:** M10, M11, M12.

---

## Phase 2: Field and reach (v1.x)

The goal is to remove field-to-office friction, cover long baselines, and meet users where they already work. Milestones in this phase can run in parallel.

### M14 Field capture [L]

- [ ] Field mode in the web app (D-023), installable on phones and tablets.
- [ ] Occupation log form:
  - [ ] Mark search, with nearby suggestions.
  - [ ] Receiver.
  - [ ] Antenna, picked from IGS codes.
  - [ ] Height method and value, with a photo of the tape.
  - [ ] Notes and operator.
- [ ] Rough position from the phone's GNSS, for the mark-identity checks.
- [ ] Offline-first storage, with bundle export and import. Safari can evict storage for sites that aren't installed, so field mode asks users to install it and exports a bundle after each occupation.
- [ ] Sync into the office project, with conflict handling.

**Exit criteria:** field logs flow into projects without retyping, and the height, antenna and mark checks run on import.

### M15 In-field occupation check [M]

- [ ] On a field laptop, process the current occupation against the nearest CORS when online, or with PPP otherwise (Ginan through the CLI, M16). Report whether there's enough data, with fix status and estimated precision.
- [ ] Recommend minimum occupations by baseline length and conditions, based on the benchmark (M10).

**Exit criteria:** field crews can decide to pack up based on evidence, which saves return trips to remote marks.

### M16 Ginan integration [L]

- [ ] Engine driver for Ginan's `pea`: configuration, products (orbits, clocks, biases) and outputs.
- [ ] Run `pea` natively in the CLI and server, and let the web app hand jobs to them. Assess whether a WebAssembly build is practical.
- [ ] Single-site PPP workflow, an offline equivalent of AUSPOS.
- [ ] Long-baseline support through PPP-derived constraints.
- [ ] Benchmark long baselines and single-site PPP against truth.

**Exit criteria:** long-baseline and single-site results are validated, and available through the CLI and server, and from the web app when it's connected to either.

### M17 Python and ArcGIS Pro [M]

- [ ] PyO3 bindings built with maturin: abi3 wheels for Windows, macOS and Linux, mirroring the command API.
- [ ] ArcGIS Pro Python toolbox (.pyt):
  - [ ] Run the workflow.
  - [ ] Write marks, vectors and error ellipses to a file geodatabase with arcpy.
  - [ ] Apply symbology.
- [ ] Optional publishing to ArcGIS Online or Enterprise through the REST API (`applyEdits`).
- [ ] Examples: a Jupyter notebook and a Pro project template.

**Exit criteria:** a Pro user runs the workflow from a toolbox and gets feature classes.

### M18 Server and MCP [M]

- [ ] An axum HTTP server exposing the command API, with authentication and multiple projects.
- [ ] Agency deployments: the static web app hosted internally, with the server for headless, batch and native-only jobs.
- [ ] An MCP server with read and analysis tools (explain outliers, summarise QA, propose fixes). Write actions require explicit approval.

**Exit criteria:** headless deployment and agent access both work against the same core.

### M19 More jurisdictions [M]

- [ ] Document the pack format.
- [ ] New Zealand pack: NZGD2000 with its deformation model, NZVD2016 heights, LINZ standards and data sources.
- [ ] Pacific packs, built with partner agencies.
- [ ] A generic ITRF-only pack for use anywhere.

**Exit criteria:** at least one pack other than Australia's is used on real projects.

---

## Phase 3: Engine and breadth (v2.0)

The goal is to go beyond RTKLIB rather than re-implement it, and to broaden what can be adjusted.

### M20 Rust batch double-difference engine [XL]

- [ ] Build it as an engine module, like RTKLIB-EX, so it runs the same way in the browser and the CLI.
- [ ] Observation modelling:
  - [ ] Code and phase for GPS, Galileo and BeiDou (GLONASS later), multi-frequency.
  - [ ] PCO and PCV from ANTEX.
  - [ ] Troposphere: an a priori model plus ZTD estimation with a mapping function.
  - [ ] Solid earth tides.
  - [ ] Correctly correlated double differences.
- [ ] Cycle-slip detection and repair (geometry-free, Melbourne-Wübbena, residual-based).
- [ ] A batch float solution over the whole session, with an ionosphere strategy chosen by baseline length: ignore, weighted or ionosphere-free.
- [ ] Integer ambiguity resolution:
  - [ ] LAMBDA/MLAMBDA, with partial ambiguity resolution.
  - [ ] Validation by ratio test or FFRT.
  - [ ] Wide-lane/narrow-lane for longer baselines.
- [ ] Realistic covariances: handle temporal correlation, or apply empirical scaling learned from the benchmark.
- [ ] Validate against RTKLIB-EX and the benchmark.

**Exit criteria:** the engine matches or beats RTKLIB-EX on every benchmark gate, especially the wrong-fix rate, before it becomes the default.

### M21 Multi-baseline session processing [L]

- [ ] Process all receivers in a session simultaneously, and output baseline clusters with full covariance. M2 already supports clusters.

**Exit criteria:** correlated session solutions improve covariance realism on the benchmark.

### M22 Dual-engine cross-validation [S]

- [ ] Process baselines with both RTKLIB-EX and the Rust engine, and flag disagreements larger than the combined uncertainty.

**Exit criteria:** the disagreement check is on by default.

### M23 Terrestrial observations [L]

- [ ] Levelling (height differences, with geoid and AHD handling).
- [ ] Total station: distances, directions and zenith angles, with refraction and instrument heights.
- [ ] Combined adjustment.
- [ ] Imports: DynaML terrestrial types and common data collector exports.

**Exit criteria:** combined GNSS, levelling and total station networks match DynAdjust on oracle sets.

### M24 Monitoring [L]

- [ ] Continuous processing of permanent low-cost stations (hourly and daily solutions), in the CLI or server.
- [ ] Time series, change detection and alert thresholds.
- [ ] Reports for monitoring programs, such as tailings dams and landslides.

**Exit criteria:** a pilot monitoring deployment runs for three months.

---

## Every release

- [ ] Changelog and upgrade notes, including project schema migrations.
- [ ] Web app deployed at a new versioned URL, and its bundle published. Earlier versions stay up.
- [ ] Benchmark report published, with regressions explained.
- [ ] Security review of new parsers and network calls.
- [ ] Accessibility check of new UI.
- [ ] Docs updated for every user-visible change.
