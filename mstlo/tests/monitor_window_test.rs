//! Regression tests for `finding_1.md` — window evaluation in the temporal operators.
//!
//! Two problems, each checked on all four semantics:
//!
//! * **P1** — an empty window returns the operator's `identity()` (`false` / `-inf` /
//!   `(-inf,-inf)` for `Eventually`) instead of the value the signal actually holds there.
//! * **P2** — a window is declared closed against the *input* step timestamp while the
//!   operand's output still lags behind it, so the verdict is finalized too early.
//!
//! Each test pins down two things: the *value* of the verdict, and *which input step* the
//! monitor was fed when it produced it. The second half matters as much as the first — a
//! verdict that is right but emitted one step too early is exactly the P2 bug, and for P2 the
//! premature and the correct emission carry the same timestamp (1s), so only the step tells
//! them apart.
//!
//! All eight are expected to FAIL on the current implementation, except `Rosi` in the P2 case,
//! which is already correct.

use mstlo::monitor::{DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor};
use mstlo::{FormulaDefinition, RobustnessInterval, RobustnessSemantics, Step};
use mstlo::{step, stl};
use pretty_assertions::assert_eq;
use std::fmt::Debug;
use std::time::Duration;
use std::vec;

/// Feeds `signal` to a monitor and returns the last verdict emitted for timestamp `at`,
/// together with the input step that was being processed when it was emitted.
///
/// The *last* one is what matters: `Rosi` re-emits refinements for a timestamp until the
/// window closes, so only its final emission is the verdict, and the step it lands on is the
/// step at which the verdict becomes final. The other three semantics emit a timestamp once,
/// so for them "last" and "only" coincide.
fn last_verdict_at<Y, S>(
    formula: FormulaDefinition,
    signal: Vec<Step<f64>>,
    semantics: S,
    at: Duration,
) -> Option<(Step<f64>, Y)>
where
    Y: RobustnessSemantics + 'static + Copy + Debug + PartialEq,
    S: mstlo::monitor::semantic_markers::SemanticType<Output = Y> + Copy,
{
    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(semantics)
        .build()
        .unwrap();

    let mut verdict = None;
    for step in signal {
        for out in monitor.update(&step).all_raw_outputs() {
            if out.timestamp == at {
                verdict = Some((step.clone(), out.value));
            }
        }
    }
    verdict
}

// -----------------------------------------------------------------------------
// P1: the window `[t+0.98, t+0.99]` never contains a sample
// -----------------------------------------------------------------------------

/// `x` is above the threshold at every sample, so `x > 3` is the constant-true signal and
/// `F[0.98, 0.99](x > 3)` holds everywhere. But no sample ever lands inside a window, so
/// every window is empty.
fn out_of_phase_signal() -> Vec<Step<f64>> {
    vec![
        step!("x", 7.0, Duration::from_secs(0)),
        step!("x", 4.0, Duration::from_secs(1)),
        step!("x", 7.0, Duration::from_secs(2)),
        step!("x", 4.0, Duration::from_secs(3)),
    ]
}

// mstlo uses DENSE (zero-order-hold) semantics: a subformula holds its value between the
// samples of its operand, exactly as `SynchronizationStrategy::ZeroOrderHold` does for raw
// signals. So at t=0 the window [0.98, 0.99] lies inside the segment starting at the sample
// at 0s, and the robustness of `x > 3` there is 7 - 3 = 4.
//
// The current implementation instead returns `Eventually`'s identity for an empty window,
// reporting a definite violation of a formula that is satisfied.
//
// EMISSION STEP. The verdict for t=0 arrives on `x @1s`, for every semantics including the
// eager one. At `x @0s` the monitor knows the signal only up to 0s: the sample at 0s holds
// its value forward, but nothing yet rules out a further sample at, say, 0.5s that would
// change it. The window [0.98, 0.99] is first covered by *confirmed* signal when the sample
// at 1s arrives and closes the segment [0s, 1s). Eager cannot short-circuit before that,
// because it has no in-window value to short-circuit on.

#[test]
fn p1_empty_window_delayed_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0.98, 0.99](x > 3.0)),
        out_of_phase_signal(),
        DelayedQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 4.0, Duration::from_secs(1)), true))
    );
}

#[test]
fn p1_empty_window_delayed_quantitative() {
    let verdict = last_verdict_at(
        stl!(F[0.98, 0.99](x > 3.0)),
        out_of_phase_signal(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 4.0, Duration::from_secs(1)), 4.0))
    );
}

#[test]
fn p1_empty_window_eager_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0.98, 0.99](x > 3.0)),
        out_of_phase_signal(),
        EagerQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 4.0, Duration::from_secs(1)), true))
    );
}

#[test]
fn p1_empty_window_rosi() {
    let verdict = last_verdict_at(
        stl!(F[0.98, 0.99](x > 3.0)),
        out_of_phase_signal(),
        Rosi,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((
            step!("x", 4.0, Duration::from_secs(1)),
            RobustnessInterval(4.0, 4.0)
        ))
    );
}

// -----------------------------------------------------------------------------
// P2: the window closes before the operand has produced its value
// -----------------------------------------------------------------------------

/// Both signals cross the threshold at 1s, so `(x > 3) && (y > 3)` is
/// `false` at 0s and `true` at 1s, and `F[0, 1]` at t=0 (window `[0, 1]`) is therefore
/// `true` with robustness `min(4 - 3, 4 - 3) = 1`.
///
/// The steps arrive `x` before `y` at each timestamp. When `x @1s` arrives the conjunction
/// cannot be computed yet — `y @1s` is still missing — but the input clock has already
/// reached 1s, which is enough for `F[0, 1]` to consider the window at t=0 closed.
fn two_signals_x_then_y() -> Vec<Step<f64>> {
    vec![
        step!("x", 1.0, Duration::from_secs(0)),
        step!("y", 1.0, Duration::from_secs(0)),
        step!("x", 4.0, Duration::from_secs(1)),
        step!("y", 4.0, Duration::from_secs(1)),
    ]
}

// EMISSION STEP. The verdict for t=0 must arrive on `y @1s` — the step that completes the
// conjunction at 1s — and not on `x @1s`, which is where the current implementation emits it.
// Both steps carry the timestamp 1s, so the emission step, not the emission time, is what
// distinguishes the bug from the fix.

#[test]
fn p2_premature_close_delayed_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1]((x > 3.0) && (y > 3.0))),
        two_signals_x_then_y(),
        DelayedQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("y", 4.0, Duration::from_secs(1)), true))
    );
}

#[test]
fn p2_premature_close_delayed_quantitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1]((x > 3.0) && (y > 3.0))),
        two_signals_x_then_y(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("y", 4.0, Duration::from_secs(1)), 1.0))
    );
}

#[test]
fn p2_premature_close_eager_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1]((x > 3.0) && (y > 3.0))),
        two_signals_x_then_y(),
        EagerQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("y", 4.0, Duration::from_secs(1)), true))
    );
}

/// Already passes: `Rosi` closes the window against the operand's output frontier
/// (`unary_temporal_operators.rs:312`) rather than the input step timestamp.
#[test]
fn p2_premature_close_rosi() {
    let verdict = last_verdict_at(
        stl!(F[0, 1]((x > 3.0) && (y > 3.0))),
        two_signals_x_then_y(),
        Rosi,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((
            step!("y", 4.0, Duration::from_secs(1)),
            RobustnessInterval(1.0, 1.0)
        ))
    );
}

// -----------------------------------------------------------------------------
// P3: punctual intervals, `F[a,a]` and `G[a,a]`
// -----------------------------------------------------------------------------

/// `x > 3` is true on `[0s, 1s)`, false on `[1s, 2s)`, true from 2s.
fn punctual_signal() -> Vec<Step<f64>> {
    vec![
        step!("x", 7.0, Duration::from_secs(0)),
        step!("x", 1.0, Duration::from_secs(1)),
        step!("x", 7.0, Duration::from_secs(2)),
    ]
}

// A punctual window is the single point `t + a`, so `F[a,a] φ` and `G[a,a] φ` are both just
// `φ(t + a)` — they must return the *same* verdict. Under P1 they return opposite ones: the
// window contains no sample, so `F` collapses to `false` and `G` to `true`.
//
// Here `t + a = 0.5s`, which lies inside the segment starting at the sample at 0s, so both
// must report `true` with robustness 7 - 3 = 4, emitted on `x @1s` — the step that confirms
// the segment [0s, 1s) and therefore the value at 0.5s.

#[test]
fn p3_punctual_eventually_delayed_qualitative() {
    let verdict0 = last_verdict_at(
        stl!(F[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        DelayedQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict0,
        Some((step!("x", 1.0, Duration::from_secs(1)), true))
    );
    let verdict1 = last_verdict_at(
        stl!(F[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        DelayedQualitative,
        Duration::from_secs(1),
    );
    assert_eq!(
        verdict1,
        Some((step!("x", 7.0, Duration::from_secs(2)), false))
    );
}

#[test]
fn p3_punctual_globally_delayed_qualitative() {
    let verdict0 = last_verdict_at(
        stl!(G[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        DelayedQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict0,
        Some((step!("x", 1.0, Duration::from_secs(1)), true))
    );
    let verdict1 = last_verdict_at(
        stl!(G[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        DelayedQualitative,
        Duration::from_secs(1),
    );
    assert_eq!(
        verdict1,
        Some((step!("x", 7.0, Duration::from_secs(2)), false))
    );
}

#[test]
fn p3_punctual_eventually_delayed_quantitative() {
    let verdict0 = last_verdict_at(
        stl!(F[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict0,
        Some((step!("x", 1.0, Duration::from_secs(1)), 4.0))
    );
    let verdict1 = last_verdict_at(
        stl!(F[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        DelayedQuantitative,
        Duration::from_secs(1),
    );
    assert_eq!(
        verdict1,
        Some((step!("x", 7.0, Duration::from_secs(2)), -2.0))
    );
}

#[test]
fn p3_punctual_globally_delayed_quantitative() {
    let verdict0 = last_verdict_at(
        stl!(G[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict0,
        Some((step!("x", 1.0, Duration::from_secs(1)), 4.0))
    );
    let verdict1 = last_verdict_at(
        stl!(G[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        DelayedQuantitative,
        Duration::from_secs(1),
    );
    assert_eq!(
        verdict1,
        Some((step!("x", 7.0, Duration::from_secs(2)), -2.0))
    );
}

#[test]
fn p3_punctual_eventually_eager_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        EagerQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 1.0, Duration::from_secs(1)), true))
    );
}

#[test]
fn p3_punctual_globally_eager_qualitative() {
    let verdict = last_verdict_at(
        stl!(G[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        EagerQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 1.0, Duration::from_secs(1)), true))
    );
}

#[test]
fn p3_punctual_eventually_rosi() {
    let verdict = last_verdict_at(
        stl!(F[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        Rosi,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((
            step!("x", 1.0, Duration::from_secs(1)),
            RobustnessInterval(4.0, 4.0)
        ))
    );
}

#[test]
fn p3_punctual_globally_rosi() {
    let verdict = last_verdict_at(
        stl!(G[0.5, 0.5](x > 3.0)),
        punctual_signal(),
        Rosi,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((
            step!("x", 1.0, Duration::from_secs(1)),
            RobustnessInterval(4.0, 4.0)
        ))
    );
}

// The on-grid punctual case the P1 fix must not regress: `t + a = 1s` *is* a sample, where
// `x = 1`, so both operators report `false` with robustness 1 - 3 = -2.

#[test]
fn p3_punctual_on_grid_eventually() {
    let verdict = last_verdict_at(
        stl!(F[1, 1](x > 3.0)),
        punctual_signal(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 1.0, Duration::from_secs(1)), -2.0))
    );
}

#[test]
fn p3_punctual_on_grid_globally() {
    let verdict = last_verdict_at(
        stl!(G[1, 1](x > 3.0)),
        punctual_signal(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 1.0, Duration::from_secs(1)), -2.0))
    );
}
