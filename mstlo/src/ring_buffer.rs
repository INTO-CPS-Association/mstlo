//! Ring-buffer primitives used by the STL monitor implementation.
//!
//! This module provides:
//! - [`Step`], a timestamped signal sample,
//! - [`RingBufferTrait`], an abstraction over ring-buffer-like storage, and
//! - [`RingBuffer`], a `VecDeque`-backed implementation.
//!
//! It also includes [`guarded_prune`], a helper for pruning while protecting
//! data needed by pending evaluations.

#[cfg(feature = "track-cache-size")]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{collections::VecDeque, time::Duration};

#[cfg(feature = "track-cache-size")]
/// Global counter for elements currently residing in any ring buffer.
pub static GLOBAL_CACHE_SIZE: AtomicUsize = AtomicUsize::new(0);

/// A single sampled value of a named signal at a given timestamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Step<T> {
    /// Signal identifier this step belongs to.
    pub signal: &'static str,
    /// Sampled value for the signal.
    pub value: T,
    /// Logical/event timestamp of this sample.
    pub timestamp: Duration,
}

impl<T> Step<T> {
    /// Creates a new [`Step`].
    ///
    /// # Arguments
    /// * `signal` - Signal identifier.
    /// * `value` - Sample value.
    /// * `timestamp` - Timestamp associated with the sample.
    pub fn new(signal: &'static str, value: T, timestamp: Duration) -> Self {
        Step {
            signal,
            value,
            timestamp,
        }
    }
}

/// Common interface for timestamped ring buffers.
///
/// Implementors are expected to maintain steps in ascending timestamp order.
pub trait RingBufferTrait {
    /// The value type stored in each [`Step`].
    type Value;

    /// The backing container type that stores steps.
    type Container: IntoIterator;

    /// Immutable iterator over stored steps.
    type Iter<'a>: Iterator<Item = &'a Step<Self::Value>>
    where
        Self: 'a;

    /// Mutable iterator over stored steps.
    type IterMut<'a>: Iterator<Item = &'a mut Step<Self::Value>>
    where
        Self: 'a;

    /// Creates an empty buffer.
    fn new() -> Self;
    /// Returns `true` when the buffer contains no steps.
    fn is_empty(&self) -> bool;
    /// Returns the number of stored steps.
    fn len(&self) -> usize;
    /// Returns the newest step, if any.
    fn get_back(&self) -> Option<&Step<Self::Value>>;
    /// Returns the oldest step, if any.
    fn get_front(&self) -> Option<&Step<Self::Value>>;
    /// Removes and returns the oldest step.
    fn pop_front(&mut self) -> Option<Step<Self::Value>>;
    /// Removes and returns the newest step.
    fn pop_back(&mut self) -> Option<Step<Self::Value>>;

    /// Appends a step to the buffer.
    fn add_step(&mut self, step: Step<Self::Value>);
    /// Replaces a step with matching timestamp.
    ///
    /// Returns `true` if a step was updated, `false` if no matching timestamp
    /// was found.
    fn update_step(&mut self, step: Step<Self::Value>) -> bool;

    /// Prune steps older than `max_age` from the buffer.
    fn prune(&mut self, max_age: Duration);

    /// Returns an iterator over all steps from oldest to newest.
    fn iter<'a>(&'a self) -> Self::Iter<'a>;

    #[cfg(feature = "track-cache-size")]
    /// Enables or disables global cache size tracking for this specific buffer.
    fn set_tracked(&mut self, tracked: bool);
}

/// `VecDeque`-backed ring buffer for timestamped signal steps.
#[derive(Clone, Debug)]
pub struct RingBuffer<T> {
    steps: VecDeque<Step<T>>,
    #[cfg(feature = "track-cache-size")]
    is_tracked: bool,
}

impl<T> Default for RingBuffer<T>
where
    T: Copy,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> RingBuffer<T>
where
    T: Copy,
{
    /// Creates an empty [`RingBuffer`].
    pub fn new() -> Self {
        RingBuffer {
            steps: VecDeque::new(),
            #[cfg(feature = "track-cache-size")]
            is_tracked: false, // Default to false to avoid unintended tracking
        }
    }

    /// Appends a new step.
    pub fn add_step(&mut self, step: Step<T>) {
        self.steps.push_back(step);
        #[cfg(feature = "track-cache-size")]
        if self.is_tracked {
            GLOBAL_CACHE_SIZE.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Replaces the step with the same timestamp.
    ///
    /// Returns `true` when a matching timestamp exists.
    ///
    /// This uses binary search and therefore assumes the internal storage is
    /// sorted by timestamp.
    pub fn update_step(&mut self, step: Step<T>) -> bool {
        self.steps
            .binary_search_by(|s| s.timestamp.cmp(&step.timestamp))
            .map(|index| {
                self.steps[index] = step;
            })
            .is_ok()
    }

    /// Returns an iterator over all stored steps from oldest to newest.
    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, Step<T>> {
        self.steps.iter()
    }
}

impl<T> RingBufferTrait for RingBuffer<T>
where
    T: Copy,
{
    type Value = T;
    type Container = VecDeque<Step<T>>;
    type Iter<'a>
        = std::collections::vec_deque::Iter<'a, Step<T>>
    where
        Self: 'a;

    type IterMut<'a>
        = std::collections::vec_deque::IterMut<'a, Step<T>>
    where
        Self: 'a;

    fn new() -> Self {
        Self::new()
    }
    fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
    fn len(&self) -> usize {
        self.steps.len()
    }
    fn get_back(&self) -> Option<&Step<T>> {
        self.steps.back()
    }
    fn get_front(&self) -> Option<&Step<T>> {
        self.steps.front()
    }
    fn pop_front(&mut self) -> Option<Step<Self::Value>> {
        let res = self.steps.pop_front();
        #[cfg(feature = "track-cache-size")]
        if res.is_some() && self.is_tracked {
            GLOBAL_CACHE_SIZE.fetch_sub(1, Ordering::Relaxed);
        }
        res
    }
    fn pop_back(&mut self) -> Option<Step<Self::Value>> {
        let res = self.steps.pop_back();
        #[cfg(feature = "track-cache-size")]
        if res.is_some() && self.is_tracked {
            GLOBAL_CACHE_SIZE.fetch_sub(1, Ordering::Relaxed);
        }
        res
    }

    fn add_step(&mut self, step: Step<T>) {
        self.add_step(step)
    }
    fn update_step(&mut self, step: Step<Self::Value>) -> bool {
        self.update_step(step)
    }
    fn prune(&mut self, max_age: Duration) {
        let current_time = match self.get_back() {
            Some(step) => step.timestamp,
            None => return, // Buffer is empty, nothing to prune
        };
        let max_age = current_time.saturating_sub(max_age);

        #[cfg(feature = "track-cache-size")]
        {
            let mut removed = 0;
            while let Some(front_step) = self.steps.front() {
                if front_step.timestamp < max_age {
                    removed += 1;
                    self.steps.pop_front();
                } else {
                    break;
                }
            }
            if self.is_tracked && removed > 0 {
                GLOBAL_CACHE_SIZE.fetch_sub(removed, Ordering::Relaxed);
            }
        }
        #[cfg(not(feature = "track-cache-size"))]
        {
            while let Some(front_step) = self.steps.front() {
                if front_step.timestamp < max_age {
                    self.steps.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        self.iter()
    }

    #[cfg(feature = "track-cache-size")]
    fn set_tracked(&mut self, tracked: bool) {
        self.is_tracked = tracked;
    }
}
#[cfg(feature = "track-cache-size")]
impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        let remaining = self.steps.len();
        if remaining > 0 && self.is_tracked {
            GLOBAL_CACHE_SIZE.fetch_sub(remaining, Ordering::Relaxed);
        }
    }
}

/// Prunes a cache while protecting entries at or after a given timestamp.
///
/// This is useful when pruning caches with zero lookahead but pending evaluations
/// that require recent data. The function ensures that all entries at or after
/// `protected_ts` are preserved, even if `lookahead` would normally allow their removal.
///
/// # Arguments
/// * `cache` - The ring buffer to prune
/// * `lookahead` - The normal lookahead duration for pruning
/// * `protected_ts` - Timestamp to protect; entries at or after this will not be pruned
pub fn guarded_prune<C>(cache: &mut C, lookahead: Duration, protected_ts: Duration)
where
    C: RingBufferTrait,
{
    let Some(back) = cache.get_back() else { return };
    // Preserve entries at or after the earliest pending evaluation timestamp.
    let distance_to_protected = back.timestamp.saturating_sub(protected_ts);
    let effective_max_age = lookahead.max(distance_to_protected);
    cache.prune(effective_max_age);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_creation() {
        let mut signal = RingBuffer::new();
        signal.add_step(Step {
            signal: "x",
            value: 1,
            timestamp: Duration::new(0, 0),
        });
        signal.add_step(Step {
            signal: "x",
            value: 2,
            timestamp: Duration::new(0, 0),
        });
        signal.add_step(Step {
            signal: "x",
            value: 3,
            timestamp: Duration::new(0, 0),
        });

        for i in 0..3 {
            if let Some(step) = signal.steps.get(i) {
                assert_eq!(step.value, i + 1)
            }
        }
    }

    #[test]
    fn ring_prune() {
        let mut signal = RingBuffer::new();
        signal.add_step(Step {
            signal: "x",
            value: 1,
            timestamp: Duration::from_secs(1),
        });
        signal.add_step(Step {
            signal: "x",
            value: 2,
            timestamp: Duration::from_secs(2),
        });
        signal.add_step(Step {
            signal: "x",
            value: 3,
            timestamp: Duration::from_secs(3),
        });

        // Prune steps older than 1 second from the latest timestamp (which is 3 seconds)
        // This should remove the step with timestamp 1 second
        signal.prune(Duration::from_secs(1));

        assert_eq!(signal.len(), 2);
        assert_eq!(signal.get_front().unwrap().value, 2);
        assert_eq!(signal.get_back().unwrap().value, 3);
    }
    #[test]
    fn ring_get_back() {
        let mut signal = RingBuffer::new();
        signal.add_step(Step {
            signal: "x",
            value: 1,
            timestamp: Duration::from_secs(1),
        });
        signal.add_step(Step {
            signal: "x",
            value: 2,
            timestamp: Duration::from_secs(2),
        });
        signal.add_step(Step {
            signal: "x",
            value: 3,
            timestamp: Duration::from_secs(3),
        });
        let back_step = signal.get_back().unwrap();
        assert_eq!(back_step.value, 3);
    }
    #[test]
    fn ring_get_front() {
        let mut signal = RingBuffer::new();
        signal.add_step(Step {
            signal: "x",
            value: 1,
            timestamp: Duration::from_secs(1),
        });
        signal.add_step(Step {
            signal: "x",
            value: 2,
            timestamp: Duration::from_secs(2),
        });
        signal.add_step(Step {
            signal: "x",
            value: 3,
            timestamp: Duration::from_secs(3),
        });
        let front_step = signal.get_front().unwrap();
        assert_eq!(front_step.value, 1);
    }
    #[test]
    fn ring_pop_front() {
        let mut signal = RingBuffer::new();
        signal.add_step(Step {
            signal: "x",
            value: 1,
            timestamp: Duration::from_secs(1),
        });
        signal.add_step(Step {
            signal: "x",
            value: 2,
            timestamp: Duration::from_secs(2),
        });
        signal.add_step(Step {
            signal: "x",
            value: 3,
            timestamp: Duration::from_secs(3),
        });
        let popped_step = signal.pop_front().unwrap();
        assert_eq!(popped_step.value, 1);
        assert_eq!(signal.len(), 2);
        assert_eq!(signal.get_front().unwrap().value, 2);
        assert_eq!(signal.get_back().unwrap().value, 3);
    }
    #[test]
    fn ring_iter() {
        let mut signal = RingBuffer::new();
        signal.add_step(Step {
            signal: "x",
            value: 1,
            timestamp: Duration::from_secs(1),
        });
        signal.add_step(Step {
            signal: "x",
            value: 2,
            timestamp: Duration::from_secs(2),
        });
        signal.add_step(Step {
            signal: "x",
            value: 3,
            timestamp: Duration::from_secs(3),
        });
        let mut iter = signal.iter();
        assert_eq!(iter.next().unwrap().value, 1);
        assert_eq!(iter.next().unwrap().value, 2);
        assert_eq!(iter.next().unwrap().value, 3);
        assert!(iter.next().is_none());
    }
    #[test]
    fn ring_is_empty() {
        let signal: RingBuffer<bool> = RingBuffer::new();
        assert!(signal.is_empty());
    }

    #[test]
    fn default_ring_buffer() {
        let signal: RingBuffer<i32> = RingBuffer::default();
        assert!(signal.is_empty());
    }

    #[test]
    fn ring_prune_empty_buffer() {
        let mut signal: RingBuffer<f64> = RingBuffer::new();
        // Pruning an empty buffer should be a no-op
        signal.prune(Duration::from_secs(1));
        assert!(signal.is_empty());
    }

    #[test]
    fn ring_update_step() {
        let mut signal = RingBuffer::new();
        signal.add_step(Step {
            signal: "x",
            value: 1,
            timestamp: Duration::from_secs(1),
        });
        signal.add_step(Step {
            signal: "x",
            value: 2,
            timestamp: Duration::from_secs(2),
        });

        // Update existing timestamp
        let updated = signal.update_step(Step {
            signal: "x",
            value: 10,
            timestamp: Duration::from_secs(1),
        });
        assert!(updated);
        assert_eq!(signal.get_front().unwrap().value, 10);

        // Update non-existing timestamp
        let not_updated = signal.update_step(Step {
            signal: "x",
            value: 20,
            timestamp: Duration::from_secs(5),
        });
        assert!(!not_updated);
    }
}
