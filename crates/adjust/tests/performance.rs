//! The M2 performance targets: a network of 1,000 marks and 3,000 vectors
//! adjusts in under 2 s natively and under 6 s in the browser
//! (ARCHITECTURE.md section 20).
//!
//! Timings only mean something in an optimised build, so this runs only in
//! release builds. CI runs it natively and in headless Chrome with
//! `cargo test --release -p gnss-adjust --test performance`.

#![allow(clippy::unwrap_used)]

use gnss_adjust::synthetic::{Spec, Synthetic};
use gnss_adjust::{Datum, Options};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

/// The target, in seconds.
const TARGET: f64 = if cfg!(target_arch = "wasm32") {
    6.0
} else {
    2.0
};

/// Seconds since some fixed point.
fn now() -> f64 {
    #[cfg(target_arch = "wasm32")]
    return js_sys::Date::now() / 1000.0;
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        use std::time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_secs_f64()
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing: run in release")]
fn a_thousand_marks_and_three_thousand_vectors_adjust_in_time() {
    let synthetic = Synthetic::generate(&Spec {
        marks: 1000,
        neighbours: 5,
        radius_m: 50_000.0,
        control: 10,
        seed: 3,
        ..Spec::default()
    })
    .unwrap();
    let network = synthetic.observe(1);
    let vectors = network.observations().len() - synthetic.control.len();
    assert!(vectors >= 3000 - 10, "{vectors} vectors");

    let start = now();
    let adjustment = network
        .adjust(
            &Datum::Constrained { fixed: Vec::new() },
            &Options::default(),
        )
        .unwrap();
    let elapsed = now() - start;
    let message = format!(
        "{} marks and {vectors} vectors adjusted in {elapsed:.3} s (σ̂₀² = {:.3})",
        network.marks().len(),
        adjustment.statistics().variance_factor.unwrap()
    );
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::console_log!("{message}");
    #[cfg(not(target_arch = "wasm32"))]
    println!("{message}");
    assert!(elapsed < TARGET, "{message}: over {TARGET} s");
}
