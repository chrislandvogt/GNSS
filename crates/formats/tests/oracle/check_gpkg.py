#!/usr/bin/env python3
"""Check the formats crate's GeoPackage output with SQLite and with GDAL.

The ignored `write_samples` test in tests/gpkg.rs writes GeoPackages in three
CRSs: 5,000 points (enough for interior B-tree pages), polygons long enough
for overflow pages, a line and an empty layer. The api crate's export test
writes DynAdjust's sample network, adjusted: marks, vectors and error
ellipses. This runs SQLite's integrity, quick and foreign key checks on each,
and reads each layer back with GDAL (through pyogrio, which bundles it),
checking CRS, geometry type, feature count and fields.

    python3 -m venv .venv && .venv/bin/pip install pyogrio
    .venv/bin/python -I crates/formats/tests/oracle/check_gpkg.py
"""

import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

import pyogrio

CRATE = Path(__file__).resolve().parent.parent.parent
EXPECTED = {
    "marks": ("Point", 5000, ["name", "height", "fixed"]),
    "shapes": ("Polygon", 50, ["name"]),
    "vectors": ("LineString", 1, ["id"]),
    "empty": ("Point", 0, []),
}
NETWORK = {
    "marks": ("Point", 43, ["name", "latitude", "longitude", "height", "x", "y", "z", "zone",
                            "easting", "northing", "sd_e", "sd_n", "sd_u", "pu_h95", "pu_v95",
                            "held"]),
    "vectors": ("LineString", 133, ["id", "from_mark", "to_mark", "length", "v_e", "v_n", "v_u",
                                    "w_e", "w_n", "w_u", "vector_test", "flagged", "used",
                                    "source"]),
    "ellipses": ("Polygon", 43, ["name", "semi_major_95", "semi_minor_95", "azimuth",
                                 "exaggeration"]),
}


def main():
    with tempfile.TemporaryDirectory() as directory:
        env = dict(os.environ, GNSS_GPKG_DIR=directory)
        subprocess.run(
            ["cargo", "test", "--quiet", "-p", "gnss-formats", "--test", "gpkg",
             "write_samples", "--", "--ignored"],
            cwd=CRATE, env=env, check=True,
        )
        subprocess.run(
            ["cargo", "test", "--quiet", "-p", "gnss-api", "--test", "exports",
             "geopackage_has_marks_vectors_and_ellipses"],
            cwd=CRATE, env=env, check=True,
        )
        failures = 0
        for path in sorted(Path(directory).glob("*.gpkg")):
            code = 7844 if path.stem == "sample-network" else int(path.stem)
            expected = NETWORK if path.stem == "sample-network" else EXPECTED
            db = sqlite3.connect(path)
            checks = [
                db.execute("pragma integrity_check").fetchall(),
                db.execute("pragma quick_check").fetchall(),
            ]
            if checks != [[("ok",)], [("ok",)]] or db.execute("pragma foreign_key_check").fetchall():
                print(f"{path.name}: SQLite finds {checks}")
                failures += 1
            db.close()
            for layer, (geometry, count, fields) in expected.items():
                info = pyogrio.read_info(path, layer=layer)
                got = (info["crs"], info["geometry_type"], info["features"], list(info["fields"]))
                if got != (f"EPSG:{code}", geometry, count, fields):
                    print(f"{path.name} {layer}: GDAL reads {got}")
                    failures += 1
            print(f"{path.name}: SQLite and GDAL ({pyogrio.__gdal_version_string__}) read it")
        sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
