# mstlo

[![Rust CI](https://github.com/INTO-CPS-Association/mstlo/workflows/Rust%20CI/badge.svg)](https://github.com/INTO-CPS-Association/mstlo/actions/workflows/rust.yml)
[![Python Tests](https://github.com/INTO-CPS-Association/mstlo/workflows/Python%20Tests/badge.svg)](https://github.com/INTO-CPS-Association/mstlo/actions/workflows/python-tests.yml)
[![codecov](https://codecov.io/gh/INTO-CPS-Association/mstlo/branch/main/graph/badge.svg)](https://codecov.io/gh/INTO-CPS-Association/mstlo)
[![docs.rs](https://img.shields.io/docsrs/mstlo)](https://docs.rs/mstlo)
[![Docs](https://img.shields.io/badge/docs-GitHub%20Pages-blue)](https://INTO-CPS-Association.github.io/mstlo/)
[![crates.io](https://img.shields.io/crates/v/mstlo.svg)](https://crates.io/crates/mstlo)
[![PyPI](https://img.shields.io/pypi/v/mstlo-python.svg)](https://pypi.org/project/mstlo-python/)
[![Python versions](https://img.shields.io/pypi/pyversions/mstlo-python.svg)](https://pypi.org/project/mstlo-python/)
<!-- [![Crates.io Downloads](https://img.shields.io/crates/d/mstlo)](https://crates.io/crates/mstlo)
[![PyPI Downloads](https://img.shields.io/pypi/dm/mstlo-python.svg?label=PyPI%20downloads)](https://pypi.org/project/mstlo-python/) -->

mstlo (_mistletoe_) is a Rust library for online monitoring of Signal Temporal Logic (STL) specifications. It is designed for high performance and low memory usage, making it suitable for real-time monitoring of Cyber-Physical Systems. Python bindings are published as [`mstlo-python`](https://pypi.org/project/mstlo-python/).

<!-- TOC -->

- [mstlo](#mstlo)
  - [Features](#features)
  - [Installation](#installation)
  - [Usage](#usage)
    - [Rust](#rust)
    - [Python](#python)
  - [Documentation](#documentation)
  - [Building from Source](#building-from-source)
  - [License](#license)

<!-- /TOC -->

## Features

- **Embedded DSL:** the `stl!` macro embeds specifications in Rust and syntax-checks them at compile time.
- **Unified semantics interface:** delayed qualitative, delayed quantitative, eager qualitative and Robust Satisfaction Intervals (RoSI) behind a single monitor API.
- **Dense-time signals:** samples may arrive at arbitrary, irregular timestamps, per signal, with zero-order hold or linear interpolation between them.
- **Python bindings:** the `mstlo-python` package (import name `mstlo_python`) enables interactive workflows, e.g. in Jupyter notebooks.
- **High performance:** benchmarks show throughput exceeding existing state-of-the-art tools.

## Installation

Rust:

```bash
cargo add mstlo
```

Python:

```bash
pip install mstlo-python
```

## Usage

### Rust

The monitor is configured with a builder, then fed samples as they arrive. This monitors `G[0, 2](x > 5)` ("x stays above 5 for the next two seconds") under RoSI semantics:

```rust
use mstlo::monitor::{Rosi, StlMonitor};
use mstlo::{step, stl};

fn main() {
    // Define a formula using the embedded DSL
    let formula = stl! {G[0, 2](x > 5.0)};

    // Build the monitor
    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(Rosi)
        .build()
        .expect("Failed to build monitor");

    // Feed data steps to the monitor
    let out1 = monitor.update(&step!("x", 7.0, 0s));
    println!("{:?}", out1.verdicts());
    // [Step { signal: "x", value: RobustnessInterval(-inf, 2.0), timestamp: 0ns }]
    // at time 0, the robustness value is in the interval (-inf, 2.0)

    let out2 = monitor.update(&step!("x", 4.0, 1s));
    println!("{:?}", out2.verdicts());
    // [Step { signal: "x", value: RobustnessInterval(-inf, -1.0), timestamp: 0ns },
    //  Step { signal: "x", value: RobustnessInterval(-inf, -1.0), timestamp: 1s }]
    // early violation detection for times 0 and 1
}
```

Whole traces can also be fed at once with `update_batch` and the `steps!` macro, see [Batch Updates](https://github.com/INTO-CPS-Association/mstlo/blob/main/docs/implementation.md#batch-updates). More examples are in [`mstlo/examples`](https://github.com/INTO-CPS-Association/mstlo/tree/main/mstlo/examples).

### Python

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

More examples, including a notebook comparing the semantics, are in [`mstlo-python/examples`](https://github.com/INTO-CPS-Association/mstlo/tree/main/mstlo-python/examples).

## Documentation

- [Signal Temporal Logic and Evaluation Semantics](https://github.com/INTO-CPS-Association/mstlo/blob/main/docs/signal_temporal_logic.md): the supported STL syntax and the formal definitions of the four semantics.
- [Implementation](https://github.com/INTO-CPS-Association/mstlo/blob/main/docs/implementation.md): the signal model, how input is consumed, when verdicts are emitted, and settings that can hurt performance.
- Rust API reference: [docs.rs/mstlo](https://docs.rs/mstlo)
- Python API reference: [INTO-CPS-Association.github.io/mstlo](https://INTO-CPS-Association.github.io/mstlo/)

## Building from Source

Prerequisites: [Rust](https://rustup.rs/) (stable toolchain), and for the Python bindings [Python 3.9+](https://www.python.org/) and [maturin](https://github.com/PyO3/maturin).

```bash
git clone https://github.com/INTO-CPS-Association/mstlo.git
cd mstlo
cargo test -p mstlo
cargo run -p mstlo --example intro_example
```

Python bindings, from the repository root:

```bash
cd mstlo-python
pip install maturin pytest
maturin develop
pytest
```

## License

This project is distributed under the INTO-CPS Association Public License (ICAPL) with GPL v3 as a supported subsidiary mode. See [`LICENSE`](https://github.com/INTO-CPS-Association/mstlo/blob/main/LICENSE) for the full terms and [`ICA-USAGE-MODE.txt`](https://github.com/INTO-CPS-Association/mstlo/blob/main/ICA-USAGE-MODE.txt) for the selected usage mode in this distribution.
