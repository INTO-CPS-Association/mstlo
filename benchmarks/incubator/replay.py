"""Run the recorded signal through the monitors and save the verdicts.

The experiment gathers the plant signal and measures what each `update()` call
costs; producing the verdicts is left to this script, so a long collection does
not also have to write them while it runs.

The monitors are deterministic in the sequence of samples they are fed: the
same monitors, built the same way, fed the same signal in the same order give
the same verdicts.  Note that the replay covers the whole session while the
live monitors were only fed `normal` and `lid_open` (see run_experiment.py), so
the verdicts over those two phases are the ones the live monitors produced and
the earlier ones are what they would have produced had they been running.

    python replay.py
"""

import argparse
import csv
import os

import pandas as pd

from stl_monitors import build_monitors

VERDICT_FIELDS = ["emitted_at", "spec", "t", "robustness"]

# Empty: the whole session, so the monitors see one uninterrupted signal.
# Skipping a phase in the middle would leave a gap that the temporal operators
# would read as elapsed time rather than as missing samples.  process_results.py,
# benchmark.py and the native benchmark default to the same, so every stage sees
# the recording as it was made; narrow one and you must narrow all four.
MONITORED_PHASES = ()


def replay(datadir, outdir, phases=MONITORED_PHASES):
    signal = pd.read_csv(os.path.join(datadir, "signal.csv"))
    if "phase" in signal.columns and phases:
        signal = signal[signal["phase"].isin(phases)].reset_index(drop=True)
    monitors = build_monitors(max_T=signal["max_T"].iloc[0],
                              min_T=signal["min_T"].iloc[0])

    os.makedirs(outdir, exist_ok=True)
    path = os.path.join(outdir, "verdicts.csv")
    emitted = 0

    with open(path, "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=VERDICT_FIELDS)
        writer.writeheader()

        for row in signal.itertuples():
            params = {"max_T": row.max_T, "min_T": row.min_T}
            for monitor in monitors:
                value = params[monitor.param_name]
                for t, robustness, _ in monitor.update(row.t, row.temperature,
                                                       value):
                    writer.writerow({"emitted_at": row.t, "spec": monitor.name,
                                     "t": t, "robustness": robustness})
                    emitted += 1

    print(f"{len(signal)} samples -> {emitted} verdicts -> {path}")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--datadir", default="data")
    parser.add_argument("--outdir", default="data")
    parser.add_argument("--phases", nargs="*", default=list(MONITORED_PHASES),
                        help="phases to replay; empty replays the whole session")
    args = parser.parse_args()
    replay(args.datadir, args.outdir, tuple(args.phases))


if __name__ == "__main__":
    main()
