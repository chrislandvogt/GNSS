//! Observation codes and carrier frequencies.

/// The speed of light, m/s, as GNSS defines it.
pub const SPEED_OF_LIGHT: f64 = 299_792_458.0;

/// What an observation code measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A pseudorange (code), in metres: RINEX `C`, or `P` in RINEX 2.
    Code,
    /// A carrier phase, in cycles: `L`.
    Phase,
    /// A Doppler shift, in hertz: `D`.
    Doppler,
    /// A signal strength: `S`.
    Strength,
}

/// An observation code split into what it measures and its frequency band,
/// such as `L1C` (RINEX 3) or `P2` (RINEX 2).
pub fn parse_code(code: &str) -> Option<(Kind, u8)> {
    let mut chars = code.chars();
    let kind = match chars.next()? {
        'C' | 'P' => Kind::Code,
        'L' => Kind::Phase,
        'D' => Kind::Doppler,
        'S' => Kind::Strength,
        _ => return None,
    };
    let band = chars.next()?.to_digit(10)?;
    Some((kind, u8::try_from(band).ok()?))
}

/// A band's carrier frequency in hertz, for a system's letter. GLONASS's
/// first two bands are FDMA: they need the satellite's frequency channel.
pub fn frequency(system: char, band: u8, glonass_channel: Option<i8>) -> Option<f64> {
    let mhz = match (system, band) {
        ('G' | 'J' | 'S', 1) => 1575.42,
        ('G' | 'J', 2) => 1227.60,
        ('G' | 'J' | 'S', 5) => 1176.45,
        ('J', 6) => 1278.75,
        ('R', 1) => 1602.0 + 0.5625 * f64::from(glonass_channel?),
        ('R', 2) => 1246.0 + 0.4375 * f64::from(glonass_channel?),
        ('R', 3) => 1202.025,
        ('R', 4) => 1600.995,
        ('R', 6) => 1248.06,
        ('E', 1) => 1575.42,
        ('E', 5) => 1176.45,
        ('E', 7) => 1207.14,
        ('E', 8) => 1191.795,
        ('E', 6) => 1278.75,
        ('C', 1) => 1575.42,
        ('C', 2) => 1561.098,
        ('C', 5) => 1176.45,
        ('C', 7) => 1207.14,
        ('C', 6) => 1268.52,
        ('C', 8) => 1191.795,
        ('I', 5) => 1176.45,
        ('I', 9) => 2492.028,
        _ => return None,
    };
    Some(mhz * 1e6)
}

/// The dual-frequency pairs used for slips and multipath, in order of
/// preference, for a system.
pub fn pairs(system: char) -> &'static [(u8, u8)] {
    match system {
        'G' | 'J' => &[(1, 2), (1, 5)],
        'R' => &[(1, 2), (1, 3)],
        'E' => &[(1, 5), (1, 7)],
        'C' => &[(2, 7), (2, 6), (1, 5)],
        'I' => &[(5, 9)],
        'S' => &[(1, 5)],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{Kind, frequency, parse_code};

    #[test]
    fn codes_and_frequencies() {
        assert_eq!(parse_code("L1C"), Some((Kind::Phase, 1)));
        assert_eq!(parse_code("P2"), Some((Kind::Code, 2)));
        assert_eq!(parse_code("S5Q"), Some((Kind::Strength, 5)));
        assert_eq!(parse_code("X1"), None);
        assert_eq!(parse_code("L"), None);
        assert_eq!(frequency('G', 1, None), Some(1575.42e6));
        assert_eq!(frequency('R', 1, Some(-7)), Some(1598.0625e6));
        assert_eq!(frequency('R', 2, None), None);
        assert_eq!(frequency('C', 7, None), Some(1207.14e6));
        assert_eq!(frequency('G', 7, None), None);
    }
}
