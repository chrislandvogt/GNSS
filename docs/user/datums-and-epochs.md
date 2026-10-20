# Datums and epochs

How the tool handles reference frames, coordinate epochs, transformations and heights, and why it won't let you mix them up by accident.

## Frames

| Frame | EPSG | Kind | Notes |
|---|---|---|---|
| ITRF2020 | 9988 | dynamic | The current international frame |
| IGS20 | 10176 | dynamic | The IGS realisation of ITRF2020, used by GNSS orbits since late 2022. Treated as identical to ITRF2020 |
| ITRF2014 | 7789 | dynamic | The frame GDA2020 and ATRF2014 are tied to |
| ATRF2014 | 9307 | dynamic | Australia's realisation of ITRF2014, and the frame adjustments work in |
| GDA2020 | 7842 | static, at 2020.0 | Australia's datum |
| GDA94 | 4938 | static, at 1994.0 | Australia's previous datum |
| WGS 84 (G2296) | 10604 | dynamic | Aligned with ITRF2020 to about 1 cm |
| WGS 84 (G2139) | 9753 | dynamic | Aligned with ITRF2014 to about 1 cm |
| WGS 84 | 4978 | dynamic | No stated realisation, so good to about 2 m |

## Static and dynamic frames

Australia moves about 7 cm a year to the north-north-east. A dynamic frame, such as ITRF2020 or ATRF2014, is fixed to the Earth as a whole, so coordinates of a mark in it change every year. A coordinate in a dynamic frame means nothing without its **coordinate epoch**: the time it was true.

A static frame, such as GDA2020, is fixed to the Australian plate, so a mark's coordinates don't change. Every GDA2020 coordinate is at GDA2020's reference epoch, 2020.0.

So the tool enforces two rules:

- **A position in a dynamic frame must have an epoch.** "ATRF2014" alone is refused; "ATRF2014 at 2023.5" is fine.
- **A position in a static frame can't have any other epoch.** "GDA2020 at 2023.5" doesn't exist: the coordinate you mean is in ATRF2014 at 2023.5, and the tool says so. Transform it to GDA2020 instead.

Positions in different frames or at different epochs can't be combined, for example to compute a vector between them, until one is transformed to match the other.

## Coordinate epochs

Epochs are decimal years: the calendar year plus the elapsed fraction of it, in GPS time. 1 January 2018 is 2018.0, and noon on 2 July 2018 is 2018.5. Leap years have 366 days.

Software differs on this. GeodePy counts 365.25-day years from a transformation's reference date, which can differ by up to three quarters of a day. That's about 0.15 mm of plate motion. The calendar convention reproduces the GDA2020 Technical Manual's worked example more closely.

## Transformations

| From | To | Method | Source | Uncertainty (one sigma) |
|---|---|---|---|---|
| ITRF2014 | ITRF2020 | 14-parameter Helmert, at the coordinate epoch | EPSG:9991, IERS | About 0.2 mm, plus 0.2 mm a year from 2015 |
| IGS20 | ITRF2020 | Identity | IGS | None published |
| ITRF2014 | ATRF2014 | Identity, in Australia | EPSG:9460 | 10 mm |
| ITRF2014, ATRF2014 | GDA2020 | Australian Plate Motion Model: rotation rates about each axis | EPSG:8049 and 9459; GDA2020 Technical Manual | Grows with time from 2020.0: about 0.1 mm a year |
| GDA94 | GDA2020 | 7-parameter conformal Helmert | EPSG:8048; GDA2020 Technical Manual section 3.1 | About 1 mm |
| GDA94 | GDA2020 | ICSM's conformal and distortion grid, with heights by EPSG:8048 | EPSG:8447 | The grid's own, where it gives one (median about 1 cm), on top of the conformal 1 mm |
| WGS 84 (G2296) | ITRF2020 | Identity | EPSG:10608 | 10 mm |
| WGS 84 (G2139) | ITRF2014 | Identity | EPSG:9757 | 10 mm |
| WGS 84 | WGS 84 (G2139) | Identity | EPSG datum ensemble | 2 m |

Each transformation can be used in either direction. The Australian ones are only defined over Australia and its offshore territories.

### GDA94 to GDA2020: conformal, or conformal and distortion?

GDA94 coordinates from the state and territory survey networks carry those networks' distortion: errors that built up as the networks were adjusted. It's typically about a centimetre, but over a metre in parts of Western Australia and central Australia. ICSM's conformal and distortion grid models it, and the conformal transformation doesn't. In Queensland the grid models no distortion, so there it gives the conformal transformation's result. The GDA2020 Technical Manual and ICSM's fact sheet T1 give the guidance; in short:

- **Conformal and distortion grid (EPSG:8447)**: GDA94 coordinates of survey control, cadastre and other data tied to the state networks.
- **Conformal (EPSG:8048)**: GDA94 coordinates computed directly from CORS or AUSPOS, which don't carry the distortion, and anything on Christmas or Cocos Islands, which the grid doesn't cover.

The tool uses the conformal transformation unless a project loads the grid. With the grid loaded, it's used everywhere it covers (mainland Australia and Tasmania), and the conformal transformation everywhere else. Each step of the path says which was used, with the grid file and subgrid.

The grid is 2D: it shifts latitude and longitude. Ellipsoidal heights still change between GDA94 and GDA2020 (GDA2020's are 7 to 12 cm lower), so the tool takes them from the conformal transformation.

## Paths

When you transform a position, the tool finds the shortest chain of published transformations from its frame and epoch to the ones you asked for. It shows every step, with the transformation used and its source, and records them in the project's provenance. For example, ITRF2020 at 2023.5 to GDA2020 is:

1. ITRF2020 at 2023.5 to ITRF2014 at 2023.5, by the inverse of EPSG:9991
2. ITRF2014 at 2023.5 to GDA2020, by the plate motion model (EPSG:8049)

Moving a coordinate to another epoch in a dynamic frame needs a model of how the ground moves. Inside Australia that's the plate motion model, so the path goes through GDA2020: for example, ATRF2014 at 2018.0 to GDA2020, then to ATRF2014 at 2025.0. Outside Australia there's no such model yet, and the tool says so rather than guessing.

## Heights

Every height says what it's measured from, and how it was obtained. There's no untyped "height".

- **Ellipsoidal heights** are above a frame's ellipsoid. GNSS gives these.
- **AHD heights** are above the Australian Height Datum. They come from GDA2020 ellipsoidal heights through AUSGeoid2020 (EPSG:8451), as H = h − N, where N is AUSGeoid2020's separation between the ellipsoid and AHD at the point. Going the other way, h = H + N.

AUSGeoid2020 is defined on GDA2020, so it only takes GDA2020 positions. A position in ATRF2014 at an epoch, or in GDA94, is refused with a pointer to transform it to GDA2020 first.

### How sure is an AHD height?

Geoscience Australia publishes AUSGeoid2020's uncertainty as a separate grid: a one-sigma value for N at every point, typically 9 to 12 cm and at most 16 cm. An AHD height's sigma combines it with the position's own vertical uncertainty. If either is unknown, so is the height's: the tool doesn't guess.

The uncertainty of N is largely shared between nearby points, so a height difference between two marks is much better known than either height. The per-point sigma is what's reported for now; differences come with the adjustment (milestone M4).

### At the edge of the model

N is interpolated between the four grid nodes around a point. Offshore, AUSGeoid2020 stops; where only some of the four nodes have a value, the tool uses those, with their weights rescaled, as PROJ does. Where none has a value, there's no AHD height.

## Grid files

The GDA94 to GDA2020 grid and AUSGeoid2020 are published as files, too big to bundle with the tool (AUSGeoid2020 is 247 MB). The Australian pack (`packs/au/pack.toml`) lists each file with its URL, size and SHA-256, and the tool checks the hash before using one.

| Grid | File | Size | From | Browser |
|---|---|---|---|---|
| GDA94 to GDA2020 conformal and distortion | `au_icsm_GDA94_GDA2020_conformal_and_distortion.tif` | 27 MB | PROJ-data (ICSM's grid as GeoTIFF) | Direct |
| AUSGeoid2020 | `AUSGeoid2020_20180201.gsb` | 247 MB | Geoscience Australia | Relay |
| AUSGeoid2020 | `au_ga_AUSGeoid2020_20180201.tif` | 6 MB | PROJ-data (N only, identical values) | Direct |
| AUSGeoid2020 uncertainty | `AUSGeoid2020_20180201_error.gsb` | 247 MB | Geoscience Australia | Relay |

"Direct" means the web app can download the file itself; "relay" means the host doesn't allow that, so the file comes through the relay or a data pack (see ARCHITECTURE.md section 13). The command line downloads either.

The tool reads both NTv2 (`.gsb`) files, as ICSM and Geoscience Australia publish them, and GeoTIFF grids as PROJ publishes them. It reads only the parts of a file it needs, so a 247 MB grid never has to fit in memory. ICSM's own NTv2 version of the GDA94 to GDA2020 grid reads too; the pack will list it once its hash is checked.

## Uncertainty

A transformed position carries two uncertainties, kept apart:

- **The position's own covariance**, propagated through each step. This is what matters for vectors and adjustments.
- **The transformations' uncertainty**: how well their parameters are known. Every position transformed along the same path shares this error, so it doesn't change the vectors between them, only their absolute position. It's reported separately, so it's added only where absolute uncertainty is what's wanted.

## The Map Grid of Australia

MGA2020 is defined on GDA2020, and MGA94 on GDA94. The tool computes grid coordinates only from positions in those frames, so an ATRF2014 coordinate can't be passed off as MGA2020.

- **Zones** are 6° wide, numbered like UTM zones: 49 to 56 across the mainland. A position can be computed in a neighbouring zone on request, up to 30° from the zone's central meridian.
- **The projection** is Transverse Mercator by the Krüger n-series to eighth order, accurate to well under a micrometre across a zone.
- **Point scale factor and grid convergence.** The grid convergence follows the GDA2020 Technical Manual's sign: grid bearing = azimuth + convergence. In the southern hemisphere it's negative west of a zone's central meridian. Some software uses the opposite sign.
- **Line, height and combined scale factors** turn ground distances into grid distances. The line scale factor uses Simpson's rule, and the height factor uses the ellipsoidal height.

## How it's checked

On every change, CI compares the tool's results with independent software, natively and in the browser:

- **The GDA2020 Technical Manual's worked examples** (ALIC) agree within the manual's 0.1 mm rounding.
- **PROJ** agrees on 551 transformations to the last bit, and on geodetic conversions and MGA coordinates to a few nanometres.
- **GeodePy** agrees on transformations and their covariance propagation, and on MGA within its 0.1 mm rounding.
- **DynAdjust's coordinates for the 109 national CORS** agree within the 0.2 to 0.4 mm their files are rounded to.
- **The GDA94 to GDA2020 grid** agrees with PROJ to under a micrometre, both ways, and with GeodePy's shifts and accuracies within its rounding.
- **AUSGeoid2020** agrees with PROJ on N to well under a nanometre.

The grid tests use small windows cut from the published grids. CI also reads PROJ's published grid files themselves, and fuzzes the grid readers with malformed files.

CI also checks that the results are bit for bit the same on Windows, macOS, Linux and in the browser, so the CLI and the web app always agree.

## Not yet supported

- AVWS heights
- AGD66, AGD84 and other jurisdictions' datums
- Bicubic interpolation of grids, which some software offers alongside bilinear
