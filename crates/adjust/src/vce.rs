//! Variance component estimation by group (ARCHITECTURE.md section 10.7).
//!
//! Each group's scale factor is estimated with the simplified Helmert
//! (Förstner) estimator, k̂ = k Ω_g / (σ₀² r_g), from the group's share of
//! the weighted sum of squares, Ω_g = Σ vᵀPv, and of the redundancy,
//! r_g = tr((Qᵥᵥ P)_gg). The adjustment is repeated with the new factors
//! until they settle. Observations in no group, such as control, keep their
//! covariances as given.

use std::collections::BTreeSet;

use crate::adjustment::Adjustment;
use crate::error::AdjustError;
use crate::network::{Datum, Network, Options};

/// A group with less redundancy than this gives an estimate too uncertain to
/// rely on: its relative standard deviation, √(2/r), is over 45%.
const WEAK_REDUNDANCY: f64 = 10.0;

/// Below this, a group has no redundancy to estimate from, and its factor is
/// left as it is.
const NO_REDUNDANCY: f64 = 1e-6;

/// Settings for variance component estimation.
#[derive(Debug, Clone, PartialEq)]
pub struct VceOptions {
    /// The most adjustments to run.
    pub max_iterations: usize,
    /// The estimates have settled when every group's Ω_g / (σ₀² r_g) is
    /// within this of 1.
    pub tolerance: f64,
}

impl Default for VceOptions {
    /// 50 iterations, to within 1e-6.
    fn default() -> Self {
        VceOptions {
            max_iterations: 50,
            tolerance: 1e-6,
        }
    }
}

/// One group's variance component.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupComponent {
    /// The group's name.
    pub group: String,
    /// The number of observations in the group.
    pub observations: usize,
    /// The group's redundancy, r_g: its share of the degrees of freedom.
    pub redundancy: f64,
    /// The group's share of the weighted sum of squares, Ω_g.
    pub sum_of_squares: f64,
    /// The scale factor for the group's covariances.
    pub scale: f64,
    /// True if the redundancy is below 10, too little for a meaningful
    /// estimate.
    pub weak: bool,
}

/// The result of variance component estimation.
#[derive(Debug, Clone, PartialEq)]
pub struct VarianceComponents {
    /// Each group's component, by name.
    pub groups: Vec<GroupComponent>,
    /// The number of adjustments run.
    pub iterations: usize,
    /// True if the estimates settled within the iterations allowed.
    pub converged: bool,
    /// The scale factors each adjustment used, in the order of `groups`:
    /// the convergence record.
    pub history: Vec<Vec<f64>>,
    /// The options with the estimated scale factors.
    pub options: Options,
    /// The adjustment with the estimated scale factors.
    pub adjustment: Adjustment,
}

impl Network {
    /// Estimates a scale factor for each group of observations, starting
    /// from the factors in `options`, and adjusts with them.
    pub fn estimate_variance_components(
        &self,
        datum: &Datum,
        options: &Options,
        vce: &VceOptions,
    ) -> Result<VarianceComponents, AdjustError> {
        let names: Vec<String> = self
            .observations()
            .iter()
            .filter_map(|o| o.group.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let group_of: Vec<Option<usize>> = self
            .observations()
            .iter()
            .map(|o| o.group.as_ref().and_then(|g| names.binary_search(g).ok()))
            .collect();
        let mut options = options.clone();
        let mut history = Vec::new();
        let mut iterations = 0;
        loop {
            let scales: Vec<f64> = names.iter().map(|g| options.scale(Some(g))).collect();
            let adjustment = self.adjust(datum, &options)?;
            iterations += 1;
            history.push(scales.clone());

            let mut redundancy = vec![0.0; names.len()];
            let mut sum_of_squares = vec![0.0; names.len()];
            let mut observations = vec![0; names.len()];
            for (result, group) in adjustment.observations().iter().zip(&group_of) {
                let Some(g) = *group else { continue };
                observations[g] += 1;
                sum_of_squares[g] += result.sum_of_squares;
                // The redundancy numbers of an element's components sum to
                // the trace of its block of QᵥᵥP, on any axes.
                redundancy[g] += result
                    .elements
                    .iter()
                    .flat_map(|e| &e.components)
                    .map(|c| c.redundancy)
                    .sum::<f64>();
            }
            let ratios: Vec<f64> = (0..names.len())
                .map(|g| {
                    if redundancy[g] < NO_REDUNDANCY {
                        1.0
                    } else {
                        sum_of_squares[g] / (options.variance_factor * redundancy[g])
                    }
                })
                .collect();
            let converged = ratios.iter().all(|r| (r - 1.0).abs() < vce.tolerance);
            if converged || iterations >= vce.max_iterations {
                let groups = names
                    .iter()
                    .enumerate()
                    .map(|(g, name)| GroupComponent {
                        group: name.clone(),
                        observations: observations[g],
                        redundancy: redundancy[g],
                        sum_of_squares: sum_of_squares[g],
                        scale: scales[g],
                        weak: redundancy[g] < WEAK_REDUNDANCY,
                    })
                    .collect();
                return Ok(VarianceComponents {
                    groups,
                    iterations,
                    converged,
                    history,
                    options,
                    adjustment,
                });
            }
            for (name, (scale, ratio)) in names.iter().zip(scales.iter().zip(&ratios)) {
                options.group_scales.insert(name.clone(), scale * ratio);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::{Spec, Synthetic};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const CONSTRAINED: Datum = Datum::Constrained { fixed: Vec::new() };

    fn grouped() -> Synthetic {
        Synthetic::generate(&Spec {
            marks: 40,
            neighbours: 4,
            groups: 2,
            control: 4,
            seed: 31,
            ..Spec::default()
        })
        .unwrap()
    }

    #[test]
    fn recovers_each_groups_scale() {
        // Group 2's covariances are four times too optimistic.
        let synthetic = grouped();
        let truth = |g: &str| if g == "group-2" { 4.0 } else { 1.0 };
        let draws = 20;
        let mut estimates = vec![Vec::new(); 2];
        for seed in 0..draws {
            let network =
                synthetic.observe_with(700 + seed, |o| o.group.as_deref().map_or(1.0, truth));
            let vce = network
                .estimate_variance_components(
                    &CONSTRAINED,
                    &Options::default(),
                    &VceOptions::default(),
                )
                .unwrap();
            assert!(vce.converged, "{:?}", vce.history);
            assert_eq!(vce.history.len(), vce.iterations);
            for (g, component) in vce.groups.iter().enumerate() {
                assert!(!component.weak);
                estimates[g].push(component.scale / truth(&component.group));
            }
            // After estimation, each group fits: Ω_g = r_g, to the tolerance.
            for component in &vce.groups {
                let ratio = component.sum_of_squares / component.redundancy;
                assert!((ratio - 1.0).abs() < 1e-6, "{component:?}");
            }
        }
        // Each estimate's relative spread is about √(2/r_g); their mean is 1.
        for e in estimates {
            let n = e.len() as f64;
            let mean = e.iter().sum::<f64>() / n;
            let variance = e.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
            let standard_error = libm::sqrt(variance / n);
            assert!(
                (mean - 1.0).abs() < 4.0 * standard_error,
                "mean {mean}, standard error {standard_error}"
            );
        }
    }

    #[test]
    fn a_small_group_is_weak_and_no_groups_need_one_adjustment() {
        let mut network = grouped().observe(5);
        // Two vectors in a group of their own.
        for o in &mut network.observations_mut()[..2] {
            o.group = Some("small".into());
        }
        let vce = network
            .estimate_variance_components(&CONSTRAINED, &Options::default(), &VceOptions::default())
            .unwrap();
        let small = vce.groups.iter().find(|g| g.group == "small").unwrap();
        assert_eq!(small.observations, 2);
        assert!(small.weak);

        for o in network.observations_mut() {
            o.group = None;
        }
        let vce = network
            .estimate_variance_components(&CONSTRAINED, &Options::default(), &VceOptions::default())
            .unwrap();
        assert!(vce.groups.is_empty() && vce.converged);
        assert_eq!(vce.iterations, 1);
    }

    #[test]
    fn stops_at_the_iteration_limit() {
        let network = grouped().observe_with(9, |_| 3.0);
        let limit = VceOptions {
            max_iterations: 2,
            tolerance: 1e-12,
        };
        let vce = network
            .estimate_variance_components(&CONSTRAINED, &Options::default(), &limit)
            .unwrap();
        assert!(!vce.converged);
        assert_eq!(vce.iterations, 2);
        assert_eq!(vce.history[0], vec![1.0, 1.0]);
    }
}
