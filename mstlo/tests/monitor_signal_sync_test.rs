#[cfg(test)]
mod common;
mod fixtures;

use mstlo::monitor::{Algorithm, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor};
use mstlo::step;
use mstlo::stl;
use mstlo::{Step, SynchronizationStrategy};
use rstest::rstest;
use std::time::Duration;
use std::vec;

use common::*;

#[rstest]
fn test_signal_interleaving(
    #[values(
        SynchronizationStrategy::ZeroOrderHold,
        SynchronizationStrategy::Linear
    )]
    interpolation_strategy: SynchronizationStrategy,
) {
    // test that outputs are correctly produced when signals are interleaved over multiple timesteps
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
        .formula(stl! { G[0,20]((x > 0) && (y < 150)) })
        .semantics(Rosi)
        .algorithm(Algorithm::Incremental)
        .synchronization_strategy(interpolation_strategy)
        .build()
        .unwrap();

    // feed step 0
    let out0 = monitor.update(&steps[0]);
    assert_eq!(out0.verdicts().len(), 0); // not enough data yet
    // feed step 1
    let out1 = monitor.update(&steps[1]);
    assert_eq!(out1.verdicts().len(), 1); // now we have both signals at t=0
    // feed step 2
    let out2 = monitor.update(&steps[2]);
    assert_eq!(out2.verdicts().len(), 1); // not enough data yet
    // feed step 3
    let out3 = monitor.update(&steps[3]);
    assert_eq!(out3.verdicts().len(), 1); // not enough data yet
    // feed step 4
    let out4 = monitor.update(&steps[4]);
    assert_eq!(out4.verdicts().len(), 1); // not enough data yet
    // feed step 5
    let out5 = monitor.update(&steps[5]);
    assert_eq!(out5.verdicts().len(), 1); // not enough data yet
    // feed step 6
    let out6 = monitor.update(&steps[6]);
    assert_eq!(out6.verdicts().len(), 5); // now we have both signals at t=10
}

#[rstest]
fn test_until_two_disjoint_signals(
    #[values(
        SynchronizationStrategy::ZeroOrderHold,
        SynchronizationStrategy::Linear
    )]
    strategy: SynchronizationStrategy,
) {
    let formula = stl! {G[0,2](x > 0) U[0, 5] (y > 5)};

    let x_steps = create_steps("x", vec![5.0, 3.0, 1.0, -7.0, 1.0], vec![0, 3, 4, 5, 7, 8]);
    let y_steps = create_steps("y", vec![1.0, 8.0, 8.0, 10.0], vec![2, 6, 9, 10]);
    let signal = combine_and_sort_steps(vec![x_steps, y_steps]);

    // Validate Incremental path (the one affected by eval_buffer changes)
    let mut incr_f64 = StlMonitor::builder()
        .formula(formula.clone())
        .semantics(DelayedQuantitative)
        .algorithm(Algorithm::Incremental)
        .synchronization_strategy(strategy)
        .build()
        .unwrap();

    let mut incr_bool = StlMonitor::builder()
        .formula(formula.clone())
        .semantics(EagerQualitative)
        .algorithm(Algorithm::Incremental)
        .synchronization_strategy(strategy)
        .build()
        .unwrap();

    let mut f64_per_step: Vec<Vec<Step<f64>>> = Vec::new();
    let mut bool_per_step: Vec<Vec<Step<bool>>> = Vec::new();
    for step in &signal {
        f64_per_step.push(incr_f64.update(step).all_raw_outputs());
        bool_per_step.push(incr_bool.update(step).all_raw_outputs());
    }

    eprintln!("=== Disjoint Until: {:?} ===", strategy);
    eprintln!("Raw signals: {:?}", signal);
    for (i, (f64_out, bool_out)) in f64_per_step.iter().zip(bool_per_step.iter()).enumerate() {
        eprintln!("  step {}: f64={:?}  bool={:?}", i, f64_out, bool_out);
    }

    let f64_outputs: Vec<_> = f64_per_step.into_iter().flatten().collect();
    let bool_outputs: Vec<_> = bool_per_step.into_iter().flatten().collect();

    assert!(
        !f64_outputs.is_empty(),
        "Should produce quantitative outputs for two-disjoint-signal Until"
    );
    assert!(
        !bool_outputs.is_empty(),
        "Should produce qualitative outputs for two-disjoint-signal Until"
    );
}

#[rstest]
fn test_synchronization(
    #[values(
        SynchronizationStrategy::ZeroOrderHold,
        SynchronizationStrategy::Linear
    )]
    interpolation_strategy: SynchronizationStrategy,
) {
    // x_steps are even timestamps from 0 to 100
    let x_steps: Vec<Step<f64>> = (0..101)
        .step_by(2)
        .map(|i| step!("x", i as f64, Duration::from_secs(i)))
        .collect();
    // y_steps are odd timestamps from 1 to 99
    let y_steps: Vec<Step<f64>> = (1..101)
        .step_by(2)
        .map(|i| step!("y", i as f64, Duration::from_secs(i)))
        .collect();

    let mut monitor = StlMonitor::builder()
        .formula(stl! { (x > 0) && (y < 150) })
        .semantics(Rosi)
        .algorithm(Algorithm::Incremental)
        .synchronization_strategy(interpolation_strategy)
        .build()
        .unwrap();

    let mut outputs = Vec::new();

    for step in combine_and_sort_steps(vec![x_steps, y_steps]) {
        let output = monitor.update(&step);
        outputs.push(output.all_raw_outputs());
    }

    // Check that we have outputs for all timestamps appearing in both x_steps and y_steps
    // note that '0' is excluded since y_steps starts at t=1
    // and '100' is excluded since y_steps ends at t=99
    let expected_timestamps: Vec<Duration> = (1..100).map(Duration::from_secs).collect();
    let output_timestamps: Vec<Duration> = outputs
        .iter()
        .flat_map(|steps| steps.iter().map(|s| s.timestamp))
        .collect();
    for ts in expected_timestamps {
        assert!(
            output_timestamps.contains(&ts),
            "Missing output for timestamp {:?}",
            ts
        );
    }
}
