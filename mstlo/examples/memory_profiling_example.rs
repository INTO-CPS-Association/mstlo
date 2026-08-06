//! Memory profiling example for the STL monitor.
//!
//! 1. Long trace with a short window → memory plateaus early, never
//!    grows with trace length.
//! 2. Bad-then-good signal for Lemire → memory rises during the bad
//!    phase (increasing signal under G/min), then drops during the
//!    good phase (decreasing signal collapses the cache).

use mstlo::{step, stl, DelayedQuantitative, StlMonitor};
use std::time::Duration;

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn separator(title: &str) {
    println!();
    println!("═══ {title} ═══");
}

// ──────────────────────────────────────────────────────────────────────
// Part 1 — memory bounded by window, not trace length
// ──────────────────────────────────────────────────────────────────────

fn part1_steady_state() {
    separator("Part 1: memory bounded by window, not trace length");
    println!("Formula: G[0, 1000ms] (x > 0)");
    println!("100 ms steps → at most ~10 samples in the 1s window\n");

    let mut monitor = StlMonitor::builder()
        .formula(stl!(G[0, 1.0] (x > 0.0)))
        .semantics(DelayedQuantitative)
        .build()
        .expect("build");

    println!(
        "  initial (empty)                 total_size={} bytes",
        monitor.total_size()
    );

    for i in 0..200u64 {
        let t = i * 100;
        monitor.update(&step!("x", 5.0, ms(t)));

        if i == 0 || i == 5 || i == 10 || i == 11 || i == 50 || i == 100 || i == 199 {
            println!(
                "  step {i:>3}  t={t:>5}ms  total_size={:>6} bytes",
                monitor.total_size()
            );
        }
    }

    println!();
    println!(
        "  → After 200 steps memory = {} bytes — same as at step 11.",
        monitor.total_size()
    );
    println!("    Window holds ~10 samples regardless of trace length.");
}

// ──────────────────────────────────────────────────────────────────────
// Part 2 — Lemire: bad signal then good signal
// ──────────────────────────────────────────────────────────────────────
//
// G[0, 2000ms] uses min-semantics.  pop_dominated_values prunes when
// `old >= new`.  An increasing signal keeps all entries; a decreasing
// signal prunes down to one.
//
// RingBuffer now calls shrink_to_fit() during prune() when capacity
// exceeds 2× length, so memory actually drops during the good phase.

fn part2_lemire_transition() {
    separator("Part 2: Lemire — increasing (bad) then decreasing (good)");
    println!("Formula: G[0, 2000ms] (x > 0)");
    println!("100 ms steps — window holds up to ~20 samples\n");
    println!("  Phase A (0..4s):  strictly increasing → cache fills up");
    println!("  Phase B (4..7s):  strictly decreasing → cache collapses\n");

    let mut monitor = StlMonitor::builder()
        .formula(stl!(G[0, 2.0] (x > 0.0)))
        .semantics(DelayedQuantitative)
        .build()
        .expect("build");

    println!(
        "  initial (empty)                 total_size={} bytes",
        monitor.total_size()
    );

    // Phase A: strictly increasing 1.0, 2.0, ..., 40.0   (t=0..3.9s)
    println!("\n  ── Phase A: increasing (bad for G/min) ──");
    for i in 1..=40u64 {
        let t = (i - 1) * 100;
        monitor.update(&step!("x", i as f64, ms(t)));
        if i <= 5 || i % 10 == 0 {
            println!(
                "    step {i:>3}  t={t:>5}ms  val={:>6.1}  total_size={:>6} bytes",
                i as f64,
                monitor.total_size()
            );
        }
    }

    // Phase B: strictly decreasing 40.0, 39.0, ..., 1.0   (t=4.0..6.9s)
    println!("\n  ── Phase B: decreasing (good for G/min) ──");
    for i in 1..=30u64 {
        let value = (41 - i) as f64;
        let t = 4000 + (i - 1) * 100;
        monitor.update(&step!("x", value, ms(t)));
        if i <= 5 || i % 10 == 0 {
            println!(
                "    step {i:>3}  t={t:>5}ms  val={value:>6.1}  total_size={:>6} bytes",
                monitor.total_size()
            );
        }
    }

    println!("\n  → Phase A (increasing):  cache holds ~20 entries → memory rises.");
    println!("    Phase B (decreasing): each new value prunes the old →");
    println!("    cache collapses to 1, capacity freed by shrink_to_fit().");
}

// ──────────────────────────────────────────────────────────────────────

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║    STL Monitor — Memory Profiling                           ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("DelayedQuantitative (f64) semantics, signal \"x\", 100 ms steps.");
    println!();

    part1_steady_state();
    part2_lemire_transition();

    println!();
    println!("═══ Summary ═══");
    println!("  Memory depends on samples within each temporal window,");
    println!("  not on total trace length.  Lemire optimisation can");
    println!("  shrink the cache from O(window_samples) to O(1) when");
    println!("  the signal cooperates.");
}
