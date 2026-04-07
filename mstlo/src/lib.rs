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
//! ```no_run
//! use mstlo::ring_buffer::Step;
//! use mstlo::stl::monitor::{Algorithm, DelayedQuantitative, StlMonitor};
//! use std::time::Duration;
//!
//! // Build a monitor from the macro DSL.
//! let formula = mstlo::stl!(G[0, 2](x > 5.0));
//! let mut monitor = StlMonitor::builder()
//!     .formula(formula)
//!     .algorithm(Algorithm::Incremental)
//!     .semantics(DelayedQuantitative)
//!     .build()
//!     .unwrap();
//!
//! // Stream updates
//! let out1 = monitor.update(&Step::new("x", 7.0, Duration::from_secs(0)));
//! let out2 = monitor.update(&Step::new("x", 6.0, Duration::from_secs(1)));
//! let out3 = monitor.update(&Step::new("x", 4.0, Duration::from_secs(2)));
//!
//!
//! ```
//!

// Enable use of ::mstlo:: paths within this crate for the proc-macro
extern crate self as mstlo;

pub mod ring_buffer;
pub mod stl;
pub mod synchronizer;

// Re-export the stl macro at crate root for convenience
pub use mstlo_macros::stl;
