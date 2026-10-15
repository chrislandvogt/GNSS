# Project plan

This is the product plan: why the project exists, who it serves, what's in and out of scope, and how we'll know it's working. The work breakdown is in [ROADMAP.md](ROADMAP.md), the technical design in [ARCHITECTURE.md](ARCHITECTURE.md), the trust strategy in [VALIDATION.md](VALIDATION.md), and existing tools in [LANDSCAPE.md](LANDSCAPE.md).

## 1. Vision

Any surveyor, agency or student, anywhere, can turn raw GNSS observations into adjusted coordinates they can defend, using free, open, auditable software that shows its working.

**Thesis.** Don't build a free Trimble Business Center or a better RTKLIB. The baseline maths is largely solved, and good open engines already exist. What costs people money is:

- blunders that slip through
- results they can't defend
- hours lost chasing data and writing reports

So this tool makes a static survey correct by construction and defensible. The baseline engine is a swappable part.

## 2. The problem

| Pain | Why it matters | What we do about it |
|---|---|---|
| Metadata blunders | A wrong antenna height or type, or a swapped mark ID, produces errors from centimetres to metres. The adjustment can partly absorb or hide them. Processing software mostly takes header or controller values on trust. | Field-log capture, header reconciliation and automated blunder checks |
| Opaque QA | Fix status and a ratio aren't evidence. Formal baseline covariances are usually several times too optimistic, so networks look better than they are. | Statistical testing, reliability analysis, variance component estimation, and visual explanation on the map |
| Datum and epoch confusion | GDA94, GDA2020 and ATRF2014; ellipsoidal, AHD and AVWS heights; CORS coordinates at different epochs. The errors are silent. | Frame and epoch carried by every coordinate, explicit transformations, and a show-your-working panel |
| Data chasing | CORS RINEX, orbits, clocks and ANTEX come from different portals, and some need logins | Automatic fetch and cache, plus offline data packs |
| Reporting | Standards reporting (SP1 positional and relative uncertainty) is manual and error-prone | Built-in standards evaluation and report templates |
| Reproducibility | A result can't be reproduced years later unless inputs, settings and software were pinned. Cadastral disputes and audits need exactly that. | Content-addressed pipeline and an append-only provenance record |
| Cost and lock-in | Commercial seats cost thousands of dollars, and mixed-brand fleets are awkward | Free, vendor-neutral and RINEX-first, and it runs in a browser with nothing to install |

## 3. Users and jobs to be done

| ID | Persona | Job to be done | What they need first |
|---|---|---|---|
| P1 | Low-cost receiver user (maker, agriculture, drone operator, small contractor) | "Establish my base and a few marks to centimetres, with an honest uncertainty." | RINEX in, automatic CORS fetch, a guided workflow, antenna calibration warnings |
| P2 | Surveyor in a small firm or council | "Run a control survey to SP1 and lodge the result." | The full workflow, blunder checks, SP1 report, agency exports |
| P3 | Surveyor using TBC or Infinity | "Get an independent check of my network." | Vector import, rigorous adjustment and QA, a comparison report |
| P4 | Educator or student | "Understand and demonstrate the maths." | Transparent intermediate results, reproducible examples, readable code |
| P5 | Government, defence or agency user | "Process offline, auditably and repeatably, at scale." | Air-gapped operation, provenance, CLI and batch processing, signed release bundles, SBOM |
| P6 | Pacific or developing-region land agency | "Densify control and modernise the datum affordably." | A free tool, offline operation, jurisdiction packs, training material |

**Design target:** P2's rigour with P1's simplicity. P3 is served from v0.1, before anyone has to trust our baseline processing.

## 4. Goals and non-goals

### Goals for v1.0

- **G1 End-to-end static control survey.** RINEX and field logs go in. Adjusted GDA2020/AHD coordinates and an SP1 report come out. Tested on campaigns from 2 to 20 receivers and up to several hundred baselines. Processing covers baselines up to about 20 km, which is the RTKLIB-EX envelope.
- **G2 Independent adjustment** of vectors imported from other software, available from v0.1.
- **G3 Automated blunder detection** for the common field errors: antenna height, antenna type, mark identity and session timing.
- **G4 Explicit frames and epochs**, with an Australian jurisdiction pack covering GDA2020, ATRF2014, GDA94, AHD via AUSGeoid2020, AVWS and MGA2020.
- **G5 Reproducible projects**, offline operation and no telemetry.
- **G6 Runs in current desktop browsers with nothing to install**, and works offline once loaded. A native CLI covers Windows, macOS and Linux, and a release bundle serves air-gapped machines.
- **G7 A public benchmark** with release gates.

### Non-goals for v1.0

- Field data collection, controller software or RTK. We capture occupation metadata only.
- Reading proprietary raw formats such as Trimble T02/T04 or Leica MDB. The input is RINEX. Open formats (UBX, SBF, Unicore, NovAtel, RTCM3) are supported through conversion.
- Kinematic or PPK trajectories, and drone image geotagging. The engine can do them, but they aren't a v1 workflow.
- Total station, levelling or combined adjustments. These come in Phase 3.
- CAD, drafting, point clouds and cadastral plan preparation.
- Scientific geodesy (Bernese and GAMIT territory): orbit determination, global networks and atmospheric research.
- Real-time positioning and NTRIP casting.
- Hosted processing of users' data (software as a service). The web app runs on the user's own machine. Agencies can host their own server in Phase 2.

## 5. Product principles

1. **Correct by construction.** Frames, epochs, units and height types live in the type system. Invalid combinations don't compile, or they're rejected when loaded.
2. **Show the working.** Every number traces back to its inputs, settings and maths. Nothing is a black box.
3. **Never silently guess.** Missing or conflicting metadata raises a visible issue with a suggested fix. Defaults are explicit and logged.
4. **Raw data is sacred.** Original files are immutable and hash-verified. Corrections are recorded as overlays, with reasons.
5. **Reproducible by default.** The same inputs, settings and versions give the same answer, and every result records how it was made.
6. **Local and offline first.** Processing happens on the user's machine, in the browser or the CLI. Project data never leaves it unless the user exports it or configures a destination. The network is optional, and air-gapped operation is a supported mode, not a workaround.
7. **Engine-agnostic.** Engines are plugins behind one interface. Their results are compared, not trusted.
8. **Validated in public.** Benchmark results are published with every release, and regressions block releases.
9. **Open in, open out.** RINEX, DynaML, STAR*NET, LandXML, GeoPackage and CSV. No proprietary lock-in.
10. **Surveyor-grade UX.** Fast, keyboard-friendly, legible outdoors and honest about uncertainty.

## 6. Release plan

| Release | Phase | Headline | Milestones | Primary users |
|---|---|---|---|---|
| v0.1 | 0 | Second-opinion adjustment in the browser and CLI: import vectors, adjust, test, report uncertainties | M0 to M4 | P3, P4 |
| v0.3 | 1 | CLI end to end: RINEX to RTKLIB-EX baselines to adjustment, with blunder checks | M5 to M9 | P1, P4 |
| v0.5 beta | 1 | Web app beta with map, baseline inspector and adjustment dashboard; first public benchmark | M10, M11 (beta) | P1, P2 pilots |
| v1.0 | 1 | Production static workflow with SP1 reporting, versioned releases with signed offline bundles, and release gates met | M11 to M13 | P2, P1, P3 |
| v1.x | 2 | Field capture in the web app, in-field checks, Ginan PPP, Python and ArcGIS Pro, server and MCP, more jurisdictions | M14 to M19 | P5, P6 |
| v2.0 | 3 | Rust batch engine, multi-baseline sessions, dual-engine checks, terrestrial observations, monitoring | M20 to M24 | All |

## 7. Workstreams

| Workstream | Covers |
|---|---|
| Core | Data model, datums and geodesy, adjustment, formats, pipeline, command API |
| Processing | RINEX ingest and QC, data fetch, engines, blunder checks |
| App | Web app, CLI, reports |
| Validation | Benchmark datasets, oracles, CI runs, published results |
| Platform | Native and WebAssembly builds, hosting and versioned releases, security, supply chain |
| Community and standards | Partners, documentation, training, governance |

Validation and Platform start in M0 and run alongside everything else. They aren't end-of-project activities.

## 8. Validation and trust

Surveyors carry professional liability. They'll only adopt a tool they can see is right. The project's most important asset is therefore a public benchmark rather than code:

- Hundreds of baselines formed between CORS whose coordinates are known to the millimetre, cut to any length and session window.
- Contributed low-cost receiver datasets observed on known marks.
- Synthetic networks and oracle comparisons for the adjustment, using DynAdjust as the reference implementation.

The benchmark runs in CI, and its results are published with every release. See [VALIDATION.md](VALIDATION.md).

## 9. Partners, community and adoption

### Partners to approach, roughly in order

1. **Geoscience Australia.** The Ginan, DynAdjust and GNSS data teams. Co-own the benchmark and review the datum handling.
2. **ICSM Permanent Committee on Geodesy.** Alignment with SP1 and its guidelines, and a path to recognition.
3. **A state agency**, for example NSW Spatial Services (SCIMS). Pilot projects and submission formats.
4. **Universities with surveying programs.** Teaching use, thesis-sized contributions and validation work.
5. **Geospatial Council of Australia and state surveyor boards.** CPD sessions and awareness.
6. **The low-cost GNSS community** (RTKLIB-EX users, ArduSimple, AgOpenGPS, Emlid forums). Early users and datasets.
7. **OSGeo Oceania and FOSS4G SotM Oceania.** Visibility in the FOSS community.

### Adoption tactics

- **Lead with the second-opinion tool.** It's useful without anyone having to trust our baseline engine, and it runs in a browser, so trying it takes a link rather than an install.
- **Publish worked example projects** on public data, with expected outputs.
- **Run "bring your own data" sessions and CPD workshops** with pilot organisations.
- **Make short screen recordings** for each core workflow.

### Governance

- Start with a single maintainer and a decision log.
- Add maintainers as contributors earn trust.
- Move to a small steering group if partner organisations join.
- Use the Contributor Covenant code of conduct and DCO sign-off for contributions.

### Funding avenues to explore

- Partner in-kind support from agencies and universities
- University research grants and student projects
- Research software programs such as ARDC
- Sponsorship through GitHub Sponsors or Open Collective

Check fit before applying to any of these.

## 10. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Wrong integer fixes from an engine damage trust | Medium | High | Benchmark with a wrong-fix gate, conservative ambiguity validation, a published envelope, and dual-engine checks in Phase 3 |
| Scope creep into a full TBC clone | High | High | A non-goals list, phase gates, and v1.0 limited to static control surveys |
| Single maintainer (bus factor) | High | High | Docs first, decision log, modular crates, and recruiting university and agency contributors early |
| Datum handling subtly wrong | Medium | High | Cross-validation against GeodePy and PROJ, explicit epochs, and review by GA |
| RTKLIB-EX weak on long baselines and short sessions | High | Medium | Publish the envelope, use Ginan for long baselines, and build our own Rust engine in Phase 3 |
| Low-cost antenna calibrations missing from ANTEX | High | Medium | Warnings with the expected bias, calibration import, and guidance on using the same antenna type at both ends |
| RTKLIB-EX doesn't build or perform well as WebAssembly | Medium | High | A spike in M0, before other work depends on it. The fallback is running it natively through the CLI (D-023) |
| Browser limits: no folder access in Firefox and Safari, storage eviction, memory per WebAssembly instance | Medium | Medium | Folders where supported and zips elsewhere, persistent storage requests, inputs trimmed before processing, and the CLI for very large jobs |
| Data sources that block browser downloads | High | Medium | Per-source flags in packs, an optional self-hostable relay, data packs, and serving files with the app where licences allow |
| Tampering with the web app through its host or dependencies | Low | High | No third-party scripts, a strict Content Security Policy, dependency review, immutable versioned deployments, and signed bundles for local serving |
| Liability concerns block professional use | Medium | High | Apache-2.0 warranty disclaimer, published validation evidence, positioning as an independent check, and partner endorsements |
| Licence contamination from linking GPL or AGPL code | Low | High | A cargo-deny licence policy in CI, with copyleft tools run only as separate processes |
| Data licences limit redistributing the benchmark | Medium | Medium | Publish manifests and fetch scripts instead of raw data, and prefer CC BY sources |
| Churn in upstream dependencies (rinex, faer, Svelte, MapLibre, browser APIs) | Medium | Medium | Pin versions, wrap dependencies behind our own interfaces, and automate dependency updates |
| Sensitive coordinates in shared data | Low | High | Local processing, no telemetry, the relay off by default, and redaction guidance for benchmark contributors |

## 11. Success measures

These are targets to calibrate with pilot users, not commitments.

| Area | Measure |
|---|---|
| Trust | Benchmark published every release, engine gates met, and adjustment agreement with DynAdjust within 0.1 mm |
| Usefulness | Time from RINEX to report on the reference project, measured with pilots. Target under 30 minutes for a 10-mark campaign. |
| Adoption | At least 3 pilot organisations by v1.0, at least 3 regular contributors beyond the founder, and use per release |
| Quality | Zero data-loss bugs, and time to fix for bugs that affect correctness |
| Reach | At least one jurisdiction pack beyond Australia in v1.x |

We collect no telemetry and run no analytics scripts. Adoption is measured through CLI and bundle downloads, the static host's aggregate request counts, pilots and an opt-in survey.

## 12. Open questions

These are tracked in [DECISIONS.md](DECISIONS.md):

- Project name (D-014)
- Confirming Apache-2.0 versus dual MIT/Apache-2.0 (D-008)
- Which static host serves the web app, and whether the project runs a public relay (D-023)
- Whether RTKLIB-EX works well as a WebAssembly module (the M0 spike, D-023)
- How to produce PDF reports (D-012)
- GLONASS ambiguity resolution policy (D-019)
- Default adjustment frame strategy, to validate with GA (D-017)
- Which state agency to pilot with
- Benchmark hosting: CI compute and storage for RINEX data
