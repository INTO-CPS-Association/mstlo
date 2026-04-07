//! Unary temporal STL operators (`Eventually`, `Globally`).
//!
//! This module implements sliding-window temporal evaluation over an operand
//! stream, with support for delayed, eager, and (RoSI) execution via
//! const generics.

use crate::ring_buffer::{RingBufferTrait, Step, guarded_prune};
use crate::stl::core::{
    RobustnessSemantics, SignalIdentifier, StlOperatorAndSignalIdentifier, StlOperatorTrait,
    TimeInterval,
};
use std::collections::{BTreeSet, HashSet};
use std::fmt::{Debug, Display};
use std::time::Duration;

/// Returns `true` if it is safe to evict the cache entry at `back_ts` in favour
/// of a new entry arriving at `new_ts`.
///
/// Lemire back-eviction is only valid when every pending evaluation task whose
/// window contains `back_ts` also contains `new_ts`.  Evicting prematurely
/// removes an entry that a pending task still needs, which corrupts the result.
///
/// The check uses a sufficient O(1) condition based solely on the *oldest*
/// pending task (the tightest constraint in the evaluation buffer):
///
/// * **Condition A** – `oldest + b ≥ new_ts`: the oldest pending task's window
///   right-edge still reaches `new_ts`, so `new_ts` is inside every pending
///   window.  Safe to evict.
/// * **Condition B** – `oldest > back_ts − a`: even the oldest pending task's
///   window starts after `back_ts`, meaning no pending window contains
///   `back_ts` at all.  Safe to evict.
///
/// If neither condition holds, eviction is conservatively deferred.  Any entry
/// retained this way is a true minimum/maximum candidate for some still-open
/// window and will be cleaned up by `guarded_prune` once no longer needed.
fn is_lemire_eviction_safe(
    back_ts: Duration,
    new_ts: Duration,
    interval: &TimeInterval,
    eval_buffer: &BTreeSet<Duration>,
) -> bool {
    let Some(&oldest) = eval_buffer.first() else {
        return true; // No pending evaluations — unconditionally safe.
    };
    // Condition A: oldest + b >= new_ts  <=>  oldest >= new_ts - b
    if oldest >= new_ts.saturating_sub(interval.end) {
        return true;
    }
    // Condition B: oldest > back_ts - a
    if oldest > back_ts.saturating_sub(interval.start) {
        return true;
    }
    false
}

/// Removes dominated values from the back of a monotone cache (Lemire
/// sliding min/max), guarded by an O(1) safety check.
///
/// `is_max = true` is used for `Eventually`; `is_max = false` for `Globally`.
///
/// Each entry is pushed at most once and evicted at most once, so the total
/// cost across all insertions is O(n) — amortised O(1) per step.
///
/// The safety gate ensures that an entry is only evicted once every pending
/// evaluation task whose window covers that entry also covers the new entry.
/// For dense (gap = 1) timestamps the gate always passes, so Lemire behaves
/// exactly as before.  For sparse timestamps where a gap exceeds the window
/// width the gate blocks the eviction; the entry stays and is later cleaned
/// up by `guarded_prune`.  The window query uses a bounded scan
/// (`skip_while`/`take_while`) so it remains correct in both cases: with a
/// healthy Lemire deque the scan terminates at the monotone-minimum front
/// entry in O(1); with a sparse-timestamp gap the window contains at most one
/// entry, also O(1).
///
/// ### Eager mode (`is_eager = true`)
///
/// In eager qualitative mode the safety gate is **not applied**.  Any task
/// still pending in `eval_buffer` has already been checked for the
/// short-circuit condition (`atomic_false` / `atomic_true`) at every prior
/// step; if it had been triggered the task would have been removed.  The
/// surviving tasks therefore contain only identity values in their windows so
/// far (`true` for `Globally`, `false` for `Eventually`).  Evicting an
/// identity value from the Lemire deque never changes the fold result, so
/// back-eviction is always semantically safe and must not be blocked.
///
/// Concretely: without this exception, on a formula like `G[0,1000](φ)` the
/// safety gate's Condition A (`oldest ≥ new_ts − b`) fails permanently once
/// `t > 1000`, causing every eviction to be blocked and the cache to balloon
/// to ~`max_lookahead` entries — which is O(1100) for the inner-F[\0,100\]
/// nesting — instead of the O(1) steady-state size that Lemire normally
/// achieves.
fn pop_dominated_values<C, Y>(
    cache: &mut C,
    sub_step: &Step<Y>,
    is_max: bool,
    interval: &TimeInterval,
    eval_buffer: &BTreeSet<Duration>,
    is_eager: bool,
) where
    C: RingBufferTrait<Value = Y>,
    Y: RobustnessSemantics + Debug,
{
    while let Some(back) = cache.get_back() {
        if !Y::prune_dominated(back.value.clone(), sub_step.value.clone(), is_max) {
            break; // Back is not dominated; no earlier entry will be either.
        }
        if !is_eager
            && !is_lemire_eviction_safe(back.timestamp, sub_step.timestamp, interval, eval_buffer)
        {
            break; // Value-dominated but not yet safe to remove.
        }
        cache.pop_back();
    }
}

#[derive(Clone)]
/// Temporal eventually operator `F[a,b](φ)`.
///
/// For each evaluation timestamp `t`, this computes the operand aggregation over
/// the window `[t + a, t + b]` using [`RobustnessSemantics::or`].
pub struct Eventually<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> {
    interval: TimeInterval,
    operand: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
    cache: C,
    eval_buffer: BTreeSet<Duration>,
    max_lookahead: Duration,
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Eventually<T, C, Y, IS_EAGER, IS_ROSI> {
    /// Creates a new `Eventually` operator.
    ///
    /// `max_lookahead` is computed as `interval.end + operand.get_max_lookahead()`.
    /// Optional cache and evaluation buffer can be injected for state restore.
    pub fn new(
        interval: TimeInterval,
        operand: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
        cache: Option<C>,
        eval_buffer: Option<BTreeSet<Duration>>,
    ) -> Self
    where
        T: Clone + 'static,
        C: RingBufferTrait<Value = Y> + Clone + 'static,
        Y: RobustnessSemantics + 'static,
    {
        let max_lookahead = interval.end + operand.get_max_lookahead();
        #[cfg(feature = "track-cache-size")]
        {
            let mut c = cache.unwrap_or_else(|| C::new());
            c.set_tracked(true); // Enable tracking for this cache
            Eventually {
                interval,
                operand,
                cache: c,
                eval_buffer: eval_buffer.unwrap_or_default(),
                max_lookahead,
            }
        }
        #[cfg(not(feature = "track-cache-size"))]
        {
            let c = cache.unwrap_or_else(|| C::new());
            Eventually {
                interval,
                operand,
                cache: c,
                eval_buffer: eval_buffer.unwrap_or_default(),
                max_lookahead,
            }
        }
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> StlOperatorTrait<T>
    for Eventually<T, C, Y, IS_EAGER, IS_ROSI>
where
    T: Clone + 'static,
    C: RingBufferTrait<Value = Y> + Clone + 'static,
    Y: RobustnessSemantics + Debug + 'static,
{
    type Output = Y;

    fn get_max_lookahead(&self) -> Duration {
        self.max_lookahead
    }

    /// Updates temporal state with one input sample and emits available outputs.
    ///
    /// Behavior depends on mode:
    /// - delayed (`IS_ROSI = false`, `IS_EAGER = false`): emits only closed-window results,
    /// - eager (`IS_EAGER = true`): may finalize early on semantic `true`,
    /// - RoSI (`IS_ROSI = true`): can emit intermediate refinable values using `unknown()`.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>> {
        let sub_robustness_vec = self.operand.update(step);
        let mut output_robustness = Vec::new();

        // 1. Add new sub-formula results to the cache
        if IS_ROSI {
            for sub_step in sub_robustness_vec {
                self.eval_buffer.insert(sub_step.timestamp);

                // Try to update existing step
                if !self.cache.update_step(sub_step.clone()) {
                    // Not found in cache.
                    // Check if this is a NEW step or an OLD one that was pruned.
                    let is_new_step = match self.cache.get_back() {
                        Some(back) => sub_step.timestamp > back.timestamp,
                        None => true,
                    };

                    if is_new_step {
                        // New step: Safe to prune back and append (Lemire)
                        pop_dominated_values(
                            &mut self.cache,
                            &sub_step,
                            true,
                            &self.interval,
                            &self.eval_buffer,
                            false, // IS_ROSI: always apply safety gate
                        ); // true for Max (Eventually)
                        self.cache.add_step(sub_step);
                    }
                }
            }
        } else {
            // Non-RoSI path (f64/bool)
            for sub_step in &sub_robustness_vec {
                self.eval_buffer.insert(sub_step.timestamp);
                pop_dominated_values(
                    &mut self.cache,
                    sub_step,
                    true,
                    &self.interval,
                    &self.eval_buffer,
                    IS_EAGER, // skip safety gate in eager mode — see fn doc
                ); // true for Max
                self.cache.add_step(sub_step.clone());
            }
        }

        let mut tasks_to_remove = Vec::new();
        let current_time = step.timestamp;

        // 2. Process the evaluation buffer
        for &t_eval in self.eval_buffer.iter() {
            let window_start = t_eval + self.interval.start;
            let window_end = t_eval + self.interval.end;

            // Bounded window scan — correct for any timestamp density.
            //
            // With dense timestamps the Lemire safety gate always passes, so
            // the deque is properly maintained: the first entry at or after
            // `window_start` is the maximum and the scan terminates in O(1).
            // With sparse timestamps (gap > window width) the gate may block
            // some evictions, leaving a non-monotone deque; the bounded
            // `take_while` ensures we never read past `window_end` in that
            // case, so correctness is preserved regardless.
            let windowed_max_value = self
                .cache
                .iter()
                .skip_while(|entry| entry.timestamp < window_start)
                .take_while(|entry| entry.timestamp <= window_end)
                .map(|entry| entry.value.clone())
                .fold(Y::eventually_identity(), Y::or);

            let final_value: Option<Y>;
            let mut remove_task = false;

            let t = if IS_ROSI {
                self.cache
                    .get_back()
                    .map(|s| s.timestamp)
                    .unwrap_or(Duration::ZERO)
            } else {
                current_time
            };

            // state-based logic
            if t >= t_eval + self.max_lookahead {
                // Case 1: Full window has passed. This is a final, "closed" value.
                final_value = Some(windowed_max_value);
                remove_task = true;
            } else if IS_EAGER && windowed_max_value == Y::atomic_true() {
                // Case 2: Eager short-circuit. Found "true" before window closed.
                final_value = Some(windowed_max_value);
                remove_task = true;
            } else if IS_ROSI {
                // Case 3: Intermediate ROSI. Window is still open.
                // We must 'or' with the unknown future.
                let intermediate_value = Y::or(windowed_max_value, Y::unknown()); // Use unknown() from your code
                final_value = Some(intermediate_value);
                // DO NOT remove task, it's not finished
            } else {
                // Case 4: Cannot evaluate yet (e.g., bool/f64 and window is still open)
                // Since the buffer is time-ordered, we stop.
                break;
            }

            if let Some(val) = final_value {
                output_robustness.push(Step::new("output", val, t_eval));
            }

            if remove_task {
                tasks_to_remove.push(t_eval);
            }
        }

        // 3. Prune the cache and buffer
        let protected_ts = self.eval_buffer.first().copied().unwrap_or(Duration::ZERO);
        let lookahead = self.get_max_lookahead();
        guarded_prune(&mut self.cache, lookahead, protected_ts);
        for t in tasks_to_remove {
            self.eval_buffer.remove(&t);
        }

        output_robustness
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> SignalIdentifier
    for Eventually<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Returns the signal identifiers referenced by the operand.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        self.operand.get_signal_identifiers()
    }
}

#[derive(Clone)]
/// Temporal globally operator `G[a,b](φ)`.
///
/// For each evaluation timestamp `t`, this computes the operand aggregation over
/// the window `[t + a, t + b]` using [`RobustnessSemantics::and`].
pub struct Globally<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> {
    interval: TimeInterval,
    operand: Box<dyn StlOperatorAndSignalIdentifier<T, Y> + 'static>,
    cache: C,
    eval_buffer: BTreeSet<Duration>,
    max_lookahead: Duration,
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Globally<T, C, Y, IS_EAGER, IS_ROSI> {
    /// Creates a new `Globally` operator.
    ///
    /// `max_lookahead` is computed as `interval.end + operand.get_max_lookahead()`.
    /// Optional cache and evaluation buffer can be injected for state restore.
    pub fn new(
        interval: TimeInterval,
        operand: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
        cache: Option<C>,
        eval_buffer: Option<BTreeSet<Duration>>,
    ) -> Self
    where
        T: Clone + 'static,
        C: RingBufferTrait<Value = Y> + Clone + 'static,
        Y: RobustnessSemantics + 'static,
    {
        let max_lookahead = interval.end + operand.get_max_lookahead();
        #[cfg(feature = "track-cache-size")]
        {
            let mut c = cache.unwrap_or_else(|| C::new());
            c.set_tracked(true); // Enable tracking for this cache
            Globally {
                interval,
                operand,
                cache: c,
                eval_buffer: eval_buffer.unwrap_or_default(),
                max_lookahead,
            }
        }
        #[cfg(not(feature = "track-cache-size"))]
        {
            let c = cache.unwrap_or_else(|| C::new());
            Globally {
                interval,
                operand,
                cache: c,
                eval_buffer: eval_buffer.unwrap_or_default(),
                max_lookahead,
            }
        }
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> StlOperatorTrait<T>
    for Globally<T, C, Y, IS_EAGER, IS_ROSI>
where
    T: Clone + 'static,
    C: RingBufferTrait<Value = Y> + Clone + 'static,
    Y: RobustnessSemantics + Debug + 'static,
{
    type Output = Y;

    fn get_max_lookahead(&self) -> Duration {
        self.max_lookahead
    }

    /// Updates temporal state with one input sample and emits available outputs.
    ///
    /// Behavior depends on mode:
    /// - delayed (`IS_ROSI = false`, `IS_EAGER = false`): emits only closed-window results,
    /// - eager (`IS_EAGER = true`): may finalize early on semantic `false`,
    /// - RoSI (`IS_ROSI = true`): can emit intermediate refinable values using `unknown()`.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>> {
        let sub_robustness_vec = self.operand.update(step);
        let mut output_robustness = Vec::new();

        // 1. Add new sub-formula results to the cache
        if IS_ROSI {
            for sub_step in sub_robustness_vec {
                self.eval_buffer.insert(sub_step.timestamp);

                if !self.cache.update_step(sub_step.clone()) {
                    // Not found in cache.
                    // Check if this is a NEW step or an OLD one that was pruned.
                    let is_new_step = match self.cache.get_back() {
                        Some(back) => sub_step.timestamp > back.timestamp,
                        None => true,
                    };

                    if is_new_step {
                        // New step: Safe to prune back and append (Lemire)
                        pop_dominated_values(
                            &mut self.cache,
                            &sub_step,
                            false,
                            &self.interval,
                            &self.eval_buffer,
                            false, // IS_ROSI: always apply safety gate
                        ); // false for Min (Globally)
                        self.cache.add_step(sub_step);
                    }
                }
            }
        } else {
            // Non-RoSI path (f64/bool)
            for sub_step in &sub_robustness_vec {
                pop_dominated_values(
                    &mut self.cache,
                    sub_step,
                    false,
                    &self.interval,
                    &self.eval_buffer,
                    IS_EAGER, // skip safety gate in eager mode — see fn doc
                ); // false for Min
                self.eval_buffer.insert(sub_step.timestamp);
                self.cache.add_step(sub_step.clone());
            }
        }

        let mut tasks_to_remove = Vec::new();
        let current_time = step.timestamp;

        // 2. Process the evaluation buffer
        for &t_eval in self.eval_buffer.iter() {
            let window_start = t_eval + self.interval.start;
            let window_end = t_eval + self.interval.end;

            // Bounded window scan — correct for any timestamp density.
            //
            // With dense timestamps the Lemire safety gate always passes, so
            // the deque is properly maintained: the first entry at or after
            // `window_start` is the minimum and the scan terminates in O(1).
            // With sparse timestamps (gap > window width) the gate may block
            // some evictions, leaving a non-monotone deque; the bounded
            // `take_while` ensures we never read past `window_end` in that
            // case, so correctness is preserved regardless.
            let windowed_min_value = self
                .cache
                .iter()
                .skip_while(|entry| entry.timestamp < window_start)
                .take_while(|entry| entry.timestamp <= window_end)
                .map(|entry| entry.value.clone())
                .fold(Y::globally_identity(), Y::and);

            let final_value: Option<Y>;
            let mut remove_task = false;

            // we can finalize when cache has a ver
            let t = if IS_ROSI {
                self.cache
                    .get_back()
                    .map(|s| s.timestamp)
                    .unwrap_or(Duration::ZERO)
            } else {
                current_time
            };

            // state-based logic
            if t >= t_eval + self.max_lookahead {
                // Case 1: Full window has passed. This is a final, "closed" value.
                final_value = Some(windowed_min_value);
                remove_task = true;
            } else if IS_EAGER && windowed_min_value == Y::atomic_false() {
                // Case 2: Eager short-circuit. Found "false" before window closed.
                final_value = Some(windowed_min_value);
                remove_task = true;
            } else if IS_ROSI {
                // Case 3: Intermediate ROSI. Window is still open.
                // We must 'and' with the unknown future.
                let intermediate_value = Y::and(windowed_min_value, Y::unknown());
                final_value = Some(intermediate_value);
                // DO NOT remove task, it's not finished
            } else {
                // Case 4: Cannot evaluate yet (e.g., bool/f64 and window is still open)
                // Since the buffer is time-ordered, we stop.
                break;
            }

            if let Some(val) = final_value {
                output_robustness.push(Step::new("output", val, t_eval));
            }

            if remove_task {
                tasks_to_remove.push(t_eval);
            }
        }

        // 3. Prune the cache and buffer
        let protected_ts = self.eval_buffer.first().copied().unwrap_or(Duration::ZERO);
        let lookahead = self.get_max_lookahead();
        guarded_prune(&mut self.cache, lookahead, protected_ts);
        for t in tasks_to_remove {
            self.eval_buffer.remove(&t);
        }

        output_robustness
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> SignalIdentifier
    for Globally<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Returns the signal identifiers referenced by the operand.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        self.operand.get_signal_identifiers()
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Display
    for Globally<T, Y, C, IS_EAGER, IS_ROSI>
{
    /// Formats as `G[start, end](operand)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "G[{}, {}]({})",
            self.interval.start.as_secs_f64(),
            self.interval.end.as_secs_f64(),
            self.operand
        )
    }
}

impl<T, C, Y, const IS_EAGER: bool, const IS_ROSI: bool> Display
    for Eventually<T, C, Y, IS_EAGER, IS_ROSI>
{
    /// Formats as `F[start, end](operand)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "F[{}, {}]({})",
            self.interval.start.as_secs_f64(),
            self.interval.end.as_secs_f64(),
            self.operand
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_buffer::{RingBuffer, Step};
    use crate::stl::core::{StlOperatorTrait, TimeInterval};
    use crate::stl::operators::atomic_operators::Atomic;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn eventually_operator_robustness() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        let mut eventually = Eventually::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(atomic),
            None,
            None,
        );
        eventually.get_signal_identifiers();

        let signal_values = vec![15.0, 12.0, 8.0, 5.0, 12.0];
        let signal_timestamps = vec![0, 2, 4, 6, 8];
        let signal: Vec<_> = signal_values
            .into_iter()
            .zip(signal_timestamps)
            .map(|(val, ts)| Step::new("x", val, Duration::from_secs(ts)))
            .collect();

        let mut all_outputs = Vec::new();
        for s in &signal {
            all_outputs.extend(eventually.update(s));
        }

        let expected_outputs = [
            Step::new("output", 5.0, Duration::from_secs(0)),
            Step::new("output", 2.0, Duration::from_secs(2)),
            Step::new("output", 2.0, Duration::from_secs(4)),
        ];

        assert_eq!(all_outputs.len(), expected_outputs.len());
        for (output, expected) in all_outputs.iter().zip(expected_outputs.iter()) {
            assert_eq!(output.timestamp, expected.timestamp);
            assert!(
                (output.value - expected.value).abs() < 1e-9,
                "left: {}, right: {}",
                output.value,
                expected.value
            );
        }
    }

    #[test]
    fn globally_operator_robustness() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        let mut globally = Globally::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(atomic),
            None,
            None,
        );
        globally.get_signal_identifiers();

        let signal_values = vec![15.0, 12.0, 8.0, 5.0, 12.0];
        let signal_timestamps = vec![0, 2, 4, 6, 8];
        let signal: Vec<_> = signal_values
            .into_iter()
            .zip(signal_timestamps)
            .map(|(val, ts)| Step::new("x", val, Duration::from_secs(ts)))
            .collect();

        let mut all_outputs = Vec::new();
        for s in &signal {
            all_outputs.extend(globally.update(s));
        }

        let expected_outputs = [
            Step::new("output", -2.0, Duration::from_secs(0)),
            Step::new("output", -5.0, Duration::from_secs(2)),
            Step::new("output", -5.0, Duration::from_secs(4)),
        ];

        assert_eq!(all_outputs.len(), expected_outputs.len());
        for (output, expected) in all_outputs.iter().zip(expected_outputs.iter()) {
            assert_eq!(output.timestamp, expected.timestamp);
            assert!(
                (output.value - expected.value).abs() < 1e-9,
                "left: {}, right: {}",
                output.value,
                expected.value
            );
        }
    }

    #[test]
    fn unary_temporal_signal_identifiers() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(4),
        };
        let atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        let mut globally = Globally::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(atomic),
            None,
            None,
        );
        let ids = globally.get_signal_identifiers();
        let expected_ids: HashSet<&'static str> = vec!["x"].into_iter().collect();
        assert_eq!(ids, expected_ids);
    }

    #[test]
    fn globally_display() {
        let interval = TimeInterval {
            start: Duration::from_secs(1),
            end: Duration::from_secs(5),
        };
        let atomic = Atomic::<f64>::new_greater_than("x", 10.0);
        let globally = Globally::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(atomic),
            None,
            None,
        );
        assert_eq!(format!("{globally}"), "G[1, 5](x > 10)");
    }

    #[test]
    fn eventually_display() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(3),
        };
        let atomic = Atomic::<f64>::new_less_than("y", 5.0);
        let eventually = Eventually::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(atomic),
            None,
            None,
        );
        assert_eq!(format!("{eventually}"), "F[0, 3](y < 5)");
    }
}

#[cfg(test)]
mod sparse_timestamp_tests {
    //! Regression tests for the Lemire safety-gate fix.
    //!
    //! Root cause: `pop_dominated_values` would evict `v@t_old` in favour of
    //! `v'@t_new` even when a pending `t_eval` had `t_old` inside its window
    //! but `t_new` outside it.  With dense timestamps the oldest pending eval
    //! is always finalised before the gap can grow large enough to trigger
    //! this, so it never mattered.  With sparse timestamps (gap > window
    //! width) the invariant is violated and the scan returned the wrong value.
    //!
    //! The non-RoSI path additionally used an unbounded `find(ts >=
    //! window_start)` that had no `window_end` guard; it happened to return a
    //! value that was within the window in the dense case (because the deque
    //! front is always ≤ window_end when Lemire is healthy) but silently
    //! returned an out-of-window entry once the deque was corrupted.
    //!
    //! Formula under test: G[0,2](x > 3), inputs at t = 0, 1, 2, 5, 10.
    //!
    //! Robustness (x − 3): 122.5 @ 0, 12.0 @ 1, 12.0 @ 2, −1.0 @ 5, −1.0 @ 10
    //!
    //! Expected delayed-quantitative output (finalised when current_time ≥ t_eval + 2):
    //!   t_eval=0  (finalised at t=2):  min(122.5, 12, 12) = 12.0
    //!   t_eval=1  (finalised at t=5):  window [1,3] → min(12, 12) = 12.0
    //!   t_eval=2  (finalised at t=5):  window [2,4] → min(12) = 12.0
    //!   t_eval=5  (finalised at t=10): window [5,7] → min(−1) = −1.0

    use super::*;
    use crate::ring_buffer::{RingBuffer, Step};
    use crate::stl::core::{RobustnessInterval, StlOperatorTrait, TimeInterval};
    use crate::stl::operators::atomic_operators::Atomic;
    use std::time::Duration;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn g02_globally_f64() -> Globally<f64, RingBuffer<f64>, f64, false, false> {
        let interval = TimeInterval {
            start: secs(0),
            end: secs(2),
        };
        let atomic = Atomic::<f64>::new_greater_than("x", 3.0);
        Globally::new(interval, Box::new(atomic), None, None)
    }
    fn g02_globally_rosi()
    -> Globally<f64, RingBuffer<RobustnessInterval>, RobustnessInterval, true, true> {
        let interval = TimeInterval {
            start: secs(0),
            end: secs(2),
        };
        let atomic = Atomic::<RobustnessInterval>::new_greater_than("x", 3.0);
        Globally::new(interval, Box::new(atomic), None, None)
    }
    fn g02_globally_eager_qual() -> Globally<f64, RingBuffer<bool>, bool, true, false> {
        let interval = TimeInterval {
            start: secs(0),
            end: secs(2),
        };
        let atomic = Atomic::<bool>::new_greater_than("x", 3.0);
        Globally::new(interval, Box::new(atomic), None, None)
    }

    fn sparse_steps() -> Vec<Step<f64>> {
        vec![
            Step::new("x", 100.0, secs(0)),
            Step::new("x", 15.0, secs(1)),
            Step::new("x", 16.0, secs(2)),
            Step::new("x", 2.0, secs(5)),
            Step::new("x", 2.0, secs(10)),
        ]
    }

    fn find_output<Y>(outputs: &[Step<Y>], ts: u64) -> Y
    where
        Y: Copy,
    {
        outputs
            .iter()
            .find(|s| s.timestamp == secs(ts))
            .unwrap_or_else(|| panic!("no output for t_eval={ts}"))
            .value
    }

    // This test covers the handling of sparse timestamps in the globally operator. It checks also whether RoSI, delayed and eager qualitative modes agree on the same final values, and whether eager qual can finalize early on true and false as expected.
    // Eventually has a reflective implementation so not tested here... but we should add a similar test for it as well.
    #[test]
    fn globally_sparse_timestamps() {
        let mut op_f64 = g02_globally_f64();
        let mut op_rosi = g02_globally_rosi();
        let mut op_eager_qual = g02_globally_eager_qual();

        for step in &sparse_steps() {
            let outputs_f64 = op_f64.update(step);
            let outputs_rosi = op_rosi.update(step);
            let outputs_eager_qual = op_eager_qual.update(step);
            match step.timestamp.as_secs() {
                0 => {
                    assert!(
                        outputs_f64.is_empty() && outputs_eager_qual.is_empty(),
                        "t_eval={} expected no output, got {:?}",
                        step.timestamp.as_secs(),
                        outputs_f64
                    );
                    assert!(
                        outputs_rosi.len() == 1,
                        "t_eval={} expected no output, got {:?}",
                        step.timestamp.as_secs(),
                        outputs_rosi
                    );
                }
                1 => {
                    assert!(
                        outputs_f64.is_empty() && outputs_eager_qual.is_empty(),
                        "t_eval={} expected no output, got {:?}",
                        step.timestamp.as_secs(),
                        outputs_f64
                    );
                    assert!(
                        outputs_rosi.len() == 2,
                        "t_eval={} expected no output, got {:?}",
                        step.timestamp.as_secs(),
                        outputs_rosi
                    );
                }
                // at 2 f64 emits for 0, and rosi agrees in bounds
                2 => {
                    assert!(
                        (find_output(&outputs_f64, 0) - 12.0).abs() < 1e-9,
                        "t_eval=0 expected 12.0, got {}",
                        find_output(&outputs_f64, 0)
                    );
                    let rosi_val = find_output(&outputs_rosi, 0);
                    assert!(
                        rosi_val.0 == 12.0 && rosi_val.1 == 12.0,
                        "t_eval=0 expected ROSI bounds to contain 12.0, got {:?}",
                        rosi_val
                    );
                    assert!(
                        find_output(&outputs_eager_qual, 0),
                        "t_eval=0 expected eager qual to be true, got {}",
                        find_output(&outputs_eager_qual, 0)
                    )
                }
                // at 5 f64 emits for 1 and 2, and rosi agrees in bounds
                5 => {
                    assert!(
                        (find_output(&outputs_f64, 1) - 12.0).abs() < 1e-9,
                        "t_eval=1 expected 12.0, got {}",
                        find_output(&outputs_f64, 1)
                    );
                    assert!(
                        (find_output(&outputs_f64, 2) - 13.0).abs() < 1e-9,
                        "t_eval=2 expected 13.0, got {}",
                        find_output(&outputs_f64, 2)
                    );
                    let rosi_val_1 = find_output(&outputs_rosi, 1);
                    let rosi_val_2 = find_output(&outputs_rosi, 2);
                    let rosi_val_5 = find_output(&outputs_rosi, 5);
                    assert!(
                        rosi_val_1.0 == 12.0 && rosi_val_1.1 == 12.0,
                        "t_eval=1 expected ROSI bounds to contain 12.0, got {:?}",
                        rosi_val_1
                    );
                    assert!(
                        rosi_val_2.0 == 13.0 && rosi_val_2.1 == 13.0,
                        "t_eval=2 expected ROSI bounds to contain 13.0, got {:?}",
                        rosi_val_2
                    );
                    assert!(
                        find_output(&outputs_eager_qual, 1),
                        "t_eval=1 expected eager qual to be true, got {}",
                        find_output(&outputs_eager_qual, 1)
                    );
                    assert!(
                        find_output(&outputs_eager_qual, 2),
                        "t_eval=2 expected eager qual to be true, got {}",
                        find_output(&outputs_eager_qual, 2)
                    );
                    // it sees the -1.0 at t=5 and finalizes to false immediately, without waiting for t=10
                    assert!(
                        !find_output(&outputs_eager_qual, 5),
                        "t_eval=5 expected eager qual to be false, got {}",
                        find_output(&outputs_eager_qual, 5)
                    );
                    // should agree with rosi upper bound being negative already at t=5, even if it can't finalize yet
                    assert!(
                        rosi_val_5.1 < 0.0,
                        "t_eval=5 expected ROSI upper bound to be negative, got {:?}",
                        rosi_val_5
                    );
                }
                10 => {
                    assert!(
                        (find_output(&outputs_f64, 5) + 1.0).abs() < 1e-9,
                        "t_eval=5 expected -1.0, got {}",
                        find_output(&outputs_f64, 5)
                    );
                    let rosi_val = find_output(&outputs_rosi, 5);
                    assert!(
                        rosi_val.0 == -1.0 && rosi_val.1 == -1.0,
                        "t_eval=5 expected ROSI bounds to contain -1.0, got {:?}",
                        rosi_val
                    );
                    // eager can short-circuit to false for t=10 already
                    assert!(
                        !find_output(&outputs_eager_qual, 10),
                        "t_eval=5 expected eager qual to be false, got {}",
                        find_output(&outputs_eager_qual, 10)
                    );
                }
                _ => panic!("unexpected output at t={}", step.timestamp.as_secs()),
            }
        }
    }
}
