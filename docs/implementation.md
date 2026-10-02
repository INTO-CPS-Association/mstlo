# Implementation

This page describes how mstlo evaluates formulas, how it interprets signals between samples, and when it emits verdicts. For the logic and the semantics themselves, see [Signal Temporal Logic and Evaluation Semantics](signal_temporal_logic.md).

<!-- TOC -->

- [Implementation](#implementation)
  - [Algorithm](#algorithm)
  - [Signal Model](#signal-model)
    - [Zero-Order Hold (default)](#zero-order-hold-default)
    - [Linear Interpolation](#linear-interpolation)
  - [Input](#input)
    - [Batch Updates](#batch-updates)
    - [Multiple Signals](#multiple-signals)
    - [Initialization of Signals](#initialization-of-signals)
    - [Refinable Verdicts](#refinable-verdicts)
  - [Settings that can hurt performance](#settings-that-can-hurt-performance)
  - [References](#references)

<!-- /TOC -->

## Algorithm

To compute the [evaluation semantics](signal_temporal_logic.md#evaluation-semantics) efficiently, mstlo uses a bottom-up dynamic programming approach, where each operator caches the verdicts of its suboperators, only maintaining the required state. For sliding window operations (like _eventually_ and _globally_), the library incorporates Lemire's algorithm [1] to reduce cache footprints and computation time. The amount of reduction depends both on chosen semantics and signal characteristics.

## Signal Model

We assume a dense signal model: signals are real-valued and observed at real-valued timestamps that can arrive at any point in time, i.e. no sampling rate needs to be specified, samples need not be equally spaced, and different signals need not share timestamps.

As is often assumed for online monitors, we assume that signals have finite variability, meaning that for each signal $x^j$ observed as a sequence of time-value pairs $(t_0,x^j_0), \dots, (t_n,x^j_n)$ there exists some $\Delta^j > 0$ such that $t_{i+1}-t_i > \Delta^j$ for all $i \geq 0$. We do, however, not impose any restrictions on $\Delta^j$.

Since signals are defined densely but observed at discrete timestamps, the monitor must decide what a signal is between two samples. This is configured once for the whole monitor, and applies to every signal in the formula, whether there is one or several:

```rust
let monitor = StlMonitor::builder()
    .formula(stl! {G[0, 2](x > 4.0)})
    .semantics(DelayedQualitative)
    .signal_interpolation(SignalInterpolation::ZeroOrderHold) // the default
    .build()?;
```

No samples are synthesized between a signal's own samples; the interpolation is applied internally at the operator level. The one sample that is synthesized is each signal's value at $t=0$, see [Initialization of Signals](#initialization-of-signals).

### Zero-Order Hold (default)

With `SignalInterpolation::ZeroOrderHold`, signals are piecewise constant: a sample holds until the next sample of the same signal. That is, for all $t\in [t_i, t_{i+1})$ we have $x^j(t)=x^j(t_i)$. This results in a staircase approximation between samples.

Two consequences follow:

- The value of a signal after its newest sample is not known until the next sample arrives, since that sample decides where the step ends. A delayed verdict at $t$ therefore waits for a sample at or after $t$; the eager semantics answer earlier where the prefix already decides the formula, and RoSI answers immediately with an interval it later refines.
- The verdicts are themselves a piecewise-constant signal: sorted by timestamp, a verdict at $t$ holds until the next verdict.

If the verdict of a specification changes between samples, the output contains verdicts for timestamps that were never input to the monitor. Each temporal operator shifts timestamps by its interval bounds to find where its verdict can change. We illustrate this with the following example:

Let the signal be given by $x=[1,1,7,1,1]$ for timestamps $t=[0,1,2,3,4]$ and the formula by `F[0,1](G[0.4,1](x > 3))`, under delayed qualitative semantics. Since $x>3$ only on $[2,3)$, the inner `G[0.4,1]` holds for $t\in[1.6,2)$. The property is therefore satisfied at $t=0.6$, since $1.6 \in [0.6+0,0.6+1]$ and $x>3$ on all of $[1.6+0.4,1.6+1.0]$. The monitor outputs:

| Input      | Verdicts emitted        |
| ---------- | ----------------------- |
| $x=1$ at 0 | –                       |
| $x=1$ at 1 | –                       |
| $x=7$ at 2 | 0: `false`              |
| $x=1$ at 3 | 0.6: `true`, 1: `true`  |
| $x=1$ at 4 | 1.6: `true`, 2: `false` |

Read as a signal, the property is violated on $[0,0.6)$, satisfied on $[0.6,2)$ and violated from $2$. Note that $0.6 = 2 - 1.4$ is the input timestamp shifted by both formulas' bounds.

### Linear Interpolation

With `SignalInterpolation::Linear`, signals run in a straight line between consecutive samples: for $t\in[t_i,t_{i+1}]$, $x^j(t)=x^j_i + (x^j_{i+1}-x^j_i)\frac{t-t_i}{t_{i+1}-t_i}$.

Since predicates compare a signal against a constant, whether a predicate holds is still piecewise constant; it can only change where the line crosses the threshold. When a new sample arrives, the predicate computes the exact crossing time (rounded to nanoseconds) and emits it as an extra breakpoint. Every operator above the predicates is unchanged. This is illustrated with the following example:

For $x=6$ at $0$ and $x=2$ at $4$, $x$ crosses $4$ at $t=2$:

| Formula         | Zero-order hold       | Linear                            |
| --------------- | --------------------- | --------------------------------- |
| `x > 4`         | 0: `true`, 4: `false` | 0: `true`, 2: `false`, 4: `false` |
| `G[0,2](x > 4)` | 0: `true`, 2: `false` | 0: `false`, 2: `false`            |

Under linear interpolation, `G[0,2](x > 4)` is violated at $0$ because $x=4$ at the end of the closed window $[0,2]$.

Predicates are strict, so a predicate fails exactly on its threshold. A predicate that becomes false at a crossing does so at $t_c$ itself. One that becomes true, as `x > 4` does on a rising line, only holds from just after $t_c$, so its verdict is emitted at $t_c + 1\,\text{ns}$. The same holds for a sample exactly on the threshold: its own verdict is emitted at once, and a verdict that differs for the segment after it is emitted 1 ns later. This way `x == 5` (and `x >= 5`, `x <= 5`) are also satisfied at the single instant where the line meets 5: for $x=4$ at $0$ and $x=6$ at $2$, `x == 5` is `true` at $1$ and `false` from $1\,\text{s} + 1\,\text{ns}$.

Note that linear interpolation is only supported for the qualitative semantics (delayed and eager) with the incremental algorithm. Quantitative semantics (i.e. robustness values) are piecewise linear between samples, which is not implemented, so `build()` returns an error for delayed quantitative and RoSI semantics.

> The deprecated `synchronization_strategy(..)` maps `None` and `ZeroOrderHold` to `SignalInterpolation::ZeroOrderHold`, and `Linear` to `SignalInterpolation::Linear`.

## Input

Each sample is a `Step`: a signal name, a value, and a timestamp. In Rust the timestamp is a `Duration` (nanosecond resolution), e.g. `step!("x", 7.0, 1500ms)`; in Python it is in seconds, e.g. `monitor.update("x", 7.0, 1.5)`. Samples are given one at a time with `update`, or as a batch with `update_batch`, which sorts them by timestamp first.

- Timestamps must be strictly increasing per signal. A sample at or before the previous sample of the same signal is ignored, and a warning is printed.
- Samples of different signals may arrive interleaved in any pattern, and may share timestamps. Feed them in timestamp order across all signals to avoid potential excessive buffering.
- Each update returns a `MonitorOutput`. `verdicts()` gives the latest verdict per timestamp, while `raw_outputs()` also includes the intermediate refinements RoSI emits for the same timestamp.

### Batch Updates

Whole traces can be fed at once with `update_batch`, which accepts any iterable of steps. The `steps!` macro builds one, either signal-major (one trace per signal) or flat (interleaved samples):

```rust
use mstlo::monitor::{Rosi, StlMonitor};
use mstlo::{steps, stl};

let mut monitor = StlMonitor::builder()
    .formula(stl! {G[0, 2](x > 5.0) && (y < 10.0)})
    .semantics(Rosi)
    .initialize_signals_to_zero()
    .build()
    .expect("Failed to build monitor");

// Signal-major: the signal name is written once per trace.
let out = monitor.update_batch(&steps! {
    "x": [(7.0, 0s), (4.0, 1s), (6.0, 2s)],
    "y": [(3.0, 0s), (12.0, 2s)],
});
println!("{}", out); // all finalized verdicts

// Flat: each entry is exactly a `step!` argument list.
let out = monitor.update_batch(&steps![("x", 8.0, 3s), ("y", 1.0, 3s)]);
```

Steps are sorted by timestamp before evaluation, so a batch may be given in any order; the sort is stable, so the result is fully determined by the input sequence. In this example, signals are initialized to zero with `initialize_signals_to_zero()` to ensure all signals are defined at $t=0$s. The actual updates at this timepoint override the initialization values. It is also possible to manually initialize with values with `.initialize_signals([("x", 10.0), ("y", -120.0)])` instead, see [Initialization of Signals](#initialization-of-signals).

### Multiple Signals

When several signals are monitored, the formula's verdict can change at any sample of any of them, so every timestamp of every signal is a breakpoint. Under the delayed semantics a verdict is only given at $t$ once every signal the formula depends on is known at $t$. Signals are not resampled onto a common clock.

Let the formula be `(a > 0) && (b > 0)`, with $a=[1,-1,1]$ observed at $t_a=[0,2,4]$ and $b=[1,1]$ at $t_b=[0,5]$. The monitor outputs:

| Input       | Delayed               | Eager      |
| ----------- | --------------------- | ---------- |
| $a=1$ at 0  | –                     | –          |
| $b=1$ at 0  | 0: `true`             | 0: `true`  |
| $a=-1$ at 2 | –                     | 2: `false` |
| $a=1$ at 4  | –                     | –          |
| $b=1$ at 5  | 2: `false`, 4: `true` | 4: `true`  |

Under the delayed semantics the verdict at $0$ is given as soon as both signals are known there, but the verdicts at $2$ and $4$ wait for the sample of $b$ at $5$: until it arrives, $b$ could have changed anywhere after $0$. The eager semantics answer $2$ already at $a=-1$, since one false conjunct decides the conjunction on its own; $4$ still waits, since a true conjunct does not. Under both semantics the verdict at $5$ waits for the next sample of $a$.

Since a sample answers the timestamps that it decides rather than the ones that precede them, verdicts under the eager semantics need not be emitted in timestamp order. Let $a=[7,-4]$ be observed at $t_a=[0,2]$ and $b=[7,4]$ at $t_b=[0,1]$, arriving in the order $a(0), b(0), a(2), b(1)$:

| Input       | Verdicts emitted |
| ----------- | ---------------- |
| $a=7$ at 0  | –                |
| $b=7$ at 0  | 0: `true`        |
| $a=-4$ at 2 | 2: `false`       |
| $b=4$ at 1  | 1: `true`        |

The verdict at $1$ is emitted last, after the verdict at $2$, because it is the sample $b(1)$ that decides it.

### Initialization of Signals

A signal is not extrapolated backwards from its first sample, so a formula over several signals would otherwise be read over a prefix where some of them do not exist yet. A monitor over more than one signal is therefore built with a value per signal, which that signal holds from $t=0$ until its own first sample:

```rust
let monitor = StlMonitor::builder()
    .formula(stl! {(x > 0.0) && (y > 0.0)})
    .semantics(EagerQualitative)
    .initialize_signals([("x", 0.0), ("y", 1.0)])
    .build()?;
```

`initialize_signals_to_zero()` fills in every signal that was not given a value, and `initialize_signal(name, value)` sets one. `build()` returns an error if a signal of a formula over several signals is left without a value. A formula over a single signal is defined wherever that signal is and needs no initial value; giving one has no effect. In Python the argument is `init_signals={"x": 0.0}` and anything left out is $0.0$, so a monitor always builds.

A signal that is really sampled at $t=0$ overrides its initial value, provided that sample arrives before the first sample past $t=0$; a trace that starts every signal together is therefore monitored exactly as it is written. For `(x > 0) && (y > 0)` with $x$ at $0, 1, 2, 3$ and $y$ first at $2.5$, the monitor reads $y$ as its initial value on $[0, 2.5)$ and gives verdicts from $0$.

### Refinable Verdicts

The [RoSI](signal_temporal_logic.md#robust-satisfaction-intervals-rosi) semantics are the only ones that answer a timestamp before the trace decides it. A verdict is emitted as soon as the timestamp exists, and re-emitted, narrower, as samples arrive; a bound sits at $\pm\infty$ for as long as the prefix admits a continuation that would move it. `verdicts()` keeps the newest interval per timestamp, `raw_outputs()` every refinement in the order produced.

The two bounds tighten independently. In `G[0,10](x > 0)`, for instance, the first sample of $x$ in the window already caps the infimum, so the upper bound is finite from $t$ while the lower bound waits for $t+10$; `F` is the mirror image. In `phi U[a,b] psi`, a witness for `psi` that has not arrived yet still needs `phi` to hold up to it, so the upper bound follows the running minimum of `phi` and goes negative as soon as `phi` is violated, since future continuations would never satisfy it. 

An interval collapses to a point once every signal is known at $t+h$, where $h$ is the formula's horizon: the interval upper bounds summed along its deepest chain of temporal operators. That point is the delayed robustness value, so the two semantics agree wherever the delayed one answers. For `G[0,1.5]((x > 0) -> F[0.5,2](y > 0))` the horizon is $1.5+2=3.5$, and feeding $x$ at $0,1,2,3.5$ and $y$ at $1,3.5$ gives $(-\infty,\infty)$ at $t=0$ on every update until the samples at $3.5$ arrive, which finalize it at $(1,1)$.

## Settings that can hurt performance

There are certain settings where the combination of formula and signal characteristics can significantly impact performance.

**Signals that are not observed.** If more than one signal is monitored, it is expected that all signals are eventually observed. Since verdicts wait for every signal they depend on, a signal that stops arriving makes pending verdicts pile up, and memory usage grows. For instance, consider the bounded response property `G[0,a]((x > 0) -> F[b,c](y > 0))`. If we observe $x$ and $y$ at $t=0$ and then keep observing only $x$, every new sample of $x$ adds pending state. $y$ can potentially still arrive, so nothing can be released. Once $y$ arrives, all pending verdicts are emitted at once.

**Refinable verdicts over a long trace.** A timestamp answered under the interval semantics stays live until it is final, and is re-emitted on each update that narrows it. The work per update therefore grows with how many timestamps are still open, which is set by the formula's horizon and the sample rate, and the output carries the refinements as well as the verdicts. On a formula with wide intervals over a long trace this is orders of magnitude more expensive than the delayed semantics, which emit each timestamp once.

**Timestamps that do not align with the formula's bounds.** A temporal operator with interval $[a,b]$ evaluates at $t$, $t-a$ and $t-b$ for each breakpoint $t$ of its operand. On a regular grid with bounds that are multiples of the sample period these coincide with existing timestamps. Otherwise each sample adds new evaluation timestamps, and this compounds with nesting (see $0.6$ and $1.6$ in the [zero-order hold example](#zero-order-hold-default)). Linear interpolation adds a breakpoint at each threshold crossing on top of this.

## References

[1] D. Lemire, “Streaming maximum-minimum filter using no more than three comparisons per element,” Nordic J. of Computing, vol. 13, no. 4, pp. 328–339, Dec. 2006.
