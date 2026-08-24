//! Thread-local string interner for signal and variable names.
//!
//! Names appear in [`Step`](crate::Step), in formula definitions, and as keys in
//! the synchronizer and operator lookups, where they are compared on every
//! evaluated step. Storing them as `&'static str` keeps those comparisons cheap
//! and keeps [`Step`](crate::Step) `Copy`-friendly, but names read at runtime — from a CSV
//! header, a config file, or a language binding — are not `'static`.
//!
//! [`intern`] bridges that gap: it hands out a `&'static str` per *distinct*
//! name, reusing it for every later request. A program that reads the same
//! signal name a million times allocates it once.
//!
//! # Memory
//!
//! Interned names are never freed — that is what makes the `&'static str` sound.
//! The table is therefore bounded by the number of *distinct* names a program
//! ever uses, which for monitoring workloads is a handful, and it does not grow
//! with the number of steps, updates, or monitors.
//!
//! # Threading
//!
//! The table lives in thread-local storage, so interning needs no lock and no
//! atomics. Monitoring state is single-threaded anyway.
//!
//! The names themselves are leaked, hence genuinely `'static`, and may be sent
//! between threads freely. Only the *table* is per-thread: a name interned on
//! two threads is allocated twice and the two references differ. Nothing in
//! this crate depends on that identity — signal names are matched by contents,
//! and hashed by contents — so the sole cost is one extra allocation per name
//! per thread that interns it.
//!

use std::cell::RefCell;
use std::collections::HashSet;

thread_local! {
    /// Every name handed out to this thread so far, keyed by its own contents.
    static INTERNED: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
}

/// Returns a `&'static str` with the same contents as `name`.
///
/// Calling this twice with equal contents *on the same thread* returns the very
/// same reference, so the name is allocated at most once per thread.
///
/// # Example
///
/// ```
/// use mstlo::{Step, intern};
/// use std::time::Duration;
///
/// // A signal name that only exists at runtime, e.g. read from a CSV header.
/// let header = String::from("temperature");
/// let step = Step::new(intern(&header), 21.4, Duration::from_secs(0));
///
/// assert_eq!(step.signal, "temperature");
/// // Interning the same name again reuses the same allocation.
/// assert!(std::ptr::eq(intern("temperature"), step.signal));
/// ```
pub fn intern(name: &str) -> &'static str {
    INTERNED.with_borrow_mut(|table| {
        if let Some(&existing) = table.get(name) {
            return existing;
        }

        let interned: &'static str = Box::leak(name.to_owned().into_boxed_str());
        table.insert(interned);
        interned
    })
}

/// Returns the number of distinct names interned by the current thread.
///
/// Intended for diagnostics and tests; the count never decreases.
pub fn interned_count() -> usize {
    INTERNED.with_borrow(HashSet::len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_equal_names_yields_the_same_reference() {
        let a = intern("interner_test_signal");
        let b = intern(&String::from("interner_test_signal"));

        assert_eq!(a, "interner_test_signal");
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn interning_distinct_names_yields_distinct_references() {
        let a = intern("interner_test_left");
        let b = intern("interner_test_right");

        assert!(!std::ptr::eq(a, b));
        assert_eq!((a, b), ("interner_test_left", "interner_test_right"));
    }

    #[test]
    fn repeated_interning_allocates_once() {
        // Every repeat hands back the same allocation, so the table does not grow.
        let first = intern("interner_test_repeat");

        for _ in 0..1_000 {
            assert!(std::ptr::eq(intern("interner_test_repeat"), first));
        }
    }

    #[test]
    fn interning_a_new_name_grows_the_table() {
        let before = interned_count();
        intern("interner_test_new_name");

        assert!(interned_count() > before);
    }

    #[test]
    fn names_interned_on_another_thread_stay_valid() {
        let from_worker = std::thread::spawn(|| intern("interner_test_cross_thread"))
            .join()
            .expect("worker thread panicked");

        // The leaked name outlives its thread and compares by contents, which
        // is all the crate ever asks of it.
        assert_eq!(from_worker, "interner_test_cross_thread");
        assert_eq!(from_worker, intern("interner_test_cross_thread"));
    }
}
