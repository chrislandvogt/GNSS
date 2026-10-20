# File formats

Which files the tool reads and writes, and what it does with each. For getting vectors out of a particular processing package, see [export-recipes.md](export-recipes.md).

## At a glance

| Format | Read as | Written as | Says its frame and epoch? |
|---|---|---|---|
| DynaML (DynAdjust XML) | Vectors, clusters, stations and point clusters | The network as adjusted | Yes |
| STAR*NET GPS vectors | Vectors | The network's vectors | No: give the import a frame |
| TDEF (Trimble Data Exchange Format) | Vectors, with marks as starting positions | | No: give the import a frame |
| RTKLIB `.pos` | One baseline | | No: WGS 84 is assumed unless you give one |
| SINEX (such as AUSPOS) | Weighted control with full covariance | | Usually, in its comments |
| CSV vectors | Vectors | | Optional `frame` and `epoch` columns |
| CSV control | Fixed, free or weighted marks | Adjusted marks | Optional `frame` and `epoch` columns |
| GeoPackage | | Marks, vectors and error ellipses | |
| LandXML | | Control points in MGA | |
| HTML | | The adjustment report | |

Every vector and mark keeps the frame and epoch it came in. The adjustment transforms them into the project's frame and epoch, and the report lists every transformation applied ([datums-and-epochs.md](datums-and-epochs.md)). A file in a dynamic frame, such as ITRF2014, also needs an epoch.

Each observation is identified by its file and line, such as `vectors.xml:435`. Edits (exclusions) and the report use that identifier, so it stays the same however often you adjust.

## Reading

### DynaML

Geoscience Australia's DynAdjust format, as described in the DynAdjust User's Guide (appendix B) and `DynaML.xsd`. The station and measurement files can be imported separately or together.

- **Stations**: `LLH` (orthometric heights), `LLh` (ellipsoidal heights), `XYZ` and `UTM`, with latitudes and longitudes in DynAdjust's `ddd.mmssss` notation. Constraints are read per coordinate: `CCC` held, `FFF` free. Partial constraints, such as `CCF`, aren't supported yet and are refused with a message rather than changed.
- **Orthometric heights** are fine for free marks, since they're only starting values; the report says when they were used that way. A held mark needs an ellipsoidal height.
- **Measurements**: GNSS baselines (`G`), baseline clusters (`X`) with their full covariance, and point clusters (`Y`) in `XYZ`. Each measurement may give its own frame and epoch. `Vscale`, `Pscale`, `Lscale` and `Hscale` scale the covariances as DynAdjust's user guide describes. (DynAdjust 1.4.1 itself applies `Vscale` twice when partial scales are also given.)
- `<Ignore>*</Ignore>` becomes an exclusion, with that as its reason.
- Terrestrial measurements (angles, distances, levelling) are skipped with a note. They arrive in a later milestone (M23).

### STAR*NET

STAR*NET GPS vector files, as processing packages export them for STAR*NET:

```text
.GPS WEIGHT COVARIANCE
G0 'Session 12  base 0759 to rover 3040
G1 0759-3040   2022.0123  -467.4561  2612.3678
G2 1.2345E-06  1.4321E-06  9.8765E-07
G3 -2.1E-07  3.4E-07  -1.2E-07
```

- `G1` gives the marks (`FROM-TO`) and the ECEF components in metres. `G0` is an optional name.
- With `.GPS WEIGHT COVARIANCE` (the default), `G2` holds the variances of X, Y and Z, and `G3` the covariances XY, XZ and YZ, in m². With `.GPS WEIGHT STDERRCORR`, `G2` holds standard errors in metres and `G3` correlations.
- If the file has no `.GPS WEIGHT` line and its `G3` values look like correlations, the import is refused rather than guessed at. Mixing the two up changes the weights by orders of magnitude.
- `.GPS FACTOR` scales the standard errors, horizontally and vertically.
- `G4` records and antenna heights on `G1` lines are refused, since ignoring them would change the vectors. Other records (terrestrial observations, coordinates) are skipped with a note.
- **The files don't say which frame or epoch the vectors are in**, so the import needs both: for example GDA2020, or ITRF2014 at 2026.5.

### TDEF (Trimble Business Center)

The Trimble Data Exchange Format, written by Trimble Business Center's TDEF exporter, and reportedly by CHC Geomatics Office's "TGO baseline result" export. Trimble doesn't publish a specification, so the reader follows real TBC exports and ICSM's PynAdjust converter.

- `[GPS]` `Vector` records give the two marks, ΔX, ΔY and ΔZ in metres, and the covariance in m² (XX, XY, XZ, YY, YZ, ZZ), then the antenna heights and the observation's start and end. A covariance that isn't positive definite is refused, which also catches a misread order of its terms.
- The vectors must be mark to mark (`GPSVectors=MarkToMark`, as TBC writes them), in metres. Anything else is refused rather than guessed at.
- `[Stations]` and `[Keyed In Coordinates]` (latitude, longitude and ellipsoidal height in the project's coordinate system, such as `Australia/GDA2020`) become free marks: starting positions only. Keyed-in coordinates are used where both are given. To hold control, import it separately; the CORS coordinates keyed into TBC can go in a CSV control file.
- **The file doesn't say which frame the vectors are in**, so the import needs one. Since version 2024.10, TBC sets it under *Project Settings → Computations → GNSS Vector*: the global datum at its reference epoch, or ITRS at the measurement epoch. Give the import the matching frame. For a dynamic frame, each vector's epoch is the middle of its observation unless the import gives one. In Australia, ITRF2014 at the survey's epoch and GDA2020 differ for vectors only by the plate's rotation since 2020.0: about 0.01 ppm a year, or half a millimetre over 50 km for each year.

### RTKLIB `.pos`

A static baseline solution from RTKLIB's `rnx2rtkp` or RTKPOST, or from a package built on RTKLIB, such as Emlid Studio.

- All three of RTKLIB's position forms are read: ECEF, latitude and longitude (degrees, or degrees, minutes and seconds) and height, and east, north and up from the base.
- RTKLIB writes each covariance term as a signed square root, which is undone. It writes standard deviations to 0.1 mm, so very precise static baselines carry that rounding: up to about 20% in their variances.
- Export with a single static solution (`out-solstatic=single` in RTKLIB's configuration). If the file has several solutions, the last is used, with a note.
- Only fixed (Q=1), float (Q=2) and DGPS (Q=4) solutions are accepted.
- **Marks:** the base is the reference position in the header, and the rover is the solution. Their names default to the first two observation files the header lists, the rover's first. Give them with the import's base and rover names (`--base` and `--rover` in the CLI) when the file names aren't the marks' names.
- **Frame:** RTKLIB doesn't record it. Broadcast orbits give vectors in WGS 84, which is assumed with a note; give the import a frame if precise orbits were used. The epoch is the middle of the observations.

### SINEX

Station positions with full covariance, such as an AUSPOS solution or an IGS weekly solution. They become weighted control: one point cluster holding every site, with the correlations between them.

- `SOLUTION/ESTIMATE` positions (`STAX`, `STAY`, `STAZ`) and `SOLUTION/MATRIX_ESTIMATE` as a covariance (`COVA`), correlations (`CORR`) or an information matrix (`INFO`).
- SINEX has no field for the frame, so it's taken from the import if given, or else from the file's comments: AUSPOS names it there. A file whose comments name more than one frame isn't guessed at.
- Sites that no vector reaches, such as the CORS in an AUSPOS solution, are taken out of the cluster properly, keeping the correlations between the rest.

### CSV vectors

For packages with no other format in common. Columns are found by name, in any order, ignoring case, spaces, underscores and units in brackets (`dX (m)` is `dx`). Lines starting with `#` are comments. Commas separate fields, or semicolons or tabs if the heading uses them.

| Column | Meaning |
|---|---|
| `from`, `to` | The marks (also `base` and `rover`) |
| `dx`, `dy`, `dz` | ECEF components, metres |
| `cov_xx`, `cov_xy`, `cov_xz`, `cov_yy`, `cov_yz`, `cov_zz` | The covariance, m², or: |
| `sd_x`, `sd_y`, `sd_z` | standard deviations, m, with optional `corr_xy`, `corr_xz`, `corr_yz` |
| `frame`, `epoch` | Optional: the frame, and the epoch as a decimal year or a date |
| `id` | Optional: a name for the vector |
| `ignore` | Optional: `yes` to leave the vector out |

```csv
# A 9.6 km baseline, in WGS 84 from broadcast orbits, observed on 15 June 2025
from,to,dx,dy,dz,sd_x,sd_y,sd_z,corr_xy,corr_xz,corr_yz,frame,epoch
MTHR,SNGO,-6450.1070,-4661.1080,5356.1452,0.0012,0.0007,0.0006,-0.762,0.681,-0.595,WGS 84,2025-06-15
```

Without `frame` and `epoch` columns, the import needs a frame (and an epoch for a dynamic frame).

### CSV control

| Column | Meaning |
|---|---|
| `name` | The mark |
| `x`, `y`, `z` | ECEF, metres, or: |
| `latitude`, `longitude`, `height` | Degrees (decimal, or `d m s`) and ellipsoidal height. Use `orthometric_height` (or `elevation`) for heights above the geoid. Or: |
| `easting`, `northing`, `zone`, `height` | MGA (UTM south) |
| `sd_e`, `sd_n`, `sd_u` | Standard deviations on local east, north and up, m. Or `sd_h` and `sd_v` (horizontal and vertical), `sd_x`, `sd_y`, `sd_z` with `corr_xy`…, or `cov_xx`… in ECEF |
| `constraint` | `fixed`, `free` (a starting position only) or `weighted` |
| `frame`, `epoch`, `description` | Optional |

Without a `constraint` column, marks with standard deviations are weighted and the rest are fixed.

```csv
name,latitude,longitude,height,sd_h,sd_v,frame
BEEC,-36.346434,146.657743,442.9330,0.003,0.008,GDA2020
MYRT,-36.557956,146.722203,227.1843,,,GDA2020
```

Here BEEC is weighted (3 mm horizontally, 8 mm vertically) and MYRT is held fixed.

## Writing

| Export | Files | What's in them |
|---|---|---|
| GeoPackage | `results.gpkg` | Three layers in the frame's geographic CRS. **marks**: name, latitude, longitude, height, ECEF, MGA zone, easting and northing where the frame has MGA, standard deviations, PU and how each was held. **vectors**: each vector as a line, with residuals, w-tests, the vector test, whether it was used or flagged, and its source. **ellipses**: 95% error ellipses, drawn exaggerated (1,000 times by default) so they're visible on a map, with their true sizes and azimuths as attributes |
| CSV | `marks.csv`, `observations.csv` | Marks: ECEF, latitude, longitude and height, MGA, standard deviations, SP1 PU at 95%, and the frame and epoch. Observations: residuals, w-tests and the vector test on local axes, redundancy numbers, MDBs and N-statistics. `marks.csv` reads back in as CSV control |
| LandXML | `control.xml` | LandXML 1.2 control points in MGA, in the zone holding most of the marks, with ellipsoidal heights. Only for GDA2020 and GDA94 |
| DynaML | `adjusted-stn.xml`, `adjusted-msr.xml` | The network as adjusted: stations at their adjusted positions, and the vectors as used, in the network's frame. For cross-checking in DynAdjust, or for submission |
| STAR*NET | `adjusted.gps` | The vectors as used, with their covariances (`.GPS WEIGHT COVARIANCE`) |
| HTML report | `report.html` | One self-contained page: the verdict, tests, ranked suspects with MDBs, the edits and their reasons, coordinates with PU, RU between observed pairs, loop closures and repeated vectors, every observation's residuals and tests, and provenance (input files with SHA-256 hashes, transformations, notes and settings) |

Uncertainties use the a priori variance factor by default, as DynAdjust's PU does. Set `uncertainty = "a-posteriori"` in `project.toml`, or choose it on the web page, to scale them by the estimated σ̂₀² instead.

The same project gives the same bytes in every export, natively on Windows, macOS and Linux and in the browser, so a download from the web page matches the CLI's file.
