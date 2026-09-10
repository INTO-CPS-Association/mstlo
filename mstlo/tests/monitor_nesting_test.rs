//! Regression tests for `finding_1.md` — zero-order hold across *nested* temporal operators.
//!
//! `monitor_window_test.rs` pins down the dense reading of a window over a raw atomic. This
//! file does the same one level up: the operand of the outer operator is itself a temporal
//! operator, so the value being held between breakpoints is a *verdict*, and the inner
//! window is shifted by the outer evaluation time.
//!
//! The formula under test is
//!
//! ```text
//! F[0, 1](G[0.4, 1](x > 3))
//! ```
//!
//! read at `t = 0`: *somewhere in the next second there is an instant `s` from which `x`
//! stays above 3 for the whole of `[s + 0.4, s + 1]`*.
//!
//! What makes it a good probe is that the inner interval has a non-zero lower bound. With
//! `a = 0` the inner window starts exactly on `s`, which is a sample whenever `s` is, so the
//! held value is never needed and the pointwise and dense readings agree. With `a = 0.4` the
//! inner window opens strictly between two samples, and only the held value covers it.

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
/// Same helper as in `monitor_window_test.rs`: the *last* emission is the one that counts,
/// because `Rosi` refines a timestamp until its window closes.
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
// N1: the inner window reads the value held from the previous sample
// -----------------------------------------------------------------------------

/// `x > 3` is false on `[0s, 2s)` and true from 2s onwards.
fn late_rise_signal() -> Vec<Step<f64>> {
    vec![
        step!("x", 1.0, Duration::from_secs(0)),
        step!("x", 1.0, Duration::from_secs(1)),
        step!("x", 7.0, Duration::from_secs(2)),
    ]
}

// Worked out at t = 0, under zero-order hold. The outer `F[0, 1]` ranges over `s` in
// `[0, 1]`, and the only `s` for which the inner window `[s + 0.4, s + 1]` can reach the
// rise at 2s is `s = 1`, giving `[1.4, 2.0]`.
//
// That window is not empty and it is not the single point 2.0 either: it starts at 1.4,
// inside the segment that the sample at 1s holds, where `x = 1`. `G` takes the minimum over
// the whole window, so the held `1 - 3 = -2` wins over the `7 - 3 = 4` contributed at 2.0,
// and the inner operator is *false* at `s = 1`. Every earlier `s` is false for the same
// reason, so the whole formula is false at t = 0, with robustness -2.
//
// Reading the window pointwise sees only the sample at 2.0 and reports `true` / 4.0 —
// that is assuming `x = 7` backwards in time, into a segment where the signal was 1.
//
// EMISSION STEP. The verdict for t=0 arrives on `x @2s` for all four semantics. The outer
// window at t=0 needs the inner verdict at s=1, which in turn needs the signal up to 2s.

#[test]
fn n1_held_inner_window_delayed_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        late_rise_signal(),
        DelayedQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 7.0, Duration::from_secs(2)), false))
    );
}

#[test]
fn n1_held_inner_window_delayed_quantitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        late_rise_signal(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 7.0, Duration::from_secs(2)), -2.0))
    );
}

#[test]
fn n1_held_inner_window_eager_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        late_rise_signal(),
        EagerQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 7.0, Duration::from_secs(2)), false))
    );
}

#[test]
fn n1_held_inner_window_rosi() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        late_rise_signal(),
        Rosi,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((
            step!("x", 7.0, Duration::from_secs(2)),
            RobustnessInterval(-2.0, -2.0)
        ))
    );
}

// -----------------------------------------------------------------------------
// N2: control — with `a = 0` the held value is never needed
// -----------------------------------------------------------------------------

// `G[0, 1]` opens its window on `s` itself. For every `s` the monitor evaluates, `s` is a
// sample of the operand, so the pointwise and the dense reading coincide and the verdict is
// false either way. This is the case that already worked before the fix; it is here to keep
// the contrast with N1 explicit, and to catch a fix that changes it.

#[test]
fn n2_zero_lower_bound_delayed_quantitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0, 1](x > 3.0))),
        late_rise_signal(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 7.0, Duration::from_secs(2)), -2.0))
    );
}

#[test]
fn n2_zero_lower_bound_eager_qualitative() {
    // Eager settles one step earlier here: at `x @1s` the inner `G[0, 1]` is already
    // decidably false at s=0 (its window [0, 1] contains the sample at 1s), and the outer
    // `F` still has s=1 to go, so this is a refutation of the whole window only once the
    // frontier passes it. `x @1s` is where the outer window's own short-circuit budget runs
    // out — see the eager notes in `monitor_window_test.rs`.
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0, 1](x > 3.0))),
        late_rise_signal(),
        EagerQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 1.0, Duration::from_secs(1)), false))
    );
}

// -----------------------------------------------------------------------------
// N3: positive control — the same formula, satisfied
// -----------------------------------------------------------------------------

/// `x > 3` is false on `[0s, 1s)` and true from 1s onwards.
fn early_rise_signal() -> Vec<Step<f64>> {
    vec![
        step!("x", 1.0, Duration::from_secs(0)),
        step!("x", 7.0, Duration::from_secs(1)),
        step!("x", 7.0, Duration::from_secs(2)),
    ]
}

// At `s = 1` the inner window `[1.4, 2.0]` now lies entirely inside the segment held by the
// sample at 1s, where `x = 7`, so the inner `G` is true with robustness 4 and the outer `F`
// reports it. Same window, same held value — only the value held is different, which is what
// separates this from N1 and rules out a fix that simply always answers false.

#[test]
fn n3_satisfied_delayed_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        early_rise_signal(),
        DelayedQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 7.0, Duration::from_secs(2)), true))
    );
}

#[test]
fn n3_satisfied_delayed_quantitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        early_rise_signal(),
        DelayedQuantitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 7.0, Duration::from_secs(2)), 4.0))
    );
}

#[test]
fn n3_satisfied_eager_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        early_rise_signal(),
        EagerQualitative,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((step!("x", 7.0, Duration::from_secs(2)), true))
    );
}

#[test]
fn n3_satisfied_rosi() {
    let verdict = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        early_rise_signal(),
        Rosi,
        Duration::from_secs(0),
    );
    assert_eq!(
        verdict,
        Some((
            step!("x", 7.0, Duration::from_secs(2)),
            RobustnessInterval(4.0, 4.0)
        ))
    );
}

// -----------------------------------------------------------------------------
// N4: the residual gap — an inner satisfaction interval with no sample in it
// -----------------------------------------------------------------------------

/// `x > 3` is true on exactly one segment, `[2s, 3s)`.
fn pulse_signal() -> Vec<Step<f64>> {
    vec![
        step!("x", 1.0, Duration::from_secs(0)),
        step!("x", 1.0, Duration::from_secs(1)),
        step!("x", 7.0, Duration::from_secs(2)),
        step!("x", 1.0, Duration::from_secs(3)),
        step!("x", 1.0, Duration::from_secs(4)),
    ]
}

/// Known limitation, see `finding_1.md` §5 Option C. Holding the operand's value between its
/// samples is not the same as evaluating the operator between them, and this is where the
/// difference shows.
///
/// Read at t = 1s: the inner `G[0.4, 1]` is true at `s` exactly when `[s + 0.4, s + 1]` is
/// inside `[2s, 3s)`, i.e. for `s` in `[1.6, 2)`. That is a non-empty sub-interval of the
/// outer window `[1, 2]`, so `F[0, 1]` is true at t = 1s.
///
/// The monitor evaluates the inner operator only at operand breakpoints — 0, 1, 2, 3, 4 —
/// and at none of them is it true: at `s = 1` the window `[1.4, 2]` still holds `x = 1`,
/// and at `s = 2` the window `[2.4, 3]` already reaches the fall at 3s. The satisfaction
/// interval `[1.6, 2)` falls between two evaluation points and is missed, so the monitor
/// answers false.
///
/// The fix is to evaluate at the shifted breakpoints as well — for each operand breakpoint
/// `ts`, additionally at `ts - a` and `ts - b` (here 2 - 0.4 = 1.6 and 2 - 1 = 1), which is
/// the Maler–Nickovic interval construction. That is a separate change from the
/// zero-order-hold read tested above; it is prototyped in `finding_1_optionc.patch`, with
/// which this test passes. `mstlo/examples/nesting_breakpoint_gap.rs` walks through the case.
#[test]
#[ignore = "fixed only by finding_1_optionc.patch, not by P1/P2; see finding_1.md §5 Option C"]
fn n4_satisfaction_between_breakpoints() {
    let verdict0 = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        pulse_signal(),
        DelayedQualitative,
        Duration::from_secs(0),
    )
    .map(|(_, value)| value);
    assert_eq!(verdict0, Some(false));
    let verdict1 = last_verdict_at(
        stl!(F[0, 1](G[0.4, 1](x > 3.0))),
        pulse_signal(),
        DelayedQualitative,
        Duration::from_secs(1),
    )
    .map(|(_, value)| value);
    assert_eq!(verdict1, Some(true));
}
