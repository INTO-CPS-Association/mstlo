//! Binary STL operators (`And`, `Or`).
//!
//! This module combines two child operators while supporting three execution
//! modes through const generics:
//! - delayed (`IS_EAGER = false`, `IS_ROSI = false`),
//! - eager short-circuiting (`IS_EAGER = true`, `IS_ROSI = false`), and
//! - refinable interval semantics (`IS_ROSI = true`).

use crate::core::{
    Reach, RobustnessSemantics, SignalIdentifier, StlOperatorAndSignalIdentifier, StlOperatorTrait,
};
use crate::ring_buffer::{RingBufferTrait, Step, guarded_prune};
use std::collections::HashSet;
use std::fmt::{Debug, Display};
use std::time::Duration;

/// The timestamp up to which both operands have reported, or `None` if either has not.
fn joint_frontier(left: Option<Duration>, right: Option<Duration>) -> Option<Duration> {
    Some(left?.min(right?))
}

/// How far eager output is settled: the joint frontier, extended for as long as the
/// leading operand holds `short_circuit_val` without interruption from there on.
///
/// The lagging operand cannot change the output over that stretch.
fn eager_known_through<C, Y>(
    left: (&C, Option<Duration>),
    right: (&C, Option<Duration>),
    short_circuit_val: Y,
) -> Option<Duration>
where
    C: RingBufferTrait<Value = Y>,
    Y: PartialEq,
{
    let (cache, frontier) = if left.1 > right.1 { left } else { right };
    // Until the lagging operand reports at all, the stretch starts at the leading
    // operand's first entry: nothing is pruned before then, and the output is undefined
    // earlier.
    let joint = joint_frontier(left.1, right.1).or_else(|| {
        cache
            .iter()
            .next()
            .filter(|entry| entry.value == short_circuit_val)
            .map(|entry| entry.timestamp)
    })?;
    let held_at_joint = cache
        .partition_point(|entry| entry.timestamp <= joint)
        .saturating_sub(1);

    let mut known = joint;
    let mut covered = joint;
    for entry in cache.iter().skip(held_at_joint) {
        if entry.timestamp > covered
            || Some(entry.timestamp) > frontier
            || entry.value != short_circuit_val
        {
            break;
        }
        known = known.max(entry.timestamp);
        covered = entry.held_until;
    }
    Some(known)
}

/// Whether `ts` lies inside the region both operands have reported on.
fn within(joint: Option<Duration>, ts: Duration) -> bool {
    joint.is_some_and(|frontier| ts <= frontier)
}

/// Settles the emission watermark for eager mode after a batch of output.
///
/// Advances it to the newest output at or below the joint frontier. Outputs
/// short-circuited past that frontier do not move it, since the lagging operand still has
/// to be read there.
fn settle_eager_watermark<Y>(
    output: &[Step<Y>],
    joint: Option<Duration>,
    last_eval_time: &mut Option<Duration>,
) {
    if let Some(eval_time) = output
        .iter()
        .map(|step| step.timestamp)
        .rfind(|ts| within(joint, *ts))
    {
        *last_eval_time = Some(eval_time);
    }
}

/// One side of a binary operator: the cache of what that operand has emitted.
struct Operand<'a, C> {
    cache: &'a C,
    /// Newest timestamp the operand has produced a value for.
    frontier: Option<Duration>,
    /// Newest timestamp the operand has produced a *final* value for. Under RoSI a
    /// non-final interval may still be refined, so a held value past this mark is
    /// re-masked (see [`read_operand`]).
    settled: Option<Duration>,
    /// The operand's [`StlOperatorTrait::reach`], used to mask RoSI bounds.
    reach: Reach,
}

/// Reads the operand's value at `ts` from its cache under zero-order hold.
///
/// `None` past the operand's frontier. Under RoSI, a value held from an earlier entry
/// (`entry.timestamp < ts`) is only trusted where the operand has settled; past that
/// mark its bounds whose [`Reach`] lag has not elapsed are widened to
/// [`RobustnessSemantics::unknown`]. A fresh emission at `ts`, or one within the settled
/// region, is used as-is, so a settled bound (e.g. a positive `F` lower bound read by an
/// `or`) is never discarded.
///
/// `newest` is the operand's newest entry at or before `ts`, tracked by the caller's walk.
fn read_operand<C, Y, const IS_ROSI: bool>(
    operand: &Operand<'_, C>,
    newest: Option<&Step<Y>>,
    ts: Duration,
) -> Option<Y>
where
    C: RingBufferTrait<Value = Y>,
    Y: RobustnessSemantics + Copy,
{
    let frontier = operand.frontier.filter(|bound| ts <= *bound)?;
    let entry = newest.filter(|entry| entry.held_until > ts)?;
    if IS_ROSI && entry.timestamp < ts && operand.settled.is_none_or(|settled| ts > settled) {
        return Some(operand.reach.mask(entry.value, ts, frontier));
    }
    Some(entry.value)
}

/// A unified binary processor that handles Delayed, Eager, and Refinable (RoSI) semantics correctly.
///
/// The output is evaluated at the union of both operands' breakpoints, reading each
/// operand under zero-order hold.
///
/// A timestamp is answered once both operands are known there. Eager mode may also
/// short-circuit on one operand alone past the joint frontier, but only as far as
/// [`eager_known_through`], so output is always emitted in timestamp order.
fn process_binary<C, Y, F, const IS_EAGER: bool, const IS_ROSI: bool>(
    left: &Operand<'_, C>,
    right: &Operand<'_, C>,
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
    let joint = joint_frontier(left.frontier, right.frontier);
    // Eager can run ahead to the furthest operand; other modes stop at the joint frontier.
    let horizon = if IS_EAGER && !IS_ROSI {
        left.frontier.max(right.frontier)
    } else {
        joint
    };
    let Some(horizon) = horizon else {
        return output_robustness;
    };
    // Eager only short-circuits past the joint frontier over this stretch, so the lagging
    // operand can never deliver a breakpoint that changes an earlier output.
    let settled = match short_circuit_val {
        Some(value) if IS_EAGER && !IS_ROSI => eager_known_through(
            (left.cache, left.frontier),
            (right.cache, right.frontier),
            value,
        ),
        _ => None,
    };

    // Skip breakpoints already answered. RoSI passes `None`, since it refines past outputs.
    let l_skip = start_after.map_or(0, |ts| {
        left.cache.partition_point(|entry| entry.timestamp <= ts)
    });
    let r_skip = start_after.map_or(0, |ts| {
        right.cache.partition_point(|entry| entry.timestamp <= ts)
    });
    // Start one entry early: the last skipped entry is the value held at the first breakpoint.
    let mut l_iter = left.cache.iter().skip(l_skip.saturating_sub(1)).peekable();
    let mut r_iter = right.cache.iter().skip(r_skip.saturating_sub(1)).peekable();
    let mut l_newest = if l_skip > 0 { l_iter.next() } else { None };
    let mut r_newest = if r_skip > 0 { r_iter.next() } else { None };

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
            l_newest = l_iter.next();
        }
        if r_ts == Some(ts) {
            r_newest = r_iter.next();
        }
        if ts > horizon {
            break;
        }

        let left_value = read_operand::<_, _, IS_ROSI>(left, l_newest, ts);
        let right_value = read_operand::<_, _, IS_ROSI>(right, r_newest, ts);

        match (left_value, right_value) {
            (Some(l), Some(r)) => {
                output_robustness.push(Step::new("output", combine_op(l, r), ts));
            }
            // One side was pruned, so `ts` was already answered.
            (Some(_), None) | (None, Some(_)) if joint.is_some_and(|frontier| ts <= frontier) => {}
            // Eager past the joint frontier: short-circuit if possible, otherwise wait.
            (Some(value), None) | (None, Some(value)) => {
                if short_circuit_val != Some(value) || settled.is_none_or(|bound| ts > bound) {
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
    /// Newest timestamp each operand has produced a value for.
    left_frontier: Option<Duration>,
    right_frontier: Option<Duration>,
    /// Newest timestamp each operand has produced a *final* value for.
    left_settled: Option<Duration>,
    right_settled: Option<Duration>,
    /// Eager only: see [`eager_known_through`].
    known: Option<Duration>,
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
            left_settled: None,
            right_settled: None,
            known: None,
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

    fn reach(&self) -> Reach {
        self.left.reach().join(self.right.reach())
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
        self.left_settled = None;
        self.right_settled = None;
        self.known = None;
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
            if update.value.is_final() {
                self.left_settled = self.left_settled.max(Some(update.timestamp));
            }
            if check_relevance(update.timestamp, self.last_eval_time)
                && !self.left_cache.update_step(update.clone())
            {
                // Operand output can arrive out of order; see `RingBufferTrait::insert_step`.
                self.left_cache.insert_step(update.clone());
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
            if update.value.is_final() {
                self.right_settled = self.right_settled.max(Some(update.timestamp));
            }
            if check_relevance(update.timestamp, self.last_eval_time)
                && !self.right_cache.update_step(update.clone())
            {
                // Operand output can arrive out of order; see `RingBufferTrait::insert_step`.
                self.right_cache.insert_step(update.clone());
            }
        }

        let mut output = process_binary::<C, Y, _, IS_EAGER, IS_ROSI>(
            &Operand {
                cache: &self.left_cache,
                frontier: self.left_frontier,
                settled: self.left_settled,
                reach: self.left.reach(),
            },
            &Operand {
                cache: &self.right_cache,
                frontier: self.right_frontier,
                settled: self.right_settled,
                reach: self.right.reach(),
            },
            if IS_ROSI { None } else { self.last_eval_time },
            Y::and,
            Some(Y::atomic_false()),
        );

        // Ensure we don't emit stale timestamps for non-refinable types.
        if !IS_ROSI && let Some(last_time) = self.last_eval_time {
            output.retain(|step| step.timestamp > last_time);
        }

        let lookahead = self.get_max_lookahead();
        let joint = joint_frontier(self.left_frontier, self.right_frontier);
        if IS_EAGER && !IS_ROSI {
            // A late breakpoint inside the settled stretch repeats the held value.
            output.retain(|step| self.known.is_none_or(|known| step.timestamp > known));
            self.known = eager_known_through(
                (&self.left_cache, self.left_frontier),
                (&self.right_cache, self.right_frontier),
                Y::atomic_false(),
            );
        }

        // we protect up to the minimum of the last known timestamps minus lookahead
        // we can safely prune it if both sides have verdict and are beyond lookahead
        let protected_ts = joint.unwrap_or_default().saturating_sub(lookahead);

        guarded_prune(&mut self.left_cache, lookahead, protected_ts);
        guarded_prune(&mut self.right_cache, lookahead, protected_ts);

        // Update last_eval_time based on delayed semantics
        if IS_ROSI {
            // For intervals, we track the *start* of the batch because we might re-evaluate it
            if let Some(eval_time) = output.first().map(|step| step.timestamp) {
                self.last_eval_time = Some(eval_time);
            }
        } else if IS_EAGER {
            settle_eager_watermark(&output, joint, &mut self.last_eval_time);
        } else {
            // For delayed/bool, we track the *end* because everything before is finalized
            if let Some(eval_time) = output.last().map(|step| step.timestamp) {
                self.last_eval_time = Some(eval_time);
            }
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
    /// Newest timestamp each operand has produced a value for.
    left_frontier: Option<Duration>,
    right_frontier: Option<Duration>,
    /// Newest timestamp each operand has produced a *final* value for.
    left_settled: Option<Duration>,
    right_settled: Option<Duration>,
    /// Eager only: see [`eager_known_through`].
    known: Option<Duration>,
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
            left_settled: None,
            right_settled: None,
            known: None,
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

    fn reach(&self) -> Reach {
        self.left.reach().join(self.right.reach())
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
        self.left_settled = None;
        self.right_settled = None;
        self.known = None;
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
            if update.value.is_final() {
                self.left_settled = self.left_settled.max(Some(update.timestamp));
            }
            if check_relevance(update.timestamp, self.last_eval_time)
                && !self.left_cache.update_step(update.clone())
            {
                // Operand output can arrive out of order; see `RingBufferTrait::insert_step`.
                self.left_cache.insert_step(update.clone());
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
            if update.value.is_final() {
                self.right_settled = self.right_settled.max(Some(update.timestamp));
            }
            if check_relevance(update.timestamp, self.last_eval_time)
                && !self.right_cache.update_step(update.clone())
            {
                // Operand output can arrive out of order; see `RingBufferTrait::insert_step`.
                self.right_cache.insert_step(update.clone());
            }
        }

        let mut output = process_binary::<C, Y, _, IS_EAGER, IS_ROSI>(
            &Operand {
                cache: &self.left_cache,
                frontier: self.left_frontier,
                settled: self.left_settled,
                reach: self.left.reach(),
            },
            &Operand {
                cache: &self.right_cache,
                frontier: self.right_frontier,
                settled: self.right_settled,
                reach: self.right.reach(),
            },
            if IS_ROSI { None } else { self.last_eval_time },
            Y::or,
            Some(Y::atomic_true()),
        );

        // Ensure we don't emit stale timestamps for non-refinable types.
        if !IS_ROSI && let Some(last_time) = self.last_eval_time {
            output.retain(|step| step.timestamp > last_time);
        }

        let lookahead = self.get_max_lookahead();
        let joint = joint_frontier(self.left_frontier, self.right_frontier);
        if IS_EAGER && !IS_ROSI {
            // A late breakpoint inside the settled stretch repeats the held value.
            output.retain(|step| self.known.is_none_or(|known| step.timestamp > known));
            self.known = eager_known_through(
                (&self.left_cache, self.left_frontier),
                (&self.right_cache, self.right_frontier),
                Y::atomic_true(),
            );
        }

        // we protect up to the minimum of the last known timestamps minus lookahead
        // we can safely prune it if both sides have verdict and are beyond lookahead
        let protected_ts = joint.unwrap_or_default().saturating_sub(lookahead);

        guarded_prune(&mut self.left_cache, lookahead, protected_ts);
        guarded_prune(&mut self.right_cache, lookahead, protected_ts);

        // Update last_eval_time based on delayed semantics
        if IS_ROSI {
            // For intervals, we track the *start* of the batch because we might re-evaluate it
            if let Some(eval_time) = output.first().map(|step| step.timestamp) {
                self.last_eval_time = Some(eval_time);
            }
        } else if IS_EAGER {
            settle_eager_watermark(&output, joint, &mut self.last_eval_time);
        } else {
            // For delayed/bool, we track the *end* because everything before is finalized
            if let Some(eval_time) = output.last().map(|step| step.timestamp) {
                self.last_eval_time = Some(eval_time);
            }
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
