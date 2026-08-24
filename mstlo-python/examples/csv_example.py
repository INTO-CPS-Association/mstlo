"""Monitor a trace loaded from a CSV file against a property loaded from a file.

Nothing about the signals is hard-coded: the CSV header names them, and the
property refers to them by those names.

    data/trace.csv          data/property.stl
    time,temperature,...    G[0, 1](temperature < 30.0) && (pressure > 99.0)
    0.0,21.4,101.1
    ...

Run with: `python examples/csv_example.py`
"""

import csv
from pathlib import Path

import mstlo_python as mstlo

DATA = Path(__file__).parent / "data"


def read_trace(path):
    """Parse a `time,<signal>,..` CSV into (signal, value, timestamp) steps."""
    with open(path, newline="") as file:
        # DictReader takes the signal names straight from the header row.
        reader = csv.DictReader(file)
        return [
            (signal, float(value), float(row["time"]))
            for row in reader
            for signal, value in row.items()
            if signal != "time"
        ]


trace = read_trace(DATA / "trace.csv")
print(f"Read {len(trace)} steps from CSV")

# The property is text too, so it is parsed at runtime: "the temperature stays
# below 30 while the pressure holds above 99."
formula = mstlo.parse_formula((DATA / "property.stl").read_text().strip())
print(f"Monitoring: {formula}")

monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

# Feed the parsed steps to the monitor in order.
for signal, value, timestamp in trace:
    for verdict_time, robustness in monitor.update(signal, value, timestamp).verdicts():
        print(f"t={verdict_time:>4.1f}s: {robustness:>6.2f}")
