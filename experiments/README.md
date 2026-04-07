# Experiments

- [Experiments](#experiments)
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

Make also sure that the python bindings for mstlo are properly installed. You can do this by running `pip install -e .` in the `mstlo-python` directory.

## Running the Experiments

To reproduce the experiments in the paper, run the `bench_all.sh` script. This will generate the necessary signals, run the benchmarks for both M=1 and M=50, perform regression analysis, and generate the performance comparison plot.

```bash
cd experiments
./bench_all.sh
```

## Results

The results of the benchmarks will, by default, be saved in the `experiments/BENCH_RESULTS` directory, and the performance comparison plot will be saved as `performance_comparison.pdf`. You can open this PDF to visually compare the performance of our implementation against RTAMT and the Python STL library.

## Results from the paper

The results from the paper are available in the `paper_results/` directory. You can find the raw benchmark results in CSV format, as well as the generated performance comparison plot.
