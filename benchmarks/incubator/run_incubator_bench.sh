#!/usr/bin/env sh
#
# Everything downstream of the experiment: the recorded signal is reused as it
# is:
#
#   1. replay the signal through the monitors and derive the dataset
#   2. time mstlo-python and RTAMT on that signal (SKIP_RTAMT drops RTAMT)
#   3. time the native Rust monitor, record its memory footprint, and take a
#      separate single pass for the cache sizes
#   4. draw the figures
#
# Override any of the paths below if yours differ.
#
#   ./run_incubator_bench.sh                          # M = 50, whole session
#   M_RUNS=5 ./run_incubator_bench.sh                 # quick pass
#   PHASES=normal,lid_open ./run_incubator_bench.sh   # steady state only
#   SKIP_RTAMT=1 ./run_incubator_bench.sh             # mstlo only, no RTAMT

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DATA_DIR="$SCRIPT_DIR/data"

MSTLO_DIR="${MSTLO_DIR:-$SCRIPT_DIR/../../mstlo}"

M_RUNS="${M_RUNS:-50}"
WARMUP_RUNS="${WARMUP_RUNS:-1}"
SIGNAL="$DATA_DIR/signal.csv"

# Which phases of the recording every stage sees, comma-separated.  Empty, the
# default, is the whole session: the monitors then get one uninterrupted signal,
# and the replay, the datasets, both benchmarks and the figures all cover the
# same samples.
PHASES="${PHASES:-}"

# Set to anything non-empty to leave RTAMT out: stage 2 then times mstlo alone
# and never imports rtamt, so the script also runs in an environment without it.
SKIP_RTAMT="${SKIP_RTAMT:-}"

if [ -n "$SKIP_RTAMT" ]; then
	RTAMT_ARGS="--no-rtamt"
	TOOLS="mstlo-python"
else
	RTAMT_ARGS=""
	TOOLS="mstlo-python and RTAMT"
fi

if [ ! -f "$SIGNAL" ]; then
	echo "missing $SIGNAL -- run run_experiment.py first" >&2
	exit 1
fi

if [ ! -f "$MSTLO_DIR/Cargo.toml" ]; then
	echo "no crate at $MSTLO_DIR -- set MSTLO_DIR" >&2
	exit 1
fi

# The Python stages take the phases as separate words, the Rust one as the
# comma-separated string it already parses.  A typo would otherwise leave every
# stage with an empty signal, so each name is checked against the recording.
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

echo "=== 1/4  monitors and datasets ($SCOPE) ==="
cd "$SCRIPT_DIR"

python replay.py $PHASE_ARGS

python process_results.py $PHASE_ARGS

echo
echo "=== 2/4  $TOOLS, M = $M_RUNS ==="

python benchmark.py --m-runs "$M_RUNS" --warmup-runs "$WARMUP_RUNS" \
	$PHASE_ARGS $RTAMT_ARGS

echo
echo "=== 3/4  native Rust, timings (M = $M_RUNS), memory and cache sizes ==="
(
	cd "$MSTLO_DIR"
	M_RUNS="$M_RUNS" WARMUP_RUNS="$WARMUP_RUNS" \
		SIGNAL_PATH="$SIGNAL" \
		PHASES="$PHASES" \
		OUTPUT_CSV="$DATA_DIR/benchmark_rust.csv" \
		OUTPUT_RAW_CSV="$DATA_DIR/benchmark_rust_runs.csv" \
		OUTPUT_MEMORY_CSV="$DATA_DIR/benchmark_rust_memory.csv" \
		cargo bench --bench incubator_benchmark

	# The cache counter is read after every update, inside the timed loop, so
	# these timings are not the ones above and go to their own file.  One pass
	# is enough -- the sizes are deterministic -- and the memory profiling is
	# off here because the run above already did it.
	M_RUNS=1 WARMUP_RUNS=0 MEMORY_RUNS=0 \
		SIGNAL_PATH="$SIGNAL" \
		PHASES="$PHASES" \
		OUTPUT_CSV="$DATA_DIR/benchmark_rust_cache_size_M=1.csv" \
		OUTPUT_RAW_CSV="$DATA_DIR/benchmark_rust_cache_size_M=1_raw.csv" \
		cargo bench --bench incubator_benchmark --features track-cache-size
)

echo
echo "=== 4/4  figures ==="
cd "$SCRIPT_DIR"
python plot_results.py

echo
echo "done. results in $DATA_DIR:"
echo "  benchmark.csv                       $TOOLS"
echo "  benchmark_rust.csv                  native Rust timings and memory summary"
echo "  benchmark_rust_memory.csv           native Rust footprint after every update"
echo "  benchmark_rust_cache_size_M=1.csv   native Rust cached steps (M = 1)"
echo "  figures/                            the paper figures"
