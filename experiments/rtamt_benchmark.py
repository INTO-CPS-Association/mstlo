"""
Comprehensive benchmark suite for rtamt monitoring.

Tests formula families with varying temporal bounds and records performance metrics,
matching the structure and approach of ostl_results.py.
"""

import csv
import math
import time
import os
import argparse
from tqdm import tqdm
import numpy as np
import rtamt

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
DEFAULT_M = 50  # Number of runs to average over
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
    "rtamt_benchmark_results_cpp.csv",
)
DEFAULT_OUTPUT_RAW_CSV = os.path.join(
    os.path.dirname(__file__),
    "results",
    "rtamt_benchmark_results_cpp_raw.csv",
)

# Generate formulas matching the ostl benchmark catalog
# Uses the same structure: globally, eventually, until with varying bounds
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
for bnd in b[:11]:  # limit to 1, 100, ..., 1000 for until
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
    with open(path, newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            signal.append((float(row["timestep"]), float(row["value"])))
    return signal


def make_discrete_online_monitor_cpp(
    spec: str,
) -> rtamt.StlDiscreteTimeOnlineSpecificationCpp:
    """Create a fresh rtamt discrete-time online monitor using C++ backend."""
    monitor = rtamt.StlDiscreteTimeOnlineSpecificationCpp()
    monitor.declare_var("x", "float")
    monitor.spec = spec
    monitor.parse()
    monitor.pastify()
    return monitor


def make_discrete_online_monitor_python(
    spec: str,
) -> rtamt.StlDiscreteTimeOnlineSpecification:
    """Create a fresh rtamt discrete-time online monitor for online monitoring."""
    monitor = rtamt.StlDiscreteTimeOnlineSpecification()
    monitor.declare_var("x", "float")
    monitor.spec = spec
    monitor.parse()
    monitor.pastify()
    return monitor


def make_dense_online_monitor_python(
    spec: str,
) -> rtamt.StlDenseTimeOnlineSpecification:
    """Create a fresh rtamt dense-time online monitor for online monitoring."""
    monitor = rtamt.StlDenseTimeOnlineSpecification()
    monitor.declare_var("x", "float")
    monitor.spec = spec
    monitor.parse()
    monitor.pastify()
    return monitor


def make_discrete_offline_monitor_python(
    spec: str,
) -> rtamt.StlDiscreteTimeOfflineSpecification:
    """Create a fresh rtamt discrete-time offline monitor for offline monitoring."""
    monitor = rtamt.StlDiscreteTimeOfflineSpecification()
    monitor.declare_var("x", "float")
    monitor.spec = spec
    monitor.parse()
    return monitor


def make_dense_offline_monitor_python(
    spec: str,
) -> rtamt.StlDenseTimeOfflineSpecification:
    """Create a fresh rtamt dense-time offline monitor for offline monitoring."""
    monitor = rtamt.StlDenseTimeOfflineSpecification()
    monitor.declare_var("x", "float")
    monitor.spec = spec
    monitor.parse()
    return monitor


def bench_online_monitor(
    fid: int,
    spec: str,
    signal: list[tuple[float, float]],
    m: int,
    monitor_func: callable,
    warmup_runs: int = 1,
) -> dict:
    """Benchmark discrete-time online monitoring (one sample at a time)."""
    try:
        n_samples = len(signal)
        total_time = 0.0
        run_times: list[float] = []

        for i in range(-warmup_runs, m):
            monitor = monitor_func(spec)
            t0 = time.perf_counter()
            for ts, val in signal:
                monitor.update(ts, [("x", val)])
            t1 = time.perf_counter()
            if i >= 0:
                elapsed = t1 - t0
                run_times.append(elapsed)
                total_time += elapsed

        avg_total = total_time / m
        avg_per_sample = avg_total / n_samples
        std_total = (
            math.sqrt(sum((t - avg_total) ** 2 for t in run_times) / (m - 1))
            if m > 1
            else 0.0
        )
        std_per_sample = std_total / n_samples

        return {
            "formula_id": fid,
            "spec": spec,
            "monitor_type": monitor_func.__name__,
            "mode": "online",
            "n_samples": n_samples,
            "m_runs": m,
            "avg_total_s": avg_total,
            "std_total_s": std_total,
            "avg_per_sample_s": avg_per_sample,
            "std_per_sample_s": std_per_sample,
            "avg_per_sample_us": avg_per_sample * 1e6,
            "std_per_sample_us": std_per_sample * 1e6,
            "run_times": run_times,
        }
    except Exception as e:
        print(f"ERROR: Formula {fid} failed: {str(e)[:100]}")
        return None


def bench_dense_online_monitor(
    fid: int,
    spec: str,
    signal: list[tuple[float, float]],
    m: int,
    monitor_func: callable,
    warmup_runs: int = 1,
) -> dict:
    """Benchmark dense-time online monitoring (full signal per update call)."""
    try:
        n_samples = len(signal)
        total_time = 0.0

        for i in range(-warmup_runs, m):
            monitor = monitor_func(spec)
            t0 = time.perf_counter()
            monitor.update(["x", signal])
            t1 = time.perf_counter()
            if i >= 0:
                total_time += t1 - t0

        avg_total = total_time / m
        avg_per_sample = avg_total / n_samples

        return {
            "formula_id": fid,
            "spec": spec,
            "monitor_type": monitor_func.__name__,
            "mode": "online",
            "n_samples": n_samples,
            "m_runs": m,
            "avg_total_s": avg_total,
            "avg_per_sample_s": avg_per_sample,
            "avg_per_sample_us": avg_per_sample * 1e6,
        }
    except Exception as e:
        print(f"ERROR: Formula {fid} failed: {str(e)[:100]}")
        return None


def bench_offline_monitor(
    fid: int,
    spec: str,
    signal: list[list[float, float]],
    m: int,
    monitor_func: callable,
    warmup_runs: int = 1,
) -> dict:
    """Benchmark dense-time offline monitoring."""
    try:
        n_samples = len(signal)
        total_time = 0.0

        for i in range(-warmup_runs, m):
            monitor = monitor_func(spec)
            t0 = time.perf_counter()
            monitor.evaluate(["x", signal])
            t1 = time.perf_counter()
            if i >= 0:
                total_time += t1 - t0

        avg_total = total_time / m
        avg_per_sample = avg_total / n_samples

        return {
            "formula_id": fid,
            "spec": spec,
            "monitor_type": monitor_func.__name__,
            "mode": "offline",
            "n_samples": n_samples,
            "m_runs": m,
            "avg_total_s": avg_total,
            "avg_per_sample_s": avg_per_sample,
            "avg_per_sample_us": avg_per_sample * 1e6,
        }
    except Exception as e:
        print(
            f"ERROR: Formula {fid} failed for {monitor_func.__name__}: {str(e)[:100]}"
        )
        return None


def bench_discrete_offline_monitor(
    fid: int,
    spec: str,
    signal: list[tuple[float, float]],
    m: int,
    monitor_func: callable,
    warmup_runs: int = 1,
) -> dict:
    """Benchmark discrete-time offline monitoring."""
    try:
        n_samples = len(signal)
        total_time = 0.0

        times = [ts for ts, _ in signal]
        values = [v for _, v in signal]
        dataset = {"time": times, "x": values}

        for i in range(-warmup_runs, m):
            monitor = monitor_func(spec)
            t0 = time.perf_counter()
            monitor.evaluate(dataset)
            t1 = time.perf_counter()
            if i >= 0:
                total_time += t1 - t0

        avg_total = total_time / m
        avg_per_sample = avg_total / n_samples

        return {
            "formula_id": fid,
            "spec": spec,
            "monitor_type": monitor_func.__name__,
            "mode": "offline",
            "n_samples": n_samples,
            "m_runs": m,
            "avg_total_s": avg_total,
            "avg_per_sample_s": avg_per_sample,
            "avg_per_sample_us": avg_per_sample * 1e6,
        }
    except Exception as e:
        print(
            f"ERROR: Formula {fid} failed for {monitor_func.__name__}: {str(e)[:100]}"
        )
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
        default=5,
        help="Untimed warmup runs before measurement",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()

    signal = load_signal(args.signal_csv)
    print(f"Loaded signal with {len(signal)} samples from {args.signal_csv}")
    print(f"Averaging over M = {args.m_runs} runs (+ {args.warmup_runs} warmup)\n")

    # Initialize CSV file with headers
    out_path = args.output
    raw_out_path = args.output_raw
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
        "std_total_s",
        "avg_per_sample_s",
        "std_per_sample_s",
        "avg_per_sample_us",
        "std_per_sample_us",
    ]
    writer = csv.DictWriter(csv_file, fieldnames=fieldnames)
    writer.writeheader()

    raw_csv_file = open(raw_out_path, "w", newline="", encoding="utf-8")
    raw_fieldnames = [
        "formula_id",
        "spec",
        "monitor_type",
        "mode",
        "n_samples",
        "run_id",
        "total_s",
        "per_sample_s",
        "per_sample_us",
    ]
    raw_writer = csv.DictWriter(raw_csv_file, fieldnames=raw_fieldnames)
    raw_writer.writeheader()

    # --- Discrete-Time Online (C++ Backend) ---
    print("\nDiscrete-Time — Online (C++ Backend)")
    pbar = tqdm(FORMULAS, desc="Discrete-Time Online C++")
    for fid, spec in pbar:
        pbar.set_postfix({"fid": fid, "spec": spec[:40] + "..."})
        res = bench_online_monitor(
            fid,
            spec,
            signal,
            args.m_runs,
            make_discrete_online_monitor_cpp,
            warmup_runs=args.warmup_runs,
        )
        if res is not None:
            res["monitor_type"] = "discrete-time-cpp"
            run_times = res.pop("run_times")
            writer.writerow(res)
            csv_file.flush()
            for run_id, t in enumerate(run_times):
                per_sample_s = t / res["n_samples"]
                raw_writer.writerow(
                    {
                        "formula_id": res["formula_id"],
                        "spec": res["spec"],
                        "monitor_type": res["monitor_type"],
                        "mode": res["mode"],
                        "n_samples": res["n_samples"],
                        "run_id": run_id,
                        "total_s": t,
                        "per_sample_s": per_sample_s,
                        "per_sample_us": per_sample_s * 1e6,
                    }
                )
            raw_csv_file.flush()

    # # --- Discrete-Time Online ---
    # print("\nDiscrete-Time — Online")
    # pbar = tqdm(FORMULAS, desc="Discrete-Time Online")
    # for fid, spec in pbar:
    #     pbar.set_postfix({"fid": fid, "spec": spec[:40] + "..."})
    #     res = bench_online_monitor(
    #         fid,
    #         spec,
    #         signal,
    #         args.m_runs,
    #         make_discrete_online_monitor_python,
    #         warmup_runs=args.warmup_runs,
    #     )
    #     if res is not None:
    #         writer.writerow(res)
    #         csv_file.flush()

    # # --- Dense-Time Online ---
    # print("\nDense-Time — Online")
    # pbar = tqdm(FORMULAS, desc="Dense-Time Online")
    # for fid, spec in pbar:
    #     pbar.set_postfix({"fid": fid, "spec": spec[:40] + "..."})
    #     res = bench_dense_online_monitor(
    #         fid,
    #         spec,
    #         signal,
    #         args.m_runs,
    #         make_dense_online_monitor_python,
    #         warmup_runs=args.warmup_runs,
    #     )
    #     if res is not None:
    #         writer.writerow(res)
    #         csv_file.flush()

    # # --- Dense-Time Offline ---
    # print("\nDense-Time — Offline")
    # pbar = tqdm(FORMULAS, desc="Dense-Time Offline")
    # for fid, spec in pbar:
    #     pbar.set_postfix({"fid": fid, "spec": spec[:40] + "..."})
    #     res = bench_offline_monitor(
    #         fid,
    #         spec,
    #         signal,
    #         args.m_runs,
    #         make_dense_offline_monitor_python,
    #         warmup_runs=args.warmup_runs,
    #     )
    #     if res is not None:
    #         writer.writerow(res)
    #         csv_file.flush()

    # # --- Discrete-Time Offline ---
    # print("\nDiscrete-Time — Offline")
    # pbar = tqdm(FORMULAS, desc="Discrete-Time Offline")
    # for fid, spec in pbar:
    #     pbar.set_postfix({"fid": fid, "spec": spec[:40] + "..."})
    #     res = bench_discrete_offline_monitor(
    #         fid,
    #         spec,
    #         signal,
    #         args.m_runs,
    #         make_discrete_offline_monitor_python,
    #         warmup_runs=args.warmup_runs,
    #     )
    #     if res is not None:
    #         writer.writerow(res)
    #         csv_file.flush()

    csv_file.close()
    raw_csv_file.close()
    print(f"\nResults saved to {out_path}")
    print(f"Raw results saved to {raw_out_path}")


if __name__ == "__main__":
    main()
