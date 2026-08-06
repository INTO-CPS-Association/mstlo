# Synthetic Signal

- [Synthetic Signal](#synthetic-signal)
  - [Prerequisites](#prerequisites)
  - [Running the Experiments](#running-the-experiments)
  - [Results](#results)
  - [Results from the paper](#results-from-the-paper)

## Prerequisites

To run the experiments, ensure you have the following installed:

- Rust (with Cargo)
- Python 3.9 or higher
- Python packages listed in `requirements.txt` (install via `pip install -r requirements.txt`)
- RTAMT. This can be installed from source by following the instructions in their [GitHub repository](https://github.com/nickovic/rtamt). Follow their instructions, to make sure to build the CPP library and have it accessible in your Python environment.
  - To match the build profile of mstlo, make sure to build RTAMT in release mode.

Make also sure that the python bindings for mstlo are installed in your Python environment. You can do this by running `pip install -e .` in the `mstlo-python` directory.

`mstlo-python` builds the Rust code in release mode. To make sure that results are consistent, the bench profile used for the Rust benchmarks is set to inherit from the release profile (see [Cargo.toml](../Cargo.toml) in the root). The panic handling is set to `unwind`, but faster panic handling (e.g., `abort`) could be used for even better performance. However, this would make the benchmarks less comparable to the Python implementations, which do not have this option, as the benchmarking profile overrides the panic handling to `unwind`.

## Running the Experiments

To reproduce the experiments in the paper, run the `bench_all.sh` script. This will generate the necessary signals, run the benchmarks for mstlo, mstlo-python, and RTAMT and then perform the data analysis and plots.

```bash
sh experiments/bench_all.sh
```

**NOTE: The discrete-time monitors from RTAMT run very slowly on until-formulas. You might want to skip these tests by outcommenting the relevant lines in `rtamt_benchmark.py`.**

## Results

The results of the benchmarks will, by default, be saved in the `experiments/BENCH_RESULTS` directory, and the performance comparison plot will be saved as `performance_comparison.pdf`. You can open this PDF to visually compare the performance of our implementation against RTAMT and the Python STL library.

## Results from the paper

The results from the paper are available in the `paper_results/` directory. You can find the raw benchmark results in CSV format, as well as the generated performance comparison plot. They were run with Python 3.11.15 and rustc 1.95.0 on a MacBook Pro with an Apple M4 Pro chip. The RTAMT version used was 0.4.10.
