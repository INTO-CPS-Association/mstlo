mod common;

use common::*;
use mstlo::monitor::{Algorithm, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor};
use mstlo::{RobustnessInterval, Step, step, stl};
use std::collections::HashSet;
use std::time::Duration;

/// A conjunction is known only where both signals are. The timestamps `x` reports since
/// `y @0s` wait for `y @10s`, which settles all four at once.
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

    let mut monitor = StlMonitor::builder()
        .formula(stl! { (x > 0) && (y < 150) })
        .semantics(Rosi)
        .algorithm(Algorithm::Incremental)
        .initialize_signals(initial_values(&steps))
        .build()
        .unwrap();

    let verdicts: Vec<_> = steps
        .iter()
        .map(|step| monitor.update(step).verdicts())
        .collect();
    let at = |update: usize, secs: u64| {
        verdicts[update]
            .iter()
            .find(|verdict| verdict.timestamp == Duration::from_secs(secs))
            .map(|verdict| verdict.value)
    };

    // Nothing past `y`'s frontier is reported before it moves, not even bounded by `x`.
    for update in 2..=5 {
        assert_eq!(at(update, 2), None, "update {update}");
    }
    for secs in [2, 4, 6, 8] {
        assert_eq!(
            at(6, secs),
            Some(RobustnessInterval(1.0, 1.0)),
            "at {secs}s"
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
        .initialize_signals(initial_values(&signal))
        .build()
        .unwrap();
    let mut eager = StlMonitor::builder()
        .formula(formula)
        .semantics(EagerQualitative)
        .algorithm(Algorithm::Incremental)
        .initialize_signals(initial_values(&signal))
        .build()
        .unwrap();

    let (mut quantitative_verdicts, mut eager_verdicts) = (0, 0);
    for step in &signal {
        quantitative_verdicts += quantitative.update(step).total_raw_outputs();
        eager_verdicts += eager.update(step).total_raw_outputs();
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

    let signal = combine_and_sort_steps(vec![x_steps, y_steps]);
    let mut monitor = StlMonitor::builder()
        .formula(stl! { (x > 0) && (y < 150) })
        .semantics(Rosi)
        .algorithm(Algorithm::Incremental)
        .initialize_signals(initial_values(&signal))
        .build()
        .unwrap();

    let answered: HashSet<Duration> = signal
        .iter()
        .flat_map(|step| monitor.update(step).all_raw_outputs())
        .map(|verdict| verdict.timestamp)
        .collect();
    for ts in (1..100).map(Duration::from_secs) {
        assert!(answered.contains(&ts), "missing verdict at {ts:?}");
    }
}
