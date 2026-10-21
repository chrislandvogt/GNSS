# A second-opinion adjustment

You've processed GNSS baselines in a commercial package and adjusted them there. This tool adjusts the same vectors independently, tests them, and reports on them, so you can check the result before you rely on it. It's open source, it shows its working, and it runs in a browser or from the command line, with the same results either way.

What you need:

1. **Vectors** with their full covariances, exported from your processing package in a format the tool reads. [export-recipes.md](export-recipes.md) says how for each package, and [formats.md](formats.md) describes the formats.
2. **Control**: the marks you hold or weight, with coordinates in a known frame, and uncertainties for weighted marks. DynaML stations, CSV or a SINEX file (such as an AUSPOS solution) will do. Without control, adjust minimally constrained to test the vectors on their own.
3. **The frame to adjust in**, normally GDA2020. Vectors in other frames, such as ITRF2014 at the survey's epoch, are transformed first, and the report lists each transformation.

## In the browser

Open the web app. Nothing is uploaded: the files stay in your browser, and the adjustment runs there.

1. **Add files.** Choose or drop your vector and control files. The format is guessed from each file's contents; check it. For STAR*NET and CSV files without a frame column, choose the frame the vectors are in (and an epoch, for a dynamic frame). For an RTKLIB solution, give the base and rover mark names if the file names aren't the marks' names.
2. **Settings.** Choose the frame to adjust in, the datum (constrained by the control, or minimally constrained), and whether uncertainties use the a priori or a posteriori variance factor.
3. **Adjust.** The result shows the verdict, the statistics and the suspects, worst first.
4. **Judge the suspects.** For each, decide: exclude it with a reason (and the adjustment runs again), or keep it. Excluded observations stay in the report with their reasons and their misclosures, and you can include them again.
5. **Download** the report and the exports: GeoPackage, CSV, LandXML, DynaML or STAR*NET.

*Download project.toml* saves the settings, imports and exclusions. To carry on with the command line, create a project with `gnss init`, import the same files, then replace its `project.toml` with the downloaded one.

## With the command line

The `gnss` command keeps a project in a folder:

```text
my-survey/
  project.toml     settings, imports and exclusions, which you can edit
  raw/             the imported files, never changed
  raw.lock         their SHA-256 hashes, checked on every run
  results/         reports and exports, which can always be made again
```

A typical session:

```sh
gnss init my-survey --name "Bright control survey" --frame GDA2020
cd my-survey
gnss import control ../export/stations.xml --format dynaml
gnss import vectors ../export/vectors.xml --format dynaml
gnss adjust
gnss report                 # results/report.html
gnss exclude vectors.xml:435 --reason "Vector test 28.4; re-observed on day 2"
gnss adjust
gnss export --format gpkg   # results/results.gpkg
```

- `gnss import vectors` takes `--format dynaml`, `starnet`, `tdef`, `rtklib` or `csv`; `gnss import control` takes `dynaml`, `csv` or `sinex`. Without `--format`, the format is guessed from the file's contents and name, and the guess is shown. Give `--frame` (and `--epoch`) for files that don't say, and `--base` and `--rover` for an RTKLIB solution.
- `gnss adjust --minimal` adjusts minimally constrained for one run, without changing the project.
- `gnss export --format` takes `gpkg`, `csv`, `landxml`, `dynaml` or `starnet`. `--output` writes elsewhere.
- Every command takes `--project <folder>` (the current folder by default) and `--json`, for scripts.
- **Exit codes:** 0 for success, 1 for an error, and 2 when the adjustment ran but a statistical test failed or there was no redundancy to test with. Scripts and CI can stop on it:

```sh
gnss --project my-survey adjust --json > summary.json || echo "check the suspects"
```

To include an excluded observation again, remove its `[[edit]]` from `project.toml`. A raw file that changes after import stops the project with an error, since its hash no longer matches `raw.lock`. Import the new version under another name instead.

## Reading the report

- **The verdict** at the top says whether the global test passed and how many tests of observations failed.
- **Tests.** The global test checks the variance factor against χ² at 5%. Each vector's components are w-tested on local east, north and up axes at the level α₀ (0.1% by default), and each vector as a whole by its 3-degree-of-freedom test, with the B-method tying the levels together at power β₀ (80%). The minimal detectable bias (MDB) is the smallest error each test would catch.
- **Suspects** are ranked by how far each test failed. A failed test is evidence, not a verdict. Look at the vector's processing, its session and its set-up before excluding it, and record why.
- **Coordinates** come with their standard deviations and SP1's positional uncertainty (PU) at 95%. The relative uncertainty (RU) table covers each pair of marks joined by an observation. Both follow the ICSM *Guideline for Adjustment and Evaluation of Survey Control*.
- **Loop closures and repeated vectors** are tested on their own, so a blunder can be found before it's spread through the adjustment. They screen every vector, including those you've excluded, so a loop through an excluded vector still shows why it was left out.
- **Provenance** lists every input file with its SHA-256 hash, every transformation applied, and every assumption the readers made, such as orthometric heights used as starting values.

The adjustment is checked against DynAdjust on DynAdjust's own sample network: transformed vectors agree within 0.5 µm, coordinates within 0.7 µm, and the same outliers are flagged ([VALIDATION.md](../VALIDATION.md)).
