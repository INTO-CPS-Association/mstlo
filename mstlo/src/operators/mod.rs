//! Executable STL operator implementations.
//!
//! This module groups the concrete runtime operators used by the monitor:
//! - [`atomic_operators`] for predicate leaves,
//! - [`binary_operators`] for logical conjunction/disjunction,
//! - [`not_operator`] for logical negation,
//! - [`unary_temporal_operators`] for `Eventually`/`Globally`, and
//! - [`until_operator`] for temporal `Until`.

pub mod atomic_operators;
pub mod binary_operators;
pub mod not_operator;
pub mod unary_temporal_operators;
pub mod until_operator;
