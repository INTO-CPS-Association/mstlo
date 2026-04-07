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

### Signal Synchronization

- **ZeroOrderHold** (`synchronization="ZeroOrderHold"`, default): Zero-order hold interpolation
- **Linear** (`synchronization="Linear"`): Linear interpolation
- **None** (`synchronization="None"`): No interpolation

## Constructing STL Formulas

You can construct STL formulas using the provided API. For example:

```python
# Define a formula: Always[0,5](x > 0.5)
formula = mstlo.Formula.always(0, 5, mstlo.Formula.gt("x", 0.5))
# using parser
formula = mstlo.parse_formula("G[0,5](x > 0.5)")
```

This creates a formula that states "x should always be greater than 0.5 in the interval [0, 5]". See the API reference for more details on constructing formulas.
