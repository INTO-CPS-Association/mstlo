//! Dense-time semantics of the temporal operators.
//!
//! A sample at `ts` holds its value over `[ts, next_ts)`. Every expectation is worked out by
//! hand under that reading, and pins both the verdict and the input step it is final on.
//! Unless a test says otherwise, all four semantics agree.

mod common;

use common::*;
use mstlo::monitor::EagerQualitative;
use mstlo::{FormulaDefinition, step, stl};

/// `F[0,1](G[0.4,1](x > 3))`: the inner window starts between samples, so only the held
/// value covers its start.
fn eventually_globally() -> FormulaDefinition {
    stl!(F[0, 1](G[0.4, 1](x > 3.0)))
}

// ── Windows over a signal ────────────────────────────────────────────────────

/// No sample ever falls inside `[t+0.98, t+0.99]`; the window reads the held `x = 7`.
#[test]
fn empty_window_reads_the_held_value() {
    let signal = [
        step!("x", 7.0, secs(0.0)),
        step!("x", 4.0, secs(1.0)),
        step!("x", 7.0, secs(2.0)),
        step!("x", 4.0, secs(3.0)),
    ];
    assert_verdict_at(
        stl!(F[0.98, 0.99](x > 3.0)),
        &signal,
        secs(0.0),
        4.0,
        &signal[1],
    );
}

/// A punctual window is the single point `t + a`, so `F` and `G` agree: `x = 7` held at 0.5s,
/// `x = 1` held at 1.5s, and `x = 1` sampled at 1s.
#[test]
fn punctual_window_is_the_value_at_that_point() {
    let signal = [
        step!("x", 7.0, secs(0.0)),
        step!("x", 1.0, secs(1.0)),
        step!("x", 7.0, secs(2.0)),
    ];
    for formula in [stl!(F[0.5, 0.5](x > 3.0)), stl!(G[0.5, 0.5](x > 3.0))] {
        assert_verdict_at(formula.clone(), &signal, secs(0.0), 4.0, &signal[1]);
        assert_verdict_at(formula, &signal, secs(1.0), -2.0, &signal[2]);
    }
    for formula in [stl!(F[1, 1](x > 3.0)), stl!(G[1, 1](x > 3.0))] {
        assert_verdict_at(formula, &signal, secs(0.0), -2.0, &signal[1]);
    }
}

/// `6.75s = 8.75s - 2s` is a shifted evaluation timestamp, queued only once `x @8.75s`
/// arrives. The sample in force there must not have been pruned by then.
#[test]
fn window_start_survives_pruning() {
    let satisfied = [
        step!("x", 7.0, secs(4.75)),
        step!("x", 1.0, secs(7.75)),
        step!("x", 1.0, secs(8.75)),
    ];
    assert_verdict_at(
        stl!(F[0, 2](x > 3.0)),
        &satisfied,
        secs(6.75),
        4.0,
        &satisfied[2],
    );

    let violated = [
        step!("x", 1.0, secs(0.5)),
        step!("x", 7.0, secs(7.75)),
        step!("x", 7.0, secs(8.75)),
    ];
    assert_verdict_at(
        stl!(G[0, 2](x > 3.0)),
        &violated,
        secs(6.75),
        -2.0,
        &violated[2],
    );
}

/// The window closes on the operand's frontier, not the input clock: `x @1s` alone cannot
/// complete the conjunction at 1s, so the verdict waits for `y @1s`.
#[test]
fn window_closes_when_the_operand_catches_up() {
    let signal = [
        step!("x", 1.0, secs(0.0)),
        step!("y", 1.0, secs(0.0)),
        step!("x", 4.0, secs(1.0)),
        step!("y", 4.0, secs(1.0)),
    ];
    let formula = stl!(F[0, 1]((x > 3.0) && (y > 3.0)));
    assert_verdict_at(formula, &signal, secs(0.0), 1.0, &signal[3]);
}

// ── Nested operators ─────────────────────────────────────────────────────────

/// At `s = 1` the inner window `[1.4, 2]` starts on the held `x = 1`, so it is violated
/// even though it ends on `x = 7`.
#[test]
fn nested_window_reads_the_held_inner_value() {
    let signal = [
        step!("x", 1.0, secs(0.0)),
        step!("x", 1.0, secs(1.0)),
        step!("x", 7.0, secs(2.0)),
    ];
    assert_verdict_at(eventually_globally(), &signal, secs(0.0), -2.0, &signal[2]);
}

/// Control: the same inner window over a held `x = 7` is satisfied.
#[test]
fn nested_window_satisfied() {
    let signal = [
        step!("x", 1.0, secs(0.0)),
        step!("x", 7.0, secs(1.0)),
        step!("x", 7.0, secs(2.0)),
    ];
    assert_verdict_at(eventually_globally(), &signal, secs(0.0), 4.0, &signal[2]);
}

/// Control: with a zero lower bound the inner window starts on a sample. Eager decides on
/// `x @1s`, where every inner window from `s` in `[0, 1]` already contains `x = 1`.
#[test]
fn nested_window_with_zero_lower_bound() {
    let signal = [
        step!("x", 1.0, secs(0.0)),
        step!("x", 1.0, secs(1.0)),
        step!("x", 7.0, secs(2.0)),
    ];
    assert_verdict_at_eager_on(
        stl!(F[0, 1](G[0, 1](x > 3.0))),
        &signal,
        secs(0.0),
        -2.0,
        &signal[2],
        &signal[1],
    );
}

/// `x > 3` holds only on `[2, 3)`, so the inner `G` holds for `s` in `[1.6, 2)`: strictly
/// between the operand's samples, reachable only through shifted breakpoints.
#[test]
fn satisfaction_between_breakpoints() {
    let signal = [
        step!("x", 1.0, secs(0.0)),
        step!("x", 1.0, secs(1.0)),
        step!("x", 7.0, secs(2.0)),
        step!("x", 1.0, secs(3.0)),
        step!("x", 1.0, secs(4.0)),
    ];
    assert_verdict_at(eventually_globally(), &signal, secs(0.0), -2.0, &signal[2]);
    assert_verdict_at(eventually_globally(), &signal, secs(1.0), 4.0, &signal[3]);

    // The `Until` counterpart: `psi` holds for `t'` in `[1.6, 2)` and `phi` everywhere.
    let until = stl!((x < 8.0) U[0, 1](G[0.4, 1.0](x > 5.0)));
    assert_verdict_at(until.clone(), &signal, secs(0.0), -4.0, &signal[2]);
    assert_verdict_at(until, &signal, secs(1.0), 2.0, &signal[3]);
}

// ── Non-monotone window cache ────────────────────────────────────────────────

/// A dominated cache entry kept for a pending window must not be read as the window's
/// extremum: `G[0.4,1]` holds on `[1.1, 1.5)`, which `F[0,1]` at 0.1s reaches.
#[test]
fn dominated_entry_kept_for_a_pending_window() {
    let signal = [
        step!("x", 0.0, secs(0.0)),
        step!("x", 4.0, secs(1.5)),
        step!("x", 3.0, secs(2.5)),
    ];
    assert_verdict_at(eventually_globally(), &signal, secs(0.1), 1.0, &signal[2]);
}

/// Eager `||` runs ahead on `y` and hands on breakpoints behind its newest, which the outer
/// cache inserts in place. `y > 2` from 5.15s decides `[4.25, 6.25]`; `[2.15, 4.15]` sees
/// `y = 1` and inner windows reaching the negative `x` at 3.15s.
#[test]
fn breakpoint_inserted_behind_the_newest() {
    let signal = [
        step!("y", 2.0, secs(0.0)),
        step!("x", -1.0, secs(1.0)),
        step!("x", 0.0, secs(1.5)),
        step!("x", 4.0, secs(1.6)),
        step!("y", 1.0, secs(1.85)),
        step!("x", -3.69, secs(3.15)),
        step!("y", 4.0, secs(5.15)),
        step!("x", 1.0, secs(5.25)),
    ];
    let formula = stl!(F[0, 2]((G[0, 1](x > 0.0)) || (y > 2.0)));
    assert_verdict_at(formula.clone(), &signal, secs(2.15), -1.0, &signal[7]);
    // Only eager can decide this window before the trace ends.
    assert_eq!(
        verdict_at(&formula, &signal, EagerQualitative, secs(4.25)),
        Some((signal[7].clone(), true))
    );
}
