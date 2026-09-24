//! Temporal `Until` operator implementation.
//!
//! This module provides [`Until`], a stateful evaluator for `φ U[a,b] ψ` over
//! streamed samples, supporting delayed, eager, and refinable (RoSI) modes via
//! const generics.

use crate::core::{
    RobustnessSemantics, SignalIdentifier, StlOperatorAndSignalIdentifier, StlOperatorTrait,
    TimeInterval,
};
use crate::operators::unary_temporal_operators::enqueue_eval;
use crate::ring_buffer::{RingBufferTrait, Step, guarded_prune};
use std::collections::{HashSet, VecDeque};
use std::fmt::Display;
use std::time::Duration;

/// The value an operand holds at `t`, read off the cache entry in force there.
///
/// Under RoSI, a value carried forward past `settled_through` is still being refined, so it
/// reads as `Y::unknown()`.
fn held_value<Y: RobustnessSemantics, const IS_ROSI: bool>(
    entry: &Step<Y>,
    t: Duration,
    settled_through: Duration,
) -> Y {
    if IS_ROSI && entry.timestamp < t && t > settled_through {
        Y::unknown()
    } else {
        entry.value.clone()
    }
}

#[derive(Clone)]
/// Temporal until operator `φ U[a,b] ψ`.
///
/// For each evaluation timestamp `t`, this operator computes the usual STL until
/// aggregation on the interval `[t + a, t + b]` using the selected robustness
/// domain `Y`.
///
/// Internally it keeps per-operand caches and an evaluation task buffer so that
/// results can be emitted incrementally as data arrives.
pub struct Until<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> {
    interval: TimeInterval,
    left: Box<dyn StlOperatorAndSignalIdentifier<T, Y> + 'static>,
    right: Box<dyn StlOperatorAndSignalIdentifier<T, Y> + 'static>,
    left_cache: C,
    right_cache: C,
    t_max: (Duration, Duration), // (left t_max, right t_max)
    /// Newest timestamp for which each operand has reported a final value.
    settled: (Duration, Duration),
    eval_buffer: VecDeque<Duration>,
    left_signals_set: HashSet<&'static str>,
    right_signals_set: HashSet<&'static str>,
    max_lookahead: Duration,
    /// Timestamp of the first operand output. Earlier evaluation timestamps are dropped.
    first_ts: Option<Duration>,
    /// Newest evaluation timestamp with a final answer. Earlier ones are not re-queued.
    finalized_ts: Option<Duration>,
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Until<T, C, Y, IS_EAGER, IS_ROSI> {
    /// Creates a new `Until` operator.
    ///
    /// `max_lookahead` is computed as:
    /// `interval.end + max(left.get_max_lookahead(), right.get_max_lookahead())`.
    ///
    /// If caches are `None`, empty caches are created.
    pub fn new(
        interval: TimeInterval,
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
        let max_lookahead = interval.end + left.get_max_lookahead().max(right.get_max_lookahead());

        #[cfg(feature = "track-cache-size")]
        {
            let mut l_cache = left_cache.unwrap_or_else(|| C::new());
            l_cache.set_tracked(true); // Enable tracking for this cache
            let mut r_cache = right_cache.unwrap_or_else(|| C::new());
            r_cache.set_tracked(true); // Enable tracking for this cache
            Until {
                interval,
                left,
                right,
                left_cache: l_cache,
                right_cache: r_cache,
                t_max: (Duration::ZERO, Duration::ZERO),
                settled: (Duration::ZERO, Duration::ZERO),
                eval_buffer: VecDeque::new(),
                left_signals_set: HashSet::new(),
                right_signals_set: HashSet::new(),
                max_lookahead,
                first_ts: None,
                finalized_ts: None,
            }
        }
        #[cfg(not(feature = "track-cache-size"))]
        {
            Until {
                interval,
                left,
                right,
                left_cache: left_cache.unwrap_or_else(|| C::new()),
                right_cache: right_cache.unwrap_or_else(|| C::new()),
                t_max: (Duration::ZERO, Duration::ZERO),
                settled: (Duration::ZERO, Duration::ZERO),
                eval_buffer: VecDeque::new(),
                left_signals_set: HashSet::new(),
                right_signals_set: HashSet::new(),
                max_lookahead,
                first_ts: None,
                finalized_ts: None,
            }
        }
    }

    /// Adds a step to a cache, in timestamp order.
    ///
    /// Returns `true` when the step landed *behind* the newest one already cached.
    fn add_to_cache(cache: &mut C, step: Step<Y>) -> bool
    where
        C: RingBufferTrait<Value = Y>,
        Y: Clone,
    {
        let is_late = cache
            .get_back()
            .is_some_and(|back| step.timestamp < back.timestamp);
        // An eager child can emit out of order, so insert rather than append.
        cache.insert_step(step);
        is_late
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> StlOperatorTrait<T>
    for Until<T, C, Y, IS_EAGER, IS_ROSI>
where
    T: Clone + 'static,
    C: RingBufferTrait<Value = Y> + Clone + 'static,
    Y: RobustnessSemantics + 'static + std::fmt::Debug,
{
    type Output = Y;

    fn get_max_lookahead(&self) -> Duration {
        self.max_lookahead
    }

    fn total_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.left_cache.heap_size()
            + self.right_cache.heap_size()
            + self.eval_buffer.capacity() * std::mem::size_of::<Duration>()
            + self.left_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.right_signals_set.capacity() * (std::mem::size_of::<&str>() + 1)
            + self.left.total_size()
            + self.right.total_size()
    }

    fn reset(&mut self) {
        self.left_cache.clear();
        self.right_cache.clear();
        self.eval_buffer.clear();
        self.t_max = (Duration::ZERO, Duration::ZERO);
        self.first_ts = None;
        self.finalized_ts = None;
        self.left.reset();
        self.right.reset();
    }

    /// Eager output is gap-free up to `finalized_ts`, further capped by the operands' own
    /// bounds shifted back by `interval.end`. Delayed and RoSI emit in order and have no gaps.
    fn known_through(&self) -> Option<Duration> {
        if !IS_EAGER || IS_ROSI {
            return None;
        }
        let answered = self.finalized_ts.unwrap_or(Duration::ZERO);
        let operand_bound = match (self.left.known_through(), self.right.known_through()) {
            (None, None) => return Some(answered),
            (left, right) => left
                .unwrap_or(Duration::MAX)
                .min(right.unwrap_or(Duration::MAX)),
        };
        Some(answered.min(operand_bound.saturating_sub(self.interval.end)))
    }

    /// Updates the operator with one input sample and emits newly available outputs.
    ///
    /// High-level flow:
    /// 1. Update child operators and cache their outputs.
    /// 2. Iterate pending evaluation timestamps and compute `Until` aggregation.
    /// 3. Finalize/short-circuit/refine depending on mode.
    /// 4. Prune caches and remove completed tasks.
    ///
    /// Mode behavior:
    /// - delayed (`IS_EAGER = false`, `IS_ROSI = false`): emit only finalized values,
    /// - eager (`IS_EAGER = true`): may short-circuit on early `true`/`false`,
    /// - RoSI (`IS_ROSI = true`): emit refinable intermediate values widened with `unknown()`.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>> {
        let mut output_robustness = Vec::new();

        // 1. Populate caches with results from children operators
        let left_updates =
            if self.left_signals_set.contains(&step.signal) || self.left_signals_set.is_empty() {
                self.left.update(step)
            } else {
                Vec::new()
            };
        let right_updates =
            if self.right_signals_set.contains(&step.signal) || self.right_signals_set.is_empty() {
                self.right.update(step)
            } else {
                Vec::new()
            };

        // t_max is the minimum of the latest timestamp in both caches
        if let Some(last_left) = left_updates.last() {
            self.t_max.0 = self.t_max.0.max(last_left.timestamp);
        }
        if let Some(last_right) = right_updates.last() {
            self.t_max.1 = self.t_max.1.max(last_right.timestamp);
        }

        // Cap each frontier at how far the operand is gap-free. This bound can decrease,
        // so it is applied after the `max`.
        if let Some(bound) = self.left.known_through() {
            self.t_max.0 = self.t_max.0.min(bound);
        }
        if let Some(bound) = self.right.known_through() {
            self.t_max.1 = self.t_max.1.min(bound);
        }

        let t_max_combined = self.t_max.0.min(self.t_max.1);

        // Add all updates to eval_buffer and caches.
        // Collect timestamps from both children, sort, dedup, then push
        // in order so interleaved timestamps don't violate monotonicity.
        let mut all_ts: Vec<Duration> =
            Vec::with_capacity(left_updates.len() + right_updates.len());
        // Breakpoints that arrived behind the cache back; they may re-open answered windows.
        let mut late_ts: Vec<Duration> = Vec::new();
        for update in &right_updates {
            all_ts.push(update.timestamp);
            if update.value.is_final() {
                self.settled.1 = self.settled.1.max(update.timestamp);
            }
            if Self::add_to_cache(&mut self.right_cache, update.clone()) {
                late_ts.push(update.timestamp);
            }
        }
        for update in &left_updates {
            all_ts.push(update.timestamp);
            if update.value.is_final() {
                self.settled.0 = self.settled.0.max(update.timestamp);
            }
            if Self::add_to_cache(&mut self.left_cache, update.clone()) {
                late_ts.push(update.timestamp);
            }
        }
        all_ts.sort();
        all_ts.dedup();
        // Each operand breakpoint `ts` queues `ts`, `ts - a` and `ts - b`: the output can
        // only change where a window boundary crosses a breakpoint.
        for ts in all_ts {
            let earliest = *self.first_ts.get_or_insert(ts);
            for candidate in [
                Some(ts),
                ts.checked_sub(self.interval.start),
                ts.checked_sub(self.interval.end),
            ] {
                // An answered window is re-opened only by a late breakpoint, and only if
                // the window was not already fully covered by data.
                let reopenable = late_ts.contains(&ts)
                    && candidate.is_some_and(|t| t + self.interval.end > t_max_combined);
                if let Some(t) = candidate
                    && t >= earliest
                    && (reopenable || self.finalized_ts.is_none_or(|answered| t > answered))
                {
                    enqueue_eval(&mut self.eval_buffer, t);
                }
            }
        }
        let mut tasks_to_remove = Vec::new();
        let mut newest_answered: Option<Duration> = None;
        // Finalized entries (Case 1) are always at the front — track separately
        // to use pop_front() instead of retain() in the common case.
        let mut n_front_to_pop: usize = 0;
        let current_time = step.timestamp;

        // If there is no data in the left cache it cannot be calculated yet
        if self.left_cache.is_empty() || self.right_cache.is_empty() {
            return output_robustness;
        }

        let or_into = |acc: Option<Y>, value: Y| match acc {
            Some(acc) => Some(Y::or(acc, value)),
            None => Some(value),
        };

        // How far each operand's values are final. Outside RoSI that is the frontier.
        let (left_settled, right_settled) = if IS_ROSI {
            (
                self.settled.0.min(self.t_max.0),
                self.settled.1.min(self.t_max.1),
            )
        } else {
            (self.t_max.0, self.t_max.1)
        };

        // 2. Process the evaluation buffer for tasks
        for &t_eval in self.eval_buffer.iter() {
            let window_start_t_eval = t_eval + self.interval.start;
            // let window_start_t_eval = t_eval;
            let window_end_t_eval = t_eval + self.interval.end;

            // This is the outer `max` (Eventually)
            let mut max_robustness: Option<Y> = None;
            let mut falsified = false;

            // We can only evaluate up to the data we have.
            // We must use the minimum of the current time and the window end.
            let effective_end_time = current_time.min(window_end_t_eval);

            // Case 1 gate: both operands are settled through the end of the window.
            let window_covered =
                left_settled >= window_end_t_eval && right_settled >= window_end_t_eval;

            // Delayed mode stops at the first uncovered window, so skip walking it.
            if !IS_EAGER && !IS_ROSI && !window_covered {
                break;
            }

            // The running infimum of phi starts at the value phi holds at t_eval. If phi is
            // not known that far yet, neither this nor any later t_eval can be evaluated.
            let phi_held = (t_eval <= self.t_max.0)
                .then(|| self.left_cache.zoh_at(t_eval))
                .flatten()
                .map(|entry| held_value::<Y, IS_ROSI>(entry, t_eval, left_settled));
            let Some(phi_held) = phi_held else { break };

            // Candidate t' are the window start plus every operand breakpoint inside the
            // window: the inner expression is piecewise constant and only changes there.
            let left_from = self
                .left_cache
                .partition_point(|entry| entry.timestamp <= window_start_t_eval);
            let right_from = self
                .right_cache
                .partition_point(|entry| entry.timestamp <= window_start_t_eval);
            let mut left_ts = self
                .left_cache
                .iter()
                .skip(left_from)
                .take_while(|entry| entry.timestamp <= effective_end_time)
                .map(|entry| entry.timestamp)
                .peekable();
            let mut right_ts = self
                .right_cache
                .iter()
                .skip(right_from)
                .take_while(|entry| entry.timestamp <= effective_end_time)
                .map(|entry| entry.timestamp)
                .peekable();
            // Lazy, deduplicated merge of the window start and both breakpoint runs.
            let t_primes = (window_start_t_eval <= effective_end_time)
                .then_some(window_start_t_eval)
                .into_iter()
                .chain(std::iter::from_fn(|| {
                    let next = match (left_ts.peek(), right_ts.peek()) {
                        (Some(&l), Some(&r)) => l.min(r),
                        (Some(&l), None) => l,
                        (None, Some(&r)) => r,
                        (None, None) => return None,
                    };
                    left_ts.next_if_eq(&next);
                    right_ts.next_if_eq(&next);
                    Some(next)
                }));

            // phi samples after t_eval, folded into the running min as t' reaches them. The
            // obligation on phi is `inf over [t_eval, t']`, closed at t': phi must also hold
            // where psi does.
            let phi_from = self
                .left_cache
                .partition_point(|entry| entry.timestamp <= t_eval);
            let mut left_cache_iter = self.left_cache.iter().skip(phi_from).peekable();
            let mut left_cache_t_prime_min = phi_held;

            // The timestamp at which phi's running infimum reached `atomic_false`, if any.
            let mut phi_died_at = (left_cache_t_prime_min == Y::atomic_false()).then_some(t_eval);

            // Cursor over psi, starting at the entry in force at the window start.
            let mut psi_iter = self
                .right_cache
                .iter()
                .skip(right_from.saturating_sub(1))
                .peekable();
            let mut psi_held: Option<&Step<Y>> = None;

            for t_prime in t_primes {
                // 1. Fold phi samples in (t_eval, t'] into the cumulative min.
                while let Some(left_step) = left_cache_iter.next_if(|s| s.timestamp <= t_prime) {
                    left_cache_t_prime_min =
                        Y::and(left_cache_t_prime_min, left_step.value.clone());
                    if phi_died_at.is_none() && left_cache_t_prime_min == Y::atomic_false() {
                        phi_died_at = Some(left_step.timestamp);
                    }
                }
                let phi_known_min = left_cache_t_prime_min.clone();

                // Past phi's settled mark the obligation is unknown. RoSI keeps the upper
                // bound from the samples seen so far.
                let robustness_phi_left = if t_prime <= left_settled {
                    phi_known_min.clone()
                } else if IS_ROSI {
                    Y::and(phi_known_min.clone(), Y::unknown())
                } else {
                    Y::unknown()
                };

                // 2. rho_psi(t'): the value psi holds at t', or unknown past its frontier.
                while let Some(entry) = psi_iter.next_if(|entry| entry.timestamp <= t_prime) {
                    psi_held = Some(entry);
                }
                let robustness_psi_right = (t_prime <= self.t_max.1)
                    .then(|| psi_held.filter(|entry| entry.held_until > t_prime))
                    .flatten()
                    .map_or_else(Y::unknown, |entry| {
                        held_value::<Y, IS_ROSI>(entry, t_prime, right_settled)
                    });

                // 3. Eager falsification: once phi has died at `d`, no witness at or after
                //    `d` is possible. The window is false if psi is also known up to `d`,
                //    ruling out earlier witnesses, or if `d` precedes the window start.
                let psi_rules_out_earlier_witnesses = phi_died_at
                    .is_some_and(|died| died < window_start_t_eval || self.t_max.1 >= died);
                if IS_EAGER && psi_rules_out_earlier_witnesses && t_max_combined >= t_eval {
                    falsified = true;
                    max_robustness = or_into(max_robustness, Y::atomic_false());
                    break;
                }

                // 4. Combine: min(rho_psi(t'), robustness_phi_left)
                let robustness_t_prime = Y::and(robustness_psi_right, robustness_phi_left);
                max_robustness = or_into(max_robustness, robustness_t_prime);
            }

            let Some(max_robustness) = max_robustness else {
                break; // No data to evaluate yet
            };

            // ---
            // **State-based Eager/delayed/ROSI logic**
            // ---
            let final_value: Option<Y>;
            let mut remove_task = false;

            if window_covered {
                // Case 1: Full window covered by both operands.
                // This is a final, "closed" value for delayed mode.
                // (This also captures Eager results that were `false` until the end)
                final_value = Some(max_robustness);
                remove_task = true;
                n_front_to_pop += 1;
            } else if IS_EAGER && max_robustness == Y::atomic_true() {
                // Case 2a: Eager short-circuit (Satisfaction). Found "true" before window closed.
                final_value = Some(max_robustness); // which is Y::atomic_true()
                remove_task = true;
            } else if IS_EAGER && falsified {
                // Case 2b: Eager short-circuit (Falsification). Found "false" before window closed.
                final_value = Some(max_robustness); // which is Y::atomic_false()
                remove_task = true;
            } else if IS_ROSI {
                // Case 3: Intermediate ROSI. Window is still open, no short-circuit.
                // We must account for unknown future contributions. For Until, the
                // outer sup (max_robustness) should be widened with unknown() so
                // that subsequent negations or compositions see the correct
                // refinable bounds (mirrors behavior in Eventually/Globally).
                let intermediate_value = Y::or(max_robustness, Y::unknown());
                final_value = Some(intermediate_value);
                // DO NOT remove task, it's not finished
            } else {
                // Case 4: Cannot evaluate yet (e.g., Delayed/Eager bool/f64 and window is still open)
                // Since the buffer is time-ordered, we stop.
                break;
            }

            if let Some(val) = final_value {
                output_robustness.push(Step::new("output", val, t_eval));
            }

            if remove_task {
                tasks_to_remove.push(t_eval);
                newest_answered = Some(t_eval);
            }
        }

        if let Some(answered) = newest_answered {
            self.finalized_ts = Some(answered);
        }

        // 3. Prune the caches and remove completed tasks from the buffer.
        //
        // Any timestamp after `finalized_ts` can still become a task, so pruning keeps
        // the values in force from the older of that and the buffer front.
        let task_floor = self.finalized_ts.unwrap_or(Duration::ZERO);
        let protected_ts = self
            .eval_buffer
            .front()
            .map_or(task_floor, |front| (*front).min(task_floor));
        let lookahead = self.max_lookahead;
        guarded_prune(&mut self.left_cache, lookahead, protected_ts);
        guarded_prune(&mut self.right_cache, lookahead, protected_ts);

        // Pop finalized prefix from front (Case 1: always contiguous).
        for _ in 0..n_front_to_pop {
            self.eval_buffer.pop_front();
        }
        if tasks_to_remove.len() > n_front_to_pop {
            let non_front = &tasks_to_remove[n_front_to_pop..];
            self.eval_buffer.retain(|t| !non_front.contains(t));
        }

        output_robustness
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> SignalIdentifier
    for Until<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Returns the union of signal identifiers from left and right operands.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        self.left_signals_set
            .extend(self.left.get_signal_identifiers());
        self.right_signals_set
            .extend(self.right.get_signal_identifiers());

        let mut ids = self.left_signals_set.clone();
        ids.extend(self.right_signals_set.iter().cloned());
        ids
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Display
    for Until<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Formats as `(left) U[start, end] (right)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "({}) U[{}, {}] ({})",
            self.left,
            self.interval.start.as_secs_f64(),
            self.interval.end.as_secs_f64(),
            self.right
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{StlOperatorTrait, TimeInterval};
    use crate::operators::atomic_operators::Atomic;
    use crate::ring_buffer::RingBuffer;
    use crate::step;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn debug_rosi() {
        use crate::core::RobustnessInterval;
        use crate::operators::unary_temporal_operators::{Eventually, Globally};
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(6),
        };
        let interval_2 = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(2),
        };
        let atomic_left = Atomic::<RobustnessInterval>::new_greater_than("x", 0.0);
        let atomic_right = Atomic::<RobustnessInterval>::new_greater_than("x", 3.0);

        let globally = Globally::<
            f64,
            RingBuffer<RobustnessInterval>,
            RobustnessInterval,
            false,
            true,
        >::new(interval_2, Box::new(atomic_left), None, None);

        let eventually = Eventually::<
            f64,
            RingBuffer<RobustnessInterval>,
            RobustnessInterval,
            false,
            true,
        >::new(interval_2, Box::new(atomic_right), None, None);

        let mut until =
            Until::<f64, RingBuffer<RobustnessInterval>, RobustnessInterval, false, true>::new(
                interval,
                Box::new(globally),
                Box::new(eventually),
                None,
                None,
            );
        println!("Until operator: {}", until);

        let signals = vec![
            step!("x", 1.0, Duration::from_secs(0)),
            step!("x", 2.0, Duration::from_secs(1)),
            step!("x", 3.0, Duration::from_secs(2)),
            step!("x", 8.0, Duration::from_secs(3)),
            step!("x", 10.0, Duration::from_secs(6)),
            step!("x", 15.0, Duration::from_secs(8)),
        ];
        for signal in signals {
            let outputs = until.update(&signal);
            let outputs_globally = until.left.update(&signal);
            let outputs_eventually = until.right.update(&signal);
            println!("Output at signal t={:?}:", signal.timestamp);
            for output in outputs_globally {
                println!(
                    "  Globally t={:?}:\n   {:?}",
                    output.timestamp, output.value
                );
            }
            for output in outputs_eventually {
                println!(
                    "  Eventually t={:?}:\n   {:?}",
                    output.timestamp, output.value
                );
            }
            for output in outputs {
                println!("  Until t={:?}:\n   {:?}", output.timestamp, output.value);
            }
            println!("---");
        }
    }

    #[test]
    fn debug_eager() {
        use crate::operators::unary_temporal_operators::{Eventually, Globally};
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let atomic_left = Atomic::<f64>::new_greater_than("x", 2.0);
        let atomic_right = Atomic::<f64>::new_greater_than("x", 8.0);
        let mut globally = Globally::<f64, RingBuffer<f64>, f64, true, false>::new(
            interval,
            Box::new(atomic_left.clone()),
            None,
            None,
        );
        let mut eventually = Eventually::<f64, RingBuffer<f64>, f64, true, false>::new(
            interval,
            Box::new(atomic_right.clone()),
            None,
            None,
        );

        let mut until = Until::<f64, RingBuffer<f64>, f64, true, false>::new(
            interval,
            Box::new(globally.clone()),
            Box::new(eventually.clone()),
            None,
            None,
        );
        println!("Until operator: {}", until);

        let signals = vec![
            step!("x", 1.0, Duration::from_secs(0)),
            step!("x", 2.0, Duration::from_secs(1)),
            step!("x", 3.0, Duration::from_secs(2)),
            step!("x", 8.0, Duration::from_secs(3)),
            step!("x", 12.0, Duration::from_secs(6)),
            step!("x", 15.0, Duration::from_secs(8)),
        ];
        println!("Until operator: {}", until);

        for signal in signals {
            let outputs = until.update(&signal);
            let outputs_globally = globally.update(&signal);
            let outputs_eventually = eventually.update(&signal);
            println!("Output at signal t={:?}:", signal.timestamp);
            for output in outputs_globally {
                println!("  Globally t={:?}:   {:?}", output.timestamp, output.value);
            }
            for output in outputs_eventually {
                println!(
                    "  Eventually t={:?}:   {:?}",
                    output.timestamp, output.value
                );
            }
            for output in outputs {
                println!("  Until t={:?}:   {:?}", output.timestamp, output.value);
            }
            println!("---");
        }
    }

    #[test]
    fn debug_bool() {
        use crate::operators::unary_temporal_operators::{Eventually, Globally};
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let atomic_left = Atomic::<bool>::new_greater_than("x", 2.0);
        let atomic_right = Atomic::<bool>::new_greater_than("x", 8.0);
        let mut globally = Globally::<f64, RingBuffer<bool>, bool, true, false>::new(
            interval,
            Box::new(atomic_left.clone()),
            None,
            None,
        );
        let mut eventually = Eventually::<f64, RingBuffer<bool>, bool, true, false>::new(
            interval,
            Box::new(atomic_right.clone()),
            None,
            None,
        );

        let mut until = Until::<f64, RingBuffer<bool>, bool, true, false>::new(
            interval,
            Box::new(globally.clone()),
            Box::new(eventually.clone()),
            None,
            None,
        );
        println!("Until operator: {}", until);

        let signals = vec![
            step!("x", 1.0, Duration::from_secs(0)),
            step!("x", 2.0, Duration::from_secs(1)),
            step!("x", 3.0, Duration::from_secs(2)),
            step!("x", 8.0, Duration::from_secs(3)),
            step!("x", 12.0, Duration::from_secs(6)),
            step!("x", 15.0, Duration::from_secs(8)),
        ];
        for signal in signals {
            let outputs = until.update(&signal);
            let outputs_globally = globally.update(&signal);
            let outputs_eventually = eventually.update(&signal);
            println!("Output at signal t={:?}:", signal.timestamp);
            for output in outputs_globally {
                println!("  Globally t={:?}:   {:?}", output.timestamp, output.value);
            }
            for output in outputs_eventually {
                println!(
                    "  Eventually t={:?}:   {:?}",
                    output.timestamp, output.value
                );
            }
            for output in outputs {
                println!("  Until t={:?}:   {:?}", output.timestamp, output.value);
            }
            println!("---");
        }
    }

    #[test]
    fn until_operator_robustness() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let atomic_left = Atomic::<bool>::new_less_than("x", 10.0);
        let atomic_right = Atomic::<bool>::new_greater_than("x", 5.0);
        let mut until = Until::<f64, RingBuffer<bool>, bool, false, false>::new(
            interval,
            Box::new(atomic_left),
            Box::new(atomic_right),
            None,
            None,
        );
        until.get_signal_identifiers();
        let signal_values = vec![2.0, 2.0, 2.0, 6.0, 12.0];
        let signal_timestamps = vec![0, 2, 4, 6, 8];

        let signal: Vec<_> = signal_values
            .into_iter()
            .zip(signal_timestamps)
            .map(|(val, ts)| step!("x", val, Duration::from_secs(ts)))
            .collect();

        let expected_outputs = [
            step!("output", false, Duration::from_secs(0)),
            step!("output", true, Duration::from_secs(2)),
            step!("output", true, Duration::from_secs(4)),
        ];

        let mut all_outputs = Vec::new();
        for s in &signal {
            let up = until.update(s);
            println!("Updates at t={:?}: {:?}", s.timestamp, up);
            all_outputs.extend(up);
        }

        assert_eq!(all_outputs.len(), expected_outputs.len());
        for (output, expected) in all_outputs.iter().zip(expected_outputs.iter()) {
            assert_eq!(output.timestamp, expected.timestamp);
            assert_eq!(output.value, expected.value);
        }
    }
    #[test]
    fn until_operator_robustness_nonzero_lowbound() {
        let interval = TimeInterval {
            start: Duration::from_secs(3),
            end: Duration::from_secs(4),
        };
        let atomic_left = Atomic::<bool>::new_less_than("x", 10.0);
        let atomic_right = Atomic::<bool>::new_greater_than("x", 5.0);
        let mut until = Until::<f64, RingBuffer<bool>, bool, false, false>::new(
            interval,
            Box::new(atomic_left),
            Box::new(atomic_right),
            None,
            None,
        );
        until.get_signal_identifiers();
        let signal_values = vec![2.0, 6.0, 2.0, 2.0, 6.0, 12.0];
        let signal_timestamps = vec![0, 2, 3, 4, 6, 8];

        let signal: Vec<_> = signal_values
            .into_iter()
            .zip(signal_timestamps)
            .map(|(val, ts)| step!("x", val, Duration::from_secs(ts)))
            .collect();

        let expected_outputs = [
            step!("output", false, Duration::from_secs(0)), // x>5 inbetween [3,4], which it isn't
            // t = 1 is the breakpoint at 4s shifted by the lower bound. x holds at 2 over
            // its window [4, 5].
            step!("output", false, Duration::from_secs(1)),
            step!("output", true, Duration::from_secs(2)),
            step!("output", true, Duration::from_secs(3)),
            step!("output", true, Duration::from_secs(4)),
        ];

        let mut all_outputs = Vec::new();
        for s in &signal {
            let up = until.update(s);
            println!("Updates at t={:?}: {:?}", s.timestamp, up);
            all_outputs.extend(up);
        }

        assert_eq!(all_outputs.len(), expected_outputs.len());
        for (output, expected) in all_outputs.iter().zip(expected_outputs.iter()) {
            assert_eq!(
                output.timestamp, expected.timestamp,
                "output timestamp: {:?} != expected timestamp: {:?}",
                output.timestamp, expected.timestamp
            );
            assert_eq!(
                output.value, expected.value,
                "output value: {:?} != expected value: {:?}",
                output.value, expected.value
            );
        }
    }

    /// A window must not be finalized while psi is still behind.
    ///
    /// `x > 0` holds throughout and `y > 5` becomes true at 1s, inside the window `[0, 2]`
    /// of the evaluation at 0s. The answer at 0s must wait for `y` to cover the window,
    /// and then be `true`.
    #[test]
    fn until_does_not_finalize_before_psi_frontier_covers_the_window() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(2),
        };
        let mut until = Until::<f64, RingBuffer<bool>, bool, false, false>::new(
            interval,
            Box::new(Atomic::<bool>::new_greater_than("x", 0.0)),
            Box::new(Atomic::<bool>::new_greater_than("y", 5.0)),
            None,
            None,
        );
        until.get_signal_identifiers();

        let at_zero = |outputs: &[Step<bool>]| {
            outputs
                .iter()
                .find(|s| s.timestamp == Duration::from_secs(0))
                .map(|s| s.value)
        };

        let mut all_outputs = Vec::new();
        // `x` runs ahead to 3s while `y` is still at 0s.
        for step in [
            step!("x", 1.0, Duration::from_secs(0)),
            step!("y", 0.0, Duration::from_secs(0)),
            step!("x", 1.0, Duration::from_secs(3)),
            step!("y", 10.0, Duration::from_secs(1)),
        ] {
            all_outputs.extend(until.update(&step));
        }

        // `y` has only reached 1s, short of the window end at 2s, so 0s is undecided.
        assert_eq!(
            at_zero(&all_outputs),
            None,
            "0s was finalized while psi only reached 1s; got {all_outputs:?}"
        );

        // `y` now covers the window end, so the window closes.
        all_outputs.extend(until.update(&step!("y", 10.0, Duration::from_secs(5))));
        assert_eq!(
            at_zero(&all_outputs),
            Some(true),
            "psi becomes true at 1s, inside the window [0, 2]; got {all_outputs:?}"
        );
    }

    #[test]
    fn until_signal_identifiers() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let atomic_left = Atomic::<bool>::new_greater_than("x", 5.0);
        let atomic_right = Atomic::<bool>::new_less_than("y", 10.0);
        let mut until = Until::<f64, RingBuffer<bool>, bool, false, false>::new(
            interval,
            Box::new(atomic_left),
            Box::new(atomic_right),
            None,
            None,
        );
        let ids = until.get_signal_identifiers();
        let expected_ids: HashSet<&'static str> = vec!["x", "y"].into_iter().collect();
        assert_eq!(ids, expected_ids);
    }

    #[test]
    fn total_size_includes_children() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let a_left = Atomic::<f64>::new_greater_than("x", 5.0);
        let a_right = Atomic::<f64>::new_less_than("y", 10.0);
        let child_sum = <Atomic<f64> as StlOperatorTrait<f64>>::total_size(&a_left)
            + <Atomic<f64> as StlOperatorTrait<f64>>::total_size(&a_right);
        let until = Until::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(a_left),
            Box::new(a_right),
            None,
            None,
        );
        assert!(until.total_size() >= child_sum + std::mem::size_of_val(&until));
    }

    #[test]
    fn total_size_after_data() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let a_left = Atomic::<f64>::new_greater_than("x", 5.0);
        let a_right = Atomic::<f64>::new_less_than("x", 10.0);
        let mut until = Until::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(a_left),
            Box::new(a_right),
            None,
            None,
        );
        until.get_signal_identifiers();
        let before = until.total_size();
        until.update(&step!("x", 7.0, Duration::from_secs(0)));
        assert!(until.total_size() >= before + std::mem::size_of::<Step<f64>>());
    }

    #[test]
    fn until_display() {
        let interval = TimeInterval {
            start: Duration::from_secs(1),
            end: Duration::from_secs(5),
        };
        let atomic_left = Atomic::<f64>::new_greater_than("x", 0.0);
        let atomic_right = Atomic::<f64>::new_less_than("y", 10.0);
        let until = Until::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(atomic_left),
            Box::new(atomic_right),
            None,
            None,
        );
        assert_eq!(format!("{until}"), "(x > 0) U[1, 5] (y < 10)");
    }
}
