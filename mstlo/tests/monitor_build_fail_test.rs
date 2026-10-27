mod common;
mod fixtures;
#[cfg(test)]
use mstlo::monitor::{Algorithm, EagerQualitative, Rosi, StlMonitor};
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

use mstlo::monitor::DelayedQualitative;
use mstlo::{FormulaDefinition, MAX_TEMPORAL_DEPTH, Step, TimeInterval};
use std::time::Duration;

/// `G[0, 10](F[start, end] x > 0)`.
fn nested(start: Duration, end: Duration) -> FormulaDefinition {
    FormulaDefinition::Globally(
        TimeInterval {
            start: Duration::ZERO,
            end: Duration::from_secs(10),
        },
        Box::new(FormulaDefinition::Eventually(
            TimeInterval { start, end },
            Box::new(FormulaDefinition::GreaterThan("x", 0.0)),
        )),
    )
}

#[test]
fn test_monitor_build_fails_on_invalid_intervals() {
    for formula in [
        // Used to panic with "overflow when adding durations".
        nested(Duration::ZERO, Duration::MAX),
        // Representable, but over the limit.
        nested(Duration::ZERO, MAX_TEMPORAL_DEPTH),
        // `TimeInterval` fields are public, so `start > end` can bypass the parser.
        nested(Duration::from_secs(5), Duration::from_secs(1)),
    ] {
        let result = StlMonitor::builder()
            .formula(formula)
            .semantics(Rosi)
            .build();
        assert!(result.is_err());
    }
}

#[test]
fn test_monitor_runs_at_max_temporal_depth() {
    // With `Duration::MAX` as the bound, the RoSI monitor used to panic on the second sample.
    let formula = FormulaDefinition::Eventually(
        TimeInterval {
            start: Duration::ZERO,
            end: MAX_TEMPORAL_DEPTH,
        },
        Box::new(FormulaDefinition::GreaterThan("x", 0.0)),
    );
    let builder = || StlMonitor::builder().formula(formula.clone());
    let mut delayed = builder().semantics(DelayedQualitative).build().unwrap();
    let mut eager = builder().semantics(EagerQualitative).build().unwrap();
    let mut rosi = builder().semantics(Rosi).build().unwrap();
    for t in [0, 1, 2, (1 << 63) - 1] {
        let step = Step::new("x", -1.0, Duration::from_secs(t));
        delayed.update(&step);
        eager.update(&step);
        rosi.update(&step);
    }
}
