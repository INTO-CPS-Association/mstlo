# mstlo-python

[![Python Tests](https://github.com/INTO-CPS-Association/mstlo/workflows/Python%20Tests/badge.svg)](https://github.com/INTO-CPS-Association/mstlo/actions/workflows/python-tests.yml)
[![PyPI](https://img.shields.io/pypi/v/mstlo-python.svg)](https://pypi.org/project/mstlo-python/)
[![Python versions](https://img.shields.io/pypi/pyversions/mstlo-python.svg)](https://pypi.org/project/mstlo-python/)
[![Docs](https://img.shields.io/badge/docs-GitHub%20Pages-blue)](https://INTO-CPS-Association.github.io/mstlo/)

Python bindings for [mstlo](https://github.com/INTO-CPS-Association/mstlo) (_mistletoe_), a Rust library for online monitoring of Signal Temporal Logic (STL) specifications. The package exposes the Rust monitoring engine through a Pythonic API, so STL formulas can be parsed and evaluated over streaming signals from Python applications and notebooks at comparable performance.

- PyPI package: `mstlo-python`
- Import name: `mstlo_python`
- Rust core crate: [`mstlo`](https://crates.io/crates/mstlo)

<!-- TOC -->

- [mstlo-python](#mstlo-python)
  - [Features](#features)
  - [Installation](#installation)
  - [Usage](#usage)
    - [Batch updates](#batch-updates)
  - [Documentation](#documentation)
  - [Building from Source](#building-from-source)
  - [License](#license)

<!-- /TOC -->

## Features

- **Unified semantics interface:** delayed qualitative, delayed quantitative, eager qualitative and Robust Satisfaction Intervals (RoSI) behind a single `Monitor` class.
- **Dense-time signals:** samples may arrive at arbitrary, irregular timestamps, per signal, with zero-order hold or linear interpolation between them.
- **High performance:** evaluation runs in the Rust engine; benchmarks show throughput exceeding existing state-of-the-art tools.

## Installation

```bash
pip install mstlo-python
```

## Usage

A formula is parsed from the DSL syntax, then a monitor is fed samples as `(signal, value, timestamp)`, with timestamps in seconds. This monitors `G[0, 10](x > 5)` ("x stays above 5 for the next ten seconds") under RoSI semantics:

```python
import mstlo_python as mstlo

# Parse formula using the DSL syntax
phi = mstlo.parse_formula("G[0, 10](x > 5)")

# Create a monitor using the selected semantics
monitor = mstlo.Monitor(phi, semantics="Rosi")

# Update the monitor with streaming data (signal_name, value, timestamp)
output = monitor.update("x", 6.0, 0.5)
print(f"Verdicts: {output.verdicts()}")
# Verdicts: [(0.5, (-inf, 1.0))]  # (timestamp, (lower_bound, upper_bound) for robustness)

output = monitor.update("x", 3.0, 1.2)
print(f"Verdicts: {output.verdicts()}")
# Verdicts: [(0.5, (-inf, -2.0)), (1.2, (-inf, -2.0))]  # early indication of violation
```

The semantics are selected with `semantics=` (`"DelayedQualitative"`, `"DelayedQuantitative"`, `"EagerQualitative"` or `"Rosi"`), the interpolation between samples with `signal_interpolation=` (`"ZeroOrderHold"` or `"Linear"`), and the values signals hold before their first sample with `init_signals={"x": 0.0}`.

### Batch updates

Whole traces can be fed at once with `update_batch`, either signal-major (a dict of `(value, timestamp)` lists) or flat (an iterable of `(signal, value, timestamp)` tuples):

```python
monitor = mstlo.Monitor(mstlo.parse_formula("x > 10.0"), semantics="Rosi")

output = monitor.update_batch({"x": [(5.0, 0.0), (15.0, 1.0), (8.0, 2.0)]})
output = monitor.update_batch([("x", 12.0, 3.0), ("x", 9.0, 4.0)])
```

More examples, including a notebook comparing the semantics, are in [`mstlo-python/examples`](https://github.com/INTO-CPS-Association/mstlo/tree/main/mstlo-python/examples).

## Documentation

- Python API reference: [INTO-CPS-Association.github.io/mstlo](https://INTO-CPS-Association.github.io/mstlo/)
- [Signal Temporal Logic and Evaluation Semantics](https://github.com/INTO-CPS-Association/mstlo/blob/main/docs/signal_temporal_logic.md): the supported STL syntax and the formal definitions of the four semantics.
- [Implementation](https://github.com/INTO-CPS-Association/mstlo/blob/main/docs/implementation.md): the signal model, how input is consumed, when verdicts are emitted, and settings that can hurt performance.
- Rust crate: [README](https://github.com/INTO-CPS-Association/mstlo#readme) and [docs.rs/mstlo](https://docs.rs/mstlo)

## Building from Source

Prerequisites: [Rust](https://rustup.rs/) (stable toolchain), [Python 3.9+](https://www.python.org/) and [maturin](https://github.com/PyO3/maturin).

```bash
git clone https://github.com/INTO-CPS-Association/mstlo.git
cd mstlo/mstlo-python
pip install maturin pytest
maturin develop
pytest
```

## License

This package is distributed under the INTO-CPS Association Public License (ICAPL) with GPL v3 as a supported subsidiary mode. See [`LICENSE`](https://github.com/INTO-CPS-Association/mstlo/blob/main/LICENSE) for the full terms and [`ICA-USAGE-MODE.txt`](https://github.com/INTO-CPS-Association/mstlo/blob/main/ICA-USAGE-MODE.txt) for the selected usage mode in this distribution.
