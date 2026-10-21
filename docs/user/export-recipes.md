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

Most packages don't export DynaML or STAR*NET vectors with covariances. Each has its own exchange format, so each recipe says which of the tool's formats you can get to, and how. **Checked** means the tool has read a real export; **documented** means the vendor's or MicroSurvey's documentation says so, with the source listed below; **not confirmed** means a plausible route nobody has checked yet. Please report what works for you, or doesn't, as an issue.

| Package | Route | Status |
|---|---|---|
| Trimble Business Center | TDEF export, read directly | Checked |
| CHC Geomatics Office | "TGO baseline result" export, read as TDEF | Documented, not confirmed |
| Leica Infinity | STAR*NET export (5.0), or SKI-ASCII through PynAdjust to DynaML | Documented, not confirmed |
| Topcon MAGNET / Topcon Tools | TVF, through STAR*NET or rearranged as CSV | Documented, not confirmed |
| Emlid Studio | RTKLIB `.pos`, read directly | Documented |

### Trimble Business Center

1. Select the vectors to export.
2. Export them with the **TDEF exporter** ("Export Trimble Data Exchange Format (TDEF) File", in every edition) [1].
3. Import the `.asc` file as `tdef`, giving the frame of the vectors. Since TBC 2024.10, that's set under *Project Settings → Computations → GNSS Vector*: the global datum at its reference epoch, or ITRS at the measurement epoch [2].

The TDEF file carries each vector's full covariance, and the marks' coordinates, which become starting positions. Import your control separately; the CORS coordinates keyed into TBC can go in a CSV control file. Two real TBC exports are in the tool's tests. One of them, 31 vectors around Bright, Victoria, fails its global test, and its two vectors into one mark fail their w-tests. Left out, the rest pass.

Not confirmed: whether TBC scales the covariances it exports. A question about this on Trimble's community forum is unanswered [3]. TBC has no documented STAR*NET vector export.

### CHC Geomatics Office

Export **File → Export → Trimble Data Exchange → TGO_Baseline_Result (\*.ASC)**, as MicroSurvey's STAR*NET import guide describes, citing section 8.5 of CHC's manual [4]. Import it as `tdef`. The format should be the same as TBC's, but no CHC file has been checked yet.

### Leica Infinity

Two routes, neither checked yet:

- **STAR*NET.** Infinity 5.0's export has an output type *MicroSurvey STAR\*NET DAT* that includes post-processed GNSS vectors; versions 4.3.1 to 5.0 need a module from Leica [5]. Whether it writes the vectors' covariances (`G2` and `G3` records) isn't documented. Import it as `starnet` with the frame: the tool refuses vectors without weights rather than guessing them.
- **SKI-ASCII, converted to DynaML.** Export with *Home → Export → All*, format *SKI ASCII - ASC*, coordinate type *WGS84 Cartesian*, with covariance, antenna heights and offsets, and epoch information turned on [6]. ICSM's PynAdjust converts the file to DynaML (`ski-ascii.py`) [7], which the tool reads. Note that the converter scales the covariances by the file's a posteriori σ₀² and sets `Vscale` to 100, which multiplies them by 100 again. Check that's what you want; the tool applies `Vscale` as DynaML says.

A direct SKI-ASCII reader is planned, once a real export is available to test it against.

### Topcon MAGNET Tools and Topcon Tools

Topcon's vector export is **TVF**, a comma-separated file with each vector's marks, ΔX, ΔY and ΔZ, standard errors and correlations [8]. The tool doesn't read it directly yet. Two routes:

- STAR*NET's *Topcon Tools TVF* importer turns it into STAR*NET `G` records, which the tool reads [8].
- Or rearrange its columns into the CSV vector format (`from`, `to`, `dx`, `dy`, `dz`, `sd_x`, `sd_y`, `sd_z`, `corr_xy`, `corr_xz`, `corr_yz`).

MicroSurvey warns that TVF vectors aren't corrected for antenna heights, and that older files need their units header edited [8]. Check both before using either route.

### Emlid Studio

Emlid Studio is built on RTKLIB and writes its results in RTKLIB's `.pos` format [9].

1. Process in **Static** mode with *Output solution* set to *One best* (the default), which gives one solution for the session [10].
2. Import the `.pos` file as `rtklib`, giving the base and rover mark names.

Studio writes latitude, longitude and height with the full covariance on local axes, which the tool reads. The header always says WGS84 [11]. The vector is in the frame of the orbits used, which for broadcast orbits is WGS 84, as the tool assumes. Its covariances are the processor's formal values, which users report as optimistic [12], so expect a variance factor above 1.

### Coming later

NGS's **GVX** (GNSS Vector Exchange) format is exported with full covariances by TBC, Infinity and MAGNET [13]. A reader for it would give one route from all three; it's planned, as are direct readers for SKI-ASCII and TVF.

## Sources

Checked on 20 October 2026. Several vendor sites couldn't be reached directly, so some claims rest on search engines' copies of these pages.

1. Trimble, *TBC command matrix by edition*: https://help.fieldsystems.trimble.com/tbc/pdf/tbc-command-matrix-editions-modules.pdf
2. Trimble, *TBC 2024.10 release notes*: https://help.fieldsystems.trimble.com/tbc/pdf/tbc-2024.10-release-notes.pdf
3. Trimble Community, *Clarification on a priori covariance and standard deviation computation in baseline processing*: https://community.trimble.com/question/clarification-on-a-priori-covariance-and-standard-deviation-computation-in-baseline-processing
4. MicroSurvey, *STAR\*NET import guide: Trimble Data Exchange*: https://s3.microsurvey.com/support/Knowledgebase/StarNet/ImportGuide/Content/SN_Importer4_6.htm
5. MicroSurvey, *Export conventional and GNSS data from Infinity to STAR\*NET*: https://helpdesk.microsurvey.com/article/2345-export-conventional-and-gnss-data-from-infinity-to-star-net
6. MicroSurvey, *Importing post-processed vectors from Infinity*: https://helpdesk.microsurvey.com/article/1344-importing-post-processed-vectors-from-infinity
7. ICSM, *PynAdjust* (`surveyconvert/ski-ascii.py` and `tdef.py`): https://github.com/icsm-au/PynAdjust
8. MicroSurvey, *Importing TVF GPS vector files from Topcon Tools into STAR\*NET*: https://helpdesk.microsurvey.com/article/497-importing-tvf-gps-vector-files-from-topcon-tools-into-star-net
9. Emlid Community, *Emlid Studio .pos file headers*: https://community.emlid.com/t/emlid-studio-pos-file-headers/29772
10. Emlid, *Static processing workflow*: https://docs.emlid.com/emlid-studio/static-processing-workflow/
11. Emlid Community, *Consider adding .pos file formatting options*: https://community.emlid.com/t/consider-adding-pos-file-formatting-options-in-the-processing-settings-dialog/34343
12. Emlid Community, *Constructing a realistic a posteriori covariance matrix from an RTKLIB .pos static baseline*: https://community.emlid.com/t/constructing-realistic-a-posteriori-covariance-matrix-from-rtklib-pos-static-baseline/44261
13. NGS, *GVX format*: https://geodesy.noaa.gov/data/formats/GVX/
