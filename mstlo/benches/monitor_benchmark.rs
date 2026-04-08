// In benches/monitor_benchmark.rs
use criterion::{
    AxisScale, Criterion, PlotConfiguration, Throughput, criterion_group, criterion_main,
};
use mstlo::monitor::{
    Algorithm, DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor,
};
use mstlo::ring_buffer::Step;
use mstlo::step;
use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
use std::time::Duration;

// in order to get a json file with all the needed info do:
// Install it: cargo install cargo-criterion
// Run: cargo criterion --message-format=json > results.json

// Helper to read a single signal CSV into a Vec<Step<f64>>
fn read_signal_from_csv<P>(filename: P) -> io::Result<Vec<Step<f64>>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename).expect("failed to read file: {filename}");
    let reader = io::BufReader::new(file);
    let mut signal = Vec::new();
    // we skip the first line which is the header
    for (i, line) in reader.lines().skip(1).enumerate() {
        if let Ok(value_str) = line {
            // we split the line by the comma and take the second element
            let columns: Vec<&str> = value_str.split(',').collect();
            if columns.len() == 2
                && let Ok(val) = columns[1].trim().parse::<f64>()
            {
                let t = Duration::from_secs_f64(i as f64);
                signal.push(step!("x", val, t));
            }
        }
    }
    Ok(signal)
}

/// Reads signals from the specified CSV files.
pub fn get_signals_from_csv() -> Vec<Vec<Step<f64>>> {
    let filenames = [
        "benches/signal_generation/signals/signal_5000.csv",
        "benches/signal_generation/signals/signal_10000.csv",
        "benches/signal_generation/signals/signal_20000.csv",
    ];
    let mut signals = Vec::new();

    for filename in &filenames {
        match read_signal_from_csv(filename) {
            Ok(signal) => signals.push(signal),
            Err(e) => panic!("Failed to read signal from {}: {}", filename, e),
        }
    }

    signals
}

// ---
// The Benchmark Function
// ---
fn benchmark_monitors(c: &mut Criterion) {
    let formulas = mstlo::formulas::get_formulas(&[4, 5, 6, 7, 8, 9]);
    let signals = get_signals_from_csv();

    for (id, formula) in formulas {
        // Create a temp monitor to get the specification string for the group name
        let temp = StlMonitor::builder()
            .formula(formula.clone())
            .semantics(DelayedQualitative)
            .build()
            .unwrap();
        let spec_str = temp.specification();
        let formula_name = format!("{}: {}", id, spec_str);

        let mut group = c.benchmark_group(&formula_name);
        group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

        // Increase sample size for better statistical significance
        group.sample_size(50);
        // Increase measurement time
        group.measurement_time(Duration::from_secs(3));

        for signal in &signals {
            let signal_size = signal.len();
            group.throughput(Throughput::Elements(signal_size as u64));

            // 1. Incremental bool Delayed
            group.bench_with_input(
                criterion::BenchmarkId::new("Incremental_bool_Delayed", signal_size),
                signal,
                |b, signal| {
                    b.iter_batched(
                        || {
                            let monitor: StlMonitor<f64, bool> = StlMonitor::builder()
                                .formula(formula.clone())
                                .algorithm(Algorithm::Incremental)
                                .semantics(DelayedQualitative)
                                .build()
                                .unwrap();
                            (monitor, signal.clone())
                        },
                        |(mut monitor, signal)| {
                            for step in signal {
                                monitor.update(&step);
                            }
                        },
                        // Use LargeInput to exclude the significant drop/clone time of the signal
                        criterion::BatchSize::LargeInput,
                    );
                },
            );

            // 2. Incremental f64 Delayed
            group.bench_with_input(
                criterion::BenchmarkId::new("Incremental_f64_Delayed", signal_size),
                signal,
                |b, signal| {
                    b.iter_batched(
                        || {
                            let monitor = StlMonitor::builder()
                                .formula(formula.clone())
                                .semantics(DelayedQuantitative)
                                .algorithm(Algorithm::Incremental)
                                .build()
                                .unwrap();
                            (monitor, signal.clone())
                        },
                        |(mut monitor, signal)| {
                            for step in signal {
                                monitor.update(&step);
                            }
                        },
                        criterion::BatchSize::LargeInput,
                    );
                },
            );

            // If this formula is among the first 12, also run Naive strategy (delayed) benchmarks
            if id <= 11 {
                // Naive bool Delayed
                group.bench_with_input(
                    criterion::BenchmarkId::new("Naive_bool_Delayed", signal_size),
                    signal,
                    |b, signal| {
                        b.iter_batched(
                            || {
                                let monitor: StlMonitor<f64, bool> = StlMonitor::builder()
                                    .formula(formula.clone())
                                    .algorithm(Algorithm::Naive)
                                    .semantics(DelayedQualitative)
                                    .build()
                                    .unwrap();
                                (monitor, signal.clone())
                            },
                            |(mut monitor, signal)| {
                                for step in signal {
                                    monitor.update(&step);
                                }
                            },
                            criterion::BatchSize::LargeInput,
                        );
                    },
                );

                // Naive f64 Delayed
                group.bench_with_input(
                    criterion::BenchmarkId::new("Naive_f64_Delayed", signal_size),
                    signal,
                    |b, signal| {
                        b.iter_batched(
                            || {
                                let monitor: StlMonitor<f64, f64> = StlMonitor::builder()
                                    .formula(formula.clone())
                                    .algorithm(Algorithm::Naive)
                                    .semantics(DelayedQuantitative)
                                    .build()
                                    .unwrap();
                                (monitor, signal.clone())
                            },
                            |(mut monitor, signal)| {
                                for step in signal {
                                    monitor.update(&step);
                                }
                            },
                            criterion::BatchSize::LargeInput,
                        );
                    },
                );
            }

            // 3. Incremental bool Eager
            group.bench_with_input(
                criterion::BenchmarkId::new("Incremental_bool_Eager", signal_size),
                signal,
                |b, signal| {
                    b.iter_batched(
                        || {
                            let monitor: StlMonitor<f64, bool> = StlMonitor::builder()
                                .formula(formula.clone())
                                .algorithm(Algorithm::Incremental)
                                .semantics(EagerQualitative)
                                .build()
                                .unwrap();
                            (monitor, signal.clone())
                        },
                        |(mut monitor, signal)| {
                            for step in signal {
                                monitor.update(&step);
                            }
                        },
                        criterion::BatchSize::LargeInput,
                    );
                },
            );

            // 4. Incremental RobustnessInterval Eager
            group.bench_with_input(
                criterion::BenchmarkId::new("Incremental_RoSI_Eager", signal_size),
                signal,
                |b, signal| {
                    b.iter_batched(
                        || {
                            let monitor = StlMonitor::builder()
                                .formula(formula.clone())
                                .semantics(Rosi)
                                .algorithm(Algorithm::Incremental)
                                .build()
                                .unwrap();
                            (monitor, signal.clone())
                        },
                        |(mut monitor, signal)| {
                            for step in signal {
                                monitor.update(&step);
                            }
                        },
                        criterion::BatchSize::LargeInput,
                    );
                },
            );
        }
        group.finish();
    }
}

criterion_group!(benches, benchmark_monitors);
criterion_main!(benches);
