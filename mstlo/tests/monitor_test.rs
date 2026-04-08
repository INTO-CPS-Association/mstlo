#[cfg(test)]
mod common;
mod fixtures;

use mstlo::Step;
use mstlo::monitor::{
    Algorithm, DelayedQualitative, DelayedQuantitative, EagerQualitative, StlMonitor,
};
use mstlo::{FormulaDefinition, RobustnessSemantics};
use pretty_assertions::assert_eq;
use rstest::rstest;
use std::fmt::Debug;
use std::vec;

use fixtures::formulas::*;
use fixtures::oracles::*;
use fixtures::signals::*;

/// This helper function contains the actual test logic.
/// It is called by the `rstest` runners below.
fn run_monitor_test<Y, S>(
    formulas: Vec<FormulaDefinition>,
    signal: Vec<Step<f64>>,
    strategy: Algorithm,
    semantics: S,
    expected: Vec<Vec<Step<Y>>>,
) where
    Y: RobustnessSemantics + 'static + Copy + Debug + PartialEq,
    S: mstlo::monitor::semantic_markers::SemanticType<Output = Y> + Copy,
{
    for (i, formula) in formulas.into_iter().enumerate() {
        let mut monitor = StlMonitor::builder()
            .formula(formula.clone())
            .algorithm(strategy)
            .semantics(semantics)
            .build()
            .unwrap();

        let mut all_results = Vec::new();
        for step in signal.clone() {
            let output = monitor.update(&step);
            println!(
                "Step at {:?}, Monitor Output: {:?}",
                step.timestamp, &output
            );
            all_results.push(output.all_raw_outputs());
        }

        assert_eq!(
            all_results,
            expected,
            "Test failed for formula at index {} ({}) with algorithm {:?} and semantics {:?}",
            i,
            monitor.specification(),
            strategy,
            S::as_enum()
        );
    }
}

#[rstest]
// --- f64 Delayed Cases ---
// These run with Robustness and are tested against
// both Naive and Incremental strategies.
#[case::f1_s1(vec![formula_1(), formula_1_alt(), formula_1_alt_2()], signal_1(), exp_f1_s1_f64_delayed())]
#[case::f2_s2(vec![formula_2()], signal_2(), exp_f2_s2_f64_delayed())]
#[case::f3_s3(vec![formula_3(), formula_3_alt()], signal_3(), exp_f3_s3_f64_delayed())]
#[case::f6_s2(vec![formula_6(), formula_6_alt()], signal_2(), exp_f6_s2_f64_delayed())]
#[case::f4_s3(vec![formula_4(), formula_5(), formula_5_alt()], signal_3(), exp_f4_s3_f64_delayed())]
#[case::f7_s3(vec![formula_7()], signal_3(), exp_f7_s3_f64_delayed())]
#[case::f8_s4(vec![formula_8()], signal_4(), exp_f8_s4_f64_delayed())]
fn test_f64_delayed(
    #[case] formulas: Vec<FormulaDefinition>,
    #[case] signal: Vec<Step<f64>>,
    #[case] expected: Vec<Vec<Step<f64>>>,
    #[values(Algorithm::Naive, Algorithm::Incremental)] strategy: Algorithm,
) {
    run_monitor_test(formulas, signal, strategy, DelayedQuantitative, expected);
}

#[rstest]
// --- bool Delayed Cases ---
// These run with DelayedQualitative and are tested against
// both Naive and Incremental strategies.
#[case::f1_s1(vec![formula_1(), formula_1_alt(), formula_1_alt_2()], signal_1(), exp_f1_s1_bool_delayed())]
#[case::f2_s2(vec![formula_2()], signal_2(), exp_f2_s2_bool_delayed())]
#[case::f3_s3(vec![formula_3(), formula_3_alt()], signal_3(), exp_f3_s3_bool_delayed())]
#[case::f6_s2(vec![formula_6(), formula_6_alt()], signal_2(), exp_f6_s2_bool_delayed())]
#[case::f4_s3(vec![formula_4(), formula_5(), formula_5_alt()], signal_3(), exp_f4_s3_bool_delayed())]
#[case::f7_s3(vec![formula_7()], signal_3(), exp_f7_s3_bool_delayed())]
#[case::f8_s4(vec![formula_8()], signal_4(), exp_f8_s4_bool_delayed())]
fn test_bool_delayed(
    #[case] formulas: Vec<FormulaDefinition>,
    #[case] signal: Vec<Step<f64>>,
    #[case] expected: Vec<Vec<Step<bool>>>,
    #[values(Algorithm::Naive, Algorithm::Incremental)] strategy: Algorithm,
) {
    run_monitor_test(formulas, signal, strategy, DelayedQualitative, expected);
}

#[rstest]
// --- bool Eager Cases ---
// These run with EagerSatisfaction and are tested *only*
// against the Incremental strategy (Naive+Eager is invalid).
#[case::f1_s1(vec![formula_1(), formula_1_alt(), formula_1_alt_2()], signal_1(), exp_f1_s1_bool_eager())]
#[case::f2_s2(vec![formula_2()], signal_2(), exp_f2_s2_bool_eager())]
#[case::f3_s3(vec![formula_3(), formula_3_alt()], signal_3(), exp_f3_s3_bool_eager())]
#[case::f6_s2(vec![formula_6(), formula_6_alt()], signal_2(), exp_f6_s2_bool_eager())]
#[case::f4_s3(vec![formula_4(), formula_5(), formula_5_alt()], signal_3(), exp_f4_s3_bool_eager())]
#[case::f7_s3(vec![formula_7()], signal_3(), exp_f7_s3_bool_eager())]
#[case::f8_s4(vec![formula_8()], signal_4(), exp_f8_s4_bool_eager())]
#[case::f10_s3(vec![formula_10()], signal_3(), exp_f10_s3_bool_eager())]
#[case::f11_s3(vec![formula_11()], signal_3(), exp_f11_s3_bool_eager())]
fn test_bool_eager(
    #[case] formulas: Vec<FormulaDefinition>,
    #[case] signal: Vec<Step<f64>>,
    #[case] expected: Vec<Vec<Step<bool>>>,
    #[values(Algorithm::Incremental)] strategy: Algorithm,
) {
    run_monitor_test(formulas, signal, strategy, EagerQualitative, expected);
}
