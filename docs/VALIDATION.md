# Validation and benchmark

Surveyors carry professional liability, so they'll only adopt a tool they can see is right. This document defines how we prove it: what we test, against which truth, with which metrics, and which gates a release must pass. It's a specification. The benchmark itself is built in milestones M2, M4 and M10 ([ROADMAP.md](ROADMAP.md)).

## 1. Principles

- **Known truth, not plausibility.** Every metric compares against an independent truth or a reference implementation, never against our own earlier output alone.
- **Publish everything.** Every bin, including the bad ones, is published every release. No cherry-picking.
- **Reproducible by anyone.** One command and a public manifest reproduce the full benchmark.
- **Gates block releases.** A regression beyond its tolerance blocks a release unless it's documented and accepted.
- **Validate the envelope, not just the middle.** Short sessions, long baselines, disturbed ionosphere and low-cost antennas are where tools fail, so that's where we look hardest.

## 2. What we validate

| Component | Truth or oracle | Datasets | Key metrics |
|---|---|---|---|
| Datums and geodesy | GeodePy, PROJ, GDA2020 Technical Manual examples | D5 | Max absolute difference by transformation type |
| Adjustment | DynAdjust, synthetic truth | D3, D4 | Coordinate agreement, variance factor, flagged observations, detection power |
| Baseline engines | CORS coordinates and velocities, known marks, zero baselines | D1, D2 | Fix rate, wrong-fix rate, error distributions, covariance realism |
| Blunder checks | Injected errors with known answers | D6 | Detection rate and false-alarm rate per check |
| End to end | Reference projects with expected outputs | D7 | Report equality, reproducibility |

## 3. Datasets

### D1 CORS-pair baselines (engines)

Baselines formed between continuously operating reference stations whose coordinates are known to the millimetre. Daily files can be cut into any session window, which makes this the backbone of the engine benchmark.

- **Source:** the GA GNSS Data Repository (daily 30 s RINEX from all stations, and 1 s hourly data from a subset).
- **Stations:**
  - ARGN, AuScope and openly licensed state CORS with published coordinates and velocities.
  - Stable time series.
  - No equipment changes in the window (checked against site logs).
- **Grid of conditions:**

  | Dimension | Values |
  |---|---|
  | Length | 0 to 1, 1 to 5, 5 to 10, 10 to 20, 20 to 50, 50 to 100, 100 to 300 km |
  | Session window | 15 min, 30 min, 1 h, 2 h, 4 h, 8 h, 24 h |
  | Ionosphere | Quiet and disturbed days, including the 2024 to 2025 solar maximum |
  | Time of day and season | Day and night, all seasons |
  | Latitude | Tropical north to Tasmania |
  | Site | Including sites with known multipath |

- **Target:** at least 100 samples per length × window cell where the station geometry allows. The shortest bins rely on co-located stations and on D2.

### D2 Low-cost receiver datasets (engines)

- **Contributed** observations from UM980, ZED-F9P, mosaic-X5 class and similar receivers, on:
  - marks with known coordinates and published uncertainty (for example SCIMS marks)
  - sites co-located with CORS
- **Zero-baseline** (antenna splitter) and very-short-baseline set-ups, to separate receiver noise from everything else.
- **Metadata template:** receiver, firmware, antenna and radome, cable, height and method, mark, photos.
- **Licence:** CC BY 4.0. Contributors confirm they have the right to share the data and that no location is security-sensitive.

### D3 Synthetic networks (adjustment)

- **The generator controls:**
  - network geometry: radial, braced, chain and mixed
  - number of marks and sessions
  - control configuration
  - covariance realism (sampled from D1 statistics)
- **Injected blunders** of known size, type and location, which test detection power against MDB theory.
- **The generator itself** is versioned and seeded, so every network can be regenerated exactly.

The first generator is in place (`gnss_adjust::synthetic`, M2). It scatters marks over a disc, joins each to its nearest neighbours by vectors, measures some vectors together from a common base so they're correlated, and adds weighted control. Noise is drawn from each observation's own covariance, so the statistics should follow theory exactly. Geometry types and covariance realism from D1 come later. The tests in `crates/adjust/tests` found:

| Check | Result |
|---|---|
| Against a textbook dense adjustment, in each datum (weighted control, weighted plus fixed control, one fixed mark, inner constraints) | Positions within 1e-8 m. w, MDB, redundancy, vector test and external reliability within 1e-7 relative |
| Invariance between two fixed marks and inner constraints | Residuals within 1e-10 m, test statistics within 1e-8 relative, and positions that differ only by a translation |
| σ̂₀² over 200 noise draws (r = 138) | Mean 1.013, with a standard error of 0.0085 |
| Global test at 5% | Passed 194 of 200 times (190 expected) |
| 45,600 w-tests without blunders | Mean 0.002, variance 1.013, and 47 false alarms at α₀ = 0.1% (45.6 expected) |
| 15,200 vector tests without blunders | 87 false alarms at the B-method's 0.55% (83.6 expected) |
| A bias of one MDB, over 400 draws (G-POW) | Detected by the w-test 79.8% of the time and by the vector test 80.3% (β₀ = 80%) |
| Coordinate errors against their covariances (PU), over 4,000 draws | Mean dᵀΣ⁻¹d of 2.981 ± 0.024 (3 expected) |
| Relative errors of all 91 pairs against their covariances (RU), over 4,000 draws | Mean 2.989 ± 0.015 (3 expected) |
| Horizontal errors inside the 95% PU circle (ICSM's formula), over 4,000 draws | 95.06% |
| Every block of Qₓ, including pairs that share no observation, against the dense adjustment | Within 1e-14 m² in each datum |
| Critical values against SciPy | Within 1e-12 relative (B-method critical values within 1e-10) |
| Variance component estimation with one group's covariances four times too optimistic, over 200 draws | Estimated factors average 1.009 ± 0.013 and 0.990 ± 0.010 of the truth, in 9 to 24 rounds |
| Loop closures: 34 loops in a network of clusters, over 1,000 draws | Mean statistic 2.973 ± 0.020 (3 expected); 0.553% fail at the 0.55% level |
| A good baseline excluded, over 10,000 draws each for four baselines | Mean misclosure test 2.98 to 3.02, ± 0.025 (3 expected) |
| The same, against its vector test with it in | Equal to 1e-7 relative, the rounding of ECEF coordinates |
| A 10 cm blunder | First among the suspects; excluded, its misclosure shows the 10 cm and fails, and the global test then passes |
| Set-ups against the dense adjustment, in each datum | Errors within 1e-9 m; their priors' tests within 1e-7 relative |
| A network with 88 set-ups, their errors drawn from their priors, over 1,000 draws | σ̂₀² 1.0009 ± 0.0042; 184 false alarms in 189,000 vector w-tests and 264 in 264,000 prior w-tests (189 and 264 expected); loop statistics 3.020 ± 0.025 |
| A set-up's height wrong by three of its MDBs | The set-up's prior is first among the suspects, with the error estimated upward |

### D4 Oracle networks (adjustment)

- D3 networks and real (anonymised) networks run through DynAdjust.
- Outputs are stored as golden files, with the DynAdjust version and settings recorded.
- STAR*NET comparisons are made by partners who hold licences, then published manually. Commercial software can't run in CI.

The first three synthetic networks are in place (`crates/adjust/tests/dynadjust.rs`), with DynAdjust 1.4.1's results made by `tests/oracle/dynadjust.py`. They have baselines, clusters, weighted control (point clusters), a fixed mark, and injected blunders of 2 to 6 cm. DynAdjust flags a component when its N-stat, the residual over its own standard deviation, exceeds 1.96 (95%), so it's compared with our standardised residuals. Against G-ADJ:

| Network | Coordinates | σ̂₀² | PU | N-stat | Flagged by both |
|---|---|---|---|---|---|
| 30 marks, clusters of three vectors, 4 control marks | 4.7e-10 m | Within 1e-6 relative | 1.5e-9 relative | 1.2e-7 | The same 18 of 246 components |
| 25 marks, single baselines, 3 control marks and a fixed mark | 2.7e-7 m | Within 1e-6 relative | 3.7e-8 relative | 8.2e-6 | The same 10 of 147 |
| 120 marks over 50 km, pairs of vectors, 6 control marks | 6.6e-10 m | Within 1e-6 relative | 4.5e-9 relative | 2.1e-7 | The same 62 of 1,131 |

The degrees of freedom match exactly, and the global test's bounds to 1e-9.

Two things in DynAdjust need allowing for:
- **Free stations get a weak a priori constraint**, 10 m by default. The oracle sets it to 1 km, where it changes nothing at this precision.
- **Fixed stations are held only to 1 µm.** In the network with a fixed mark, the control moves the whole network by about 0.3 µm in DynAdjust, which accounts for that network's larger differences.

**DynAdjust's own sample network**, end to end (`crates/api/tests/dynaml.rs`, made by `tests/oracle/dynadjust_sample.py`): 43 marks, 129 ITRF2008 baselines and an ITRF2014 baseline cluster, adjusted in GDA2020 on 6 weighted control marks. It goes through the whole second-opinion path: reading DynaML, transforming the vectors to GDA2020 as DynAdjust's dnareftran does, building the network and adjusting it. Against DynAdjust 1.4.1:

| Check | Agreement |
|---|---|
| Vectors transformed to GDA2020 | 0.5 µm |
| Adjusted coordinates | 0.7 µm |
| N-statistics | 1.2e-4 |
| χ² (336.64 on 288 degrees of freedom) and flagged components | The same, and the same 10 components |

The same network, written out as DynaML, as STAR*NET with CSV control, and as CSV, reads back to bit-identical adjustments. Its exports and report are pinned by a hash (`crates/api/tests/project.rs`), checked natively and in headless Chromium, so the web page's downloads are byte for byte the CLI's.

**A real Trimble Business Center export** (`crates/api/tests/tbc.rs`): 31 vectors between 11 marks around Bright, Victoria, observed in December 2019, read from TBC's TDEF export. They're adjusted in GDA2020 on the five CORS held at the GDA2020 coordinates keyed into TBC. There's no oracle for the result, but it behaves as a second opinion should:
- σ̂₀² is 1.97 and the global test fails.
- The two vectors into one mark, 324900700, fail their w-tests and vector tests.
- Without those two, σ̂₀² is 1.26, the global test passes, and no test fails.
- Taking the vectors as GDA2020 rather than ITRF2014 at the survey's epoch changes the global test's statistic by 0.02, as expected so close to 2020.0.

### D5 Geodesy points (datums)

- The GDA2020 Technical Manual worked examples.
- A generated point grid across Australia at epochs from 1994 to 2030, covering every transformation and projection in the AU pack.
- Reference values come from GeodePy and PROJ, with versions recorded.
- DynAdjust's GDA2020 coordinates for the 109 Recognised Value Standard CORS (from GeodePy's test suite).

The analytic part is in place (`crates/datum/tests`, generated by `tests/oracle/generate.py`): 86 points over Australia and its offshore territories at six epochs from 1994.0 to 2031.0. Against the G-DAT threshold of 0.1 mm, the first results were:

| Comparison | Largest difference |
|---|---|
| Helmert and plate motion transformations vs PROJ (551 cases) | 0 (identical bits) |
| Transformations vs GeodePy, with covariance propagation | 0; covariances to 4e-16 relative |
| Geodetic and ECEF vs PROJ | 1.9 nm |
| MGA2020 vs PROJ, own and neighbouring zones | 2.4 nm; point scale 5e-11; convergence 6.5e-10° |
| MGA2020 vs GeodePy | 0.05 mm (GeodePy's rounding) |
| The 109 CORS vs DynAdjust | 0.19 mm XYZ, 0.36 mm MGA (the file's rounding) |
| GDA2020 Technical Manual examples (ALIC) | within the manual's 0.1 mm rounding |

The grids are tested the same way (`tests/grids.rs`, generated by `tests/oracle/generate_grids.py`). The published grids are too big to commit, so the tests use windows cut from them around 49 GDA94 to GDA2020 points and 33 AUSGeoid2020 points, as NTv2 and as GDAL-written GeoTIFF. CI also reads PROJ's published GeoTIFFs at 400 more points each, and the published NTv2 files were checked the same way before committing. Against the G-DAT threshold of 1 mm for grids:

| Comparison | Largest difference |
|---|---|
| GDA94 to GDA2020 grid vs PROJ, both directions | 0.07 µm (0.09 µm on the published grid) |
| The grid's shifts and accuracies vs GeodePy | within GeodePy's 1e-6" rounding |
| GDA94 to GDA2020 positions with heights (grid, then EPSG:8048 for heights) vs PROJ | 0.07 µm |
| AUSGeoid2020 N vs PROJ | 5e-11 m |
| AUSGeoid2020's uncertainty vs an independent interpolation (NumPy) | 1e-15 m |

The differences from PROJ come from PROJ rounding each node's shift to single precision in radians.

### D6 Blunder corpus (checks)

- D1, D2 and D7 projects with injected errors:
  - **antenna height:** ±1 cm to ±1 m, slant/vertical confusion, wrong measure-up point
  - **antenna type:** wrong model, missing radome
  - **mark IDs:** swapped or mistyped
  - **time:** wrong windows, duplicated files
- Clean copies of the same projects measure the false-alarm rate.

### D7 Reference projects (end to end)

- Complete projects (RINEX, field logs, control) with expected outputs and reports.
- At least one is fully public and doubles as the worked example in the docs.
- Pilot projects are included with permission, and are never published without consent.

## 4. Truth

- **CORS truth.** GA-published coordinates and velocities, propagated to the observation epoch. A vector's truth is the difference of the propagated positions. The truth's own uncertainty is carried into every comparison.
- **Exclusions.** Windows that span equipment changes (from site logs), known discontinuities (earthquakes, antenna changes) or poor time-series fits.
- **Known marks.** Published coordinates with uncertainty, or a long geodetic-grade occupation tied to CORS.
- **Zero baselines.** The truth is exactly zero.

## 5. Metrics

| Metric | Definition |
|---|---|
| Fix rate | Share of baselines whose final solution is fixed (Q=1), passes the ratio threshold, and passes our forward/backward agreement check |
| Wrong-fix rate | Share of *fixed* baselines whose horizontal or vertical error against truth exceeds max(4 × scaled formal σ, 3 cm). Proposed definition, to be finalised with partners. This is the most important engine metric. |
| Accuracy | Horizontal (2D) and vertical errors against truth: median, RMS and 95th percentile per bin, for fixed solutions and for all solutions |
| Covariance realism | Normalised errors (error / formal σ) per component. Reports the empirical scale factor (RMS of normalised errors) per bin, which sets default covariance scaling |
| Minimum occupation | Shortest session window that reaches a 95% fix rate in each length bin. This feeds in-field guidance and the overlap check. |
| Adjustment agreement | Max absolute coordinate difference against DynAdjust, relative variance factor difference, residual differences, and whether the same observations are flagged at the same α₀ and β₀ |
| Detection power | Share of injected blunders at or above MDB that are flagged, compared with the theoretical β₀ |
| Geodesy agreement | Max absolute difference against the references, per transformation type |
| Check performance | Detection rate (recall) on D6 and false-alarm rate on clean projects, per check |
| Reproducibility | Bitwise hash equality for the same build (the web build in any browser on any operating system, the native build on the same platform), and max coordinate difference between builds and platforms |
| Speed | Processing and adjustment time against the targets in [ARCHITECTURE.md](ARCHITECTURE.md#20-performance-targets) |

## 6. Release gates

The values below are proposals. They'll be revisited after the first full benchmark run (M10) and agreed with at least one partner before v1.0.

| Gate | Scope | Proposed threshold |
|---|---|---|
| G-DAT | D5 | Analytic transformations within 0.1 mm, grid transformations within 1 mm |
| G-ADJ | D4 | Coordinates within 0.1 mm, variance factor within 0.1%, identical flagged observations |
| G-POW | D3 | Detection power for blunders at MDB consistent with β₀ (within sampling error) |
| G-ENG-1 | D1: geodetic antennas, up to 20 km, sessions of 1 h or more | Fix rate at least 95%, wrong-fix rate at most 0.5%, 95th percentile error at most 10 mm horizontal and 25 mm vertical |
| G-ENG-2 | D2: low-cost receivers, up to 10 km, sessions of 1 h or more | Published from the first run; threshold set from those results |
| G-COV | D1 and D2 fixed baselines | Empirical scale factor published; defaults updated when it shifts |
| G-BLD | D6 | 100% detection by metadata checks where a field log exists; geometry-based checks at or above the theoretical power; false-alarm rate published |
| G-REP | D7 | The CLI on every platform and the web app in every browser give bit-identical outputs for the same inputs (D-024) |
| G-REG | All | No metric regresses beyond its tolerance against the previous release without a documented, accepted reason |

## 7. Running it

| Tier | When | Contents |
|---|---|---|
| Fast | Every PR | Unit, property and golden tests; D5; D4 oracle subset; an engine smoke set of about 20 cached baselines |
| Nightly | Scheduled | Full D1 and D2 engine benchmark from cache, D6 blunder corpus, D7 reference projects |
| Release | Tagged builds | Everything, plus reproducibility across browsers, platforms and builds, and publication |

- **One engine for both front ends.** The benchmark runs through the CLI, which runs the same engine modules as the web app (D-023), so the engine results apply to both.
- **Data cache.** The GA archive is also available from a public S3 bucket in Sydney, which makes CI fetches cheap. Nightly runs use a persistent cache, on either a self-hosted runner or object storage.
- **One-command reproduction:**

  ```
  gnss bench run --manifest benchmarks/cors-pairs-v1.toml --out bench-results/
  ```

- **Manifest format** (illustrative):

  ```toml
  [benchmark]
  id = "cors-pairs-v1"
  spec_version = 1
  truth = "ga-published-coordinates-and-velocities"

  [[baseline]]
  base = "XXXX00AUS"
  rover = "YYYY00AUS"
  start = "2025-03-02T00:00:00Z"
  window = "1h"
  tags = ["length:10-20km", "iono:disturbed", "site:multipath"]
  ```

## 8. Publishing

- **A dashboard on GitHub Pages**, updated nightly and frozen for each release, showing:
  - fix-rate heatmaps (length × window)
  - error CDFs per bin
  - covariance realism plots
  - check performance
  - trends over time
- **Machine-readable results** (JSON) alongside, with the manifest hash and the versions of the software, engine modules and products.
- **Each release report** lists its gate results and explains any accepted regressions.

## 9. Data licensing and ethics

- **GA data** is published under Creative Commons Attribution. GA's pages cite both CC BY 3.0 AU and CC BY 4.0, so confirm the version with GA. Attribution is carried into results and reports.
- **State CORS data** is included only where it's openly licensed.
- **Raw data** is fetched by the manifests rather than redistributed, unless the licence clearly allows redistribution.
- **Contributed data** is CC BY 4.0. Contributors confirm their right to share it.
- **Security-sensitive locations** are excluded entirely. Moving observation data isn't meaningful, so sensitive sites simply aren't used.

## 10. Governance

- The benchmark spec is versioned. Changes go through PRs that explain the rationale and the expected effect on results.
- Partners (GA, ICSM members, state agencies, universities) are invited to co-maintain the spec and the datasets.
- Independent reproduction is encouraged, and its results are linked from the dashboard.
