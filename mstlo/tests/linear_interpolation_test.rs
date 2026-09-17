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
use mstlo::{
    FormulaDefinition, RobustnessSemantics, SemanticType, SignalInterpolation, Step, step, stl,
};
use rstest::rstest;
use std::collections::BTreeMap;
use std::fmt::Debug;
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

/// A crossing at `t_c = t₀ + (c − v₀)/(v₁ − v₀) · (t₁ − t₀)` is emitted exactly, and a segment
/// that does not change the predicate's truth emits no extra breakpoint.
#[rstest]
#[case::falling_crossing(stl! {x > 4}, &[(0, 6.0), (4, 2.0)], &[(0, true), (2, false), (4, false)])]
#[case::rising_crossing(stl! {x > 4}, &[(0, 2.0), (4, 6.0)], &[(0, false), (2, true), (4, true)])]
#[case::no_crossing(stl! {x > 4}, &[(0, 5.0), (4, 6.0)], &[(0, true), (4, true)])]
// `x = 4` is not `> 4`, but the rising segment makes `[0s, 4s)` true: the verdict at 0s is
// refined rather than duplicated.
#[case::crossing_at_left_sample(stl! {x > 4}, &[(0, 4.0), (4, 8.0)], &[(0, true), (4, true)])]
// Touching the threshold without exceeding it is no crossing.
#[case::tangential_touch(stl! {x > 4}, &[(0, 2.0), (2, 4.0), (4, 2.0)], &[(0, false), (2, false), (4, false)])]
#[case::less_than(stl! {x < 4}, &[(0, 6.0), (4, 2.0)], &[(0, false), (2, true), (4, true)])]
fn atomic_crossings(
    #[case] formula: FormulaDefinition,
    #[case] points: &[(u64, f64)],
    #[case] expected: &[(u64, bool)],
) {
    assert_eq!(linear(formula, &x_trace(points)), expect(expected));
}

/// Control: held zero-order, the falling trace has no crossing, so the cases above pass
/// because `Linear` was selected.
#[test]
fn zero_order_hold_invents_no_crossing() {
    let verdicts = zoh(stl! {x > 4}, &x_trace(&[(0, 6.0), (4, 2.0)]));
    assert_eq!(verdicts, expect(&[(0, true), (4, false)]));
}

// ── B. Formula level: where Linear and ZOH must disagree ──────────────────────

/// `x = 4` at 2s, the right end of the closed window `[0, 2]`, which is not `> 4`. Held
/// zero-order, `x = 6` across the whole window.
#[test]
fn globally_sees_the_crossing_inside_a_closed_window() {
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

/// The crossing at 2s is also queued shifted by the window bound, as 1s, so the verdict
/// flips exactly at 2s rather than at the next sample.
#[test]
fn eventually_answers_the_shifted_crossing_timestamps() {
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

/// Each operand's crossing survives the union walk: `x` crosses at 2s and `y` at 3s, on
/// disjoint sample grids. Held zero-order, neither is a breakpoint.
#[test]
fn conjunction_walks_both_operands_crossings() {
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

/// `y` crosses 4 at 2s, inside `[0, 3]`; its next sample, at 4s, is outside it.
#[test]
fn until_finds_a_witness_strictly_between_psi_samples() {
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

/// Interpolation is per signal, so a single-signal formula keeps `Linear`.
#[test]
fn single_signal_formula_is_not_downgraded() {
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
// Crossings make the satisfaction signal exact, but not the robustness signal, which is
// piecewise linear between them. Quantitative semantics are refused at build time.

fn build_linear<S, Y>(
    semantics: S,
    algorithm: Algorithm,
) -> Result<StlMonitor<f64, Y>, &'static str>
where
    S: SemanticType<Output = Y>,
    Y: RobustnessSemantics + Copy + Debug + PartialEq + 'static,
{
    StlMonitor::builder()
        .formula(stl! {x > 4})
        .semantics(semantics)
        .algorithm(algorithm)
        .signal_interpolation(SignalInterpolation::Linear)
        .build()
}

#[test]
fn linear_is_rejected_where_it_is_not_exact() {
    let error = build_linear(DelayedQuantitative, Algorithm::Incremental)
        .err()
        .expect("quantitative robustness under Linear is not exact and must be refused");
    assert!(
        error.to_lowercase().contains("linear"),
        "the error should name the offending setting; got {error:?}"
    );
    build_linear(Rosi, Algorithm::Incremental)
        .err()
        .expect("RoSI carries quantitative bounds, so Linear must be refused");
    build_linear(DelayedQualitative, Algorithm::Naive)
        .err()
        .expect("the naive backend does not route through Atomic, so it cannot cross");
}

#[test]
fn linear_builds_for_both_qualitative_semantics() {
    build_linear(DelayedQualitative, Algorithm::Incremental)
        .expect("delayed qualitative is exact under Linear");
    build_linear(EagerQualitative, Algorithm::Incremental)
        .expect("eager qualitative is exact under Linear");
}
