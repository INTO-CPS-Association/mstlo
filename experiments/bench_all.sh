#!/usr/bin/env sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SIGNAL_DIR="$SCRIPT_DIR/BENCH_RESULTS/signal_generation/signals"
OUTPUT_DIR="$SCRIPT_DIR/BENCH_RESULTS/outputs"

M1_RESULTS="$OUTPUT_DIR/cache_size_results_M=1.csv" # for measuring cache sizes
M50_RESULTS="$OUTPUT_DIR/performance_results_M=50.csv" # for performance comparison
REGRESSION_OUT="$OUTPUT_DIR/regression_fit_results.csv"
FIGURE_OUT="$OUTPUT_DIR/performance_comparison.pdf"
PY_RESULTS="$OUTPUT_DIR/python_performance_results_M=50.csv"
RTAMT_RESULTS="$OUTPUT_DIR/rtamt_benchmark_results.csv"

mkdir -p "$SIGNAL_DIR" "$OUTPUT_DIR"

# Generate signal for benchmarks
python "$SCRIPT_DIR/signal_generation/signal_generator.py" --num-samples 20000 --output-path "$SIGNAL_DIR/signal_20000_chirp.csv" --signal-type chirp

# ensure latest python is built in current python environment
pip install -e "$PROJECT_ROOT/mstlo-python" --force-reinstall

# Run Python benchmark scripts in experiments/
python "$SCRIPT_DIR/python_benchmark.py" \
	--signal-csv "$SIGNAL_DIR/signal_20000_chirp.csv" \
	--m-runs 50 \
	--output "$PY_RESULTS" \
	# --overwrite

python "$SCRIPT_DIR/rtamt_cpponline_benchmark.py" \
	--signal-csv "$SIGNAL_DIR/signal_20000_chirp.csv" \
	--m-runs 50 \
	--output "$RTAMT_RESULTS" \
 	# --overwrite

(
	cd "$PROJECT_ROOT/mstlo" || exit 1
	M_RUNS=1 FORMULA_IDS="1,2,3" SIGNAL_PATH="$SIGNAL_DIR/signal_20000_chirp.csv" OUTPUT_CSV="$M1_RESULTS" cargo bench --bench paper_benchmark --features track-cache-size
	M_RUNS=50 SIGNAL_PATH="$SIGNAL_DIR/signal_20000_chirp.csv" OUTPUT_CSV="$M50_RESULTS" cargo bench --bench paper_benchmark

	python "$SCRIPT_DIR/data_analysis/regression_analysis.py" \
		--native-csv "$M50_RESULTS" \
		--output "$REGRESSION_OUT"

	python "$SCRIPT_DIR/data_analysis/performance_comparison.py" \
		--benchmark-csv "$M50_RESULTS" \
		--regression-csv "$REGRESSION_OUT" \
		--output "$FIGURE_OUT"
)
