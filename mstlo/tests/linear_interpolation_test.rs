//! Specification for `SignalInterpolation::Linear`.
//!
//! This suite is the contract for reading input signals as **piecewise linear** between
//! consecutive samples instead of holding them zero-order.
//!
//! For a predicate `p` and consecutive samples `(t₀, v₀)`, `(t₁, v₁)`:
//!
//! * If `p(v₀) == p(v₁)` the segment holds no crossing: emit only at `t₁`.
//! * Otherwise emit `p(v₁)` at `t_c = t₀ + (c − v₀)/(v₁ − v₀) · (t₁ − t₀)`, then again
//!   at `t₁`.
//!
use mstlo::monitor::{
    Algorithm, DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor,
};
use mstlo::{FormulaDefinition, SignalInterpolation, Step, step, stl};
use std::collections::BTreeMap;
use std::time::Duration;

/// Latest verdict per timestamp, which is what a consumer of the stream ends up holding.
fn run(
    formula: FormulaDefinition,
    interpolation: SignalInterpolation,
    trace: &[Step<f64>],
) -> BTreeMap<Duration, bool> {
    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(DelayedQualitative)
        .algorithm(Algorithm::Incremental)
        .signal_interpolation(interpolation)
        .build()
        .unwrap();

    let mut verdicts = BTreeMap::new();
    for input in trace {
        for verdict in monitor.update(input).verdicts() {
            verdicts.insert(verdict.timestamp, verdict.value);
        }
    }
    verdicts
}

fn linear(formula: FormulaDefinition, trace: &[Step<f64>]) -> BTreeMap<Duration, bool> {
    run(formula, SignalInterpolation::Linear, trace)
}

fn zoh(formula: FormulaDefinition, trace: &[Step<f64>]) -> BTreeMap<Duration, bool> {
    run(formula, SignalInterpolation::ZeroOrderHold, trace)
}

fn secs(t: u64) -> Duration {
    Duration::from_secs(t)
}

/// `[(second, value)]` sugar for single-signal traces.
fn x_trace(points: &[(u64, f64)]) -> Vec<Step<f64>> {
    points
        .iter()
        .map(|&(t, v)| step!("x", v, secs(t)))
        .collect()
}

fn expect(pairs: &[(u64, bool)]) -> BTreeMap<Duration, bool> {
    pairs.iter().map(|&(t, v)| (secs(t), v)).collect()
}

// ── A. Atomic exactness ───────────────────────────────────────────────────────

/// Falling crossing: `x` leaves `x > 4` at `t = 2s`.
#[test]
fn a1_falling_crossing_is_emitted_at_the_exact_time() {
    let verdicts = linear(stl! {x > 4}, &x_trace(&[(0, 6.0), (4, 2.0)]));
    assert_eq!(verdicts, expect(&[(0, true), (2, false), (4, false)]));
}

/// Rising crossing: `x` enters `x > 4` at `t = 2s`.
#[test]
fn a2_rising_crossing_is_emitted_at_the_exact_time() {
    let verdicts = linear(stl! {x > 4}, &x_trace(&[(0, 2.0), (4, 6.0)]));
    assert_eq!(verdicts, expect(&[(0, false), (2, true), (4, true)]));
}

/// No crossing: the segment stays inside the predicate, so no breakpoint is invented.
#[test]
fn a3_segment_without_a_crossing_emits_no_extra_breakpoint() {
    let verdicts = linear(stl! {x > 4}, &x_trace(&[(0, 5.0), (4, 6.0)]));
    assert_eq!(verdicts, expect(&[(0, true), (4, true)]));
}

/// `v₀` sits exactly on the threshold, so `t_c == t₀`.
///
/// `x = 4` at `0s` is not `> 4`, so `false@0s` is emitted on arrival. The segment then
/// rises, the crossing lands back on `0s`, and the half-open rule says the value on
/// `[0s, 4s)` is `p(v₁) = true`. The earlier verdict is refined rather than duplicated.
#[test]
fn a4_crossing_at_the_left_sample_refines_that_samples_verdict() {
    let verdicts = linear(stl! {x > 4}, &x_trace(&[(0, 4.0), (4, 8.0)]));
    assert_eq!(verdicts, expect(&[(0, true), (4, true)]));
}

/// Tangential touch: `x` reaches the threshold and retreats without ever exceeding it.
///
/// Comparing truth values rather than `v − c` handles this with no special case: both
/// segments have `p(v₀) == p(v₁) == false`, so neither reports a crossing.
#[test]
fn a5_tangential_touch_produces_no_crossing() {
    let verdicts = linear(stl! {x > 4}, &x_trace(&[(0, 2.0), (2, 4.0), (4, 2.0)]));
    assert_eq!(verdicts, expect(&[(0, false), (2, false), (4, false)]));
}

/// `LessThan` mirrors `GreaterThan` on the same trace.
#[test]
fn a6_less_than_mirrors_greater_than() {
    let verdicts = linear(stl! {x < 4}, &x_trace(&[(0, 6.0), (4, 2.0)]));
    assert_eq!(verdicts, expect(&[(0, false), (2, true), (4, true)]));
}

/// The same trace under `ZeroOrderHold` must keep today's behaviour exactly.
///
/// Without this, A1 could pass for the wrong reason: a crossing emitted unconditionally
/// rather than because `Linear` was selected.
#[test]
fn a7_zero_order_hold_invents_no_crossing() {
    let verdicts = zoh(stl! {x > 4}, &x_trace(&[(0, 6.0), (4, 2.0)]));
    assert_eq!(verdicts, expect(&[(0, true), (4, false)]));
}

// ── B. Formula level: where Linear and ZOH must disagree ──────────────────────

/// `x` crosses `x > 4` at `2s`, so `G[0,2](x > 4)` is false at `0s`: the window is
/// closed and `x = 4` at its right end, which is not `> 4`.
///
/// Under ZOH the predicate stays true until the `4s` sample, so the same window reads
/// true. This is the headline behavioural difference between the two interpolations.
#[test]
fn b1_globally_sees_the_crossing_inside_a_closed_window() {
    let trace = x_trace(&[(0, 6.0), (4, 2.0)]);

    assert_eq!(
        linear(stl! {G[0,2] (x > 4)}, &trace).get(&secs(0)),
        Some(&false),
        "x reaches 4 at t=2s and `>` is strict, so the closed window [0,2] fails"
    );
    assert_eq!(
        zoh(stl! {G[0,2] (x > 4)}, &trace).get(&secs(0)),
        Some(&true),
        "held zero-order, x is 6 across all of [0,2]"
    );
}

/// `F[0,1](x > 4)` also exercises the *shifted* breakpoints.
///
/// The crossing at `2s` enters the evaluation-timestamp set both directly and shifted by
/// the window bound, as `2s` and `1s`, which is what lets the verdict flip exactly at
/// `2s` rather than at the next sample.
#[test]
fn b2_eventually_answers_the_shifted_crossing_timestamps() {
    let trace = x_trace(&[(0, 6.0), (4, 2.0)]);

    assert_eq!(
        linear(stl! {F[0,1] (x > 4)}, &trace),
        expect(&[(0, true), (1, true), (2, false), (3, false)]),
        "true while [t, t+1] still overlaps [0s, 2s), false once t reaches the crossing"
    );

    let held = zoh(stl! {F[0,1] (x > 4)}, &trace);
    assert!(
        held.values().all(|&v| v),
        "held zero-order the predicate never falls inside the evaluated range; got {held:?}"
    );
}

/// Each operand's crossings must survive the union walk in `process_binary`.
///
/// `x` and `y` sit on deliberately disjoint grids and cross at different times -- `x` at
/// `2s`, `y` at `3s` -- so a crossing is never masked by the other operand happening to
/// have a sample there. Under ZOH neither `2s` nor `3s` is a breakpoint at all.
#[test]
fn b3_conjunction_walks_both_operands_crossings() {
    let trace = vec![
        step!("x", 6.0, secs(0)),
        step!("y", 6.0, secs(1)),
        step!("x", 2.0, secs(4)),
        step!("y", 2.0, secs(5)),
    ];
    let formula = stl! {(x > 4) && (y > 4)};

    let verdicts = linear(formula.clone(), &trace);
    assert_eq!(
        verdicts.get(&secs(1)),
        Some(&true),
        "both operands are above the threshold once y is known at 1s; got {verdicts:?}"
    );
    assert_eq!(
        verdicts.get(&secs(2)),
        Some(&false),
        "x's crossing at 2s must appear even though y has no sample there; got {verdicts:?}"
    );

    assert_eq!(
        zoh(formula, &trace).get(&secs(2)),
        None,
        "held zero-order, 2s is not a breakpoint of either operand"
    );
}

/// `Until` must find a witness that exists only between two samples of `ψ`.
///
/// `y` crosses `y > 4` at `2s`, inside the `[0,3]` window. Its next actual sample is at
/// `4s`, outside it, so under ZOH the window closes with no witness.
#[test]
fn b4_until_finds_a_witness_strictly_between_psi_samples() {
    let mut trace = vec![step!("y", 2.0, secs(0)), step!("y", 6.0, secs(4))];
    trace.extend((0..=8).map(|t| step!("x", 5.0, secs(t))));
    trace.sort_by_key(|s| s.timestamp);
    let formula = stl! {(x > 0) U[0,3] (y > 4)};

    assert_eq!(
        linear(formula.clone(), &trace).get(&secs(0)),
        Some(&true),
        "y crosses 4 at 2s, inside [0,3]"
    );
    assert_eq!(
        zoh(formula, &trace).get(&secs(0)),
        Some(&false),
        "held zero-order, y first exceeds 4 at 4s, outside [0,3]"
    );
}

/// A single-signal formula must actually be monitored linearly.
///
/// Synchronization was cross-signal and so pointless on a one-signal formula. Signal
/// interpolation is a property of each signal on its own, so it must not be downgraded
/// away on that basis: the request has to be honoured, and observably so.
#[test]
fn b5_single_signal_formula_is_not_downgraded() {
    let monitor = StlMonitor::builder()
        .formula(stl! {G[0,2] (x > 4)})
        .semantics(DelayedQualitative)
        .algorithm(Algorithm::Incremental)
        .signal_interpolation(SignalInterpolation::Linear)
        .build()
        .unwrap();

    assert_eq!(
        monitor.signal_interpolation(),
        SignalInterpolation::Linear,
        "one signal identifier must not silently downgrade the interpolation"
    );
    assert!(
        format!("{monitor}").contains("Linear"),
        "Display should report the interpolation in force"
    );
}

// ── C. Guard rails ────────────────────────────────────────────────────────────
//
// Predicate-layer crossings give the exact *qualitative* answer because the satisfaction
// signal is piecewise constant. The robustness signal is not: it is piecewise linear,
// and a window sup/inf over it is not recovered from the crossings. Rather than ship a
// quietly wrong number, the combination is refused at build time.

#[test]
fn c1_linear_is_rejected_for_delayed_quantitative() {
    let error = StlMonitor::builder()
        .formula(stl! {x > 4})
        .semantics(DelayedQuantitative)
        .algorithm(Algorithm::Incremental)
        .signal_interpolation(SignalInterpolation::Linear)
        .build()
        .err()
        .expect("quantitative robustness under Linear is not exact and must be refused");

    assert!(
        error.to_lowercase().contains("linear"),
        "the error should name the offending setting; got {error:?}"
    );
}

#[test]
fn c2_linear_is_rejected_for_robustness_interval() {
    StlMonitor::builder()
        .formula(stl! {x > 4})
        .semantics(Rosi)
        .algorithm(Algorithm::Incremental)
        .signal_interpolation(SignalInterpolation::Linear)
        .build()
        .err()
        .expect("RoSI carries quantitative bounds, so Linear must be refused");
}

#[test]
fn c3_linear_is_rejected_for_the_naive_algorithm() {
    StlMonitor::builder()
        .formula(stl! {x > 4})
        .semantics(DelayedQualitative)
        .algorithm(Algorithm::Naive)
        .signal_interpolation(SignalInterpolation::Linear)
        .build()
        .err()
        .expect("the naive backend does not route through Atomic, so it cannot cross");
}

#[test]
fn c4_linear_builds_for_both_qualitative_semantics() {
    StlMonitor::builder()
        .formula(stl! {x > 4})
        .semantics(DelayedQualitative)
        .algorithm(Algorithm::Incremental)
        .signal_interpolation(SignalInterpolation::Linear)
        .build()
        .expect("delayed qualitative is exact under Linear");

    StlMonitor::builder()
        .formula(stl! {x > 4})
        .semantics(EagerQualitative)
        .algorithm(Algorithm::Incremental)
        .signal_interpolation(SignalInterpolation::Linear)
        .build()
        .expect("eager qualitative is exact under Linear");
}
