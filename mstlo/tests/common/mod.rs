#![allow(dead_code)]

use mstlo::monitor::{DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, StlMonitor};
use mstlo::{FormulaDefinition, RobustnessInterval, RobustnessSemantics, SemanticType, Step, step};
use std::fmt::Debug;
use std::time::Duration;

pub fn secs(t: f64) -> Duration {
    Duration::from_secs_f64(t)
}

/// Defines every signal of `trace` from `t=0` by its own first sample, which a monitor over
/// more than one signal requires. A signal already sampled at `t=0` ignores it, so a trace
/// that starts them all together is monitored exactly as it is written.
pub fn initial_values(trace: &[Step<f64>]) -> Vec<(&'static str, f64)> {
    let mut initial: Vec<(&'static str, f64)> = Vec::new();
    for step in trace {
        if !initial.iter().any(|(signal, _)| *signal == step.signal) {
            initial.push((step.signal, step.value));
        }
    }
    initial
}

/// The last verdict emitted for `at`, with the input step it was first emitted on. RoSI
/// refines a timestamp until it is final, and may repeat it after.
pub fn verdict_at<S, Y>(
    formula: &FormulaDefinition,
    signal: &[Step<f64>],
    semantics: S,
    at: Duration,
) -> Option<(Step<f64>, Y)>
where
    S: SemanticType<Output = Y> + Copy,
    Y: RobustnessSemantics + 'static + Copy + Debug + PartialEq,
{
    let mut monitor = StlMonitor::builder()
        .formula(formula.clone())
        .semantics(semantics)
        .initialize_signals(initial_values(signal))
        .build()
        .unwrap();
    let mut verdict = None;
    for input in signal {
        for out in monitor.update(input).all_raw_outputs() {
            if out.timestamp == at
                && verdict
                    .as_ref()
                    .is_none_or(|(_, value)| *value != out.value)
            {
                verdict = Some((input.clone(), out.value));
            }
        }
    }
    verdict
}

/// Asserts the verdict at `at` under all four semantics: robustness `rho`, satisfied iff
/// `rho > 0`, emitted on input step `on`.
pub fn assert_verdict_at(
    formula: FormulaDefinition,
    signal: &[Step<f64>],
    at: Duration,
    rho: f64,
    on: &Step<f64>,
) {
    assert_verdict_at_eager_on(formula, signal, at, rho, on, on);
}

/// As [`assert_verdict_at`], but where the prefix decides `at` before the trace covers it, so
/// the eager semantics answer on `eager_on` rather than on `on`.
pub fn assert_verdict_at_eager_on(
    formula: FormulaDefinition,
    signal: &[Step<f64>],
    at: Duration,
    rho: f64,
    on: &Step<f64>,
    eager_on: &Step<f64>,
) {
    let context = format!("{formula} at {at:?}");
    let eager_on = eager_on.clone();
    let on = on.clone();
    assert_eq!(
        verdict_at(&formula, signal, DelayedQualitative, at),
        Some((on.clone(), rho > 0.0)),
        "DelayedQualitative, {context}"
    );
    assert_eq!(
        verdict_at(&formula, signal, DelayedQuantitative, at),
        Some((on.clone(), rho)),
        "DelayedQuantitative, {context}"
    );
    assert_eq!(
        verdict_at(&formula, signal, EagerQualitative, at),
        Some((eager_on, rho > 0.0)),
        "EagerQualitative, {context}"
    );
    assert_eq!(
        verdict_at(&formula, signal, Rosi, at),
        Some((on, RobustnessInterval(rho, rho))),
        "Rosi, {context}"
    );
}

pub fn convert_f64_vec_to_bool_vec(input: Vec<Vec<Step<f64>>>) -> Vec<Vec<Step<bool>>> {
    input
        .into_iter()
        .map(|inner_vec| {
            inner_vec
                .into_iter()
                .map(|step| {
                    let bool_value =
                        step.value > 0.0 || (step.value == 0.0 && step.value.is_sign_negative());
                    step!("output", bool_value, step.timestamp)
                })
                .collect()
        })
        .collect()
}

// Helper to create a vector of steps
pub fn create_steps(name: &'static str, values: Vec<f64>, timestamps: Vec<u64>) -> Vec<Step<f64>> {
    values
        .into_iter()
        .zip(timestamps)
        .map(|(val, ts)| step!(name, val, Duration::from_secs(ts)))
        .collect()
}

pub fn combine_and_sort_steps(step_vectors: Vec<Vec<Step<f64>>>) -> Vec<Step<f64>> {
    let mut combined_steps = step_vectors.into_iter().flatten().collect::<Vec<_>>();
    combined_steps.sort_by_key(|step| step.timestamp);
    combined_steps
}
