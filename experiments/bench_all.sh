#!/usr/bin/env sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SIGNAL_DIR="$SCRIPT_DIR/paper_results/signal_generation/signals"
OUTPUT_DIR="$SCRIPT_DIR/paper_results/outputs"

WARMUP_RUNS=10
M_RUNS=50
SIGNAL_SIZE=20000
SIGNAL_FILE="$SIGNAL_DIR/signal_${SIGNAL_SIZE}_chirp.csv"

CACHE_SIZE_RESULTS="$OUTPUT_DIR/mstlo/cache_size_results_M=1.csv" # for measuring cache sizes
NATIVE_RESULTS="$OUTPUT_DIR/mstlo/performance_results_M=${M_RUNS}.csv" # for performance comparison
NATIVE_RESULTS_RAW="$OUTPUT_DIR/mstlo/performance_results_M=${M_RUNS}_raw.csv"
PY_RESULTS="$OUTPUT_DIR/mstlo/python_performance_results_M=${M_RUNS}.csv"
PY_RESULTS_RAW="$OUTPUT_DIR/mstlo/python_performance_results_M=${M_RUNS}_raw.csv"
RTAMT_RESULTS="$OUTPUT_DIR/rtamt/rtamt_benchmark_results.csv"
RTAMT_RESULTS_RAW="$OUTPUT_DIR/rtamt/rtamt_benchmark_results_raw.csv"


mkdir -p "$SIGNAL_DIR" "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR/mstlo" "$OUTPUT_DIR/rtamt" "$OUTPUT_DIR/data_analysis"

# Generate signal for benchmarks.
# The default sampling-rate=1.0 Hz produces integer timesteps 0,1,2,...
# This means formula bounds (e.g. G[0,1000]) are numerically identical
# between mstlo (Duration-based, seconds) and RTAMT (discrete step indices).
python "$SCRIPT_DIR/signal_generation/signal_generator.py" --num-samples $SIGNAL_SIZE --output-path "$SIGNAL_FILE" --signal-type chirp

# ensure latest python is built in current python environment
pip install "$PROJECT_ROOT/mstlo-python" --force-reinstall

# Run Python benchmark scripts
python "$SCRIPT_DIR/python_benchmark.py" \
	--signal-csv "$SIGNAL_FILE" \
	--m-runs $M_RUNS \
	--warmup-runs $WARMUP_RUNS \
	--output "$PY_RESULTS" \
	--output-raw "$PY_RESULTS_RAW" \

# Run RTAMT benchmark script
python "$SCRIPT_DIR/rtamt_benchmark.py" \
	--signal-csv "$SIGNAL_FILE" \
	--m-runs $M_RUNS \
	--warmup-runs $WARMUP_RUNS \
	--output "$RTAMT_RESULTS" \
	--output-raw "$RTAMT_RESULTS_RAW" \

(
	cd "$PROJECT_ROOT/mstlo" || exit 1
	
	# run first to measure cache sizes
	WARMUP_RUNS=$WARMUP_RUNS M_RUNS=1 FORMULA_IDS="1,2,3,4" SIGNAL_PATH="$SIGNAL_FILE" OUTPUT_CSV="$CACHE_SIZE_RESULTS" cargo bench --bench paper_benchmark --features track-cache-size

	# run for performance comparison
	WARMUP_RUNS=$WARMUP_RUNS M_RUNS=$M_RUNS SIGNAL_PATH="$SIGNAL_FILE" OUTPUT_CSV="$NATIVE_RESULTS" OUTPUT_RAW_CSV="$NATIVE_RESULTS_RAW" cargo bench --bench paper_benchmark
)

### Data analysis and plotting
(
	# regression analysis
	python "$SCRIPT_DIR/data_analysis/regression_analysis.py" \
		--native-csv "$NATIVE_RESULTS" \
		--python-csv "$PY_RESULTS" \
		--rtamt-csv "$RTAMT_RESULTS" \
		--output "$OUTPUT_DIR/data_analysis/regression_fit_results.csv"

	# mann whitney U tests
	python "$SCRIPT_DIR/data_analysis/mann_whitney.py" \
        --csv-a "$NATIVE_RESULTS_RAW" \
        --csv-b "$RTAMT_RESULTS_RAW" \
        --label-a "native" --label-b "rtamt" \
        --group-by "formula_id" \
        --filter-a "semantics == 'DelayedQuantitative' and formula_id in [1, 2, 3, 4]" \
        --output "$OUTPUT_DIR/data_analysis/native_vs_rtamt_mwu_1_to_4.csv"

	python "$SCRIPT_DIR/data_analysis/mann_whitney.py" \
        --csv-a "$PY_RESULTS_RAW" \
        --csv-b "$RTAMT_RESULTS_RAW" \
        --label-a "mstlo-python" --label-b "rtamt" \
        --group-by "formula_id" \
        --filter-a "semantics == 'DelayedQuantitative' and formula_id in [1, 2, 3, 4]" \
        --output "$OUTPUT_DIR/data_analysis/mstlopython_vs_rtamt_mwu_1_to_4.csv"

	python "$SCRIPT_DIR/data_analysis/mann_whitney.py" \
        --csv-a "$NATIVE_RESULTS_RAW" \
        --csv-b "$PY_RESULTS_RAW" \
        --label-a "native" --label-b "python" \
        --group-by "semantics" \
        --output "$OUTPUT_DIR/data_analysis/native_vs_python_mwu.csv"

	# mstlo plots
	python "$SCRIPT_DIR/data_analysis/performance_comparison.py" \
		--benchmark-csv "$NATIVE_RESULTS" \
		--regression-csv "$OUTPUT_DIR/data_analysis/regression_fit_results.csv" \
		--output "$OUTPUT_DIR/data_analysis/performance_comparison_all.pdf"

	python "$SCRIPT_DIR/data_analysis/performance_comparison.py" \
		--benchmark-csv "$NATIVE_RESULTS" \
		--regression-csv "$OUTPUT_DIR/data_analysis/regression_fit_results.csv" \
		--output "$OUTPUT_DIR/data_analysis/performance_comparison_U_delquant.pdf" \
		--plot-operators U \
		--plot-semantics "delquant" \
		--plot-std \
		--no-log-scale

	python "$SCRIPT_DIR/data_analysis/performance_comparison.py" \
		--benchmark-csv "$NATIVE_RESULTS" \
		--regression-csv "$OUTPUT_DIR/data_analysis/regression_fit_results.csv" \
		--output "$OUTPUT_DIR/data_analysis/performance_comparison_FG_delquant.pdf" \
		--plot-operators F G\
		--plot-semantics "delquant" \
		--fg-mode "both" \
		--plot-std \
		--no-log-scale

	# RTAMT plots
	python "$SCRIPT_DIR/data_analysis/rtamt_performance_plot.py" \
		--benchmark-csv "$RTAMT_RESULTS" \
		--output "$OUTPUT_DIR/data_analysis/rtamt_performance_all.pdf"

	python "$SCRIPT_DIR/data_analysis/rtamt_performance_plot.py" \
		--benchmark-csv "$RTAMT_RESULTS" \
		--output "$OUTPUT_DIR/data_analysis/rtamt_performance_U.pdf" \
		--plot-std \
		--plot-operators U

	python "$SCRIPT_DIR/data_analysis/rtamt_performance_plot.py" \
		--benchmark-csv "$RTAMT_RESULTS" \
		--output "$OUTPUT_DIR/data_analysis/rtamt_performance_FG.pdf" \
		--plot-std \
		--plot-operators F G
)
