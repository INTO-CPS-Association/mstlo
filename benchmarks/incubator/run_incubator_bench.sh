#!/usr/bin/env sh
#
# Everything downstream of the experiment: the recorded signal is reused as it
# is:
#
#   1. replay the signal through the monitors and derive the dataset
#   2. time mstlo-python and RTAMT on that signal
#   3. time the native Rust monitor, record its memory footprint, and take a
#      separate single pass for the cache sizes
#   4. draw the figures
#
# RTAMT has to be installed; see ../synthetic_signal/README.md for the build.
# Override any of the paths below if yours differ.
#
#   ./run_incubator_bench.sh                          # M = 50, normal+lid_open
#   M_RUNS=5 ./run_incubator_bench.sh                 # quick pass
#   PHASES= ./run_incubator_bench.sh                  # the whole session
#   DATA_DIR=rv26_results ./run_incubator_bench.sh    # another recording
#   RESULTS_DIR=/tmp/run1 ./run_incubator_bench.sh    # keep the recording intact

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Every stage below runs from $SCRIPT_DIR, so a path handed in relative to the
# caller's directory has to be resolved before the first cd.
abspath() {
	case "$1" in
	/*) printf '%s\n' "$1" ;;
	*) printf '%s\n' "$PWD/$1" ;;
	esac
}

# DATA_DIR only supplies the recording and is never written to; everything
# derived from it -- verdicts, datasets, timings, figures -- goes to RESULTS_DIR.
# The two default to the same directory, which is the layout rv26_results has.
DATA_DIR="$(abspath "${DATA_DIR:-$SCRIPT_DIR/data}")"
RESULTS_DIR="$(abspath "${RESULTS_DIR:-$DATA_DIR}")"

MSTLO_DIR="$(abspath "${MSTLO_DIR:-$SCRIPT_DIR/../../mstlo}")"

M_RUNS="${M_RUNS:-50}"
WARMUP_RUNS="${WARMUP_RUNS:-1}"
SIGNAL="$(abspath "${SIGNAL:-$DATA_DIR/signal.csv}")"

# Which phases of the recording every stage sees, comma-separated. Empty is the
# whole session (1337 samples): the monitors then get one uninterrupted signal,
# and the replay, the datasets, both benchmarks and the figures all cover the
# same samples.
PHASES="${PHASES-normal,lid_open}"

TOOLS="mstlo-python and RTAMT"

if [ ! -f "$SIGNAL" ]; then
	echo "missing $SIGNAL -- run run_experiment.py first" >&2
	exit 1
fi

if [ ! -f "$MSTLO_DIR/Cargo.toml" ]; then
	echo "no crate at $MSTLO_DIR -- set MSTLO_DIR" >&2
	exit 1
fi

# The Python stages take the phases as separate words
PHASE_ARGS=""
if [ -n "$PHASES" ]; then
	for phase in $(echo "$PHASES" | tr ',' ' '); do
		if ! awk -F, -v want="$phase" '
			NR == 1 { for (i = 1; i <= NF; i++) if ($i == "phase") col = i; next }
			col && $col == want { found = 1; exit }
			END { exit !found }
		' "$SIGNAL"; then
			echo "no samples labelled '$phase' in $SIGNAL" >&2
			exit 1
		fi
	done
	PHASE_ARGS="--phases $(echo "$PHASES" | tr ',' ' ')"
	SCOPE="$PHASES"
else
	SCOPE="all phases"
fi

mkdir -p "$RESULTS_DIR"

echo "=== 1/4  monitors and datasets ($SCOPE) ==="
cd "$SCRIPT_DIR"

python replay.py --datadir "$DATA_DIR" --outdir "$RESULTS_DIR" $PHASE_ARGS

python process_results.py --datadir "$DATA_DIR" --outdir "$RESULTS_DIR" \
	$PHASE_ARGS

echo
echo "=== 2/4  $TOOLS, M = $M_RUNS ==="

python benchmark.py --m-runs "$M_RUNS" --warmup-runs "$WARMUP_RUNS" \
	--datadir "$DATA_DIR" --outdir "$RESULTS_DIR" \
	$PHASE_ARGS

echo
echo "=== 3/4  native Rust, timings (M = $M_RUNS), memory and cache sizes ==="
(
	cd "$MSTLO_DIR"
	M_RUNS="$M_RUNS" WARMUP_RUNS="$WARMUP_RUNS" \
		SIGNAL_PATH="$SIGNAL" \
		PHASES="$PHASES" \
		OUTPUT_CSV="$RESULTS_DIR/benchmark_rust.csv" \
		OUTPUT_RAW_CSV="$RESULTS_DIR/benchmark_rust_runs.csv" \
		OUTPUT_MEMORY_CSV="$RESULTS_DIR/benchmark_rust_memory.csv" \
		cargo bench --bench incubator_benchmark

	# The cache counter is read after every update, inside the timed loop, so these timings are not the ones above and go to their own file.  One pass is enough -- the sizes are deterministic.
	M_RUNS=1 WARMUP_RUNS=0 MEMORY_RUNS=0 \
		SIGNAL_PATH="$SIGNAL" \
		PHASES="$PHASES" \
		OUTPUT_CSV="$RESULTS_DIR/benchmark_rust_cache_size_M=1.csv" \
		OUTPUT_RAW_CSV="$RESULTS_DIR/benchmark_rust_cache_size_M=1_raw.csv" \
		cargo bench --bench incubator_benchmark --features track-cache-size
)

echo
echo "=== 4/4  figures ==="
cd "$SCRIPT_DIR"
python plot_results.py --datadir "$RESULTS_DIR" --figdir "$RESULTS_DIR/figures"

echo
echo "done. results in $RESULTS_DIR:"
echo "  benchmark.csv                       $TOOLS"
echo "  benchmark_rust.csv                  native Rust timings and memory summary"
echo "  benchmark_rust_memory.csv           native Rust footprint after every update"
echo "  benchmark_rust_cache_size_M=1.csv   native Rust cached steps (M = 1)"
echo "  figures/                            the paper figures"
