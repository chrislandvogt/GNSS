# Getting vectors out of your processing package

A second-opinion adjustment needs the same vectors your package adjusted, with their covariances, in a format the tool reads ([formats.md](formats.md)). This page says what to export and how, package by package.

## What any export must carry

- **Every vector's three components** (ΔX, ΔY, ΔZ in metres, Earth-centred) and the two marks it joins.
- **The full covariance**: three variances and three covariances (or standard deviations and correlations) per vector. Standard deviations alone throw away the correlations between components, which for GNSS are large, and the tests then judge the vectors against the wrong ellipsoid.
- **The covariances as the processor computed them**, before any scaling an adjustment applied. If your package scales them (by a variance factor, or a "scalar" applied to all vectors), note the factor: you can apply the same one in the tool, but the second opinion is most useful when it starts from the processing results.
- **The frame and epoch of the vectors.** Processors usually work in the frame of the orbits (WGS 84 with broadcast orbits, the IGS frame with precise ones) at the time of the survey. Some packages transform vectors to the project's datum on export, and some don't. Formats that don't record it (STAR*NET, RTKLIB and CSV without frame columns) need it given on import.
- **Control** in a known frame: DynaML stations, CSV, or a SINEX file such as an AUSPOS solution.

The tool tells you what it assumed: every transformation, every default frame and every skipped record is listed in the report's provenance section.

## Package by package

Each recipe is being checked against the package's own documentation, and against real exports where they can be had. Until a package's recipe is here, export its vectors as DynaML or STAR*NET if it offers either; otherwise use the CSV vector format ([formats.md](formats.md#csv-vectors)) with the full covariance.

| Package | Recipe |
|---|---|
| Trimble Business Center | Being checked |
| Leica Infinity | Being checked |
| Topcon MAGNET | Being checked |
| CHC Geomatics Office | Being checked |
| Emlid Studio | Being checked. It's built on RTKLIB, so its static solutions should read as RTKLIB `.pos` files |

If you've exported vectors from one of these, what worked (or didn't) is very welcome as an issue.
