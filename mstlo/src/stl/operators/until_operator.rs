//! Temporal `Until` operator implementation.
//!
//! This module provides [`Until`], a stateful evaluator for `φ U[a,b] ψ` over
//! streamed samples, supporting delayed, eager, and refinable (RoSI) modes via
//! const generics.

use crate::ring_buffer::{RingBufferTrait, Step, guarded_prune};
use crate::stl::core::{
    RobustnessSemantics, SignalIdentifier, StlOperatorAndSignalIdentifier, StlOperatorTrait,
    TimeInterval,
};
use std::collections::{BTreeSet, HashSet};
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
    eval_buffer: BTreeSet<Duration>,
    left_signals_set: HashSet<&'static str>,
    right_signals_set: HashSet<&'static str>,
    max_lookahead: Duration,
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
                eval_buffer: BTreeSet::new(),
                left_signals_set: HashSet::new(),
                right_signals_set: HashSet::new(),
                max_lookahead,
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
                eval_buffer: BTreeSet::new(),
                left_signals_set: HashSet::new(),
                right_signals_set: HashSet::new(),
                max_lookahead,
            }
        }
    }

    /// Adds a step to a cache.
    ///
    /// In RoSI mode this performs update-or-insert by timestamp; otherwise it
    /// appends the step.
    fn add_to_cache<const ROSI: bool>(cache: &mut C, step: Step<Y>)
    where
        C: RingBufferTrait<Value = Y>,
        Y: Clone,
    {
        if ROSI {
            if !cache.update_step(step.clone()) {
                cache.add_step(step);
            }
        } else {
            cache.add_step(step);
        }
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

        let t_max_combined = self.t_max.0.min(self.t_max.1);

        // Add all updates to eval_buffer and caches
        for update in &right_updates {
            self.eval_buffer.insert(update.timestamp);
            Self::add_to_cache::<IS_ROSI>(&mut self.right_cache, update.clone());
        }
        for update in &left_updates {
            self.eval_buffer.insert(update.timestamp);
            Self::add_to_cache::<IS_ROSI>(&mut self.left_cache, update.clone());
        }
        let mut tasks_to_remove = Vec::new();
        let current_time = step.timestamp;

        // If there is no data in the left cache it cannot be calculated yet
        if self.left_cache.is_empty() || self.right_cache.is_empty() {
            return output_robustness;
        }

        // 2. Process the evaluation buffer for tasks
        for &t_eval in self.eval_buffer.iter() {
            let window_start_t_eval = t_eval;
            let window_end_t_eval = t_eval + self.interval.end;

            // This is the outer `max` (Eventually)
            let mut max_robustness_vec = Vec::new();
            let mut falsified = false;

            // We can only evaluate up to the data we have.
            // We must use the minimum of the current time and the window end.
            let effective_end_time = current_time.min(window_end_t_eval);

            // Iterate over all t' in [window_start_t_eval, effective_end_time].
            // We use the eval_buffer as the source of t' timestamps.
            let t_prime_iter = self
                .eval_buffer
                .iter()
                .copied()
                .skip_while(|s| s < &window_start_t_eval)
                .take_while(|s| s <= &effective_end_time); // Only up to current time

            let mut left_cache_t_prime_min = Y::globally_identity();
            // Collect left_cache into a vector for progressive skipping
            let left_cache_vec: Vec<_> = self.left_cache.iter().collect();
            let right_cache_vec: Vec<_> = self.right_cache.iter().collect();

            // Find how many elements to skip based on t_eval position
            let skip_count = left_cache_vec
                .iter()
                .take_while(|entry| entry.timestamp < t_eval)
                .count();

            let mut left_cache_iter = left_cache_vec.iter().skip(skip_count);
            let mut right_cache_iter = right_cache_vec.iter().skip(skip_count);

            for _ in t_prime_iter {
                // 1. Get cumulative min of phi (left operand) up to t'
                let left_step = match left_cache_iter.next() {
                    Some(step) => step,
                    None => break,
                };
                left_cache_t_prime_min = Y::and(left_cache_t_prime_min, left_step.value.clone());
                let robustness_phi_left = left_cache_t_prime_min.clone();

                // 2. Get rho_psi(t') - the right operand at t'
                let robustness_psi_right = match right_cache_iter.next() {
                    Some(val) => val.value.clone(),
                    None => Y::unknown(),
                };

                // 3. Eager falsification check: if phi has become false, short-circuit
                if IS_EAGER && robustness_phi_left == Y::atomic_false() && t_max_combined >= t_eval
                {
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

            if current_time >= t_eval + self.get_max_lookahead() {
                // Case 1: Full window *and* child lookaheads have passed.
                // This is a final, "closed" value for delayed mode.
                // (This also captures Eager results that were `false` until the end)
                final_value = Some(max_robustness);
                remove_task = true;
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
            }
        }

        // 3. Prune the caches and remove completed tasks from the buffer.
        let protected_ts = self.eval_buffer.first().copied().unwrap_or(Duration::ZERO);
        let lookahead = self.max_lookahead;
        guarded_prune(&mut self.left_cache, lookahead, protected_ts);
        guarded_prune(&mut self.right_cache, lookahead, protected_ts);

        for t in tasks_to_remove {
            // println!("Removing completed task at t_eval={:?}", t);
            self.eval_buffer.remove(&t);
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
    use crate::ring_buffer::{RingBuffer, Step};
    use crate::stl::core::{StlOperatorTrait, TimeInterval};
    use crate::stl::operators::atomic_operators::Atomic;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn debug_rosi() {
        use crate::stl::core::RobustnessInterval;
        use crate::stl::operators::unary_temporal_operators::{Eventually, Globally};
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
            Step::new("x", 1.0, Duration::from_secs(0)),
            Step::new("x", 2.0, Duration::from_secs(1)),
            Step::new("x", 3.0, Duration::from_secs(2)),
            Step::new("x", 8.0, Duration::from_secs(3)),
            Step::new("x", 10.0, Duration::from_secs(6)),
            Step::new("x", 15.0, Duration::from_secs(8)),
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
        use crate::stl::operators::unary_temporal_operators::{Eventually, Globally};
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
            Step::new("x", 1.0, Duration::from_secs(0)),
            Step::new("x", 2.0, Duration::from_secs(1)),
            Step::new("x", 3.0, Duration::from_secs(2)),
            Step::new("x", 8.0, Duration::from_secs(3)),
            Step::new("x", 12.0, Duration::from_secs(6)),
            Step::new("x", 15.0, Duration::from_secs(8)),
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
        use crate::stl::operators::unary_temporal_operators::{Eventually, Globally};
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
            Step::new("x", 1.0, Duration::from_secs(0)),
            Step::new("x", 2.0, Duration::from_secs(1)),
            Step::new("x", 3.0, Duration::from_secs(2)),
            Step::new("x", 8.0, Duration::from_secs(3)),
            Step::new("x", 12.0, Duration::from_secs(6)),
            Step::new("x", 15.0, Duration::from_secs(8)),
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
            .map(|(val, ts)| Step::new("x", val, Duration::from_secs(ts)))
            .collect();

        let expected_outputs = [
            Step::new("output", false, Duration::from_secs(0)),
            Step::new("output", true, Duration::from_secs(2)),
            Step::new("output", true, Duration::from_secs(4)),
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
