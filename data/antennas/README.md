# Antenna catalogue

What the app knows about GNSS antennas, read by the `gnss-antennas` crate
(ARCHITECTURE.md section 6).

## From the IGS

| File | What it is |
|---|---|
| `rcvr_ant.json` | The IGS antenna, radome and receiver codes, with descriptions |
| `antenna.gra` | Each antenna's antenna reference point (ARP) and north reference point, and drawings with dimensions in metres |
| `calibrated.txt` | The antenna and radome pairs with an IGS type-mean calibration in `igs20.atx`, listed by `update.py` (the 60 MB file itself isn't kept) |
| `sources.toml` | Where each came from, when, and its SHA-256 |

They're copied unchanged from the IGS Central Bureau,
<https://files.igs.org/pub/station/general/>, described at
<https://igs.org/wg/antenna/>. The IGS provides its data and products openly
"for use without restriction", and asks users to cite and attribute them:

> International GNSS Service (IGS), Central Bureau and Antenna Working
> Group: antenna and radome codes (`rcvr_ant.json`), antenna reference
> points and drawings (`antenna.gra`) and antenna calibrations
> (`igs20.atx`). See the IGS Data and Product Disclaimer and Terms of Use
> (5 August 2020).

To refresh them, run `python update.py` here, then the tests (`cargo test -p
gnss-antennas`), which check the files against `sources.toml` and read every
drawing. Review what changed before committing.

## Slant-height geometry

`slant.toml` is ours. Converting a slant height to the vertical height of
the ARP needs the slant point's radius from the antenna's axis and its height
above the ARP:

    vertical = √(slant² − radius²) − above_arp

The IGS files don't give these: `antenna.gra` marks a slant-height point on
only a few antennas, and never with its radius. So the table starts empty,
and a slant height on an antenna without an entry is refused with a message
rather than converted with a guess.

To add an antenna, add an entry with a source a reviewer can check, such as
the manufacturer's manual, its version and the page. Don't take numbers from
memory, or by measuring a drawing. Name the slant point as surveyors will
write it in an occupation's height (`to`), and give the numbers in metres.
