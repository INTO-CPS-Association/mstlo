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
use std::collections::{HashSet, VecDeque};
use std::fmt::{Debug, Display};
use std::time::Duration;

/// The timestamp up to which both operands have reported, if they both have.
///
/// `None` for an operand that has produced nothing yet is what separates it from one that
/// has reported at time zero: the first cannot answer anywhere, the second answers there.
fn joint_frontier(left: Option<Duration>, right: Option<Duration>) -> Option<Duration> {
    Some(left?.min(right?))
}

/// Caps a frontier by what the operand reports as gap-free; see
/// [`StlOperatorTrait::known_through`]. A `None` bound means no holes, so no cap.
///
/// Apply this after taking the max with the newest emitted timestamp: unlike that
/// timestamp this bound does not only rise, and must be able to pull the frontier back
/// when a hole opens.
fn clamp_to_known(frontier: Option<Duration>, known: Option<Duration>) -> Option<Duration> {
    match known {
        Some(bound) => Some(frontier?.min(bound)),
        None => frontier,
    }
}

/// Whether `ts` lies inside the region both operands have reported on.
fn within(joint: Option<Duration>, ts: Duration) -> bool {
    joint.is_some_and(|frontier| ts <= frontier)
}

/// Settles the emission watermark for eager mode after a batch of output.
///
/// Advances it to the newest output at or below the joint frontier, and records the
/// timestamps short-circuited past that frontier separately. Those answers are final, but
/// must not move the watermark: the lagging operand can still deliver a breakpoint earlier
/// in time, and the watermark is what would drop it.
fn settle_eager_watermark<Y>(
    output: &[Step<Y>],
    joint: Option<Duration>,
    last_eval_time: &mut Option<Duration>,
    answered_beyond_joint: &mut VecDeque<Duration>,
) {
    // Only timestamps both operands could answer finalize in the ordinary sense.
    if let Some(eval_time) = output
        .iter()
        .map(|step| step.timestamp)
        .rfind(|ts| within(joint, *ts))
    {
        *last_eval_time = Some(eval_time);
    }

    answered_beyond_joint.extend(
        output
            .iter()
            .map(|step| step.timestamp)
            .filter(|ts| !within(joint, *ts)),
    );

    // Once the watermark reaches a remembered timestamp it suppresses re-emission on its
    // own, so the queue only ever holds the genuinely-ahead ones and stays bounded by the
    // frontier gap rather than by the length of the trace.
    if let Some(last) = *last_eval_time {
        answered_beyond_joint.retain(|ts| *ts > last);
    }
}

/// One side of a binary operator: the cache of what that operand has emitted, plus what is
/// needed to read a value back out of it correctly.
struct Operand<'a, C> {
    cache: &'a C,
    /// Newest timestamp the operand has produced a value for. Its output is known up to
    /// here and no further.
    frontier: Option<Duration>,
    /// The operand's [`StlOperatorTrait::get_max_lookahead`], which for a temporal operator
    /// is exactly the distance from its window end back to the evaluation timestamp.
    lookahead: Duration,
}

/// Reads what an operand is worth at `ts`, through the cache of its emissions.
///
/// An operand's output is piecewise constant with breakpoints exactly where it emits, so a
/// zero-order-hold read of the cache is exact -- but only up to `frontier - lookahead`.
/// Past that mark the operand's own window is still open at `ts`, so the value it emitted
/// below `ts` is refinable, and holding it forward would pass on a finality the operand
/// never claimed. `ts` is then simply outside what this operand is known over, which is the
/// `[f_inf, f_sup]` case of the RoSI atomic rule: [`RobustnessSemantics::unknown`].
fn read_operand<C, Y, const IS_ROSI: bool>(operand: &Operand<'_, C>, ts: Duration) -> Option<Y>
where
    C: RingBufferTrait<Value = Y>,
    Y: RobustnessSemantics + Copy,
{
    if !within(operand.frontier, ts) {
        return None;
    }
    if IS_ROSI
        && operand
            .frontier
            .is_some_and(|bound| ts + operand.lookahead > bound)
    {
        return Some(Y::unknown());
    }
    operand.cache.zoh_at(ts).map(|entry| entry.value)
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
    // `None` orders below every `Some`, so the maximum is the operand that has reported the
    // furthest, and is `None` only while neither has reported at all.
    let horizon = if IS_EAGER && !IS_ROSI {
        left.frontier.max(right.frontier)
    } else {
        joint
    };
    let Some(horizon) = horizon else {
        return output_robustness;
    };

    // Breakpoints at or before `start_after` have been answered already, and outside RoSI
    // an operand never revises them, so the walk can start past them and cost only what is
    // new. RoSI passes `None`: a refinement rewrites timestamps that were already emitted.
    let l_skip = start_after.map_or(0, |ts| {
        left.cache.partition_point(|entry| entry.timestamp <= ts)
    });
    let r_skip = start_after.map_or(0, |ts| {
        right.cache.partition_point(|entry| entry.timestamp <= ts)
    });
    let mut l_iter = left.cache.iter().skip(l_skip).peekable();
    let mut r_iter = right.cache.iter().skip(r_skip).peekable();

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

        let left_value = read_operand::<_, _, IS_ROSI>(left, ts);
        let right_value = read_operand::<_, _, IS_ROSI>(right, ts);

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
    /// Timestamps eager answered ahead of the joint frontier. Held apart from
    /// `last_eval_time` so a short-circuit cannot strand a later-arriving earlier
    /// breakpoint; see [`settle_eager_watermark`].
    answered_beyond_joint: VecDeque<Duration>,
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
            answered_beyond_joint: VecDeque::new(),
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

    /// Eager short-circuiting past the joint frontier is what puts holes in this stream, so
    /// the joint frontier is exactly how far it is gap-free. Every other mode answers a
    /// timestamp only once both operands are known there, and has no holes to declare.
    fn known_through(&self) -> Option<Duration> {
        if !IS_EAGER || IS_ROSI {
            return None;
        }
        Some(joint_frontier(self.left_frontier, self.right_frontier).unwrap_or(Duration::ZERO))
    }

    fn total_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.left_cache.heap_size()
            + self.right_cache.heap_size()
            + self.answered_beyond_joint.capacity() * std::mem::size_of::<Duration>()
            + self.left_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.right_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.left.total_size()
            + self.right.total_size()
    }

    fn reset(&mut self) {
        self.left_cache.clear();
        self.right_cache.clear();
        self.last_eval_time = None;
        self.answered_beyond_joint.clear();
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
        self.left_frontier = clamp_to_known(self.left_frontier, self.left.known_through());
        for update in &left_updates {
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
        self.right_frontier = clamp_to_known(self.right_frontier, self.right.known_through());
        for update in &right_updates {
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
                lookahead: self.left.get_max_lookahead(),
            },
            &Operand {
                cache: &self.right_cache,
                frontier: self.right_frontier,
                lookahead: self.right.get_max_lookahead(),
            },
            if IS_ROSI { None } else { self.last_eval_time },
            Y::and,
            Some(Y::atomic_false()),
        );

        // Ensure we don't emit stale timestamps for non-refinable types.
        if !IS_ROSI && let Some(last_time) = self.last_eval_time {
            output.retain(|step| step.timestamp > last_time);
        }

        // A timestamp short-circuited beyond the joint frontier is re-walked on every later
        // update, because the watermark deliberately stops short of it. Drop the repeats:
        // the value is final and was already reported.
        if IS_EAGER && !IS_ROSI && !self.answered_beyond_joint.is_empty() {
            output.retain(|step| !self.answered_beyond_joint.contains(&step.timestamp));
        }

        let lookahead = self.get_max_lookahead();
        let joint = joint_frontier(self.left_frontier, self.right_frontier);

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
            settle_eager_watermark(
                &output,
                joint,
                &mut self.last_eval_time,
                &mut self.answered_beyond_joint,
            );
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
    /// Timestamps eager answered ahead of the joint frontier. Held apart from
    /// `last_eval_time` so a short-circuit cannot strand a later-arriving earlier
    /// breakpoint; see [`settle_eager_watermark`].
    answered_beyond_joint: VecDeque<Duration>,
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
            answered_beyond_joint: VecDeque::new(),
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

    /// Eager short-circuiting past the joint frontier is what puts holes in this stream, so
    /// the joint frontier is exactly how far it is gap-free. Every other mode answers a
    /// timestamp only once both operands are known there, and has no holes to declare.
    fn known_through(&self) -> Option<Duration> {
        if !IS_EAGER || IS_ROSI {
            return None;
        }
        Some(joint_frontier(self.left_frontier, self.right_frontier).unwrap_or(Duration::ZERO))
    }

    fn total_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.left_cache.heap_size()
            + self.right_cache.heap_size()
            + self.answered_beyond_joint.capacity() * std::mem::size_of::<Duration>()
            + self.left_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.right_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.left.total_size()
            + self.right.total_size()
    }

    fn reset(&mut self) {
        self.left_cache.clear();
        self.right_cache.clear();
        self.last_eval_time = None;
        self.answered_beyond_joint.clear();
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
        self.left_frontier = clamp_to_known(self.left_frontier, self.left.known_through());
        for update in &left_updates {
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
        self.right_frontier = clamp_to_known(self.right_frontier, self.right.known_through());
        for update in &right_updates {
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
                lookahead: self.left.get_max_lookahead(),
            },
            &Operand {
                cache: &self.right_cache,
                frontier: self.right_frontier,
                lookahead: self.right.get_max_lookahead(),
            },
            if IS_ROSI { None } else { self.last_eval_time },
            Y::or,
            Some(Y::atomic_true()),
        );

        // Ensure we don't emit stale timestamps for non-refinable types.
        if !IS_ROSI && let Some(last_time) = self.last_eval_time {
            output.retain(|step| step.timestamp > last_time);
        }

        // A timestamp short-circuited beyond the joint frontier is re-walked on every later
        // update, because the watermark deliberately stops short of it. Drop the repeats:
        // the value is final and was already reported.
        if IS_EAGER && !IS_ROSI && !self.answered_beyond_joint.is_empty() {
            output.retain(|step| !self.answered_beyond_joint.contains(&step.timestamp));
        }

        let lookahead = self.get_max_lookahead();
        let joint = joint_frontier(self.left_frontier, self.right_frontier);

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
            settle_eager_watermark(
                &output,
                joint,
                &mut self.last_eval_time,
                &mut self.answered_beyond_joint,
            );
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
