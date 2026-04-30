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
[![Docs Status](https://img.shields.io/badge/docs-latest-brightgreen)](https://INTO-CPS-Association.github.io/mstlo/)
[![PyPI Downloads](https://img.shields.io/pypi/dm/mstlo-python.svg?label=PyPI%20downloads)](https://pypi.org/project/mstlo-python/) -->

mstlo is a Rust library for online monitoring of Signal Temporal Logic (STL) specifications. It is designed for high performance and low memory usage, making it suitable for real-time applications. The Python bindings are published as `mstlo-python`.

- [mstlo](#mstlo)
  - [About](#about)
  - [Theory](#theory)
    - [Signal Temporal Logic (STL)](#signal-temporal-logic-stl)
    - [Evaluation Semantics](#evaluation-semantics)
  - [Installation](#installation)
    - [Rust](#rust)
    - [Python](#python)
  - [Usage](#usage)
    - [Rust Usage](#rust-usage)
    - [Python Usage](#python-usage)
  - [Building from Source](#building-from-source)
    - [Prerequisites](#prerequisites)
    - [Rust Crate](#rust-crate)
    - [Python Bindings](#python-bindings)
  - [References](#references)
  - [License](#license)

## About

Cyber-Physical Systems (CPSs) increasingly rely on real-time fault detection and runtime monitoring to ensure safe operation. mstlo provides a unified monitoring interface that addresses the stringent performance requirements of these systems. Key features include:

- **Embedded DSL:** A macro-based DSL (`stl!`) allows specifications to be embedded and syntax-checked directly at compile time in Rust.
- **Unified Semantics Interface:** Supports multiple online evaluation modes (Qualitative, Quantitative, Eager, and RoSI) in a single framework.
- **Python Bindings:** Exposed via the `mstlo-python` package (import name: `mstlo_python`) to enable interactive workflows in environments like Jupyter Notebooks.
- **High Performance:** Benchmarks demonstrate throughput exceeding existing state-of-the-art tools.

Published package pages:

- Rust crate: [crates.io/crates/mstlo](https://crates.io/crates/mstlo)
- Python package: [pypi.org/project/mstlo-python](https://pypi.org/project/mstlo-python/)
- Rust API docs: [docs.rs/mstlo](https://docs.rs/mstlo)
- Python docs: [INTO-CPS-Association.github.io/mstlo](https://INTO-CPS-Association.github.io/mstlo/)

## Theory

### Signal Temporal Logic (STL)

Signal Temporal Logic (STL) is a formalism for specifying properties of real-valued signals that evolve over time, providing a compact language to describe the desired behaviors of dynamic systems. STL evaluates properties over signals, which are defined as functions mapping a time domain (such as nonnegative real numbers, $\mathbb{R}_{\ge0}$) to a value domain.

mstlo focuses on bounded STL, meaning all temporal operators are constrained by finite time intervals of the form $[a, b]$, where $0 \le a < t$.

The core syntax of STL is built from a minimal set of primitive operators:

- **True ($\top$)**: The Boolean constant True.

- **Atomic Predicates ($\mu(x) < c$)**: Evaluates to True if the function over the signal is less than a constant $c$.

- **Negation ($\neg\phi$)**: The logical NOT of a formula.

- **Conjunction ($\phi \wedge \psi$)**: The logical AND of two formulas.

- **Until ($\phi \mathcal{U}_{[a,b]} \psi$)**: States that $\phi$ must hold continuously until $\psi$ becomes true within the time interval $[a, b]$.

From these primitives, the library derives other highly useful operators to simplify specifications:

- **Disjunction (OR)**: $\phi \vee \psi$

- **Implication**: $\phi \rightarrow \psi$

- **Eventually (Future)**: $\diamondsuit_{[a,b]}\phi$

- **Globally (Always)**: $\Box_{[a,b]}\phi$

### Evaluation Semantics

See also [semantics-comparison.ipynb](mstlo-python/examples/semantics-comparison.ipynb) for an interactive demonstration of the different semantics.

An online monitor observes a system's behavior incrementally as discrete samples arrive. mstlo provides a unified interface supporting four distinct monitoring semantics, allowing users to trade off between expressiveness and verdict latency:

- **Delayed Qualitative:** Computes standard Boolean satisfaction. This mode requires the signal to be fully resolved up to the formula's maximum temporal horizon before emitting a strict true/false verdict.

- **Delayed Quantitative:** Computes a real-valued robustness score indicating the precise degree of satisfaction or violation. Similar to the qualitative mode, it requires full signal availability up to the temporal depth.

- **Eager Qualitative:** Leverages the monotonicity in Boolean and temporal logic to emit early verdicts over partial traces. For example, a violation of a "globally" ($\Box$) property immediately yields a false verdict without waiting for the full interval to elapse.

- **Robust Satisfaction Intervals (RoSI):** Provides quantitative reasoning over partial traces. Instead of a single robustness value, the monitor computes an interval $[\rho_{min}, \rho_{max}]$ that encloses all possible future robustness values. A formula is definitively satisfied when $\rho_{min} > 0$ and definitively violated when $\rho_{max} < 0$.

To compute these semantics efficiently, mstlo uses a bottom-up dynamic programming approach. For sliding window operations (like *eventually* and *globally*), the library incorporates Lemire's algorithm to aggressively reduce cache footprints and computation time.

## Installation

### Rust

Add mstlo to your `Cargo.toml`:

```toml
[dependencies]
mstlo = "0.1.0"
```

### Python

Install the Python bindings via pip:

```bash
pip install mstlo-python
```

## Usage

### Rust Usage

For more examples, see the [`mstlo/examples`](./mstlo/examples) directory.
The following snippet demonstrates how to create a monitor for the STL formula $\Box_{[0, 2]}(x > 5)$ using the embedded DSL and process incoming signal data.

mstlo utilizes the Builder pattern to configure the monitor's formula, semantics, and algorithm before processing the data stream.

```rust
use mstlo::step;
use mstlo::monitor::{Rosi, StlMonitor};
use std::time::Duration;

fn main() {
    // Define a formula using the embedded DSL
    let formula = mstlo::stl!(G[0, 2](x > 5.0));

    // Build the monitor
    let mut monitor = StlMonitor::builder()
        .formula(formula)
        .semantics(Rosi)
        .build()
        .expect("Failed to build monitor");

    // Feed data steps to the monitor
    let out1 = monitor.update(&step!("x", 7.0, 0s));
    println!("{:?}", out1.verdicts());
    // [Step { signal: "x", value: RobustnessInterval(-inf, 2.0), timestamp: 0ns }] // at time 0, robustness value is in interval (-inf, 2.0)
    let out2 = monitor.update(&step!("x", 4.0, 1s));
    println!("{:?}", out2.verdicts());
    // Output after second update: [Step { signal: "x", value: RobustnessInterval(-inf, -1.0), timestamp: 0ns }, Step { signal: "x", value: RobustnessInterval(-inf, -1.0), timestamp: 1s }] // early violation detection for times 0 and 1
}
```

### Python Usage

For more Python examples, see the [`mstlo-python/examples`](./mstlo-python/examples) directory.
The Python API wraps the core Rust engine, offering comparable performance via an intuitive Pythonic interface.

```python
import mstlo_python as mstlo

# Parse formula using the DSL syntax
phi = mstlo.parse_formula("G[0, 10](x > 5)")

# Create a monitor using the selected semantics
monitor = mstlo.Monitor(phi, semantics="Rosi")

# Update the monitor with streaming data (signal_name, value, timestamp)
output = monitor.update("x", 6.0, 0.5)

# Print formatted verdicts or extract structured data
print(f"Verdicts: {output.verdicts()}")
# Verdicts: [(0.5, (-inf, 1.0))] # (timestamp, (lower_bound, upper_bound) for robustness)
output = monitor.update("x", 3.0, 1.2)
print(f"Verdicts: {output.verdicts()}")
# Verdicts: [(0.5, (-inf, -2.0)), (1.2, (-inf, -2.0))] # early indication of violation
```

## Building from Source

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Python 3.9+](https://www.python.org/) and [maturin](https://github.com/PyO3/maturin) (for Python bindings only)

### Rust Crate

```bash
git clone https://github.com/INTO-CPS-Association/mstlo.git
cd mstlo/mstlo
cargo build
```

Run the test suite:

```bash
cargo test
```

Run the included examples:

```bash
cargo run --example intro_example
cargo run --example simple_example
cargo run --example variables_example
```

### Python Bindings

```bash
cd mstlo-python
pip install maturin
maturin develop
```

Run the Python tests:

```bash
pip install pytest
pytest
```

## References

1. Deshmukh, J.V., et al. "Robust Online Monitoring of Signal Temporal Logic." *arXiv preprint arXiv:1506.08234* (2015).
2. Lemire, D. "Streaming Maximum-Minimum Filter Using No More than Three Comparisons per Element." *arXiv preprint arXiv:cs/0610046* (2007).
3. Maler, O., & Nickovic, D. "Monitoring Temporal Properties of Continuous Signals." *Formal Techniques, Modelling and Analysis of Timed and Fault-Tolerant Systems* (2004).

## License

This project is distributed under the INTO-CPS Association Public License (ICAPL) with GPL v3 as a supported subsidiary mode. See `LICENSE` for the full terms and `ICA-USAGE-MODE.txt` for the selected usage mode in this distribution.
