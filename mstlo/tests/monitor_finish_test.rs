//! `StlMonitor::finish`: ending a trace reads every signal as holding its
//! last value forever, which settles every pending verdict.
//!
//! The reference is a brute-force robustness oracle on a gapless unit grid, where the
//! pointwise and dense-time readings of ZOH coincide.

mod common;

use common::initial_values;
use mstlo::monitor::{DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor};
use mstlo::{
    Algorithm, FormulaDefinition as Fd, RobustnessInterval, RobustnessSemantics, SemanticType,
    SignalInterpolation, Step, TimeInterval, stl,
};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::time::Duration;

fn secs(t: u64) -> Duration {
    Duration::from_secs(t)
}

fn interval(a: u64, b: u64) -> TimeInterval {
    TimeInterval {
        start: secs(a),
        end: secs(b),
    }
}

fn x() -> Fd {
    Fd::GreaterThan("x", 0.0)
}

fn y() -> Fd {
    Fd::LessThan("y", 0.0)
}

fn eventually(a: u64, b: u64, f: Fd) -> Fd {
    Fd::Eventually(interval(a, b), Box::new(f))
}

fn globally(a: u64, b: u64, f: Fd) -> Fd {
    Fd::Globally(interval(a, b), Box::new(f))
}

fn until(a: u64, b: u64, l: Fd, r: Fd) -> Fd {
    Fd::Until(interval(a, b), Box::new(l), Box::new(r))
}

fn formulas() -> Vec<Fd> {
    vec![
        x(),
        eventually(0, 5, x()),
        globally(0, 5, x()),
        eventually(2, 5, x()),
        globally(2, 4, x()),
        until(0, 5, x(), y()),
        until(1, 4, x(), y()),
        globally(0, 6, eventually(0, 2, x())),
        eventually(0, 3, globally(1, 2, y())),
        Fd::Not(Box::new(eventually(0, 4, x()))),
        Fd::And(
            Box::new(globally(0, 4, x())),
            Box::new(eventually(0, 2, y())),
        ),
        Fd::Or(
            Box::new(eventually(0, 6, x())),
            Box::new(globally(0, 1, y())),
        ),
        globally(
            0,
            3,
            Fd::Implies(Box::new(x()), Box::new(eventually(1, 2, y()))),
        ),
    ]
}

/// Deterministic traces over `x` and `y`, both sampled at every second from `t=0`, with
/// values that keep every robustness nonzero so qualitative verdicts are unambiguous.
fn traces() -> Vec<Vec<Step<f64>>> {
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..80)
        .map(|_| {
            let len = 1 + next() % 14;
            (0..len)
                .flat_map(|t| {
                    let values = [-2.0, -1.0, 1.0, 2.0];
                    let vx = values[(next() % 4) as usize];
                    let vy = values[(next() % 4) as usize];
                    [Step::new("x", vx, secs(t)), Step::new("y", vy, secs(t))]
                })
                .collect()
        })
        .collect()
}

/// Robustness at `t` of `formula` over `trace` held at its last values forever.
fn oracle(formula: &Fd, trace: &[Step<f64>], t: u64) -> f64 {
    let end = trace.last().unwrap().timestamp.as_secs();
    let value = |signal: &str, t: u64| {
        trace
            .iter()
            .rev()
            .find(|s| s.signal == signal && s.timestamp.as_secs() <= t.min(end))
            .unwrap()
            .value
    };
    let window = |i: &TimeInterval| t + i.start.as_secs()..=t + i.end.as_secs();
    match formula {
        Fd::GreaterThan(s, c) => value(s, t) - c,
        Fd::LessThan(s, c) => c - value(s, t),
        Fd::Not(f) => -oracle(f, trace, t),
        Fd::And(l, r) => oracle(l, trace, t).min(oracle(r, trace, t)),
        Fd::Or(l, r) => oracle(l, trace, t).max(oracle(r, trace, t)),
        Fd::Implies(l, r) => (-oracle(l, trace, t)).max(oracle(r, trace, t)),
        Fd::Eventually(i, f) => window(i)
            .map(|u| oracle(f, trace, u))
            .fold(f64::NEG_INFINITY, f64::max),
        Fd::Globally(i, f) => window(i)
            .map(|u| oracle(f, trace, u))
            .fold(f64::INFINITY, f64::min),
        // The closed obligation: `l` holds on [t, u], including the witness `u`.
        Fd::Until(i, l, r) => window(i)
            .map(|u| {
                (t..=u)
                    .map(|v| oracle(l, trace, v))
                    .fold(oracle(r, trace, u), f64::min)
            })
            .fold(f64::NEG_INFINITY, f64::max),
        _ => unimplemented!(),
    }
}

/// Every raw output of `trace` followed by `finish`, with the outputs of `finish` alone.
fn run<S, Y>(
    formula: &Fd,
    trace: &[Step<f64>],
    semantics: S,
    algorithm: Algorithm,
) -> (Vec<Step<Y>>, Vec<Step<Y>>)
where
    S: SemanticType<Output = Y>,
    Y: RobustnessSemantics + Copy + Debug + 'static,
{
    let mut monitor: StlMonitor<f64, Y> = StlMonitor::builder()
        .formula(formula.clone())
        .semantics(semantics)
        .algorithm(algorithm)
        .initialize_signals(initial_values(trace))
        .build()
        .unwrap();
    let mut outputs: Vec<Step<Y>> = trace
        .iter()
        .flat_map(|step| monitor.update(step).all_raw_outputs())
        .collect();
    let finished = monitor.finish().all_raw_outputs();
    outputs.extend(finished.iter().cloned());
    (outputs, finished)
}

/// The value of an eager breakpoint stream at every second up to `end`.
fn under_zoh(stream: &[Step<bool>], end: u64) -> Vec<Option<bool>> {
    (0..=end)
        .map(|t| {
            stream
                .iter()
                .rev()
                .find(|s| s.timestamp <= secs(t))
                .map(|s| s.value)
        })
        .collect()
}

#[test]
fn finish_settles_delayed_verdicts_like_the_zoh_extension() {
    for formula in formulas() {
        for trace in traces() {
            let end = trace.last().unwrap().timestamp.as_secs();
            let expected: Vec<(Duration, f64)> = (0..=end)
                .map(|t| (secs(t), oracle(&formula, &trace, t)))
                .collect();
            let (rho, _) = run(
                &formula,
                &trace,
                DelayedQuantitative,
                Algorithm::Incremental,
            );
            let rho: Vec<_> = rho.iter().map(|s| (s.timestamp, s.value)).collect();
            assert_eq!(rho, expected, "{formula} on {trace:?}");

            let (sat, _) = run(&formula, &trace, DelayedQualitative, Algorithm::Incremental);
            let sat: Vec<_> = sat.iter().map(|s| (s.timestamp, s.value)).collect();
            let expected_sat: Vec<_> = expected.iter().map(|&(t, r)| (t, r > 0.0)).collect();
            assert_eq!(sat, expected_sat, "{formula} on {trace:?}");
        }
    }
}

#[test]
fn finish_settles_eager_verdicts_like_the_zoh_extension() {
    for formula in formulas() {
        for trace in traces() {
            let end = trace.last().unwrap().timestamp.as_secs();
            let (stream, finished) =
                run(&formula, &trace, EagerQualitative, Algorithm::Incremental);
            let expected: Vec<_> = (0..=end)
                .map(|t| Some(oracle(&formula, &trace, t) > 0.0))
                .collect();
            assert_eq!(under_zoh(&stream, end), expected, "{formula} on {trace:?}");
            assert!(finished.iter().all(|s| s.timestamp <= secs(end)));
        }
    }
}

#[test]
fn finish_settles_rosi_intervals_to_the_zoh_extension() {
    for formula in formulas() {
        for trace in traces() {
            let end = trace.last().unwrap().timestamp.as_secs();
            let (outputs, finished) = run(&formula, &trace, Rosi, Algorithm::Incremental);
            let mut last: BTreeMap<Duration, RobustnessInterval> = BTreeMap::new();
            for step in &outputs {
                let RobustnessInterval(lo, hi) = step.value;
                // Every refinement encloses the final value.
                let rho = oracle(&formula, &trace, step.timestamp.as_secs());
                assert!(lo <= rho && rho <= hi, "{formula} on {trace:?}: {step:?}");
                last.insert(step.timestamp, step.value);
            }
            let expected: BTreeMap<_, _> = (0..=end)
                .map(|t| {
                    let rho = oracle(&formula, &trace, t);
                    (secs(t), RobustnessInterval(rho, rho))
                })
                .collect();
            assert_eq!(last, expected, "{formula} on {trace:?}");
            assert!(finished.iter().all(|s| s.timestamp <= secs(end)));
        }
    }
}

/// The verdicts of `output` as `(timestamp, value)` pairs.
fn settled<Y: Clone>(output: mstlo::MonitorOutput<f64, Y>) -> Vec<(Duration, Y)> {
    output
        .verdicts()
        .into_iter()
        .map(|step| (step.timestamp, step.value))
        .collect()
}

fn delayed_monitor(formula: Fd) -> StlMonitor<f64, bool> {
    StlMonitor::builder()
        .formula(formula)
        .semantics(DelayedQualitative)
        .build()
        .unwrap()
}

#[test]
fn input_after_finish_continues_past_the_held_values() {
    let mut monitor = delayed_monitor(stl!(G[0, 2](x > 0.0)));
    monitor.update(&Step::new("x", 1.0, secs(0)));
    assert_eq!(settled(monitor.finish()), vec![(secs(0), true)]);

    // x is held at 1.0 through 2s, so older input is ignored.
    assert!(
        !monitor
            .update(&Step::new("x", -1.0, secs(1)))
            .has_verdicts()
    );
    assert!(
        !monitor
            .update(&Step::new("x", -1.0, secs(2)))
            .has_verdicts()
    );

    // Newer input continues the trace: G[0,2] at 2s reads the held 1.0 and then -1.0.
    monitor.update(&Step::new("x", -1.0, secs(3)));
    monitor.update(&Step::new("x", -1.0, secs(4)));
    assert_eq!(
        settled(monitor.update(&Step::new("x", -1.0, secs(5)))),
        vec![(secs(3), false)]
    );

    monitor.reset();
    monitor.update(&Step::new("x", -1.0, secs(0)));
    assert_eq!(settled(monitor.finish()), vec![(secs(0), false)]);
}

#[test]
fn finishing_an_empty_trace_reports_nothing() {
    let mut monitor = delayed_monitor(stl!(F[0, 2](x > 0.0)));
    assert!(!monitor.finish().has_verdicts());
}

#[test]
fn finish_is_unsupported_under_naive() {
    let mut monitor: StlMonitor<f64, bool> = StlMonitor::builder()
        .formula(stl!(G[0, 2](x > 0.0)))
        .semantics(DelayedQualitative)
        .algorithm(Algorithm::Naive)
        .build()
        .unwrap();
    monitor.update(&Step::new("x", 1.0, secs(0)));
    assert!(!monitor.finish().has_verdicts());
    // The trace goes on.
    assert!(monitor.update(&Step::new("x", 1.0, secs(2))).has_verdicts());
}

#[test]
fn finish_holds_signals_that_stopped_early() {
    // y stops at 1s while x runs on to 3s, so y is held over [1s, 3s] and beyond.
    let mut monitor: StlMonitor<f64, bool> = StlMonitor::builder()
        .formula(stl!(G[0, 4]((x > 0.0) && (y < 0.0))))
        .semantics(DelayedQualitative)
        .initialize_signals([("x", 1.0), ("y", -1.0)])
        .build()
        .unwrap();
    monitor.update_batch(&[
        Step::new("x", 1.0, secs(0)),
        Step::new("y", -1.0, secs(0)),
        Step::new("y", -1.0, secs(1)),
        Step::new("x", 1.0, secs(2)),
        Step::new("x", 1.0, secs(3)),
    ]);
    let verdicts = monitor.finish().verdicts();
    let timestamps: Vec<_> = verdicts.iter().map(|s| s.timestamp).collect();
    assert_eq!(timestamps, vec![secs(0), secs(1), secs(2), secs(3)]);
    assert!(verdicts.iter().all(|s| s.value));
}

#[test]
fn finish_without_temporal_operators_adds_no_samples() {
    let mut monitor = delayed_monitor(stl!(x > 0.0));
    assert!(monitor.update(&Step::new("x", 1.0, secs(0))).has_verdicts());
    assert!(!monitor.finish().has_verdicts());
}

#[test]
fn finish_settles_linear_interpolation_with_a_flat_extension() {
    let mut monitor: StlMonitor<f64, bool> = StlMonitor::builder()
        .formula(stl!(F[0, 3](x > 2.0)))
        .semantics(DelayedQualitative)
        .signal_interpolation(SignalInterpolation::Linear)
        .build()
        .unwrap();
    monitor.update_batch(&[Step::new("x", 0.0, secs(0)), Step::new("x", 4.0, secs(4))]);
    let verdicts = monitor.finish().verdicts();
    // x crosses 2 at 2s, so F[0,3] holds everywhere on [0s, 4s], held at 4 after.
    assert!(!verdicts.is_empty());
    assert!(verdicts.iter().all(|s| s.value && s.timestamp <= secs(4)));
    assert_eq!(verdicts.last().unwrap().timestamp, secs(4));
}

#[test]
fn finish_reaches_the_last_sample_at_epoch_timestamps() {
    // Sums of float seconds lose nanoseconds at this magnitude; `finish` does not.
    let t = Duration::from_secs_f64(1.761_998_123_456_789e9);
    let mut monitor = delayed_monitor(stl!(G[0, 0.1](x > 0.0)));
    monitor.update(&Step::new("x", 1.0, t));
    assert_eq!(settled(monitor.finish()), vec![(t, true)]);
}

#[test]
fn eager_g_with_a_test_timeout_bound_reports_satisfaction_on_finish() {
    let mut monitor: StlMonitor<f64, bool> = StlMonitor::builder()
        .formula(stl!(G[0, 3600](x > 0.0)))
        .semantics(EagerQualitative)
        .build()
        .unwrap();
    for t in 0..10 {
        assert!(!monitor.update(&Step::new("x", 1.0, secs(t))).has_verdicts());
    }
    assert_eq!(settled(monitor.finish()).first(), Some(&(secs(0), true)));
}
