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
    eval_buffer: VecDeque<Duration>,
    left_signals_set: HashSet<&'static str>,
    right_signals_set: HashSet<&'static str>,
    max_lookahead: Duration,
    /// Timestamp of the first operand output ever seen. A shifted evaluation timestamp
    /// earlier than this is dropped: neither operand has a value to report there.
    first_ts: Option<Duration>,
    /// Newest evaluation timestamp already answered for good; a shifted timestamp at or
    /// before it is dropped rather than re-queued into a window that has closed.
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
                eval_buffer: VecDeque::new(),
                left_signals_set: HashSet::new(),
                right_signals_set: HashSet::new(),
                max_lookahead,
                first_ts: None,
                finalized_ts: None,
            }
        }
    }

    /// Adds a step to a cache.
    ///
    /// In RoSI mode this performs update-or-insert by timestamp; otherwise it
    /// appends the step.
    /// Returns `true` when the step landed *behind* the newest one already cached.
    fn add_to_cache<const ROSI: bool>(cache: &mut C, step: Step<Y>) -> bool
    where
        C: RingBufferTrait<Value = Y>,
        Y: Clone,
    {
        if ROSI {
            if !cache.update_step(step.clone()) {
                cache.add_step(step);
            }
            return false;
        }
        let is_late = cache
            .get_back()
            .is_some_and(|back| step.timestamp < back.timestamp);
        // Not `add_step`: an eager child emits a short-circuit ahead of its joint frontier
        // and fills the gap behind it later, so a breakpoint can arrive earlier than one
        // already cached. Appending it there would leave the cache unsorted, and every
        // `zoh_at` read after that answers from the wrong step.
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

    /// A hole in this operator's output is a window start that has not been answered yet but
    /// still lies below one that has. `finalized_ts` is that upper mark, and over gap-free
    /// operands it is the whole answer: their breakpoints arrive in ascending order, every
    /// window start they open below the mark is refused, and the answered prefix stays solid.
    ///
    /// Over an operand with holes it is not. A breakpoint arriving behind that operand's
    /// newest is let through that gate, precisely so the window it opens is not lost, and it
    /// opens one as far back as `ts - interval.end`. Only the stretch below the earliest such
    /// window start is settled.
    ///
    /// Delayed and RoSI emit windows in order and have no holes. See
    /// [`StlOperatorTrait::known_through`].
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

        // An operand is known only as far as it is gap-free. `t_max` gates both the
        // `phi_held` read and the window-close test, and holding phi or psi across a hole
        // an eager binary left behind would close a window against a value that operand
        // never asserted. See [`StlOperatorTrait::known_through`].
        //
        // Applied after the `max`, since this bound can fall as well as rise.
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
        // A breakpoint arriving behind one already cached lands inside a stretch the
        // operand had not reported when the windows there were answered, so those windows
        // have to be opened again. `finalized_ts` must not veto it.
        let mut late_ts: Vec<Duration> = Vec::new();
        for update in &right_updates {
            all_ts.push(update.timestamp);
            if Self::add_to_cache::<IS_ROSI>(&mut self.right_cache, update.clone()) {
                late_ts.push(update.timestamp);
            }
        }
        for update in &left_updates {
            all_ts.push(update.timestamp);
            if Self::add_to_cache::<IS_ROSI>(&mut self.left_cache, update.clone()) {
                late_ts.push(update.timestamp);
            }
        }
        all_ts.sort();
        all_ts.dedup();
        // A breakpoint `ts` of either operand queues up to three evaluation timestamps. The
        // satisfaction signal of `phi U[a,b] psi` changes only where a window boundary
        // crosses an operand breakpoint, which is at `ts - a` and `ts - b`; evaluating only
        // at `ts` misses an interval of satisfaction that opens and closes between two
        // breakpoints. `ts` itself is kept so the operator still answers at the timestamps
        // that were submitted to it.
        for ts in all_ts {
            let earliest = *self.first_ts.get_or_insert(ts);
            for candidate in [
                Some(ts),
                ts.checked_sub(self.interval.start),
                ts.checked_sub(self.interval.end),
            ] {
                // Re-opening a window already answered is worth it only when this
                // breakpoint could change the answer. That needs both: it has to have
                // arrived behind the operand's newest, landing inside a stretch that was
                // unknown when the window was answered, and the window must not already
                // have been decided on data covering it in full.
                //
                // Without the first condition an eager short-circuit is re-answered with
                // the same value on every later sample, for the rest of the trace.
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

        // 2. Process the evaluation buffer for tasks
        for &t_eval in self.eval_buffer.iter() {
            let window_start_t_eval = t_eval + self.interval.start;
            // let window_start_t_eval = t_eval;
            let window_end_t_eval = t_eval + self.interval.end;

            // This is the outer `max` (Eventually)
            let mut max_robustness_vec = Vec::new();
            let mut falsified = false;

            // We can only evaluate up to the data we have.
            // We must use the minimum of the current time and the window end.
            let effective_end_time = current_time.min(window_end_t_eval);

            // phi must hold from t_eval onwards, so the running infimum starts at the value
            // phi holds *at* t_eval. That value is generally carried by an earlier sample:
            // a shifted evaluation timestamp need not be a breakpoint of phi at all. If phi
            // is not known that far yet this t_eval cannot be evaluated, and neither can any
            // later one, so stop.
            let phi_held = (t_eval <= self.t_max.0)
                .then(|| self.left_cache.zoh_at(t_eval))
                .flatten()
                .map(|entry| entry.value.clone());
            let Some(phi_held) = phi_held else { break };

            // Candidate t'. Both operands are piecewise constant, so
            // `min(psi(t'), inf over [t_eval, t') of phi)` is piecewise constant too, and its
            // supremum over the window is attained either at the window start or at an
            // operand breakpoint inside it. The candidates therefore come from the caches
            // and not from `eval_buffer`: a t' matters because an operand changes there, not
            // because a verdict happens to have been asked for there.
            // Both caches are ascending, so the breakpoints inside the window form a
            // contiguous run -- binary search for its start, and stop at its end.
            let left_from = self
                .left_cache
                .partition_point(|entry| entry.timestamp <= window_start_t_eval);
            let right_from = self
                .right_cache
                .partition_point(|entry| entry.timestamp <= window_start_t_eval);
            // Each side is ascending, so their union is a merge of the two runs, dropping
            // duplicates as they appear. The window start comes first: every breakpoint
            // taken is strictly after it.
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
            let mut t_primes: Vec<Duration> = Vec::new();
            if window_start_t_eval <= effective_end_time {
                t_primes.push(window_start_t_eval);
            }
            loop {
                let next = match (left_ts.peek(), right_ts.peek()) {
                    (Some(&l), Some(&r)) => l.min(r),
                    (Some(&l), None) => l,
                    (None, Some(&r)) => r,
                    (None, None) => break,
                };
                left_ts.next_if_eq(&next);
                right_ts.next_if_eq(&next);
                if t_primes.last() != Some(&next) {
                    t_primes.push(next);
                }
            }

            // phi samples after t_eval, folded into the running min as t' reaches them.
            //
            // The obligation on phi is `inf over [t_eval, t']`, **closed at t'**: phi is
            // required to hold at the very time-point where psi holds. This is the STL
            // convention of Maler-Nickovic and Donze-Maler, and it differs deliberately
            // from the conventional LTL/MTL until, which uses the half-open `[t, t')`.
            // The two diverge whenever a breakpoint of phi lands exactly on the witness --
            // with sample-aligned breakpoints that is common, not measure-zero.
            //
            // Flip to the half-open form by seeding with `Y::globally_identity()`, folding
            // `phi_held` in only once `t_prime > t_eval`, and changing the `<= t_prime`
            // fold below to `< t_prime`.
            let phi_from = self
                .left_cache
                .partition_point(|entry| entry.timestamp <= t_eval);
            let mut left_cache_iter = self.left_cache.iter().skip(phi_from).peekable();
            let mut left_cache_t_prime_min = phi_held;

            // The instant phi's running infimum died, or `None` while it still holds.
            //
            // `atomic_false` is the absorbing element of `and` in every semantics
            // (`false`, `-inf`, the empty interval), so once the fold reaches it the
            // infimum stays there and this is recorded exactly once. It is the timestamp
            // the eager falsification check needs -- not the `t'` the walk happens to have
            // reached when it notices, which can be arbitrarily later.
            let mut phi_died_at = (left_cache_t_prime_min == Y::atomic_false()).then_some(t_eval);

            // Cursor over psi, positioned at the entry in force at the window start:
            // `right_from` is the first entry strictly after it, so its predecessor is
            // the one holding there.
            let mut psi_iter = self
                .right_cache
                .iter()
                .skip(right_from.saturating_sub(1))
                .peekable();
            let mut psi_held: Option<&Step<Y>> = None;

            for t_prime in t_primes {
                // 1. Fold phi samples in (t_eval, t'] into the cumulative min. Inclusive of
                //    t' itself: phi must hold where psi does.
                while let Some(left_step) = left_cache_iter.next_if(|s| s.timestamp <= t_prime) {
                    left_cache_t_prime_min =
                        Y::and(left_cache_t_prime_min, left_step.value.clone());
                    if phi_died_at.is_none() && left_cache_t_prime_min == Y::atomic_false() {
                        phi_died_at = Some(left_step.timestamp);
                    }
                }
                // The min over the phi samples that have actually arrived.
                let phi_known_min = left_cache_t_prime_min.clone();

                // phi is known only as far as `t_max.0`. Past that the newest cache entry
                // reads as holding indefinitely, because a piecewise-constant signal has no
                // way to say "and then nothing" -- so an obligation nobody has verified
                // would be accepted as satisfied, and a witness admitted on it. That is the
                // same unsoundness `t_max.1` already guards against for psi on the line
                // below, and it is the direction eager short-circuits on, so it decides a
                // whole `Until` true off the strength of it.
                //
                // Substituted exactly the way psi is on the line below, a bare
                // `Y::unknown()`, because what that value means is semantics-specific and
                // only the semantics knows it: `false` for the qualitative ones, `NaN` for
                // `DelayedQuantitative`, the unbounded interval for RoSI. Folding it in
                // with `and` instead would push RoSI's lower bound to negative infinity and
                // take a min against `NaN`, neither of which says "not known yet".
                //
                // RoSI is excluded. Its verdicts are refinable by construction, so an
                // over-optimistic intermediate is corrected on the next update rather than
                // frozen -- the unsoundness being guarded here is specifically that a
                // *final* verdict rests on an unverified obligation. And its `unknown()` is
                // the unbounded interval, which carries no information at all: substituting
                // it leaves the enclosing `and` unable to report a bound, so the operator
                // emits nothing and the refinement never starts.
                let robustness_phi_left = if IS_ROSI || t_prime <= self.t_max.0 {
                    phi_known_min.clone()
                } else {
                    Y::unknown()
                };

                // 2. rho_psi(t'): the value psi holds at t'. t' is as often a breakpoint of
                //    phi, or a bare window start, as it is a sample of psi.
                // `t_primes` ascends, so a cursor over the cache tracks the entry psi
                // holds at t'. The `held_until` test is the one `zoh_at` makes: a sample
                // that has been superseded no longer answers.
                while let Some(entry) = psi_iter.next_if(|entry| entry.timestamp <= t_prime) {
                    psi_held = Some(entry);
                }
                let robustness_psi_right = (t_prime <= self.t_max.1)
                    .then(|| psi_held.filter(|entry| entry.held_until > t_prime))
                    .flatten()
                    .map_or_else(Y::unknown, |entry| entry.value.clone());

                // 3. Eager falsification check: if phi has become false, short-circuit.
                //
                //    phi dying at `d` rules out every witness from `d` onwards -- the
                //    obligation `inf over [t_eval, t'']` is closed at `t''` and monotone
                //    non-increasing in `t''`, so it is false for all `t'' >= d`. It says
                //    nothing about the witnesses *before* `d`: phi's obligation runs up to
                //    each witness separately, and an earlier one is unaffected by a later
                //    failure. So falsifying the window means ruling those out too, and that
                //    needs psi to have actually reported across `[window_start, d)`. Where
                //    it has not, `robustness_psi_right` falls back to `Y::unknown()`, which
                //    for `bool` is `false` and is indistinguishable here from a psi that
                //    genuinely does not hold.
                //
                //    Hence the gate is on `d` and not on `t_prime`. Gating on `t_prime` --
                //    the point the walk has reached when it *notices* -- is sound but
                //    needlessly strong, and it deadlocks: the check would wait on psi data
                //    from beyond the stretch it actually has to rule out, the task would
                //    stay in `eval_buffer` unanswered, the front prefix would block behind
                //    it, and eager would go quiet while its last verdict went stale. That
                //    measured a net loss (24 wrong -> 36). Gating on `d` asks for exactly
                //    the data the conclusion rests on, so the wait is bounded by it.
                //
                //    `d < window_start_t_eval` needs no psi at all: phi died before the
                //    first admissible witness, so there is no witness to rule out. This is
                //    the run-up `[t_eval, t_eval + a)`, empty whenever `a == 0`.
                //
                //    The death is read off the *known* min, not the frontier-gated one: a
                //    phi that is merely unverified past `t_max.0` must not read as a
                //    violation.
                let psi_rules_out_earlier_witnesses = phi_died_at
                    .is_some_and(|died| died < window_start_t_eval || self.t_max.1 >= died);
                if IS_EAGER && psi_rules_out_earlier_witnesses && t_max_combined >= t_eval {
                    falsified = true;
                    max_robustness_vec.push(Y::atomic_false());
                    break;
                }

                // 4. Combine: min(rho_psi(t'), robustness_phi_left)
                let robustness_t_prime = Y::and(robustness_psi_right, robustness_phi_left);
                max_robustness_vec.push(robustness_t_prime);
            }

            let max_robustness = if max_robustness_vec.is_empty() {
                break; // No data to evaluate yet
            } else {
                max_robustness_vec.into_iter().reduce(Y::or).unwrap()
            };

            // ---
            // **State-based Eager/delayed/ROSI logic**
            // ---
            let final_value: Option<Y>;
            let mut remove_task = false;

            // Case 1 gate: both operands are known through the end of the window.
            //
            // This must be tested per operand. `step.timestamp` is the arrival clock of
            // *whichever* signal moved last, so with phi and psi on different signals a
            // burst on phi's signal drives it past the horizon while psi is still behind.
            // The tail of the window then reads as `Y::unknown()` -- which for `bool` is
            // `false` (`core.rs`, `impl RobustnessSemantics for bool`) -- and folds into
            // the outer `or` as a genuine `false`. Case 1 would close the window on that,
            // and `finalized_ts` would refuse to re-open it when psi finally arrived.
            //
            // `t_max` carries each operand's own output frontier, so requiring both to
            // reach the window end closes it exactly when the data to decide it is in
            // hand, and no earlier. Child lookaheads need no separate term: a child's
            // frontier only advances when that child could answer.
            //
            // phi is read over `[t_eval, t']` and psi over `[t_eval + a, t']`, with t' up
            // to the window end, so the window end is the bound for both.
            let window_covered = if IS_ROSI {
                current_time >= t_eval + self.get_max_lookahead()
            } else {
                self.t_max.0 >= window_end_t_eval && self.t_max.1 >= window_end_t_eval
            };

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

        // A shifted timestamp derived later from a sample that has only just arrived must
        // not re-open a window that has already been answered for good. A breakpoint
        // arriving *behind* the operand's newest is the exception, and is let through at the
        // enqueue site above.
        if let Some(answered) = newest_answered {
            self.finalized_ts = Some(answered);
        }

        // 3. Prune the caches and remove completed tasks from the buffer.
        //
        // Protecting only `eval_buffer.front()` is not enough. Any timestamp above
        // `finalized_ts` can still *become* a task: a breakpoint that has not arrived yet
        // queues `ts - a` and `ts - b` as well as `ts`, and those shifts can land earlier
        // than anything currently in the buffer. Eager makes this routine -- it finalizes
        // windows in whatever order they resolve, so the front runs ahead of the floor --
        // but it is not eager-specific.
        //
        // What such a task needs is the phi/psi sample *in force* at it, which may predate
        // it by any amount. Pruned, `zoh_at` returns `None`, the `phi_held` guard above
        // breaks out of the loop, and because the buffer is processed front-first every
        // later task stalls behind the unanswerable one: the operator goes quiet for good
        // and its last verdict is held stale by anything reading the stream.
        //
        // Both bounds advance, so this stays bounded. It also *tightens* the empty-buffer
        // case, which used to fall back to `ZERO` and suppress pruning entirely.
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
            // t = 1 is the breakpoint at 4s shifted by the lower bound. Its window is
            // [4, 5], over which x is held at 2 by the sample at 4s, so x > 5 is false
            // throughout. No sample of the signal lies in that window.
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
    /// Case 1 closes a window on `current_time >= t_eval + max_lookahead`, where
    /// `current_time` is the timestamp of whichever signal just arrived. With phi on `x`
    /// and psi on `y`, an `x` sample can therefore close a window that `y` has not yet
    /// reached. Every `t_prime` past `t_max.1` then reads as `Y::unknown()`, which is
    /// `false` for `bool`, and the verdict is finalized on that substitution -- with
    /// `finalized_ts` refusing to re-open it once `y` does arrive.
    ///
    /// Here `x > 0` holds throughout and `y > 5` becomes true at 1s, inside the window
    /// `[0, 2]` of the evaluation at 0s, so the answer at 0s is `true`.
    ///
    /// The test pins both halves, because passing only the first would be trivial -- an
    /// operator that never finalizes anything would satisfy it:
    ///
    /// 1. while `y`'s frontier is short of the window end, 0s must not be answered at all;
    /// 2. once `y` covers the window end, 0s must be answered `true`.
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
        // `x` runs ahead to 3s while `y` is still at 0s, which is what drags
        // `current_time` past the horizon of the window opening at 0s.
        for step in [
            step!("x", 1.0, Duration::from_secs(0)),
            step!("y", 0.0, Duration::from_secs(0)),
            step!("x", 1.0, Duration::from_secs(3)),
            step!("y", 10.0, Duration::from_secs(1)),
        ] {
            all_outputs.extend(until.update(&step));
        }

        // `y` has only reached 1s, short of the window end at 2s, so 0s is undecided.
        // Answering it here means answering it on `unknown()`, i.e. on `false`.
        assert_eq!(
            at_zero(&all_outputs),
            None,
            "0s was finalized while psi only reached 1s; got {all_outputs:?}"
        );

        // `y` now covers the window end, so the window closes -- on real data this time.
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
