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

`natadjust_rvs_example.dat` is copied unchanged from [GeodePy](https://github.com/geoscienceaustralia/GeodePy), © Geoscience Australia, under the Apache License 2.0. Its columns are: station, constraints, easting, northing, zone, latitude and longitude (DDD.MMSSsssss), AHD height, ellipsoidal height, X, Y, Z, and the standard deviations of east, north and up.
