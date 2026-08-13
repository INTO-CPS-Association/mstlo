//! Native benchmark for the incubator experiment.
//!
//! Feeds the temperature signal recorded from the incubator digital twin
//! through the two deployed specifications and times every `update()` call,
//! mirroring what the Python side of that experiment measures so the numbers
//! can be put side by side.
//!
//! Each formula is also profiled for memory: extra, untimed passes over the
//! signal that read [`StlMonitor::total_size`] after every update and write the
//! whole series out, so the monitor's footprint can be plotted against the
//! signal it is watching.  Those passes are separate from the timed runs, so
//! the sampling never shows up in the timings.  They are repeated because a
//! single pass is not reproducible -- see `profile_memory`.
//!
//! The `track-cache-size` feature adds the number of cached steps held in the
//! ring buffers, read from [`GLOBAL_CACHE_SIZE`] after every update, as two more
//! columns in the summary.  As in `paper_benchmark`, that counter is sampled
//! *inside* the timed loop, so a run with the feature on does not produce
//! comparable timings -- give it its own output file and a single run
//! (`M_RUNS=1 WARMUP_RUNS=0`), the way `run_incubator_bench.sh` does.
//!
//! Environment overrides:
//!   SIGNAL_PATH        the recorded signal
//!   PHASES             comma-separated phases to benchmark (empty: all)
//!   M_RUNS             timed runs per formula
//!   WARMUP_RUNS        untimed runs first
//!   MEMORY_RUNS        untimed memory passes per formula (0: no profiling)
//!   OUTPUT_CSV         summary output
//!   OUTPUT_RAW_CSV     per-run output
//!   OUTPUT_MEMORY_CSV  per-step memory output

#[cfg(feature = "track-cache-size")]
use mstlo::GLOBAL_CACHE_SIZE;
use mstlo::monitor::{Algorithm, DelayedQuantitative, StlMonitor};
use mstlo::parse_stl;
use mstlo::{Step, step};
use std::fs::{File, create_dir_all};
use std::hint::black_box;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};
#[cfg(feature = "track-cache-size")]
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const DEFAULT_M_RUNS: usize = 50;
const DEFAULT_WARMUP_RUNS: usize = 1;
const DEFAULT_MEMORY_RUNS: usize = 21;
const DEFAULT_SIGNAL_PATH: &str = "signal.csv";
/// Empty: the whole recorded session, matching `replay.py` and the Python
/// benchmark, so every side sees one signal.
const DEFAULT_PHASES: &str = "";
const DEFAULT_OUTPUT_CSV: &str = "benchmark_rust.csv";
const DEFAULT_OUTPUT_RAW_CSV: &str = "benchmark_rust_runs.csv";
const DEFAULT_OUTPUT_MEMORY_CSV: &str = "benchmark_rust_memory.csv";

/// The deployed specifications. Thresholds are constant in these recordings,
/// so they are inlined here where the service carries them as runtime
/// variables.
const FORMULAS: [(&str, &str); 2] = [
    ("phi_hi", "G[0,30] ((T >= 39.0) -> F[0,360] (T <= 39.0))"),
    ("phi_lo", "G[0,30] ((T <= 36.0) -> F[0,360] (T >= 36.0))"),
];

struct BenchResult {
    spec_name: String,
    spec: String,
    n_samples: usize,
    m_runs: usize,
    avg_total_s: f64,
    std_total_s: f64,
    avg_per_sample_s: f64,
    std_per_sample_s: f64,
    /// `None` when the memory passes were switched off with `MEMORY_RUNS=0`.
    avg_total_size_bytes: Option<f64>,
    max_total_size_bytes: Option<usize>,
    /// Cached steps held in the ring buffers, averaged over every sampled
    /// update of every timed run.
    #[cfg(feature = "track-cache-size")]
    avg_cache_size: f64,
    #[cfg(feature = "track-cache-size")]
    max_cache_size: usize,
}

/// Monitor footprint in bytes after the update at `t` seconds.
struct MemorySample {
    t: f64,
    total_size_bytes: usize,
}

fn env_usize_or_default(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn env_string_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Read `t` and `temperature` from a recorded incubator signal, locating the
/// columns by name so the file can carry the other recorded fields too.
///
/// Rows are kept only if their `phase` is in `phases`; an empty `phases` keeps
/// the recording whole, which is the default.  Feeding the monitors a different
/// stretch of the session than the Python side would make the two sets of
/// numbers incomparable, which is the whole point of running both, so narrow
/// this only together with `replay.py` and `benchmark.py`.  Recordings from
/// before phases were labelled have no such column and are read whole.
fn read_signal<P: AsRef<Path>>(filename: P, phases: &[String]) -> io::Result<Vec<Step<f64>>> {
    let file = File::open(&filename)?;
    let reader = io::BufReader::new(file);
    let mut lines = reader.lines();

    let header = lines
        .next()
        .transpose()?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "signal file is empty"))?;
    let columns: Vec<&str> = header.split(',').map(str::trim).collect();
    let index_of = |name: &str| -> io::Result<usize> {
        columns.iter().position(|c| *c == name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("column '{name}' not found in {:?}", columns),
            )
        })
    };
    let (t_index, value_index) = (index_of("t")?, index_of("temperature")?);
    let phase_index = columns.iter().position(|c| *c == "phase");

    let mut signal = Vec::new();
    for line in lines {
        let line = line?;
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() <= t_index.max(value_index) {
            continue;
        }
        if let Some(i) = phase_index
            && !phases.is_empty()
            && !fields
                .get(i)
                .is_some_and(|p| phases.iter().any(|w| w == p.trim()))
        {
            continue;
        }
        if let (Ok(t), Ok(value)) = (
            fields[t_index].trim().parse::<f64>(),
            fields[value_index].trim().parse::<f64>(),
        ) {
            signal.push(step!("T", value, Duration::from_secs_f64(t)));
        }
    }

    Ok(signal)
}

fn mean_sd(values: &[f64]) -> (f64, f64) {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if values.len() < 2 {
        return (mean, 0.0);
    }
    let variance =
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    (mean, variance.sqrt())
}

fn build_monitor(formula: &mstlo::FormulaDefinition) -> StlMonitor<f64, f64> {
    StlMonitor::builder()
        .formula(formula.clone())
        .algorithm(Algorithm::Incremental)
        .semantics(DelayedQuantitative)
        .build()
        .unwrap()
}

// Per-step footprint
fn profile_memory(
    formula: &mstlo::FormulaDefinition,
    signal: &[Step<f64>],
    runs: usize,
) -> Vec<MemorySample> {
    if runs == 0 {
        return Vec::new();
    }

    let mut passes: Vec<Vec<usize>> = Vec::with_capacity(runs);

    for _ in 0..runs {
        let mut monitor = build_monitor(formula);
        let mut sizes = Vec::with_capacity(signal.len());
        for step in signal {
            black_box(monitor.update(step));
            sizes.push(monitor.total_size());
        }
        passes.push(sizes);
    }

    let mut at_step: Vec<usize> = Vec::with_capacity(runs);
    signal
        .iter()
        .enumerate()
        .map(|(i, step)| {
            at_step.clear();
            at_step.extend(passes.iter().map(|pass| pass[i]));
            at_step.sort_unstable();
            MemorySample {
                t: step.timestamp.as_secs_f64(),
                total_size_bytes: at_step[at_step.len() / 2],
            }
        })
        .collect()
}

fn bench_formula(
    spec_name: &str,
    spec: &str,
    signal: &[Step<f64>],
    m_runs: usize,
    warmup_runs: usize,
    memory_runs: usize,
) -> (BenchResult, Vec<f64>, Vec<MemorySample>) {
    let formula = parse_stl(spec).unwrap_or_else(|e| panic!("invalid formula '{spec}': {e}"));
    let n_samples = signal.len();

    let memory = profile_memory(&formula, signal, memory_runs);
    let total_size_sum: u128 = memory.iter().map(|m| m.total_size_bytes as u128).sum();
    let avg_total_size = (!memory.is_empty()).then(|| total_size_sum as f64 / n_samples as f64);
    let max_total_size = memory.iter().map(|m| m.total_size_bytes).max();

    let mut run_times: Vec<f64> = Vec::with_capacity(m_runs);
    #[cfg(feature = "track-cache-size")]
    let mut cache_size_sum: u128 = 0;
    #[cfg(feature = "track-cache-size")]
    let mut max_cache_size = 0usize;

    for run in 0..(warmup_runs + m_runs) {
        // The counter is global, so the previous run's monitor has to be gone
        // and the count back at zero before this one starts.
        #[cfg(feature = "track-cache-size")]
        GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);

        let mut monitor = build_monitor(&formula);

        #[cfg(not(feature = "track-cache-size"))]
        let elapsed = {
            let t0 = Instant::now();
            for step in signal {
                black_box(monitor.update(step));
            }
            t0.elapsed().as_secs_f64()
        };

        #[cfg(feature = "track-cache-size")]
        let (elapsed, run_cache_sum, run_cache_max) = {
            let (mut sum, mut max) = (0usize, 0usize);
            let t0 = Instant::now();
            for step in signal {
                black_box(monitor.update(step));
                let current = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                sum += current;
                max = max.max(current);
            }
            (t0.elapsed().as_secs_f64(), sum, max)
        };

        if run >= warmup_runs {
            run_times.push(elapsed);
            #[cfg(feature = "track-cache-size")]
            {
                cache_size_sum += run_cache_sum as u128;
                max_cache_size = max_cache_size.max(run_cache_max);
            }
        }

        drop(monitor);
        #[cfg(feature = "track-cache-size")]
        {
            let residual = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
            if residual != 0 {
                eprintln!(
                    "  warning: residual GLOBAL_CACHE_SIZE={residual} after run for {spec_name}"
                );
                GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
            }
        }
    }

    let per_sample: Vec<f64> = run_times.iter().map(|t| t / n_samples as f64).collect();
    let (avg_total, std_total) = mean_sd(&run_times);
    let (avg_per_sample, std_per_sample) = mean_sd(&per_sample);

    (
        BenchResult {
            spec_name: spec_name.to_string(),
            spec: spec.to_string(),
            n_samples,
            m_runs,
            avg_total_s: avg_total,
            std_total_s: std_total,
            avg_per_sample_s: avg_per_sample,
            std_per_sample_s: std_per_sample,
            avg_total_size_bytes: avg_total_size,
            max_total_size_bytes: max_total_size,
            #[cfg(feature = "track-cache-size")]
            avg_cache_size: cache_size_sum as f64 / (n_samples as f64 * m_runs as f64),
            #[cfg(feature = "track-cache-size")]
            max_cache_size,
        },
        run_times,
        memory,
    )
}

fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn write_header(w: &mut BufWriter<File>) -> io::Result<()> {
    let columns = "tool,spec_name,spec,semantics,algorithm,mode,n_samples,m_runs,\
         avg_total_s,std_total_s,avg_per_sample_s,std_per_sample_s,\
         avg_per_sample_us,std_per_sample_us,avg_total_size_bytes,max_total_size_bytes";
    #[cfg(feature = "track-cache-size")]
    {
        writeln!(w, "{columns},avg_cache_size,max_cache_size")
    }
    #[cfg(not(feature = "track-cache-size"))]
    {
        writeln!(w, "{columns}")
    }
}

/// The memory columns are left empty rather than zero where the passes were
/// switched off, so a reader cannot mistake "not measured" for "nothing".
fn write_row(w: &mut BufWriter<File>, r: &BenchResult) -> io::Result<()> {
    write!(
        w,
        "mstlo-rust,{},{},DelayedQuantitative,Incremental,online,{},{},\
         {:.12},{:.12},{:.12},{:.12},{:.6},{:.6},{},{}",
        r.spec_name,
        csv_escape(&r.spec),
        r.n_samples,
        r.m_runs,
        r.avg_total_s,
        r.std_total_s,
        r.avg_per_sample_s,
        r.std_per_sample_s,
        r.avg_per_sample_s * 1e6,
        r.std_per_sample_s * 1e6,
        r.avg_total_size_bytes
            .map_or(String::new(), |v| format!("{v:.3}")),
        r.max_total_size_bytes
            .map_or(String::new(), |v| v.to_string()),
    )?;
    #[cfg(feature = "track-cache-size")]
    write!(w, ",{:.6},{}", r.avg_cache_size, r.max_cache_size)?;
    writeln!(w)
}

fn write_raw_header(w: &mut BufWriter<File>) -> io::Result<()> {
    writeln!(
        w,
        "tool,spec_name,run_id,total_s,per_sample_s,per_sample_us"
    )
}

fn write_raw_row(
    w: &mut BufWriter<File>,
    r: &BenchResult,
    run_id: usize,
    total_s: f64,
) -> io::Result<()> {
    let per_sample_s = total_s / r.n_samples as f64;
    writeln!(
        w,
        "mstlo-rust,{},{},{:.12},{:.12},{:.6}",
        r.spec_name,
        run_id,
        total_s,
        per_sample_s,
        per_sample_s * 1e6
    )
}

fn write_memory_header(w: &mut BufWriter<File>) -> io::Result<()> {
    writeln!(w, "tool,spec_name,step,t,total_size_bytes")
}

fn write_memory_rows(
    w: &mut BufWriter<File>,
    spec_name: &str,
    memory: &[MemorySample],
) -> io::Result<()> {
    for (step, sample) in memory.iter().enumerate() {
        writeln!(
            w,
            "mstlo-rust,{},{},{:.6},{}",
            spec_name, step, sample.t, sample.total_size_bytes
        )?;
    }
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_dir_all(parent)?;
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let m_runs = env_usize_or_default("M_RUNS", DEFAULT_M_RUNS);
    let warmup_runs = std::env::var("WARMUP_RUNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_WARMUP_RUNS);
    // 0 is meaningful here -- it switches the memory passes off entirely.
    let memory_runs = std::env::var("MEMORY_RUNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MEMORY_RUNS);
    let signal_path = env_string_or_default("SIGNAL_PATH", DEFAULT_SIGNAL_PATH);
    let phases: Vec<String> = env_string_or_default("PHASES", DEFAULT_PHASES)
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let output_path = PathBuf::from(env_string_or_default("OUTPUT_CSV", DEFAULT_OUTPUT_CSV));
    let raw_output_path = PathBuf::from(env_string_or_default(
        "OUTPUT_RAW_CSV",
        DEFAULT_OUTPUT_RAW_CSV,
    ));
    let memory_output_path = PathBuf::from(env_string_or_default(
        "OUTPUT_MEMORY_CSV",
        DEFAULT_OUTPUT_MEMORY_CSV,
    ));

    ensure_parent_dir(&output_path)?;
    let mut writer = BufWriter::new(File::create(&output_path)?);
    write_header(&mut writer)?;

    ensure_parent_dir(&raw_output_path)?;
    let mut raw_writer = BufWriter::new(File::create(&raw_output_path)?);
    write_raw_header(&mut raw_writer)?;

    let mut memory_writer = if memory_runs > 0 {
        ensure_parent_dir(&memory_output_path)?;
        let mut w = BufWriter::new(File::create(&memory_output_path)?);
        write_memory_header(&mut w)?;
        Some(w)
    } else {
        None
    };

    if memory_runs > 0 {
        println!(
            "M = {m_runs} timed runs (+ {warmup_runs} warmup), \
             {memory_runs} untimed memory passes (per-step median)"
        );
    } else {
        println!("M = {m_runs} timed runs (+ {warmup_runs} warmup), no memory passes");
    }
    #[cfg(feature = "track-cache-size")]
    println!("cache sizes are sampled inside the timed loop -- timings are not comparable");

    let signal = read_signal(&signal_path, &phases)?;
    let scope = if phases.is_empty() {
        "all phases".to_string()
    } else {
        phases.join(", ")
    };
    println!("\n{} samples from {signal_path} ({scope})", signal.len());

    let mut pooled: Vec<f64> = Vec::new();

    for (spec_name, spec) in FORMULAS {
        let (result, run_times, memory) =
            bench_formula(spec_name, spec, &signal, m_runs, warmup_runs, memory_runs);

        let memory_note = match (result.avg_total_size_bytes, result.max_total_size_bytes) {
            (Some(avg), Some(max)) => format!(", memory avg {avg:.0} B, max {max} B"),
            _ => String::new(),
        };
        #[cfg(not(feature = "track-cache-size"))]
        let cache_note = String::new();
        #[cfg(feature = "track-cache-size")]
        let cache_note = format!(
            ", cache avg {:.2} steps, max {} steps",
            result.avg_cache_size, result.max_cache_size
        );
        println!(
            "  {spec_name:8} {:8.3} +- {:.3} us per update{memory_note}{cache_note}",
            result.avg_per_sample_s * 1e6,
            result.std_per_sample_s * 1e6,
        );

        write_row(&mut writer, &result)?;
        for (run_id, &total) in run_times.iter().enumerate() {
            write_raw_row(&mut raw_writer, &result, run_id, total)?;
        }
        if let Some(w) = memory_writer.as_mut() {
            write_memory_rows(w, spec_name, &memory)?;
        }

        pooled.extend(run_times.iter().map(|t| t / result.n_samples as f64 * 1e6));
    }

    let (mean, sd) = mean_sd(&pooled);
    println!(
        "  {:8} {mean:8.3} +- {sd:.3} us per update ({} runs)",
        "aggregate",
        pooled.len()
    );

    writer.flush()?;
    raw_writer.flush()?;
    println!("\nResults saved to {}", output_path.display());
    println!("Raw results saved to {}", raw_output_path.display());
    if let Some(w) = memory_writer.as_mut() {
        w.flush()?;
        println!("Memory series saved to {}", memory_output_path.display());
    }
    Ok(())
}
