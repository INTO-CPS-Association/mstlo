//! Monitor a trace from `data/trace.csv` against the property in
//! `data/property.stl`.
//!
//! Run with: `cargo run --example csv_example`

use mstlo::monitor::*;
use mstlo::{Step, intern, parse_stl, step};
use std::fs;
use std::time::Duration;

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/data/");

/// Parses a `time,<signal>,..` CSV into one [`Step`] per cell.
fn read_trace(path: String) -> Vec<Step<f64>> {
    let mut reader = csv::Reader::from_path(path).expect("Cannot read trace");

    // The header names the signals. `Step` stores `&'static str`, so the names
    // are interned. Column 0 is the timestamp.
    let signals: Vec<&'static str> = reader
        .headers()
        .expect("CSV must have a header row")
        .iter()
        .skip(1)
        .map(intern)
        .collect();

    let mut trace = Vec::new();
    for record in reader.records() {
        let record = record.expect("Malformed CSV row");
        let mut cells = record.iter();
        let time: f64 = cells.next().expect("missing timestamp").parse().unwrap();

        for (&signal, cell) in signals.iter().zip(cells) {
            let value: f64 = cell.parse().expect("value must be a number");
            trace.push(step!(signal, value, Duration::from_secs_f64(time)));
        }
    }
    trace
}

fn main() {
    let trace = read_trace(format!("{DATA}trace.csv"));
    println!("Read {} steps from CSV", trace.len());

    // The property is text, so it is parsed at runtime rather than by the
    // `stl!` macro.
    let property = fs::read_to_string(format!("{DATA}property.stl")).expect("Cannot read property");
    let formula = parse_stl(property.trim()).expect("Invalid property");
    println!("Monitoring: {formula}");

    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(DelayedQuantitative)
        .build()
        .expect("Failed to build monitor");

    // Feed the parsed steps to the monitor in order.
    for step in &trace {
        for verdict in monitor.update(step).verdicts() {
            println!(
                "t={:>4.1}s: {:>6.2}",
                verdict.timestamp.as_secs_f64(),
                verdict.value
            );
        }
    }

    // The whole trace can also go in as one batch. `update_batch` takes any
    // iterable of steps, so the parsed `Vec` goes straight in — there is no
    // need to group it by signal first — and it sorts by timestamp, so a trace
    // read out of order still evaluates chronologically.
    //
    // For this property the two forms produce identical verdicts. They can
    // differ under `Rosi`, where a batch collapses the refinements of one
    // timestamp into its final value, whereas the loop above reports each
    // refinement as it happens.
    //
    // The `reset()` is only needed to run this *after* the loop above, which
    // has already advanced the monitor past the end of the trace; replacing the
    // loop outright needs only the two lines that follow it.
    //
    // monitor.reset();
    // for verdict in monitor.update_batch(&trace).verdicts() {
    //     println!(
    //         "t={:>4.1}s: {:>6.2}",
    //         verdict.timestamp.as_secs_f64(),
    //         verdict.value
    //     );
    // }
}
