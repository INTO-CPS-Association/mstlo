//! Atomic STL operators.
//!
//! This module contains predicate leaves used by the STL operator tree.
//! Atomic operators evaluate one input sample and emit one output sample in the
//! selected robustness domain `Y`.

use crate::ring_buffer::Step;
use crate::stl::core::{RobustnessSemantics, SignalIdentifier, StlOperatorTrait, Variables};
use std::collections::HashSet;
use std::fmt::Display;
use std::time::Duration;

/// Atomic predicates for STL formulas.
///
/// Supports both constant thresholds (e.g., `x > 5.0`) and variable thresholds
/// (e.g., `x > $A` where `A` is looked up from a `Variables` context at runtime).
#[derive(Clone)]
pub enum Atomic<Y> {
    /// Signal less than constant: signal < value
    LessThan(&'static str, f64, std::marker::PhantomData<Y>),
    /// Signal greater than constant: signal > value
    GreaterThan(&'static str, f64, std::marker::PhantomData<Y>),
    /// Signal less than variable: signal < $var_name
    LessThanVar(
        &'static str,
        &'static str,
        Variables,
        std::marker::PhantomData<Y>,
    ),
    /// Signal greater than variable: signal > $var_name
    GreaterThanVar(
        &'static str,
        &'static str,
        Variables,
        std::marker::PhantomData<Y>,
    ),
    /// Always true
    True(std::marker::PhantomData<Y>),
    /// Always false
    False(std::marker::PhantomData<Y>),
}

impl<Y> Atomic<Y> {
    /// Creates an atomic predicate `signal_name < val`.
    pub fn new_less_than(signal_name: &'static str, val: f64) -> Self {
        Atomic::LessThan(signal_name, val, std::marker::PhantomData)
    }

    /// Creates an atomic predicate `signal_name > val`.
    pub fn new_greater_than(signal_name: &'static str, val: f64) -> Self {
        Atomic::GreaterThan(signal_name, val, std::marker::PhantomData)
    }

    /// Creates an atomic predicate `signal_name < $var_name`.
    ///
    /// The threshold is resolved from `vars` at evaluation time.
    pub fn new_less_than_var(
        signal_name: &'static str,
        var_name: &'static str,
        vars: Variables,
    ) -> Self {
        Atomic::LessThanVar(signal_name, var_name, vars, std::marker::PhantomData)
    }

    /// Creates an atomic predicate `signal_name > $var_name`.
    ///
    /// The threshold is resolved from `vars` at evaluation time.
    pub fn new_greater_than_var(
        signal_name: &'static str,
        var_name: &'static str,
        vars: Variables,
    ) -> Self {
        Atomic::GreaterThanVar(signal_name, var_name, vars, std::marker::PhantomData)
    }

    /// Creates a constant `True` atomic operator.
    pub fn new_true() -> Self {
        Atomic::True(std::marker::PhantomData)
    }

    /// Creates a constant `False` atomic operator.
    pub fn new_false() -> Self {
        Atomic::False(std::marker::PhantomData)
    }
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

        // filter by signal if this operator has specific signals (True/False have none)
        let signals = self.get_signal_identifiers();
        if !signals.is_empty() && !signals.contains(step.signal) {
            return vec![];
        }

        let result = match self {
            Atomic::True(_) => Y::atomic_true(),
            Atomic::False(_) => Y::atomic_false(),
            Atomic::GreaterThan(_signal_name, c, _) => Y::atomic_greater_than(value, *c),
            Atomic::LessThan(_signal_name, c, _) => Y::atomic_less_than(value, *c),
            Atomic::GreaterThanVar(_signal_name, var_name, vars, _) => {
                let c = vars
                    .get(var_name)
                    .unwrap_or_else(|| panic!("Variable '{}' not found in context", var_name));
                Y::atomic_greater_than(value, c)
            }
            Atomic::LessThanVar(_signal_name, var_name, vars, _) => {
                let c = vars
                    .get(var_name)
                    .unwrap_or_else(|| panic!("Variable '{}' not found in context", var_name));
                Y::atomic_less_than(value, c)
            }
        };

        vec![Step {
            signal: "output",
            value: result,
            timestamp: step.timestamp,
        }]
    }

    fn get_max_lookahead(&self) -> Duration {
        Duration::ZERO
    }
}

impl<Y> SignalIdentifier for Atomic<Y> {
    /// Returns the referenced signal for predicate variants.
    ///
    /// Constant variants (`True`/`False`) return an empty set.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        let mut ids = std::collections::HashSet::new();
        match self {
            Atomic::LessThan(signal_name, _, _)
            | Atomic::GreaterThan(signal_name, _, _)
            | Atomic::LessThanVar(signal_name, _, _, _)
            | Atomic::GreaterThanVar(signal_name, _, _, _) => {
                ids.insert(*signal_name);
            }
            Atomic::True(_) | Atomic::False(_) => {}
        }
        ids
    }
}

impl<Y> Display for Atomic<Y> {
    /// Formats the atomic operator using formula-like notation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Atomic::LessThan(signal_name, c, _) => write!(f, "{signal_name} < {c}"),
            Atomic::GreaterThan(signal_name, c, _) => write!(f, "{signal_name} > {c}"),
            Atomic::LessThanVar(signal_name, var_name, _, _) => {
                write!(f, "{signal_name} < ${var_name}")
            }
            Atomic::GreaterThanVar(signal_name, var_name, _, _) => {
                write!(f, "{signal_name} > ${var_name}")
            }
            Atomic::True(_) => write!(f, "True"),
            Atomic::False(_) => write!(f, "False"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_buffer::Step;
    use crate::stl::core::StlOperatorTrait;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn atomic_unexisting_identifier() {
        let mut atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        atomic.get_signal_identifiers();
        let step = Step::new("y", 15.0, Duration::from_secs(5));
        let robustness = atomic.update(&step);
        assert!(robustness.is_empty());
    }

    #[test]
    #[should_panic(expected = "Variable 'A' not found in context")]
    fn atomics_gt_variables_not_in_context() {
        let vars = Variables::new();
        let mut atomic = Atomic::<f64>::new_greater_than_var("x", "A", vars);
        atomic.get_signal_identifiers();
        let step = Step::new("x", 15.0, Duration::from_secs(5));
        atomic.update(&step);
    }

    #[test]
    #[should_panic(expected = "Variable 'A' not found in context")]
    fn atomics_lt_variables_not_in_context() {
        let vars = Variables::new();
        let mut atomic = Atomic::<f64>::new_less_than_var("x", "A", vars);
        atomic.get_signal_identifiers();
        let step = Step::new("x", 15.0, Duration::from_secs(5));
        atomic.update(&step);
    }

    #[test]
    fn atomic_greater_than_robustness() {
        let mut atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        atomic.get_signal_identifiers();
        let step1 = Step::new("x", 15.0, Duration::from_secs(5));
        let robustness = atomic.update(&step1);
        assert_eq!(
            robustness,
            vec![Step::new("output", 5.0, Duration::from_secs(5))]
        );

        let step2 = Step::new("x", 8.0, Duration::from_secs(6));
        let robustness2 = atomic.update(&step2);
        assert_eq!(
            robustness2,
            vec![Step::new("output", -2.0, Duration::from_secs(6))]
        );
    }

    #[test]
    fn atomic_less_than_robustness() {
        let mut atomic = Atomic::<f64>::new_less_than("x", 10.0);
        atomic.get_signal_identifiers();
        let step1 = Step::new("x", 5.0, Duration::from_secs(5));
        let robustness = atomic.update(&step1);
        assert_eq!(
            robustness,
            vec![Step::new("output", 5.0, Duration::from_secs(5))]
        );

        let step2 = Step::new("x", 12.0, Duration::from_secs(6));
        let robustness2 = atomic.update(&step2);
        assert_eq!(
            robustness2,
            vec![Step::new("output", -2.0, Duration::from_secs(6))]
        );
    }

    #[test]
    fn atomic_true_robustness() {
        let mut atomic = Atomic::<f64>::new_true();
        atomic.get_signal_identifiers();
        let step = Step::new("x", 0.0, Duration::from_secs(5));
        let robustness = atomic.update(&step);
        assert_eq!(
            robustness,
            vec![Step::new("output", f64::INFINITY, Duration::from_secs(5))]
        );
    }

    #[test]
    fn atomic_false_robustness() {
        let mut atomic = Atomic::<f64>::new_false();
        atomic.get_signal_identifiers();
        let step = Step::new("x", 0.0, Duration::from_secs(5));
        let robustness = atomic.update(&step);
        assert_eq!(
            robustness,
            vec![Step::new(
                "output",
                f64::NEG_INFINITY,
                Duration::from_secs(5)
            )]
        );
    }

    #[test]
    fn atomic_with_variables_robustness() {
        let vars = Variables::new();
        vars.set("A", 10.0);
        let mut atomic = Atomic::<f64>::new_greater_than_var("x", "A", vars);
        atomic.get_signal_identifiers();

        let step1 = Step::new("x", 15.0, Duration::from_secs(5));
        let robustness = atomic.update(&step1);
        assert_eq!(
            robustness,
            vec![Step::new("output", 5.0, Duration::from_secs(5))]
        );

        let vars = Variables::new();
        vars.set("A", 10.0);
        let mut atomic = Atomic::<f64>::new_less_than_var("x", "A", vars);
        atomic.get_signal_identifiers();

        let step1 = Step::new("x", 15.0, Duration::from_secs(5));
        let robustness = atomic.update(&step1);
        assert_eq!(
            robustness,
            vec![Step::new("output", -5.0, Duration::from_secs(5))]
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
