//! The outlier workflow (ARCHITECTURE.md section 10.5): suspects ranked by
//! their tests, and exclusions and re-weightings that only a person makes,
//! recorded as edits.

#![allow(clippy::unwrap_used)]

use gnss_adjust::synthetic::{Spec, Synthetic};
use gnss_adjust::{Action, AdjustError, Datum, Edit, Element, Network, Observation, Options};
use gnss_datum::GRS80;
use nalgebra::{Matrix3, Vector3};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const CONSTRAINED: Datum = Datum::Constrained { fixed: Vec::new() };

fn synthetic() -> Synthetic {
    Synthetic::generate(&Spec {
        marks: 30,
        neighbours: 4,
        control: 4,
        seed: 51,
        ..Spec::default()
    })
    .unwrap()
}

/// Adds `up` metres on the local up axis to an observation's first element.
fn blunder(network: &mut Network, observation: usize, up: f64) {
    let first = network.observations()[observation].elements[0].first_mark();
    let at = GRS80
        .ecef_to_geodetic(&network.marks()[first].approximate.unwrap())
        .unwrap();
    let shift = at.enu_rotation().row(2).transpose() * up;
    if let Element::Difference { vector, .. } =
        &mut network.observations_mut()[observation].elements[0]
    {
        *vector += shift;
    }
}

fn exclude(network: &Network, observation: usize) -> Edit {
    Edit {
        observation: network.observations()[observation].id.clone(),
        action: Action::Exclude,
        reason: "fails its tests; the field notes show a misread height".into(),
    }
}

#[test]
fn a_blunder_tops_the_suspects_and_excluding_it_is_recorded() {
    let mut network = synthetic().observe(510);
    let target = 20;
    blunder(&mut network, target, 0.1);

    // Nothing happens without an edit: every observation is used.
    let first = network.adjust(&CONSTRAINED, &Options::default()).unwrap();
    assert!(first.observations().iter().all(|o| o.used));
    assert!(first.edits().is_empty());
    assert!(!first.statistics().global_test.as_ref().unwrap().passed);
    let suspects = first.suspects();
    assert_eq!(suspects[0].observation, target, "{suspects:?}");
    assert!(suspects.windows(2).all(|w| w[0].excess >= w[1].excess));

    let edit = exclude(&network, target);
    let options = Options {
        edits: vec![edit.clone()],
        ..Options::default()
    };
    let second = network.adjust(&CONSTRAINED, &options).unwrap();
    assert_eq!(second.edits(), [edit]);
    let excluded = &second.observations()[target];
    assert!(!excluded.used);
    assert_eq!(excluded.sum_of_squares, 0.0);
    // Its misclosure against the rest, adjusted minus observed, shows the
    // 10 cm, and fails its test.
    let element = &excluded.elements[0];
    assert!(
        (element.components[2].residual + 0.1).abs() < 0.03,
        "{element:?}"
    );
    assert!(element.vector_test.unwrap() > second.criteria().vector);
    assert_eq!(
        second.statistics().degrees_of_freedom,
        first.statistics().degrees_of_freedom - 3
    );
    assert!(second.statistics().global_test.as_ref().unwrap().passed);
    assert!(second.suspects().iter().all(|s| s.observation != target));
}

#[test]
fn an_excluded_good_observation_fits() {
    // A good observation's misclosure test is χ²(3), and its standardised
    // misclosures are N(0, 1).
    let synthetic = synthetic();
    let target = 11;
    let draws = 200;
    let (mut tests, mut squares) = (Vec::new(), 0.0);
    for seed in 0..draws {
        let network = synthetic.observe(900 + seed);
        let options = Options {
            edits: vec![exclude(&network, target)],
            ..Options::default()
        };
        let adjustment = network.adjust(&CONSTRAINED, &options).unwrap();
        let element = &adjustment.observations()[target].elements[0];
        tests.push(element.vector_test.unwrap());
        squares += element
            .standardised
            .iter()
            .map(|s| s.unwrap() * s.unwrap())
            .sum::<f64>();
    }
    let n = tests.len() as f64;
    let mean = tests.iter().sum::<f64>() / n;
    // χ²(3) has variance 6.
    assert!(
        (mean - 3.0).abs() < 4.0 * libm::sqrt(6.0 / n),
        "mean {mean}"
    );
    let mean_square = squares / (3.0 * n);
    assert!(
        (mean_square - 1.0).abs() < 4.0 * libm::sqrt(2.0 / (3.0 * n)),
        "{mean_square}"
    );
}

#[test]
fn an_excluded_vectors_test_is_its_vector_test_in_the_full_adjustment() {
    // Leaving one baseline out and testing its misclosure against the rest
    // is the same test as its vector test with it in: data snooping is
    // leave-one-out prediction. They agree to the rounding of ECEF
    // coordinates, about a nanometre on a millimetre misclosure.
    let network = synthetic().observe(514);
    let full = network.adjust(&CONSTRAINED, &Options::default()).unwrap();
    for target in [0, 9, 17, 33] {
        let options = Options {
            edits: vec![exclude(&network, target)],
            ..Options::default()
        };
        let left_out = network.adjust(&CONSTRAINED, &options).unwrap();
        let used = full.observations()[target].elements[0].vector_test.unwrap();
        let excluded = left_out.observations()[target].elements[0]
            .vector_test
            .unwrap();
        assert!(
            (used - excluded).abs() < 1e-6 * used.max(1.0),
            "{used} vs {excluded}"
        );
    }
}

#[test]
fn reweighting_scales_the_covariance() {
    let network = synthetic().observe(511);
    let target = 7;
    let options = Options {
        edits: vec![
            Edit {
                observation: network.observations()[target].id.clone(),
                action: Action::Reweight(2.0),
                reason: "multipath at the base".into(),
            },
            Edit {
                observation: network.observations()[target].id.clone(),
                action: Action::Reweight(2.0),
                reason: "and a short session".into(),
            },
        ],
        ..Options::default()
    };
    let reweighted = network.adjust(&CONSTRAINED, &options).unwrap();
    assert_eq!(reweighted.observations()[target].scale, 4.0);

    // The same as scaling the covariance by hand.
    let mut by_hand = network.clone();
    by_hand.observations_mut()[target].covariance *= 4.0;
    let expected = by_hand.adjust(&CONSTRAINED, &Options::default()).unwrap();
    for (a, b) in reweighted.marks().iter().zip(expected.marks()) {
        assert_eq!(a.position, b.position);
    }
    assert_eq!(
        reweighted.statistics().sum_of_squares,
        expected.statistics().sum_of_squares
    );
}

#[test]
fn edits_must_name_one_observation() {
    let mut network = synthetic().observe(512);
    let edit = |id: &str, action| Options {
        edits: vec![Edit {
            observation: id.into(),
            action,
            reason: String::new(),
        }],
        ..Options::default()
    };
    assert_eq!(
        network.adjust(&CONSTRAINED, &edit("nothing", Action::Exclude)),
        Err(AdjustError::NoSuchObservation {
            id: "nothing".into()
        })
    );
    let id = network.observations()[3].id.clone();
    assert!(matches!(
        network.adjust(&CONSTRAINED, &edit(&id, Action::Reweight(0.0))),
        Err(AdjustError::InvalidOption { .. })
    ));
    network.observations_mut()[4].id = id.clone();
    assert_eq!(
        network.adjust(&CONSTRAINED, &edit(&id, Action::Exclude)),
        Err(AdjustError::AmbiguousObservation { id, count: 2 })
    );

    // Excluding a spur's only vector leaves its mark with nothing.
    let mut network = synthetic().observe(513);
    let x = network.marks()[0].approximate.unwrap() + Vector3::new(500.0, -300.0, 200.0);
    let spur = network.add_mark("SPUR", Some(x)).unwrap();
    let vector = x - network.marks()[0].approximate.unwrap();
    let covariance = Matrix3::from_diagonal(&Vector3::new(1e-5, 1e-5, 4e-5));
    network.add_observation(Observation::baseline("spur", 0, spur, vector, covariance));
    assert_eq!(
        network.adjust(&CONSTRAINED, &edit("spur", Action::Exclude)),
        Err(AdjustError::OnlyExcluded {
            name: "SPUR".into()
        })
    );
}
