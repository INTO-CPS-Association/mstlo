//! Input admission and the signal model.
//!
//! Every sample passes through here before reaching the operator tree. Timestamps are
//! checked to be strictly increasing per signal, and each signal's initial value is
//! emitted at `t=0`; see [`Synchronizer::set_initial_values`]. The configured
//! [`SignalInterpolation`] is stored here but applied at the predicate layer.

use std::collections::{BTreeMap, HashMap, VecDeque};
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

/// Deprecated; superseded by [`SignalInterpolation`]. Each variant selects the
/// interpolation of the same name.
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
    /// The value a signal is initialized to when no other one is given.
    fn zero() -> Self;
}

impl Interpolatable for f64 {
    fn zero() -> Self {
        0.0
    }
}

/// Admits input steps on their way to the operator tree.
///
/// Rejects steps that go backwards in time per signal, and places admitted steps on
/// [`Self::pending`] in arrival order, preceded by any initial values.
pub struct Synchronizer<T> {
    /// How signals are read between their own samples. Applied at the predicate layer.
    interpolation: SignalInterpolation,
    /// Timestamp of the last admitted step per signal.
    last_timestamps: HashMap<&'static str, Duration>,
    /// Initial value per signal, as configured. Survives [`Self::reset`].
    initial_values: BTreeMap<&'static str, T>,
    /// Those of [`Self::initial_values`] not yet resolved, in emission order.
    pending_inits: BTreeMap<&'static str, T>,
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
            initial_values: BTreeMap::new(),
            pending_inits: BTreeMap::new(),
            pending: VecDeque::new(),
        }
    }

    /// Defines each of `initial_values` from `t=0`, until its own first sample arrives.
    ///
    /// Initial values are emitted as steps at `t=0` when the first sample past `t=0` is
    /// admitted. A signal with its own sample at `t=0` before then gets none.
    pub fn set_initial_values(
        &mut self,
        initial_values: impl IntoIterator<Item = (&'static str, T)>,
    ) where
        T: Copy,
    {
        self.initial_values = initial_values.into_iter().collect();
        self.pending_inits = self.initial_values.clone();
    }

    /// Whether any signal is defined from `t=0` by [`Self::set_initial_values`].
    pub fn has_initial_values(&self) -> bool {
        !self.initial_values.is_empty()
    }

    /// Returns how this synchronizer reads signals between their own samples.
    pub fn interpolation(&self) -> SignalInterpolation {
        self.interpolation
    }

    /// Resets all runtime state (last seen timestamps, pending queue).
    ///
    /// The signal interpolation and the initial values are preserved, and the latter are
    /// armed again.
    pub fn reset(&mut self)
    where
        T: Copy,
    {
        self.last_timestamps.clear();
        self.pending.clear();
        self.pending_inits = self.initial_values.clone();
    }

    /// Returns estimated heap memory in bytes used by the synchronizer's
    /// internal data structures.
    pub fn heap_size(&self) -> usize {
        self.pending.capacity() * std::mem::size_of::<Step<T>>()
            + self.last_timestamps.capacity()
                * (std::mem::size_of::<&str>() + std::mem::size_of::<Duration>() + 1)
            + (self.initial_values.len() + self.pending_inits.len())
                * (std::mem::size_of::<&str>() + std::mem::size_of::<T>() + 1)
    }

    /// Admits a new step, appending it to `self.pending`.
    ///
    /// Timestamps must be strictly increasing per signal. Steps violating this
    /// are ignored and a warning is printed.
    ///
    /// Any initial value still owed is emitted first; see [`Self::set_initial_values`].
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

        if !self.pending_inits.is_empty() {
            if current_time == Duration::ZERO {
                // The signal defines itself at t=0; its initial value is not needed.
                self.pending_inits.remove(&signal_id);
            } else {
                // Past t=0 the prefix is fixed: every signal still without a sample takes
                // its initial value from t=0.
                for (signal, value) in std::mem::take(&mut self.pending_inits) {
                    self.last_timestamps.insert(signal, Duration::ZERO);
                    self.pending
                        .push_back(Step::new(signal, value, Duration::ZERO));
                }
            }
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

    /// Drains everything admitted so far.
    fn drain(sync: &mut Synchronizer<f64>) -> Vec<Step<f64>> {
        std::iter::from_fn(|| sync.pending.pop_front()).collect()
    }

    /// An initial value is emitted at `t=0`, but not before a sample past `t=0` needs it.
    #[test]
    fn test_initial_values_flushed_at_first_step_past_zero() {
        let mut sync = Synchronizer::new(SignalInterpolation::ZeroOrderHold);
        sync.set_initial_values([("A", 1.0), ("B", 10.0)]);
        assert!(sync.pending.is_empty(), "nothing is owed before a sample");

        let sample = Step::new("A", 5.0, Duration::from_secs(1));
        sync.evaluate(sample.clone());

        assert_eq!(
            drain(&mut sync),
            vec![
                Step::new("A", 1.0, Duration::ZERO),
                Step::new("B", 10.0, Duration::ZERO),
                sample,
            ],
            "both signals are defined from t=0, in signal order, before the sample"
        );
    }

    /// A signal sampled at `t=0` defines itself there; its initial value is dropped.
    #[test]
    fn test_real_zero_sample_overrides_initial_value() {
        let mut sync = Synchronizer::new(SignalInterpolation::ZeroOrderHold);
        sync.set_initial_values([("A", 1.0), ("B", 10.0)]);

        let real = Step::new("A", 5.0, Duration::ZERO);
        sync.evaluate(real.clone());
        assert_eq!(drain(&mut sync), vec![real], "no initial value for A");

        // B has still not been sampled, so it is the only one left to define.
        let past_zero = Step::new("A", 6.0, Duration::from_secs(1));
        sync.evaluate(past_zero.clone());
        assert_eq!(
            drain(&mut sync),
            vec![Step::new("B", 10.0, Duration::ZERO), past_zero]
        );
    }

    /// Initial values survive a reset and are owed again.
    #[test]
    fn test_reset_rearms_initial_values() {
        let mut sync = Synchronizer::new(SignalInterpolation::ZeroOrderHold);
        sync.set_initial_values([("A", 1.0)]);
        sync.evaluate(Step::new("A", 5.0, Duration::from_secs(1)));
        drain(&mut sync);

        sync.reset();
        sync.evaluate(Step::new("A", 7.0, Duration::from_secs(1)));
        assert_eq!(
            drain(&mut sync),
            vec![
                Step::new("A", 1.0, Duration::ZERO),
                Step::new("A", 7.0, Duration::from_secs(1)),
            ]
        );
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
