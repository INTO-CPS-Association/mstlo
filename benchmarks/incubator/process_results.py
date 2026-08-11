"""Join the plant samples with the verdicts into one dataset.

Delayed quantitative semantics emits one final robustness per window, once the
temporal depth has elapsed, so this is a straight join: every sample gets the
verdict for the window starting at it, or nothing if that window has not closed
by the end of the recording.

    python process_results.py
"""

import argparse
import os

import pandas as pd

SPECS = ("phi_hi", "phi_lo")

# Empty: the whole session, matching replay.py.  A phase kept here but not
# replayed would carry no verdict, and one replayed but dropped here would not
# be plottable.
MONITORED_PHASES = ()

FIELDS = ["t", "phase", "timestamp_ns", "temperature", "heater_on", "lid_open",
          "max_T", "min_T", "phi_hi", "phi_hi_at", "phi_lo", "phi_lo_at"]


def process(datadir, phases=MONITORED_PHASES):
    signal = pd.read_csv(os.path.join(datadir, "signal.csv"))
    verdicts = pd.read_csv(os.path.join(datadir, "verdicts.csv"))
    if "phase" in signal.columns and phases:
        signal = signal[signal["phase"].isin(phases)].reset_index(drop=True)
    else:
        signal["phase"] = signal.get("phase", "")

    out = signal.set_index("t")
    for spec in SPECS:
        rows = verdicts[verdicts["spec"] == spec].set_index("t")
        out[spec] = rows["robustness"]
        out[f"{spec}_at"] = rows["emitted_at"]
    out = out.reset_index()

    path = os.path.join(datadir, "dataset.csv")
    out.to_csv(path, index=False, columns=FIELDS)
    report(out, path)


def report(df, path):
    print(f"{len(df)} samples -> {path}")
    print(f"temperature range: {df['temperature'].min():.2f} .. "
          f"{df['temperature'].max():.2f} C")

    for spec in SPECS:
        known = df[df[spec].notna()]
        violated = known[known[spec] < 0]
        if violated.empty:
            print(f"{spec}: SATISFIED -- all {len(known)} verdicts positive, "
                  f"tightest margin {known[spec].min():.2f}")
        else:
            first = violated.iloc[0]
            print(f"{spec}: VIOLATED -- {len(violated)}/{len(known)} verdicts "
                  f"negative, worst {known[spec].min():.2f}; first violation "
                  f"at t = {first['t']:.0f} s, reported at "
                  f"t = {first[f'{spec}_at']:.0f} s")
        print(f"  {len(df) - len(known)} samples have no verdict yet "
              "(their window has not closed)")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--datadir", default="data")
    parser.add_argument("--phases", nargs="*", default=list(MONITORED_PHASES),
                        help="phases to keep; empty keeps the whole session")
    args = parser.parse_args()
    process(args.datadir, tuple(args.phases))


if __name__ == "__main__":
    main()
