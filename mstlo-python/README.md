# mstlo Python (`mstlo-python`)

Python bindings for mstlo, the `mstlo` online Signal Temporal Logic (STL) monitoring engine.

- PyPI package: `mstlo-python`
- Python import name: `mstlo_python`
- Rust core crate: `mstlo`

## What this package provides

`mstlo-python` exposes the high-performance Rust monitoring engine through a Pythonic API so you can parse STL formulas and evaluate streaming signals from Python applications and notebooks.

Supported monitoring semantics include:

- Delayed Qualitative
- Delayed Quantitative
- Eager Qualitative
- RoSI (Robust Satisfaction Intervals)

## Installation

From PyPI:

```bash
pip install mstlo-python
```

## Quick Start

```python
import mstlo_python as mstlo

phi = mstlo.parse_formula("G[0, 10](x > 5)")
monitor = mstlo.Monitor(phi, semantics="DelayedQuantitative")

output = monitor.update("x", 6.0, 0.5)
print(output)
print(output.to_dict())
```

## Project links

- Repository: [github.com/INTO-CPS-Association/mstlo](https://github.com/INTO-CPS-Association/mstlo)
- Documentation: [INTO-CPS-Association.github.io/mstlo](https://INTO-CPS-Association.github.io/mstlo/)
- Rust API docs: [docs.rs/mstlo](https://docs.rs/mstlo)

## License

This package is distributed under the INTO-CPS Association Public License (ICAPL), with GPL v3 as a supported subsidiary mode. See `LICENSE` in the repository root and `ICA-USAGE-MODE.txt` for the selected mode.

## Developer notes

For local development, wheel building, and docs generation instructions, see the repository documentation.
