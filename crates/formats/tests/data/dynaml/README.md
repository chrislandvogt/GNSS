# DynaML test data

`gnss-networkstn.xml` and `gnss-networkmsr.xml` are DynAdjust's sample GNSS
network, copied unchanged from `sampleData/` in
[DynAdjust](https://github.com/icsm-au/DynAdjust) v1.4.1 (commit a5fa92e).
DynAdjust is © Geoscience Australia and released under the Apache License 2.0.

The network is 43 marks around Bright, Victoria, observed with 129 GNSS
baselines in ITRF2008 at 18 February 2015, one cluster of four baselines in
ITRF2014 at 17 January 2018, and one cluster of six positions in GDA2020. The
files keep their Windows line endings, which the reader must handle.

`crates/api/tests/oracle/dynadjust_sample.py` adjusts them with DynAdjust, and
`crates/api/tests/dynaml.rs` compares.
