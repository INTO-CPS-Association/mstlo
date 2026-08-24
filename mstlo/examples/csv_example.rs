//! Monitor a trace loaded from a CSV file against a property loaded from a file.
//!
//! Nothing about the signals is hard-coded: the CSV header names them, and the
//! property refers to them by those names.
//!
//! ```text
//! data/trace.csv          data/property.stl
//! time,temperature,...    G[0, 1](temperature < 30.0) && (pressure > 99.0)
//! 0.0,21.4,101.1
//! ...
//! ```
//!
//! Run with: `cargo run --example csv_example`

use mstlo::monitor::*;
use mstlo::{Step, intern, parse_stl};
use std::fs;
use std::time::Duration;

/// Resolves a path next to this example, so it runs from any directory.
macro_rules! data_path {
    ($name:literal) => {
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/data/", $name)
    };
}

/// Parses a `time,<signal>,..` CSV into one [`Step`] per cell.
fn read_trace(csv: &str) -> Vec<Step<f64>> {
    let mut lines = csv.lines().filter(|line| !line.trim().is_empty());

    // The header names the signals. `Step` stores `&'static str`, so the names
    // are interned: allocated once each, and reused on every later run.
    let header = lines.next().expect("CSV must have a header row");
    let signals: Vec<&'static str> = header
        .split(',')
        .skip(1) // column 0 is the timestamp
        .map(|name| intern(name.trim()))
        .collect();

    lines
        .flat_map(|line| {
            let mut cells = line.split(',').map(|cell| cell.trim());
            let time: f64 = cells.next().expect("missing timestamp").parse().unwrap();

            signals
                .iter()
                .zip(cells)
                .map(move |(&signal, cell)| {
                    Step::new(signal, cell.parse().unwrap(), Duration::from_secs_f64(time))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn main() {
    let csv = fs::read_to_string(data_path!("trace.csv")).expect("Cannot read trace.csv");
    let trace = read_trace(&csv);
    println!("Read {} steps from CSV", trace.len());

    // The property is text too, so it is parsed at runtime rather than by the
    // `stl!` macro: "the temperature stays below 30 while the pressure holds
    // above 99."
    let property =
        fs::read_to_string(data_path!("property.stl")).expect("Cannot read property.stl");
    let formula = parse_stl(property.trim()).expect("Invalid property");
    println!("Monitoring: {formula}");

    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(DelayedQuantitative)
        .build()
        .expect("Failed to build monitor");

    // Feed the parsed steps to the monitor in order.
    for step in &trace {
        println!(
            "t={:>4.1}s, {}={} ",
            step.timestamp.as_secs_f64(),
            step.signal,
            step.value,
        );
        for verdict in monitor.update(step).verdicts() {
            println!(
                "VERDICTS: \n \t t={:>4.1}s: {:>6.2}",
                verdict.timestamp.as_secs_f64(),
                verdict.value
            );
        }
    }
}
