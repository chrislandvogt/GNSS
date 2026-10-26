//! The reader and the decompressor on the fixtures in `tests/data/rinex`:
//! restored files match the reference CRX2RNX byte for byte, and a file
//! reads the same compressed or not.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

use gnss_rinex_io::{Epoch, EpochFlag, ObsTypes, decompress, read_obs};

/// A fixture: its name, the RINEX file, it compressed, and that restored
/// by CRX2RNX.
type Fixture = (&'static str, &'static [u8], &'static [u8], &'static [u8]);

const FIXTURES: [Fixture; 4] = [
    (
        "synthetic3",
        include_bytes!("data/rinex/synthetic3.rnx"),
        include_bytes!("data/rinex/synthetic3.rnx.crx"),
        include_bytes!("data/rinex/synthetic3.rnx.crx2rnx"),
    ),
    (
        "synthetic2",
        include_bytes!("data/rinex/synthetic2.11o"),
        include_bytes!("data/rinex/synthetic2.11o.crx"),
        include_bytes!("data/rinex/synthetic2.11o.crx2rnx"),
    ),
    (
        "SWTC",
        include_bytes!("data/rinex/SWTC00AUS_20251660000.rnx"),
        include_bytes!("data/rinex/SWTC00AUS_20251660000.rnx.crx"),
        include_bytes!("data/rinex/SWTC00AUS_20251660000.rnx.crx2rnx"),
    ),
    (
        "MOBS",
        include_bytes!("data/rinex/MOBS0050.20o"),
        include_bytes!("data/rinex/MOBS0050.20o.crx"),
        include_bytes!("data/rinex/MOBS0050.20o.crx2rnx"),
    ),
];

fn epochs(bytes: &[u8]) -> Vec<Epoch> {
    let mut reader = read_obs(bytes).unwrap();
    let mut out = Vec::new();
    while let Some(epoch) = reader.next_epoch().unwrap() {
        out.push(epoch);
    }
    out
}

#[test]
fn compressed_files_restore_exactly_as_crx2rnx_restores_them() {
    for (name, _, crx, expected) in FIXTURES {
        let mut restored = Vec::new();
        let lines = decompress(crx, &mut restored).unwrap();
        assert_eq!(
            lines as usize,
            expected.iter().filter(|&&b| b == b'\n').count(),
            "{name}"
        );
        if restored != expected {
            let at = restored
                .iter()
                .zip(expected)
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            let line = expected[..at].iter().filter(|&&b| b == b'\n').count() + 1;
            panic!("{name}: restored file differs from CRX2RNX's at line {line}");
        }
    }
}

#[test]
fn files_read_the_same_compressed_or_not() {
    for (name, rnx, crx, restored) in FIXTURES {
        let plain = epochs(rnx);
        assert!(!plain.is_empty(), "{name}");
        assert_eq!(epochs(crx), plain, "{name}: compressed");
        assert_eq!(epochs(restored), plain, "{name}: restored");
    }
}

#[test]
fn the_rinex3_fixture_reads_as_written() {
    let mut reader = read_obs(FIXTURES[0].2).unwrap();
    assert_eq!(reader.header().marker_name, "SYNTH3");
    assert_eq!(reader.header().antenna_delta, Some([1.532, 0.0, 0.0]));
    let mut all = Vec::new();
    while let Some(epoch) = reader.next_epoch().unwrap() {
        all.push(epoch);
    }
    // 16 epochs of observations and two events.
    assert_eq!(all.len(), 18);
    let events: Vec<_> = all.iter().filter(|e| !e.flag.has_observations()).collect();
    assert_eq!(events[0].flag, EpochFlag::NewSite);
    assert_eq!(events[0].records.len(), 2);
    assert_eq!(events[1].flag, EpochFlag::ExternalEvent);
    assert_eq!(events[1].time, None);
    // The new site's header records took effect.
    assert_eq!(reader.header().marker_name, "TRIG2");
    assert_eq!(reader.header().antenna_delta, Some([1.601, 0.0, 0.0]));

    let obs: Vec<&Epoch> = all.iter().filter(|e| e.flag.has_observations()).collect();
    let first = obs[0];
    assert_eq!(
        first.time.unwrap().to_string(),
        "2026-01-01T00:00:00.0000000"
    );
    assert_eq!(first.clock_offset, Some(-0.000123456));
    assert_eq!(obs[5].clock_offset, None);
    assert_eq!(obs[12].clock_offset, Some(-0.000123312));
    let svs = |e: &Epoch| {
        e.satellites
            .iter()
            .map(|s| s.sv.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(svs(obs[6]), "G01 G02 R03 E04");
    assert_eq!(svs(obs[7]), "G01 R03 E04 G05");
    assert_eq!(svs(obs[10]), "G01 G02 R03 E04 G05");
    // G02's L1C lost lock at epoch 3; E04's C5Q is blank at 4 and 5.
    assert_eq!(obs[3].satellites[1].observations[1].unwrap().lli, Some(1));
    assert_eq!(obs[4].satellites[3].observations[2], None);
    assert!(obs[6].satellites[3].observations[2].is_some());
    // The cycle slip on G01 L1C at epoch 12: a million cycles, on top of
    // the 1.234 + 0.0101 (12² - 11²) the generator adds each epoch.
    let l1 = |k: usize| obs[k].satellites[0].observations[1].unwrap().value;
    assert!(
        (l1(12) - l1(11) - 1_000_001.466).abs() < 0.001,
        "{}",
        l1(12) - l1(11)
    );
    // Values under 1 and negative values.
    assert_eq!(obs[1].satellites[0].observations[3].unwrap().value, 0.5);
    assert_eq!(obs[2].satellites[0].observations[3].unwrap().value, -0.25);
    assert!(obs[0].satellites[2].observations[0].unwrap().value < 0.0);
}

#[test]
fn the_rinex2_fixture_reads_as_written() {
    let mut reader = read_obs(FIXTURES[1].2).unwrap();
    let ObsTypes::Common(types) = &reader.header().obs_types else {
        panic!()
    };
    assert_eq!(types.join(" "), "L1 L2 C1 P2 S1 S2");
    let mut all = Vec::new();
    while let Some(epoch) = reader.next_epoch().unwrap() {
        all.push(epoch);
    }
    assert_eq!(all.len(), 9);
    assert_eq!(all[4].flag, EpochFlag::StartMoving);
    let obs: Vec<&Epoch> = all.iter().filter(|e| e.flag.has_observations()).collect();
    assert_eq!(obs[0].satellites.len(), 14);
    assert_eq!(obs[2].satellites.len(), 13);
    assert_eq!(obs[0].satellites[13].sv.to_string(), "R02");
    assert_eq!(obs[0].clock_offset, Some(0.000012345));
    assert_eq!(obs[5].clock_offset, None);
    assert_eq!(obs[5].satellites[3].observations[0].unwrap().lli, Some(1));
    assert_eq!(obs[6].satellites[2].observations[3], None);
    assert_eq!(obs[0].satellites[0].observations[5].unwrap().value, 41.0);
}

#[test]
fn real_files_read() {
    let swtc = epochs(FIXTURES[2].1);
    assert_eq!(swtc.len(), 6);
    assert!(swtc.iter().all(|e| e.satellites.len() > 20));
    let mobs = epochs(FIXTURES[3].1);
    assert_eq!(mobs.len(), 6);
    assert_eq!(
        mobs[5].time.unwrap().to_string(),
        "2020-01-05T00:02:30.0000000"
    );
}

#[test]
fn damaged_compressed_files_are_refused_not_crashed_on() {
    let crx = FIXTURES[0].2;
    // Cut short anywhere, flipped anywhere: always an error or a shorter
    // file, never a panic.
    for cut in (0..crx.len()).step_by(97) {
        let _ = decompress(&crx[..cut], std::io::sink());
        if let Ok(mut r) = read_obs(&crx[..cut]) {
            while let Ok(Some(_)) = r.next_epoch() {}
        }
    }
    for at in (0..crx.len()).step_by(13) {
        let mut damaged = crx.to_vec();
        damaged[at] = damaged[at].wrapping_add(17);
        let _ = decompress(&damaged[..], std::io::sink());
    }
    let err = decompress(&b"hello\n"[..], std::io::sink()).unwrap_err();
    assert!(err.to_string().contains("isn't a Hatanaka-compressed file"));
    let gz = read_obs(&[0x1f, 0x8b, 8, 0][..]).err().unwrap();
    assert!(gz.to_string().contains("gzip"));
}
