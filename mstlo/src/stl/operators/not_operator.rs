//! Unary logical negation operator.
//!
//! This module provides [`Not`], which wraps a single child operator and applies
//! [`RobustnessSemantics::not`] to each emitted value.

use crate::ring_buffer::Step;
use crate::stl::core::{
    RobustnessSemantics, SignalIdentifier, StlOperatorAndSignalIdentifier, StlOperatorTrait,
};
use std::collections::HashSet;
use std::fmt::Display;
use std::time::Duration;

#[derive(Clone)]
/// STL negation operator `¬φ`.
pub struct Not<T, Y> {
    operand: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
    max_lookahead: Duration,
}

impl<T, Y> Not<T, Y> {
    /// Creates a new negation operator from a child operand.
    ///
    /// The resulting lookahead equals the child's lookahead.
    pub fn new(operand: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>) -> Self
    where
        T: Clone + 'static,
        Y: RobustnessSemantics + 'static,
    {
        let max_lookahead = operand.get_max_lookahead();
        Not {
            operand,
            max_lookahead,
        }
    }
}

impl<T, Y> StlOperatorTrait<T> for Not<T, Y>
where
    T: Clone + 'static,
    Y: RobustnessSemantics + 'static,
{
    type Output = Y;

    fn get_max_lookahead(&self) -> Duration {
        self.max_lookahead
    }

    /// Updates the child operator and negates each produced value.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>> {
        let operand_updates = self.operand.update(step);

        let output_robustness: Vec<Step<Y>> = operand_updates
            .into_iter()
            .map(|step| {
                let negated_value = Y::not(step.value);
                Step {
                    signal: "output",
                    value: negated_value,
                    timestamp: step.timestamp,
                }
            })
            .collect();

        output_robustness
    }
}

impl<T, Y> SignalIdentifier for Not<T, Y> {
    /// Returns the signal identifiers of the wrapped operand.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        self.operand.get_signal_identifiers()
    }
}

impl<T, Y> Display for Not<T, Y> {
    /// Formats as `¬(operand)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "¬({})", self.operand)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_buffer::Step;
    use crate::stl::core::StlOperatorTrait;
    use crate::stl::operators::atomic_operators::Atomic;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    // Logical operators
    #[test]
    fn not_operator_robustness() {
        let atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        let mut not = Not::new(Box::new(atomic));
        not.get_signal_identifiers();
        let step = Step::new("x", 15.0, Duration::from_secs(5));
        let robustness = not.update(&step);
        assert_eq!(
            robustness,
            vec![Step::new("output", -5.0, Duration::from_secs(5))]
        );
    }

    #[test]
    fn not_signal_identifiers() {
        let atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        let mut not: Not<f64, f64> = Not::new(Box::new(atomic));
        let ids = not.get_signal_identifiers();
        let expected_ids: HashSet<&'static str> = vec!["x"].into_iter().collect();
        assert_eq!(ids, expected_ids);
    }

    #[test]
    fn not_display() {
        let atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        let not: Not<f64, f64> = Not::new(Box::new(atomic));
        let display_str = format!("{}", not);
        assert_eq!(display_str, "¬(x > 10)");
    }
}
