#[cfg(test)]
mod common;
mod fixtures;

use mstlo::ring_buffer::Step;
use mstlo::stl;
use mstlo::monitor::{Algorithm, Rosi, StlMonitor};
use mstlo::synchronizer::SynchronizationStrategy;
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
        Step::new("x", 1.0, Duration::from_secs(0)),
        Step::new("y", 1.0, Duration::from_secs(0)),
        Step::new("x", 1.0, Duration::from_secs(2)),
        Step::new("x", 1.0, Duration::from_secs(4)),
        Step::new("x", 1.0, Duration::from_secs(6)),
        Step::new("x", 1.0, Duration::from_secs(8)),
        Step::new("y", 1.0, Duration::from_secs(10)),
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
        .map(|i| Step::new("x", i as f64, Duration::from_secs(i)))
        .collect();
    // y_steps are odd timestamps from 1 to 99
    let y_steps: Vec<Step<f64>> = (1..101)
        .step_by(2)
        .map(|i| Step::new("y", i as f64, Duration::from_secs(i)))
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
