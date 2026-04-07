//! Signal Temporal Logic (STL) public module.
//!
//! This namespace contains the main building blocks for parsing and monitoring:
//! - [`core`] shared traits and semantics,
//! - [`formula_definition`] AST definitions,
//! - [`formulas`] helper constructors/macros-facing utilities,
//! - [`monitor`] high-level streaming monitor API,
//! - [`naive_operators`] reference/naive evaluation backend,
//! - [`operators`] incremental operator implementations, and
//! - [`parser`] textual STL parser.

pub mod core;
pub mod formula_definition;
pub mod formulas;
pub mod monitor;
pub mod naive_operators;
pub mod operators;
pub mod parser;

/// Parser error type re-exported for convenience.
pub use parser::{ParseError, parse_stl};
