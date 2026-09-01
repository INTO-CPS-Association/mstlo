//! Feed a whole trace to a monitor at once with [`StlMonitor::update_batch`].
//!
//! Run with: `cargo run --example batch_update`

use mstlo::monitor::*;
use mstlo::{steps, stl};

fn main() {
    let formula = stl!(x > 10.0);

    let mut batch_monitor = StlMonitor::builder()
        .formula(formula.clone())
        .semantics(Rosi)
        .build()
        .expect("Failed to build monitor");

    // Signal-major form: one trace per signal, so the name is written once.
    // Use this when you have the samples grouped by signal.
    let output = batch_monitor.update_batch(&steps! {
        "x": [
            (5.0, 0s),  // robustness: 5 - 10 = -5
            (15.0, 1s), // robustness: 15 - 10 = 5
            (8.0, 2s),  // robustness: 8 - 10 = -2
            (12.0, 3s), // robustness: 12 - 10 = 2
        ],
    });

    println!("Batch Update Results:");
    println!("{output}");

    // Flat form: each entry is a `step!` argument list. Use this when samples
    // from different signals are interleaved, e.g. read from a log.
    let mut interleaved_monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(Rosi)
        .build()
        .expect("Failed to build monitor");

    let output = interleaved_monitor.update_batch(&steps![
        ("x", 5.0, 0s),
        ("x", 15.0, 1s),
        ("x", 8.0, 2s),
        ("x", 12.0, 3s),
    ]);

    println!("\nSame trace, flat form:");
    println!("{output}");
}
