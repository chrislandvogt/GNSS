/* Broadcast satellite positions from a navigation file, by RTKLIB, as the
   oracle for our orbits. Prints satellite, GPS seconds of week, ECEF x y z
   (m), clock (s) and health at `count` times `step` seconds apart from
   `offset` seconds into the date:

       rtklib_satpos NAV YYYY MM DD STEP COUNT OFFSET

   Build against RTKLIB's sources (BSD-2-Clause; we used rtklibexplorer's
   RTKLIB 2.5.1, commit 62d4677), from its src directory:

       gcc -O2 -DENAGLO -DENAGAL -DENAQZS -DENACMP -DENAIRN -DNFREQ=3 \
           -DNEXOBS=3 -I. -o rtklib_satpos path/to/rtklib_satpos.c \
           rtkcmn.c rinex.c ephemeris.c preceph.c sbas.c trace.c -lm -lpthread

   Galileo is set to I/NAV, which our selection prefers. */
#include <stdlib.h>
#include "rtklib.h"

int main(int argc, char **argv)
{
    nav_t nav = {0};
    obs_t obs = {0};
    sta_t sta;
    if (argc < 8) return 2;
    if (readrnxt(argv[1], 1, (gtime_t){0}, (gtime_t){0}, 0.0, "", &obs, &nav, &sta) <= 0) return 1;
    uniqnav(&nav);
    setseleph(SYS_GAL, 1);
    double ep[6] = {atof(argv[2]), atof(argv[3]), atof(argv[4]), 0, 0, 0};
    gtime_t t0 = epoch2time(ep);
    double step = atof(argv[5]), offset = atof(argv[7]);
    for (int k = 0; k < atoi(argv[6]); k++) {
        gtime_t t = timeadd(t0, offset + step * k);
        for (int sat = 1; sat <= MAXSAT; sat++) {
            double rs[6], dts[2], var;
            int svh, week;
            char id[8];
            if (!satpos(t, t, sat, EPHOPT_BRDC, &nav, rs, dts, &var, &svh)) continue;
            satno2id(sat, id);
            printf("%s %.1f %.4f %.4f %.4f %.12e %d\n", id, time2gpst(t, &week), rs[0], rs[1],
                   rs[2], dts[0], svh);
        }
    }
    return 0;
}
