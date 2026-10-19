# Reference data for the datum tests

| File | What it is | Source |
|---|---|---|
| `geodetic_ecef.csv` | Geodetic and ECEF coordinates of 86 points across Australia and its offshore territories, on GRS80 | PROJ, through `tests/oracle/generate.py` |
| `mga_proj.csv` | MGA2020 coordinates, point scale factors and grid convergences of those points, in their own zone and the zone to the west | PROJ (`tmerc`, Poder/Engsager), through `generate.py` |
| `mga_geodepy.csv` | The same, from GeodePy, which rounds coordinates to 0.1 mm | GeodePy, through `generate.py` |
| `helmert_proj.csv` | GDA94 to GDA2020, ITRF2014 to GDA2020, GDA2020 to ATRF2014 and ITRF2020 to ITRF2014, at six epochs | PROJ's EPSG pipelines, through `generate.py` |
| `helmert_geodepy.csv` | The same transformations with covariance propagation, including the parameters' uncertainties | GeodePy, through `generate.py` |
| `natadjust_rvs_example.dat` | GDA2020 coordinates of the 109 Recognised Value Standard CORS, written by DynAdjust | GeodePy's test suite (`geodepy/tests/resources`) |

The first line of each generated file records the versions it was made with. To regenerate, see `tests/oracle/generate.py`.

## Grids

`grids/` holds windows cut from the published grids around the test points, and reference values from the full grids, all written by `tests/oracle/generate_grids.py` from the files `packs/au/pack.toml` lists.

| File | What it is | Source |
|---|---|---|
| `gda94_gda2020_crop.gsb` | 44 windows of ICSM's GDA94 to GDA2020 conformal and distortion grid, as NTv2 | PROJ-data's GeoTIFF of ICSM's grid, written here |
| `gda94_gda2020_crop.tif` | The same windows as GeoTIFF in PROJ's conventions, `PixelIsArea`, 16 × 16 tiles, band by band | Written by GDAL 3.12 through rasterio |
| `gda94_gda2020.csv` | 49 points: GDA2020 latitude and longitude by the grid both ways, heights by EPSG:8048, ECEF, and GeodePy's shifts and accuracies | PROJ (full grid) and GeodePy (the NTv2 windows) |
| `gda94_gda2020_full.csv` | 400 points over the whole grid, both ways | PROJ |
| `ausgeoid2020_crop.gsb`, `ausgeoid2020_error_crop.gsb` | 33 windows of AUSGeoid2020 and of its uncertainty grid, node records copied unchanged | Geoscience Australia's NTv2 files |
| `ausgeoid2020_crop.tif` | The same windows of N as GeoTIFF, `PixelIsPoint`, in strips | PROJ-data's GeoTIFF, written by GDAL |
| `ausgeoid2020.csv` | N and its uncertainty at 33 points, one at the edge of the model's coverage | PROJ (N, full grid) and NumPy (uncertainty, full grid) |
| `ausgeoid2020_full.csv` | The same at 400 points over the whole model | PROJ and NumPy |

The grids are © ICSM and © Geoscience Australia, under Creative Commons Attribution 4.0 (see NOTICE).

`natadjust_rvs_example.dat` is copied unchanged from [GeodePy](https://github.com/geoscienceaustralia/GeodePy), © Geoscience Australia, under the Apache License 2.0. Its columns are: station, constraints, easting, northing, zone, latitude and longitude (DDD.MMSSsssss), AHD height, ellipsoidal height, X, Y, Z, and the standard deviations of east, north and up.
