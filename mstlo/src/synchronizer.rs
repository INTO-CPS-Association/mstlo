//! Multi-signal timestamp synchronization and interpolation.
//!
//! The monitor evaluates formulas against time-aligned samples. This module
//! fills timestamp gaps per signal when required, based on a chosen
//! [`SynchronizationStrategy`], and emits synchronized steps through a pending
//! queue.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::iter::Iterator;
use std::ops::{Add, Mul, Sub};
use std::time::Duration;

use crate::ring_buffer::Step;

/// Strategy used to synthesize missing samples at known timeline timestamps.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SynchronizationStrategy {
    /// No synchronization/interpolation; forward only real input steps.
    None,
    #[default]
    /// Zero-order hold: $v(t) = v_0$.
    ZeroOrderHold,
    /// Linear interpolation: $v(t)=v_0 + (v_1-v_0) \cdot \frac{t-t_0}{t_1-t_0}$.
    Linear,
}

/// Value requirements for synchronization interpolation.
///
/// Types must support affine interpolation via `+`, `-`, and scalar multiply by `f64`.
pub trait Interpolatable:
    Copy + Add<Output = Self> + Sub<Output = Self> + Mul<f64, Output = Self>
{
    /// The additive identity used as the default initial value when a signal is
    /// not given an explicit one via [`Synchronizer::set_initial_values`].
    fn zero() -> Self;
}

impl Interpolatable for f64 {
    fn zero() -> Self {
        0.0
    }
}

/// Tagged "last known value" for a signal.
///
/// A signal starts as [`LastValue::Init`] when it is initialized; once its first
/// real sample arrives it becomes [`LastValue::Sample`]. The distinction lets a
/// real sample at `t=0` override the synthetic init instead of being rejected by
/// the monotonicity check.
#[derive(Clone, Debug)]
enum LastValue<T> {
    /// Synthetic initial value at `t=0`; no real sample has arrived yet.
    Init(T),
    /// A real sample.
    Sample(Step<T>),
}

impl<T> LastValue<T> {
    /// The timestamp associated with this value: `t=0` for init, the sample
    /// timestamp otherwise.
    fn timestamp(&self) -> Duration {
        match self {
            LastValue::Init(_) => Duration::ZERO,
            LastValue::Sample(step) => step.timestamp,
        }
    }
}

/// Synchronizer struct that handles interpolation of missing steps across multiple signals.
/// It maintains a timeline of all timestamps and the last known step for each active signal.
/// A signal is considered active if it has received at least one step.
pub struct Synchronizer<T> {
    /// Synchronization/interpolation mode.
    strategy: SynchronizationStrategy,
    /// Last seen real step (or synthetic init) per signal.
    last_steps: HashMap<&'static str, LastValue<T>>,
    /// Configured initial values per signal. Survives [`Synchronizer::reset`].
    init_values: BTreeMap<&'static str, T>,
    /// Global set of observed timestamps used as interpolation targets.
    timeline: BTreeSet<Duration>,
    /// Queue of synchronized outputs to be drained by consumers.
    pub pending: VecDeque<Step<T>>,
}

impl<T> Synchronizer<T>
where
    T: Interpolatable,
{
    /// Creates a new synchronizer with the selected strategy.
    pub fn new(strategy: SynchronizationStrategy) -> Self {
        Self {
            strategy,
            last_steps: HashMap::new(),
            init_values: BTreeMap::new(),
            timeline: BTreeSet::new(),
            pending: VecDeque::new(),
        }
    }

    /// Returns the synchronization strategy used by this synchronizer.
    pub fn strategy(&self) -> SynchronizationStrategy {
        self.strategy
    }

    /// Resets all runtime state (last seen steps, timeline, pending queue) and
    /// re-seeds the configured initial values.
    ///
    /// The synchronization strategy and initial values are preserved.
    pub fn reset(&mut self) {
        self.last_steps.clear();
        self.timeline.clear();
        self.pending.clear();
        self.seed_initial();
    }

    /// Configures initial values for signals and seeds them immediately.
    ///
    /// Each initialized signal is assigned its value at `t=0`; until its first
    /// real sample arrives this value is used for interpolation, and a synthetic
    /// `t=0` step is queued so it can be combined with other signals.
    pub fn set_initial_values(&mut self, init_values: impl IntoIterator<Item = (&'static str, T)>) {
        self.init_values = init_values.into_iter().collect();
        self.seed_initial();
    }

    /// Seeds [`Self::last_steps`] and the pending queue with the configured
    /// initial values (each at `t=0`), in deterministic signal order.
    fn seed_initial(&mut self) {
        for (&signal, &value) in &self.init_values {
            self.last_steps.insert(signal, LastValue::Init(value));
            self.timeline.insert(Duration::ZERO);
            self.pending.push_back(Step {
                signal,
                value,
                timestamp: Duration::ZERO,
            });
        }
    }

    /// Returns estimated heap memory in bytes used by the synchronizer's
    /// internal data structures.
    pub fn heap_size(&self) -> usize {
        self.pending.capacity() * std::mem::size_of::<Step<T>>()
            + self.last_steps.capacity()
                * (std::mem::size_of::<&str>() + std::mem::size_of::<LastValue<T>>() + 1)
            + self.init_values.len() * (std::mem::size_of::<&str>() + std::mem::size_of::<T>() + 1)
            + self.timeline.len()
                * (std::mem::size_of::<Duration>() + 2 * std::mem::size_of::<usize>())
    }

    /// Processes a new real step and generates interpolated steps if necessary.
    /// All resulting steps (interpolated + real) are added to `self.pending`.
    ///
    /// Timestamps must be strictly increasing per signal. Steps violating this
    /// are ignored and a warning is printed.
    pub fn evaluate(&mut self, current_step: Step<T>) {
        let signal_id = current_step.signal;
        let current_time = current_step.timestamp;

        let prev = self.last_steps.get(&signal_id).cloned();

        // Validate that timestamp is strictly increasing for this signal against
        // its last *real* sample. A synthetic init is not a real sample, so a
        // real step at `t=0` overrides it rather than being rejected.
        if let Some(LastValue::Sample(prev_step)) = &prev
            && current_time <= prev_step.timestamp
        {
            eprintln!(
                "Warning: Ignoring step for signal '{}' at {:?}. Timestamp must be strictly increasing (last: {:?}).",
                signal_id, current_time, prev_step.timestamp
            );
            return;
        }

        if self.strategy == SynchronizationStrategy::None {
            self.last_steps
                .insert(signal_id, LastValue::Sample(current_step.clone()));
            self.pending.push_back(current_step);
            return;
        }

        let current_value = current_step.value;

        // 1. Add this new timestamp to the global timeline
        self.timeline.insert(current_time);

        // 2. Determine the previous known value, or early-return when there is
        //    nothing to interpolate from.
        let (prev_time, prev_val) = match &prev {
            Some(LastValue::Init(init_val)) => {
                if current_time > Duration::ZERO {
                    // Init anchors t=0; interpolate forward to the first sample.
                    (Duration::ZERO, *init_val)
                } else {
                    // First real sample lands exactly at t=0: it overrides the
                    // init, so drop the still-queued synthetic t=0 step.
                    self.pending
                        .retain(|s| !(s.signal == signal_id && s.timestamp == Duration::ZERO));
                    self.last_steps
                        .insert(signal_id, LastValue::Sample(current_step.clone()));
                    self.pending.push_back(current_step);
                    self.prune_history();
                    return;
                }
            }
            Some(LastValue::Sample(prev_step)) => (prev_step.timestamp, prev_step.value),
            None => {
                // No prior value at all: nothing to interpolate from.
                self.last_steps
                    .insert(signal_id, LastValue::Sample(current_step.clone()));
                self.pending.push_back(current_step);
                self.prune_history();
                return;
            }
        };

        // 3. Interpolate for this signal at timeline timestamps strictly between
        //    the previous known value and the current sample.
        let missed_timestamps: Vec<Duration> = self
            .timeline
            .range((
                std::ops::Bound::Excluded(prev_time),
                std::ops::Bound::Excluded(current_time),
            ))
            .cloned()
            .collect();

        for t in missed_timestamps {
            let interp_val = match self.strategy {
                SynchronizationStrategy::None => current_value, // this will never be hit
                SynchronizationStrategy::ZeroOrderHold => prev_val,
                SynchronizationStrategy::Linear => {
                    let dt_total = current_time.as_secs_f64() - prev_time.as_secs_f64();
                    let dt_curr = t.as_secs_f64() - prev_time.as_secs_f64();
                    let alpha = if dt_total != 0.0 {
                        dt_curr / dt_total
                    } else {
                        0.0
                    };
                    prev_val + (current_value - prev_val) * alpha
                }
            };

            self.pending.push_back(Step {
                signal: signal_id,
                timestamp: t,
                value: interp_val,
            });
        }

        // 4. Update history for this signal
        self.last_steps
            .insert(signal_id, LastValue::Sample(current_step.clone()));

        // 5. Enqueue the real step
        self.pending.push_back(current_step);

        // 6. Cleanup
        self.prune_history();
    }

    fn prune_history(&mut self) {
        if self.last_steps.is_empty() {
            return;
        }
        let min_frontier = self.last_steps.values().map(|v| v.timestamp()).min();
        if let Some(frontier) = min_frontier {
            let keep = self.timeline.split_off(&frontier);
            self.timeline = keep;
        }
    }
}

// -----------------------------------------------------------------------------
// tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_synchronizer_zero_order_hold() {
        let steps = vec![
            Step {
                signal: "B",
                value: 0.0,
                timestamp: Duration::from_secs(0),
            },
            Step {
                signal: "A",
                value: 1.0,
                timestamp: Duration::from_secs(1),
            },
            Step {
                signal: "A",
                value: 10.0,
                timestamp: Duration::from_secs(2),
            },
            Step {
                signal: "A",
                value: 3.0,
                timestamp: Duration::from_secs(4),
            },
            Step {
                signal: "B",
                value: 30.0,
                timestamp: Duration::from_secs(5),
            },
        ];
        let mut sync = Synchronizer::new(SynchronizationStrategy::ZeroOrderHold);
        let mut result = Vec::new();
        for step in &steps {
            sync.evaluate(step.clone());
            // Drain pending steps
            while let Some(s) = sync.pending.pop_front() {
                println!("Popped step: {:?}", s);
                result.push(s);
            }
        }
        // With zero-order hold, signal A at t=2 should hold value 1.0
        assert!(result.iter().any(|s| s.signal == "B"
            && (s.timestamp == Duration::from_secs(1)
                || s.timestamp == Duration::from_secs(2)
                || s.timestamp == Duration::from_secs(4))
            && s.value == 0.0));
    }

    #[test]
    fn test_synchronizer_linear() {
        let steps = vec![
            Step {
                signal: "A",
                value: 0.0,
                timestamp: Duration::from_secs(0),
            },
            Step {
                signal: "B",
                value: 0.0,
                timestamp: Duration::from_secs(0),
            },
            Step {
                signal: "A",
                value: 10.0,
                timestamp: Duration::from_secs(2),
            },
            Step {
                signal: "B",
                value: 20.0,
                timestamp: Duration::from_secs(4),
            },
        ];
        let mut sync = Synchronizer::new(SynchronizationStrategy::Linear);
        let mut result = Vec::new();
        for step in &steps {
            sync.evaluate(step.clone());
            // Drain pending steps
            while let Some(s) = sync.pending.pop_front() {
                result.push(s);
            }
        }
        // With linear interpolation, at t=2, signal B should be linearly interpolated to 10.0
        assert!(result.iter().any(|s| s.signal == "B"
            && s.timestamp == Duration::from_secs(2)
            && (s.value - 10.0).abs() < 1e-6));
    }

    #[test]
    fn test_non_increasing_timestamp_ignored() {
        let mut sync = Synchronizer::new(SynchronizationStrategy::ZeroOrderHold);

        // First step at t=2
        sync.evaluate(Step {
            signal: "A",
            value: 10.0,
            timestamp: Duration::from_secs(2),
        });
        assert_eq!(sync.pending.len(), 1);
        sync.pending.clear();

        // Valid step at t=3 (strictly increasing)
        sync.evaluate(Step {
            signal: "A",
            value: 15.0,
            timestamp: Duration::from_secs(3),
        });
        assert_eq!(sync.pending.len(), 1);
        sync.pending.clear();

        // Invalid step at t=3 (equal, should be ignored)
        sync.evaluate(Step {
            signal: "A",
            value: 20.0,
            timestamp: Duration::from_secs(3),
        });
        assert_eq!(sync.pending.len(), 0, "Equal timestamp should be ignored");

        // Invalid step at t=1 (decreasing, should be ignored)
        sync.evaluate(Step {
            signal: "A",
            value: 25.0,
            timestamp: Duration::from_secs(1),
        });
        assert_eq!(
            sync.pending.len(),
            0,
            "Decreasing timestamp should be ignored"
        );

        // Valid step at t=5 (strictly increasing again)
        sync.evaluate(Step {
            signal: "A",
            value: 30.0,
            timestamp: Duration::from_secs(5),
        });
        assert_eq!(sync.pending.len(), 1);
    }

    #[test]
    fn test_different_signals_independent_timestamps() {
        let mut sync = Synchronizer::new(SynchronizationStrategy::None);

        // Signal A at t=5
        sync.evaluate(Step {
            signal: "A",
            value: 10.0,
            timestamp: Duration::from_secs(5),
        });
        assert_eq!(sync.pending.len(), 1);
        sync.pending.clear();

        // Signal B at t=2 is valid (different signal)
        sync.evaluate(Step {
            signal: "B",
            value: 20.0,
            timestamp: Duration::from_secs(2),
        });
        assert_eq!(sync.pending.len(), 1);
        sync.pending.clear();

        // Signal A at t=3 is invalid (less than previous A timestamp)
        sync.evaluate(Step {
            signal: "A",
            value: 15.0,
            timestamp: Duration::from_secs(3),
        });
        assert_eq!(sync.pending.len(), 0, "Signal A timestamp must be > 5");

        // Signal B at t=3 is valid (greater than previous B timestamp)
        sync.evaluate(Step {
            signal: "B",
            value: 25.0,
            timestamp: Duration::from_secs(3),
        });
        assert_eq!(sync.pending.len(), 1);
    }

    #[test]
    fn heap_size_empty() {
        let sync: Synchronizer<f64> = Synchronizer::new(SynchronizationStrategy::None);
        assert_eq!(sync.heap_size(), 0);
    }

    #[test]
    fn heap_size_after_evaluate() {
        let mut sync = Synchronizer::new(SynchronizationStrategy::None);
        sync.evaluate(Step {
            signal: "A",
            value: 1.0,
            timestamp: Duration::from_secs(1),
        });
        // pending queue holds at least one Step
        assert!(sync.heap_size() >= std::mem::size_of::<Step<f64>>());
    }

    #[test]
    fn heap_size_after_reset() {
        let mut sync = Synchronizer::new(SynchronizationStrategy::ZeroOrderHold);
        sync.evaluate(Step {
            signal: "A",
            value: 1.0,
            timestamp: Duration::from_secs(1),
        });
        let before = sync.heap_size();
        assert!(before > 0);
        sync.reset();
        // reset clears collections but capacity may remain
        assert!(sync.heap_size() <= before);
    }

    #[test]
    fn test_initial_values_override_at_zero() {
        let mut sync = Synchronizer::new(SynchronizationStrategy::ZeroOrderHold);
        sync.set_initial_values([("x", 0.0), ("y", 10.0)]);

        // A real x@0 sample must override the synthetic init instead of being
        // rejected by the monotonicity check.
        sync.evaluate(Step {
            signal: "x",
            value: 5.0,
            timestamp: Duration::ZERO,
        });

        let mut result = Vec::new();
        while let Some(s) = sync.pending.pop_front() {
            result.push(s);
        }

        // y's init step still flows (t=0, value 10.0).
        assert!(
            result
                .iter()
                .any(|s| s.signal == "y" && s.timestamp == Duration::ZERO && s.value == 10.0)
        );
        // x has exactly one t=0 step and it carries the real value, not the init.
        let x_steps: Vec<_> = result
            .iter()
            .filter(|s| s.signal == "x" && s.timestamp == Duration::ZERO)
            .collect();
        assert_eq!(x_steps.len(), 1);
        assert_eq!(x_steps[0].value, 5.0);
    }

    #[test]
    fn test_initial_values_hold_forward() {
        let mut sync = Synchronizer::new(SynchronizationStrategy::ZeroOrderHold);
        sync.set_initial_values([("x", 0.0), ("y", 10.0)]);
        while let Some(_) = sync.pending.pop_front() {}

        sync.evaluate(Step {
            signal: "x",
            value: 5.0,
            timestamp: Duration::ZERO,
        });
        sync.evaluate(Step {
            signal: "x",
            value: 6.0,
            timestamp: Duration::from_secs(1),
        });
        sync.evaluate(Step {
            signal: "y",
            value: 20.0,
            timestamp: Duration::from_secs(5),
        });

        let mut out = Vec::new();
        while let Some(s) = sync.pending.pop_front() {
            out.push(s);
        }

        // y holds its init value (10.0) at the intermediate timestamp t=1.
        assert!(
            out.iter().any(|s| s.signal == "y"
                && s.timestamp == Duration::from_secs(1)
                && s.value == 10.0)
        );
        assert!(
            out.iter().any(|s| s.signal == "y"
                && s.timestamp == Duration::from_secs(5)
                && s.value == 20.0)
        );
    }
}
