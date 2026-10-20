//! The adjustment against a dense textbook adjustment, and its statistics
//! against theory on synthetic networks with known truth.
//!
//! The statistical tests draw noise from each observation's own covariance,
//! so the distributions are exactly the theoretical ones. Seeds are fixed and
//! results are the same bits on every platform, so the bounds, about four
//! standard deviations of each estimate, are checked once and hold everywhere.

#![allow(clippy::unwrap_used)]

use gnss_adjust::synthetic::{Spec, Synthetic};
use gnss_adjust::{AdjustError, Adjustment, Datum, Element, Network, Observation, Options};
use gnss_datum::{CoordinateEpoch, FrameId, GRS80};
use nalgebra::{DMatrix, DVector, Matrix3, Vector3};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const CONSTRAINED: Datum = Datum::Constrained { fixed: Vec::new() };

/// A textbook adjustment with dense matrices: x = Qₓ AᵀPl over every mark,
/// with Qₓ the inverse of the normal matrix without the fixed marks' columns,
/// or for inner constraints its pseudo-inverse.
struct Dense {
    positions: Vec<Vector3<f64>>,
    cofactors: Vec<Matrix3<f64>>,
    /// By element: residual (ECEF), then by ENU component: w, MDB, redundancy, shift.
    residuals: Vec<Vector3<f64>>,
    w: Vec<[Option<f64>; 3]>,
    mdb: Vec<[Option<f64>; 3]>,
    redundancy: Vec<[f64; 3]>,
    shift: Vec<[Option<f64>; 3]>,
    vector_test: Vec<Option<f64>>,
    sum_of_squares: f64,
}

fn dense(network: &Network, datum: &Datum, lambda0: f64) -> Dense {
    let marks = network.marks().len();
    let x0: Vec<Vector3<f64>> = network
        .marks()
        .iter()
        .map(|m| m.approximate.unwrap())
        .collect();
    let elements: Vec<Element> = network
        .observations()
        .iter()
        .flat_map(|o| o.elements.iter().copied())
        .collect();
    let rows = 3 * elements.len();
    let mut a = DMatrix::zeros(rows, 3 * marks);
    let mut l = DVector::zeros(rows);
    let mut q = DMatrix::zeros(rows, rows);
    let mut owner = Vec::new();
    let mut row = 0;
    for (index, o) in network.observations().iter().enumerate() {
        let n = 3 * o.elements.len();
        q.view_mut((row, row), (n, n)).copy_from(&o.covariance);
        for element in &o.elements {
            match *element {
                Element::Difference { from, to, vector } => {
                    a.view_mut((row, 3 * from), (3, 3))
                        .copy_from(&-Matrix3::identity());
                    a.view_mut((row, 3 * to), (3, 3))
                        .copy_from(&Matrix3::identity());
                    l.rows_mut(row, 3)
                        .copy_from(&(vector - (x0[to] - x0[from])));
                }
                Element::Position { mark, position } => {
                    a.view_mut((row, 3 * mark), (3, 3))
                        .copy_from(&Matrix3::identity());
                    l.rows_mut(row, 3).copy_from(&(position - x0[mark]));
                }
            }
            owner.push(index);
            row += 3;
        }
    }
    let p = q.clone().try_inverse().unwrap();
    let normal = a.transpose() * &p * &a;
    let qx = match datum {
        Datum::Inner => {
            let g = DMatrix::from_fn(3 * marks, 3, |r, c| if r % 3 == c { 1.0 } else { 0.0 });
            let ggt = &g * g.transpose();
            (normal + &ggt).try_inverse().unwrap() - ggt / (marks * marks) as f64
        }
        Datum::FixedMark(f) => reduced_inverse(&normal, &[*f]),
        Datum::Constrained { fixed } => reduced_inverse(&normal, fixed),
    };
    let x = &qx * a.transpose() * &p * &l;
    let v = &a * &x - &l;
    let qvv = &q - &a * &qx * a.transpose();
    let m = &p * &qvv * &p;
    let redundancy = &qvv * &p;
    let pv = &p * &v;

    let mut result = Dense {
        positions: (0..marks)
            .map(|k| x0[k] + Vector3::from_fn(|i, _| x[3 * k + i]))
            .collect(),
        cofactors: (0..marks)
            .map(|k| qx.fixed_view::<3, 3>(3 * k, 3 * k).into_owned())
            .collect(),
        residuals: Vec::new(),
        w: Vec::new(),
        mdb: Vec::new(),
        redundancy: Vec::new(),
        shift: Vec::new(),
        vector_test: Vec::new(),
        sum_of_squares: v.dot(&pv),
    };
    for (e, element) in elements.iter().enumerate() {
        let at = GRS80.ecef_to_geodetic(&x0[element.first_mark()]).unwrap();
        let axes = at.enu_rotation();
        let own: Vec<usize> = network.observations()[owner[e]]
            .elements
            .iter()
            .flat_map(|el| match *el {
                Element::Difference { from, to, .. } => vec![from, to],
                Element::Position { mark, .. } => vec![mark],
            })
            .collect();
        let mut w = [None; 3];
        let mut mdb = [None; 3];
        let mut red = [0.0; 3];
        let mut shift = [None; 3];
        for k in 0..3 {
            let mut c = DVector::zeros(rows);
            c.rows_mut(3 * e, 3).copy_from(&axes.row(k).transpose());
            let rp: Matrix3<f64> = redundancy.fixed_view::<3, 3>(3 * e, 3 * e).into_owned();
            red[k] = (axes * rp * axes.transpose())[(k, k)];
            let cmc = c.dot(&(&m * &c));
            // Not checked by the other observations: a bridge or a spur.
            if cmc <= 1e-8 * c.dot(&(&p * &c)) {
                continue;
            }
            w[k] = Some(c.dot(&pv) / libm::sqrt(cmc));
            let bias = libm::sqrt(lambda0 / cmc);
            mdb[k] = Some(bias);
            let moved = &qx * a.transpose() * &p * &c * bias;
            shift[k] = Some(
                own.iter()
                    .map(|&o| Vector3::from_fn(|i, _| moved[3 * o + i]).norm())
                    .fold(0.0, f64::max),
            );
        }
        let m_ee: Matrix3<f64> = m.fixed_view::<3, 3>(3 * e, 3 * e).into_owned();
        let pv_e: Vector3<f64> = pv.fixed_rows::<3>(3 * e).into_owned();
        result.residuals.push(v.fixed_rows::<3>(3 * e).into_owned());
        result.w.push(w);
        result.mdb.push(mdb);
        result.redundancy.push(red);
        result.shift.push(shift);
        result.vector_test.push(
            w.iter()
                .all(Option::is_some)
                .then(|| pv_e.dot(&(m_ee.try_inverse().unwrap() * pv_e))),
        );
    }
    result
}

/// The inverse of the normal matrix without the columns of `fixed`, with
/// zeros in their place.
fn reduced_inverse(normal: &DMatrix<f64>, fixed: &[usize]) -> DMatrix<f64> {
    let keep: Vec<usize> = (0..normal.nrows())
        .filter(|i| !fixed.contains(&(i / 3)))
        .collect();
    let reduced = DMatrix::from_fn(keep.len(), keep.len(), |r, c| normal[(keep[r], keep[c])]);
    let inverse = reduced.try_inverse().unwrap();
    let mut full = DMatrix::zeros(normal.nrows(), normal.ncols());
    for (r, &i) in keep.iter().enumerate() {
        for (c, &j) in keep.iter().enumerate() {
            full[(i, j)] = inverse[(r, c)];
        }
    }
    full
}

#[track_caller]
fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance * (1.0 + b.abs()), "{a} vs {b}");
}

/// Both missing, or both there and close.
#[track_caller]
fn close_option(a: Option<f64>, b: Option<f64>, tolerance: f64) {
    match (a, b) {
        (Some(a), Some(b)) => close(a, b, tolerance),
        (None, None) => {}
        _ => panic!("{a:?} vs {b:?}"),
    }
}

fn elements(adjustment: &Adjustment) -> impl Iterator<Item = &gnss_adjust::ElementResult> {
    adjustment.observations().iter().flat_map(|o| &o.elements)
}

fn compare(adjustment: &Adjustment, dense: &Dense) {
    for (mark, (x, q)) in adjustment
        .marks()
        .iter()
        .zip(dense.positions.iter().zip(&dense.cofactors))
    {
        assert!(
            (mark.position - x).norm() < 1e-8,
            "{} vs {x}",
            mark.position
        );
        assert!(
            (mark.cofactor - q).abs().max() < 1e-14,
            "{} vs {q}",
            mark.cofactor
        );
    }
    close(
        adjustment.statistics().sum_of_squares,
        dense.sum_of_squares,
        1e-8,
    );
    for (e, element) in elements(adjustment).enumerate() {
        assert!((element.residual - dense.residuals[e]).norm() < 1e-9);
        for (k, c) in element.components.iter().enumerate() {
            close_option(c.w, dense.w[e][k], 1e-7);
            close_option(c.mdb, dense.mdb[e][k], 1e-7);
            close(c.redundancy, dense.redundancy[e][k], 1e-7);
            close_option(c.shift, dense.shift[e][k], 1e-7);
        }
        close_option(element.vector_test, dense.vector_test[e], 1e-7);
    }
}

fn clustered(control: usize) -> Synthetic {
    Synthetic::generate(&Spec {
        marks: 14,
        neighbours: 3,
        cluster: 3,
        control,
        seed: 7,
        ..Spec::default()
    })
    .unwrap()
}

#[test]
fn matches_a_dense_adjustment_in_every_datum() {
    let options = Options::default();
    let with_control = clustered(3).observe(1);
    let without = clustered(0).observe(1);
    for (network, datum) in [
        (&with_control, CONSTRAINED),
        (&with_control, Datum::Constrained { fixed: vec![4] }),
        (&without, Datum::FixedMark(5)),
        (&without, Datum::Inner),
    ] {
        let adjustment = network.adjust(&datum, &options).unwrap();
        compare(
            &adjustment,
            &dense(network, &datum, adjustment.criteria().lambda0),
        );
    }
}

#[test]
fn recovers_the_truth_from_exact_observations() {
    let synthetic = clustered(3);
    let adjustment = synthetic
        .exact()
        .adjust(&CONSTRAINED, &Options::default())
        .unwrap();
    for (mark, truth) in adjustment.marks().iter().zip(&synthetic.truth) {
        assert!((mark.position - truth).norm() < 1e-8);
    }
    assert!(adjustment.statistics().sum_of_squares < 1e-12);
}

#[test]
fn statistics_do_not_depend_on_the_minimal_datum() {
    let network = clustered(0).observe(3);
    let options = Options::default();
    let first = network.adjust(&Datum::FixedMark(0), &options).unwrap();
    let centroid = |positions: &mut dyn Iterator<Item = Vector3<f64>>| {
        positions.sum::<Vector3<f64>>() / network.marks().len() as f64
    };
    for datum in [Datum::FixedMark(9), Datum::Inner] {
        let other = network.adjust(&datum, &options).unwrap();
        close(
            other.statistics().sum_of_squares,
            first.statistics().sum_of_squares,
            1e-9,
        );
        assert_eq!(
            other.statistics().degrees_of_freedom,
            first.statistics().degrees_of_freedom
        );
        for (a, b) in elements(&first).zip(elements(&other)) {
            assert!((a.residual - b.residual).norm() < 1e-10);
            close_option(a.vector_test, b.vector_test, 1e-8);
            for (ca, cb) in a.components.iter().zip(&b.components) {
                close_option(ca.w, cb.w, 1e-8);
                close_option(ca.mdb, cb.mdb, 1e-8);
                close(ca.redundancy, cb.redundancy, 1e-8);
            }
        }
        // The positions differ by a translation only, to rounding: a
        // nanometre is a few units in the last place of an ECEF coordinate.
        let shift = other.marks()[0].position - first.marks()[0].position;
        for (a, b) in first.marks().iter().zip(other.marks()) {
            let difference = (b.position - a.position - shift).norm();
            assert!(difference < 1e-8, "{difference}");
        }
        if datum == Datum::Inner {
            let adjusted = centroid(&mut other.marks().iter().map(|m| m.position));
            let approximate = centroid(&mut network.marks().iter().map(|m| m.approximate.unwrap()));
            assert!((adjusted - approximate).norm() < 1e-9);
            assert!(other.marks().iter().all(|m| !m.fixed));
        } else {
            assert!(other.marks()[9].fixed);
            assert_eq!(other.marks()[9].cofactor, Matrix3::zeros());
        }
    }
}

#[test]
fn errors_are_consistent_with_the_cofactors() {
    let synthetic = clustered(3);
    // Each mark's error against the truth, d, makes dᵀQ⁻¹d a χ²(3) with mean
    // 3. Marks in one draw share the control's errors, so the draws, not the
    // marks, are the independent samples.
    let draws: Vec<f64> = (0..400)
        .map(|seed| {
            let adjustment = synthetic
                .observe(1000 + seed)
                .adjust(&CONSTRAINED, &Options::default())
                .unwrap();
            let total: f64 = adjustment
                .marks()
                .iter()
                .zip(&synthetic.truth)
                .map(|(mark, truth)| {
                    let error = mark.position - truth;
                    error.dot(&(mark.cofactor.try_inverse().unwrap() * error))
                })
                .sum();
            total / synthetic.truth.len() as f64
        })
        .collect();
    let n = draws.len() as f64;
    let mean = draws.iter().sum::<f64>() / n;
    let variance = draws.iter().map(|d| (d - mean) * (d - mean)).sum::<f64>() / (n - 1.0);
    let standard_error = libm::sqrt(variance / n);
    assert!(
        (mean - 3.0).abs() < 4.0 * standard_error,
        "mean {mean}, standard error {standard_error}"
    );
}

#[test]
fn test_statistics_follow_their_distributions() {
    let synthetic = Synthetic::generate(&Spec {
        marks: 30,
        neighbours: 4,
        cluster: 2,
        seed: 11,
        ..Spec::default()
    })
    .unwrap();
    let options = Options::default();
    let trials = 200;
    let (mut variance_factors, mut passed) = (0.0, 0);
    let (mut w_sum, mut w_squares, mut w_count, mut w_alarms) = (0.0, 0.0, 0.0, 0);
    let (mut vector_count, mut vector_alarms) = (0, 0);
    let mut degrees_of_freedom = 0;
    for seed in 0..trials {
        let adjustment = synthetic
            .observe(seed)
            .adjust(&CONSTRAINED, &options)
            .unwrap();
        let statistics = adjustment.statistics();
        degrees_of_freedom = statistics.degrees_of_freedom;
        variance_factors += statistics.variance_factor.unwrap();
        passed += usize::from(statistics.global_test.as_ref().unwrap().passed);
        let criteria = adjustment.criteria();
        for element in elements(&adjustment) {
            for w in element.components.iter().filter_map(|c| c.w) {
                w_sum += w;
                w_squares += w * w;
                w_count += 1.0;
                w_alarms += usize::from(w.abs() > criteria.w);
            }
            if let Some(t) = element.vector_test {
                vector_count += 1;
                vector_alarms += usize::from(t > criteria.vector);
            }
        }
    }
    let trials = trials as f64;
    // σ̂₀² has mean 1 and standard deviation √(2/r) per trial.
    let mean = variance_factors / trials;
    let spread = libm::sqrt(2.0 / degrees_of_freedom as f64 / trials);
    assert!((mean - 1.0).abs() < 4.0 * spread, "mean σ̂₀² {mean}");
    // The global test passes 95% of the time.
    let rate = passed as f64 / trials;
    assert!(
        (rate - 0.95).abs() < 4.0 * libm::sqrt(0.95 * 0.05 / trials),
        "pass rate {rate}"
    );
    // w is N(0, 1).
    let w_mean = w_sum / w_count;
    let w_variance = w_squares / w_count - w_mean * w_mean;
    assert!(w_mean.abs() < 0.03, "mean w {w_mean}");
    assert!(
        (w_variance - 1.0).abs() < 0.05,
        "variance of w {w_variance}"
    );
    // False alarms at the tests' levels, α₀ and the vector test's.
    for (alarms, count, alpha) in [
        (w_alarms, w_count, Options::default().alpha0),
        (
            vector_alarms,
            vector_count as f64,
            0.005_500_158_038_997_691,
        ),
    ] {
        let expected = count * alpha;
        assert!(
            (alarms as f64 - expected).abs() < 5.0 * libm::sqrt(expected),
            "{alarms} false alarms in {count} tests, expected {expected}"
        );
    }
}

#[test]
fn detects_a_bias_the_size_of_its_mdb_with_power_beta0() {
    let synthetic = clustered(3);
    let options = Options::default();
    let exact = synthetic.exact().adjust(&CONSTRAINED, &options).unwrap();
    // The up component of a well-checked vector in a cluster.
    let (observation, element) = (2, 1);
    let up = &exact.observations()[observation].elements[element].components[2];
    let mdb = up.mdb.unwrap();
    assert!(up.redundancy > 0.3);
    let Element::Difference { from, .. } =
        synthetic.exact().observations()[observation].elements[element]
    else {
        panic!("not a vector");
    };
    let at = GRS80
        .ecef_to_geodetic(&synthetic.exact().marks()[from].approximate.unwrap())
        .unwrap();
    let bias = at.enu_rotation().row(2).transpose() * mdb;

    let trials = 400;
    let (mut w_detected, mut vector_detected, mut w_sum) = (0, 0, 0.0);
    for seed in 0..trials {
        let mut network = synthetic.observe(5000 + seed);
        if let Element::Difference { vector, .. } =
            &mut network.observations_mut()[observation].elements[element]
        {
            *vector += bias;
        }
        let adjustment = network.adjust(&CONSTRAINED, &options).unwrap();
        let result = &adjustment.observations()[observation].elements[element];
        let w = result.components[2].w.unwrap();
        w_sum += w;
        w_detected += usize::from(w.abs() > adjustment.criteria().w);
        vector_detected += usize::from(result.vector_test.unwrap() > adjustment.criteria().vector);
    }
    let trials = trials as f64;
    // w is N(−√λ₀, 1), negative because residuals are adjusted minus
    // observed, and T is non-central χ²(3) with non-centrality λ₀: both
    // detect the bias with probability β₀.
    let w_mean = w_sum / trials;
    assert!(
        (w_mean + libm::sqrt(exact.criteria().lambda0)).abs() < 0.2,
        "mean w {w_mean}"
    );
    let spread = 4.0 * libm::sqrt(0.8 * 0.2 / trials);
    for detected in [w_detected, vector_detected] {
        let power = detected as f64 / trials;
        assert!((power - 0.8).abs() < spread, "power {power}");
    }
}

#[test]
fn a_spur_vector_is_not_tested() {
    let mut network = clustered(3).observe(2);
    let x = network.marks()[0].approximate.unwrap() + Vector3::new(500.0, -300.0, 200.0);
    let spur = network.add_mark("SPUR", Some(x)).unwrap();
    let covariance = Matrix3::from_diagonal(&Vector3::new(1e-5, 1e-5, 4e-5));
    let vector = x - network.marks()[0].approximate.unwrap();
    network.add_observation(Observation::baseline("spur", 0, spur, vector, covariance));
    let adjustment = network.adjust(&CONSTRAINED, &Options::default()).unwrap();
    let result = &adjustment.observations().last().unwrap().elements[0];
    assert!(result.vector_test.is_none());
    for c in &result.components {
        assert!(c.w.is_none() && c.mdb.is_none() && c.shift.is_none());
        assert!(c.redundancy.abs() < 1e-8);
    }
}

#[test]
fn diagnostics_name_the_marks() {
    let options = Options::default();
    let covariance = Matrix3::from_diagonal(&Vector3::new(1e-5, 1e-5, 4e-5));
    let base = Vector3::new(-4_474_000.0, 2_680_000.0, -3_667_000.0);
    let mut network = Network::new(FrameId::Gda2020, None).unwrap();
    let names = ["A", "B", "C", "D"];
    for (i, name) in names.iter().enumerate() {
        let offset = Vector3::new(1000.0 * i as f64, 0.0, 0.0);
        network.add_mark(*name, Some(base + offset)).unwrap();
    }
    let vector = Vector3::new(1000.0, 0.0, 0.0);
    network.add_observation(Observation::baseline("A-B", 0, 1, vector, covariance));
    network.add_observation(Observation::baseline("C-D", 2, 3, vector, covariance));
    assert_eq!(
        network.adjust(&Datum::FixedMark(0), &options),
        Err(AdjustError::Disconnected {
            parts: vec![vec!["A".into(), "B".into()], vec!["C".into(), "D".into()]]
        })
    );
    let datum = Datum::Constrained { fixed: vec![1] };
    let error = network.adjust(&datum, &options).unwrap_err();
    assert_eq!(
        error,
        AdjustError::UndefinedDatum {
            marks: vec!["C".into(), "D".into()]
        }
    );
    assert!(error.to_string().contains("C, D"), "{error}");
    network.add_observation(Observation::baseline("B-C", 1, 2, vector, covariance));
    assert!(network.adjust(&datum, &options).is_ok());
    assert!(network.adjust(&Datum::Inner, &options).is_ok());

    let mut unobserved = network.clone();
    unobserved.add_mark("E", None).unwrap();
    assert_eq!(
        unobserved.adjust(&datum, &options),
        Err(AdjustError::Unobserved { name: "E".into() })
    );
    assert_eq!(
        network.add_mark("A", None),
        Err(AdjustError::DuplicateMark { name: "A".into() })
    );

    let mut controlled = network.clone();
    controlled.add_observation(Observation::point("control", 3, base, covariance));
    assert!(matches!(
        controlled.adjust(&Datum::FixedMark(0), &options),
        Err(AdjustError::InvalidDatum { .. })
    ));

    let mut unplaced = Network::new(FrameId::Gda2020, None).unwrap();
    let p = unplaced.add_mark("P", Some(base)).unwrap();
    let q = unplaced.add_mark("Q", None).unwrap();
    unplaced.add_observation(Observation::baseline("P-Q", p, q, vector, covariance));
    assert_eq!(
        unplaced.adjust(&Datum::FixedMark(q), &options),
        Err(AdjustError::NoApproximatePosition {
            name: "Q".into(),
            reason: "it's held fixed there"
        })
    );
    // Q is placed along the vector from P.
    let adjusted = unplaced.adjust(&Datum::FixedMark(p), &options).unwrap();
    assert!((adjusted.marks()[q].position - (base + vector)).norm() < 1e-9);

    let invalid = |o: Observation| {
        let mut n = network.clone();
        n.add_observation(o);
        n.adjust(&datum, &options).unwrap_err()
    };
    for (observation, expected) in [
        (
            Observation::baseline("loop", 2, 2, vector, covariance),
            "observation loop: it has a vector from mark C to itself",
        ),
        (
            Observation::baseline("far", 2, 9, vector, covariance),
            "observation far: it refers to mark 9, but the network has 4 marks",
        ),
        (
            Observation::baseline("flat", 0, 3, vector, Matrix3::zeros()),
            "observation flat: its covariance isn't positive definite",
        ),
        (
            Observation::baseline_cluster(
                "short",
                &[(0, 3, vector)],
                DMatrix::identity(6, 6) * 1e-5,
            ),
            "observation short: its covariance is 6 × 6, but should be 3 × 3: three rows and \
             columns for each element",
        ),
        (
            Observation::baseline(
                "skew",
                0,
                3,
                vector,
                Matrix3::new(1e-5, 1e-6, 0.0, 0.0, 1e-5, 0.0, 0.0, 0.0, 1e-5),
            ),
            "observation skew: its covariance isn't symmetric: (1, 0) is 0 but (0, 1) is 0.000001",
        ),
        (
            Observation::baseline_cluster(
                "twin",
                &[(0, 3, vector), (0, 3, vector)],
                DMatrix::from_fn(6, 6, |r, c| match (r == c, r % 3 == c % 3) {
                    (true, _) => 1e-5,
                    (false, true) => 1e-5 * (1.0 - 1e-12),
                    _ => 0.0,
                }),
            ),
            "observation twin: its covariance is singular or nearly so: the X component of \
             element 2 is fixed by the others",
        ),
    ] {
        assert_eq!(invalid(observation).to_string(), expected);
    }

    let bad = Options {
        alpha0: 0.0,
        ..Options::default()
    };
    assert!(matches!(
        network.adjust(&datum, &bad),
        Err(AdjustError::InvalidOption { name: "alpha0", .. })
    ));
    assert_eq!(
        Network::new(FrameId::Gda2020, None)
            .unwrap()
            .adjust(&Datum::Inner, &options),
        Err(AdjustError::Empty)
    );
    assert!(Network::new(FrameId::Itrf2014, None).is_err());
    let epoch = CoordinateEpoch::from_decimal_year(2021.0).unwrap();
    assert!(Network::new(FrameId::Gda2020, Some(epoch)).is_err());
    assert!(Network::new(FrameId::Itrf2014, Some(epoch)).is_ok());
}
