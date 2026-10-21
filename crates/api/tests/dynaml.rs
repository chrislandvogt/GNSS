//! DynAdjust's sample GNSS network, read from DynaML, built and adjusted, and
//! compared with DynAdjust 1.4.1's import, transformation and adjustment of
//! the same files (`tests/oracle/dynadjust_sample.py`).
//!
//! It's real data: baselines in ITRF2008 and ITRF2014 at their own epochs,
//! with variance scaling, and a cluster of control positions in GDA2020. So
//! it checks the reader, the transformation of vectors to GDA2020, the
//! scaling, and the adjustment, together.

#![allow(clippy::unwrap_used)]

use gnss_adjust::Options;
use gnss_api::build::{DatumChoice, Settings, build};
use gnss_datum::{FrameId, TransformationGraph};
use gnss_formats::{Survey, dynaml};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const STATIONS: &str = include_str!("../../formats/tests/data/dynaml/gnss-networkstn.xml");
const MEASUREMENTS: &str = include_str!("../../formats/tests/data/dynaml/gnss-networkmsr.xml");
const ORACLE: &str = include_str!("data/dynadjust/gnss-network.csv");

fn survey() -> Survey {
    let mut survey = dynaml::read("gnss-networkstn.xml", STATIONS).unwrap();
    survey.extend(dynaml::read("gnss-networkmsr.xml", MEASUREMENTS).unwrap());
    survey
}

fn fields(line: &str) -> Vec<&str> {
    line.split(',').collect()
}

fn num(text: &str) -> f64 {
    text.parse().unwrap()
}

#[test]
fn reads_the_sample() {
    let survey = survey();
    assert_eq!(survey.stations.len(), 43);
    assert_eq!(survey.measurements.len(), 131);
    let first = &survey.measurements[0];
    assert_eq!(first.label(), "324900360 to BEEC");
    assert_eq!(first.reference.frame, FrameId::Itrf2008);
    assert_eq!(first.scaling.v, 10.0);
    assert_eq!(first.origin.line, 21);
    assert!(survey.notes.is_empty(), "{:?}", survey.notes);
}

#[test]
fn matches_dynadjust_on_its_sample_network() {
    let survey = survey();
    let settings = Settings {
        frame: FrameId::Gda2020,
        epoch: None,
        datum: DatumChoice::Constrained,
    };
    let built = build(&survey, &settings, TransformationGraph::builtin()).unwrap();
    let adjustment = built
        .network
        .adjust(
            &built.datum,
            &Options {
                edits: built.edits.clone(),
                ..Options::default()
            },
        )
        .unwrap();

    // Measurements after transformation and scaling, and N-statistics.
    let elements: Vec<_> = built
        .network
        .observations()
        .iter()
        .zip(adjustment.observations())
        .flat_map(|(o, r)| {
            o.elements.iter().enumerate().map(move |(k, e)| {
                let block = o.covariance.fixed_view::<3, 3>(3 * k, 3 * k).into_owned();
                (*e, block, r.elements[k].standardised)
            })
        })
        .collect();
    let mut rows = 0;
    let (mut worst_value, mut worst_sigma, mut worst_n): (f64, f64, f64) = (0.0, 0.0, 0.0);
    for line in ORACLE.lines().filter(|l| l.starts_with("element,")) {
        let f = fields(line);
        let (element, block, standardised) = &elements[rows];
        rows += 1;
        let value = match element {
            gnss_adjust::Element::Difference { vector, .. } => vector,
            gnss_adjust::Element::Position { position, .. } => position,
        };
        for axis in 0..3 {
            worst_value = worst_value.max((value[axis] - num(f[6 + axis])).abs());
        }
        let sigmas = [
            block[(0, 0)],
            block[(0, 1)],
            block[(0, 2)],
            block[(1, 1)],
            block[(1, 2)],
            block[(2, 2)],
        ];
        for (k, s) in sigmas.iter().enumerate() {
            worst_sigma = worst_sigma.max((s - num(f[9 + k])).abs() / block[(0, 0)]);
        }
        for axis in 0..3 {
            let ours = standardised[axis].unwrap();
            worst_n = worst_n.max((ours - num(f[15 + axis])).abs());
        }
    }
    assert_eq!(rows, elements.len());
    eprintln!(
        "transformed values within {worst_value:.1e} m, scaled covariances within \
         {worst_sigma:.1e} relative, N-statistics within {worst_n:.1e}"
    );
    // The values differ by DynAdjust's decimal years, which are about half a
    // day from ours (0.002 mas of plate motion), and the covariances by the
    // rotation DynAdjust leaves out of them.
    assert!(worst_value < 1e-6, "measurements differ by {worst_value} m");
    assert!(worst_sigma < 3e-7, "covariances differ by {worst_sigma}");
    assert!(worst_n < 3e-4, "N-statistics differ by {worst_n}");

    // The same components exceed DynAdjust's 95% critical value.
    let critical = 1.959_963_984_540_054;
    let flagged = |n: f64| n.abs() > critical;
    let ours: Vec<bool> = elements
        .iter()
        .flat_map(|(_, _, s)| s.iter().map(|n| flagged(n.unwrap())))
        .collect();
    let theirs: Vec<bool> = ORACLE
        .lines()
        .filter(|l| l.starts_with("element,"))
        .flat_map(|l| {
            let f = fields(l);
            (0..3).map(move |axis| flagged(num(f[15 + axis])))
        })
        .collect();
    assert_eq!(ours, theirs);
    assert_eq!(ours.iter().filter(|&&f| f).count(), 10);

    // The global statistics.
    let statistics = adjustment.statistics();
    let line = ORACLE
        .lines()
        .find(|l| l.starts_with("statistics,"))
        .unwrap();
    let f = fields(line);
    assert_eq!(statistics.degrees_of_freedom, num(f[2]) as usize);
    let relative = (statistics.sum_of_squares - num(f[1])).abs() / num(f[1]);
    assert!(relative < 1e-6, "chi-square differs by {relative}");

    // Adjusted coordinates.
    let mut worst_xyz: f64 = 0.0;
    for line in ORACLE.lines().filter(|l| l.starts_with("station,")) {
        let f = fields(line);
        let mark = built.network.mark(f[1]).unwrap();
        let ours = adjustment.marks()[mark].position;
        for axis in 0..3 {
            worst_xyz = worst_xyz.max((ours[axis] - num(f[2 + axis])).abs());
        }
    }
    eprintln!("adjusted coordinates within {worst_xyz:.1e} m");
    assert!(worst_xyz < 2e-6, "coordinates differ by {worst_xyz} m");

    // How the measurements got to GDA2020: GA's direct transformation from
    // ITRF2008, and the plate motion model from ITRF2014, at each epoch.
    let count = |frame: FrameId| -> usize {
        built
            .transformations
            .iter()
            .filter(|t| t.from.frame == frame)
            .map(|t| t.measurements)
            .sum()
    };
    assert_eq!(
        (count(FrameId::Itrf2008), count(FrameId::Itrf2014)),
        (71, 59)
    );
    for t in &built.transformations {
        let expected = match t.from.frame {
            FrameId::Itrf2008 => "ITRF2008 to GDA2020 (GA)",
            _ => "ITRF2014 to GDA2020 (1)",
        };
        assert_eq!(t.steps.len(), 1);
        assert!(t.steps[0].contains(expected), "{}", t.steps[0]);
    }
}
