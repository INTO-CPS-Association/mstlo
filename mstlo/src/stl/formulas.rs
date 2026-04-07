//! Benchmark formula catalog.
//!
//! This module defines a stable set of STL formulas used by performance
//! benchmarks. The IDs are intentionally fixed so benchmark runs can be compared
//! across commits, machines, and algorithm/semantics variants.
//!
//! ## Benchmarking strategy
//! The set mixes:
//! - simple boolean/atomic cases,
//! - temporal operators with increasing window sizes, and
//! - deeply nested temporal/logical constructions.
//!
//! This gives coverage from low-overhead paths to stress cases that amplify
//! temporal lookahead and operator composition costs.
//!
//! ## Reuse for custom benchmarks
//! - Use [`get_formulas`] with an explicit ID subset for targeted experiments.
//! - Keep IDs unchanged in your reports for reproducibility.
//! - Compare the same IDs when testing new monitor configurations.

use crate::stl;
use crate::stl::core::TimeInterval;
use crate::stl::formula_definition::FormulaDefinition;
use std::time::Duration;

/// Returns the vector of Signal Temporal Logic formulas.
/// If `ids` is not empty, returns only the formulas with the specified IDs.
///
/// # Formula groups
/// - `1..=12`: basic and window-scaled formulas (`And`, `Or`, `Not`, `G`, `F`, `U`).
/// - `13..=21`: nested/stress formulas with increasing structural depth.
///
/// # Arguments
/// * `ids` - Empty slice returns all formulas; otherwise only listed IDs are returned.
///
/// # Returns
/// Ordered `(id, formula)` pairs for deterministic benchmark setup.
pub fn get_formulas(ids: &[usize]) -> Vec<(usize, FormulaDefinition)> {
    let mut formulas: Vec<(usize, FormulaDefinition)> = vec![
        // --- Basic Formulas (Lines 1-12) ---
        (1, stl!((x < 0.5) and (x > -0.5))),
        (2, stl!((x < 0.5) or (x > -0.5))),
        (3, stl!(not(x < 0.5))),
        // 4-6. Globally (Always)
        (4, stl!(G[0, 10] (x < 0.5))),
        (5, stl!(G[0, 100] (x < 0.5))),
        (6, stl!(G[0, 1000] (x < 0.5))),
        // 7-9. Eventually
        (7, stl!(F[0, 10] (x < 0.5))),
        (8, stl!(F[0, 100] (x < 0.5))),
        (9, stl!(F[0, 1000] (x < 0.5))),
        // 10-12. Until
        (10, stl!((x < 0.5) until[0, 10] (x > -0.5))),
        (11, stl!((x < 0.5) until[0, 100] (x > -0.5))),
        (12, stl!((x < 0.5) until[0, 1000] (x > -0.5))),
    ];

    // --- Complex Nested Formulas (Lines 13-21) ---
    let zero_ten = TimeInterval {
        start: Duration::from_secs(0),
        end: Duration::from_secs(10),
    };

    // Pattern A
    let make_and_ev_chain = |depth: usize| -> FormulaDefinition {
        let mut curr = stl!(x > 0.0); // Base case
        for _ in 0..depth {
            curr = FormulaDefinition::And(
                Box::new(FormulaDefinition::Eventually(
                    zero_ten,
                    Box::new(curr.clone()),
                )),
                Box::new(FormulaDefinition::Eventually(
                    zero_ten,
                    Box::new(curr.clone()),
                )),
            );
        }
        curr
    };

    let next_id_start = 13usize;
    formulas.push((next_id_start, make_and_ev_chain(2)));
    formulas.push((next_id_start + 1, make_and_ev_chain(3)));
    formulas.push((next_id_start + 2, make_and_ev_chain(5)));

    // Pattern B
    let make_ev_alw_chain = |depth: usize| -> FormulaDefinition {
        let mut curr = stl!(x > 0.0); // Base case
        for _ in 0..depth {
            let alw_layer = FormulaDefinition::Globally(zero_ten, Box::new(curr));
            curr = FormulaDefinition::Eventually(zero_ten, Box::new(alw_layer));
        }
        curr
    };

    formulas.push((next_id_start + 3, make_ev_alw_chain(2)));
    formulas.push((next_id_start + 4, make_ev_alw_chain(3)));
    formulas.push((next_id_start + 5, make_ev_alw_chain(5)));

    // Pattern C
    let make_until_chain = |depth: usize| -> FormulaDefinition {
        let mut curr = stl!(x > 0.0); // Base case
        for _ in 0..depth {
            curr = FormulaDefinition::Until(zero_ten, Box::new(curr.clone()), Box::new(curr));
        }
        curr
    };

    formulas.push((next_id_start + 6, make_until_chain(2)));
    formulas.push((next_id_start + 7, make_until_chain(3)));
    formulas.push((next_id_start + 8, make_until_chain(5)));

    if ids.is_empty() {
        formulas
    } else {
        formulas
            .into_iter()
            .filter(|(id, _)| ids.contains(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_formulas() {
        let all_formulas = get_formulas(&[]);
        assert_eq!(all_formulas.len(), 21);
    }

    #[test]
    fn test_get_formulas_with_ids() {
        let selected_formulas = get_formulas(&[1, 4, 10, 13, 16, 19]);
        assert_eq!(selected_formulas.len(), 6);
        assert_eq!(selected_formulas[0].0, 1);
        assert_eq!(selected_formulas[1].0, 4);
        assert_eq!(selected_formulas[2].0, 10);
        assert_eq!(selected_formulas[3].0, 13);
        assert_eq!(selected_formulas[4].0, 16);
        assert_eq!(selected_formulas[5].0, 19);
    }
}
