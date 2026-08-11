//! Unary temporal STL operators (`Eventually`, `Globally`).
//!
//! This module implements sliding-window temporal evaluation over an operand
//! stream, with support for delayed, eager, and (RoSI) execution via
//! const generics.

use crate::core::{
    RobustnessSemantics, SignalIdentifier, StlOperatorAndSignalIdentifier, StlOperatorTrait,
    TimeInterval,
};
use crate::ring_buffer::{RingBufferTrait, Step, guarded_prune};
use std::collections::{HashSet, VecDeque};
use std::fmt::{Debug, Display};
use std::time::Duration;

/// Time-window parameters passed to [`process_eval_buffer`].
struct WindowParams<'a> {
    interval: &'a TimeInterval,
    max_lookahead: Duration,
    current_time: Duration,
    upper_bound: Option<Duration>,
}

/// Operator-specific semantic parameters passed to [`process_eval_buffer`].
struct OpParams<Y, FCombine, FIdentity>
where
    FCombine: Fn(Y, Y) -> Y,
    FIdentity: Fn() -> Y,
{
    combine: FCombine,
    identity: FIdentity,
    eager_short_circuit: Y,
}

/// Processes the evaluation buffer, computing windowed aggregations and emitting
/// finalized outputs.
///
/// Entries are removed from `eval_buffer` only as a contiguous front prefix, which
/// holds in every mode: delayed and eager `break` at the first entry that is neither
/// closed nor short-circuited, and RoSI removes only on `is_closed`, which is
/// monotone in the ascending `t_eval` order.
#[allow(clippy::skip_while_next)]
fn process_eval_buffer<C, Y, FCombine, FIdentity, const IS_EAGER: bool, const IS_ROSI: bool>(
    eval_buffer: &mut VecDeque<Duration>,
    cache: &C,
    window: WindowParams<'_>,
    op: OpParams<Y, FCombine, FIdentity>,
) -> Vec<Step<Y>>
where
    C: RingBufferTrait<Value = Y>,
    Y: RobustnessSemantics + Debug,
    FCombine: Fn(Y, Y) -> Y,
    FIdentity: Fn() -> Y,
{
    let mut output_robustness = Vec::new();
    // Length of the finalized prefix to drop from the front once the scan ends.
    let mut n_finalized: usize = 0;

    for &t_eval in eval_buffer.iter() {
        // cannot finalize after this bound
        if let Some(bound) = window.upper_bound
            && t_eval >= bound
        {
            break;
        }

        // the window and every operand lookahead behind it have elapsed
        let is_closed = window.current_time >= t_eval + window.max_lookahead;

        if !is_closed && !IS_EAGER && !IS_ROSI {
            break;
        }

        let window_start = t_eval + window.interval.start;
        let window_end = t_eval + window.interval.end;

        // Obtain the windowed value for this eval timestamp.  Behavior differs by mode:
        //
        // - RoSI: `prune_dominated` for intervals requires strict separation, so the cache
        //   keeps mutually incomparable entries and is not a monotone Lemire deque. The
        //   window has to be aggregated with combine(), or identity() when it is empty.
        //
        // - non-RoSI: the cache *is* a monotone Lemire deque, so the first entry at or after
        //   `window_start` is the window extremum and the scan stops there. Note there is
        //   deliberately no `<= window_end` bound: `pop_dominated_values` evicts an in-window
        //   entry only in favour of a later one that dominates it, so a surviving entry past
        //   `window_end` still carries the correct extremum for this window. Bounding the
        //   scan would return identity() and lose that value.
        let windowed_value = if IS_ROSI {
            cache
                .iter()
                .skip_while(|s| s.timestamp < window_start)
                .take_while(|s| s.timestamp <= window_end)
                .map(|s| s.value.clone())
                .reduce(&op.combine)
                .unwrap_or_else(&op.identity)
        } else {
            cache
                .iter()
                // .find(|f| f.timestamp >= window_start) // equivalent but wayyy slower than skip_while + next but clippy complains..
                .skip_while(|f| f.timestamp < window_start)
                .next()
                .map(|entry| entry.value.clone())
                .unwrap_or_else(&op.identity)
        };

        // The `!IS_ROSI` guard keeps eager short-circuiting out of RoSI, where it is both
        // meaningless (an interval never equals a crisp `atomic_true`/`atomic_false`) and
        // would break the front-prefix removal invariant.
        if is_closed || (IS_EAGER && !IS_ROSI && windowed_value == op.eager_short_circuit) {
            output_robustness.push(Step::new("output", windowed_value, t_eval));
            n_finalized += 1;
        } else if IS_ROSI {
            // Window still open: emit a refinable verdict and keep the entry pending.
            let intermediate_value = (op.combine)(windowed_value, Y::unknown());
            output_robustness.push(Step::new("output", intermediate_value, t_eval));
        } else {
            break;
        }
    }

    eval_buffer.drain(..n_finalized);

    output_robustness
}

/// Registers the operand's output steps into the evaluation buffer and the Lemire cache.
///
/// `eval_buffer` holds the pending evaluation timestamps: strictly ascending and unique.
/// `cache.get_back()` is the watermark of the newest timestamp ever admitted —
/// [`pop_dominated_values`] only evicts the back in favour of a strictly newer step, and
/// [`guarded_prune`] only evicts from the front — so a step is new iff its timestamp
/// exceeds it.
///
/// Under RoSI the operand re-emits already-seen timestamps as refined verdicts. Those are
/// upserted into the cache in place and must not be re-queued. `update_step` returning
/// `false` means the entry was already evicted by domination, so the refinement is
/// subsumed by the surviving value and can be dropped.
fn register_sub_steps<C, Y, const IS_ROSI: bool>(
    cache: &mut C,
    eval_buffer: &mut VecDeque<Duration>,
    sub_steps: Vec<Step<Y>>,
    is_max: bool,
    window_length: Duration,
) where
    C: RingBufferTrait<Value = Y>,
    Y: RobustnessSemantics + Debug,
{
    for sub_step in sub_steps {
        if cache
            .get_back()
            .is_none_or(|back| sub_step.timestamp > back.timestamp)
        {
            eval_buffer.push_back(sub_step.timestamp);
            pop_dominated_values(cache, &sub_step, is_max, window_length);
            cache.add_step(sub_step);
        } else if IS_ROSI {
            cache.update_step(sub_step);
        }
    }
}

/// Removes dominated values from the back of a monotone cache (Lemire
/// sliding min/max).
///
/// `is_max = true` is used for `Eventually`; `is_max = false` for `Globally`.
fn pop_dominated_values<C, Y>(
    cache: &mut C,
    sub_step: &Step<Y>,
    is_max: bool,
    window_length: Duration,
) where
    C: RingBufferTrait<Value = Y>,
    Y: RobustnessSemantics + Debug,
{
    while let Some(back) = cache.get_back() {
        if Y::prune_dominated(back.value.clone(), sub_step.value.clone(), is_max)
            && back.timestamp + window_length >= sub_step.timestamp
        {
            cache.pop_back();
        } else {
            break;
        }
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
    eval_buffer: VecDeque<Duration>,
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
        eval_buffer: Option<VecDeque<Duration>>,
    ) -> Self
    where
        T: Clone + 'static,
        C: RingBufferTrait<Value = Y> + Clone + 'static,
        Y: RobustnessSemantics + 'static,
    {
        let max_lookahead = interval.end + operand.get_max_lookahead();
        let eval_buffer = eval_buffer.unwrap_or_default();
        #[cfg(feature = "track-cache-size")]
        {
            let mut c = cache.unwrap_or_else(|| C::new());
            c.set_tracked(true); // Enable tracking for this cache
            Eventually {
                interval,
                operand,
                cache: c,
                eval_buffer,
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
                eval_buffer,
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

    fn total_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.cache.heap_size()
            + self.eval_buffer.capacity() * std::mem::size_of::<Duration>()
            + self.operand.total_size()
    }

    fn reset(&mut self) {
        self.cache.clear();
        self.eval_buffer.clear();
        self.operand.reset();
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
        let current_time = step.timestamp;

        // Phase A: finalize windows that close strictly before the
        // first new sub-step, against the current (pre-Phase-B) cache.
        if let Some(first) = sub_robustness_vec.first()
            && let Some(split_key) = first.timestamp.checked_sub(self.max_lookahead)
        {
            output_robustness.extend(process_eval_buffer::<_, _, _, _, IS_EAGER, IS_ROSI>(
                &mut self.eval_buffer,
                &self.cache,
                WindowParams {
                    interval: &self.interval,
                    max_lookahead: self.max_lookahead,
                    current_time: first.timestamp,
                    upper_bound: Some(split_key),
                },
                OpParams {
                    combine: Y::or,
                    identity: Y::eventually_identity,
                    eager_short_circuit: Y::atomic_true(),
                },
            ));
        }

        // Phase B: queue every new evaluation timestamp and update the Lemire cache.
        register_sub_steps::<_, _, IS_ROSI>(
            &mut self.cache,
            &mut self.eval_buffer,
            sub_robustness_vec,
            true,
            self.interval.window_length(),
        );

        // Phase C: process remaining eval_buffer entries with the updated cache.
        let phase_c_time = if IS_ROSI {
            self.cache
                .get_back()
                .map(|s| s.timestamp)
                .unwrap_or(Duration::ZERO)
        } else {
            current_time
        };
        output_robustness.extend(process_eval_buffer::<_, _, _, _, IS_EAGER, IS_ROSI>(
            &mut self.eval_buffer,
            &self.cache,
            WindowParams {
                interval: &self.interval,
                max_lookahead: self.max_lookahead,
                current_time: phase_c_time,
                upper_bound: None,
            },
            OpParams {
                combine: Y::or,
                identity: Y::eventually_identity,
                eager_short_circuit: Y::atomic_true(),
            },
        ));

        // Prune the cache.
        let protected_ts = self.eval_buffer.front().copied().unwrap_or(Duration::ZERO);
        guarded_prune(&mut self.cache, self.max_lookahead, protected_ts);

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
    eval_buffer: VecDeque<Duration>,
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
        eval_buffer: Option<VecDeque<Duration>>,
    ) -> Self
    where
        T: Clone + 'static,
        C: RingBufferTrait<Value = Y> + Clone + 'static,
        Y: RobustnessSemantics + 'static,
    {
        let max_lookahead = interval.end + operand.get_max_lookahead();
        let eval_buffer = eval_buffer.unwrap_or_default();
        #[cfg(feature = "track-cache-size")]
        {
            let mut c = cache.unwrap_or_else(|| C::new());
            c.set_tracked(true); // Enable tracking for this cache
            Globally {
                interval,
                operand,
                cache: c,
                eval_buffer,
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
                eval_buffer,
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

    fn total_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.cache.heap_size()
            + self.eval_buffer.capacity() * std::mem::size_of::<Duration>()
            + self.operand.total_size()
    }

    fn reset(&mut self) {
        self.cache.clear();
        self.eval_buffer.clear();
        self.operand.reset();
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
        let current_time = step.timestamp;

        // Phase A: finalize windows that close strictly before the
        // first new sub-step, against the current (pre-Phase-B) cache.
        if let Some(first) = sub_robustness_vec.first()
            && let Some(split_key) = first.timestamp.checked_sub(self.max_lookahead)
        {
            output_robustness.extend(process_eval_buffer::<_, _, _, _, IS_EAGER, IS_ROSI>(
                &mut self.eval_buffer,
                &self.cache,
                WindowParams {
                    interval: &self.interval,
                    max_lookahead: self.max_lookahead,
                    current_time: first.timestamp,
                    upper_bound: Some(split_key),
                },
                OpParams {
                    combine: Y::and,
                    identity: Y::globally_identity,
                    eager_short_circuit: Y::atomic_false(),
                },
            ));
        }

        // Phase B: queue every new evaluation timestamp and update the Lemire cache.
        register_sub_steps::<_, _, IS_ROSI>(
            &mut self.cache,
            &mut self.eval_buffer,
            sub_robustness_vec,
            false,
            self.interval.window_length(),
        );

        // Phase C: process remaining eval_buffer entries with the updated cache.
        let phase_c_time = if IS_ROSI {
            self.cache
                .get_back()
                .map(|s| s.timestamp)
                .unwrap_or(Duration::ZERO)
        } else {
            current_time
        };
        output_robustness.extend(process_eval_buffer::<_, _, _, _, IS_EAGER, IS_ROSI>(
            &mut self.eval_buffer,
            &self.cache,
            WindowParams {
                interval: &self.interval,
                max_lookahead: self.max_lookahead,
                current_time: phase_c_time,
                upper_bound: None,
            },
            OpParams {
                combine: Y::and,
                identity: Y::globally_identity,
                eager_short_circuit: Y::atomic_false(),
            },
        ));

        // Prune the cache.
        let protected_ts = self.eval_buffer.front().copied().unwrap_or(Duration::ZERO);
        guarded_prune(&mut self.cache, self.max_lookahead, protected_ts);

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
    use crate::core::{StlOperatorTrait, TimeInterval};
    use crate::operators::atomic_operators::Atomic;
    use crate::ring_buffer::RingBuffer;
    use crate::step;
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
            .map(|(val, ts)| step!("x", val, Duration::from_secs(ts)))
            .collect();

        let mut all_outputs = Vec::new();
        for s in &signal {
            all_outputs.extend(eventually.update(s));
        }

        let expected_outputs = [
            step!("output", 5.0, Duration::from_secs(0)),
            step!("output", 2.0, Duration::from_secs(2)),
            step!("output", 2.0, Duration::from_secs(4)),
        ];

        assert_eq!(all_outputs.len(), expected_outputs.len());
        for (output, expected) in all_outputs.iter().zip(expected_outputs.iter()) {
            assert_eq!(output.timestamp, expected.timestamp);
            assert!(
                (output.value - expected.value).abs() < 1e-9,
                "t={:?} output: {}, expected: {}",
                output.timestamp,
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
            .map(|(val, ts)| step!("x", val, Duration::from_secs(ts)))
            .collect();

        let mut all_outputs = Vec::new();
        for s in &signal {
            all_outputs.extend(globally.update(s));
        }

        let expected_outputs = [
            step!("output", -2.0, Duration::from_secs(0)),
            step!("output", -5.0, Duration::from_secs(2)),
            step!("output", -5.0, Duration::from_secs(4)),
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
    use crate::core::{RobustnessInterval, StlOperatorTrait, TimeInterval};
    use crate::operators::atomic_operators::Atomic;
    use crate::ring_buffer::{RingBuffer, Step};
    use crate::step;
    use std::time::Duration;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn millis(ms: u64) -> Duration {
        Duration::from_millis(ms)
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
    -> Globally<f64, RingBuffer<RobustnessInterval>, RobustnessInterval, false, true> {
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

    fn f02_eventually_f64() -> Eventually<f64, RingBuffer<f64>, f64, false, false> {
        let interval = TimeInterval {
            start: secs(0),
            end: secs(2),
        };
        let atomic = Atomic::<f64>::new_greater_than("x", 3.0);
        Eventually::new(interval, Box::new(atomic), None, None)
    }
    fn f02_eventually_rosi()
    -> Eventually<f64, RingBuffer<RobustnessInterval>, RobustnessInterval, false, true> {
        let interval = TimeInterval {
            start: secs(0),
            end: secs(2),
        };
        let atomic = Atomic::<RobustnessInterval>::new_greater_than("x", 3.0);
        Eventually::new(interval, Box::new(atomic), None, None)
    }
    fn f02_eventually_eager_qual() -> Eventually<f64, RingBuffer<bool>, bool, true, false> {
        let interval = TimeInterval {
            start: secs(0),
            end: secs(2),
        };
        let atomic = Atomic::<bool>::new_greater_than("x", 3.0);
        Eventually::new(interval, Box::new(atomic), None, None)
    }

    fn sparse_steps() -> Vec<Step<f64>> {
        vec![
            step!("x", 100.0, secs(0)),
            step!("x", 15.0, secs(1)),
            step!("x", 16.0, secs(2)),
            step!("x", 2.0, secs(5)),
            step!("x", 2.0, secs(10)),
        ]
    }

    fn find_output_secs<Y>(outputs: &[Step<Y>], ts: u64) -> Y
    where
        Y: Copy,
    {
        outputs
            .iter()
            .find(|s| s.timestamp == secs(ts))
            .unwrap_or_else(|| panic!("no output for t_eval={ts}"))
            .value
    }
    fn find_output_millis<Y>(outputs: &[Step<Y>], ts: u64) -> Y
    where
        Y: Copy,
    {
        outputs
            .iter()
            .find(|s| s.timestamp == millis(ts))
            .unwrap_or_else(|| panic!("no output for t_eval={ts}"))
            .value
    }

    /// Mirror of [`globally_sparse_timestamps`] for `Eventually`, which previously had no
    /// sparse/eager/RoSI coverage at the operator level.
    ///
    /// Formula: `F[0,2](x > 3)`, inputs at t = 0, 1, 2, 5, 10.
    /// Robustness (x − 3): 97 @ 0, 12 @ 1, 13 @ 2, −1 @ 5, −1 @ 10.
    ///
    /// Expected delayed-quantitative output (finalised when `current_time >= t_eval + 2`):
    ///   t_eval=0  (finalised at t=2):  window [0,2] → max(97, 12, 13) = 97
    ///   t_eval=1  (finalised at t=5):  window [1,3] → max(12, 13)     = 13
    ///   t_eval=2  (finalised at t=5):  window [2,4] → max(13)         = 13
    ///   t_eval=5  (finalised at t=10): window [5,7] → max(−1)         = −1
    #[test]
    fn eventually_sparse_timestamps() {
        let mut op_f64 = f02_eventually_f64();
        let mut op_rosi = f02_eventually_rosi();
        let mut op_eager = f02_eventually_eager_qual();

        for step in &sparse_steps() {
            let out_f64 = op_f64.update(step);
            let out_rosi = op_rosi.update(step);
            let out_eager = op_eager.update(step);
            let t = step.timestamp.as_secs();

            match t {
                0 | 1 => {
                    assert!(
                        out_f64.is_empty(),
                        "t={t} delayed must not emit before the window closes, got {out_f64:?}"
                    );
                    // Eager satisfies F as soon as one in-window sample is true, which is
                    // already the case at the evaluation timestamp itself.
                    assert!(
                        find_output_secs(&out_eager, t),
                        "t={t} eager expected an early true for t_eval={t}"
                    );
                    // RoSI refines every still-open entry, one verdict each.
                    assert_eq!(
                        out_rosi.len(),
                        (t + 1) as usize,
                        "t={t} unexpected RoSI emissions: {out_rosi:?}"
                    );
                    // Open F windows are lower-bounded and unbounded above.
                    let iv = find_output_secs(&out_rosi, t);
                    assert!(
                        iv.1 == f64::INFINITY,
                        "t={t} expected an open upper bound, got {iv:?}"
                    );
                }
                2 => {
                    // Window [0,2] closes.
                    assert!(
                        (find_output_secs(&out_f64, 0) - 97.0).abs() < 1e-9,
                        "t_eval=0 expected 97.0, got {}",
                        find_output_secs(&out_f64, 0)
                    );
                    let iv = find_output_secs(&out_rosi, 0);
                    assert!(
                        iv.0 == 97.0 && iv.1 == 97.0,
                        "t_eval=0 expected RoSI to collapse to [97, 97], got {iv:?}"
                    );
                    assert!(
                        find_output_secs(&out_eager, 2),
                        "t_eval=2 expected eager true"
                    );
                }
                5 => {
                    assert!(
                        (find_output_secs(&out_f64, 1) - 13.0).abs() < 1e-9,
                        "t_eval=1 expected 13.0, got {}",
                        find_output_secs(&out_f64, 1)
                    );
                    assert!(
                        (find_output_secs(&out_f64, 2) - 13.0).abs() < 1e-9,
                        "t_eval=2 expected 13.0, got {}",
                        find_output_secs(&out_f64, 2)
                    );
                    for t_eval in [1, 2] {
                        let iv = find_output_secs(&out_rosi, t_eval);
                        assert!(
                            iv.0 == 13.0 && iv.1 == 13.0,
                            "t_eval={t_eval} expected RoSI to collapse to [13, 13], got {iv:?}"
                        );
                    }
                    // x = 2 violates the atom, so eager cannot conclude yet and must wait.
                    assert!(
                        out_eager.is_empty(),
                        "t=5 eager must stay pending, got {out_eager:?}"
                    );
                    let iv = find_output_secs(&out_rosi, 5);
                    assert!(
                        iv.0 == -1.0 && iv.1 == f64::INFINITY,
                        "t_eval=5 expected [-1, +inf], got {iv:?}"
                    );
                }
                10 => {
                    assert!(
                        (find_output_secs(&out_f64, 5) + 1.0).abs() < 1e-9,
                        "t_eval=5 expected -1.0, got {}",
                        find_output_secs(&out_f64, 5)
                    );
                    let iv = find_output_secs(&out_rosi, 5);
                    assert!(
                        iv.0 == -1.0 && iv.1 == -1.0,
                        "t_eval=5 expected RoSI to collapse to [-1, -1], got {iv:?}"
                    );
                    assert!(
                        !find_output_secs(&out_eager, 5),
                        "t_eval=5 expected eager false once the window closed"
                    );
                }
                _ => panic!("unexpected input at t={t}"),
            }
        }
    }

    /// Regression test for the front-prefix removal invariant in eager mode.
    ///
    /// `G[1,3]` is used so that the newest sample can fall *outside* the window of the
    /// newest pending evaluation timestamp. That produces the one update in which all
    /// three outcomes coexist:
    ///
    /// | `t_eval` | outcome at `t = 3` |
    /// |---|---|
    /// | 0 | closed (`3 >= 0 + 3`), window `[1,3]` sees the violation → `false` |
    /// | 1 | open, window `[2,4]` sees the violation → eager short-circuits `false` |
    /// | 2 | open, window `[3,5]` sees the violation → eager short-circuits `false` |
    /// | 3 | open, window `[4,6]` still empty → stays **pending** |
    ///
    /// Removals are `{0,1,2}` — a contiguous prefix — but only `t_eval = 0` closed on
    /// time. An implementation that counts only the closed entries as poppable and
    /// compacts the remainder against a membership set will drop the pending `t_eval = 3`
    /// and never emit its verdict.
    #[test]
    fn eager_keeps_pending_entry_after_short_circuit() {
        let interval = TimeInterval {
            start: secs(1),
            end: secs(3),
        };
        let atomic = Atomic::<bool>::new_greater_than("x", 3.0);
        let mut op = Globally::<f64, RingBuffer<bool>, bool, true, false>::new(
            interval,
            Box::new(atomic),
            None,
            None,
        );

        // Violation at t = 3 only; everything from t = 4 on satisfies again.
        let steps: Vec<Step<f64>> = [
            (0u64, 10.0),
            (1, 10.0),
            (2, 10.0),
            (3, 1.0),
            (4, 10.0),
            (5, 10.0),
            (6, 10.0),
            (7, 10.0),
        ]
        .into_iter()
        .map(|(ts, v)| step!("x", v, secs(ts)))
        .collect();

        let mut all_outputs = Vec::new();
        for s in &steps {
            all_outputs.extend(op.update(s));
        }

        // t_eval = 0, 1, 2 all see the violation at t = 3 inside their windows.
        for t in 0..=2 {
            assert!(
                !find_output_secs(&all_outputs, t),
                "t_eval={t} expected false, got true"
            );
        }
        // The entry that was pending at t = 3 must survive and close at t >= 6 with
        // window [4,6] = {10, 10, 10} -> true. This is the entry the buffer compaction
        // used to discard.
        assert!(
            find_output_secs(&all_outputs, 3),
            "t_eval=3 expected true (window [4,6] is all-satisfying)"
        );
        // Exactly one verdict per evaluation timestamp, no duplicates.
        let mut seen: Vec<Duration> = all_outputs.iter().map(|s| s.timestamp).collect();
        let n_before = seen.len();
        seen.dedup();
        assert_eq!(
            n_before,
            seen.len(),
            "eager mode must emit at most one verdict per timestamp, got {all_outputs:?}"
        );
    }

    /// `total_size()` must be bounded by the window, not by the trace length.
    #[test]
    fn delayed_memory_is_bounded_by_window() {
        let mut op = g02_globally_f64();
        // Alternating values keep the Lemire cache from collapsing to a single entry.
        let mut size_after_warmup = None;
        for i in 0..2_000u64 {
            let value = if i % 2 == 0 { 10.0 } else { 1.0 };
            op.update(&step!("x", value, millis(i * 100)));
            // Let the window (2 s = 20 samples) fill before sampling the footprint.
            if i == 500 {
                size_after_warmup = Some(op.total_size());
            }
        }
        let warmed = size_after_warmup.expect("warmup sample taken");
        assert_eq!(
            op.total_size(),
            warmed,
            "total_size() grew with trace length: {warmed} -> {}",
            op.total_size()
        );
    }

    // This test covers the handling of sparse timestamps in the globally operator. It checks also whether RoSI, delayed and eager qualitative modes agree on the same final values, and whether eager qual can finalize early on true and false as expected.
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
                        (find_output_secs(&outputs_f64, 0) - 12.0).abs() < 1e-9,
                        "t_eval=0 expected 12.0, got {}",
                        find_output_secs(&outputs_f64, 0)
                    );
                    let rosi_val = find_output_secs(&outputs_rosi, 0);
                    assert!(
                        rosi_val.0 == 12.0 && rosi_val.1 == 12.0,
                        "t_eval=0 expected ROSI bounds to contain 12.0, got {:?}",
                        rosi_val
                    );
                    assert!(
                        find_output_secs(&outputs_eager_qual, 0),
                        "t_eval=0 expected eager qual to be true, got {}",
                        find_output_secs(&outputs_eager_qual, 0)
                    )
                }
                // at 5 f64 emits for 1 and 2, and rosi agrees in bounds
                5 => {
                    assert!(
                        (find_output_secs(&outputs_f64, 1) - 12.0).abs() < 1e-9,
                        "t_eval=1 expected 12.0, got {}",
                        find_output_secs(&outputs_f64, 1)
                    );
                    assert!(
                        (find_output_secs(&outputs_f64, 2) - 13.0).abs() < 1e-9,
                        "t_eval=2 expected 13.0, got {}",
                        find_output_secs(&outputs_f64, 2)
                    );
                    let rosi_val_1 = find_output_secs(&outputs_rosi, 1);
                    let rosi_val_2 = find_output_secs(&outputs_rosi, 2);
                    let rosi_val_5 = find_output_secs(&outputs_rosi, 5);
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
                        find_output_secs(&outputs_eager_qual, 1),
                        "t_eval=1 expected eager qual to be true, got {}",
                        find_output_secs(&outputs_eager_qual, 1)
                    );
                    assert!(
                        find_output_secs(&outputs_eager_qual, 2),
                        "t_eval=2 expected eager qual to be true, got {}",
                        find_output_secs(&outputs_eager_qual, 2)
                    );
                    // it sees the -1.0 at t=5 and finalizes to false immediately, without waiting for t=10
                    assert!(
                        !find_output_secs(&outputs_eager_qual, 5),
                        "t_eval=5 expected eager qual to be false, got {}",
                        find_output_secs(&outputs_eager_qual, 5)
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
                        (find_output_secs(&outputs_f64, 5) + 1.0).abs() < 1e-9,
                        "t_eval=5 expected -1.0, got {}",
                        find_output_secs(&outputs_f64, 5)
                    );
                    let rosi_val = find_output_secs(&outputs_rosi, 5);
                    assert!(
                        rosi_val.0 == -1.0 && rosi_val.1 == -1.0,
                        "t_eval=5 expected ROSI bounds to contain -1.0, got {:?}",
                        rosi_val
                    );
                    // eager can short-circuit to false for t=10 already
                    assert!(
                        !find_output_secs(&outputs_eager_qual, 10),
                        "t_eval=5 expected eager qual to be false, got {}",
                        find_output_secs(&outputs_eager_qual, 10)
                    );
                }
                _ => panic!("unexpected output at t={}", step.timestamp.as_secs()),
            }
        }
    }

    #[test]
    fn overlapping_intervals() {
        // test formula phi_G = G[0,2] x > 0 and phi_F = F[0,2] x>0
        // vals = 1,2,3 ts=1,2,3.5
        // at t=3.5, the output for t_eval=1s must be finalized to 1 (for phi_G) and to 2 for phi_F
        let interval = TimeInterval {
            start: secs(0),
            end: secs(2),
        };
        let atomic = Atomic::<f64>::new_greater_than("x", 0.0);
        let mut globally = Globally::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(atomic.clone()),
            None,
            None,
        );
        let mut eventually = Eventually::<f64, RingBuffer<f64>, f64, false, false>::new(
            interval,
            Box::new(atomic.clone()),
            None,
            None,
        );
        let mut globally_rosi =
            Globally::<f64, RingBuffer<RobustnessInterval>, RobustnessInterval, false, true>::new(
                interval,
                Box::new(Atomic::<RobustnessInterval>::new_greater_than("x", 0.0)),
                None,
                None,
            );

        let mut eventually_rosi = Eventually::<
            f64,
            RingBuffer<RobustnessInterval>,
            RobustnessInterval,
            false,
            true,
        >::new(
            interval,
            Box::new(Atomic::<RobustnessInterval>::new_greater_than("x", 0.0)),
            None,
            None,
        );

        let signal_values = vec![1.0, 2.0, 3.0];
        let signal_timestamps = vec![1000, 2000, 3500];

        let signal: Vec<_> = signal_values
            .into_iter()
            .zip(signal_timestamps)
            .map(|(val, ts)| step!("x", val, Duration::from_millis(ts)))
            .collect();

        for s in &signal {
            let globally_out = globally.update(s);
            let eventually_out = eventually.update(s);
            let globally_rosi_out = globally_rosi.update(s);
            let eventually_rosi_out = eventually_rosi.update(s);

            if s.timestamp == Duration::from_millis(1000) {
                // no delayed verdicts yet
                assert!(
                    globally_out.is_empty() && eventually_out.is_empty(),
                    "t_eval={} expected no output, got {:?}",
                    s.timestamp.as_millis(),
                    globally_out
                );
                // rosi: G has -inf, 1 and F has 1, +inf for t=1s
                let glob_rosi_val = find_output_millis(&globally_rosi_out, 1000);
                let even_rosi_val = find_output_millis(&eventually_rosi_out, 1000);
                assert!(
                    glob_rosi_val.0 == f64::NEG_INFINITY && glob_rosi_val.1 == 1.0,
                    "t_eval=1000 expected G ROSI bounds to be [-inf, 1.0], got {:?}",
                    glob_rosi_val
                );
                assert!(
                    even_rosi_val.0 == 1.0 && even_rosi_val.1 == f64::INFINITY,
                    "t_eval=1000 expected F ROSI bounds to be [1.0, +inf], got {:?}",
                    even_rosi_val
                );
                assert!(
                    globally_rosi_out.len() == 1 && eventually_rosi_out.len() == 1,
                    "t_eval=1000 expected exactly one ROSI output for G and F, got {:?} and {:?}",
                    globally_rosi_out,
                    eventually_rosi_out
                );
            } else if s.timestamp == Duration::from_millis(2000) {
                // no delayed verdicts yet
                assert!(
                    globally_out.is_empty() && eventually_out.is_empty(),
                    "t_eval={} expected no output, got {:?}",
                    s.timestamp.as_millis(),
                    globally_out
                );
                // rosi: G has -inf, 1, -inf,2 and F has 2, +inf, 2, +inf for t=1s and t=2s
                let glob_rosi_val_1 = find_output_millis(&globally_rosi_out, 1000);
                let glob_rosi_val_2 = find_output_millis(&globally_rosi_out, 2000);
                let even_rosi_val_1 = find_output_millis(&eventually_rosi_out, 1000);
                let even_rosi_val_2 = find_output_millis(&eventually_rosi_out, 2000);
                assert!(
                    glob_rosi_val_1.0 == f64::NEG_INFINITY && glob_rosi_val_1.1 == 1.0,
                    "t_eval=1000 expected G ROSI bounds to be [-inf, 1.0], got {:?}",
                    glob_rosi_val_1
                );
                assert!(
                    glob_rosi_val_2.0 == f64::NEG_INFINITY && glob_rosi_val_2.1 == 2.0,
                    "t_eval=2000 expected G ROSI bounds to be [-inf, 2.0], got {:?}",
                    glob_rosi_val_2
                );
                assert!(
                    even_rosi_val_1.0 == 2.0 && even_rosi_val_1.1 == f64::INFINITY,
                    "t_eval=1000 expected F ROSI bounds to be [2.0, +inf], got {:?}",
                    even_rosi_val_1
                );
                assert!(
                    even_rosi_val_2.0 == 2.0 && even_rosi_val_2.1 == f64::INFINITY,
                    "t_eval=2000 expected F ROSI bounds to be [2.0, +inf], got {:?}",
                    even_rosi_val_2
                );
                assert!(
                    globally_rosi_out.len() == 2 && eventually_rosi_out.len() == 2,
                    "t_eval=2000 expected exactly two ROSI outputs for G and F, got {:?} and {:?}",
                    globally_rosi_out,
                    eventually_rosi_out
                );
            } else if s.timestamp == Duration::from_millis(3500) {
                let glob_val = find_output_millis(&globally_out, 1000);
                let ev_val = find_output_millis(&eventually_out, 1000);
                assert!(
                    (glob_val - 1.0).abs() < 1e-9,
                    "t_eval=1000 expected 1.0, got {}",
                    glob_val
                );
                assert!(
                    (ev_val - 2.0).abs() < 1e-9,
                    "t_eval=1000 expected 2.0, got {}",
                    ev_val
                );
                // rosi: G has 1,1 -inf, 2 and -inf, 3 and F has 2,2, 3,+inf, 3,+inf for t=1s, t=2s and t=3.5s
                let glob_rosi_val_1 = find_output_millis(&globally_rosi_out, 1000);
                let glob_rosi_val_2 = find_output_millis(&globally_rosi_out, 2000);
                let glob_rosi_val_3 = find_output_millis(&globally_rosi_out, 3500);
                let even_rosi_val_1 = find_output_millis(&eventually_rosi_out, 1000);
                let even_rosi_val_2 = find_output_millis(&eventually_rosi_out, 2000);
                let even_rosi_val_3 = find_output_millis(&eventually_rosi_out, 3500);

                assert!(
                    glob_rosi_val_1.0 == 1.0 && glob_rosi_val_1.1 == 1.0,
                    "t_eval=1000 expected G ROSI bounds to be [1.0, 1.0], got {:?}",
                    glob_rosi_val_1
                );
                assert!(
                    glob_rosi_val_2.0 == f64::NEG_INFINITY && glob_rosi_val_2.1 == 2.0,
                    "t_eval=2000 expected G ROSI bounds to be [-inf, 2.0], got {:?}",
                    glob_rosi_val_2
                );
                assert!(
                    glob_rosi_val_3.0 == f64::NEG_INFINITY && glob_rosi_val_3.1 == 3.0,
                    "t_eval=3500 expected G ROSI bounds to be [-inf, 3.0], got {:?}",
                    glob_rosi_val_3
                );
                assert!(
                    even_rosi_val_1.0 == 2.0 && even_rosi_val_1.1 == 2.0,
                    "t_eval=1000 expected F ROSI bounds to be [2.0, 2.0], got {:?}",
                    even_rosi_val_1
                );
                assert!(
                    even_rosi_val_2.0 == 3.0 && even_rosi_val_2.1 == f64::INFINITY,
                    "t_eval=2000 expected F ROSI bounds to be [3.0, +inf], got {:?}",
                    even_rosi_val_2
                );
                assert!(
                    even_rosi_val_3.0 == 3.0 && even_rosi_val_3.1 == f64::INFINITY,
                    "t_eval=3500 expected F ROSI bounds to be [3.0, +inf], got {:?}",
                    even_rosi_val_3
                );
                assert!(
                    globally_rosi_out.len() == 3 && eventually_rosi_out.len() == 3,
                    "t_eval=3500 expected exactly three ROSI outputs for G and F, got {:?} and {:?}",
                    globally_rosi_out,
                    eventually_rosi_out
                );
            }
        }
    }
}
