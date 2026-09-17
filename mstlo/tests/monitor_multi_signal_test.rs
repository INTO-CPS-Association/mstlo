mod common;

use common::*;
use mstlo::monitor::{Algorithm, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor};
use mstlo::{Step, step, stl};
use std::time::Duration;

/// A conjunction answers only where both signals are known, so `y @10s` releases the
/// timestamps `x` reported since `y @0s`.
#[test]
fn interleaved_signals_answer_once_both_are_known() {
    let steps = [
        step!("x", 1.0, Duration::from_secs(0)),
        step!("y", 1.0, Duration::from_secs(0)),
        step!("x", 1.0, Duration::from_secs(2)),
        step!("x", 1.0, Duration::from_secs(4)),
        step!("x", 1.0, Duration::from_secs(6)),
        step!("x", 1.0, Duration::from_secs(8)),
        step!("y", 1.0, Duration::from_secs(10)),
    ];
    let expected_verdicts = [0, 1, 1, 1, 1, 1, 5];

    let mut monitor = StlMonitor::builder()
        .formula(stl! { G[0,20]((x > 0) && (y < 150)) })
        .semantics(Rosi)
        .algorithm(Algorithm::Incremental)
        .build()
        .unwrap();

    for (step, expected) in steps.iter().zip(expected_verdicts) {
        assert_eq!(
            monitor.update(step).verdicts().len(),
            expected,
            "verdicts after {step:?}"
        );
    }
}

/// `Until` whose operands are sampled on disjoint grids still answers.
#[test]
fn until_over_disjoint_signals_produces_verdicts() {
    let formula = stl! {G[0,2](x > 0) U[0, 4] (y > 5)};

    let x_steps = create_steps(
        "x",
        vec![5.0, 3.0, 1.0, -7.0, 1.0, 1.0],
        vec![0, 3, 4, 5, 7, 8],
    );
    let y_steps = create_steps("y", vec![1.0, 8.0, 8.0, 10.0], vec![2, 6, 9, 10]);
    let signal = combine_and_sort_steps(vec![x_steps, y_steps]);

    let mut quantitative = StlMonitor::builder()
        .formula(formula.clone())
        .semantics(DelayedQuantitative)
        .algorithm(Algorithm::Incremental)
        .build()
        .unwrap();
    let mut eager = StlMonitor::builder()
        .formula(formula)
        .semantics(EagerQualitative)
        .algorithm(Algorithm::Incremental)
        .build()
        .unwrap();

    let (mut quantitative_verdicts, mut eager_verdicts) = (0, 0);
    for step in &signal {
        quantitative_verdicts += quantitative.update(step).all_raw_outputs().len();
        eager_verdicts += eager.update(step).all_raw_outputs().len();
    }
    assert!(quantitative_verdicts > 0, "no DelayedQuantitative verdicts");
    assert!(eager_verdicts > 0, "no EagerQualitative verdicts");
}

/// `x` on even and `y` on odd seconds: every second from 1s to 99s, where both are known, is
/// answered.
#[test]
fn conjunction_answers_every_joint_timestamp() {
    let x_steps: Vec<Step<f64>> = (0..101)
        .step_by(2)
        .map(|i| step!("x", i as f64, Duration::from_secs(i)))
        .collect();
    let y_steps: Vec<Step<f64>> = (1..101)
        .step_by(2)
        .map(|i| step!("y", i as f64, Duration::from_secs(i)))
        .collect();

    let mut monitor = StlMonitor::builder()
        .formula(stl! { (x > 0) && (y < 150) })
        .semantics(Rosi)
        .algorithm(Algorithm::Incremental)
        .build()
        .unwrap();

    let answered: Vec<Duration> = combine_and_sort_steps(vec![x_steps, y_steps])
        .iter()
        .flat_map(|step| monitor.update(step).all_raw_outputs())
        .map(|verdict| verdict.timestamp)
        .collect();
    for ts in (1..100).map(Duration::from_secs) {
        assert!(answered.contains(&ts), "missing verdict at {ts:?}");
    }
}
