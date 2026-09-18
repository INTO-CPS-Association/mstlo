# mstlo Python Interface

Python bindings for the Online Signal Temporal Logic (STL) monitoring library.

## Installation

You can install the mstlo Python interface using pip:

```bash
pip install mstlo-python
```

## Quick Start

```python
import mstlo_python as mstlo

# Define an STL formula: Always[0,5](x > 0.5)
formula = mstlo.Formula.always(0, 5, mstlo.Formula.gt("x", 0.5))

# Create a monitor with Rosi semantics
monitor = mstlo.Monitor(formula, semantics="Rosi")

# Feed data and get verdicts
result = monitor.update("x", 1.0, 0.0)
for ts, val in result.verdicts():
    print(f"t={ts}: {val}")

# Or access structured data via dictionary
result_dict = result.to_dict()
for evaluation in result_dict['evaluations']:
    print(evaluation['outputs'])
```

## Features

### Multiple Semantics

The library supports four types of monitoring semantics:

1. **DelayedQualitative** (`semantics="DelayedQualitative"`): Boolean satisfaction with delayed evaluation
      - Returns: `True` or `False`
      - Waits for complete information before producing verdicts

2. **EagerQualitative** (`semantics="EagerQualitative"`): Boolean satisfaction with eager evaluation
      - Returns: `True` or `False`
      - Produces verdicts as soon as possible

3. **DelayedQuantitative** (`semantics="DelayedQuantitative"`, default): Quantitative robustness as a single value
      - Returns: Float value (positive = satisfied, negative = violated)

4. **Rosi** (`semantics="Rosi"`): Robustness as an interval
      - Returns: Tuple `(min, max)` representing robustness interval

### Monitoring Algorithms

- **Incremental** (`algorithm="Incremental"`, default): Efficient online monitoring using sliding windows
- **Naive** (`algorithm="Naive"`): Simple but less efficient approach

### Signal Interpolation

How a signal behaves *between its own samples*. This is what the formula is evaluated
over, so it decides the verdicts.

- **ZeroOrderHold** (`signal_interpolation="ZeroOrderHold"`, default): the signal holds
  its last value until the next sample arrives.
- **Linear** (`signal_interpolation="Linear"`): the signal ramps linearly between
  consecutive samples. A predicate then reports the exact time its threshold is crossed
  rather than waiting for the next sample. With `x = 6.0@0s, 2.0@4s`, `x > 4` becomes
  false at `2s` under Linear and at `4s` under ZeroOrderHold.

`Linear` is available for the two qualitative semantics only, and requires the
`Incremental` algorithm. Combining it with `DelayedQuantitative` or `Rosi` raises
`ValueError`: the satisfaction signal of a predicate over a linear signal is still
piecewise constant, so crossings make the *qualitative* answer exact, but a
piecewise-linear robustness needs its window supremum recovered, which crossings alone
do not give.

Only the samples you supply are ever monitored — no values are synthesized at other
signals' timestamps. A multi-signal formula is evaluated at each signal's own
timestamps, reading the other operands through the interpolation above. The one
exception is each signal's value at `t=0`, below.

### Initial Values

A formula over more than one signal is read from `t=0`. A signal whose first sample
arrives later would leave a prefix where the formula is read from the other signals
alone, so `init_signals=` gives each signal the value it holds until its own first
sample:

```python
monitor = mstlo.Monitor(
    mstlo.parse_formula("G[0,2]((x > 0) && (y > 0))"),
    init_signals={"x": 0.0, "y": 1.0},
)
```

Anything left out is `0.0`. A signal that really is sampled at `t=0` overrides its
initial value, so a trace that starts every signal together is monitored exactly as it
is written. A single-signal formula is defined wherever its signal is and ignores
`init_signals` entirely.

An initial value is data you assert, not data that was measured: with the default
`0.0`, `x > 0` is definitely false until `x` is first sampled.

### Deprecated: `synchronization`

The `synchronization=` argument is deprecated. Passing it emits a `DeprecationWarning`
and selects the signal interpolation of the same name, with `"None"` meaning
`"ZeroOrderHold"`:

| deprecated                      | use instead                               |
| ------------------------------- | ----------------------------------------- |
| `synchronization="ZeroOrderHold"` | `signal_interpolation="ZeroOrderHold"`  |
| `synchronization="None"`          | `signal_interpolation="ZeroOrderHold"`  |
| `synchronization="Linear"`        | `signal_interpolation="Linear"`         |

If both are given, `signal_interpolation` wins. `Monitor.get_synchronization_strategy()`
is deprecated in the same way; use `Monitor.get_signal_interpolation()`.

## Constructing STL Formulas

You can construct STL formulas using the provided API. For example:

```python
# Define a formula: Always[0,5](x > 0.5)
formula = mstlo.Formula.always(0, 5, mstlo.Formula.gt("x", 0.5))
# using parser
formula = mstlo.parse_formula("G[0,5](x > 0.5)")
```

This creates a formula that states "x should always be greater than 0.5 in the interval [0, 5]". See the API reference for more details on constructing formulas.
