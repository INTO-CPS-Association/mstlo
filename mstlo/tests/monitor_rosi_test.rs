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
