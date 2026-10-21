# A worked example: a second opinion on a TBC network

Thirty-one GNSS vectors processed in Trimble Business Center, from a December 2019 survey of eleven marks around Bright, Victoria. The export is public: ICSM published it with PynAdjust, and it's in this repository as `crates/formats/tests/data/tdef/tbc-bright-2019.asc`. The steps below use the command line; the web page does the same ([second-opinion.md](second-opinion.md)).

## 1. The project, the vectors and the control

```console
$ gnss init bright --name "Bright control survey"
Created project "Bright control survey" in bright, adjusting in GDA2020.
$ cd bright
$ gnss import vectors ../tbc-bright-2019.asc --frame ITRF2014
Imported tbc-bright-2019.asc as tdef (guessed from the file; give --format to choose): 11 stations and 31 measurements.
  note: tbc-bright-2019.asc, line 1: 11 marks are starting positions only (11 keyed in, 0 from processing), in GDA2020; to hold control, import it
```

TDEF files don't record their vectors' frame ([formats.md](formats.md#tdef-trimble-business-center)). Baselines are processed in the frame of the satellite orbits, close to ITRF2014 at the time of the survey, so the import says ITRF2014; each vector's epoch is the middle of its observation. So close to 2020.0, taking them as GDA2020 instead would change the global test's statistic by only 0.02. The note says the marks' coordinates in the file are only starting positions.

The control is the five CORS, held at the GDA2020 coordinates keyed into TBC, in a CSV file:

```csv
name,latitude,longitude,height,constraint,frame
BEEC,-36.346434060,146.657743009,442.9339,fixed,GDA2020
MYRT,-36.557956214,146.722203452,227.1787,fixed,GDA2020
WNGA,-36.354541422,146.323064315,162.6992,fixed,GDA2020
HOTH,-36.982063056,147.141848352,1773.9196,fixed,GDA2020
MIMI,-36.532210239,147.373232453,290.5791,fixed,GDA2020
```

```console
$ gnss import control ../cors.csv
Imported cors.csv as csv-control (guessed from the file; give --format to choose): 5 stations and 0 measurements.
```

## 2. The adjustment

```console
$ gnss adjust
Bright control survey: 11 marks, 31 observations, 75 degrees of freedom, in GDA2020.
Global test FAILED: σ̂₀² 1.971, Ω/σ₀² 147.83 outside [52.94, 100.84].
Loops: 21, 3 failed.
5 test(s) failed, worst first:
  tbc-bright-2019.asc:67 (MYRT to 324900700): w north 6.14, |w| > 3.29
  tbc-bright-2019.asc:67 (MYRT to 324900700): vector test 41.55 > 12.63
  tbc-bright-2019.asc:58 (324900360 to 324900700): vector test 29.10 > 12.63
  tbc-bright-2019.asc:58 (324900360 to 324900700): w up 4.43, |w| > 3.29
  tbc-bright-2019.asc:58 (324900360 to 324900700): w north -3.34, |w| > 3.29
```

The exit code is 2: the adjustment ran, but tests failed. The variance factor of 1.97 says the vectors disagree with each other and the control by about 1.4 times their stated precision (√1.97), more than chance allows. Every failed test is on one of two vectors into mark 324900700. Each vector is named by its file and line.

`gnss report` writes `results/report.html`, which has the detail.

## 3. Reading the evidence

**The loops point at vector 58.** All three failed loops go through it, from 324900360 to 324900700, and each misses by 53 to 57 mm in height:

| Loop | Length | Misclosure E, N, U (mm) | Statistic (limit 12.63) |
|---|---|---|---|
| MYRT → 324900700 → 324900360 | 2.4 km | 1.2, −11.1, 52.9 | 30.03 |
| 208500690 → 324900700 → 324900360 | 14.0 km | −4.7, −8.1, 57.5 | 19.06 |
| 324901050 → 324900700 → 324900360 | 5.9 km | 4.2, 6.6, 55.6 | 15.58 |

Vector 58's own vertical residual is 42 mm. The loops screen the vectors before any adjustment, so the 5 cm isn't something the control or the network created.

**Vector 67 fails on its own account.** MYRT to 324900700 is short (1.2 km) and precise. Its north residual is only 4.6 mm, but its w-test is 6.1: 6.1 times the standard deviation that residual should have. No loop singles it out; the w-test, which compares it with the whole adjusted network, does.

**The mark itself looks fine.** Five vectors reach 324900700, all from the same occupation (01:53 to 03:11 on 17 December, as TBC recorded the times). The other three pass their tests, so a set-up error at 324900700, such as a wrong antenna height, would show in all five, and doesn't. The two failing vectors are worth reprocessing (ambiguities, cycle slips, the observation window) or re-observing.

## 4. Excluding, one at a time, with reasons

Start with the clearest evidence:

```console
$ gnss exclude tbc-bright-2019.asc:58 --reason "5 cm vertical misclosure in all three loops through it; recheck its processing"
$ gnss adjust
Bright control survey: 11 marks, 31 observations, 72 degrees of freedom, in GDA2020.
Global test FAILED: σ̂₀² 1.649, Ω/σ₀² 118.73 outside [50.43, 97.35].
Loops: 21, 3 failed.
4 test(s) failed, worst first:
  tbc-bright-2019.asc:67 (MYRT to 324900700): w north 5.40, |w| > 3.29
  tbc-bright-2019.asc:67 (MYRT to 324900700): vector test 31.94 > 12.63
  tbc-bright-2019.asc:76 (208500690 to 324900700): vector test 13.23 > 12.63
  tbc-bright-2019.asc:81 (260600120 to 324900700): w north -3.37, |w| > 3.29
```

Vector 67 still fails, as expected. Its north disagreement now also pulls two of the other vectors into 324900700 just past their limits.

```console
$ gnss exclude tbc-bright-2019.asc:67 --reason "Still fails in north (w 5.4) with vector 58 out; recheck its processing"
$ gnss adjust
Bright control survey: 11 marks, 31 observations, 69 degrees of freedom, in GDA2020.
Global test passed: σ̂₀² 1.258, Ω/σ₀² 86.79 within [47.92, 93.86].
Loops: 21, 3 failed.
No observation failed a test.
```

The exit code is 0. 324900700 still has three vectors, so it's still checked. The loops still report three failures because they screen every vector, including the excluded ones: they're the record of why vector 58 went.

## 5. What to hand over

```console
$ gnss report
$ gnss export --format gpkg
```

- `results/report.html` holds the verdict, the tests, every observation's residuals, the two exclusions with their reasons, coordinates with SP1 PU, RU, the loops, and the provenance: the input files with their SHA-256 hashes, and the ITRF2014 to GDA2020 transformation applied.
- `results/results.gpkg` has the marks, vectors and 95% error ellipses, for ArcGIS Pro or QGIS.
- `project.toml` records the settings and the two exclusions with their reasons. With `raw/`, it reproduces this adjustment exactly.

σ̂₀² is 1.26, within the global test's bounds, so TBC's covariances fit the remaining vectors. To scale the uncertainties by it anyway, set `uncertainty = "a-posteriori"` in `project.toml`.
