"""Download the published grid files the Australian pack lists, checking each
against its SHA-256.

    python3 crates/datum/tests/oracle/fetch_grids.py <directory> [--format geotiff]

CI fetches the GeoTIFFs (33 MB) so `tests/grids.rs` can test the readers on
the published files; generate_grids.py needs all four (527 MB). Files already
there with the right hash aren't downloaded again.
"""

import argparse
import hashlib
import sys
import tomllib
import urllib.request
from pathlib import Path

PACK = Path(__file__).resolve().parents[4] / "packs" / "au" / "pack.toml"


def files(fmt=None):
    pack = tomllib.loads(PACK.read_text())
    for grid in pack["grids"]:
        for f in grid["files"]:
            if fmt is None or f["format"] == fmt:
                yield f


def sha256(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("directory", type=Path)
    parser.add_argument("--format", choices=["ntv2", "geotiff"], help="only files in this format")
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    for f in files(args.format):
        path = args.directory / f["file"]
        if path.exists() and sha256(path) == f["sha256"]:
            print(f"{f['file']}: already there")
            continue
        for attempt in range(3):
            try:
                with urllib.request.urlopen(f["url"], timeout=60) as r, path.open("wb") as out:
                    while block := r.read(1 << 20):
                        out.write(block)
                break
            except OSError as e:
                print(f"{f['file']}: {e}, retrying", file=sys.stderr)
        else:
            sys.exit(f"{f['file']}: couldn't download {f['url']}")
        digest = sha256(path)
        if digest != f["sha256"]:
            sys.exit(f"{f['file']}: SHA-256 is {digest}, but the pack says {f['sha256']}")
        print(f"{f['file']}: {path.stat().st_size} bytes, SHA-256 checked")


if __name__ == "__main__":
    main()
