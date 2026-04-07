//! Multi-signal timestamp synchronization and interpolation.
//!
//! The monitor evaluates formulas against time-aligned samples. This module
//! fills timestamp gaps per signal when required, based on a chosen
//! [`SynchronizationStrategy`], and emits synchronized steps through a pending
//! queue.

use std::collections::{BTreeSet, HashMap, VecDeque};
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
}

impl Interpolatable for f64 {}

/// Synchronizer struct that handles interpolation of missing steps across multiple signals.
/// It maintains a timeline of all timestamps and the last known step for each active signal.
/// A signal is considered active if it has received at least one step.
pub struct Synchronizer<T> {
    /// Synchronization/interpolation mode.
    strategy: SynchronizationStrategy,
    /// Last seen real step per signal.
    last_steps: HashMap<&'static str, Step<T>>,
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
            timeline: BTreeSet::new(),
            pending: VecDeque::new(),
        }
    }

    /// Returns the synchronization strategy used by this synchronizer.
    pub fn strategy(&self) -> SynchronizationStrategy {
        self.strategy
    }

    /// Processes a new real step and generates interpolated steps if necessary.
    /// All resulting steps (interpolated + real) are added to `self.pending`.
    ///
    /// Timestamps must be strictly increasing per signal. Steps violating this
    /// are ignored and a warning is printed.
    pub fn evaluate(&mut self, current_step: Step<T>) {
        let signal_id = current_step.signal;
        let current_time = current_step.timestamp;

        // Validate that timestamp is strictly increasing for this signal
        if let Some(prev_step) = self.last_steps.get(&signal_id)
            && current_time <= prev_step.timestamp
        {
            eprintln!(
                "Warning: Ignoring step for signal '{}' at {:?}. Timestamp must be strictly increasing (last: {:?}).",
                signal_id, current_time, prev_step.timestamp
            );
            return;
        }

        if self.strategy == SynchronizationStrategy::None {
            self.last_steps.insert(signal_id, current_step.clone());
            self.pending.push_back(current_step);
            return;
        }

        let current_value = current_step.value;

        // 1. Add this new timestamp to the global timeline
        self.timeline.insert(current_time);

        // 2. Check if we can interpolate for this specific signal
        if let Some(prev_step) = self.last_steps.get(&signal_id).cloned() {
            let prev_time = prev_step.timestamp;
            let prev_val = prev_step.value;

            if current_time > prev_time {
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
            }
        }

        // 3. Update history for this signal
        self.last_steps.insert(signal_id, current_step.clone());

        // 4. Enqueue the real step
        self.pending.push_back(current_step);

        // 5. Cleanup
        self.prune_history();
    }

    fn prune_history(&mut self) {
        if self.last_steps.is_empty() {
            return;
        }
        let min_frontier = self.last_steps.values().map(|s| s.timestamp).min();
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
}
