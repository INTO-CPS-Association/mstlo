use mstlo::monitor::{Rosi, StlMonitor};
use mstlo::{step, stl};
use std::time::Duration;

fn main() {
    // Define a formula using the embedded DSL
    let formula = stl!(G[0, 2](x > 5.0));

    // Build the monitor
    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(Rosi)
        .build()
        .expect("Failed to build monitor");

    // Feed data steps to the monitor
    let out1 = monitor.update(&step!("x", 7.0, Duration::from_secs(0)));
    println!("Outputs for t=0s:\n {:?}", out1.verdicts());
    // [Step { signal: "x", value: RobustnessInterval(-inf, 2.0), timestamp: 0ns }] // at time 0, robustness value is in interval (-inf, 2.0)
    let out2 = monitor.update(&step!("x", 4.0, Duration::from_secs(1)));
    println!("Outputs for t=1s:\n {:?}", out2.verdicts());
    // Output after second update: [Step { signal: "x", value: RobustnessInterval(-inf, -1.0), timestamp: 0ns }, Step { signal: "x", value: RobustnessInterval(-inf, -1.0), timestamp: 1s }] // early violation detection for times 0 and 1
    // Finish the currently pending verdicts by holding last signal value
    let finished_output = monitor.finish();
    println!("Finished outputs:\n {:?}", finished_output.verdicts())
}
