//! Input admission and the signal model.
//!
//! Every sample that reaches the operator tree passes through here first. Two things
//! happen: timestamps are checked to be strictly increasing per signal, and
//! the [`SignalInterpolation`] the monitor was configured with is held so the layers that
//! act on it can read it back.
//!
//! No samples are synthesized here. Reading a signal between its own samples is applied at
//! the predicate layer instead; see [`SignalInterpolation`].

use std::collections::{HashMap, VecDeque};
use std::ops::{Add, Mul, Sub};
use std::time::Duration;

use crate::ring_buffer::Step;

/// How an input signal is read *between* two consecutive samples.
///
/// This is a property of each signal, configured at the monitor level.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SignalInterpolation {
    /// The signal holds its last value until the next sample: $v(t) = v_0$.
    #[default]
    ZeroOrderHold,
    /// The signal runs straight between samples:
    /// $v(t)=v_0 + (v_1-v_0) \cdot \frac{t-t_0}{t_1-t_0}$.
    Linear,
}

/// Deprecated configuration surface, superseded by [`SignalInterpolation`].
///
/// It asked how several signals are aligned onto a common timeline, which has no
/// well-defined answer: the result depended on the sample rates of signals the formula
/// never mentioned. What callers meant by it was always how a signal is read between its
/// own samples, so each variant now simply selects the interpolation of the same name.
#[deprecated(
    since = "0.2.0",
    note = "use `SignalInterpolation`; `None` and `ZeroOrderHold` both mean \
            `SignalInterpolation::ZeroOrderHold`, and `Linear` means `SignalInterpolation::Linear`"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynchronizationStrategy {
    /// No synchronization; selects [`SignalInterpolation::ZeroOrderHold`].
    None,
    /// Zero-order hold; selects [`SignalInterpolation::ZeroOrderHold`].
    ZeroOrderHold,
    /// Selects [`SignalInterpolation::Linear`].
    Linear,
}

#[allow(deprecated, clippy::derivable_impls)]
impl Default for SynchronizationStrategy {
    fn default() -> Self {
        Self::ZeroOrderHold
    }
}

#[allow(deprecated)]
impl SynchronizationStrategy {
    /// The interpolation this strategy selects.
    pub fn interpolation(self) -> SignalInterpolation {
        match self {
            SynchronizationStrategy::Linear => SignalInterpolation::Linear,
            SynchronizationStrategy::None | SynchronizationStrategy::ZeroOrderHold => {
                SignalInterpolation::ZeroOrderHold
            }
        }
    }
}

/// Value requirements for linear interpolation.
///
/// Types must support affine interpolation via `+`, `-`, and scalar multiply by `f64`.
pub trait Interpolatable:
    Copy + Add<Output = Self> + Sub<Output = Self> + Mul<f64, Output = Self>
{
}

impl Interpolatable for f64 {}

/// Admits input steps on their way to the operator tree.
///
/// Holds the [`SignalInterpolation`] in force and the last timestamp seen per signal, so
/// that a signal cannot go backwards in time. Admitted steps are placed on [`Self::pending`]
/// unchanged and in arrival order; nothing else is ever put there.
pub struct Synchronizer<T> {
    /// How signals are read between their own samples. Acted on at the predicate layer,
    /// recorded here because this is where the input model is decided.
    interpolation: SignalInterpolation,
    /// Timestamp of the last admitted step per signal.
    last_timestamps: HashMap<&'static str, Duration>,
    /// Queue of admitted steps to be drained by consumers.
    pub pending: VecDeque<Step<T>>,
}

impl<T> Synchronizer<T>
where
    T: Interpolatable,
{
    /// Creates a new synchronizer reading signals under `interpolation`.
    pub fn new(interpolation: SignalInterpolation) -> Self {
        Self {
            interpolation,
            last_timestamps: HashMap::new(),
            pending: VecDeque::new(),
        }
    }

    /// Returns how this synchronizer reads signals between their own samples.
    pub fn interpolation(&self) -> SignalInterpolation {
        self.interpolation
    }

    /// Resets all runtime state (last seen timestamps, pending queue).
    ///
    /// The signal interpolation is preserved.
    pub fn reset(&mut self) {
        self.last_timestamps.clear();
        self.pending.clear();
    }

    /// Returns estimated heap memory in bytes used by the synchronizer's
    /// internal data structures.
    pub fn heap_size(&self) -> usize {
        self.pending.capacity() * std::mem::size_of::<Step<T>>()
            + self.last_timestamps.capacity()
                * (std::mem::size_of::<&str>() + std::mem::size_of::<Duration>() + 1)
    }

    /// Admits a new step, appending it to `self.pending`.
    ///
    /// Timestamps must be strictly increasing per signal. Steps violating this
    /// are ignored and a warning is printed.
    pub fn evaluate(&mut self, current_step: Step<T>) {
        let signal_id = current_step.signal;
        let current_time = current_step.timestamp;

        // Validate that timestamp is strictly increasing for this signal
        if let Some(prev_time) = self.last_timestamps.get(&signal_id)
            && current_time <= *prev_time
        {
            eprintln!(
                "Warning: Ignoring step for signal '{}' at {:?}. Timestamp must be strictly increasing (last: {:?}).",
                signal_id, current_time, prev_time
            );
            return;
        }

        self.last_timestamps.insert(signal_id, current_time);
        self.pending.push_back(current_step);
    }
}

// -----------------------------------------------------------------------------
// tests
#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Steps are forwarded verbatim under either interpolation; nothing is synthesized.
    #[test]
    fn test_only_real_steps_are_forwarded() {
        let steps = vec![
            Step::new("A", 0.0, Duration::from_secs(0)),
            Step::new("B", 0.0, Duration::from_secs(0)),
            Step::new("A", 10.0, Duration::from_secs(2)),
            Step::new("B", 20.0, Duration::from_secs(4)),
        ];

        for interpolation in [
            SignalInterpolation::ZeroOrderHold,
            SignalInterpolation::Linear,
        ] {
            let mut sync = Synchronizer::new(interpolation);
            let mut result = Vec::new();
            for step in &steps {
                sync.evaluate(step.clone());
                while let Some(s) = sync.pending.pop_front() {
                    result.push(s);
                }
            }
            assert_eq!(
                result, steps,
                "{:?}: only the real steps may be forwarded",
                interpolation
            );
        }
    }

    /// The deprecated strategy is nothing but a name for an interpolation.
    #[test]
    fn test_strategy_maps_onto_interpolation() {
        assert_eq!(
            SynchronizationStrategy::Linear.interpolation(),
            SignalInterpolation::Linear
        );
        for held in [
            SynchronizationStrategy::None,
            SynchronizationStrategy::ZeroOrderHold,
        ] {
            assert_eq!(held.interpolation(), SignalInterpolation::ZeroOrderHold);
        }
    }

    #[test]
    fn test_non_increasing_timestamp_ignored() {
        let mut sync = Synchronizer::new(SignalInterpolation::ZeroOrderHold);

        // First step at t=2
        sync.evaluate(Step::new("A", 10.0, Duration::from_secs(2)));
        assert_eq!(sync.pending.len(), 1);
        sync.pending.clear();

        // Valid step at t=3 (strictly increasing)
        sync.evaluate(Step::new("A", 15.0, Duration::from_secs(3)));
        assert_eq!(sync.pending.len(), 1);
        sync.pending.clear();

        // Invalid step at t=3 (equal, should be ignored)
        sync.evaluate(Step::new("A", 20.0, Duration::from_secs(3)));
        assert_eq!(sync.pending.len(), 0, "Equal timestamp should be ignored");

        // Invalid step at t=1 (decreasing, should be ignored)
        sync.evaluate(Step::new("A", 25.0, Duration::from_secs(1)));
        assert_eq!(
            sync.pending.len(),
            0,
            "Decreasing timestamp should be ignored"
        );

        // Valid step at t=5 (strictly increasing again)
        sync.evaluate(Step::new("A", 30.0, Duration::from_secs(5)));
        assert_eq!(sync.pending.len(), 1);
    }

    #[test]
    fn test_different_signals_independent_timestamps() {
        let mut sync = Synchronizer::new(SignalInterpolation::ZeroOrderHold);

        // Signal A at t=5
        sync.evaluate(Step::new("A", 10.0, Duration::from_secs(5)));
        assert_eq!(sync.pending.len(), 1);
        sync.pending.clear();

        // Signal B at t=2 is valid (different signal)
        sync.evaluate(Step::new("B", 20.0, Duration::from_secs(2)));
        assert_eq!(sync.pending.len(), 1);
        sync.pending.clear();

        // Signal A at t=3 is invalid (less than previous A timestamp)
        sync.evaluate(Step::new("A", 15.0, Duration::from_secs(3)));
        assert_eq!(sync.pending.len(), 0, "Signal A timestamp must be > 5");

        // Signal B at t=3 is valid (greater than previous B timestamp)
        sync.evaluate(Step::new("B", 25.0, Duration::from_secs(3)));
        assert_eq!(sync.pending.len(), 1);
    }

    #[test]
    fn heap_size_empty() {
        let sync: Synchronizer<f64> = Synchronizer::new(SignalInterpolation::ZeroOrderHold);
        assert_eq!(sync.heap_size(), 0);
    }

    #[test]
    fn heap_size_after_evaluate() {
        let mut sync = Synchronizer::new(SignalInterpolation::ZeroOrderHold);
        sync.evaluate(Step::new("A", 1.0, Duration::from_secs(1)));
        // pending queue holds at least one Step
        assert!(sync.heap_size() >= std::mem::size_of::<Step<f64>>());
    }

    #[test]
    fn heap_size_after_reset() {
        let mut sync = Synchronizer::new(SignalInterpolation::ZeroOrderHold);
        sync.evaluate(Step::new("A", 1.0, Duration::from_secs(1)));
        let before = sync.heap_size();
        assert!(before > 0);
        sync.reset();
        // reset clears collections but capacity may remain
        assert!(sync.heap_size() <= before);
    }
}
