# Landscape

This is what already exists, and how this project relates to it. It answers the question every partner and contributor asks first: why not just use X?

## Commercial office suites

| Tool | Vendor | What it covers |
|---|---|---|
| Trimble Business Center | Trimble | Full suite: baselines, network adjustment, CAD, point clouds, RTX post-processing |
| Leica Infinity | Hexagon | Full suite, successor to LGO |
| MAGNET Tools / Office | Topcon | Baselines and adjustment |
| CHC Geomatics Office | CHC Navigation | Lower-cost suite, common with Chinese receiver brands |
| Waypoint GrafNav / GrafNet | NovAtel (Hexagon) | PPK, GNSS/INS and static networks |
| STAR*NET | MicroSurvey | Least squares adjustment. The usual choice for independent checks of vendor vectors. |
| Emlid Studio | Emlid | Free but closed source. RTKLIB-based PPK and static processing, no network adjustment. |

These suites are capable but cost thousands of dollars per seat, are closed, and mostly favour their own hardware. We don't try to replace their CAD, point cloud or field software. We aim to be the better choice for static control processing, adjustment and QA, and to work alongside them as an independent check (P3 in [PLAN.md](PLAN.md#3-users-and-jobs-to-be-done)).

## Open-source tools

| Tool | What it does | How we relate to it |
|---|---|---|
| RTKLIB-EX (formerly demo5) | Static, kinematic, PPK and PPP processing, tuned for modern receivers | v1 baseline engine, run as a WebAssembly module (BSD-2; D-003, D-023) |
| Ginan (Geoscience Australia) | Geodetic-grade PPP-AR and network processing | Phase 2 engine for PPP and long baselines (Apache-2.0; D-004) |
| PRIDE PPP-AR | PPP with ambiguity resolution | Comparison reference only. GPL-3, so it's never linked. |
| gLAB (ESA/UPC) | SPP, PPP and data analysis for teaching and research | Reference for QC metrics |
| GNSSTk | C++ and Python GNSS library | Reference only. LGPL, so it's never linked. |
| goGPS | MATLAB GNSS processing | Reference only |
| DynAdjust (Geoscience Australia) | Rigorous adjustment at national scale; ran the GDA2020 adjustment | Test oracle and export target (D-010) |
| JAG3D | Least squares adjustment GUI with GNSS support | Comparison reference. GPL-3, so it's never linked. |
| GNU Gama | Geodetic network adjustment | Comparison reference. GPL-3, so it's never linked. |
| GeodePy (Geoscience Australia) | Python geodesy and GDA2020 transformations | Test oracle for datums (D-011) |
| PROJ | Coordinate transformations | Test oracle and optional backend (D-011) |
| `rinex` crate (nav-solutions) | Rust RINEX parsing and QC | Vetted and not used: it panics on most of a sample of GA CORS files and reads whole files into memory. We read RINEX ourselves (D-027) |
| `gnss-rtk` crate | Rust PVT solver | Not used: AGPL-3, and no integer ambiguity resolution found (D-006) |
| georinex | Python RINEX reader | Reference for parser behaviour |
| Anubis | RINEX quality checking (a teqc replacement) | Reference for QC metrics |

## Free online services

| Service | Operator | How we relate to it |
|---|---|---|
| AUSPOS | Geoscience Australia | Import its SINEX output as control. Ginan gives an offline equivalent. |
| OPUS | NGS (United States) | A reference for the generic ITRF pack |
| CSRS-PPP | NRCan (Canada) | A reference for the generic ITRF pack |

These services are excellent, but they're black boxes. They need an internet connection, and they aren't appropriate for every location or organisation.

## The gap

Engines and adjusters already exist. What's missing is the workflow layer, in one cross-platform tool:

- occupation metadata and blunder detection
- automatic data fetch
- explicit datum and epoch handling
- standards reporting
- reproducibility
- a public validation record

That gap is this project.
