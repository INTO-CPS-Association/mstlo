"""
Comprehensive benchmark suite for rtamt monitoring.

Tests formula families with varying temporal bounds and records performance metrics,
matching the structure and approach of mstlo_results.py.
"""

import csv
import time
import os
import argparse
from tqdm import tqdm
import rtamt

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
DEFAULT_M = 50  # Number of runs to average over
DEFAULT_SIGNAL_CSV = os.path.join(
    os.path.dirname(__file__),
    "BENCH_RESULTS",
    "signal_generation",
    "signals",
    "signal_20000_chirp.csv",
)
DEFAULT_OUTPUT_CSV = os.path.join(
    os.path.dirname(__file__),
    "results",
    "rtamt_benchmark_results_cpp.csv",
)

# Generate formulas matching the mstlo benchmark catalog
# Uses the same structure: globally, eventually, until with varying bounds
FORMULAS: dict[int, str] = {}

FORMULAS[1] = "(x < 0.5) and (x > -0.5)"
FORMULAS[2] = "G[0,1000] (x > 0.5 -> F[0,100] (x < 0.0))"
FORMULAS[3] = "(G[0,100] (x < 0.5)) or (G[100,150] (x > 0.0))"


def load_signal(path: str) -> list[tuple[float, float]]:
    """Return list of (timestep, value) from a CSV file."""
    signal: list[tuple[float, float]] = []
    with open(path, newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            signal.append((float(row["timestep"]), float(row["value"])))
    return signal


def make_discrete_monitor_cpp(spec: str) -> rtamt.StlDiscreteTimeOnlineSpecificationCpp:
    """Create a fresh rtamt discrete-time online monitor using C++ backend."""
    monitor = rtamt.StlDiscreteTimeOnlineSpecificationCpp()
    monitor.declare_var("x", "float")
    monitor.spec = spec
    monitor.parse()
    monitor.pastify()
    return monitor


def make_discrete_monitor_python(spec: str) -> rtamt.StlDiscreteTimeOnlineSpecification:
    """Create a fresh rtamt discrete-time online monitor for online monitoring."""
    monitor = rtamt.StlDiscreteTimeOnlineSpecification()
    monitor.declare_var("x", "float")
    monitor.spec = spec
    monitor.parse()
    monitor.pastify()
    return monitor


def bench_discrete_online_monitor(
    fid: int,
    spec: str,
    signal: list[tuple[float, float]],
    m: int,
    monitor_func: callable,
) -> dict:
    """Benchmark discrete-time online monitoring (one sample at a time)."""
    try:
        n_samples = len(signal)
        total_time = 0.0

        for _ in range(m):
            monitor = monitor_func(spec)
    """
    A focused RTAMT benchmark script for STL formulas,
    matching the structure and approach of mstlo_results.py.
    """
            total_time += t1 - t0

        avg_total = total_time / m
        avg_per_sample = avg_total / n_samples

        return {
            "formula_id": fid,
            "spec": spec,
            "monitor_type": "discrete-time",
            "mode": "online",
            "n_samples": n_samples,
            "m_runs": m,
            "avg_total_s": avg_total,
            "avg_per_sample_s": avg_per_sample,
            "avg_per_sample_us": avg_per_sample * 1e6,
        }
    except Exception as e:
        print(f"ERROR: Formula {fid} (discrete-time online) failed: {str(e)[:100]}")
        return None


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Benchmark rtamt monitors")
    parser.add_argument(
        "--signal-csv", default=DEFAULT_SIGNAL_CSV, help="Input signal CSV path"
    )
    parser.add_argument(
        "--m-runs", type=int, default=DEFAULT_M, help="Number of runs to average"
    )
    parser.add_argument("--output", default=DEFAULT_OUTPUT_CSV, help="Output CSV path")
    parser.add_argument(
        "--overwrite", action="store_true", help="Overwrite output CSV if it exists"
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()

    signal = load_signal(args.signal_csv)
    print(f"Loaded signal with {len(signal)} samples from {args.signal_csv}")
    print(f"Averaging over M = {args.m_runs} runs\n")

    # Initialize CSV file with headers
    out_path = args.output
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    if os.path.exists(out_path) and not args.overwrite:
        raise SystemExit(
            f"Output file already exists: {out_path}. Use --overwrite or set --output to a new path."
        )

    csv_file = open(out_path, "w", newline="", encoding="utf-8")
    fieldnames = [
        "formula_id",
        "spec",
        "monitor_type",
        "mode",
        "n_samples",
        "m_runs",
        "avg_total_s",
        "avg_per_sample_s",
        "avg_per_sample_us",
    ]
    writer = csv.DictWriter(csv_file, fieldnames=fieldnames)
    writer.writeheader()

    # --- Discrete-Time Online (C++ Backend) ---
    print("\nDiscrete-Time — Online (C++ Backend)")
    pbar = tqdm(FORMULAS.items(), desc="Discrete-Time Online C++")
    for fid, spec in pbar:
        pbar.set_postfix({"fid": fid, "spec": spec[:40] + "..."})
        res = bench_discrete_online_monitor(
            fid, spec, signal, args.m_runs, make_discrete_monitor_cpp
        )
        if res is not None:
            res["monitor_type"] = "discrete-time-cpp"
            writer.writerow(res)
            csv_file.flush()

    # --- Discrete-Time Online ---
    print("\nDiscrete-Time — Online")
    pbar = tqdm(FORMULAS.items(), desc="Discrete-Time Online")
    for fid, spec in pbar:
        pbar.set_postfix({"fid": fid, "spec": spec[:40] + "..."})
        res = bench_discrete_online_monitor(
            fid, spec, signal, args.m_runs, make_discrete_monitor_python
        )
        if res is not None:
            writer.writerow(res)
            csv_file.flush()

    csv_file.close()
    print(f"\nResults saved to {out_path}")


if __name__ == "__main__":
    main()
