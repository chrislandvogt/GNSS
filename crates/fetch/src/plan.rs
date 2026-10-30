//! What a set of occupations needs from outside the project.

use std::collections::{BTreeMap, BTreeSet};

use crate::products::{Constellations, Product};
use crate::time::{Day, Window};

/// Something to fetch.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Need {
    /// A CORS station's observations over a window.
    CorsObs {
        /// The station's four-character id.
        station: String,
        /// The span needed.
        window: Window,
    },
    /// Mixed-constellation broadcast navigation data for a day.
    Nav {
        /// The day.
        day: Day,
    },
    /// A precise product for a day.
    Product {
        /// The product.
        product: ProductKind,
        /// The day it covers.
        day: Day,
    },
    /// A named file from a `files` source, such as `igs20.atx`.
    File {
        /// The source's id.
        source: String,
        /// The file's name.
        name: String,
    },
}

/// [`Product`] and its constellations, ordered for sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProductKind {
    /// Orbits.
    Orbit,
    /// Clocks.
    Clock,
    /// Earth rotation parameters.
    Erp,
    /// Code and phase biases.
    Bias,
}

impl ProductKind {
    /// The product it is.
    pub fn product(self) -> Product {
        match self {
            ProductKind::Orbit => Product::Orbit,
            ProductKind::Clock => Product::Clock,
            ProductKind::Erp => Product::Erp,
            ProductKind::Bias => Product::Bias,
        }
    }
}

/// A span of observation and the CORS stations to go with it.
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    /// When the occupation observed.
    pub window: Window,
    /// The CORS stations to process it with, or candidates for them, best
    /// first.
    pub stations: Vec<String>,
    /// How many of `stations` to use: all of them if `None`. With a count,
    /// [`crate::Fetcher::choose_stations`] keeps the first that have data.
    pub count: Option<usize>,
}

impl Want {
    /// The CORS data it needs: its window padded by `pad` seconds, but not
    /// into a day it doesn't touch, which would fetch a day's files for a
    /// few minutes of margin.
    pub fn cors_window(&self, pad: i64) -> Window {
        let touched = self.window.days();
        let padded = self.window.padded(pad);
        Window::new(
            padded
                .start
                .max(touched.first().map_or(padded.start, |d| d.start())),
            padded
                .end
                .min(touched.last().map_or(padded.end, |d| d.window().end)),
        )
    }

    /// The stations to use: the first `count`, or all.
    pub fn chosen(&self) -> &[String] {
        let n = self
            .count
            .unwrap_or(self.stations.len())
            .min(self.stations.len());
        self.stations.get(..n).unwrap_or_default()
    }
}

/// What to fetch beyond the CORS files.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Seconds of CORS data either side of each window.
    pub pad: i64,
    /// Precise orbits, clocks and ERPs.
    pub precise: bool,
    /// Which constellations precise products should cover.
    pub constellations: Constellations,
    /// The ANTEX file.
    pub antex: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pad: 300,
            precise: false,
            constellations: Constellations::Operational,
            antex: true,
        }
    }
}

/// The needs for a set of wants: per station, the union of its padded
/// windows; navigation data for every day touched; precise products and
/// ANTEX if asked for. Sorted, without repeats.
pub fn plan(wants: &[Want], options: &Options) -> Vec<Need> {
    let mut by_station: BTreeMap<&str, Vec<Window>> = BTreeMap::new();
    let mut days = BTreeSet::new();
    for want in wants {
        let window = want.cors_window(options.pad);
        days.extend(want.window.days());
        for station in want.chosen() {
            by_station.entry(station).or_default().push(window);
        }
    }
    let mut needs = BTreeSet::new();
    for (station, windows) in by_station {
        for window in union(windows) {
            needs.insert(Need::CorsObs {
                station: station.to_uppercase(),
                window,
            });
        }
    }
    for &day in &days {
        needs.insert(Need::Nav { day });
        if options.precise {
            for product in [ProductKind::Orbit, ProductKind::Clock, ProductKind::Erp] {
                needs.insert(Need::Product { product, day });
            }
        }
    }
    if options.antex && !wants.is_empty() {
        needs.insert(Need::File {
            source: "igs-files".into(),
            name: "igs20.atx".into(),
        });
    }
    needs.into_iter().collect()
}

/// Overlapping or touching windows merged.
fn union(mut windows: Vec<Window>) -> Vec<Window> {
    windows.sort();
    let mut out: Vec<Window> = Vec::new();
    for w in windows {
        match out.last_mut() {
            Some(last) if w.start <= last.end => last.end = last.end.max(w.end),
            _ => out.push(w),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;
    use crate::time::parse_time;

    fn t(s: &str) -> i64 {
        parse_time(s).unwrap()
    }

    #[test]
    fn overlapping_occupations_share_cors_files() {
        let wants = [
            Want {
                window: Window::new(t("2025-06-15T01:00:00Z"), t("2025-06-15T03:00:00Z")),
                stations: vec!["SYDN".into(), "str1".into()],
                count: None,
            },
            Want {
                window: Window::new(t("2025-06-15T02:30:00Z"), t("2025-06-15T05:00:00Z")),
                stations: vec!["SYDN".into()],
                count: None,
            },
        ];
        let needs = plan(&wants, &Options::default());
        let cors: Vec<&Need> = needs
            .iter()
            .filter(|n| matches!(n, Need::CorsObs { .. }))
            .collect();
        assert_eq!(
            cors,
            [
                &Need::CorsObs {
                    station: "STR1".into(),
                    window: Window::new(t("2025-06-15T00:55:00Z"), t("2025-06-15T03:05:00Z")),
                },
                &Need::CorsObs {
                    station: "SYDN".into(),
                    window: Window::new(t("2025-06-15T00:55:00Z"), t("2025-06-15T05:05:00Z")),
                },
            ]
        );
        assert!(needs.contains(&Need::Nav {
            day: Day::parse("2025-06-15").unwrap()
        }));
        assert!(
            needs
                .iter()
                .any(|n| matches!(n, Need::File { name, .. } if name == "igs20.atx"))
        );
        assert!(!needs.iter().any(|n| matches!(n, Need::Product { .. })));
    }

    #[test]
    fn padding_stays_within_the_days_observed() {
        let wants = [Want {
            window: Window::new(t("2025-06-15T00:01:00Z"), t("2025-06-15T23:58:00Z")),
            stations: vec!["SYDN".into()],
            count: None,
        }];
        let needs = plan(&wants, &Options::default());
        assert_eq!(
            needs[0],
            Need::CorsObs {
                station: "SYDN".into(),
                window: Window::new(t("2025-06-15T00:00:00Z"), t("2025-06-16T00:00:00Z")),
            }
        );
        assert_eq!(
            needs
                .iter()
                .filter(|n| matches!(n, Need::Nav { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn precise_products_cover_every_day_touched() {
        let wants = [Want {
            window: Window::new(t("2025-06-15T22:00:00Z"), t("2025-06-16T02:00:00Z")),
            stations: vec!["SYDN".into()],
            count: None,
        }];
        let options = Options {
            precise: true,
            antex: false,
            ..Options::default()
        };
        let needs = plan(&wants, &options);
        let products = needs
            .iter()
            .filter(|n| matches!(n, Need::Product { .. }))
            .count();
        assert_eq!(products, 6);
        assert_eq!(
            needs
                .iter()
                .filter(|n| matches!(n, Need::Nav { .. }))
                .count(),
            2
        );
        assert!(plan(&[], &Options::default()).is_empty());
    }
}
