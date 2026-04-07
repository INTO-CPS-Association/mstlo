#[cfg(feature = "track-cache-size")]
use mstlo::ring_buffer::GLOBAL_CACHE_SIZE;
use mstlo::ring_buffer::Step;
use mstlo::stl::formula_definition::FormulaDefinition;
use mstlo::stl::monitor::{
    Algorithm, DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor,
};
use mstlo::stl::parse_stl;
use std::collections::HashSet;
use std::fs::{File, create_dir_all};
#[cfg(feature = "track-cache-size")]
use std::hint::black_box;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};
#[cfg(feature = "track-cache-size")]
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Configuration (env-overridable)
// ---------------------------------------------------------------------------
const DEFAULT_M_RUNS: usize = 50;
const DEFAULT_SIGNAL_PATH: &str = "benches/signal_generation/signals/signal_20000.csv";
const DEFAULT_OUTPUT_CSV: &str = "benches/results/paper_native_benchmark_results_N=20000.csv";

#[derive(Copy, Clone, PartialEq)]
enum SemanticsKind {
    DelayedQuantitative,
    DelayedQualitative,
    EagerQualitative,
    Rosi,
}

impl SemanticsKind {
    fn name(self) -> &'static str {
        match self {
            Self::DelayedQuantitative => "DelayedQuantitative",
            Self::DelayedQualitative => "DelayedQualitative",
            Self::EagerQualitative => "EagerQualitative",
            Self::Rosi => "Rosi",
        }
    }
}

#[derive(Clone)]
struct BenchItem {
    formula_id: usize,
    spec: String,
    benchmark_kind: &'static str,
    interval_len: usize,
    formula: FormulaDefinition,
}

struct BenchResult {
    formula_id: usize,
    spec: String,
    semantics: &'static str,
    algorithm: &'static str,
    mode: &'static str,
    n_samples: usize,
    m_runs: usize,
    avg_total_s: f64,
    avg_per_sample_s: f64,
    avg_per_sample_us: f64,
    #[cfg(feature = "track-cache-size")]
    avg_cache_size: f64,
    #[cfg(feature = "track-cache-size")]
    max_cache_size: usize,
    benchmark_kind: &'static str,
    interval_len: usize,
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

fn parse_formula_ids_from_env() -> io::Result<Option<HashSet<usize>>> {
    let Ok(raw) = std::env::var("FORMULA_IDS") else {
        return Ok(None);
    };

    let mut ids = HashSet::new();

    for token in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let id = token.parse::<usize>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Invalid FORMULA_IDS entry '{token}'. Expected a comma-separated list of positive integers, e.g. '1,2,3'"
                ),
            )
        })?;

        if id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid FORMULA_IDS entry '0'. Formula IDs start at 1.",
            ));
        }

        ids.insert(id);
    }

    if ids.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FORMULA_IDS was set but no valid IDs were provided.",
        ));
    }

    Ok(Some(ids))
}

fn read_signal_from_csv<P>(filename: P) -> io::Result<Vec<Step<f64>>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    let reader = io::BufReader::new(file);
    let mut signal = Vec::new();

    for line in reader.lines().skip(1) {
        let value_str = line?;
        let columns: Vec<&str> = value_str.split(',').collect();
        if columns.len() == 2
            && let (Ok(ts), Ok(val)) = (
                columns[0].trim().parse::<f64>(),
                columns[1].trim().parse::<f64>(),
            )
        {
            let t = Duration::from_secs_f64(ts);
            signal.push(Step::new("x", val, t));
        }
    }

    Ok(signal)
}

fn bench_item(
    item: &BenchItem,
    signal: Vec<Step<f64>>,
    m_runs: usize,
    semantics: SemanticsKind,
) -> Option<BenchResult> {
    if semantics == SemanticsKind::Rosi && item.interval_len > 1000 {
        // Skip very long intervals for Rosi semantics to avoid excessive runtime
        return None;
    }
    let n_samples = signal.len();
    let mut total_time = 0.0;

    #[cfg(feature = "track-cache-size")]
    let mut total_cache_size_all_runs: u128 = 0;
    #[cfg(feature = "track-cache-size")]
    let mut max_cache_size_all_runs: usize = 0;

    match semantics {
        SemanticsKind::DelayedQuantitative => {
            for _ in 0..m_runs {
                #[cfg(feature = "track-cache-size")]
                GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
                let mut monitor: StlMonitor<f64, f64> = StlMonitor::builder()
                    .formula(item.formula.clone())
                    .algorithm(Algorithm::Incremental)
                    .semantics(DelayedQuantitative)
                    .build()
                    .unwrap();

                #[cfg(not(feature = "track-cache-size"))]
                {
                    let t0 = Instant::now();
                    for step in &signal {
                        monitor.update(step);
                    }
                    total_time += t0.elapsed().as_secs_f64();
                }

                #[cfg(feature = "track-cache-size")]
                {
                    let mut total_cache_size = 0usize;
                    let mut max_cache_size = 0usize;
                    let t0 = Instant::now();
                    for step in &signal {
                        black_box(monitor.update(step));
                        let current_size = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                        total_cache_size += current_size;
                        max_cache_size = max_cache_size.max(current_size);
                    }

                    total_time += t0.elapsed().as_secs_f64();

                    let avg_size = total_cache_size as f64 / signal.len() as f64;
                    total_cache_size_all_runs += total_cache_size as u128;
                    max_cache_size_all_runs = max_cache_size_all_runs.max(max_cache_size);
                    println!(
                        "  Formula ID {}: Avg Size: {:.2}, Max Size: {}",
                        item.formula_id, avg_size, max_cache_size
                    );
                }

                drop(monitor);
                #[cfg(feature = "track-cache-size")]
                {
                    let residual = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                    if residual != 0 {
                        eprintln!(
                            "  Warning: residual GLOBAL_CACHE_SIZE={} after run for formula {}",
                            residual, item.formula_id
                        );
                        GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
                    }
                }
            }
        }
        SemanticsKind::DelayedQualitative => {
            for _ in 0..m_runs {
                #[cfg(feature = "track-cache-size")]
                GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
                let mut monitor: StlMonitor<f64, bool> = StlMonitor::builder()
                    .formula(item.formula.clone())
                    .algorithm(Algorithm::Incremental)
                    .semantics(DelayedQualitative)
                    .build()
                    .unwrap();

                #[cfg(not(feature = "track-cache-size"))]
                {
                    let t0 = Instant::now();
                    for step in &signal {
                        monitor.update(step);
                    }
                    total_time += t0.elapsed().as_secs_f64();
                }

                #[cfg(feature = "track-cache-size")]
                {
                    let mut total_cache_size = 0usize;
                    let mut max_cache_size = 0usize;
                    let t0 = Instant::now();
                    for step in &signal {
                        monitor.update(step);
                        let current_size = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                        total_cache_size += current_size;
                        max_cache_size = max_cache_size.max(current_size);
                    }

                    total_time += t0.elapsed().as_secs_f64();

                    let avg_size = total_cache_size as f64 / signal.len() as f64;
                    total_cache_size_all_runs += total_cache_size as u128;
                    max_cache_size_all_runs = max_cache_size_all_runs.max(max_cache_size);
                    println!(
                        "  Formula ID {}: Avg Size: {:.2}, Max Size: {}",
                        item.formula_id, avg_size, max_cache_size
                    );
                }

                drop(monitor);
                #[cfg(feature = "track-cache-size")]
                {
                    let residual = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                    if residual != 0 {
                        eprintln!(
                            "  Warning: residual GLOBAL_CACHE_SIZE={} after run for formula {}",
                            residual, item.formula_id
                        );
                        GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
                    }
                }
            }
        }
        SemanticsKind::EagerQualitative => {
            for _ in 0..m_runs {
                #[cfg(feature = "track-cache-size")]
                GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
                let mut monitor: StlMonitor<f64, bool> = StlMonitor::builder()
                    .formula(item.formula.clone())
                    .algorithm(Algorithm::Incremental)
                    .semantics(EagerQualitative)
                    .build()
                    .unwrap();
                #[cfg(not(feature = "track-cache-size"))]
                {
                    let t0 = Instant::now();
                    for step in &signal {
                        monitor.update(step);
                    }
                    total_time += t0.elapsed().as_secs_f64();
                }

                #[cfg(feature = "track-cache-size")]
                {
                    let mut total_cache_size = 0usize;
                    let mut max_cache_size = 0usize;
                    let t0 = Instant::now();
                    for step in &signal {
                        monitor.update(step);
                        let current_size = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                        total_cache_size += current_size;
                        max_cache_size = max_cache_size.max(current_size);
                    }
                    total_time += t0.elapsed().as_secs_f64();

                    let avg_size = total_cache_size as f64 / signal.len() as f64;
                    total_cache_size_all_runs += total_cache_size as u128;
                    max_cache_size_all_runs = max_cache_size_all_runs.max(max_cache_size);
                    println!(
                        "  Formula ID {}: Avg Size: {:.2}, Max Size: {}",
                        item.formula_id, avg_size, max_cache_size
                    );
                }

                drop(monitor);
                #[cfg(feature = "track-cache-size")]
                {
                    let residual = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                    if residual != 0 {
                        eprintln!(
                            "  Warning: residual GLOBAL_CACHE_SIZE={} after run for formula {}",
                            residual, item.formula_id
                        );
                        GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
                    }
                }
            }
        }
        SemanticsKind::Rosi => {
            for _ in 0..m_runs {
                #[cfg(feature = "track-cache-size")]
                GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
                let mut monitor = StlMonitor::builder()
                    .formula(item.formula.clone())
                    .algorithm(Algorithm::Incremental)
                    .semantics(Rosi)
                    .build()
                    .unwrap();
                #[cfg(not(feature = "track-cache-size"))]
                {
                    let t0 = Instant::now();
                    for step in &signal {
                        monitor.update(step);
                    }
                    total_time += t0.elapsed().as_secs_f64();
                }
                #[cfg(feature = "track-cache-size")]
                {
                    let mut total_cache_size = 0usize;
                    let mut max_cache_size = 0usize;
                    let t0 = Instant::now();
                    for step in &signal {
                        monitor.update(step);
                        let current_size = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                        total_cache_size += current_size;
                        max_cache_size = max_cache_size.max(current_size);
                    }
                    total_time += t0.elapsed().as_secs_f64();

                    let avg_size = total_cache_size as f64 / signal.len() as f64;
                    total_cache_size_all_runs += total_cache_size as u128;
                    max_cache_size_all_runs = max_cache_size_all_runs.max(max_cache_size);
                    println!(
                        "  Formula ID {}: Avg Size: {:.2}, Max Size: {}",
                        item.formula_id, avg_size, max_cache_size
                    );
                }

                drop(monitor);
                #[cfg(feature = "track-cache-size")]
                {
                    let residual = GLOBAL_CACHE_SIZE.load(Ordering::Relaxed);
                    if residual != 0 {
                        eprintln!(
                            "  Warning: residual GLOBAL_CACHE_SIZE={} after run for formula {}",
                            residual, item.formula_id
                        );
                        GLOBAL_CACHE_SIZE.store(0, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    let avg_total = total_time / m_runs as f64;
    let avg_per_sample = avg_total / n_samples as f64;
    #[cfg(feature = "track-cache-size")]
    let avg_cache_size = total_cache_size_all_runs as f64 / (n_samples as f64 * m_runs as f64);

    Some(BenchResult {
        formula_id: item.formula_id,
        spec: item.spec.clone(),
        semantics: semantics.name(),
        algorithm: "Incremental",
        mode: "online",
        n_samples,
        m_runs,
        avg_total_s: avg_total,
        avg_per_sample_s: avg_per_sample,
        avg_per_sample_us: avg_per_sample * 1e6,
        #[cfg(feature = "track-cache-size")]
        avg_cache_size,
        #[cfg(feature = "track-cache-size")]
        max_cache_size: max_cache_size_all_runs,
        benchmark_kind: item.benchmark_kind,
        interval_len: item.interval_len,
    })
}

/// Builds a map of all formulas to test (formula_id -> spec string)
/// Matches the Python benchmark catalog (IDs 1-21)
fn build_formulas_map() -> Vec<(usize, String)> {
    let mut formulas = Vec::new();
    let mut formula_id = 1;

    // phi1
    formulas.push((formula_id, "(x < 0.5) and (x > -0.5)".to_string()));
    formula_id += 1;

    // phi2
    formulas.push((
        formula_id,
        "G[0,1000] (x > 0.5 -> F[0,100] (x < 0.0))".to_string(),
    ));
    formula_id += 1;

    // phi3
    formulas.push((
        formula_id,
        "(G[0,100] (x < 0.5)) or (G[100,150] (x > 0.0))".to_string(),
    ));
    formula_id += 1;

    // Generate intervals: [1, 100, 200, ..., 5000]
    let mut b: Vec<usize> = (0..=50).map(|i| i * 100).collect();
    b[0] = 1; // avoid zero bound for the first formula

    // Until formulas ID 4
    for bnd in &b {
        formulas.push((formula_id, format!("(x < 0.0) U[0,{bnd}] (x > 0.0)")));
    }
    formula_id += 1;

    // Globally formulas ID 5
    for bnd in &b {
        formulas.push((formula_id, format!("G[0,{bnd}] (x > 0.0)")));
    }
    formula_id += 1;

    // Eventually formulas ID 6
    for bnd in &b {
        formulas.push((formula_id, format!("F[0,{bnd}] (x > 0.0)")));
    }

    formulas
}

fn extract_interval_length(spec: &str) -> usize {
    // Try to find pattern like [0,XXX] or [0,XXX.X]
    if let Some(start) = spec.find("[0,")
        && let Some(end) = spec[start..].find(']')
    {
        let interval_str = &spec[start + 3..start + end];
        if let Ok(interval) = interval_str.parse::<f64>() {
            return interval.ceil() as usize;
        }
    }

    0
}

fn build_bench_items(formulas: &Vec<(usize, String)>) -> Vec<BenchItem> {
    let mut items = Vec::new();

    for (formula_id, spec) in formulas {
        let formula =
            parse_stl(spec).unwrap_or_else(|e| panic!("Invalid formula '{}': {}", spec, e));

        let interval_len = extract_interval_length(spec);

        items.push(BenchItem {
            formula_id: *formula_id,
            spec: spec.clone(),
            benchmark_kind: "general",
            interval_len,
            formula,
        });
    }

    items
}

fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn write_csv_header(w: &mut BufWriter<File>) -> io::Result<()> {
    #[cfg(feature = "track-cache-size")]
    {
        writeln!(
            w,
            "formula_id,spec,semantics,algorithm,mode,n_samples,m_runs,avg_total_s,avg_per_sample_s,avg_per_sample_us,avg_cache_size,max_cache_size,benchmark_kind,interval_len"
        )
    }
    #[cfg(not(feature = "track-cache-size"))]
    {
        writeln!(
            w,
            "formula_id,spec,semantics,algorithm,mode,n_samples,m_runs,avg_total_s,avg_per_sample_s,avg_per_sample_us,benchmark_kind,interval_len"
        )
    }
}

fn write_result_row(w: &mut BufWriter<File>, r: &BenchResult) -> io::Result<()> {
    #[cfg(feature = "track-cache-size")]
    {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{:.12},{:.12},{:.6},{:.6},{},{},{}",
            r.formula_id,
            csv_escape(&r.spec),
            r.semantics,
            r.algorithm,
            r.mode,
            r.n_samples,
            r.m_runs,
            r.avg_total_s,
            r.avg_per_sample_s,
            r.avg_per_sample_us,
            r.avg_cache_size,
            r.max_cache_size,
            r.benchmark_kind,
            r.interval_len
        )
    }
    #[cfg(not(feature = "track-cache-size"))]
    {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{:.12},{:.12},{:.6},{},{}",
            r.formula_id,
            csv_escape(&r.spec),
            r.semantics,
            r.algorithm,
            r.mode,
            r.n_samples,
            r.m_runs,
            r.avg_total_s,
            r.avg_per_sample_s,
            r.avg_per_sample_us,
            r.benchmark_kind,
            r.interval_len
        )
    }
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
    let signal_path = env_string_or_default("SIGNAL_PATH", DEFAULT_SIGNAL_PATH);
    let output_path = PathBuf::from(env_string_or_default("OUTPUT_CSV", DEFAULT_OUTPUT_CSV));
    let selected_formula_ids = parse_formula_ids_from_env()?;

    let formulas = build_formulas_map();
    let formulas: Vec<(usize, String)> = match selected_formula_ids {
        Some(ids) => formulas
            .into_iter()
            .filter(|(formula_id, _)| ids.contains(formula_id))
            .collect(),
        None => formulas,
    };

    if formulas.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No formulas selected. Check FORMULA_IDS.",
        ));
    }

    let items = build_bench_items(&formulas);
    let signal = read_signal_from_csv(&signal_path)?;

    println!(
        "Loaded signal with {} samples from {}",
        signal.len(),
        signal_path
    );
    println!("Total formulas: {}", formulas.len());
    if std::env::var("FORMULA_IDS").is_ok() {
        println!("Using formula filter from FORMULA_IDS");
    }
    println!("Averaging over M = {} runs", m_runs);

    ensure_parent_dir(&output_path)?;
    let file = File::create(&output_path)?;
    let mut writer = BufWriter::new(file);
    write_csv_header(&mut writer)?;

    let semantics = [
        SemanticsKind::DelayedQuantitative,
        SemanticsKind::DelayedQualitative,
        SemanticsKind::EagerQualitative,
        SemanticsKind::Rosi,
    ];

    for sem in semantics {
        println!("\n--- Semantics: {} ---", sem.name());
        for item in &items {
            println!("formula_id={} spec={}", item.formula_id, item.spec);
            let result = bench_item(item, signal.clone(), m_runs, sem);
            if let Some(result) = result {
                write_result_row(&mut writer, &result)?;
            }
            writer.flush()?;
        }
    }

    println!("\nResults saved to {}", output_path.display());
    Ok(())
}
