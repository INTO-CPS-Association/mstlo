//! Regression tests: the value a window *opens* on must survive cache pruning.
//!
//! A temporal operator queues three evaluation timestamps per operand breakpoint `ts`:
//! `ts`, `ts - a` and `ts - b` (see `register_sub_steps`). The shifted ones are created
//! only when the sample at `ts` arrives, so they are *younger than the pruning decisions
//! already taken*: at the time the cache was last pruned, `ts - b` was not yet in the
//! evaluation buffer and nothing protected the entry carrying the value in force at its
//! window start. Pruning that entry makes the window-start ZOH read come back empty, and
//! the window is aggregated without the value it opens on.
//!
//! The verdicts are fixed by the dense / zero-order-hold reading of a signal: a sample at
//! `ts` is the value of the signal over the whole of `[ts, next_ts)`.
//!
//! * `F[0,2](x > 3)` on `x = 7 @4.75s, 1 @7.75s, 1 @8.75s`. Under ZOH `x = 7` on
//!   `[4.75, 7.75)`, which contains `6.75`. The window for `t = 6.75s` is `[6.75, 8.75]`
//!   and is witnessed by `t' = 6.75` itself, so the verdict is **true / +4**.
//!
//! * `G[0,2](x > 3)` on `x = 1 @0.5s, 7 @7.75s, 7 @8.75s`, the mirror image. `x = 1` on
//!   `[0.5, 7.75)`, so `t' = 6.75` violates the window `[6.75, 8.75]` and the verdict is
//!   **false / -2**.
//!
//! `6.75s` is `8.75s - 2s`, i.e. exactly a shifted timestamp: it is never submitted as a
//! sample, and it is the last breakpoint of the verdict signal.

use mstlo::monitor::{DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor};
use mstlo::{FormulaDefinition, RobustnessInterval, RobustnessSemantics, Step};
use mstlo::{step, stl};
use pretty_assertions::assert_eq;
use std::fmt::Debug;
use std::time::Duration;

/// Feeds `signal` and returns the last verdict emitted for timestamp `at`.
///
/// The *last* one is the verdict: RoSI re-emits refinements for a timestamp until its
/// window closes, so only the final emission is what the monitor stands behind.
fn last_verdict_at<Y, S>(
    formula: FormulaDefinition,
    signal: Vec<Step<f64>>,
    semantics: S,
    at: Duration,
) -> Option<Y>
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
                verdict = Some(out.value);
            }
        }
    }
    verdict
}

/// The evaluation timestamp under test: `8.75s - 2s`, a breakpoint shifted by the
/// interval's upper bound rather than a submitted sample.
fn shifted_ts() -> Duration {
    Duration::from_secs_f64(6.75)
}

/// `x > 3` holds on `[4.75, 7.75)`, so it is in force and satisfied at `6.75`.
fn eventually_signal() -> Vec<Step<f64>> {
    vec![
        step!("x", 7.0, Duration::from_secs_f64(4.75)),
        step!("x", 1.0, Duration::from_secs_f64(7.75)),
        step!("x", 1.0, Duration::from_secs_f64(8.75)),
    ]
}

/// `x > 3` is violated on `[0.5, 7.75)`, so it is in force and violated at `6.75`.
fn globally_signal() -> Vec<Step<f64>> {
    vec![
        step!("x", 1.0, Duration::from_secs_f64(0.5)),
        step!("x", 7.0, Duration::from_secs_f64(7.75)),
        step!("x", 7.0, Duration::from_secs_f64(8.75)),
    ]
}

// -----------------------------------------------------------------------------
// F[0,2](x > 3) -- the window opens on a satisfied segment
// -----------------------------------------------------------------------------

#[test]
fn window_start_survives_prune_eventually_delayed_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 2](x > 3.0)),
        eventually_signal(),
        DelayedQualitative,
        shifted_ts(),
    );
    assert_eq!(verdict, Some(true));
}

#[test]
fn window_start_survives_prune_eventually_delayed_quantitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 2](x > 3.0)),
        eventually_signal(),
        DelayedQuantitative,
        shifted_ts(),
    );
    assert_eq!(verdict, Some(4.0));
}

#[test]
fn window_start_survives_prune_eventually_eager_qualitative() {
    let verdict = last_verdict_at(
        stl!(F[0, 2](x > 3.0)),
        eventually_signal(),
        EagerQualitative,
        shifted_ts(),
    );
    assert_eq!(verdict, Some(true));
}

#[test]
fn window_start_survives_prune_eventually_rosi() {
    let verdict = last_verdict_at(
        stl!(F[0, 2](x > 3.0)),
        eventually_signal(),
        Rosi,
        shifted_ts(),
    );
    assert_eq!(verdict, Some(RobustnessInterval(4.0, 4.0)));
}

// -----------------------------------------------------------------------------
// G[0,2](x > 3) -- the window opens on a violated segment
// -----------------------------------------------------------------------------

#[test]
fn window_start_survives_prune_globally_delayed_qualitative() {
    let verdict = last_verdict_at(
        stl!(G[0, 2](x > 3.0)),
        globally_signal(),
        DelayedQualitative,
        shifted_ts(),
    );
    assert_eq!(verdict, Some(false));
}

#[test]
fn window_start_survives_prune_globally_delayed_quantitative() {
    let verdict = last_verdict_at(
        stl!(G[0, 2](x > 3.0)),
        globally_signal(),
        DelayedQuantitative,
        shifted_ts(),
    );
    assert_eq!(verdict, Some(-2.0));
}

#[test]
fn window_start_survives_prune_globally_eager_qualitative() {
    let verdict = last_verdict_at(
        stl!(G[0, 2](x > 3.0)),
        globally_signal(),
        EagerQualitative,
        shifted_ts(),
    );
    assert_eq!(verdict, Some(false));
}

#[test]
fn window_start_survives_prune_globally_rosi() {
    let verdict = last_verdict_at(
        stl!(G[0, 2](x > 3.0)),
        globally_signal(),
        Rosi,
        shifted_ts(),
    );
    assert_eq!(verdict, Some(RobustnessInterval(-2.0, -2.0)));
}
