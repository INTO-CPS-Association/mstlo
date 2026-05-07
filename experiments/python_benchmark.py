import mstlo_python as mstlo
import csv
import math
import time
import os
import argparse
import numpy as np
from tqdm import tqdm

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
DEFAULT_M = 1  # Number of runs to average over
DEFAULT_SIGNAL_CSV = os.path.join(
    os.path.dirname(__file__),
    "paper_results",
    "signal_generation",
    "signals",
    "signal_20000_chirp.csv",
)
DEFAULT_OUTPUT_CSV = os.path.join(
    os.path.dirname(__file__),
    "results",
    "python_benchmark_results.csv",
)
DEFAULT_OUTPUT_RAW_CSV = os.path.join(
    os.path.dirname(__file__),
    "results",
    "python_benchmark_results_raw.csv",
)

# Formulas matching the uploaded Rust benchmark CSV labeling.
# Uses the same DSL syntax as Rust's stl! macro (parsed via parse_formula)
#
# Formula IDs are family labels (not unique row IDs):
# 1: phi1
# 2: phi2
# 3: phi3
# 4: until family (all bounds)
# 5: globally family (all bounds)
# 6: eventually family (all bounds)
FORMULAS: list[tuple[int, str]] = []
b = np.arange(0, 5001, 100)
b[0] += 1  # avoid zero bound for the first formula

# phi1
FORMULAS.append((1, "(x < 0.5) and (x > -0.5)"))

# phi2
FORMULAS.append((2, "G[0,1000] (x > 0.5 -> F[0,100] (x < 0.0))"))

# phi3
FORMULAS.append((3, "(x < 0.5) U[0,1000] (x < 0.0)"))

# phi4
FORMULAS.append((4, "(G[0,100] (x < 0.5)) or (G[100,150] (x > 0.0))"))

# # until formulas
for bnd in b:
    FORMULAS.append((5, f"(x < 0.0) U[0,{bnd:.1f}] (x > 0.0)"))

# globally formulas
for bnd in b:
    FORMULAS.append((6, f"G[0,{bnd:.1f}] (x > 0.0)"))

# eventually formulas
for bnd in b:
    FORMULAS.append((7, f"F[0,{bnd:.1f}] (x > 0.0)"))


def load_signal(path: str) -> list[tuple[float, float]]:
    """Return list of (timestep, value) from a CSV file."""
    signal: list[tuple[float, float]] = []
    with open(path, newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            signal.append((float(row["timestep"]), float(row["value"])))
    return signal


def bench_formula(
    formula_id: int,
    spec: str,
    signal: list[tuple[float, float]],
    m: int,
    semantics: str = "DelayedQuantitative",
    algorithm: str = "Incremental",
    warmup_runs: int = 1,
) -> dict:
    """Run *m* passes of the formula and return per-sample timing stats.

    Each run creates a fresh monitor, then feeds the signal one sample at
    a time via monitor.update() — true online monitoring.
    *warmup_runs* untimed passes are performed first to prime allocator and
    OS page caches.
    """
    n_samples = len(signal)
    total_time = 0.0
    run_times: list[float] = []
    parsed_formula = mstlo.parse_formula(spec)
    temporal_depth = 0

    for i in range(-warmup_runs, m):
        is_warmup = i < 0
        if not is_warmup:
            print(
                f"  Run {i+1}/{m} for formula ID {formula_id}...", end="\r", flush=True
            )
        monitor = mstlo.Monitor(
            parsed_formula,
            semantics=semantics,
            algorithm=algorithm,
            synchronization="None",
        )
        t0 = time.perf_counter()
        for ts, val in signal:
            monitor.update("x", val, ts)
        t1 = time.perf_counter()
        if not is_warmup:
            elapsed = t1 - t0
            run_times.append(elapsed)
            total_time += elapsed
        temporal_depth = monitor.get_temporal_depth()

    avg_total = total_time / m
    avg_per_sample = avg_total / n_samples
    std_total = (
        math.sqrt(sum((t - avg_total) ** 2 for t in run_times) / (m - 1))
        if m > 1
        else 0.0
    )
    std_per_sample = std_total / n_samples

    return {
        "formula_id": formula_id,
        "spec": spec,
        "semantics": semantics,
        "algorithm": algorithm,
        "mode": "online",
        "n_samples": n_samples,
        "m_runs": m,
        "avg_total_s": avg_total,
        "std_total_s": std_total,
        "avg_per_sample_s": avg_per_sample,
        "std_per_sample_s": std_per_sample,
        "avg_per_sample_us": avg_per_sample * 1e6,
        "std_per_sample_us": std_per_sample * 1e6,
        "temporal_depth": temporal_depth,
        "run_times": run_times,
    }


def should_process_formula(spec: str, semantics: str) -> bool:
    """Check if formula should be processed for given semantics.

    For RoSI semantics, only process formulas with temporal bounds <= 1000.
    """
    if semantics != "Rosi":
        return True

    # Extract temporal bound from spec (e.g., "G[0,500]" -> 500)
    import re

    match = re.search(r"\[0,(\d+(?:\.\d+)?)\]", spec)
    if match:
        bound = float(match.group(1))
        return bound <= 1000

    # If no temporal operator with bound, include the formula
    return True


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Benchmark mstlo_python monitors")
    parser.add_argument(
        "--signal-csv", default=DEFAULT_SIGNAL_CSV, help="Input signal CSV path"
    )
    parser.add_argument(
        "--m-runs", type=int, default=DEFAULT_M, help="Number of runs to average"
    )
    parser.add_argument("--output", default=DEFAULT_OUTPUT_CSV, help="Output CSV path")
    parser.add_argument(
        "--output-raw",
        default=DEFAULT_OUTPUT_RAW_CSV,
        help="Output raw per-run CSV path",
    )
    parser.add_argument(
        "--overwrite", action="store_true", help="Overwrite output CSV if it exists"
    )
    parser.add_argument(
        "--warmup-runs",
        type=int,
        default=1,
        help="Untimed warmup runs before measurement",
    )
    return parser.parse_args()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
def main() -> None:
    args = parse_args()

    signal = load_signal(args.signal_csv)
    print(f"Loaded signal with {len(signal)} samples from {args.signal_csv}")
    print(f"Averaging over M = {args.m_runs} runs (+ {args.warmup_runs} warmup)\n")

    semantics = [
        "DelayedQuantitative",
        "DelayedQualitative",
        "EagerQualitative",
        "Rosi",
    ]
    algorithm = "Incremental"

    # Initialize CSV file with headers
    out_path = args.output
    raw_out_path = args.output_raw
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    if os.path.exists(out_path) and not args.overwrite:
        raise SystemExit(
            f"Output file already exists: {out_path}. Use --overwrite or set --output to a new path."
        )

    csv_file = open(out_path, "w", newline="")
    fieldnames = [
        "formula_id",
        "spec",
        "semantics",
        "algorithm",
        "mode",
        "n_samples",
        "m_runs",
        "avg_total_s",
        "std_total_s",
        "avg_per_sample_s",
        "std_per_sample_s",
        "avg_per_sample_us",
        "std_per_sample_us",
        "temporal_depth",
    ]
    writer = csv.DictWriter(csv_file, fieldnames=fieldnames)
    writer.writeheader()

    raw_csv_file = open(raw_out_path, "w", newline="")
    raw_fieldnames = [
        "formula_id",
        "spec",
        "semantics",
        "algorithm",
        "mode",
        "n_samples",
        "temporal_depth",
        "run_id",
        "total_s",
        "per_sample_s",
        "per_sample_us",
    ]
    raw_writer = csv.DictWriter(raw_csv_file, fieldnames=raw_fieldnames)
    raw_writer.writeheader()

    for sem in semantics:
        print(f"\n--- Semantics: {sem} ---")
        pbar = tqdm(FORMULAS, desc=f"Processing {sem}")
        for fid, spec in pbar:
            if not should_process_formula(spec, sem):
                continue
            pbar.set_postfix({"fid": fid, "spec": spec[:30] + "..."})
            res = bench_formula(
                fid,
                spec,
                signal,
                args.m_runs,
                semantics=sem,
                algorithm=algorithm,
                warmup_runs=args.warmup_runs,
            )
            run_times = res.pop("run_times")
            writer.writerow(res)
            csv_file.flush()
            for run_id, t in enumerate(run_times):
                per_sample_s = t / res["n_samples"]
                raw_writer.writerow(
                    {
                        "formula_id": res["formula_id"],
                        "spec": res["spec"],
                        "semantics": res["semantics"],
                        "algorithm": res["algorithm"],
                        "mode": res["mode"],
                        "n_samples": res["n_samples"],
                        "temporal_depth": res["temporal_depth"],
                        "run_id": run_id,
                        "total_s": t,
                        "per_sample_s": per_sample_s,
                        "per_sample_us": per_sample_s * 1e6,
                    }
                )
            raw_csv_file.flush()

    csv_file.close()
    raw_csv_file.close()
    print(f"\nResults saved to {out_path}")
    print(f"Raw results saved to {raw_out_path}")


if __name__ == "__main__":
    main()
