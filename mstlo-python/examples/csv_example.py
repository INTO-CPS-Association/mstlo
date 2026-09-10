"""Monitor a trace loaded from a CSV file against a property loaded from a file.

Nothing about the signals is hard-coded: the CSV header names them, and the
property refers to them by those names.

    data/trace.csv          data/property.stl
    time,temperature,...    G[0, 2](temperature < 30.0) && (pressure > 99.0)
    0.0,21.4,101.1
    ...

Run with: `python examples/csv_example.py`
"""

from pathlib import Path

import pandas as pd

import mstlo_python as mstlo

DATA = Path(__file__).parent / "data"


def read_trace(path):
    """Parse a `time,<signal>,..` CSV into (signal, value, timestamp) steps."""
    frame = pd.read_csv(path)

    # Column names are the signal names, so melting the wide table gives exactly
    # the (signal, value, timestamp) rows the monitor wants. melt groups by
    # column, so sort back into chronological order — a monitor is fed a
    # stream, and the sort is stable, so samples sharing a timestamp keep their
    # column order.
    tidy = frame.melt(id_vars="time", var_name="signal", value_name="value")
    tidy = tidy.sort_values("time", kind="stable")
    return [
        (row.signal, float(row.value), float(row.time))
        for row in tidy.itertuples(index=False)
    ]


trace = read_trace(DATA / "trace.csv")
print(f"Read {len(trace)} steps from CSV")

# The property is text too, so it is parsed at runtime: "the temperature stays
# below 30 while the pressure holds above 99."
formula = mstlo.parse_formula((DATA / "property.stl").read_text().strip())
print(f"Monitoring: {formula}")

# Multi-signal formulas need each signal to have an initial value, which the
# monitor holds from t=0 until the signal's first real sample arrives. Derive
# those from the trace itself so nothing has to be hard-coded.
init_signals = {}
for signal, value, _timestamp in trace:
    init_signals.setdefault(signal, value)

monitor = mstlo.Monitor(
    formula, semantics="DelayedQuantitative", init_signals=init_signals
)

# Feed the parsed steps to the monitor in order.
for signal, value, timestamp in trace:
    for verdict_time, robustness in monitor.update(signal, value, timestamp).verdicts():
        print(f"t={verdict_time:>4.1f}s: {robustness:>6.2f}")

# The whole trace can also go in as one batch. `update_batch` accepts either the
# flat form of (signal, value, timestamp) tuples — exactly what read_trace
# already returns — or a signal-major dict {signal: [(value, timestamp), ..]}.
# Steps are sorted by timestamp, so a trace read out of order still evaluates
# chronologically.
#
# For this property the two forms produce identical verdicts. They can differ
# under semantics="Rosi", where a batch collapses the refinements of one
# timestamp into its final value, whereas the loop above reports each
# refinement as it happens.
#
# The reset() is only needed to run this *after* the loop above, which has
# already advanced the monitor past the end of the trace; replacing the loop
# outright needs only the two lines that follow it.
#
# monitor.reset()
# for verdict_time, robustness in monitor.update_batch(trace).verdicts():
#     print(f"t={verdict_time:>4.1f}s: {robustness:>6.2f}")
