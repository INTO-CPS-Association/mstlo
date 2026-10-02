mod common;
mod fixtures;

use fixtures::formulas::*;
use fixtures::signals::*;
use mstlo::monitor::{Algorithm, DelayedQuantitative, Rosi, StlMonitor};
use mstlo::{FormulaDefinition, RobustnessInterval};
use mstlo::{Step, step, stl};
use pretty_assertions::assert_eq;
use rstest::rstest;
use std::collections::HashMap;
use std::time::Duration;

/// Replays `signal` and asserts three properties for RoSI:
///
/// 1. an interval for a timestamp only ever narrows (monotonic),
/// 2. it always brackets the value the delayed semantics settle on there, and
/// 3. it has collapsed onto that value by the time the trace covers the timestamp's horizon.
///
/// Formulas given together must be equivalent; their refinement streams are compared against
/// the first, which pins the timestamps they answer as well as the values.
fn check_rosi(formulas: Vec<FormulaDefinition>, signal: &[Step<f64>]) {
    // How much of the trace a window could fit inside. A trace that starts late covers less
    // than its newest timestamp suggests, and a single-signal formula ignores initial values.
    let span = match (
        signal.iter().map(|step| step.timestamp).min(),
        signal.iter().map(|step| step.timestamp).max(),
    ) {
        (Some(first), Some(last)) => last - first,
        _ => Duration::ZERO,
    };
    // Only equivalence groups need the streams kept; a lone formula has nothing to match.
    let compare_streams = formulas.len() > 1;
    let mut reference: Option<Vec<Step<RobustnessInterval>>> = None;

    for formula in formulas {
        let mut rosi = StlMonitor::builder()
            .formula(formula.clone())
            .semantics(Rosi)
            .algorithm(Algorithm::Incremental)
            .initialize_signals_to_zero()
            .build()
            .unwrap();
        // The same formula under the semantics that answer a timestamp only once, which is
        // the value RoSI has to agree with.
        let mut delayed = StlMonitor::builder()
            .formula(formula.clone())
            .semantics(DelayedQuantitative)
            .algorithm(Algorithm::Incremental)
            .initialize_signals_to_zero()
            .build()
            .unwrap();

        let mut newest: HashMap<Duration, RobustnessInterval> = HashMap::new();
        let mut settled: HashMap<Duration, f64> = HashMap::new();
        let mut stream: Vec<Step<RobustnessInterval>> = Vec::new();

        for input in signal {
            let intervals = rosi.update(input).all_raw_outputs();
            for out in &intervals {
                if let Some(previous) = newest.insert(out.timestamp, out.value) {
                    assert!(
                        out.value.0 >= previous.0 && out.value.1 <= previous.1,
                        "RoSI widened at {:?}: {previous:?} became {:?}, in {formula}",
                        out.timestamp,
                        out.value,
                    );
                }
            }

            // RoSI may still be refining where delayed has settled, but it has seen the same
            // prefix, so its interval must already contain the answer.
            for step in delayed.update(input).raw_outputs() {
                settled.insert(step.timestamp, step.value);
                let interval = newest[&step.timestamp];
                assert!(
                    interval.0 <= step.value && step.value <= interval.1,
                    "delayed value {} outside RoSI interval {interval:?} at {:?}, in {formula}",
                    step.value,
                    step.timestamp,
                );
            }

            if compare_streams {
                stream.extend(intervals);
            }
        }

        for (timestamp, value) in &settled {
            let interval = newest[timestamp];
            assert_eq!(
                (interval.0, interval.1),
                (*value, *value),
                "RoSI did not converge on the delayed value at {timestamp:?}, in {formula}"
            );
        }

        // Convergence is only tested where the trace outlives the horizon, so guard against
        // the loop above quietly having nothing to check.
        let horizon = rosi.temporal_depth();
        assert!(
            span <= horizon || !settled.is_empty(),
            "nothing to converge on: the trace spans {span:?}, past a horizon of {horizon:?}, \
             so {formula} should have settled at least one timestamp"
        );

        if compare_streams {
            match &reference {
                None => reference = Some(stream),
                Some(first) => assert_eq!(
                    first, &stream,
                    "equivalent formulas disagree, {formula} against the first of its group"
                ),
            }
        }
    }
}

#[rstest]
#[case::globally(vec![formula_1(), formula_1_alt(), formula_1_alt_2()])]
#[case::until_over_temporal_operands(vec![formula_2()])]
#[case::eventually_and_globally(vec![formula_3(), formula_3_alt()])]
#[case::eventually_and_constant(vec![formula_4()])]
#[case::eventually(vec![formula_5(), formula_5_alt()])]
#[case::bounded_response(vec![formula_6(), formula_6_alt()])]
#[case::negation_and_constant(vec![formula_7()])]
#[case::globally_and_a_second_signal(vec![formula_8()])]
#[case::deeply_nested_temporal(vec![formula_9()])]
fn rosi_refines_towards_the_delayed_value(
    #[case] formulas: Vec<FormulaDefinition>,
    #[values(
        monotonic_increasing(),
        monotonic_decreasing(),
        sinusoid(),
        sparse_timestamps()
    )]
    signal: Vec<Step<f64>>,
) {
    check_rosi(formulas, &signal);
}

#[rstest]
#[case::two_signals(vec![formula_12()])]
#[case::globally_and_a_second_signal(vec![formula_8(), formula_8_alt()])]
fn rosi_refines_over_two_signals(
    #[case] formulas: Vec<FormulaDefinition>,
    #[values(signal_5(), x_leads_y())] signal: Vec<Step<f64>>,
) {
    check_rosi(formulas, &signal);
}

#[rstest]
fn rosi_refines_over_the_library_formulas(
    #[values(
        monotonic_increasing(),
        monotonic_decreasing(),
        sinusoid(),
        sparse_timestamps()
    )]
    signal: Vec<Step<f64>>,
) {
    for (id, formula) in mstlo::get_formulas(&[]) {
        println!("library formula {id}: {formula}");
        check_rosi(vec![formula], &signal);
    }
}

// ---
// Traces shrunk from failures, each the smallest found that broke one part of reading an
// operand that is still refining. One formula per case: a constant emits at every step, so
// equivalent formulas answer at different timestamps and could not be compared.
// ---

/// Two signals sampled at unrelated times, so each operand settles at its own pace.
fn two_signals() -> Vec<Step<f64>> {
    vec![
        step!("x", 2.0, 0ms),
        step!("y", -2.0, 0ms),
        step!("x", 2.0, 371ms),
        step!("y", -2.0, 529ms),
        step!("y", -1.0, 763ms),
        step!("x", 0.0, 907ms),
        step!("y", -1.0, 1291ms),
        step!("x", -2.0, 1334ms),
        step!("y", -1.0, 1650ms),
        step!("x", -1.0, 2148ms),
        step!("y", 0.0, 2466ms),
        step!("y", -1.0, 2918ms),
        step!("x", 0.0, 3005ms),
        step!("y", 0.0, 3322ms),
        step!("y", -2.0, 3479ms),
        step!("y", -2.0, 3932ms),
        step!("x", 1.0, 3993ms),
        step!("x", -2.0, 4206ms),
        step!("y", 0.0, 4423ms),
        step!("x", 0.0, 4917ms),
        step!("y", 2.0, 5186ms),
        step!("x", -1.0, 5434ms),
        step!("x", 2.0, 6186ms),
        step!("x", -1.0, 6320ms),
    ]
}

/// A window opens on a value old enough to have been a pruning candidate.
fn window_opens_on_a_pruned_value() -> Vec<Step<f64>> {
    vec![
        step!("x", 2.0, 554ms),
        step!("x", -1.0, 3237ms),
        step!("x", -3.0, 3972ms),
        step!("x", 2.0, 4198ms),
    ]
}

/// A breakpoint lands inside a stretch an operand was already holding a value across.
fn breakpoint_splits_a_held_value() -> Vec<Step<f64>> {
    vec![
        step!("y", 2.0, 7106ms),
        step!("x", 3.0, 9297ms),
        step!("x", 0.0, 11221ms),
        step!("y", 2.0, 11539ms),
        step!("y", 3.0, 11694ms),
    ]
}

/// One signal runs far ahead of the other, so the conjunction may only read what is settled.
fn one_signal_far_ahead() -> Vec<Step<f64>> {
    vec![
        step!("x", -1.0, 0ms),
        step!("y", -3.0, 4699ms),
        step!("x", 3.0, 5983ms),
        step!("y", 1.0, 7817ms),
        step!("y", 3.0, 7955ms),
    ]
}

/// Until, with a breakpoint arriving behind what each operand has already reported.
fn until_breakpoint_behind_the_frontier() -> Vec<Step<f64>> {
    vec![
        step!("x", 1.0, 0ms),
        step!("x", 3.0, 1285ms),
        step!("x", -3.0, 1981ms),
        step!("x", 2.0, 3234ms),
    ]
}

/// Until, with psi held across a stretch a later breakpoint splits.
fn until_psi_held_across_a_split() -> Vec<Step<f64>> {
    vec![
        step!("x", -1.0, 3450ms),
        step!("x", 3.0, 6867ms),
        step!("x", -3.0, 6987ms),
    ]
}

/// Until, with phi held across a stretch a later breakpoint splits.
fn until_phi_held_across_a_split() -> Vec<Step<f64>> {
    vec![
        step!("x", 3.0, 2691ms),
        step!("x", 1.0, 4739ms),
        step!("x", -2.0, 4873ms),
    ]
}

#[rstest]
#[case::bounded_response(stl!(G[0, 1.5]((x > 0) -> F[0.5, 2](y > 0))), two_signals())]
#[case::until(stl!((x > 0) U[0, 2] (y > 0)), two_signals())]
#[case::constant_emits_at_every_step(formula_5_alt(), two_signals())]
#[case::pruned_value(stl!(F[0, 1](G[0.4, 1](x > 3))), window_opens_on_a_pruned_value())]
#[case::split_held_value(stl!(G[0, 1.5]((x > 0) -> F[0.5, 2](y > 0))), breakpoint_splits_a_held_value())]
#[case::unsettled_operand(stl!(G[0, 1.5]((x > 0) -> F[0.5, 2](y > 0))), one_signal_far_ahead())]
#[case::until_late_breakpoint(stl!((x > 0) U[0.5, 2] (G[0.2, 1](x > 1))), until_breakpoint_behind_the_frontier())]
#[case::until_psi_split(stl!((x > 0) U[0.5, 2] (G[0.2, 1](x > 1))), until_psi_held_across_a_split())]
#[case::until_phi_split(stl!((G[0, 2](x > 0)) U[0, 1] (x > 2)), until_phi_held_across_a_split())]
fn rosi_reads_operands_that_are_still_refining(
    #[case] formula: FormulaDefinition,
    #[case] signal: Vec<Step<f64>>,
) {
    check_rosi(vec![formula], &signal);
}

/// `G[0, 2]` is bounded above by its own sample from the moment that sample arrives, so
/// `F[0, 10]` collapses once the trace reaches `t = 10`, without waiting for `G`'s window at
/// 10 to close.
#[test]
fn eventually_globally_tightens_before_the_inner_window_closes() {
    let signal = [
        step!("x", 7.0, Duration::from_secs(0)),
        step!("x", 4.0, Duration::from_secs(1)),
        step!("x", 4.0, Duration::from_secs(5)),
        step!("x", 4.0, Duration::from_secs(10)),
        step!("x", 4.0, Duration::from_secs(11)),
    ];
    let verdict = common::verdict_at(
        &stl!(F[0, 10](G[0, 2](x > 50.0))),
        &signal,
        Rosi,
        Duration::ZERO,
    );
    assert_eq!(
        verdict,
        Some((signal[3].clone(), RobustnessInterval(-46.0, -46.0)))
    );
}

/// Replays `samples` of `x` and returns the interval RoSI reports at `t = 0` after each one.
fn rosi_at_zero_after_each(
    formula: FormulaDefinition,
    samples: &[(u64, f64)],
) -> Vec<Option<RobustnessInterval>> {
    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(Rosi)
        .build()
        .expect("Failed to build monitor");
    samples
        .iter()
        .map(|&(t, value)| {
            monitor
                .update(&step!("x", value, Duration::from_secs(t)))
                .verdict_at(Duration::ZERO)
                .copied()
        })
        .collect()
}

/// An `Until` witness that has not arrived yet still needs `phi` to hold over everything
/// seen so far, so the upper bound follows phi's running minimum instead of staying at +inf.
#[test]
fn until_upper_bound_follows_the_running_minimum_of_phi() {
    let at_zero = rosi_at_zero_after_each(
        stl!((x > 0.0) U[0, 2] (x > 50.0)),
        &[(0, 7.0), (1, 4.0), (5, 4.0)],
    );
    assert_eq!(
        at_zero,
        [
            Some(RobustnessInterval(-43.0, 7.0)),
            Some(RobustnessInterval(-43.0, 4.0)),
            Some(RobustnessInterval(-43.0, -43.0)),
        ]
    );
}

/// Same as above, but `phi` dips below zero at 1s, so the upper bound goes negative: the
/// formula is already known to be violated at 0s.
#[test]
fn until_upper_bound_goes_negative_once_phi_is_violated() {
    let at_zero = rosi_at_zero_after_each(
        stl!((x > 0.0) U[0, 2] (x > 50.0)),
        &[(0, 7.0), (1, -4.0), (5, 4.0)],
    );
    assert_eq!(
        at_zero,
        [
            Some(RobustnessInterval(-43.0, 7.0)),
            Some(RobustnessInterval(-43.0, -4.0)),
            Some(RobustnessInterval(-43.0, -43.0)),
        ]
    );
}

/// `psi = G[0, 10]` is bounded above as soon as it has data, so `Until` keeps that bound
/// instead of discarding it, while the lower bound waits for `G`'s window at 0 to close.
#[test]
fn until_keeps_the_upper_bound_of_a_globally_psi() {
    let at_zero = rosi_at_zero_after_each(
        stl!((x > 0.0) U[0, 2] (G[0, 10](x > 50.0))),
        &[(0, 7.0), (1, 4.0), (5, 4.0), (10, 4.0)],
    );
    assert_eq!(
        at_zero,
        [
            Some(RobustnessInterval(f64::NEG_INFINITY, 7.0)),
            Some(RobustnessInterval(f64::NEG_INFINITY, 4.0)),
            Some(RobustnessInterval(f64::NEG_INFINITY, -46.0)),
            Some(RobustnessInterval(-46.0, -46.0)),
        ]
    );
}

/// Operands whose lower and upper bounds are reached at different times: `G` is bounded above
/// as soon as it has data and below once its window closes, `F` the other way round. Nesting
/// them, and negating them, exercises reading one bound of an operand while the other is open.
#[rstest]
#[case::until_over_globally(stl!((x > 0) U[0, 2] (G[0, 10](x > 1))))]
#[case::until_over_eventually(stl!((x > 0) U[0, 2] (F[0, 3](x > 1))))]
#[case::until_with_offset_windows(stl!((G[0, 2](x > 0)) U[0.5, 3] (G[0.2, 1](x > 1))))]
#[case::until_over_negated_globally(stl!((x > 0) U[0.5, 2] (!(G[0, 4](x > 1)))))]
#[case::until_with_negated_phi(stl!((!(G[0, 3](x > 1))) U[0, 2] (F[0, 2](x > 0))))]
#[case::eventually_over_globally(stl!(F[0, 10](G[0, 2](x > 1))))]
#[case::globally_over_eventually(stl!(G[0, 10](F[0, 2](x > 1))))]
#[case::eventually_over_globally_offsets(stl!(F[1, 5](G[0.5, 2](x > 0))))]
#[case::globally_over_eventually_offsets(stl!(G[0.5, 4](F[0.2, 1](x > 0))))]
#[case::until_inside_eventually(stl!(F[0, 3]((x > 0) U[0, 2] (G[0, 3](x > 1)))))]
fn rosi_stays_sound_when_the_bounds_of_an_operand_differ(
    #[case] formula: FormulaDefinition,
    #[values(
        monotonic_increasing(),
        monotonic_decreasing(),
        sinusoid(),
        sparse_timestamps()
    )]
    signal: Vec<Step<f64>>,
) {
    check_rosi(vec![formula], &signal);
}

/// The same, with the operands on different signals that run at different rates.
#[rstest]
#[case::until_over_globally(stl!((x > 0) U[0, 2] (G[0, 5](y > 0))))]
#[case::until_over_eventually(stl!((y > 0) U[0.5, 3] (F[0, 2](x > 0))))]
#[case::response_over_globally(stl!(G[0, 3]((x > 0) -> F[0, 2](G[0, 1](y > 0)))))]
fn rosi_stays_sound_when_the_bounds_of_an_operand_differ_over_two_signals(
    #[case] formula: FormulaDefinition,
    #[values(signal_5(), x_leads_y())] signal: Vec<Step<f64>>,
) {
    check_rosi(vec![formula], &signal);
}
