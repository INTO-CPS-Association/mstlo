use mstlo::ring_buffer::Step;
use mstlo::stl::monitor::{Rosi, StlMonitor};
use std::time::Duration;

fn main() {
    // Define a formula using the embedded DSL
    let formula = mstlo::stl!(G[0, 2](x > 5.0));

    // Build the monitor
    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(Rosi)
        .build()
        .expect("Failed to build monitor");

    // Feed data steps to the monitor
    let out1 = monitor.update(&Step::new("x", 7.0, Duration::from_secs(0)));
    println!("{:?}", out1.verdicts());
    // [Step { signal: "x", value: RobustnessInterval(-inf, 2.0), timestamp: 0ns }] // at time 0, robustness value is in interval (-inf, 2.0)
    let out2 = monitor.update(&Step::new("x", 4.0, Duration::from_secs(1)));
    println!("{:?}", out2.verdicts());
    // Output after second update: [Step { signal: "x", value: RobustnessInterval(-inf, -1.0), timestamp: 0ns }, Step { signal: "x", value: RobustnessInterval(-inf, -1.0), timestamp: 1s }] // early violation detection for times 0 and 1
}
