//! # mstlo - Online Signal Temporal Logic
//!
//! `mstlo` provides runtime and macro-based tooling to parse, build, and execute
//! Signal Temporal Logic (STL) monitors over streaming data.
//!
//! It includes:
//! - a high-level monitor builder API,
//! - incremental evaluation backend,
//! - multiple semantics (qualitative, quantitative, RoSI), and
//! - optional multi-signal synchronization/interpolation.
//!
//! ## Simple usage
//!
//! ```
//!    use mstlo::monitor::*;
//!    use mstlo::{step, stl};
//! 
//!    // Build a monitor from the macro DSL.
//!    let formula = stl!(G[0, 1](x > 5.0));
//!    let mut monitor = StlMonitor::builder()
//!        .formula(formula)
//!        .algorithm(Algorithm::Incremental)
//!        .semantics(DelayedQuantitative)
//!        .build()
//!        .unwrap();
//!
//!    // Stream updates
//!    let out1 = monitor.update(&step!("x", 7.0, 0s));
//!    let out2 = monitor.update(&step!("x", 6.0, 1s));
//!    let out3 = monitor.update(&step!("x", 4.0, 2s));
//!    let out4 = monitor.update(&step!("x", 7.0, 3s));
//!
//!    assert_eq!(out1.verdicts(), vec![]);
//!    assert_eq!(out2.verdicts(), vec![step!("x", 1.0, 0s)]);
//!    assert_eq!(out3.verdicts(), vec![step!("x", -1.0, 1s)]);
//!    assert_eq!(out4.verdicts(), vec![step!("x", -1.0, 2s)]);
//! ```
//!

// Enable use of ::mstlo:: paths within this crate for the proc-macro
extern crate self as mstlo;

pub mod core;
pub mod formula_definition;
pub mod formulas;
pub mod monitor;
pub mod naive_operators;
pub mod operators;
pub mod parser;
pub mod ring_buffer;
pub mod synchronizer;

pub use parser::{ParseError, parse_stl};

// Re-export the stl macro at crate root for convenience
pub use mstlo_macros::{stl, step};
