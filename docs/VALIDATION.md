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

### D4 Oracle networks (adjustment)

- D3 networks and real (anonymised) networks run through DynAdjust.
- Outputs are stored as golden files, with the DynAdjust version and settings recorded.
- STAR*NET comparisons are made by partners who hold licences, then published manually. Commercial software can't run in CI.

### D5 Geodesy points (datums)

- The GDA2020 Technical Manual worked examples.
- A generated point grid across Australia at epochs from 1994 to 2030, covering every transformation and projection in the AU pack.
- Reference values come from GeodePy and PROJ, with versions recorded.

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
| Reproducibility | Bitwise hash equality on the same platform, and max coordinate difference across platforms |
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
| G-REP | D7 | Bit-identical on the same platform; within 1e-6 m across platforms |
| G-REG | All | No metric regresses beyond its tolerance against the previous release without a documented, accepted reason |

## 7. Running it

| Tier | When | Contents |
|---|---|---|
| Fast | Every PR | Unit, property and golden tests; D5; D4 oracle subset; an engine smoke set of about 20 cached baselines |
| Nightly | Scheduled | Full D1 and D2 engine benchmark from cache, D6 blunder corpus, D7 reference projects |
| Release | Tagged builds | Everything, plus cross-platform reproducibility and publication |

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
- **Machine-readable results** (JSON) alongside, with the manifest hash and the versions of the software, sidecars and products.
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
