//! # mstlo - Online Signal Temporal Logic
//!
//! `mstlo` provides runtime and macro-based tooling to parse, build, and execute
//! Signal Temporal Logic (STL) monitors over streaming data.
//!
//! It includes:
//! - a high-level monitor builder API,
//! - incremental evaluation backend,
//! - multiple semantics (qualitative, quantitative, RoSI), and
//! - selectable signal interpolation (zero-order hold or linear).
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
//! ## Batch usage
//!
//! Whole traces can be fed at once with [`StlMonitor::update_batch`], which takes
//! any iterable of steps. The [`steps!`] macro builds one, either signal-major
//! (one trace per signal) or flat (interleaved samples):
//!
//! ```
//!    use mstlo::monitor::*;
//!    use mstlo::{steps, stl};
//!
//!    let formula = stl!(G[0, 1](x > 5.0));
//!    let mut monitor = StlMonitor::builder()
//!        .formula(formula)
//!        .semantics(DelayedQuantitative)
//!        .build()
//!        .unwrap();
//!
//!    let out = monitor.update_batch(&steps! {
//!        "x": [(7.0, 0s), (6.0, 1s), (4.0, 2s), (7.0, 3s)],
//!    });
//!
//!    assert_eq!(out.verdicts().len(), 3);
//! ```
//!
//! ## Signal names known only at runtime
//!
//! [`Step`] stores its signal name as a `&'static str`, which keeps the
//! per-step name comparisons cheap. Names read at runtime — from a CSV header,
//! a config file, or a language binding — are not `'static`, so pass them
//! through [`intern`], which allocates each distinct name once and reuses it
//! thereafter:
//!
//! ```
//!    use mstlo::{Step, intern};
//!    use std::time::Duration;
//!
//!    let header = String::from("temperature");
//!    let step = Step::new(intern(&header), 21.4, Duration::from_secs(0));
//!
//!    assert_eq!(step.signal, "temperature");
//! ```
//!
//! See [`interner`] for the memory characteristics.
//!

// Enable use of ::mstlo:: paths within this crate for the proc-macro
extern crate self as mstlo;

mod core;
mod formula_definition;
mod formulas;
pub mod interner;
pub mod monitor;
mod naive_operators;
mod operators;
mod parser;
mod ring_buffer;
mod synchronizer;

pub use core::{
    RobustnessInterval, RobustnessSemantics, SignalIdentifier, TimeInterval, Variables,
};
pub use formula_definition::FormulaDefinition;
pub use formulas::get_formulas;
pub use interner::{intern, interned_count};
pub use monitor::semantic_markers::{
    DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi, SemanticType,
};
pub use monitor::{
    Algorithm, MonitorOutput, Semantics, StlMonitor, StlMonitorBuilder, SyncStepResult,
};
pub use parser::{ParseError, parse_stl};
#[cfg(feature = "track-cache-size")]
pub use ring_buffer::GLOBAL_CACHE_SIZE;
pub use ring_buffer::{RingBuffer, RingBufferTrait, Step};
#[allow(deprecated)]
pub use synchronizer::SynchronizationStrategy;
pub use synchronizer::{Interpolatable, SignalInterpolation, Synchronizer};

pub use mstlo_macros::{step, steps, stl};
