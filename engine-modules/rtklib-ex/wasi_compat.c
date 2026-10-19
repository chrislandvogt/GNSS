/*
 * WASI compatibility shims for building RTKLIB-EX as a WebAssembly module.
 *
 * WASI has no shell, so system() always reports failure. RTKLIB only calls it
 * (through execcmd) to run gzip, tar or crx2rnx on compressed input files.
 * The engine driver always passes uncompressed RINEX, so these paths are never
 * taken in normal use. If they are, RTKLIB reports a read error for that file
 * rather than silently using wrong data.
 */
#include <stdlib.h>

int system(const char *command)
{
    (void)command;
    return -1;
}
