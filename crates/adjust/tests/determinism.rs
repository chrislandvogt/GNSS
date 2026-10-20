//! Bit-for-bit determinism across platforms (D-024).
//!
//! This hashes the exact bits of adjustments of synthetic networks, in each
//! datum, and of critical values over a range of levels and degrees of
//! freedom. That covers the generator, the block-sparse factorisation, the
//! selected inverse, the S-transformation, every statistic and the
//! distributions behind every test decision. CI runs it natively on Linux,
//! Windows and macOS and in headless Chrome, and the hash must be the same
//! everywhere.
//!
//! A deliberate change to the maths changes the hash. When that happens,
//! check that the other tests still pass, then update `GOLDEN`.

#![allow(clippy::unwrap_used)]

use gnss_adjust::stats::{self, BMethod};
use gnss_adjust::synthetic::{Spec, Synthetic};
use gnss_adjust::{Adjustment, Datum, Options, Scaling, Uncertainty, VceOptions};
use sha2::{Digest, Sha256};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const GOLDEN: &str = "433b8ed519fdcfd8c140911739031c3742a9cb9c1cccc333ad781d7e0b9afd09";

fn put(hasher: &mut Sha256, value: f64) {
    hasher.update(value.to_bits().to_le_bytes());
}

fn put_option(hasher: &mut Sha256, value: Option<f64>) {
    match value {
        Some(v) => {
            hasher.update([1]);
            put(hasher, v);
        }
        None => hasher.update([0]),
    }
}

fn put_uncertainty(hasher: &mut Sha256, u: &Uncertainty) {
    u.enu.iter().for_each(|&v| put(hasher, v));
    for v in [
        u.ellipse.semi_major,
        u.ellipse.semi_minor,
        u.ellipse.azimuth,
    ] {
        put(hasher, v);
    }
    u.ellipsoid.semi_axes.iter().for_each(|&v| put(hasher, v));
    u.ellipsoid.axes.iter().for_each(|&v| put(hasher, v));
    for v in [u.sigma_up, u.horizontal_95, u.vertical_95] {
        put(hasher, v);
    }
}

fn put_adjustment(hasher: &mut Sha256, adjustment: &Adjustment) {
    // PU for every mark, and RU from the first mark to every other: most of
    // those pairs share no observation, so they come from solves.
    let marks = adjustment.marks().len();
    for m in 0..marks {
        let pu = adjustment
            .positional_uncertainty(m, Scaling::APosteriori)
            .unwrap();
        put_uncertainty(hasher, &pu);
        let ru = adjustment
            .relative_uncertainty(0, m, Scaling::APriori)
            .unwrap();
        put_uncertainty(hasher, &ru);
    }
    for mark in adjustment.marks() {
        mark.position.iter().for_each(|&v| put(hasher, v));
        mark.cofactor.iter().for_each(|&v| put(hasher, v));
    }
    for observation in adjustment.observations() {
        put(hasher, observation.sum_of_squares);
        for element in &observation.elements {
            element.residual.iter().for_each(|&v| put(hasher, v));
            put_option(hasher, element.vector_test);
            element
                .standardised
                .iter()
                .for_each(|&s| put_option(hasher, s));
            for c in &element.components {
                put(hasher, c.residual);
                put(hasher, c.redundancy);
                put_option(hasher, c.w);
                put_option(hasher, c.tau);
                put_option(hasher, c.mdb);
                put_option(hasher, c.shift);
            }
        }
    }
    let statistics = adjustment.statistics();
    put(hasher, statistics.sum_of_squares);
    hasher.update((statistics.degrees_of_freedom as u64).to_le_bytes());
    put_option(hasher, statistics.variance_factor);
    let global = statistics.global_test.as_ref().unwrap();
    for v in [global.statistic, global.lower, global.upper] {
        put(hasher, v);
    }
    let criteria = adjustment.criteria();
    for v in [
        criteria.lambda0,
        criteria.w,
        criteria.vector,
        criteria.vector_alpha,
    ] {
        put(hasher, v);
    }
    put_option(hasher, criteria.tau);
}

#[test]
fn results_are_bit_identical_on_every_platform() {
    let mut hasher = Sha256::new();
    let options = Options::default();
    for (control, cluster, seed) in [(4, 3, 21), (0, 2, 22)] {
        let synthetic = Synthetic::generate(&Spec {
            marks: 40,
            neighbours: 4,
            cluster,
            control,
            seed,
            ..Spec::default()
        })
        .unwrap();
        let network = synthetic.observe(seed + 100);
        let datums = if control > 0 {
            vec![
                Datum::Constrained { fixed: Vec::new() },
                Datum::Constrained { fixed: vec![7] },
            ]
        } else {
            vec![Datum::FixedMark(13), Datum::Inner]
        };
        for datum in datums {
            put_adjustment(&mut hasher, &network.adjust(&datum, &options).unwrap());
        }
    }

    // Variance component estimation, iterated to convergence.
    let grouped = Synthetic::generate(&Spec {
        marks: 30,
        neighbours: 4,
        groups: 3,
        control: 3,
        seed: 23,
        ..Spec::default()
    })
    .unwrap()
    .observe_with(123, |o| {
        if o.group.as_deref() == Some("group-2") {
            2.5
        } else {
            1.0
        }
    });
    let vce = grouped
        .estimate_variance_components(
            &Datum::Constrained { fixed: Vec::new() },
            &options,
            &VceOptions::default(),
        )
        .unwrap();
    hasher.update((vce.iterations as u64).to_le_bytes());
    for scales in &vce.history {
        scales.iter().for_each(|&s| put(&mut hasher, s));
    }
    for g in &vce.groups {
        for v in [g.redundancy, g.sum_of_squares, g.scale] {
            put(&mut hasher, v);
        }
    }
    put_adjustment(&mut hasher, &vce.adjustment);

    for p in [
        1e-12, 1e-6, 0.001, 0.025, 0.05, 0.2, 0.5, 0.8, 0.975, 0.999_999,
    ] {
        put(&mut hasher, stats::normal_quantile(p));
        for k in [1.0, 2.0, 3.0, 7.0, 30.0, 250.0, 6000.0] {
            put(&mut hasher, stats::chi2_quantile(p, k));
            put(&mut hasher, stats::chi2_upper_quantile(p, k));
            if p <= 0.5 {
                put(&mut hasher, stats::student_t_upper_quantile(p, k));
            }
        }
    }
    for (alpha0, beta0) in [(0.001, 0.8), (0.01, 0.8), (0.05, 0.9), (0.0001, 0.5)] {
        let b = BMethod::new(alpha0, beta0);
        put(&mut hasher, b.lambda0());
        for k in [1.0, 3.0, 6.0, 20.0] {
            let (critical, alpha) = b.critical(k);
            put(&mut hasher, critical);
            put(&mut hasher, alpha);
        }
    }

    let hash: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        hash, GOLDEN,
        "the adjustment's bits changed: either the maths changed on purpose (check the other \
         tests, then update GOLDEN) or this platform computes different bits (D-024)"
    );
}
