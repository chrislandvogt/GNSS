# RTKLIB test data

Three static solutions of one baseline, from CORS MTHR (base) to SNGO
(rover), 9.6 km apart in the Hunter region of New South Wales, from two hours
of observations on 15 June 2025 (02:00 to 04:00 GPST). They were computed by
the RTKLIB-EX 2.5.1 WebAssembly module (`engine-modules/rtklib-ex`) with the
M0 engine spike's settings (`benchmarks/spike-m0`): static, combined
filtering, GPS, Galileo and BeiDou, broadcast orbits, and
`out-solstatic=single`. The only difference between the files is
`out-solformat`: `xyz`, `llh` and `enu`.

The observations are CORSnet-NSW and Geoscience Australia CORS data,
distributed by Geoscience Australia under Creative Commons Attribution 4.0.
RTKLIB writes the files with mixed line endings, which are kept.
