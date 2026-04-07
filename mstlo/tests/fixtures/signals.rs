#![allow(dead_code)]

use mstlo::ring_buffer::Step;
use rstest::fixture;
use std::f64::consts::PI;
use std::time::Duration;

use crate::common::combine_and_sort_steps;
use crate::common::create_steps;

// ---
// Signal Fixtures
// ---

#[fixture]
#[once]
pub fn signal_1() -> Vec<Step<f64>> {
    create_steps("x", vec![5.0, 4.0, 6.0, 2.0, 5.0], vec![0, 1, 2, 3, 4])
}

#[fixture]
#[once]
pub fn signal_2() -> Vec<Step<f64>> {
    create_steps(
        "x",
        vec![1.0, 1.0, 1.0, 2.0, 3.0, 4.0, 0.0, 0.0, 0.0, 1.0, 2.0],
        vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    )
}

#[fixture]
#[once]
pub fn signal_3() -> Vec<Step<f64>> {
    create_steps(
        "x",
        vec![0.0, 6.0, 1.0, 0.0, 8.0, 1.0, 7.0],
        vec![0, 1, 2, 3, 4, 5, 6],
    )
}

#[fixture]
#[once]
pub fn signal_4() -> Vec<Step<f64>> {
    let x_steps = create_steps(
        "x",
        vec![0.0, 6.0, 1.0, 3.0, 8.0, 1.0, 7.0],
        vec![0, 1, 2, 3, 4, 5, 6],
    );
    let y_steps = create_steps(
        "y",
        vec![4.0, 3.0, 6.0, 7.0, 2.0, 1.0, 0.0],
        vec![0, 1, 2, 3, 4, 5, 6],
    );

    // Combine and sort the steps chronologically
    combine_and_sort_steps(vec![x_steps, y_steps])
}

#[fixture]
#[once]
pub fn sparse_timestamps() -> Vec<Step<f64>> {
    create_steps(
        "x",
        vec![0.0, 6.0, 1.0, 3.0, 8.0, 1.0, 7.0],
        vec![0, 1, 2, 5, 10, 20, 70],
    )
}

#[fixture]
#[once]
pub fn monotonic_increasing() -> Vec<Step<f64>> {
    const N: usize = 100;
    (0..N)
        .map(|i| {
            let timestamp = Duration::from_secs(i as u64);
            let t = i as f64 / (N as f64 - 1.0);
            let value = -10.0 + 20.0 * t; // from -10 to 10
            Step::new("x", value, timestamp)
        })
        .collect()
}
#[fixture]
#[once]
pub fn monotonic_decreasing() -> Vec<Step<f64>> {
    const N: usize = 100;
    (0..N)
        .map(|i| {
            let timestamp = Duration::from_secs(i as u64);
            let t = i as f64 / (N as f64 - 1.0);
            let value = 10.0 - 20.0 * t; // from 10 to -10
            Step::new("x", value, timestamp)
        })
        .collect()
}
#[fixture]
#[once]
pub fn sinusoid() -> Vec<Step<f64>> {
    const N: usize = 100;
    (0..N)
        .map(|i| {
            let timestamp = Duration::from_secs(i as u64);
            let t = i as f64 / (N as f64 - 1.0);
            let value = 10.0 * (2.0 * PI * t).sin();
            Step::new("x", value, timestamp)
        })
        .collect()
}
