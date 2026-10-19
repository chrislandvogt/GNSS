// A fixed static-baseline configuration for the M0 engine check. From M8 the
// core generates engine configuration from project settings
// (ARCHITECTURE.md 9.3); this exists only to show the engine running.
export const ENGINE_CHECK_CONFIG = `pos1-posmode       =static
pos1-frequency     =l1+l2
pos1-soltype       =combined
pos1-elmask        =15
pos1-ionoopt       =brdc
pos1-tropopt       =saas
pos1-sateph        =brdc
pos1-navsys        =41
pos2-armode        =continuous
pos2-gloarmode     =off
pos2-arthres       =3
out-solformat      =xyz
out-solstatic      =single
ant2-postype       =rinexhead
`
