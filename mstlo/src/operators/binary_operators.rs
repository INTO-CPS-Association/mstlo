//! Binary STL operators (`And`, `Or`).
//!
//! This module combines two child operators while supporting three execution
//! modes through const generics:
//! - delayed (`IS_EAGER = false`, `IS_ROSI = false`),
//! - eager short-circuiting (`IS_EAGER = true`, `IS_ROSI = false`), and
//! - refinable interval semantics (`IS_ROSI = true`).

use crate::core::{
    RobustnessSemantics, SignalIdentifier, StlOperatorAndSignalIdentifier, StlOperatorTrait,
};
use crate::ring_buffer::{RingBufferTrait, Step, guarded_prune};
use std::collections::HashSet;
use std::fmt::{Debug, Display};
use std::time::Duration;

/// The timestamp up to which both operands have reported, if they both have.
///
/// `None` for an operand that has produced nothing yet is what separates it from one that
/// has reported at time zero: the first cannot answer anywhere, the second answers there.
fn joint_frontier(left: Option<Duration>, right: Option<Duration>) -> Option<Duration> {
    Some(left?.min(right?))
}

/// A unified binary processor that handles Delayed, Eager, and Refinable (RoSI) semantics correctly.
///
/// Both operands are piecewise constant between the steps they emit, so the combination is
/// piecewise constant too and its breakpoints are the union of theirs. Each operand is
/// therefore read at the breakpoints of the *other* under zero-order hold. That is what lets
/// two operands with unrelated breakpoint sets be combined at all: a temporal operand
/// answers at its own window boundaries, which are generally not timestamps the other one
/// ever reports, so matching the streams on equal timestamps would leave both sides unread.
///
/// A timestamp is answered once both operands are known there, i.e. up to the older of the
/// two frontiers. Eager mode may decide a timestamp from one operand alone, but only while
/// the other has reported nothing at all: once both are running, answers are emitted in
/// timestamp order like in every other mode. Short-circuiting a later timestamp ahead of an
/// earlier one that is merely still pending would push the watermark past that earlier
/// timestamp and strand its answer.
fn process_binary<C, Y, F, const IS_EAGER: bool, const IS_ROSI: bool>(
    left_cache: &C,
    right_cache: &C,
    left_frontier: Option<Duration>,
    right_frontier: Option<Duration>,
    start_after: Option<Duration>,
    combine_op: F,
    short_circuit_val: Option<Y>,
) -> Vec<Step<Y>>
where
    C: RingBufferTrait<Value = Y>,
    Y: RobustnessSemantics + Copy + Debug + PartialEq + 'static,
    F: Fn(Y, Y) -> Y,
{
    let mut output_robustness = Vec::new();
    let joint = joint_frontier(left_frontier, right_frontier);
    // `None` orders below every `Some`, so the maximum is the operand that has reported the
    // furthest, and is `None` only while neither has reported at all.
    let horizon = if IS_EAGER && !IS_ROSI {
        left_frontier.max(right_frontier)
    } else {
        joint
    };
    let Some(horizon) = horizon else {
        return output_robustness;
    };

    // Breakpoints at or before `start_after` have been answered already, and outside RoSI
    // an operand never revises them, so the walk can start past them and cost only what is
    // new. RoSI passes `None`: a refinement rewrites timestamps that were already emitted.
    let l_skip =
        start_after.map_or(0, |ts| left_cache.partition_point(|entry| entry.timestamp <= ts));
    let r_skip =
        start_after.map_or(0, |ts| right_cache.partition_point(|entry| entry.timestamp <= ts));
    let mut l_iter = left_cache.iter().skip(l_skip).peekable();
    let mut r_iter = right_cache.iter().skip(r_skip).peekable();

    loop {
        // Walk the union of the two breakpoint sets, consuming both when they coincide.
        let l_ts = l_iter.peek().map(|entry| entry.timestamp);
        let r_ts = r_iter.peek().map(|entry| entry.timestamp);
        let ts = match (l_ts, r_ts) {
            (Some(l), Some(r)) => l.min(r),
            (Some(l), None) => l,
            (None, Some(r)) => r,
            (None, None) => break,
        };
        if l_ts == Some(ts) {
            l_iter.next();
        }
        if r_ts == Some(ts) {
            r_iter.next();
        }
        if ts > horizon {
            break;
        }

        let left_value = left_frontier
            .is_some_and(|frontier| ts <= frontier)
            .then(|| left_cache.zoh_at(ts))
            .flatten()
            .map(|entry| entry.value);
        let right_value = right_frontier
            .is_some_and(|frontier| ts <= frontier)
            .then(|| right_cache.zoh_at(ts))
            .flatten()
            .map(|entry| entry.value);

        match (left_value, right_value) {
            (Some(l), Some(r)) => {
                output_robustness.push(Step::new("output", combine_op(l, r), ts));
            }
            // Both operands have reported at `ts`, but one cache no longer holds the value
            // in force there: it was pruned, which happens only once `ts` was answered.
            (Some(_), None) | (None, Some(_)) if joint.is_some_and(|frontier| ts <= frontier) => {}
            // Past the joint frontier, so this is eager mode running ahead on one operand.
            // A conjunction is already false, and a disjunction already true, if that
            // operand is; the one still missing cannot change it. Otherwise the answer has
            // to wait -- and so does every later one, which would otherwise be reported
            // ahead of it.
            (Some(value), None) | (None, Some(value)) => {
                if short_circuit_val != Some(value) {
                    break;
                }
                output_robustness.push(Step::new("output", value, ts));
            }
            (None, None) => {}
        }
    }

    output_robustness
}

#[derive(Clone)]
/// Logical conjunction operator.
///
/// Combines two operand streams with [`RobustnessSemantics::and`].
pub struct And<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> {
    left: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
    right: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
    left_cache: C,
    right_cache: C,
    last_eval_time: Option<Duration>,
    /// Newest timestamp each operand has produced a value for. Its signal is known up to
    /// here and no further, which is what decides how far the combination can be answered.
    left_frontier: Option<Duration>,
    right_frontier: Option<Duration>,
    left_signals_set: HashSet<&'static str>,
    right_signals_set: HashSet<&'static str>,
    max_lookahead: Duration,
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> And<T, C, Y, IS_EAGER, IS_ROSI> {
    /// Creates a new conjunction operator from left and right operands.
    ///
    /// If caches are `None`, empty caches are created.
    pub fn new(
        left: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
        right: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
        left_cache: Option<C>,
        right_cache: Option<C>,
    ) -> Self
    where
        T: Clone + 'static,
        C: RingBufferTrait<Value = Y> + Clone + 'static,
        Y: RobustnessSemantics + 'static,
    {
        let max_lookahead = left.get_max_lookahead().max(right.get_max_lookahead());
        And {
            left,
            right,
            left_cache: left_cache.unwrap_or_else(|| C::new()),
            right_cache: right_cache.unwrap_or_else(|| C::new()),
            last_eval_time: None,
            left_frontier: None,
            right_frontier: None,
            left_signals_set: HashSet::new(),
            right_signals_set: HashSet::new(),
            max_lookahead,
        }
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> StlOperatorTrait<T>
    for And<T, C, Y, IS_EAGER, IS_ROSI>
where
    T: Clone + 'static,
    C: RingBufferTrait<Value = Y> + Clone + 'static,
    Y: RobustnessSemantics + 'static + Debug + Copy,
{
    type Output = Y;

    fn get_max_lookahead(&self) -> Duration {
        self.max_lookahead
    }

    fn total_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.left_cache.heap_size()
            + self.right_cache.heap_size()
            + self.left_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.right_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.left.total_size()
            + self.right.total_size()
    }

    fn reset(&mut self) {
        self.left_cache.clear();
        self.right_cache.clear();
        self.last_eval_time = None;
        self.left_frontier = None;
        self.right_frontier = None;
        self.left.reset();
        self.right.reset();
    }

    /// Updates both operands with the incoming sample and emits conjunction outputs.
    ///
    /// Output emission depends on execution mode:
    /// - delayed: only finalized timestamp-aligned outputs,
    /// - eager: may short-circuit on semantic false,
    /// - RoSI: allows refinements at already-seen timestamps.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>> {
        let check_relevance = |timestamp: Duration, last_time: Option<Duration>| -> bool {
            match last_time {
                Some(last) => {
                    if IS_ROSI {
                        timestamp >= last // Intervals: allow refinement of current step
                    } else {
                        timestamp > last // Bool/F64: strictly new data only
                    }
                }
                None => true,
            }
        };

        let left_updates =
            if self.left_signals_set.contains(&step.signal) || self.left_signals_set.is_empty() {
                self.left.update(step)
            } else {
                Vec::new()
            };

        if let Some(last) = left_updates.last() {
            self.left_frontier = self.left_frontier.max(Some(last.timestamp));
        }
        for update in &left_updates {
            if check_relevance(update.timestamp, self.last_eval_time)
                && !self.left_cache.update_step(update.clone())
            {
                self.left_cache.add_step(update.clone());
            }
        }

        let right_updates =
            if self.right_signals_set.contains(&step.signal) || self.right_signals_set.is_empty() {
                self.right.update(step)
            } else {
                Vec::new()
            };

        if let Some(last) = right_updates.last() {
            self.right_frontier = self.right_frontier.max(Some(last.timestamp));
        }
        for update in &right_updates {
            if check_relevance(update.timestamp, self.last_eval_time)
                && !self.right_cache.update_step(update.clone())
            {
                self.right_cache.add_step(update.clone());
            }
        }

        let mut output = process_binary::<C, Y, _, IS_EAGER, IS_ROSI>(
            &self.left_cache,
            &self.right_cache,
            self.left_frontier,
            self.right_frontier,
            if IS_ROSI { None } else { self.last_eval_time },
            Y::and,
            Some(Y::atomic_false()),
        );

        // Ensure we don't emit stale timestamps for non-refinable types.
        if !IS_ROSI && let Some(last_time) = self.last_eval_time {
            output.retain(|step| step.timestamp > last_time);
        }

        let lookahead = self.get_max_lookahead();

        // we protect up to the minimum of the last known timestamps minus lookahead
        // we can safely prune it if both sides have verdict and are beyond lookahead
        let protected_ts = joint_frontier(self.left_frontier, self.right_frontier)
            .unwrap_or_default()
            .saturating_sub(lookahead);

        guarded_prune(&mut self.left_cache, lookahead, protected_ts);
        guarded_prune(&mut self.right_cache, lookahead, protected_ts);

        // Update last_eval_time based on delayed semantics
        if let Some(eval_time) = if IS_ROSI {
            // For intervals, we track the *start* of the batch because we might re-evaluate it
            output.first().map(|step| step.timestamp)
        } else {
            // For delayed/bool, we track the *end* because everything before is finalized
            output.last().map(|step| step.timestamp)
        } {
            self.last_eval_time = Some(eval_time);
        }

        output
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> SignalIdentifier
    for And<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Returns the union of signal identifiers from both operands.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        let mut ids = std::collections::HashSet::new();
        self.left_signals_set
            .extend(self.left.get_signal_identifiers());
        self.right_signals_set
            .extend(self.right.get_signal_identifiers());
        ids.extend(self.left_signals_set.iter().cloned());
        ids.extend(self.right_signals_set.iter().cloned());
        ids
    }
}

#[derive(Clone)]
/// Logical disjunction operator.
///
/// Combines two operand streams with [`RobustnessSemantics::or`].
pub struct Or<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> {
    left: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
    right: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
    left_cache: C,
    right_cache: C,
    last_eval_time: Option<Duration>,
    /// Newest timestamp each operand has produced a value for. Its signal is known up to
    /// here and no further, which is what decides how far the combination can be answered.
    left_frontier: Option<Duration>,
    right_frontier: Option<Duration>,
    left_signals_set: HashSet<&'static str>,
    right_signals_set: HashSet<&'static str>,
    max_lookahead: Duration,
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Or<T, C, Y, IS_EAGER, IS_ROSI> {
    /// Creates a new disjunction operator from left and right operands.
    ///
    /// If caches are `None`, empty caches are created.
    pub fn new(
        left: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
        right: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
        left_cache: Option<C>,
        right_cache: Option<C>,
    ) -> Self
    where
        T: Clone + 'static,
        C: RingBufferTrait<Value = Y> + Clone + 'static,
        Y: RobustnessSemantics + 'static,
    {
        let max_lookahead = left.get_max_lookahead().max(right.get_max_lookahead());
        Or {
            left,
            right,
            left_cache: left_cache.unwrap_or_else(|| C::new()),
            right_cache: right_cache.unwrap_or_else(|| C::new()),
            last_eval_time: None,
            left_frontier: None,
            right_frontier: None,
            left_signals_set: HashSet::new(),
            right_signals_set: HashSet::new(),
            max_lookahead,
        }
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> StlOperatorTrait<T>
    for Or<T, C, Y, IS_EAGER, IS_ROSI>
where
    T: Clone + 'static,
    C: RingBufferTrait<Value = Y> + Clone + 'static,
    Y: RobustnessSemantics + 'static + Debug + Copy,
{
    type Output = Y;

    fn get_max_lookahead(&self) -> Duration {
        self.max_lookahead
    }

    fn total_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.left_cache.heap_size()
            + self.right_cache.heap_size()
            + self.left_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.right_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.left.total_size()
            + self.right.total_size()
    }

    fn reset(&mut self) {
        self.left_cache.clear();
        self.right_cache.clear();
        self.last_eval_time = None;
        self.left_frontier = None;
        self.right_frontier = None;
        self.left.reset();
        self.right.reset();
    }

    /// Updates both operands with the incoming sample and emits disjunction outputs.
    ///
    /// Output emission depends on execution mode:
    /// - delayed: only finalized timestamp-aligned outputs,
    /// - eager: may short-circuit on semantic true,
    /// - RoSI: allows refinements at already-seen timestamps.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>> {
        let check_relevance = |timestamp: Duration, last_time: Option<Duration>| -> bool {
            match last_time {
                Some(last) => {
                    if IS_ROSI {
                        timestamp >= last // Intervals: allow refinement of current step
                    } else {
                        timestamp > last // Bool/F64: strictly new data only
                    }
                }
                None => true,
            }
        };

        let left_updates =
            if self.left_signals_set.contains(&step.signal) || self.left_signals_set.is_empty() {
                self.left.update(step)
            } else {
                Vec::new()
            };

        if let Some(last) = left_updates.last() {
            self.left_frontier = self.left_frontier.max(Some(last.timestamp));
        }
        for update in &left_updates {
            if check_relevance(update.timestamp, self.last_eval_time)
                && !self.left_cache.update_step(update.clone())
            {
                self.left_cache.add_step(update.clone());
            }
        }

        let right_updates =
            if self.right_signals_set.contains(&step.signal) || self.right_signals_set.is_empty() {
                self.right.update(step)
            } else {
                Vec::new()
            };

        if let Some(last) = right_updates.last() {
            self.right_frontier = self.right_frontier.max(Some(last.timestamp));
        }
        for update in &right_updates {
            if check_relevance(update.timestamp, self.last_eval_time)
                && !self.right_cache.update_step(update.clone())
            {
                self.right_cache.add_step(update.clone());
            }
        }

        let mut output = process_binary::<C, Y, _, IS_EAGER, IS_ROSI>(
            &self.left_cache,
            &self.right_cache,
            self.left_frontier,
            self.right_frontier,
            if IS_ROSI { None } else { self.last_eval_time },
            Y::or,
            Some(Y::atomic_true()),
        );

        // Ensure we don't emit stale timestamps for non-refinable types.
        if !IS_ROSI && let Some(last_time) = self.last_eval_time {
            output.retain(|step| step.timestamp > last_time);
        }

        let lookahead = self.get_max_lookahead();

        // we protect up to the minimum of the last known timestamps minus lookahead
        // we can safely prune it if both sides have verdict and are beyond lookahead
        let protected_ts = joint_frontier(self.left_frontier, self.right_frontier)
            .unwrap_or_default()
            .saturating_sub(lookahead);

        guarded_prune(&mut self.left_cache, lookahead, protected_ts);
        guarded_prune(&mut self.right_cache, lookahead, protected_ts);

        // Update last_eval_time based on delayed semantics
        if let Some(eval_time) = if IS_ROSI {
            // For intervals, we track the *start* of the batch because we might re-evaluate it
            output.first().map(|step| step.timestamp)
        } else {
            // For delayed/bool, we track the *end* because everything before is finalized
            output.last().map(|step| step.timestamp)
        } {
            self.last_eval_time = Some(eval_time);
        }

        output
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> SignalIdentifier
    for Or<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Returns the union of signal identifiers from both operands.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        let mut ids = std::collections::HashSet::new();
        self.left_signals_set
            .extend(self.left.get_signal_identifiers());
        self.right_signals_set
            .extend(self.right.get_signal_identifiers());
        ids.extend(self.left_signals_set.iter().cloned());
        ids.extend(self.right_signals_set.iter().cloned());
        ids
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Display
    for And<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Formats as `(left) ∧ (right)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}) ∧ ({})", self.left, self.right)
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Display
    for Or<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Formats as `(left) v (right)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}) v ({})", self.left, self.right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::StlOperatorTrait;
    use crate::operators::atomic_operators::Atomic;
    use crate::ring_buffer::RingBuffer;
    use crate::step;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn test_binary_display() {
        let atomic1 = Atomic::<f64>::new_greater_than("x", 10.0);
        let atomic2 = Atomic::<f64>::new_less_than("y", 5.0);
        let and = And::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(atomic1.clone()),
            Box::new(atomic2.clone()),
            None,
            None,
        );
        let or = Or::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(atomic1),
            Box::new(atomic2),
            None,
            None,
        );

        assert_eq!(and.to_string(), "(x > 10) ∧ (y < 5)");
        assert_eq!(or.to_string(), "(x > 10) v (y < 5)");
    }

    #[test]
    fn test_update_wrong_signal() {
        let atomic1 = Atomic::<f64>::new_greater_than("x", 10.0);
        let atomic2 = Atomic::<f64>::new_less_than("y", 5.0);
        let mut and = And::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(atomic1.clone()),
            Box::new(atomic2.clone()),
            None,
            None,
        );
        and.get_signal_identifiers();

        let step = step!("z", 15.0, Duration::from_secs(5));
        let robustness = and.update(&step);
        assert_eq!(robustness.len(), 0);

        let mut or = Or::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(atomic1),
            Box::new(atomic2),
            None,
            None,
        );
        or.get_signal_identifiers();

        let step = step!("z", 15.0, Duration::from_secs(5));
        let robustness = or.update(&step);
        assert_eq!(robustness.len(), 0);
    }

    #[test]
    fn and_operator_robustness_delayed() {
        let atomic1 = Atomic::<f64>::new_greater_than("x", 10.0);
        let atomic2 = Atomic::<f64>::new_less_than("x", 20.0);
        let mut and = And::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(atomic1),
            Box::new(atomic2),
            None,
            None,
        );
        and.get_signal_identifiers();

        let step = step!("x", 15.0, Duration::from_secs(5));
        let robustness = and.update(&step);
        assert_eq!(
            robustness,
            vec![step!("output", 5.0, Duration::from_secs(5))]
        );
    }

    #[test]
    fn or_operator_robustness_delayed() {
        let atomic1 = Atomic::<f64>::new_greater_than("x", 10.0);
        let atomic2 = Atomic::<f64>::new_less_than("x", 5.0);
        let mut or = Or::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(atomic1),
            Box::new(atomic2),
            None,
            None,
        );
        or.get_signal_identifiers();

        let step = step!("x", 15.0, Duration::from_secs(5));
        let robustness = or.update(&step);
        assert_eq!(
            robustness,
            vec![step!("output", 5.0, Duration::from_secs(5))]
        );
    }

    #[test]
    fn total_size_includes_children() {
        let a1 = Atomic::<f64>::new_greater_than("x", 10.0);
        let a2 = Atomic::<f64>::new_less_than("y", 5.0);
        let child_sum = <Atomic<f64> as StlOperatorTrait<f64>>::total_size(&a1)
            + <Atomic<f64> as StlOperatorTrait<f64>>::total_size(&a2);
        let and = And::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(a1),
            Box::new(a2),
            None,
            None,
        );
        assert!(and.total_size() >= child_sum + std::mem::size_of_val(&and));
    }

    #[test]
    fn total_size_or_includes_children() {
        let a1 = Atomic::<f64>::new_greater_than("x", 10.0);
        let a2 = Atomic::<f64>::new_less_than("y", 5.0);
        let child_sum = <Atomic<f64> as StlOperatorTrait<f64>>::total_size(&a1)
            + <Atomic<f64> as StlOperatorTrait<f64>>::total_size(&a2);
        let or = Or::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(a1),
            Box::new(a2),
            None,
            None,
        );
        assert!(or.total_size() >= child_sum + std::mem::size_of_val(&or));
    }

    #[test]
    fn total_size_grows_after_update() {
        let a1 = Atomic::<f64>::new_greater_than("x", 10.0);
        let a2 = Atomic::<f64>::new_less_than("x", 20.0);
        let mut and = And::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(a1),
            Box::new(a2),
            None,
            None,
        );
        and.get_signal_identifiers();
        let before = and.total_size();
        and.update(&step!("x", 15.0, Duration::from_secs(5)));
        assert!(and.total_size() >= before + std::mem::size_of::<Step<f64>>());
    }

    #[test]
    fn binary_operators_signal_identifiers() {
        let atomic1 = Atomic::<f64>::new_greater_than("x", 10.0);
        let atomic2 = Atomic::<f64>::new_less_than("y", 5.0);
        let mut and = And::<f64, RingBuffer<f64>, f64, false, false>::new(
            Box::new(atomic1),
            Box::new(atomic2),
            None,
            None,
        );
        let ids = and.get_signal_identifiers();
        let expected: HashSet<&'static str> = ["x", "y"].iter().cloned().collect();
        assert_eq!(ids, expected);
    }
}
