//! Set-ups as nuisance parameters (ARCHITECTURE.md section 10.1): each
//! occupation's centring and height errors, shared by every vector measured
//! from it, are estimated with a prior. The dense oracle in `adjustment.rs`
//! checks the arithmetic; these check the behaviour.

#![allow(clippy::unwrap_used)]

use gnss_adjust::synthetic::{Spec, Synthetic};
use gnss_adjust::{AdjustError, Datum, Element, Network, Observation, Options, Subject};
use gnss_datum::GRS80;
use nalgebra::{Matrix3, Vector3};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const CONSTRAINED: Datum = Datum::Constrained { fixed: Vec::new() };

fn synthetic() -> Synthetic {
    Synthetic::generate(&Spec {
        marks: 25,
        neighbours: 4,
        cluster: 3,
        control: 3,
        setups: Some((0.002, 0.004)),
        seed: 71,
        ..Spec::default()
    })
    .unwrap()
}

#[test]
fn statistics_follow_their_distributions_with_setup_errors() {
    let synthetic = synthetic();
    let draws = 100;
    let (mut variance_factors, mut w_sum, mut w_squares, mut count) = (0.0, 0.0, 0.0, 0.0);
    let (mut prior_squares, mut prior_count) = (0.0, 0.0);
    for seed in 0..draws {
        let adjustment = synthetic
            .observe(2000 + seed)
            .adjust(&CONSTRAINED, &Options::default())
            .unwrap();
        variance_factors += adjustment.statistics().variance_factor.unwrap();
        for c in adjustment
            .observations()
            .iter()
            .flat_map(|o| &o.elements)
            .flat_map(|e| &e.components)
        {
            if let Some(w) = c.w {
                w_sum += w;
                w_squares += w * w;
                count += 1.0;
            }
        }
        for c in adjustment.setups().iter().flat_map(|s| &s.components) {
            if let Some(w) = c.w {
                prior_squares += w * w;
                prior_count += 1.0;
            }
        }
    }
    let mean = variance_factors / draws as f64;
    assert!((mean - 1.0).abs() < 0.04, "mean σ̂₀² {mean}");
    let w_mean = w_sum / count;
    assert!(w_mean.abs() < 0.04, "{w_mean}");
    assert!(
        (w_squares / count - 1.0).abs() < 0.06,
        "{}",
        w_squares / count
    );
    // The priors' w-tests are N(0, 1) too.
    assert!(prior_count > 1000.0);
    assert!(
        (prior_squares / prior_count - 1.0).abs() < 0.08,
        "{}",
        prior_squares / prior_count
    );
}

#[test]
fn a_misread_height_shows_on_its_setup() {
    let synthetic = synthetic();
    // A cluster's base set-up, shared by its vectors, too high by three of
    // its minimal detectable biases: every vector from it is short in height.
    let observation = synthetic
        .exact()
        .observations()
        .iter()
        .position(|o| o.elements.len() == 3)
        .unwrap();
    let Element::Difference { from, setups, .. } =
        synthetic.exact().observations()[observation].elements[0]
    else {
        unreachable!()
    };
    let setup = setups[0].unwrap();
    let exact = synthetic
        .exact()
        .adjust(&CONSTRAINED, &Options::default())
        .unwrap();
    let blunder = 3.0 * exact.setups()[setup].components[2].mdb.unwrap();
    let up = GRS80
        .ecef_to_geodetic(&synthetic.exact().marks()[from].approximate.unwrap())
        .unwrap()
        .enu_rotation()
        .row(2)
        .transpose();
    let mut network = synthetic.observe(77);
    for element in &mut network.observations_mut()[observation].elements {
        if let Element::Difference { vector, .. } = element {
            *vector -= up * blunder;
        }
    }
    let adjustment = network.adjust(&CONSTRAINED, &Options::default()).unwrap();
    let suspects = adjustment.suspects();
    assert_eq!(suspects[0].subject, Subject::Setup(setup), "{suspects:?}");
    // The set-up's height error is estimated upward.
    let estimated = &adjustment.setups()[setup].components[2];
    assert!(estimated.residual > 0.0 && estimated.w.unwrap() > adjustment.criteria().w);
}

#[test]
fn loops_allow_for_setup_errors() {
    // Loops through different occupations of a mark carry their set-ups'
    // errors, so their tests are χ²(3) only with the priors in.
    let synthetic = synthetic();
    let draws = 60;
    let mut means = Vec::new();
    for seed in 0..draws {
        let closures = synthetic
            .observe(4000 + seed)
            .closures(&Options::default())
            .unwrap();
        means.push(
            closures.loops.iter().map(|l| l.statistic).sum::<f64>() / closures.loops.len() as f64,
        );
    }
    let n = means.len() as f64;
    let mean = means.iter().sum::<f64>() / n;
    let variance = means.iter().map(|m| (m - mean) * (m - mean)).sum::<f64>() / (n - 1.0);
    let standard_error = libm::sqrt(variance / n);
    assert!(
        (mean - 3.0).abs() < 4.0 * standard_error,
        "mean {mean} ± {standard_error}"
    );
}

#[test]
fn setups_must_be_on_their_marks() {
    let mut network = Network::new(gnss_datum::FrameId::Gda2020, None).unwrap();
    let base = Vector3::new(-4_474_000.0, 2_680_000.0, -3_667_000.0);
    let a = network.add_mark("A", Some(base)).unwrap();
    let b = network.add_mark("B", None).unwrap();
    let covariance = Matrix3::from_diagonal(&Vector3::new(1e-5, 1e-5, 4e-5));
    assert_eq!(
        network.add_setup(7, 0.002, 0.003),
        Err(AdjustError::NoSuchMark { index: 7, count: 2 })
    );
    assert!(matches!(
        network.add_setup(a, 0.0, 0.003),
        Err(AdjustError::InvalidOption { .. })
    ));
    let on_a = network.add_setup(a, 0.002, 0.003).unwrap();
    let vector = Vector3::new(1000.0, 0.0, 0.0);
    // Set-up on A at B's end.
    network.add_observation(
        Observation::baseline("A-B", a, b, vector, covariance).on_setups(&[[None, Some(on_a)]]),
    );
    network.add_observation(Observation::point("A", a, base, covariance));
    let error = network
        .adjust(&CONSTRAINED, &Options::default())
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "observation A-B: it puts set-up 0, over mark A, at mark B"
    );
    network.observations_mut()[0].elements[0] = Element::Difference {
        from: a,
        to: b,
        vector,
        setups: [Some(5), None],
    };
    let error = network
        .adjust(&CONSTRAINED, &Options::default())
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "observation A-B: it refers to set-up 5, but the network has 1 set-ups"
    );
    // Right way round, it adjusts, and an unused set-up only has its prior.
    network.observations_mut()[0].elements[0] = Element::Difference {
        from: a,
        to: b,
        vector,
        setups: [Some(on_a), None],
    };
    network.add_setup(b, 0.002, 0.003).unwrap();
    let adjustment = network.adjust(&CONSTRAINED, &Options::default()).unwrap();
    assert_eq!(adjustment.setups()[1].error, Vector3::zeros());
}
