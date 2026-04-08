//! Core STL abstractions shared across operators, parser, and monitor.
//!
//! This module defines:
//! - time interval primitives ([`TimeInterval`]),
//! - operator traits ([`StlOperatorTrait`], [`SignalIdentifier`]),
//! - robustness domains ([`RobustnessSemantics`], [`RobustnessInterval`]), and
//! - runtime variable bindings ([`Variables`]).

use crate::ring_buffer::Step;
use core::f64;
use dyn_clone::{DynClone, clone_trait_object};
use std::collections::HashSet;
use std::fmt::Display;
use std::ops::Add;
use std::ops::Neg;
use std::ops::Sub;
use std::time::Duration;

/// Closed temporal interval $[start, end]$ used by STL temporal operators.
///
/// Both bounds are represented as [`Duration`] values and interpreted in the
/// same logical time domain as incoming [`Step`] timestamps.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TimeInterval {
    /// Lower bound of the interval.
    pub start: Duration,
    /// Upper bound of the interval.
    pub end: Duration,
}

/// Common trait for all executable STL operators.
///
/// Implementations consume new input samples through [`StlOperatorTrait::update`]
/// and may emit zero, one, or multiple output steps.
///
/// Trait objects are cloneable via [`dyn_clone`].
pub trait StlOperatorTrait<T: Clone>: DynClone + Display + SignalIdentifier {
    /// Output robustness domain (for example `f64`, `bool`, or [`RobustnessInterval`]).
    type Output;

    /// Updates the operator with one input step and returns produced outputs.
    ///
    /// The returned vector may be empty if no output is available yet.
    fn update(&mut self, step: &Step<T>) -> Vec<Step<Self::Output>>;

    /// Returns the maximal lookahead required to finalize outputs.
    ///
    /// For temporal operators this is typically the interval end plus operand
    /// lookahead; for atomic operators this is zero.
    fn get_max_lookahead(&self) -> Duration;
}

clone_trait_object!(<T: Clone, Y> StlOperatorTrait<T, Output = Y>);

/// Trait for extracting referenced signal names from operator/formula trees.
pub trait SignalIdentifier {
    /// Collects all referenced signal identifiers.
    fn get_signal_identifiers(&mut self) -> HashSet<&'static str>;
}

/// Convenience trait alias combining execution and signal-introspection behavior.
pub trait StlOperatorAndSignalIdentifier<T: Clone, Y>:
    StlOperatorTrait<T, Output = Y> + SignalIdentifier
{
}

impl<C, Y, U> StlOperatorAndSignalIdentifier<C, Y> for U
where
    C: Clone,
    U: StlOperatorTrait<C, Output = Y> + SignalIdentifier,
{
}

clone_trait_object!(<T: Clone, Y> StlOperatorAndSignalIdentifier<T, Y>);

/// Interval-valued robustness, represented as `(lower, upper)` bounds.
///
/// The bounds satisfy the intuitive interpretation that the true robustness lies
/// somewhere in $[lower, upper]$. In this crate, this type is primarily used for
/// RoSI intermediate reasoning.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct RobustnessInterval(pub f64, pub f64);

impl Add<Self> for RobustnessInterval {
    type Output = RobustnessInterval;

    fn add(self, other: Self) -> RobustnessInterval {
        RobustnessInterval(self.0 + other.0, self.1 + other.1)
    }
}

impl Add<f64> for RobustnessInterval {
    type Output = RobustnessInterval;

    fn add(self, other: f64) -> RobustnessInterval {
        RobustnessInterval(self.0 + other, self.1 + other)
    }
}

impl Add<RobustnessInterval> for f64 {
    type Output = RobustnessInterval;

    fn add(self, other: RobustnessInterval) -> RobustnessInterval {
        other + self
    }
}

impl Sub<Self> for RobustnessInterval {
    type Output = RobustnessInterval;

    fn sub(self, other: Self) -> RobustnessInterval {
        RobustnessInterval(self.0 - other.1, self.1 - other.0)
    }
}

impl Sub<f64> for RobustnessInterval {
    type Output = RobustnessInterval;

    fn sub(self, other: f64) -> RobustnessInterval {
        RobustnessInterval(self.0 - other, self.1 - other)
    }
}

impl Sub<RobustnessInterval> for f64 {
    type Output = RobustnessInterval;

    fn sub(self, other: RobustnessInterval) -> RobustnessInterval {
        RobustnessInterval(self - other.1, self - other.0)
    }
}

impl Neg for RobustnessInterval {
    type Output = RobustnessInterval;

    fn neg(self) -> RobustnessInterval {
        RobustnessInterval(-self.1, -self.0)
    }
}

/// Pointwise minimum operation for robustness domains.
pub trait Min {
    /// Returns the minimum combination of `self` and `other`.
    fn min(self, other: Self) -> Self;
}

/// Pointwise maximum operation for robustness domains.
pub trait Max {
    /// Returns the maximum combination of `self` and `other`.
    fn max(self, other: Self) -> Self;
}

/// Set-like intersection operation for interval-like domains.
pub trait Intersection {
    /// Returns the intersection of `self` and `other`.
    fn intersection(self, other: Self) -> Self;
}

impl Min for RobustnessInterval {
    fn min(self, other: Self) -> Self {
        RobustnessInterval(self.0.min(other.0), self.1.min(other.1))
    }
}
impl Max for RobustnessInterval {
    fn max(self, other: Self) -> Self {
        RobustnessInterval(self.0.max(other.0), self.1.max(other.1))
    }
}

impl Intersection for RobustnessInterval {
    /// Calculates the intersection of two intervals.
    ///
    /// If there is no overlap, this implementation returns
    /// `(-∞, +∞)` as an "unknown" sentinel used in the current design.
    fn intersection(self, other: Self) -> Self {
        let new_start = self.0.max(other.0);
        let new_end = self.1.min(other.1);
        if new_end < new_start {
            // The intervals do not overlap.
            RobustnessInterval(f64::NEG_INFINITY, f64::INFINITY)
        } else {
            // The intervals overlap. Return the new intersected interval.
            RobustnessInterval(new_start, new_end)
        }
    }
}

/// Semantic operations required by STL operators for a robustness domain.
///
/// This trait abstracts the algebra used by boolean, quantitative (`f64`), and
/// interval-valued (`RobustnessInterval`) monitoring.
pub trait RobustnessSemantics: Clone + PartialEq {
    /// Conjunction semantics.
    fn and(l: Self, r: Self) -> Self;
    /// Disjunction semantics.
    fn or(l: Self, r: Self) -> Self;
    /// Negation semantics.
    fn not(val: Self) -> Self;
    /// Implication semantics.
    fn implies(antecedent: Self, consequent: Self) -> Self;

    /// Identity element for eventuality aggregation.
    fn eventually_identity() -> Self;
    /// Identity element for globally aggregation.
    fn globally_identity() -> Self;

    /// Value representing a tautologically true atomic predicate.
    fn atomic_true() -> Self;
    /// Value representing a tautologically false atomic predicate.
    fn atomic_false() -> Self;

    /// Evaluates atomic predicate `value > c` in this semantics.
    fn atomic_greater_than(value: f64, c: f64) -> Self;
    /// Evaluates atomic predicate `value < c` in this semantics.
    fn atomic_less_than(value: f64, c: f64) -> Self;

    /// Representation of an unresolved/unknown value.
    ///
    /// Used by incremental temporal operators where windows are not yet closed.
    fn unknown() -> Self;

    /// Returns true if 'old' is strictly dominated by 'new' such that 'old'
    /// can be safely discarded from a Lemire sliding window.
    ///
    /// For RoSI, this requires strict separation of intervals.
    ///
    /// # Arguments
    /// * `old` - The value to check for domination
    /// * `new` - The potentially dominating value
    /// * `is_max` - If true, checks domination for max operations; if false, for min operations
    fn prune_dominated(old: Self, new: Self, is_max: bool) -> bool;
}
impl RobustnessSemantics for f64 {
    fn and(l: f64, r: f64) -> f64 {
        l.min(r)
    }
    fn or(l: f64, r: f64) -> f64 {
        l.max(r)
    }
    fn not(val: f64) -> f64 {
        -val
    }
    fn implies(antecedent: f64, consequent: f64) -> f64 {
        (-antecedent).max(consequent)
    }
    fn eventually_identity() -> Self {
        f64::NEG_INFINITY
    }
    fn globally_identity() -> Self {
        f64::INFINITY
    }
    fn atomic_true() -> Self {
        f64::INFINITY
    }
    fn atomic_false() -> Self {
        f64::NEG_INFINITY
    }
    fn atomic_greater_than(value: f64, c: f64) -> Self {
        value - c
    }
    fn atomic_less_than(value: f64, c: f64) -> Self {
        c - value
    }
    fn unknown() -> Self {
        f64::NAN
    }
    fn prune_dominated(old: Self, new: Self, is_max: bool) -> bool {
        if is_max { old <= new } else { old >= new }
    }
}

impl RobustnessSemantics for bool {
    fn and(l: bool, r: bool) -> bool {
        l && r
    }
    fn or(l: bool, r: bool) -> bool {
        l || r
    }
    fn not(val: bool) -> bool {
        !val
    }
    fn implies(antecedent: bool, consequent: bool) -> bool {
        !antecedent || consequent
    }
    fn eventually_identity() -> Self {
        false
    }
    fn globally_identity() -> Self {
        true
    }
    fn atomic_true() -> Self {
        true
    }
    fn atomic_false() -> Self {
        false
    }
    fn atomic_greater_than(value: f64, c: f64) -> Self {
        value > c
    }
    fn atomic_less_than(value: f64, c: f64) -> Self {
        value < c
    }
    fn unknown() -> Self {
        // In the boolean case, we can represent "unknown" as false
        false
    }
    fn prune_dominated(old: Self, new: Self, is_max: bool) -> bool {
        // false <= true
        if is_max {
            !old || new // old <= new
        } else {
            old || !new // old >= new
        }
    }
}

impl RobustnessSemantics for RobustnessInterval {
    fn and(l: Self, r: Self) -> Self {
        // conjunction => pointwise min over interval bounds
        l.min(r)
    }

    fn or(l: Self, r: Self) -> Self {
        // disjunction => pointwise max over interval bounds
        l.max(r)
    }

    fn not(val: Self) -> Self {
        // negation => interval negation (flip and negate bounds)
        -val
    }

    fn implies(antecedent: Self, consequent: Self) -> Self {
        // implication: max(-antecedent, consequent)
        // (-antecedent).max(consequent)
        Self::or(-antecedent, consequent)
    }

    fn eventually_identity() -> Self {
        // identity for sup / eventuality is negative infinity
        RobustnessInterval(f64::NEG_INFINITY, f64::NEG_INFINITY)
    }

    fn globally_identity() -> Self {
        // identity for inf / globally is positive infinity
        RobustnessInterval(f64::INFINITY, f64::INFINITY)
    }

    fn atomic_true() -> Self {
        // atomic true represents the maximal robustness
        RobustnessInterval(f64::INFINITY, f64::INFINITY)
    }

    fn atomic_false() -> Self {
        // atomic false represents the minimal robustness
        RobustnessInterval(f64::NEG_INFINITY, f64::NEG_INFINITY)
    }

    fn atomic_greater_than(value: f64, c: f64) -> Self {
        RobustnessInterval(value - c, value - c)
    }

    fn atomic_less_than(value: f64, c: f64) -> Self {
        RobustnessInterval(c - value, c - value)
    }

    fn unknown() -> Self {
        RobustnessInterval(f64::NEG_INFINITY, f64::INFINITY)
    }
    fn prune_dominated(old: Self, new: Self, is_max: bool) -> bool {
        // example: F[a,b] x>0
        // x0 = -2, x1 = 2
        // old = (-2, -2), new = (2, 2), is_max = true
        // returns true: old can be discarded since it can never exceed new and we want the best only
        // example: G[a,b] x>0
        // x0 = 2, x1 = -2
        // old = (2, 2), new = (-2, -2), is_max = false
        // returns true: old can be discarded since it can never be smaller than new and we want the worst only

        if is_max {
            // Max/Eventually: Discard old if it can never exceed new.
            // We need new's lower bound to be >= old's upper bound.
            old.1 <= new.0
        } else {
            // Min/Globally: Discard old if it can never be smaller than new.
            // We need new's upper bound to be <= old's lower bound.
            old.0 >= new.1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ri_add_sub_neg() {
        let a = RobustnessInterval(1.0, 2.0);
        let b = RobustnessInterval(0.5, 1.5);

        assert_eq!(a + b, RobustnessInterval(1.5, 3.5));
        assert_eq!(a + 1.0f64, RobustnessInterval(2.0, 3.0));
        assert_eq!(1.0f64 + a, RobustnessInterval(2.0, 3.0));

        let sub = a - b;
        // subtraction defined as (a.lo - b.hi, a.hi - b.lo)
        assert_eq!(sub, RobustnessInterval(1.0 - 1.5, 2.0 - 0.5));

        let subf = a - 1.0f64;
        assert_eq!(subf, RobustnessInterval(0.0, 1.0));

        let neg = -a;
        assert_eq!(neg, RobustnessInterval(-2.0, -1.0));

        // f64 - RobustnessInterval is implemented as (self - other.hi, self - other.lo)
        let left_f64 = 5.0f64;
        let res = left_f64 - a;
        assert_eq!(res, RobustnessInterval(5.0 - 2.0, 5.0 - 1.0));
    }

    #[test]
    fn ri_min_max_intersection() {
        let a = RobustnessInterval(1.0, 4.0);
        let b = RobustnessInterval(2.0, 3.0);

        assert_eq!(a.min(b), RobustnessInterval(1.0, 3.0));
        assert_eq!(a.max(b), RobustnessInterval(2.0, 4.0));

        let inter = a.intersection(b);
        assert_eq!(inter, RobustnessInterval(2.0, 3.0));

        // non-overlapping intervals -> returns (NEG_INFINITY, INFINITY) per implementation
        let c = RobustnessInterval(1.0, 2.0);
        let d = RobustnessInterval(3.0, 4.0);
        let non = c.intersection(d);
        assert!(non.0.is_infinite() && non.0.is_sign_negative());
        assert!(non.1.is_infinite() && non.1.is_sign_positive());
    }

    #[test]
    fn f64_semantics_basic() {
        let a = 1.5f64;
        let b = 2.0f64;

        assert_eq!(<f64 as RobustnessSemantics>::and(a, b), a.min(b));
        assert_eq!(<f64 as RobustnessSemantics>::or(a, b), a.max(b));
        assert_eq!(<f64 as RobustnessSemantics>::not(a), -a);

        // implication: max(-antecedent, consequent)
        assert_eq!(<f64 as RobustnessSemantics>::implies(a, b), (-a).max(b));

        assert!(<f64 as RobustnessSemantics>::eventually_identity().is_infinite());
        assert!(<f64 as RobustnessSemantics>::globally_identity().is_infinite());

        assert_eq!(<f64 as RobustnessSemantics>::atomic_true(), f64::INFINITY);
        assert_eq!(
            <f64 as RobustnessSemantics>::atomic_false(),
            f64::NEG_INFINITY
        );

        assert_eq!(
            <f64 as RobustnessSemantics>::atomic_greater_than(5.0, 3.0),
            2.0
        );
        assert_eq!(
            <f64 as RobustnessSemantics>::atomic_less_than(2.0, 5.0),
            3.0
        );

        let unk = <f64 as RobustnessSemantics>::unknown();
        assert!(unk.is_nan());
    }

    #[test]
    fn bool_semantics_basic() {
        assert!(!<bool as RobustnessSemantics>::and(true, false));
        assert!(<bool as RobustnessSemantics>::or(true, false));
        assert!(!<bool as RobustnessSemantics>::not(true));
        assert!(!<bool as RobustnessSemantics>::implies(true, false));

        assert!(!<bool as RobustnessSemantics>::eventually_identity());
        assert!(<bool as RobustnessSemantics>::globally_identity());

        assert!(<bool as RobustnessSemantics>::atomic_true());
        assert!(!<bool as RobustnessSemantics>::atomic_false());

        assert!(<bool as RobustnessSemantics>::atomic_greater_than(5.0, 3.0));
        assert!(!<bool as RobustnessSemantics>::atomic_less_than(2.0, 1.0));
    }

    #[test]
    fn interval_semantics_basic() {
        let a = RobustnessInterval(1.0, 2.0);
        let b = RobustnessInterval(0.5, 3.0);

        let is_dom_true = RobustnessInterval::prune_dominated(a, b, false);
        println!("is_dom_true: {}", is_dom_true);
        // and = min
        assert_eq!(
            <RobustnessInterval as RobustnessSemantics>::and(a, b),
            a.min(b)
        );
        // or = max
        assert_eq!(
            <RobustnessInterval as RobustnessSemantics>::or(a, b),
            a.max(b)
        );
        // not = negation
        assert_eq!(<RobustnessInterval as RobustnessSemantics>::not(a), -a);

        // implies = or(-antecedent, consequent)
        let imp = <RobustnessInterval as RobustnessSemantics>::implies(a, b);
        assert_eq!(imp, <RobustnessInterval as RobustnessSemantics>::or(-a, b));

        assert_eq!(
            <RobustnessInterval as RobustnessSemantics>::eventually_identity(),
            RobustnessInterval(f64::NEG_INFINITY, f64::NEG_INFINITY)
        );
        assert_eq!(
            <RobustnessInterval as RobustnessSemantics>::globally_identity(),
            RobustnessInterval(f64::INFINITY, f64::INFINITY)
        );

        assert_eq!(
            <RobustnessInterval as RobustnessSemantics>::atomic_true(),
            RobustnessInterval(f64::INFINITY, f64::INFINITY)
        );
        assert_eq!(
            <RobustnessInterval as RobustnessSemantics>::atomic_false(),
            RobustnessInterval(f64::NEG_INFINITY, f64::NEG_INFINITY)
        );

        assert_eq!(
            <RobustnessInterval as RobustnessSemantics>::atomic_greater_than(5.0, 3.0),
            RobustnessInterval(2.0, 2.0)
        );
        assert_eq!(
            <RobustnessInterval as RobustnessSemantics>::atomic_less_than(2.0, 5.0),
            RobustnessInterval(3.0, 3.0)
        );

        let unk = <RobustnessInterval as RobustnessSemantics>::unknown();
        assert_eq!(unk, RobustnessInterval(f64::NEG_INFINITY, f64::INFINITY));
    }

    #[test]
    fn time_interval_basic() {
        let ti = TimeInterval {
            start: Duration::from_secs(1),
            end: Duration::from_secs(5),
        };
        assert_eq!(ti.start, Duration::from_secs(1));
        assert_eq!(ti.end, Duration::from_secs(5));
    }
}

// -----------------------------------------------------------------------------
// Variables context for runtime variable bindings
// -----------------------------------------------------------------------------

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A container for runtime variable bindings.
///
/// Variables can be declared and updated at runtime, and their values
/// are used when evaluating formulas containing variable references.
///
/// # Example
///
/// ```
/// use mstlo::Variables;
///
/// let vars = Variables::new();
/// vars.set("threshold", 5.0);
/// assert_eq!(vars.get("threshold"), Some(5.0));
///
/// // Update the variable
/// vars.set("threshold", 10.0);
/// assert_eq!(vars.get("threshold"), Some(10.0));
/// ```
#[derive(Debug, Clone, Default)]
pub struct Variables {
    inner: Rc<RefCell<HashMap<&'static str, f64>>>,
}

impl Variables {
    /// Creates a new empty variable context.
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// Set (or update) a variable's value.
    ///
    /// # Arguments
    /// * `name` - The variable name (must be a static string)
    /// * `value` - The variable's value
    pub fn set(&self, name: &'static str, value: f64) {
        self.inner.borrow_mut().insert(name, value);
    }

    /// Get a variable's current value.
    ///
    /// Returns `None` if the variable has not been set.
    pub fn get(&self, name: &'static str) -> Option<f64> {
        self.inner.borrow().get(name).copied()
    }

    /// Returns whether a variable is currently defined.
    pub fn contains(&self, name: &'static str) -> bool {
        self.inner.borrow().contains_key(name)
    }

    /// Returns all currently defined variable names.
    ///
    /// The returned order is not specified.
    pub fn names(&self) -> Vec<&'static str> {
        self.inner.borrow().keys().copied().collect()
    }

    /// Removes a variable and returns its previous value, if any.
    pub fn remove(&self, name: &'static str) -> Option<f64> {
        self.inner.borrow_mut().remove(name)
    }

    /// Clears all stored variables.
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Returns `true` if no variables are stored.
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// Returns the number of stored variables.
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    /// Returns a snapshot of all `(name, value)` pairs.
    ///
    /// This method clones entries into a `Vec` for ergonomic iteration.
    pub fn iter(&self) -> Vec<(&'static str, f64)> {
        self.inner.borrow().iter().map(|(k, v)| (*k, *v)).collect()
    }
}

#[cfg(test)]
mod variables_tests {
    use super::*;

    #[test]
    fn test_variables_basic() {
        let vars = Variables::new();
        assert_eq!(vars.get("x"), None);

        vars.set("x", 5.0);
        assert_eq!(vars.get("x"), Some(5.0));

        vars.set("x", 10.0);
        assert_eq!(vars.get("x"), Some(10.0));
    }

    #[test]
    fn test_variables_multiple() {
        let vars = Variables::new();
        vars.set("a", 1.0);
        vars.set("b", 2.0);
        vars.set("c", 3.0);

        assert_eq!(vars.get("a"), Some(1.0));
        assert_eq!(vars.get("b"), Some(2.0));
        assert_eq!(vars.get("c"), Some(3.0));
        assert_eq!(vars.get("d"), None);
    }

    #[test]
    fn test_variables_clone() {
        let vars1 = Variables::new();
        vars1.set("x", 5.0);

        let vars2 = vars1.clone();
        assert_eq!(vars2.get("x"), Some(5.0));

        // Changes through one clone are visible through the other
        vars1.set("x", 10.0);
        assert_eq!(vars2.get("x"), Some(10.0));
    }

    #[test]
    fn test_variables_remove() {
        let vars = Variables::new();
        vars.set("x", 5.0);
        assert!(vars.contains("x"));

        let removed = vars.remove("x");
        assert_eq!(removed, Some(5.0));
        assert!(!vars.contains("x"));
    }

    #[test]
    fn test_variables_clear() {
        let vars = Variables::new();
        vars.set("a", 1.0);
        vars.set("b", 2.0);
        assert!(!vars.is_empty());

        vars.clear();
        assert!(vars.is_empty());
    }

    #[test]
    fn test_variables_len() {
        let vars = Variables::new();
        assert_eq!(vars.len(), 0);

        vars.set("a", 1.0);
        vars.set("b", 2.0);
        assert_eq!(vars.len(), 2);
    }

    #[test]
    fn test_variables_names_iter() {
        let vars = Variables::new();
        vars.set("a", 1.0);
        vars.set("b", 2.0);

        let names = vars.names();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));

        let iter = vars.iter();
        assert!(iter.contains(&("a", 1.0)));
        assert!(iter.contains(&("b", 2.0)));
    }
}
