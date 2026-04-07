//! Naive STL operator backend.
//!
//! This module provides a straightforward recursive evaluator used mainly for:
//! - correctness comparison against the incremental engine, and
//! - tests/debugging.
//!
//! ## Recommendation
//! This backend is **not recommended** for production monitoring workloads.
//! While it may use slightly less memory in some scenarios, it is typically
//! significantly slower, especially for temporal operators (`G`, `F`, `U`).
//!
//! ## Limitations
//! - No RoSI (`RobustnessInterval`) mode in the monitor builder.
//! - No eager short-circuit evaluation mode (only delayed-style evaluation).

use crate::ring_buffer::RingBufferTrait;
use crate::ring_buffer::Step;
use crate::stl::core::{RobustnessSemantics, SignalIdentifier, StlOperatorTrait, TimeInterval};
use std::fmt::Display;
use std::time::Duration;

/// Lightweight AST/operator representation used by the naive evaluator.
#[derive(Debug, Clone)]
pub enum StlOperator {
    // Boolean operators
    Not(Box<StlOperator>),
    And(Box<StlOperator>, Box<StlOperator>),
    Or(Box<StlOperator>, Box<StlOperator>),

    // Temporal operators
    Globally(TimeInterval, Box<StlOperator>),
    Eventually(TimeInterval, Box<StlOperator>),
    Until(TimeInterval, Box<StlOperator>, Box<StlOperator>),

    // logical operators
    Implies(Box<StlOperator>, Box<StlOperator>),

    // Atomic propositions
    True,
    False,
    GreaterThan(&'static str, f64), // signal name, threshold
    LessThan(&'static str, f64),    // signal name, threshold
}

/// Stateful wrapper around [`StlOperator`] for stream-based naive evaluation.
#[derive(Debug, Clone)]
pub struct StlFormula<T, C, Y>
where
    T: 'static,
    C: RingBufferTrait<Value = T> + 'static,
    Y: RobustnessSemantics + 'static,
{
    formula: StlOperator,
    signal: C,
    last_eval_time: Option<Duration>,
    _phantom: std::marker::PhantomData<Y>,
}

impl<T, C, Y> StlFormula<T, C, Y>
where
    T: 'static,
    C: RingBufferTrait<Value = T> + 'static,
    Y: RobustnessSemantics + 'static,
{
    /// Creates a new naive formula evaluator with an external signal buffer.
    pub fn new(formula: StlOperator, signal: C) -> Self {
        Self {
            formula,
            signal,
            last_eval_time: None,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T, C, Y> Display for StlFormula<T, C, Y>
where
    Y: RobustnessSemantics,
    C: RingBufferTrait<Value = T>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.formula)
    }
}

impl<T, C, Y> StlOperatorTrait<T> for StlFormula<T, C, Y>
where
    T: Clone + Copy + Into<f64> + 'static,
    C: RingBufferTrait<Value = T> + Clone + 'static,
    Y: RobustnessSemantics + 'static,
{
    type Output = Y;

    fn get_max_lookahead(&self) -> Duration {
        self.formula.get_max_lookahead()
    }

    /// Ingests one sample and evaluates at the oldest newly-finalizable timestamp.
    ///
    /// The naive engine performs recursive evaluation over buffered samples and
    /// returns at most one output step per call.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>> {
        self.signal.add_step(step.clone());

        let max_lookahead = self.formula.get_max_lookahead();
        if step.timestamp < max_lookahead {
            return vec![];
        }

        let t_eval = step.timestamp.saturating_sub(max_lookahead);

        if self.last_eval_time == Some(t_eval) {
            return vec![]; // We already evaluated this t_eval, triggered by the previous step.
        }

        let robustness = self.formula.robustness_naive(&self.signal, t_eval);

        if robustness.is_some() {
            self.last_eval_time = Some(t_eval);
        }

        self.signal.prune(max_lookahead);

        match robustness {
            Some(robustness_step) => {
                vec![Step::new(
                    "output",
                    robustness_step.value,
                    robustness_step.timestamp,
                )]
            }
            None => vec![],
        }
    }
}

impl StlOperator {
    /// Computes the robustness of the formula at a specific time `t_eval`.
    /// This function assumes all necessary signal data (up to `t_eval + max_lookahead`) is present.
    fn robustness_naive<T, C, Y>(
        &self,
        // FIX: Remove signal_name
        signal: &C,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        C: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        match self {
            StlOperator::True => self.eval_true(t_eval),
            StlOperator::False => self.eval_false(t_eval),
            StlOperator::GreaterThan(name, c) => self.eval_greater_than(*c, name, signal, t_eval),
            StlOperator::LessThan(name, c) => self.eval_less_than(*c, name, signal, t_eval),
            StlOperator::Not(phi) => self.eval_not(phi, signal, t_eval),
            StlOperator::And(phi, psi) => self.eval_and(phi, psi, signal, t_eval),
            StlOperator::Or(phi, psi) => self.eval_or(phi, psi, signal, t_eval),
            StlOperator::Implies(phi, psi) => self.eval_implies(phi, psi, signal, t_eval),
            StlOperator::Eventually(interval, phi) => {
                self.eval_eventually(interval, phi, signal, t_eval)
            }
            StlOperator::Globally(interval, phi) => {
                self.eval_globally(interval, phi, signal, t_eval)
            }
            StlOperator::Until(interval, phi, psi) => {
                self.eval_until(interval, phi, psi, signal, t_eval)
            }
        }
    }
    /// Recursively computes the maximum lookahead required by this formula.
    fn get_max_lookahead(&self) -> Duration {
        match self {
            StlOperator::Globally(interval, f) | StlOperator::Eventually(interval, f) => {
                interval.end + f.get_max_lookahead()
            }
            StlOperator::Until(interval, f1, f2) => {
                interval.end + f1.get_max_lookahead().max(f2.get_max_lookahead())
            }
            StlOperator::Not(f) => f.get_max_lookahead(),
            StlOperator::And(f1, f2) | StlOperator::Or(f1, f2) | StlOperator::Implies(f1, f2) => {
                f1.get_max_lookahead().max(f2.get_max_lookahead())
            }
            StlOperator::True
            | StlOperator::False
            | StlOperator::GreaterThan(_, _)
            | StlOperator::LessThan(_, _) => Duration::ZERO,
        }
    }

    fn eval_true<Y: RobustnessSemantics>(&self, t_eval: Duration) -> Option<Step<Y>> {
        Some(Step::new("output", Y::atomic_true(), t_eval))
    }

    fn eval_false<Y: RobustnessSemantics>(&self, t_eval: Duration) -> Option<Step<Y>> {
        Some(Step::new("output", Y::atomic_false(), t_eval))
    }

    fn eval_greater_than<T, S, Y>(
        &self,
        c: f64,
        name: &'static str,
        signal: &S,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        signal
            .iter()
            .find(|s| s.timestamp == t_eval && s.signal == name)
            .map(|step| {
                Step::new(
                    "output",
                    Y::atomic_greater_than(step.value.into(), c),
                    t_eval,
                )
            })
    }

    fn eval_less_than<T, S, Y>(
        &self,
        c: f64,
        name: &'static str,
        signal: &S,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        signal
            .iter()
            .find(|s| s.timestamp == t_eval && s.signal == name)
            .map(|step| Step::new("output", Y::atomic_less_than(step.value.into(), c), t_eval))
    }

    fn eval_not<T, S, Y>(&self, phi: &StlOperator, signal: &S, t_eval: Duration) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        // Recursively call for the same t_eval
        phi.robustness_naive(signal, t_eval)
            .map(|step| Step::new("output", Y::not(step.value), step.timestamp))
    }

    fn eval_and<T, S, Y>(
        &self,
        phi: &StlOperator,
        psi: &StlOperator,
        signal: &S,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        // Recursively call both children for the same t_eval
        phi.robustness_naive(signal, t_eval)
            .zip(psi.robustness_naive(signal, t_eval))
            .map(|(step1, step2)| {
                Step::new("output", Y::and(step1.value, step2.value), step1.timestamp)
            })
    }

    fn eval_or<T, S, Y>(
        &self,
        phi: &StlOperator,
        psi: &StlOperator,
        signal: &S,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        phi.robustness_naive(signal, t_eval)
            .zip(psi.robustness_naive(signal, t_eval))
            .map(|(step1, step2)| {
                Step::new("output", Y::or(step1.value, step2.value), step1.timestamp)
            })
    }

    fn eval_implies<T, S, Y>(
        &self,
        phi: &StlOperator,
        psi: &StlOperator,
        signal: &S,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        phi.robustness_naive(signal, t_eval)
            .zip(psi.robustness_naive(signal, t_eval))
            .map(|(step1, step2)| {
                Step::new(
                    "output",
                    Y::implies(step1.value, step2.value),
                    step1.timestamp,
                )
            })
    }

    fn eval_eventually<T, S, Y>(
        &self,
        interval: &TimeInterval,
        phi: &StlOperator,
        signal: &S,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        // We are evaluating for t_eval. We need data in the window [t_eval+a, t_eval+b]
        let lower_bound_t_prime = t_eval + interval.start;
        let upper_bound_t_prime = t_eval + interval.end;

        // Check if we have enough data in the signal
        if let Some(back_step) = signal.get_back() {
            if back_step.timestamp < upper_bound_t_prime {
                return None; // Not enough data to evaluate yet
            }
        } else {
            return None; // Signal is empty
        }

        let result = signal
            .iter()
            .filter(|step| {
                step.timestamp >= lower_bound_t_prime && step.timestamp <= upper_bound_t_prime
            })
            // Recursively call for each step's timestamp in the window
            .map(|step| phi.robustness_naive(signal, step.timestamp))
            .try_fold(Y::eventually_identity(), |acc, item| {
                item.map(|step| Y::or(acc, step.value))
            })?;

        // Return the result for the original t_eval
        Some(Step::new("output", result, t_eval))
    }

    fn eval_globally<T, S, Y>(
        &self,
        interval: &TimeInterval,
        phi: &StlOperator,
        signal: &S,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        let lower_bound_t_prime = t_eval + interval.start;
        let upper_bound_t_prime = t_eval + interval.end;

        if let Some(back_step) = signal.get_back() {
            if back_step.timestamp < upper_bound_t_prime {
                return None;
            }
        } else {
            return None;
        }

        let result = signal
            .iter()
            .filter(|step| {
                step.timestamp >= lower_bound_t_prime && step.timestamp <= upper_bound_t_prime
            })
            .map(|step| phi.robustness_naive(signal, step.timestamp))
            .try_fold(Y::globally_identity(), |acc, item| {
                item.map(|step| Y::and(acc, step.value))
            })?;

        Some(Step::new("output", result, t_eval))
    }

    fn eval_until<T, S, Y>(
        &self,
        interval: &TimeInterval,
        phi: &StlOperator,
        psi: &StlOperator,
        signal: &S,
        t_eval: Duration,
    ) -> Option<Step<Y>>
    where
        S: RingBufferTrait<Value = T>,
        T: Clone + Copy + Into<f64>,
        Y: RobustnessSemantics,
    {
        let lower_bound_t_prime = t_eval + interval.start;
        let upper_bound_t_prime = t_eval + interval.end;

        if let Some(back_step) = signal.get_back() {
            if back_step.timestamp < upper_bound_t_prime {
                return None;
            }
        } else {
            return None;
        }

        let result = signal
            .iter()
            .filter(|step| {
                step.timestamp >= lower_bound_t_prime && step.timestamp <= upper_bound_t_prime
            })
            .map(|step_t_prime| {
                let t_prime = step_t_prime.timestamp;
                let robustness_psi = psi.robustness_naive(signal, t_prime);

                let robustness_phi_g = signal
                    .iter()
                    .filter(|s| s.timestamp >= lower_bound_t_prime && s.timestamp < t_prime) // G is up to t_prime
                    .map(|s| phi.robustness_naive(signal, s.timestamp))
                    .try_fold(Y::globally_identity(), |acc, item| {
                        item.map(|step| Y::and(acc, step.value))
                    });

                robustness_psi
                    .zip(robustness_phi_g)
                    .map(|(r_psi, r_phi_val)| Y::and(r_psi.value, r_phi_val))
            })
            .try_fold(Y::eventually_identity(), |acc, item| {
                // item is Option<Y>
                item.map(|robustness_value| Y::or(acc, robustness_value))
            })?;

        Some(Step::new("output", result, t_eval))
    }

    /// Recursively generates a tree-like string representation of the formula.
    pub fn to_tree_string(&self, indent: usize) -> String {
        let padding = " ".repeat(indent);
        match self {
            StlOperator::True => format!("{padding}True"),
            StlOperator::False => format!("{padding}False"),
            StlOperator::Not(f) => format!("{}Not\n{}", padding, f.to_tree_string(indent + 2)),
            StlOperator::And(f1, f2) => format!(
                "{}And\n{}\n{}",
                padding,
                f1.to_tree_string(indent + 2),
                f2.to_tree_string(indent + 2)
            ),
            StlOperator::Or(f1, f2) => format!(
                "{}Or\n{}\n{}",
                padding,
                f1.to_tree_string(indent + 2),
                f2.to_tree_string(indent + 2)
            ),
            StlOperator::Globally(interval, f) => format!(
                "{}Always [{} - {}]\n{}",
                padding,
                interval.start.as_secs_f64(),
                interval.end.as_secs_f64(),
                f.to_tree_string(indent + 2)
            ),
            StlOperator::Eventually(interval, f) => format!(
                "{}Eventually [{} - {}]\n{}",
                padding,
                interval.start.as_secs_f64(),
                interval.end.as_secs_f64(),
                f.to_tree_string(indent + 2)
            ),
            StlOperator::Until(interval, f1, f2) => format!(
                "{}Until [{} - {}]\n{}\n{}",
                padding,
                interval.start.as_secs_f64(),
                interval.end.as_secs_f64(),
                f1.to_tree_string(indent + 2),
                f2.to_tree_string(indent + 2)
            ),
            StlOperator::Implies(f1, f2) => format!(
                "{}Implies\n{}\n{}",
                padding,
                f1.to_tree_string(indent + 2),
                f2.to_tree_string(indent + 2)
            ),
            StlOperator::GreaterThan(s, val) => format!("{padding}{s} > {val}"),
            StlOperator::LessThan(s, val) => format!("{padding}{s} < {val}"),
        }
    }
}
impl Display for StlOperator {
    /// Formats the formula in compact mathematical notation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                StlOperator::True => "True".to_string(),
                StlOperator::False => "False".to_string(),
                StlOperator::Not(f) => format!("¬({f})"),
                StlOperator::And(f1, f2) => format!("({f1}) ∧ ({f2})"),
                StlOperator::Or(f1, f2) => format!("({f1}) v ({f2})"),
                StlOperator::Globally(interval, f) => format!(
                    "G[{}, {}]({})",
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64(),
                    f
                ),
                StlOperator::Eventually(interval, f) => format!(
                    "F[{}, {}]({})",
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64(),
                    f
                ),
                StlOperator::Until(interval, f1, f2) => format!(
                    "({}) U[{}, {}] ({})",
                    f1,
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64(),
                    f2
                ),
                StlOperator::Implies(f1, f2) => format!("({f1}) → ({f2})"),
                StlOperator::GreaterThan(s, val) => format!("{s} > {val}"),
                StlOperator::LessThan(s, val) => format!("{s} < {val}"),
            }
        )
    }
}

impl<T, C, Y> SignalIdentifier for StlFormula<T, C, Y>
where
    T: 'static,
    C: RingBufferTrait<Value = T> + 'static,
    Y: RobustnessSemantics + 'static,
{
    /// Collects all signal identifiers referenced by the formula tree.
    fn get_signal_identifiers(&mut self) -> std::collections::HashSet<&'static str> {
        let mut signals = std::collections::HashSet::new();
        fn collect_signals(
            node: &StlOperator,
            signals: &mut std::collections::HashSet<&'static str>,
        ) {
            match node {
                StlOperator::GreaterThan(s, _) | StlOperator::LessThan(s, _) => {
                    signals.insert(*s);
                }
                StlOperator::True | StlOperator::False => {}
                StlOperator::Not(f) => {
                    collect_signals(f, signals);
                }
                StlOperator::And(f1, f2)
                | StlOperator::Or(f1, f2)
                | StlOperator::Implies(f1, f2) => {
                    collect_signals(f1, signals);
                    collect_signals(f2, signals);
                }
                StlOperator::Globally(_, f) | StlOperator::Eventually(_, f) => {
                    collect_signals(f, signals);
                }
                StlOperator::Until(_, f1, f2) => {
                    collect_signals(f1, signals);
                    collect_signals(f2, signals);
                }
            }
        }
        collect_signals(&self.formula, &mut signals);
        signals
    }
}

#[cfg(test)]
mod tests {

    mod stl_formula_tests {
        use super::*;

        #[test]
        fn test_get_max_lookahead() {
            let formula: StlFormula<f64, RingBuffer<f64>, f64> = StlFormula::new(
                StlOperator::Globally(
                    TimeInterval {
                        start: Duration::from_secs(1),
                        end: Duration::from_secs(2),
                    },
                    Box::new(StlOperator::False),
                ),
                RingBuffer::new(),
            );

            assert_eq!(formula.get_max_lookahead(), Duration::from_secs(2));
        }
    }

    use super::*;
    use crate::{
        ring_buffer::{RingBuffer, Step},
        stl::naive_operators::StlFormula,
    };
    use std::time::Duration;

    // Helper to create a signal from a vector of values and timestamps
    fn create_signal(values: Vec<f64>, timestamps: Vec<u64>) -> RingBuffer<f64> {
        let mut signal = RingBuffer::new();
        for (val, ts) in values.into_iter().zip(timestamps.into_iter()) {
            signal.add_step(Step::new("x", val, Duration::from_secs(ts)));
        }
        signal
    }

    #[test]
    fn atomic_greater_than_robustness() {
        let formula = StlOperator::GreaterThan("x", 10.0);

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "x > 10");

        let signal = create_signal(vec![15.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new("output", 5.0, Duration::from_secs(5)))
        );

        let signal2 = create_signal(vec![8.0], vec![5]);
        let robustness2 = formula.robustness_naive::<f64, _, f64>(&signal2, Duration::from_secs(5));
        assert_eq!(
            robustness2,
            Some(Step::new("output", -2.0, Duration::from_secs(5)))
        );
    }

    #[test]
    fn atomic_less_than_robustness() {
        let formula = StlOperator::LessThan("x", 10.0);

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "x < 10");

        let signal = create_signal(vec![5.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new("output", 5.0, Duration::from_secs(5)))
        );

        let signal2 = create_signal(vec![12.0], vec![5]);
        let robustness2 = formula.robustness_naive::<f64, _, f64>(&signal2, Duration::from_secs(5));
        assert_eq!(
            robustness2,
            Some(Step::new("output", -2.0, Duration::from_secs(5)))
        );
    }

    #[test]
    fn atomic_true_robustness() {
        let formula = StlOperator::True;

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "True");

        let signal = create_signal(vec![0.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new("output", f64::INFINITY, Duration::from_secs(5)))
        );
    }

    #[test]
    fn atomic_false_robustness() {
        let formula = StlOperator::False;

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "False");

        let signal = create_signal(vec![0.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new(
                "output",
                f64::NEG_INFINITY,
                Duration::from_secs(5)
            ))
        );
    }

    #[test]
    fn not_operator_robustness() {
        let formula = StlOperator::Not(Box::new(StlOperator::GreaterThan("x", 10.0)));

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "Not\n  x > 10");

        let signal = create_signal(vec![15.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new("output", -5.0, Duration::from_secs(5)))
        );
    }

    #[test]
    fn and_operator_robustness() {
        let formula = StlOperator::And(
            Box::new(StlOperator::GreaterThan("x", 10.0)),
            Box::new(StlOperator::LessThan("x", 20.0)),
        );

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "And\n  x > 10\n  x < 20");

        let signal = create_signal(vec![15.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new("output", 5.0, Duration::from_secs(5)))
        );
    }

    #[test]
    fn or_operator_robustness() {
        let formula = StlOperator::Or(
            Box::new(StlOperator::GreaterThan("x", 10.0)),
            Box::new(StlOperator::LessThan("x", 5.0)),
        );

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "Or\n  x > 10\n  x < 5");
        let signal = create_signal(vec![15.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new("output", 5.0, Duration::from_secs(5)))
        );
    }

    #[test]
    fn implies_operator_robustness() {
        let formula = StlOperator::Implies(
            Box::new(StlOperator::GreaterThan("x", 10.0)),
            Box::new(StlOperator::LessThan("x", 20.0)),
        );

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "Implies\n  x > 10\n  x < 20");

        let signal = create_signal(vec![15.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new("output", 5.0, Duration::from_secs(5)))
        );
    }

    #[test]
    fn eventually_operator_robustness() {
        let formula = StlOperator::Eventually(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(4),
            },
            Box::new(StlOperator::GreaterThan("x", 10.0)),
        );

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "Eventually [0 - 4]\n  x > 10");

        // Case 1: Not enough data
        let signal1 = create_signal(vec![15.0, 12.0], vec![0, 2]); // Max timestamp is 2, need up to 4
        let robustness1 = formula.robustness_naive::<f64, _, f64>(&signal1, Duration::from_secs(0));
        assert_eq!(robustness1, None);
        // Case 2: Just enough data for t_eval=0
        let signal2 = create_signal(vec![15.0, 12.0, 8.0], vec![0, 2, 4]); // Max timestamp 4
        // eval at t=0, window [0,4], values are 15, 12, 8. Robustness values: 5, 2, -2. Max is 5.
        let robustness2 = formula.robustness_naive::<f64, _, f64>(&signal2, Duration::from_secs(0));
        assert_eq!(
            robustness2,
            Some(Step::new("output", 5.0, Duration::from_secs(0)))
        );

        // Case 3: More data, eval at t=2
        let signal3 = create_signal(vec![15.0, 12.0, 8.0, 5.0], vec![0, 2, 4, 6]); // Max timestamp 6
        // eval at t=2, window [2,6], values are 12, 8, 5. Robustness values: 2, -2, -5. Max is 2.
        let robustness3 = formula.robustness_naive::<f64, _, f64>(&signal3, Duration::from_secs(2));
        assert_eq!(
            robustness3,
            Some(Step::new("output", 2.0, Duration::from_secs(2)))
        );

        // Case 4: Full signal, eval at t=4
        let signal4 = create_signal(vec![15.0, 12.0, 8.0, 5.0, 12.0], vec![0, 2, 4, 6, 8]); // Max timestamp 8
        // eval at t=4, window [4,8], values are 8, 5, 12. Robustness values: -2, -5, 2. Max is 2.
        let robustness4 = formula.robustness_naive::<f64, _, f64>(&signal4, Duration::from_secs(4));
        assert_eq!(
            robustness4,
            Some(Step::new("output", 2.0, Duration::from_secs(4)))
        );
    }

    #[test]
    fn globally_operator_robustness() {
        let formula = StlOperator::Globally(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(4),
            },
            Box::new(StlOperator::GreaterThan("x", 10.0)),
        );

        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "Always [0 - 4]\n  x > 10");

        // Case 1: Not enough data
        let signal1 = create_signal(vec![15.0, 12.0], vec![0, 2]); // Max timestamp is 2, need up to 4
        let robustness1 = formula.robustness_naive::<f64, _, f64>(&signal1, Duration::from_secs(0));
        assert_eq!(robustness1, None);

        // Case 2: Just enough data for t_eval=0
        let signal2 = create_signal(vec![15.0, 12.0, 8.0], vec![0, 2, 4]); // Max timestamp 4
        // eval at t=0, window [0,4], values are 15, 12, 8. Robustness values: 5, 2, -2. Min is -2.
        let robustness2 = formula.robustness_naive::<f64, _, f64>(&signal2, Duration::from_secs(0));
        assert_eq!(
            robustness2,
            Some(Step::new("output", -2.0, Duration::from_secs(0)))
        );

        // Case 3: More data, eval at t=2
        let signal3 = create_signal(vec![15.0, 12.0, 8.0, 5.0], vec![0, 2, 4, 6]); // Max timestamp 6
        // eval at t=2, window [2,6], values are 12, 8, 5. Robustness values: 2, -2, -5. Min is -5.
        let robustness3 = formula.robustness_naive::<f64, _, f64>(&signal3, Duration::from_secs(2));
        assert_eq!(
            robustness3,
            Some(Step::new("output", -5.0, Duration::from_secs(2)))
        );

        // Case 4: Full signal, eval at t=4
        let signal4 = create_signal(vec![15.0, 12.0, 8.0, 5.0, 12.0], vec![0, 2, 4, 6, 8]); // Max timestamp 8
        // eval at t=4, window [4,8], values are 8, 5, 12. Robustness values: -2, -5, 2. Min is -5.
        let robustness4 = formula.robustness_naive::<f64, _, f64>(&signal4, Duration::from_secs(4));
        assert_eq!(
            robustness4,
            Some(Step::new("output", -5.0, Duration::from_secs(4)))
        );
    }

    #[test]
    fn until_operator_robustness() {
        let formula = StlOperator::Until(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(4),
            },
            Box::new(StlOperator::GreaterThan("x", 0.0)),
            Box::new(StlOperator::GreaterThan("x", 10.0)),
        );

        // Case 1: Not enough data
        let signal1 = create_signal(vec![5.0, 8.0], vec![0, 2]);
        let robustness1 = formula.robustness_naive::<f64, _, f64>(&signal1, Duration::from_secs(0));
        assert_eq!(robustness1, None);

        // Case 2: Enough data - psi satisfied at t=2 (value=12 > 10)
        let signal2 = create_signal(vec![5.0, 12.0, 3.0], vec![0, 2, 4]);
        let robustness2 = formula.robustness_naive::<f64, _, f64>(&signal2, Duration::from_secs(0));
        assert!(robustness2.is_some());
    }

    #[test]
    fn implies_operator_tree_string() {
        let formula = StlOperator::Implies(
            Box::new(StlOperator::GreaterThan("x", 5.0)),
            Box::new(StlOperator::LessThan("y", 10.0)),
        );
        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "Implies\n  x > 5\n  y < 10");
    }

    #[test]
    fn until_operator_tree_string() {
        let formula = StlOperator::Until(
            TimeInterval {
                start: Duration::from_secs(1),
                end: Duration::from_secs(5),
            },
            Box::new(StlOperator::GreaterThan("x", 0.0)),
            Box::new(StlOperator::LessThan("y", 10.0)),
        );
        let tree_string = formula.to_tree_string(0);
        assert_eq!(tree_string, "Until [1 - 5]\n  x > 0\n  y < 10");
    }

    #[test]
    fn display_impl_all_operators() {
        let gt = StlOperator::GreaterThan("x", 5.0);
        assert_eq!(format!("{gt}"), "x > 5");

        let lt = StlOperator::LessThan("y", 3.0);
        assert_eq!(format!("{lt}"), "y < 3");

        let t = StlOperator::True;
        assert_eq!(format!("{t}"), "True");

        let f = StlOperator::False;
        assert_eq!(format!("{f}"), "False");

        let not = StlOperator::Not(Box::new(StlOperator::GreaterThan("x", 1.0)));
        assert_eq!(format!("{not}"), "¬(x > 1)");

        let and = StlOperator::And(
            Box::new(StlOperator::GreaterThan("x", 1.0)),
            Box::new(StlOperator::LessThan("y", 2.0)),
        );
        assert_eq!(format!("{and}"), "(x > 1) ∧ (y < 2)");

        let or = StlOperator::Or(
            Box::new(StlOperator::GreaterThan("x", 1.0)),
            Box::new(StlOperator::LessThan("y", 2.0)),
        );
        assert_eq!(format!("{or}"), "(x > 1) v (y < 2)");

        let globally = StlOperator::Globally(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(10),
            },
            Box::new(StlOperator::GreaterThan("x", 5.0)),
        );
        assert_eq!(format!("{globally}"), "G[0, 10](x > 5)");

        let eventually = StlOperator::Eventually(
            TimeInterval {
                start: Duration::from_secs(1),
                end: Duration::from_secs(5),
            },
            Box::new(StlOperator::LessThan("y", 3.0)),
        );
        assert_eq!(format!("{eventually}"), "F[1, 5](y < 3)");

        let until = StlOperator::Until(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(10),
            },
            Box::new(StlOperator::GreaterThan("x", 1.0)),
            Box::new(StlOperator::LessThan("y", 2.0)),
        );
        assert_eq!(format!("{until}"), "(x > 1) U[0, 10] (y < 2)");

        let implies = StlOperator::Implies(
            Box::new(StlOperator::GreaterThan("x", 1.0)),
            Box::new(StlOperator::LessThan("y", 2.0)),
        );
        assert_eq!(format!("{implies}"), "(x > 1) → (y < 2)");
    }

    #[test]
    fn get_signal_identifiers_naive() {
        let mut formula: StlFormula<f64, RingBuffer<f64>, f64> = StlFormula::new(
            StlOperator::And(
                Box::new(StlOperator::GreaterThan("x", 5.0)),
                Box::new(StlOperator::Until(
                    TimeInterval {
                        start: Duration::from_secs(0),
                        end: Duration::from_secs(5),
                    },
                    Box::new(StlOperator::LessThan("y", 10.0)),
                    Box::new(StlOperator::Not(Box::new(StlOperator::GreaterThan(
                        "z", 1.0,
                    )))),
                )),
            ),
            RingBuffer::new(),
        );

        let ids = formula.get_signal_identifiers();
        assert!(ids.contains("x"));
        assert!(ids.contains("y"));
        assert!(ids.contains("z"));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn eval_naive_full_pipeline() {
        // Test the full StlFormula eval_naive pipeline with Eventually
        let mut formula: StlFormula<f64, RingBuffer<f64>, f64> = StlFormula::new(
            StlOperator::Eventually(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(2),
                },
                Box::new(StlOperator::GreaterThan("x", 10.0)),
            ),
            RingBuffer::new(),
        );

        // Not enough data yet (timestamp 0 < max_lookahead 2)
        let step1 = Step::new("x", 15.0, Duration::from_secs(0));
        let result1 = formula.update(&step1);
        assert!(result1.is_empty());

        let step2 = Step::new("x", 12.0, Duration::from_secs(1));
        let result2 = formula.update(&step2);
        assert!(result2.is_empty());

        // Now t=2, t_eval = 2-2 = 0
        let step3 = Step::new("x", 8.0, Duration::from_secs(2));
        let result3 = formula.update(&step3);
        assert_eq!(result3.len(), 1);
        assert_eq!(result3[0].timestamp, Duration::from_secs(0));
    }

    #[test]
    fn eval_naive_display_formula() {
        let formula: StlFormula<f64, RingBuffer<f64>, f64> = StlFormula::new(
            StlOperator::Globally(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(5),
                },
                Box::new(StlOperator::GreaterThan("x", 10.0)),
            ),
            RingBuffer::new(),
        );

        let display = format!("{formula}");
        assert_eq!(display, "G[0, 5](x > 10)");
    }

    #[test]
    fn eval_naive_empty_signal_eventually() {
        // Test evaluation of eventually with empty signal
        let formula = StlOperator::Eventually(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(2),
            },
            Box::new(StlOperator::GreaterThan("x", 10.0)),
        );
        let signal: RingBuffer<f64> = RingBuffer::new();
        let result = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(0));
        assert_eq!(result, None);
    }

    #[test]
    fn eval_naive_empty_signal_globally() {
        let formula = StlOperator::Globally(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(2),
            },
            Box::new(StlOperator::GreaterThan("x", 10.0)),
        );
        let signal: RingBuffer<f64> = RingBuffer::new();
        let result = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(0));
        assert_eq!(result, None);
    }

    #[test]
    fn eval_naive_empty_signal_until() {
        let formula = StlOperator::Until(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(2),
            },
            Box::new(StlOperator::GreaterThan("x", 0.0)),
            Box::new(StlOperator::GreaterThan("x", 10.0)),
        );
        let signal: RingBuffer<f64> = RingBuffer::new();
        let result = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(0));
        assert_eq!(result, None);
    }

    #[test]
    fn less_than_robustness_via_naive() {
        // Ensure LessThan is exercised through robustness_naive (line 216)
        let formula = StlOperator::LessThan("x", 10.0);
        let signal = create_signal(vec![7.0], vec![5]);
        let robustness = formula.robustness_naive::<f64, _, f64>(&signal, Duration::from_secs(5));
        assert_eq!(
            robustness,
            Some(Step::new("output", 3.0, Duration::from_secs(5)))
        );
    }

    #[test]
    fn get_signal_identifiers_implies_or_eventually_globally() {
        let mut formula: StlFormula<f64, RingBuffer<f64>, f64> = StlFormula::new(
            StlOperator::Implies(
                Box::new(StlOperator::Or(
                    Box::new(StlOperator::GreaterThan("a", 1.0)),
                    Box::new(StlOperator::Globally(
                        TimeInterval {
                            start: Duration::from_secs(0),
                            end: Duration::from_secs(1),
                        },
                        Box::new(StlOperator::LessThan("b", 2.0)),
                    )),
                )),
                Box::new(StlOperator::Eventually(
                    TimeInterval {
                        start: Duration::from_secs(0),
                        end: Duration::from_secs(1),
                    },
                    Box::new(StlOperator::GreaterThan("c", 3.0)),
                )),
            ),
            RingBuffer::new(),
        );

        let ids = formula.get_signal_identifiers();
        assert!(ids.contains("a"));
        assert!(ids.contains("b"));
        assert!(ids.contains("c"));
        assert_eq!(ids.len(), 3);
    }
}
