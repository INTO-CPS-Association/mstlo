mod common;
mod fixtures;
#[cfg(test)]
use mstlo::stl::monitor::{Algorithm, EagerQualitative, Rosi, StlMonitor};
use rstest::rstest;

use fixtures::formulas::formula_1;

// ---
// Test Runner for Build Failures
// ---

#[rstest]
#[should_panic]
fn test_monitor_build_fails_bool_naive_eager() {
    // Eager mode + Naive strategy is an invalid combination
    let _ = StlMonitor::builder()
        .formula(formula_1())
        .algorithm(Algorithm::Naive)
        .semantics(EagerQualitative)
        .build()
        .unwrap();
}

#[rstest]
#[should_panic]
fn test_monitor_build_fails_bool_rosi() {
    // Eager mode + Naive strategy is an invalid combination
    let _ = StlMonitor::builder()
        .formula(formula_1())
        .algorithm(Algorithm::Naive)
        .semantics(Rosi)
        .build()
        .unwrap();
}
