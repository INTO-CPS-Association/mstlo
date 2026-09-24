//! A/B regression benchmark for changes to the operator layer.
//!
//! This is deliberately *not* the paper benchmark. It exists to answer one
//! question quickly: "did the patch I just applied make the monitor slower, or
//! make it emit more?" It is self-contained -- the signal is generated in
//! process, so there is no CSV to produce first and both sides of the A/B are
//! guaranteed to see byte-identical input.
//!
//! # Running an A/B
//!
//! Criterion's own baselines, nothing else:
//!
//! ```text
//! cargo bench --bench ab_benchmark -- --save-baseline before
//! git apply finding_1_fix.patch finding_1_p1_fix.patch finding_1_optionc.patch
//! cargo bench --bench ab_benchmark -- --baseline before
//! ```
//!
//! The second run prints criterion's per-case timing delta, and then an output
//! volume table for the same two trees. Volume is tracked here because timing
//! alone hides it: a patch can leave every update as cheap as it was and still
//! double the number of verdicts a downstream consumer receives. The counts are
//! keyed by the same baseline name criterion was given, so they follow
//! `--save-baseline` / `--baseline` around without any extra bookkeeping.
//!
//! # Why these formulas
//!
//! Two groups, sampled at 1 Hz -- the rate `monitor_benchmark.rs` uses and the unit
//! the intervals are written in.
//!
//! **`catalog/NN`** is [`mstlo::get_formulas`] verbatim, the stable IDs, as the
//! regression guard: this is the existing benchmark set, and the fixes must not
//! make it appreciably slower.
//!
//! **Everything else** exists because the catalog *cannot see the fixes at all*.
//! Every interval in it is a whole number of seconds and every sample sits on a
//! whole second, so the shifted evaluation timestamps `ts - a` and `ts - b` land on
//! samples that were already going to be evaluated: the shift costs exactly nothing
//! there, and the nesting bug it fixes never fires either. Those cases therefore
//! come in aligned/off-grid pairs -- same shape, same window length, bounds moved
//! off the sample grid -- so the pair difference isolates what the shift costs.
//! `wide` variants scale the window instead of the shape, and the `control` and
//! `until` cases are untouched by the three patches, so movement there is collateral
//! damage rather than the price of a fix.
//!
//! # Knobs
//!
//! ```text
//! AB_N            samples in the generated signal   (default 2000)
//! AB_CATALOG      get_formulas IDs: list|all|none    (default 1-12)
//! AB_SEMANTICS    subset of bool,f64,eager,rosi     (default f64,eager,rosi)
//! AB_SAMPLE_SIZE  criterion samples per benchmark   (default 10)
//! AB_MEASURE_MS   criterion measurement time, ms    (default 1000)
//! ```
//!
//! The sample-count defaults are deliberately loose, tuned for a run you will wait
//! for rather than for publishable error bars; raise `AB_SAMPLE_SIZE` when a case
//! looks marginal. Keep every knob identical across the two runs, or the comparison
//! means nothing.
//! Criterion's own filter narrows the run further, e.g.
//! `cargo bench --bench ab_benchmark -- nest2 --baseline before`; the volume
//! table always covers every case, since counting is cheap.

use criterion::measurement::WallTime;
use criterion::{
    AxisScale, BatchSize, BenchmarkGroup, BenchmarkId, Criterion, PlotConfiguration, Throughput,
    criterion_group, criterion_main,
};
use mstlo::monitor::semantic_markers::SemanticType;
use mstlo::{
    Algorithm, DelayedQualitative, DelayedQuantitative, EagerQualitative, FormulaDefinition,
    RobustnessSemantics, Rosi, Step, StlMonitor, step, stl,
};
use std::fmt::Debug;
use std::hint::black_box;
use std::time::Duration;

/// Sampling period of the generated signal.
///
/// One second, because that is the unit the intervals in [`mstlo::get_formulas`] are
/// written in and the rate `monitor_benchmark.rs` feeds them at, so the catalog cases
/// below see the window sizes they were designed for. Every "aligned" bound is a whole
/// number of seconds and so lands on a sample; every "off-grid" bound does not.
const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------
// Signal
// ---------------------------------------------------------------------------

/// A linear chirp in `[-1, 1]`, sampled at [`SAMPLE_PERIOD`].
///
/// A chirp is used rather than a fixed sine so the atomic proposition flips at a
/// rate that varies across the run: the Lemire deque sees both long stretches
/// where one extremum dominates and stretches where every sample supersedes its
/// predecessor, which are the cheap and expensive paths through
/// `pop_dominated_values`.
fn generate_signal(n: usize) -> Vec<Step<f64>> {
    let h = SAMPLE_PERIOD.as_secs_f64();
    let duration = (n as f64) * h;
    let (f0, f1) = (0.005, 0.2);
    (0..n)
        .map(|i| {
            let t = i as f64 * h;
            let phase =
                std::f64::consts::TAU * (f0 * t + (f1 - f0) * t * t / (2.0 * duration.max(h)));
            step!("x", phase.sin(), SAMPLE_PERIOD * i as u32)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Formula catalog
// ---------------------------------------------------------------------------

/// `(name, formula)` pairs, ordered so aligned/off-grid pairs sit next to each
/// other in criterion's output.
fn cases() -> Vec<(&'static str, FormulaDefinition)> {
    vec![
        // No temporal operator at all. Nothing in the three patches touches this
        // path, so any movement here is measurement noise or a build difference.
        ("control/atomic", stl!(x > 0.0)),
        ("control/bool", stl!((x > 0.0) and (x < 0.5))),
        // Single temporal operator. The aligned/off-grid pair differs only in
        // whether the shifted evaluation timestamps coincide with samples.
        ("leaf/G/aligned", stl!(G[0, 10](x > 0.0))),
        ("leaf/G/offgrid", stl!(G[0.5, 10.5](x > 0.0))),
        ("leaf/F/aligned", stl!(F[0, 10](x > 0.0))),
        ("leaf/F/offgrid", stl!(F[0.5, 10.5](x > 0.0))),
        // Both bounds off-grid, but at *different* phases, so `ts - a` and `ts - b`
        // fall on two distinct lattices instead of one and cannot dedup with each
        // other. This is the worst case for the shift; `offgrid` above, where the two
        // bounds share a phase, is the common one.
        ("leaf/G/offgrid/phased", stl!(G[0.3, 10.7](x > 0.0))),
        // The same pair with a window an order of magnitude wider, to separate the
        // per-evaluation cost from the per-window cost.
        ("leaf/G/aligned/wide", stl!(G[0, 100](x > 0.0))),
        ("leaf/G/offgrid/wide", stl!(G[0.5, 100.5](x > 0.0))),
        // Window shorter than the sampling period, so most windows contain no
        // sample at all: the empty-window path.
        ("leaf/F/subsample", stl!(F[0, 0.5](x > 0.0))),
        // Nesting, where the operand's output lags the input frontier and the
        // shifted timestamps of one layer become breakpoints for the next.
        ("nest2/FG/aligned", stl!(F[0, 10](G[0, 10](x > 0.0)))),
        ("nest2/FG/offgrid", stl!(F[0, 10](G[0.5, 10.5](x > 0.0)))),
        ("nest2/GF/offgrid", stl!(G[0, 10](F[0.5, 10.5](x > 0.0)))),
        (
            "nest3/FGF/offgrid",
            stl!(F[0, 10](G[0.5, 10.5](F[0.5, 10.5](x > 0.0)))),
        ),
        (
            "nest2/FG/offgrid/wide",
            stl!(F[0, 100](G[0.5, 100.5](x > 0.0))),
        ),
        // Until is not modified by any of the three patches, but it consumes the
        // output of operands that are, so nesting one is a collateral check.
        ("until/leaf", stl!((x > 0.0) until[0, 10] (x < 0.5))),
        (
            "until/nested",
            stl!(F[0, 10]((x > 0.0) until[0.5, 10.5] (x < 0.5))),
        ),
    ]
}

/// The stable benchmark catalog, as the regression guard.
///
/// None of these can exhibit the bugs the patches fix -- whole-second intervals on
/// whole-second samples, so the shifted evaluation timestamps land on samples that
/// were already going to be evaluated -- which is exactly why they belong here: they
/// are what the fixes must not slow down. `AB_CATALOG` takes a comma-separated ID
/// list, `all` for all 21, or `none` to skip the group; the default is the basic
/// `1..=12`, since `13..=21` build trees of up to 2^5 operators and dominate the run.
fn catalog() -> Vec<(String, FormulaDefinition)> {
    let selection = std::env::var("AB_CATALOG").unwrap_or_else(|_| "1-12".to_string());
    let ids: Vec<usize> = match selection.trim() {
        "none" | "" => return Vec::new(),
        "all" => Vec::new(),
        "1-12" => (1..=12).collect(),
        list => list
            .split(',')
            .filter_map(|id| id.trim().parse().ok())
            .collect(),
    };
    mstlo::get_formulas(&ids)
        .into_iter()
        .map(|(id, formula)| (format!("catalog/{id:02}"), formula))
        .collect()
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn build<S>(formula: &FormulaDefinition, _marker: S) -> StlMonitor<f64, S::Output>
where
    S: SemanticType,
    S::Output: RobustnessSemantics + Copy + Debug + 'static,
{
    StlMonitor::builder()
        .formula(formula.clone())
        .algorithm(Algorithm::Incremental)
        .semantics(_marker)
        .build()
        .expect("monitor builds")
}

/// One untimed pass, reporting `(raw outputs emitted, final monitor size)`.
///
/// Output volume is the axis criterion cannot show: a patch that answers at more
/// timestamps can be no slower per update and still change what downstream code
/// receives.
fn profile<S>(formula: &FormulaDefinition, signal: &[Step<f64>], marker: S) -> (usize, usize)
where
    S: SemanticType,
    S::Output: RobustnessSemantics + Copy + Debug + 'static,
{
    let mut monitor = build(formula, marker);
    let outputs = signal
        .iter()
        .map(|s| monitor.update(s).total_raw_outputs())
        .sum();
    (outputs, monitor.total_size())
}

fn bench<S>(
    group: &mut BenchmarkGroup<'_, WallTime>,
    label: &str,
    formula: &FormulaDefinition,
    signal: &[Step<f64>],
    marker: S,
) where
    S: SemanticType + Copy,
    S::Output: RobustnessSemantics + Copy + Debug + 'static,
{
    group.bench_with_input(BenchmarkId::from_parameter(label), signal, |b, signal| {
        b.iter_batched_ref(
            || build(formula, marker),
            |monitor| {
                for step in signal {
                    black_box(monitor.update(step));
                }
            },
            BatchSize::LargeInput,
        );
    });
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A row read back from a recorded baseline: `(case, semantics, raw outputs)`.
type RecordedRow = (String, String, usize);

/// `(case, semantics, raw outputs emitted, final monitor bytes)`.
type VolumeRow = (String, &'static str, usize, usize);

fn benchmark_ab(c: &mut Criterion) {
    let n = env_usize("AB_N", 2000);
    let sample_size = env_usize("AB_SAMPLE_SIZE", 10).max(10);
    let measure_ms = env_usize("AB_MEASURE_MS", 1000) as u64;
    let selected = std::env::var("AB_SEMANTICS").unwrap_or_else(|_| "f64,eager,rosi".to_string());
    let wanted = |name: &str| selected.split(',').any(|s| s.trim() == name);

    let signal = generate_signal(n);
    let all: Vec<(String, FormulaDefinition)> = cases()
        .into_iter()
        .map(|(name, formula)| (name.to_string(), formula))
        .chain(catalog())
        .collect();

    let mut volume: Vec<VolumeRow> = Vec::new();

    for (name, formula) in &all {
        let mut group = c.benchmark_group(name);
        group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));
        group.sample_size(sample_size);
        group.measurement_time(Duration::from_millis(measure_ms));
        group.warm_up_time(Duration::from_millis(500));
        group.throughput(Throughput::Elements(signal.len() as u64));

        // The macro keeps the four semantics from becoming four copies of the
        // same block; each has a different `Y`, so a loop cannot express it.
        macro_rules! run {
            ($key:literal, $marker:expr) => {
                if wanted($key) {
                    let (outputs, bytes) = profile(formula, &signal, $marker);
                    volume.push((name.clone(), $key, outputs, bytes));
                    bench(&mut group, $key, formula, &signal, $marker);
                }
            };
        }
        run!("bool", DelayedQualitative);
        run!("f64", DelayedQuantitative);
        run!("eager", EagerQualitative);
        run!("rosi", Rosi);

        group.finish();
    }

    report_volume(&volume, n);
}

/// The baseline name criterion was given, and whether this run records or compares.
///
/// Criterion owns the `--save-baseline` / `--baseline` flags; reading them back out
/// of `argv` is what lets the volume counts travel with the timings under the same
/// name, so an A/B stays two plain `cargo bench` invocations. With neither flag,
/// criterion records under `base`, and so does this.
fn baseline() -> (String, bool) {
    let args: Vec<String> = std::env::args().collect();
    for (i, arg) in args.iter().enumerate() {
        let saving = match arg.as_str() {
            "--save-baseline" => true,
            "--baseline" | "--baseline-lenient" => false,
            _ => continue,
        };
        if let Some(name) = args.get(i + 1) {
            return (name.clone(), saving);
        }
    }
    ("base".to_string(), true)
}

/// Records this run's output volume, and prints it against the baseline's when
/// there is one to compare with.
fn report_volume(rows: &[VolumeRow], n: usize) {
    let (name, saving) = baseline();
    // `CARGO_TARGET_TMPDIR` is set by cargo for bench targets and points inside the
    // workspace target directory, so this lands somewhere already ignored by git no
    // matter which directory `cargo bench` was invoked from.
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ab_volume");
    let path = dir.join(format!("{name}.csv"));

    let previous = (!saving).then(|| read_volume(&path)).flatten();
    if let Some((recorded_n, _)) = &previous
        && *recorded_n != n
    {
        eprintln!(
            "\nWARNING: baseline \"{name}\" was recorded with AB_N={recorded_n}, this run used \
             AB_N={n}. The volume ratios below compare different signal lengths and mean nothing; \
             criterion's timings are per-run and unaffected."
        );
    }

    eprintln!();
    match &previous {
        None => {
            eprintln!("=== output volume, baseline \"{name}\" (N = {n} samples) ===");
            eprintln!(
                "{:<26} {:<6} {:>12} {:>12}",
                "case", "sem", "outputs", "bytes"
            );
            for (case, sem, outputs, bytes) in rows {
                eprintln!("{case:<26} {sem:<6} {outputs:>12} {bytes:>12}");
            }
            if !saving {
                eprintln!("\n(no recorded volume for baseline \"{name}\"; showing this run only)");
            }
        }
        Some((_, before)) => {
            eprintln!("=== output volume vs baseline \"{name}\" (N = {n} samples) ===");
            eprintln!(
                "{:<26} {:<6} {:>10} {:>10} {:>8}",
                "case", "sem", "before", "now", "ratio"
            );
            for (case, sem, outputs, _) in rows {
                let was = before
                    .iter()
                    .find(|(c, s, _)| c == case && s == sem)
                    .map(|(_, _, v)| *v);
                match was {
                    Some(was) if was > 0 => {
                        let ratio = *outputs as f64 / was as f64;
                        let mark = if (ratio - 1.0).abs() > 0.005 {
                            "  <--"
                        } else {
                            ""
                        };
                        eprintln!(
                            "{case:<26} {sem:<6} {was:>10} {outputs:>10} {ratio:>7.2}x{mark}"
                        );
                    }
                    _ => eprintln!("{case:<26} {sem:<6} {:>10} {outputs:>10}", "-"),
                }
            }
        }
    }

    // Written only when criterion itself is recording, so repeated `--baseline` runs
    // keep comparing against the same tree rather than against the previous one.
    if !saving {
        eprintln!();
        return;
    }
    let body: String = std::iter::once(format!("n,{n},,\n"))
        .chain(
            rows.iter()
                .map(|(case, sem, outputs, bytes)| format!("{case},{sem},{outputs},{bytes}\n")),
        )
        .collect();
    let written =
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, body.as_bytes()));
    match written {
        Ok(()) => eprintln!("\nrecorded volume for \"{name}\" in {}\n", path.display()),
        Err(e) => eprintln!("\ncould not write {}: {e}\n", path.display()),
    }
}

/// Reads back `(signal length, rows)`, where each row is `(case, semantics, outputs)`.
fn read_volume(path: &std::path::Path) -> Option<(usize, Vec<RecordedRow>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let n = lines
        .next()?
        .strip_prefix("n,")?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let rows = lines
        .filter_map(|line| {
            let mut f = line.split(',');
            Some((
                f.next()?.to_string(),
                f.next()?.to_string(),
                f.next()?.parse().ok()?,
            ))
        })
        .collect();
    Some((n, rows))
}

criterion_group!(benches, benchmark_ab);
criterion_main!(benches);
