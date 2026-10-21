# TDEF test data

Two Trimble Data Exchange Format files exported by Trimble Business Center,
copied byte for byte from the test resources of ICSM's PynAdjust
(https://github.com/icsm-au/PynAdjust, Apache License 2.0, commit
804e0376aa995fe05976aeb47bf9dea2ff974408):

| File here | PynAdjust's name | Contents |
|---|---|---|
| `tbc-bright-2019.asc` | `tdef_example.asc` | 31 vectors between 11 marks around Bright, Victoria, observed on 16 and 17 December 2019, with keyed-in GDA2020 coordinates |
| `tbc-geelong-2020.asc` | `skiasciisample.asc` | 3 vectors between 4 marks near Geelong, Victoria, observed on 16 and 17 February 2020. Despite PynAdjust's name, it's a TBC TDEF export, not Leica SKI-ASCII |

Both are in a GDA2020 project (`ProjCoordinateSystem=Australia/GDA2020`)
with mark-to-mark vectors.
