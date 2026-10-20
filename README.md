# GNSS (working title)

Free and open-source GNSS static survey processing: from field log to adjusted, defensible coordinates.

> **Status: the second-opinion adjustment (milestone M3) works, in the browser and from the command line.** Load GNSS vectors and control exported from a commercial package (Trimble's TDEF, DynaML, STAR*NET, RTKLIB, SINEX or CSV), adjust them in GDA2020 or any other frame, and get a report with the tests, suspects, SP1 uncertainties, loop closures and provenance, plus GeoPackage, CSV, LandXML, DynaML and STAR*NET exports ([how](docs/user/second-opinion.md)). Underneath are the datum core (M1), checked against PROJ, GeodePy and DynAdjust ([how it works](docs/user/datums-and-epochs.md)), and the adjustment engine (M2): baselines, clusters, control and set-ups in any datum, with the global test, w-tests, reliability, PU and RU, variance components, loop closures and a recorded outlier workflow. On DynAdjust's own sample network the whole path agrees with DynAdjust within a micrometre and flags the same outliers. Baseline processing from RINEX and the full app come next. Start with [docs/PLAN.md](docs/PLAN.md), then work from [docs/ROADMAP.md](docs/ROADMAP.md).

## The problem

Commercial GNSS office suites (Trimble Business Center, Leica Infinity and others) are capable but expensive, closed and tied to vendor ecosystems. Good open-source engines exist (RTKLIB-EX, Ginan), but nothing open covers the survey workflow around them.

The baseline maths is largely solved. What actually costs surveyors time and money is:

- metadata blunders (antenna heights, antenna types, mark IDs) that slip through
- QA that can't be defended ("fixed, ratio 3.2" isn't evidence)
- datum and epoch errors that are silent
- chasing CORS data, orbits and antenna calibrations across portals
- producing standards-compliant reports by hand
- results that can't be reproduced years later

## What this project will be

A web app that runs entirely in your browser, plus a CLI and library, that makes a static GNSS survey **correct by construction and defensible**:

- **Blunder detection** that reconciles field logs against RINEX headers and catches height, antenna and mark-ID errors before they reach the adjustment.
- **A rigorous least squares adjustment that explains itself**: minimally constrained and constrained solutions, variance component estimation, global test, w-tests, reliability, loop closures, and uncertainty reporting per ICSM SP1.
- **Datums by construction**: every coordinate carries its frame and epoch, and every transformation is explicit and shown.
- **No data chasing**: CORS RINEX, orbits, clocks and ANTEX are fetched and cached automatically, or loaded from an offline data pack.
- **Reproducible, offline projects**: a project is a folder that works in Git, processing is a cached build graph, and every result records exactly how it was produced.
- **Swappable engines**: RTKLIB-EX first, Ginan for long baselines and PPP, our own Rust engine later.
- **Validated in public**: a benchmark of baselines with known answers runs in CI and is published with every release.

## Who it's for

- Users of low-cost multi-band receivers (UM980 and ZED-F9P class) who have no vendor software
- Small survey firms, councils and Pacific land agencies that can't justify commercial seats
- Surveyors with commercial tools who want an independent second-opinion adjustment
- Universities teaching geodetic surveying
- Government and defence users who need offline, auditable processing

## Tech

The core is in Rust, which gives memory-safe parsing and plenty of speed, and compiles both to WebAssembly and to native code. The main app is a web app with a Svelte and MapLibre UI. It runs entirely in your browser: there's nothing to install, your survey data never leaves your machine, and it works offline once loaded. RTKLIB-EX runs inside it as a WebAssembly module. A native CLI runs the same core for scripting, batch work and the benchmark, and Python bindings support ArcGIS Pro. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and decision [D-023](docs/DECISIONS.md).

## Documents

| Document | Contents |
|---|---|
| [PLAN.md](docs/PLAN.md) | Vision, users, goals and non-goals, principles, release plan, partners, risks, success measures |
| [ROADMAP.md](docs/ROADMAP.md) | Phases, milestones, tasks, exit criteria and where to start |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Technical design: crates, data model, pipeline, engines, adjustment maths, datums, UI, packaging |
| [VALIDATION.md](docs/VALIDATION.md) | Benchmark datasets, metrics and release gates |
| [DECISIONS.md](docs/DECISIONS.md) | Decision log, including open decisions |
| [CONVENTIONS.md](docs/CONVENTIONS.md) | Coding conventions: units, frames, time, errors, numerics and host access |
| [spikes/m0-engine.md](docs/spikes/m0-engine.md) | The M0 engine spike: RTKLIB-EX as a WebAssembly module, compared with native builds |
| [user/second-opinion.md](docs/user/second-opinion.md) | For users: a second-opinion adjustment in the browser or with the CLI, and reading the report |
| [user/formats.md](docs/user/formats.md) | For users: the file formats read and written, with the CSV formats in full |
| [user/export-recipes.md](docs/user/export-recipes.md) | For users: getting vectors out of TBC, Infinity, MAGNET, CHC Geomatics Office and Emlid Studio |
| [user/datums-and-epochs.md](docs/user/datums-and-epochs.md) | For users: how frames, epochs, transformations and MGA work in the tool |
| [LANDSCAPE.md](docs/LANDSCAPE.md) | Existing commercial and open-source tools, and how this project relates to them |

## Licence

Apache-2.0: see [LICENSE](LICENSE) and [D-008](docs/DECISIONS.md). Third-party components keep their own licences; see [NOTICE](NOTICE).

## Feedback

The project isn't open for code contributions yet. Feedback on the plan is welcome via issues. It's especially useful from surveyors and geodesists, and from anyone with GNSS datasets they're willing to share for the benchmark.
