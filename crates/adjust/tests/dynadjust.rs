//! The adjustment against DynAdjust, Geoscience Australia's least squares
//! adjustment, on synthetic networks with blunders (ROADMAP.md M2 exit
//! criteria: coordinates within 0.1 mm, the variance factor within 0.1%, and
//! the same flagged outliers).
//!
//! DynAdjust's results are in `tests/data/dynadjust`, made by
//! `tests/oracle/dynadjust.py`, which writes these networks as DynaML through
//! the ignored test below and runs DynAdjust on them. CI needs only the
//! results.
//!
//! DynAdjust flags a component when its N-stat, the residual over its own
//! standard deviation, exceeds the normal quantile at its confidence
//! interval (1.96 at 95%). That's our `standardised` residual, not the
//! w-test, which also uses the correlations.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::fmt::Write as _;

use gnss_adjust::synthetic::{Spec, Synthetic};
use gnss_adjust::{Datum, Element, Network, Options, Scaling};
use gnss_datum::GRS80;
use nalgebra::Vector3;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

/// DynAdjust's critical value at its default 95%: the normal quantile at 0.975.
const CRITICAL: f64 = 1.959_963_984_540_054;

struct Oracle {
    name: &'static str,
    network: Network,
    datum: Datum,
    results: &'static str,
}

/// Adds a bias, given on local east, north and up axes at the element's
/// first mark, to one element of one observation.
fn blunder(network: &mut Network, observation: usize, element: usize, enu: Vector3<f64>) {
    let first = network.observations()[observation].elements[element].first_mark();
    let at = GRS80
        .ecef_to_geodetic(&network.marks()[first].approximate.unwrap())
        .unwrap();
    let ecef = at.enu_rotation().transpose() * enu;
    match &mut network.observations_mut()[observation].elements[element] {
        Element::Difference { vector, .. } => *vector += ecef,
        Element::Position { position, .. } => *position += ecef,
    }
}

/// The index of the `nth` observation with at least `elements` elements.
fn nth_with(network: &Network, elements: usize, nth: usize) -> usize {
    network
        .observations()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.elements.len() >= elements)
        .nth(nth)
        .unwrap()
        .0
}

fn oracles() -> Vec<Oracle> {
    // Clusters of three vectors from each base, with weighted control.
    let synthetic = Synthetic::generate(&Spec {
        marks: 30,
        neighbours: 4,
        cluster: 3,
        control: 4,
        seed: 41,
        ..Spec::default()
    })
    .unwrap();
    let mut clusters = synthetic.observe(410);
    let (a, b) = (nth_with(&clusters, 3, 2), nth_with(&clusters, 2, 6));
    blunder(&mut clusters, a, 1, Vector3::new(0.0, 0.0, 0.04));
    blunder(&mut clusters, b, 0, Vector3::new(0.03, 0.0, 0.0));

    // Independent baselines, weighted control and a fixed mark.
    let synthetic = Synthetic::generate(&Spec {
        marks: 25,
        neighbours: 3,
        control: 3,
        seed: 42,
        ..Spec::default()
    })
    .unwrap();
    let mut baselines = synthetic.observe(420);
    // Held at its true position, as control would be.
    baselines
        .set_approximate(6, Some(synthetic.truth[6]))
        .unwrap();
    blunder(&mut baselines, 7, 0, Vector3::new(0.0, -0.025, 0.0));

    // A larger network over 50 km, in pairs of vectors.
    let synthetic = Synthetic::generate(&Spec {
        marks: 120,
        neighbours: 5,
        radius_m: 50_000.0,
        cluster: 2,
        control: 6,
        seed: 43,
        ..Spec::default()
    })
    .unwrap();
    let mut larger = synthetic.observe(430);
    let (a, b) = (nth_with(&larger, 2, 40), nth_with(&larger, 1, 90));
    blunder(&mut larger, a, 1, Vector3::new(0.0, 0.0, -0.06));
    blunder(&mut larger, b, 0, Vector3::new(0.02, 0.02, 0.0));

    vec![
        Oracle {
            name: "clusters",
            network: clusters,
            datum: Datum::Constrained { fixed: Vec::new() },
            results: include_str!("data/dynadjust/clusters.csv"),
        },
        Oracle {
            name: "baselines",
            network: baselines,
            datum: Datum::Constrained { fixed: vec![6] },
            results: include_str!("data/dynadjust/baselines.csv"),
        },
        Oracle {
            name: "larger",
            network: larger,
            datum: Datum::Constrained { fixed: Vec::new() },
            results: include_str!("data/dynadjust/larger.csv"),
        },
    ]
}

/// DynaML station and measurement files for a network.
#[cfg(not(target_arch = "wasm32"))]
fn dynaml(oracle: &Oracle) -> (String, String) {
    let network = &oracle.network;
    let fixed: &[usize] = match &oracle.datum {
        Datum::Constrained { fixed } => fixed,
        _ => &[],
    };
    let header = |kind: &str| {
        format!(
            "<?xml version=\"1.0\"?>\n<DnaXmlFormat type=\"{kind}\" referenceframe=\"GDA2020\" \
             epoch=\"01.01.2020\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
             xsi:noNamespaceSchemaLocation=\"DynaML.xsd\">\n"
        )
    };
    let mut stations = header("Station File");
    for (m, mark) in network.marks().iter().enumerate() {
        let x = mark.approximate.unwrap();
        let constraints = if fixed.contains(&m) { "CCC" } else { "FFF" };
        writeln!(
            stations,
            "  <DnaStation>\n    <Name>{0}</Name>\n    <Constraints>{constraints}</Constraints>\n    \
             <Type>XYZ</Type>\n    <StationCoord>\n      <Name>{0}</Name>\n      \
             <XAxis>{1}</XAxis>\n      <YAxis>{2}</YAxis>\n      <Height>{3}</Height>\n    \
             </StationCoord>\n    <Description>Synthetic</Description>\n  </DnaStation>",
            mark.name, x.x, x.y, x.z
        )
        .unwrap();
    }
    stations.push_str("</DnaXmlFormat>\n");

    let mut measurements = header("Measurement File");
    for o in network.observations() {
        let n = o.elements.len();
        let points = matches!(o.elements[0], Element::Position { .. });
        let kind = match (points, n) {
            (true, _) => "Y",
            (false, 1) => "G",
            (false, _) => "X",
        };
        measurements.push_str(&format!(
            "  <DnaMeasurement>\n    <Type>{kind}</Type>\n    <Ignore/>\n    \
             <ReferenceFrame>GDA2020</ReferenceFrame>\n    <Epoch>01.01.2020</Epoch>\n    \
             <Vscale>1.000</Vscale>\n    <Pscale>1.000</Pscale>\n    <Lscale>1.000</Lscale>\n    \
             <Hscale>1.000</Hscale>\n"
        ));
        if points {
            measurements.push_str("    <Coords>XYZ</Coords>\n");
        }
        if kind != "G" {
            writeln!(measurements, "    <Total>{n}</Total>").unwrap();
        }
        let c = &o.covariance;
        for (p, element) in o.elements.iter().enumerate() {
            let (value, tag, covariance_tag) = match *element {
                Element::Difference { from, to, vector } => {
                    writeln!(
                        measurements,
                        "    <First>{}</First>\n    <Second>{}</Second>",
                        network.marks()[from].name,
                        network.marks()[to].name
                    )
                    .unwrap();
                    (vector, "GPSBaseline", "GPSCovariance")
                }
                Element::Position { mark, position } => {
                    writeln!(
                        measurements,
                        "    <First>{}</First>",
                        network.marks()[mark].name
                    )
                    .unwrap();
                    (position, "Clusterpoint", "PointCovariance")
                }
            };
            let r = 3 * p;
            writeln!(
                measurements,
                "    <{tag}>\n      <X>{}</X>\n      <Y>{}</Y>\n      <Z>{}</Z>\n      \
                 <SigmaXX>{:e}</SigmaXX>\n      <SigmaXY>{:e}</SigmaXY>\n      \
                 <SigmaXZ>{:e}</SigmaXZ>\n      <SigmaYY>{:e}</SigmaYY>\n      \
                 <SigmaYZ>{:e}</SigmaYZ>\n      <SigmaZZ>{:e}</SigmaZZ>",
                value.x,
                value.y,
                value.z,
                c[(r, r)],
                c[(r, r + 1)],
                c[(r, r + 2)],
                c[(r + 1, r + 1)],
                c[(r + 1, r + 2)],
                c[(r + 2, r + 2)]
            )
            .unwrap();
            // The covariance with each later element.
            for q in p + 1..n {
                writeln!(measurements, "      <{covariance_tag}>").unwrap();
                for i in 0..3 {
                    for j in 0..3 {
                        writeln!(
                            measurements,
                            "        <m{}{}>{:e}</m{}{}>",
                            i + 1,
                            j + 1,
                            c[(r + i, 3 * q + j)],
                            i + 1,
                            j + 1
                        )
                        .unwrap();
                    }
                }
                writeln!(measurements, "      </{covariance_tag}>").unwrap();
            }
            writeln!(measurements, "    </{tag}>").unwrap();
        }
        measurements.push_str("  </DnaMeasurement>\n");
    }
    measurements.push_str("</DnaXmlFormat>\n");
    (stations, measurements)
}

/// Writes the networks as DynaML into `GNSS_DYNAML_DIR`, for
/// `tests/oracle/dynadjust.py`.
#[test]
#[ignore = "writes DynaML for tests/oracle/dynadjust.py"]
#[cfg(not(target_arch = "wasm32"))]
fn write_dynaml() {
    let directory = std::path::PathBuf::from(std::env::var("GNSS_DYNAML_DIR").unwrap());
    for oracle in oracles() {
        let (stations, measurements) = dynaml(&oracle);
        std::fs::write(directory.join(format!("{}stn.xml", oracle.name)), stations).unwrap();
        std::fs::write(
            directory.join(format!("{}msr.xml", oracle.name)),
            measurements,
        )
        .unwrap();
    }
}

/// DynAdjust's results for one network.
#[derive(Default)]
struct Results {
    chisq: f64,
    dof: usize,
    variance_factor: f64,
    bounds: (f64, f64),
    /// By station name: X, Y, Z, horizontal and vertical PU.
    stations: BTreeMap<String, [f64; 5]>,
    /// By measurement and element: first and second station, and N-stat in X, Y and Z.
    nstat: Vec<(usize, usize, String, String, [f64; 3])>,
}

fn parse(text: &str) -> Results {
    let mut results = Results::default();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let fields: Vec<&str> = line.split(',').collect();
        let number = |i: usize| fields[i].parse::<f64>().unwrap();
        match fields[0] {
            "statistics" => {
                results.chisq = number(1);
                results.dof = fields[2].parse().unwrap();
                results.variance_factor = number(3);
                results.bounds = (number(4), number(5));
            }
            "station" => {
                results.stations.insert(
                    fields[1].to_owned(),
                    [number(2), number(3), number(4), number(5), number(6)],
                );
            }
            "nstat" => results.nstat.push((
                fields[1].parse().unwrap(),
                fields[2].parse().unwrap(),
                fields[3].to_owned(),
                fields[4].to_owned(),
                [number(5), number(6), number(7)],
            )),
            other => panic!("unknown record {other}"),
        }
    }
    results
}

#[test]
fn matches_dynadjust() {
    for oracle in oracles() {
        let name = oracle.name;
        let theirs = parse(oracle.results);
        let ours = oracle
            .network
            .adjust(&oracle.datum, &Options::default())
            .unwrap();
        let statistics = ours.statistics();
        let marks = oracle.network.marks();

        // The fit: χ², degrees of freedom, σ̂₀² and the global test's bounds.
        assert_eq!(statistics.degrees_of_freedom, theirs.dof, "{name}");
        let relative = |a: f64, b: f64| (a - b).abs() / b.abs();
        assert!(
            relative(statistics.sum_of_squares, theirs.chisq) < 1e-6,
            "{name}"
        );
        let variance_factor = statistics.variance_factor.unwrap();
        assert!(
            relative(variance_factor, theirs.variance_factor) < 1e-6,
            "{name}"
        );
        let global = statistics.global_test.as_ref().unwrap();
        let r = statistics.degrees_of_freedom as f64;
        assert!(relative(global.lower / r, theirs.bounds.0) < 1e-9, "{name}");
        assert!(relative(global.upper / r, theirs.bounds.1) < 1e-9, "{name}");

        // Coordinates, and PU for marks that aren't fixed (DynAdjust holds
        // those to 1 µm).
        let mut largest = [0.0_f64; 3];
        for (m, mark) in ours.marks().iter().enumerate() {
            let [x, y, z, horizontal, vertical] = theirs.stations[&marks[m].name];
            let difference = (mark.position - Vector3::new(x, y, z)).norm();
            largest[0] = largest[0].max(difference);
            assert!(
                difference < 1e-6,
                "{name} {}: {difference} m",
                marks[m].name
            );
            if !mark.fixed {
                let pu = ours.positional_uncertainty(m, Scaling::APriori).unwrap();
                let pu_difference =
                    relative(pu.horizontal_95, horizontal).max(relative(pu.vertical_95, vertical));
                largest[1] = largest[1].max(pu_difference);
                assert!(pu_difference < 1e-5, "{name} {}: PU {pu:?}", marks[m].name);
            }
        }

        // N-stats, and the outliers they flag. DynAdjust holds fixed marks
        // only to 1 µm, so where there are any, the control moves the whole
        // network by a fraction of a micrometre and N-stats by a few parts
        // in a million.
        let tolerance = match &oracle.datum {
            Datum::Constrained { fixed } if !fixed.is_empty() => 1e-4,
            _ => 1e-6,
        };
        let mut flagged = (0, 0);
        for (observation, element, first, second, nstat) in &theirs.nstat {
            let o = &oracle.network.observations()[*observation];
            let names: Vec<&str> = match o.elements[*element] {
                Element::Difference { from, to, .. } => {
                    vec![marks[from].name.as_str(), marks[to].name.as_str()]
                }
                Element::Position { mark, .. } => vec![marks[mark].name.as_str()],
            };
            assert_eq!(names[0], first, "{name} observation {observation}");
            assert_eq!(names.get(1).copied().unwrap_or(""), second);
            let result = &ours.observations()[*observation].elements[*element];
            for (s, &n) in result.standardised.iter().zip(nstat) {
                let s = s.unwrap();
                let difference = (s - n).abs() / n.abs().max(1.0);
                largest[2] = largest[2].max(difference);
                assert!(difference < tolerance, "{name} {observation}: {s} vs {n}");
                // A flag near the critical value could go either way on rounding.
                if (n.abs() - CRITICAL).abs() > 1e-6 {
                    assert_eq!(s.abs() > CRITICAL, n.abs() > CRITICAL);
                }
                flagged.0 += usize::from(n.abs() > CRITICAL);
                flagged.1 += usize::from(s.abs() > CRITICAL);
            }
        }
        assert_eq!(flagged.0, flagged.1);
        let components: usize = oracle
            .network
            .observations()
            .iter()
            .map(|o| 3 * o.elements.len())
            .sum();
        assert_eq!(
            theirs.nstat.len() * 3,
            components,
            "{name}: every component compared"
        );
        println!(
            "{name}: largest differences {:.1e} m in position, {:.1e} relative in PU, {:.1e} in \
             N-stat; {} of {components} components flagged by both; σ̂₀² {variance_factor:.4}",
            largest[0], largest[1], largest[2], flagged.0
        );
    }
}
