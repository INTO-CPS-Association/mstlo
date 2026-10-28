use mstlo::monitor::*;
use mstlo::{step, stl};

fn main() {
    // Build a monitor from the macro DSL.
    let formula = stl!(G[0, 2](x > 5.0));
    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .algorithm(Algorithm::Incremental)
        .semantics(DelayedQuantitative)
        .build()
        .unwrap();

    // Stream updates
    let out1 = monitor.update(&step!("x", 7.0, 0s));
    let out2 = monitor.update(&step!("x", 6.0, 1s));
    let out3 = monitor.update(&step!("x", 4.0, 2s));
    let out4 = monitor.update(&step!("x", 7.0, 3s));

    println!("{:?}", out1.verdicts());
    println!("{:?}", out2.verdicts());
    println!("{:?}", out3.verdicts());
    println!("{:?}", out4.verdicts());

    assert_eq!(out1.verdicts(), vec![]);
    assert_eq!(out2.verdicts(), vec![]);
    assert_eq!(out3.verdicts(), vec![step!("x", -1.0, 0s)]);
    assert_eq!(out4.verdicts(), vec![step!("x", -1.0, 1s)]);

    // The currently pending verdicts (for t=2s and t=3s) can be finished if you reached end of trace
    // This is done by holding last value of every signal
    let final_out = monitor.finish();
    println!("{:?}", final_out.verdicts());
    assert_eq!(
        final_out.verdicts(),
        vec![step!("", -1.0, 2s), step!("", 2.0, 3s)]
    )
}
