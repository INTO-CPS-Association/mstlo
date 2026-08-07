"""Time mstlo against RTAMT on the recorded signal.

Run with the `stl` environment -- RTAMT's C++ backend is only installed there:

    /path/to/envs/stl/bin/python benchmark.py

Three monitors, all fed one sample per `update()` call so the per-update cost
is comparable:

  * mstlo, delayed quantitative semantics -- the deployed monitor
  * RTAMT discrete-time online, C++ backend, told the 3 s sampling period
  * RTAMT dense-time online

`--no-rtamt` times mstlo alone, which also drops the `rtamt` import, so the
benchmark runs in an environment without it installed.

Structured after ``mstlo/experiments/rtamt_benchmark.py``: a fresh monitor per
run, untimed warmup runs first, and per-sample statistics over M runs.
Aggregates pool the per-run measurements -- one observation per (scenario,
formula, run) -- so every run counts equally regardless of trace length.
"""

import argparse
import csv
import math
import os
import time

import pandas as pd

import mstlo_python as mstlo
from stl_monitors import EVENTUALLY_WINDOW, GLOBALLY_WINDOW, SEMANTICS

THRESHOLDS = {"hi": ("max_T", 39.0), "lo": ("min_T", 36.0)}

SUMMARY_FIELDS = [
    "tool", "spec_name", "spec", "mode", "n_samples", "m_runs",
    "avg_total_s", "std_total_s", "avg_per_sample_s", "std_per_sample_s",
    "avg_per_sample_us", "std_per_sample_us",
]
RAW_FIELDS = ["tool", "spec_name", "run_id", "total_s", "per_sample_s",
              "per_sample_us"]


def specs(direction, a=GLOBALLY_WINDOW, b=EVENTUALLY_WINDOW):
    """The same requirement written for mstlo and for RTAMT.

    The controller thresholds are constant in these recordings, so RTAMT gets
    them inlined where mstlo carries them as runtime variables.
    """
    param, value = THRESHOLDS[direction]
    ante, cons = (">=", "<=") if direction == "hi" else ("<=", ">=")
    return (f"G[0,{a}]((T {ante} ${param}) -> F[0,{b}](T {cons} ${param}))",
            f"always[0,{a}]((T {ante} {value}) implies "
            f"(eventually[0,{b}](T {cons} {value})))",
            param, value)


def time_mstlo(signal, spec, param, value, m_runs, warmup_runs):
    parsed = mstlo.parse_formula(spec)
    rows = list(zip(signal["t"].to_numpy(), signal["temperature"].to_numpy()))
    totals = []

    for run in range(-warmup_runs, m_runs):
        variables = mstlo.Variables()
        variables.set(param, value)
        monitor = mstlo.Monitor(formula=parsed, semantics=SEMANTICS,
                                variables=variables)
        started = time.perf_counter()
        for t, temperature in rows:
            monitor.update("T", temperature, t)
        elapsed = time.perf_counter() - started
        if run >= 0:
            totals.append(elapsed)

    return totals


def time_rtamt_discrete(signal, spec, period, m_runs, warmup_runs):
    import rtamt

    rows = list(zip(signal["t"].to_numpy(), signal["temperature"].to_numpy()))
    totals = []

    for run in range(-warmup_runs, m_runs):
        monitor = rtamt.StlDiscreteTimeOnlineSpecificationCpp()
        monitor.declare_var("T", "float")
        monitor.set_sampling_period(period, "s", 0.1)
        monitor.spec = spec
        monitor.parse()
        monitor.pastify()

        started = time.perf_counter()
        for t, temperature in rows:
            monitor.update(t, [("T", temperature)])
        elapsed = time.perf_counter() - started
        if run >= 0:
            totals.append(elapsed)

    return totals


def time_rtamt_dense(signal, spec, m_runs, warmup_runs):
    import rtamt

    rows = list(zip(signal["t"].to_numpy(), signal["temperature"].to_numpy()))
    totals = []

    for run in range(-warmup_runs, m_runs):
        monitor = rtamt.StlDenseTimeOnlineSpecification()
        monitor.declare_var("T", "float")
        monitor.spec = spec
        monitor.parse()
        monitor.pastify()

        started = time.perf_counter()
        for t, temperature in rows:
            monitor.update(["T", [(t, temperature)]])
        elapsed = time.perf_counter() - started
        if run >= 0:
            totals.append(elapsed)

    return totals


def mean_sd(values):
    mean = sum(values) / len(values)
    if len(values) < 2:
        return mean, 0.0
    return mean, math.sqrt(sum((x - mean) ** 2 for x in values) / (len(values) - 1))


def summarise(tool, spec_name, spec, totals, n_samples, m_runs):
    per_sample_us = [total / n_samples * 1e6 for total in totals]
    avg_total, std_total = mean_sd(totals)
    avg_us, std_us = mean_sd(per_sample_us)
    return {
        "tool": tool, "spec_name": spec_name,
        "spec": spec, "mode": "online", "n_samples": n_samples,
        "m_runs": m_runs,
        "avg_total_s": avg_total, "std_total_s": std_total,
        "avg_per_sample_s": avg_us / 1e6, "std_per_sample_s": std_us / 1e6,
        "avg_per_sample_us": avg_us, "std_per_sample_us": std_us,
    }, per_sample_us


def aggregate(tool, members, pooled):
    avg_us, std_us = mean_sd(pooled)
    return {
        "tool": tool, "spec_name": "aggregate",
        "spec": f"pooled over {len(pooled)} runs", "mode": "online",
        "n_samples": sum(m["n_samples"] for m in members),
        "m_runs": len(pooled),
        "avg_total_s": sum(m["avg_total_s"] for m in members) / len(members),
        "std_total_s": sum(m["std_total_s"] for m in members) / len(members),
        "avg_per_sample_s": avg_us / 1e6, "std_per_sample_s": std_us / 1e6,
        "avg_per_sample_us": avg_us, "std_per_sample_us": std_us,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--datadir", default="data")
    parser.add_argument("--outdir", default="data")
    parser.add_argument("--m-runs", type=int, default=50)
    parser.add_argument("--warmup-runs", type=int, default=1)
    parser.add_argument("--phases", nargs="*", default=[],
                        help="phases to benchmark; empty uses the whole session")
    parser.add_argument("--no-rtamt", action="store_true",
                        help="time mstlo only, leaving RTAMT out entirely")
    args = parser.parse_args()

    signal = pd.read_csv(os.path.join(args.datadir, "signal.csv"))
    # The whole session by default, the same samples replay.py feeds the
    # monitors and the native benchmark times.
    if "phase" in signal.columns and args.phases:
        signal = signal[signal["phase"].isin(args.phases)].reset_index(drop=True)
        signal["t"] = signal["t"] - signal["t"].iloc[0]
    # RTAMT's discrete-time monitor requires a uniform sampling period that
    # divides the operator bounds exactly.  The emulator publishes every 3 s but
    # the recorded timestamps carry a few ms of jitter, so the nominal period is
    # used and the jitter is absorbed by RTAMT's sampling tolerance.
    measured_period = float(signal["t"].diff().median())
    period = round(measured_period)
    if not args.no_rtamt and (GLOBALLY_WINDOW % period
                              or EVENTUALLY_WINDOW % period):
        raise SystemExit(f"bounds {GLOBALLY_WINDOW}/{EVENTUALLY_WINDOW} are not "
                         f"multiples of the {period} s sampling period")

    n = len(signal)
    tools = "mstlo only" if args.no_rtamt else "mstlo and RTAMT"
    print(f"{n} samples, {measured_period:.3f} s apart (nominal {period:.0f} s), "
          f"{args.m_runs} timed runs (+{args.warmup_runs} warmup), {tools}\n")

    summaries, raw, pooled = [], [], {}
    for direction in ("hi", "lo"):
        name = f"phi_{direction}"
        mstlo_spec, rtamt_spec, param, value = specs(direction)

        measured = {
            "mstlo": (mstlo_spec,
                      time_mstlo(signal, mstlo_spec, param, value,
                                 args.m_runs, args.warmup_runs)),
        }
        if not args.no_rtamt:
            measured["rtamt-discrete-cpp"] = (
                rtamt_spec,
                time_rtamt_discrete(signal, rtamt_spec, period, args.m_runs,
                                    args.warmup_runs))
            measured["rtamt-dense"] = (
                rtamt_spec,
                time_rtamt_dense(signal, rtamt_spec, args.m_runs,
                                 args.warmup_runs))

        for tool, (spec, totals) in measured.items():
            row, per_sample_us = summarise(tool, name, spec, totals, n,
                                           args.m_runs)
            summaries.append(row)
            pooled.setdefault(tool, []).extend(per_sample_us)
            print(f"  {tool:20s} {name}  {row['avg_per_sample_us']:8.2f} +- "
                  f"{row['std_per_sample_us']:.2f} us per update")

            for run_id, total in enumerate(totals):
                raw.append({"tool": tool, "spec_name": name, "run_id": run_id,
                            "total_s": total, "per_sample_s": total / n,
                            "per_sample_us": total / n * 1e6})

    print("\naggregate over both formulas:")
    for tool, values in pooled.items():
        members = [s for s in summaries if s["tool"] == tool]
        row = aggregate(tool, members, values)
        summaries.append(row)
        print(f"  {tool:20s} {row['avg_per_sample_us']:8.2f} +- "
              f"{row['std_per_sample_us']:.2f} us per update "
              f"({row['m_runs']} runs)")

    os.makedirs(args.outdir, exist_ok=True)
    summary_path = os.path.join(args.outdir, "benchmark.csv")
    with open(summary_path, "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=SUMMARY_FIELDS)
        writer.writeheader()
        writer.writerows(summaries)

    runs_path = os.path.join(args.outdir, "benchmark_runs.csv")
    pd.DataFrame(raw).to_csv(runs_path, index=False, columns=RAW_FIELDS)

    print(f"\nwrote {summary_path}       (per tool and formula, plus aggregates)")
    print(f"wrote {runs_path}  (raw per-run totals)")


if __name__ == "__main__":
    main()
