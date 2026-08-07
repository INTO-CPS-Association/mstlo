//! Per-step memory profile of the paper's four specifications.
//!
//! Replays the synthetic signal through phi1..phi4 under each of the four
//! semantics and reads [`StlMonitor::total_size`] after every `update()`,
//! writing the whole series out so the footprint can be plotted against time.
//!
//! This is the per-step counterpart to `paper_benchmark_memory`, which runs the
//! same measurement over the full formula catalog but only reports the average
//! and maximum.  It is deliberately narrow -- four formulas, one pass each, no
//! timing -- because the point is the shape of the series, not a benchmark
//! number.
//!
//! One pass is not exactly reproducible: `total_size` charges the temporal
//! operators' `eval_buffer_set` at `HashSet::capacity()`, and a hash set grows
//! according to where its keys land, which `RandomState` reseeds per instance.
//! So individual steps can wobble by a slot or two between runs.  That is fine
//! at this scale -- the tiers being plotted are kilobytes apart -- but it is why
//! `paper_benchmark_memory` takes a median over passes for its aggregates.
//!
//! Environment overrides:
//!   SIGNAL_PATH  the signal to replay
//!   OUTPUT_CSV   per-step output

use mstlo::monitor::{
    Algorithm, DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor,
};
use mstlo::parse_stl;
use mstlo::{FormulaDefinition, RobustnessSemantics, SemanticType, Step, step};
use std::fmt::Debug;
use std::fs::{File, create_dir_all};
use std::hint::black_box;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_SIGNAL_PATH: &str =
    "../benchmarks/synthetic_signal/paper_results/signal_generation/signals/signal_20000_chirp.csv";
const DEFAULT_OUTPUT_CSV: &str = "benches/results/memory_profile_N=20000.csv";

/// phi1..phi4 of the paper catalog, matching IDs 1-4 in `paper_benchmark`.
const FORMULAS: [(usize, &str); 4] = [
    (1, "(x < 0.5) and (x > -0.5)"),
    (2, "G[0,1000] (x > 0.5 -> F[0,100] (x < 0.0))"),
    (3, "(x < 0.5) U[0,1000] (x < 0.0)"),
    (4, "(G[0,100] (x < 0.5)) or (G[100,150] (x > 0.0))"),
];

const SEMANTICS: [&str; 4] = [
    "DelayedQuantitative",
    "DelayedQualitative",
    "EagerQualitative",
    "Rosi",
];

fn env_string_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn read_signal<P: AsRef<Path>>(filename: P) -> io::Result<Vec<Step<f64>>> {
    let file = File::open(filename)?;
    let reader = io::BufReader::new(file);
    let mut signal = Vec::new();

    for line in reader.lines().skip(1) {
        let line = line?;
        let columns: Vec<&str> = line.split(',').collect();
        if columns.len() == 2
            && let (Ok(ts), Ok(val)) = (
                columns[0].trim().parse::<f64>(),
                columns[1].trim().parse::<f64>(),
            )
        {
            signal.push(step!("x", val, Duration::from_secs_f64(ts)));
        }
    }

    Ok(signal)
}

/// The monitor's footprint in bytes after each update, over one pass.
fn profile<S>(marker: S, formula: &FormulaDefinition, signal: &[Step<f64>]) -> Vec<usize>
where
    S: SemanticType,
    S::Output: RobustnessSemantics + Copy + Debug + 'static,
{
    let mut monitor: StlMonitor<f64, S::Output> = StlMonitor::builder()
        .formula(formula.clone())
        .algorithm(Algorithm::Incremental)
        .semantics(marker)
        .build()
        .unwrap();

    signal
        .iter()
        .map(|step| {
            black_box(monitor.update(step));
            monitor.total_size()
        })
        .collect()
}

/// `profile` for a semantics named at runtime; the marker types are distinct,
/// so the dispatch cannot be folded into the generic.
fn profile_for(semantics: &str, formula: &FormulaDefinition, signal: &[Step<f64>]) -> Vec<usize> {
    match semantics {
        "DelayedQuantitative" => profile(DelayedQuantitative, formula, signal),
        "DelayedQualitative" => profile(DelayedQualitative, formula, signal),
        "EagerQualitative" => profile(EagerQualitative, formula, signal),
        "Rosi" => profile(Rosi, formula, signal),
        other => panic!("unknown semantics '{other}'"),
    }
}

fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn main() -> io::Result<()> {
    let signal_path = env_string_or_default("SIGNAL_PATH", DEFAULT_SIGNAL_PATH);
    let output_path = PathBuf::from(env_string_or_default("OUTPUT_CSV", DEFAULT_OUTPUT_CSV));

    let signal = read_signal(&signal_path)?;
    println!("{} samples from {signal_path}", signal.len());

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(File::create(&output_path)?);
    writeln!(writer, "formula_id,spec,semantics,step,t,total_size_bytes")?;

    for (formula_id, spec) in FORMULAS {
        let formula = parse_stl(spec).unwrap_or_else(|e| panic!("invalid formula '{spec}': {e}"));
        let escaped = csv_escape(spec);

        for semantics in SEMANTICS {
            let sizes = profile_for(semantics, &formula, &signal);
            println!(
                "  phi{formula_id} {semantics:20} max {} B",
                sizes.iter().max().copied().unwrap_or(0)
            );

            for (step, (sample, size)) in signal.iter().zip(&sizes).enumerate() {
                writeln!(
                    writer,
                    "{formula_id},{escaped},{semantics},{step},{:.6},{size}",
                    sample.timestamp.as_secs_f64()
                )?;
            }
        }
        writer.flush()?;
    }

    println!("\nMemory series saved to {}", output_path.display());
    Ok(())
}
