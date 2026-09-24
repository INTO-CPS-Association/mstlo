//! Atomic STL operators.
//!
//! This module contains predicate leaves used by the STL operator tree.
//! Atomic operators evaluate one input sample and emit one output sample in the
//! selected robustness domain `Y`.

use crate::core::{RobustnessSemantics, SignalIdentifier, StlOperatorTrait, Variables};
use crate::ring_buffer::Step;
use crate::synchronizer::SignalInterpolation;
use std::collections::HashSet;
use std::fmt::Display;
use std::time::Duration;

/// The comparison an [`Atomic`] performs.
///
/// Every variant compares one signal against a constant, so its satisfaction signal stays
/// piecewise constant under [`SignalInterpolation::Linear`].
#[derive(Clone)]
pub enum Predicate {
    /// Signal less than constant: signal < value
    LessThan(&'static str, f64),
    /// Signal greater than constant: signal > value
    GreaterThan(&'static str, f64),
    /// Signal less than variable: signal < $var_name
    LessThanVar(&'static str, &'static str, Variables),
    /// Signal greater than variable: signal > $var_name
    GreaterThanVar(&'static str, &'static str, Variables),
    /// Always true
    True,
    /// Always false
    False,
}

impl Predicate {
    /// The signal this predicate reads, or `None` for the constants.
    fn signal(&self) -> Option<&'static str> {
        match self {
            Predicate::LessThan(s, _)
            | Predicate::GreaterThan(s, _)
            | Predicate::LessThanVar(s, _, _)
            | Predicate::GreaterThanVar(s, _, _) => Some(s),
            Predicate::True | Predicate::False => None,
        }
    }

    /// The threshold in force right now, or `None` for the constants.
    ///
    /// For the `…Var` forms this is read from the variables context on every call.
    fn threshold(&self) -> Option<f64> {
        match self {
            Predicate::LessThan(_, c) | Predicate::GreaterThan(_, c) => Some(*c),
            Predicate::LessThanVar(_, var_name, vars)
            | Predicate::GreaterThanVar(_, var_name, vars) => Some(
                vars.get(var_name)
                    .unwrap_or_else(|| panic!("Variable '{}' not found in context", var_name)),
            ),
            Predicate::True | Predicate::False => None,
        }
    }

    /// Whether the predicate holds at `value`. Used to detect crossings between samples.
    fn holds(&self, value: f64) -> bool {
        match self {
            Predicate::True => true,
            Predicate::False => false,
            Predicate::GreaterThan(_, _) | Predicate::GreaterThanVar(_, _, _) => {
                self.threshold().is_some_and(|c| value > c)
            }
            Predicate::LessThan(_, _) | Predicate::LessThanVar(_, _, _) => {
                self.threshold().is_some_and(|c| value < c)
            }
        }
    }
}

/// Atomic predicates for STL formulas.
///
/// Supports both constant thresholds (e.g., `x > 5.0`) and variable thresholds
/// (e.g., `x > $A` where `A` is looked up from a `Variables` context at runtime).
///
/// Under [`SignalInterpolation::Linear`] it also emits a breakpoint at each threshold
/// crossing between samples.
#[derive(Clone)]
pub struct Atomic<Y> {
    predicate: Predicate,
    interpolation: SignalInterpolation,
    /// The previous sample of the referenced signal, under `Linear` only.
    prev: Option<(Duration, f64)>,
    /// Set when `prev` sits exactly on the threshold. Its verdict holds on
    /// `[prev, next breakpoint)`, so it is withheld until the next sample shows which way
    /// the signal goes.
    deferred: bool,
    _phantom: std::marker::PhantomData<Y>,
}

impl<Y> Atomic<Y> {
    fn from_predicate(predicate: Predicate) -> Self {
        Atomic {
            predicate,
            interpolation: SignalInterpolation::default(),
            prev: None,
            deferred: false,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Selects how the input signal is read between samples.
    pub fn with_interpolation(mut self, interpolation: SignalInterpolation) -> Self {
        self.interpolation = interpolation;
        self
    }

    /// Creates an atomic predicate `signal_name < val`.
    pub fn new_less_than(signal_name: &'static str, val: f64) -> Self {
        Self::from_predicate(Predicate::LessThan(signal_name, val))
    }

    /// Creates an atomic predicate `signal_name > val`.
    pub fn new_greater_than(signal_name: &'static str, val: f64) -> Self {
        Self::from_predicate(Predicate::GreaterThan(signal_name, val))
    }

    /// Creates an atomic predicate `signal_name < $var_name`.
    ///
    /// The threshold is resolved from `vars` at evaluation time.
    pub fn new_less_than_var(
        signal_name: &'static str,
        var_name: &'static str,
        vars: Variables,
    ) -> Self {
        Self::from_predicate(Predicate::LessThanVar(signal_name, var_name, vars))
    }

    /// Creates an atomic predicate `signal_name > $var_name`.
    ///
    /// The threshold is resolved from `vars` at evaluation time.
    pub fn new_greater_than_var(
        signal_name: &'static str,
        var_name: &'static str,
        vars: Variables,
    ) -> Self {
        Self::from_predicate(Predicate::GreaterThanVar(signal_name, var_name, vars))
    }

    /// Creates a constant `True` atomic operator.
    pub fn new_true() -> Self {
        Self::from_predicate(Predicate::True)
    }

    /// Creates a constant `False` atomic operator.
    pub fn new_false() -> Self {
        Self::from_predicate(Predicate::False)
    }
}

/// The instant a straight segment from `(t0, v0)` to `(t1, v1)` meets the threshold `c`.
///
/// Returns `None` when the crossing does not fall strictly inside the segment at
/// nanosecond resolution.
fn crossing_time(t0: Duration, v0: f64, t1: Duration, v1: f64, c: f64) -> Option<Duration> {
    let span = t1.checked_sub(t0)?;
    let alpha = (c - v0) / (v1 - v0);
    if !alpha.is_finite() {
        return None;
    }
    let offset = Duration::try_from_secs_f64(span.as_secs_f64() * alpha.clamp(0.0, 1.0)).ok()?;
    let t_c = t0 + offset;
    (t_c > t0 && t_c < t1).then_some(t_c)
}

impl<T, Y> StlOperatorTrait<T> for Atomic<Y>
where
    T: Into<f64> + Clone + 'static,
    Y: RobustnessSemantics + 'static,
{
    type Output = Y;

    /// Evaluates this atomic operator for the incoming sample.
    ///
    /// If the sample's signal identifier does not match the predicate signal,
    /// no output is emitted.
    ///
    /// For variable-based predicates, this method panics if the variable is not
    /// present in the provided [`Variables`] context.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>> {
        let value = step.value.clone().into();

        // Filter by signal. `True`/`False` reference no signal and accept any
        // step. Compared directly rather than via `get_signal_identifiers()`,
        // which would allocate a `HashSet` on every step.
        let accepts_step = self
            .predicate
            .signal()
            .is_none_or(|signal_name| signal_name == step.signal);
        if !accepts_step {
            return vec![];
        }

        let result = self.robustness(value);

        // Zero-order hold, and the constants, which have no signal to interpolate.
        let Some(threshold) = self
            .predicate
            .threshold()
            .filter(|_| self.interpolation == SignalInterpolation::Linear)
        else {
            return vec![Step::new("output", result, step.timestamp)];
        };

        let mut output = Vec::new();
        if let Some((prev_ts, prev_value)) = self.prev {
            if self.deferred {
                // `prev` sat on the threshold: emit its verdict now that the direction is known.
                output.push(Step::new("output", result.clone(), prev_ts));
            } else if self.predicate.holds(prev_value) != self.predicate.holds(value)
                && let Some(t_c) =
                    crossing_time(prev_ts, prev_value, step.timestamp, value, threshold)
            {
                output.push(Step::new("output", result.clone(), t_c));
            }
        }

        self.prev = Some((step.timestamp, value));
        self.deferred = value == threshold;
        if !self.deferred {
            output.push(Step::new("output", result, step.timestamp));
        }
        output
    }

    fn get_max_lookahead(&self) -> Duration {
        Duration::ZERO
    }

    fn reset(&mut self) {
        self.prev = None;
        self.deferred = false;
    }

    fn total_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl<Y> Atomic<Y>
where
    Y: RobustnessSemantics,
{
    /// The predicate's value at `value`, in the robustness domain `Y`.
    fn robustness(&self, value: f64) -> Y {
        match &self.predicate {
            Predicate::True => Y::atomic_true(),
            Predicate::False => Y::atomic_false(),
            Predicate::GreaterThan(_, c) => Y::atomic_greater_than(value, *c),
            Predicate::LessThan(_, c) => Y::atomic_less_than(value, *c),
            Predicate::GreaterThanVar(_, var_name, vars) => {
                let c = vars
                    .get(var_name)
                    .unwrap_or_else(|| panic!("Variable '{}' not found in context", var_name));
                Y::atomic_greater_than(value, c)
            }
            Predicate::LessThanVar(_, var_name, vars) => {
                let c = vars
                    .get(var_name)
                    .unwrap_or_else(|| panic!("Variable '{}' not found in context", var_name));
                Y::atomic_less_than(value, c)
            }
        }
    }
}

impl<Y> SignalIdentifier for Atomic<Y> {
    /// Returns the referenced signal for predicate variants.
    ///
    /// Constant variants (`True`/`False`) return an empty set.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        let mut ids = std::collections::HashSet::new();
        if let Some(signal_name) = self.predicate.signal() {
            ids.insert(signal_name);
        }
        ids
    }
}

impl<Y> Display for Atomic<Y> {
    /// Formats the atomic operator using formula-like notation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.predicate {
            Predicate::LessThan(signal_name, c) => write!(f, "{signal_name} < {c}"),
            Predicate::GreaterThan(signal_name, c) => write!(f, "{signal_name} > {c}"),
            Predicate::LessThanVar(signal_name, var_name, _) => {
                write!(f, "{signal_name} < ${var_name}")
            }
            Predicate::GreaterThanVar(signal_name, var_name, _) => {
                write!(f, "{signal_name} > ${var_name}")
            }
            Predicate::True => write!(f, "True"),
            Predicate::False => write!(f, "False"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::StlOperatorTrait;
    use crate::step;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn atomic_unexisting_identifier() {
        let mut atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        atomic.get_signal_identifiers();
        let step = step!("y", 15.0, Duration::from_secs(5));
        let robustness = atomic.update(&step);
        assert!(robustness.is_empty());
    }

    #[test]
    #[should_panic(expected = "Variable 'A' not found in context")]
    fn atomics_gt_variables_not_in_context() {
        let vars = Variables::new();
        let mut atomic = Atomic::<f64>::new_greater_than_var("x", "A", vars);
        atomic.get_signal_identifiers();
        let step = step!("x", 15.0, Duration::from_secs(5));
        atomic.update(&step);
    }

    #[test]
    #[should_panic(expected = "Variable 'A' not found in context")]
    fn atomics_lt_variables_not_in_context() {
        let vars = Variables::new();
        let mut atomic = Atomic::<f64>::new_less_than_var("x", "A", vars);
        atomic.get_signal_identifiers();
        let step = step!("x", 15.0, Duration::from_secs(5));
        atomic.update(&step);
    }

    #[test]
    fn atomic_greater_than_robustness() {
        let mut atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        atomic.get_signal_identifiers();
        let step1 = step!("x", 15.0, Duration::from_secs(5));
        let robustness = atomic.update(&step1);
        assert_eq!(
            robustness,
            vec![step!("output", 5.0, Duration::from_secs(5))]
        );

        let step2 = step!("x", 8.0, Duration::from_secs(6));
        let robustness2 = atomic.update(&step2);
        assert_eq!(
            robustness2,
            vec![step!("output", -2.0, Duration::from_secs(6))]
        );
    }

    #[test]
    fn atomic_less_than_robustness() {
        let mut atomic = Atomic::<f64>::new_less_than("x", 10.0);
        atomic.get_signal_identifiers();
        let step1 = step!("x", 5.0, Duration::from_secs(5));
        let robustness = atomic.update(&step1);
        assert_eq!(
            robustness,
            vec![step!("output", 5.0, Duration::from_secs(5))]
        );

        let step2 = step!("x", 12.0, Duration::from_secs(6));
        let robustness2 = atomic.update(&step2);
        assert_eq!(
            robustness2,
            vec![step!("output", -2.0, Duration::from_secs(6))]
        );
    }

    #[test]
    fn atomic_true_robustness() {
        let mut atomic = Atomic::<f64>::new_true();
        atomic.get_signal_identifiers();
        let step = step!("x", 0.0, Duration::from_secs(5));
        let robustness = atomic.update(&step);
        assert_eq!(
            robustness,
            vec![step!("output", f64::INFINITY, Duration::from_secs(5))]
        );
    }

    #[test]
    fn atomic_false_robustness() {
        let mut atomic = Atomic::<f64>::new_false();
        atomic.get_signal_identifiers();
        let step = step!("x", 0.0, Duration::from_secs(5));
        let robustness = atomic.update(&step);
        assert_eq!(
            robustness,
            vec![step!("output", f64::NEG_INFINITY, Duration::from_secs(5))]
        );
    }

    #[test]
    fn atomic_with_variables_robustness() {
        let vars = Variables::new();
        vars.set("A", 10.0);
        let mut atomic = Atomic::<f64>::new_greater_than_var("x", "A", vars);
        atomic.get_signal_identifiers();

        let step1 = step!("x", 15.0, Duration::from_secs(5));
        let robustness = atomic.update(&step1);
        assert_eq!(
            robustness,
            vec![step!("output", 5.0, Duration::from_secs(5))]
        );

        let vars = Variables::new();
        vars.set("A", 10.0);
        let mut atomic = Atomic::<f64>::new_less_than_var("x", "A", vars);
        atomic.get_signal_identifiers();

        let step1 = step!("x", 15.0, Duration::from_secs(5));
        let robustness = atomic.update(&step1);
        assert_eq!(
            robustness,
            vec![step!("output", -5.0, Duration::from_secs(5))]
        );
    }

    #[test]
    fn atomic_signal_identifiers() {
        let mut atomic_gt = Atomic::<f64>::new_greater_than("x", 10.0);
        let ids_gt = atomic_gt.get_signal_identifiers();
        assert_eq!(ids_gt.len(), 1);
        assert!(ids_gt.contains("x"));
        let mut atomic_lt = Atomic::<f64>::new_less_than("y", 5.0);
        let ids_lt = atomic_lt.get_signal_identifiers();
        assert_eq!(ids_lt.len(), 1);
        assert!(ids_lt.contains("y"));
        let mut atomic_true = Atomic::<f64>::new_true();
        let ids_true = atomic_true.get_signal_identifiers();
        assert_eq!(ids_true.len(), 0);
        let mut atomic_false = Atomic::<f64>::new_false();
        let ids_false = atomic_false.get_signal_identifiers();
        assert_eq!(ids_false.len(), 0);
    }

    #[test]
    fn atomic_display() {
        let atomic_gt = Atomic::<f64>::new_greater_than("x", 10.0);
        assert_eq!(format!("{}", atomic_gt), "x > 10");
        let atomic_lt = Atomic::<f64>::new_less_than("y", 5.0);
        assert_eq!(format!("{}", atomic_lt), "y < 5");
        let vars = Variables::new();
        let atomic_gt_var = Atomic::<f64>::new_greater_than_var("x", "A", vars.clone());
        assert_eq!(format!("{}", atomic_gt_var), "x > $A");
        let atomic_lt_var = Atomic::<f64>::new_less_than_var("y", "B", vars);
        assert_eq!(format!("{}", atomic_lt_var), "y < $B");
        let atomic_true = Atomic::<f64>::new_true();
        assert_eq!(format!("{}", atomic_true), "True");
        let atomic_false = Atomic::<f64>::new_false();
        assert_eq!(format!("{}", atomic_false), "False");
    }
}
