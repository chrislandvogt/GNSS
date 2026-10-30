//! Precise products from CODE (AIUB, the Bernese team's analysis centre).
//!
//! CODE publishes three operational series for GPS, GLONASS and Galileo, and
//! a multi-GNSS (MGEX) final series that adds BeiDou and QZSS, all in the
//! IGS long names (`COD0OPSFIN_20251660000_01D_05M_ORB.SP3.gz`):
//!
//! | Series | Where | Latency (seen 2026-10-29) |
//! |---|---|---|
//! | Final | `CODE/YYYY/`, gzipped | about 12 days |
//! | Rapid | `CODE/`, plain, until the final replaces it | under a day |
//! | Ultra-rapid | `CODE/`, plain, orbits and ERPs only | hours |
//! | MGEX final | `CODE_MGEX/CODE/YYYY/`, gzipped | weeks to months |
//!
//! A product for a day is looked for in order of quality, final first,
//! falling back while the better ones aren't out yet ([`candidates`]).

use crate::time::Day;

/// The series, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Latency {
    /// Final products: the best, after about 12 days.
    Final,
    /// Rapid products, within a day.
    Rapid,
    /// Ultra-rapid products, within hours; part predicted.
    UltraRapid,
}

impl Latency {
    /// Its name, in lower case.
    pub fn name(self) -> &'static str {
        match self {
            Latency::Final => "final",
            Latency::Rapid => "rapid",
            Latency::UltraRapid => "ultra-rapid",
        }
    }

    /// The citation CODE asks for (`CODE/0000_CODE.ACN`).
    pub fn citation(self) -> &'static str {
        match self {
            Latency::Final => {
                "Dach, R., S. Schaer, D. Arnold, E. Brockmann, M. Kalarus, C. Kobel, M. Lasser, P. Stebler, A. Jaeggi (2026). CODE final product series for the IGS. Published by Astronomical Institute, University of Bern. DOI 10.48620/97773."
            }
            Latency::Rapid => {
                "Dach, R., S. Schaer, D. Arnold, E. Brockmann, M. Kalarus, C. Kobel, M. Lasser, P. Stebler, A. Jaeggi (2026). CODE rapid product series for the IGS. Published by Astronomical Institute, University of Bern. DOI 10.48620/97772."
            }
            Latency::UltraRapid => {
                "Dach, R., S. Schaer, D. Arnold, E. Brockmann, M. Kalarus, C. Kobel, M. Lasser, P. Stebler, A. Jaeggi (2026). CODE ultra-rapid product series for the IGS. Published by Astronomical Institute, University of Bern. DOI 10.48620/97774."
            }
        }
    }
}

/// Which systems the products cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Constellations {
    /// GPS, GLONASS and Galileo: the operational series.
    Operational,
    /// Also BeiDou and QZSS: the MGEX series, finals only.
    Multi,
}

/// A kind of product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    /// Precise orbits, SP3.
    Orbit,
    /// Satellite and station clocks, RINEX clock.
    Clock,
    /// Earth rotation parameters.
    Erp,
    /// Observable-specific code and phase biases, Bias-SINEX.
    Bias,
    /// Global ionosphere maps, IONEX.
    Ionosphere,
}

/// A product file to look for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductFile {
    /// The kind of product.
    pub product: Product,
    /// Its series.
    pub latency: Latency,
    /// The constellations it covers.
    pub constellations: Constellations,
    /// The day it covers.
    pub day: Day,
    /// The path under the source's base URL.
    pub path: String,
}

impl ProductFile {
    /// The file's name.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The citation CODE asks for.
    pub fn citation(&self) -> &'static str {
        match self.constellations {
            Constellations::Operational => self.latency.citation(),
            Constellations::Multi => {
                "Dach, R., S. Schaer, D. Arnold, E. Brockmann, M. Kalarus, C. Kobel, M. Lasser, P. Stebler, A. Jaeggi (2026). CODE final product series for the IGS (MGEX). Published by Astronomical Institute, University of Bern. DOI 10.48620/97775."
            }
        }
    }
}

/// The file for a product, day and series, if CODE publishes one.
pub fn file(
    product: Product,
    latency: Latency,
    constellations: Constellations,
    day: Day,
) -> Option<ProductFile> {
    let tail = match (constellations, latency, product) {
        (Constellations::Multi, Latency::Final, p) => match p {
            Product::Orbit => "01D_05M_ORB.SP3",
            Product::Clock => "01D_30S_CLK.CLK",
            Product::Erp => "01D_12H_ERP.ERP",
            Product::Bias => "01D_01D_OSB.BIA",
            Product::Ionosphere => return None,
        },
        (Constellations::Multi, _, _) => return None,
        (_, Latency::UltraRapid, Product::Orbit) => "01D_05M_ORB.SP3",
        (_, Latency::UltraRapid, Product::Erp) => "01D_01D_ERP.ERP",
        (_, Latency::UltraRapid, _) => return None,
        (_, Latency::Final, Product::Clock) => "01D_05S_CLK.CLK",
        (_, Latency::Rapid, Product::Clock) => "01D_30S_CLK.CLK",
        (_, _, Product::Orbit) => "01D_05M_ORB.SP3",
        (_, _, Product::Erp) => "01D_01D_ERP.ERP",
        (_, _, Product::Bias) => "01D_01D_OSB.BIA",
        // The rapid IONEX file is the one CODE gzips.
        (_, _, Product::Ionosphere) => "01D_01H_GIM.INX",
    };
    let year = day.year();
    let stamp = format!("{}0000", day.yyyyddd());
    let path = match (constellations, latency) {
        (Constellations::Multi, _) => format!("CODE_MGEX/CODE/{year}/COD0MGXFIN_{stamp}_{tail}.gz"),
        (_, Latency::Final) => format!("CODE/{year}/COD0OPSFIN_{stamp}_{tail}.gz"),
        (_, Latency::Rapid) if product == Product::Ionosphere => {
            format!("CODE/COD0OPSRAP_{stamp}_{tail}.gz")
        }
        (_, Latency::Rapid) => format!("CODE/COD0OPSRAP_{stamp}_{tail}"),
        (_, Latency::UltraRapid) => format!("CODE/COD0OPSULT_{stamp}_{tail}"),
    };
    Some(ProductFile {
        product,
        latency,
        constellations,
        day,
        path,
    })
}

/// Days after a day's end before its final and rapid products are expected.
const FINAL_AFTER_DAYS: i64 = 14;
const RAPID_AFTER_DAYS: i64 = 1;

/// The files to try for a product and day, best first, leaving out series
/// that can't be out yet on `today`. The multi-GNSS series has finals only,
/// so when they're too new, the operational ones are tried after it.
pub fn candidates(
    product: Product,
    constellations: Constellations,
    day: Day,
    today: Day,
) -> Vec<ProductFile> {
    let age = day.days_until(today);
    let mut out = Vec::new();
    let series: &[Latency] = if age > FINAL_AFTER_DAYS {
        // Old enough that only finals are kept; rapids have been removed.
        &[Latency::Final]
    } else if age >= RAPID_AFTER_DAYS {
        &[Latency::Final, Latency::Rapid, Latency::UltraRapid]
    } else {
        &[Latency::UltraRapid]
    };
    if constellations == Constellations::Multi {
        out.extend(file(product, Latency::Final, Constellations::Multi, day));
    }
    for &latency in series {
        out.extend(file(product, latency, Constellations::Operational, day));
    }
    out
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    fn day(s: &str) -> Day {
        Day::parse(s).unwrap()
    }

    #[test]
    fn names_follow_codes_archive() {
        let d = day("2025-06-15");
        let f = |p, l, c| file(p, l, c, d).map(|f| f.path);
        use Constellations::*;
        use Latency::*;
        assert_eq!(
            f(Product::Orbit, Final, Operational).unwrap(),
            "CODE/2025/COD0OPSFIN_20251660000_01D_05M_ORB.SP3.gz"
        );
        assert_eq!(
            f(Product::Clock, Final, Operational).unwrap(),
            "CODE/2025/COD0OPSFIN_20251660000_01D_05S_CLK.CLK.gz"
        );
        assert_eq!(
            f(Product::Clock, Rapid, Operational).unwrap(),
            "CODE/COD0OPSRAP_20251660000_01D_30S_CLK.CLK"
        );
        assert_eq!(
            f(Product::Ionosphere, Rapid, Operational).unwrap(),
            "CODE/COD0OPSRAP_20251660000_01D_01H_GIM.INX.gz"
        );
        assert_eq!(
            f(Product::Orbit, UltraRapid, Operational).unwrap(),
            "CODE/COD0OPSULT_20251660000_01D_05M_ORB.SP3"
        );
        assert_eq!(f(Product::Clock, UltraRapid, Operational), None);
        assert_eq!(
            f(Product::Orbit, Final, Multi).unwrap(),
            "CODE_MGEX/CODE/2025/COD0MGXFIN_20251660000_01D_05M_ORB.SP3.gz"
        );
        assert_eq!(
            f(Product::Clock, Final, Multi).unwrap(),
            "CODE_MGEX/CODE/2025/COD0MGXFIN_20251660000_01D_30S_CLK.CLK.gz"
        );
        assert_eq!(f(Product::Orbit, Rapid, Multi), None);
    }

    #[test]
    fn the_series_tried_depend_on_the_days_age() {
        let d = day("2026-10-20");
        let series = |today: &str, c| -> Vec<Latency> {
            candidates(Product::Orbit, c, d, day(today))
                .iter()
                .map(|f| f.latency)
                .collect()
        };
        use Latency::*;
        assert_eq!(
            series("2026-10-20", Constellations::Operational),
            [UltraRapid]
        );
        assert_eq!(
            series("2026-10-22", Constellations::Operational),
            [Final, Rapid, UltraRapid]
        );
        assert_eq!(series("2026-12-01", Constellations::Operational), [Final]);
        let multi = candidates(Product::Orbit, Constellations::Multi, d, day("2026-12-01"));
        assert_eq!(multi[0].constellations, Constellations::Multi);
        assert_eq!(multi[1].constellations, Constellations::Operational);
        assert!(multi[0].citation().contains("97775"));
        assert!(multi[1].citation().contains("97773"));
    }
}
