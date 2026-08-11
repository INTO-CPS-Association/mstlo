"""Plot the recorded run: temperature, heater, and the robustness of both specs.

    python plot_results.py                     # the whole run
    python plot_results.py --start 300 --end 1800

Writes two figures, both sized to sit at 0.49\\textwidth in LLNCS, where
\\textwidth is about 4.8in: drawing at 3.3in means it is scaled by ~0.7, putting
the 11pt house font at a readable ~7.7pt on the page.

  <name>.pdf         temperature and heater over the robustness of both specs
  <name>_memory.pdf  the monitors' memory footprint over the same time axis

Each specification is named at its own line rather than in a legend, and the
transparent red half-plane below zero carries the sign, which is what lets both
specifications share one panel.  Under
delayed quantitative semantics a verdict is a single final number, so the
samples whose window has not closed simply have none and are not drawn.

The footprint is sampled after every `update()` by the native benchmark, which
is the only place `total_size()` is exposed, so the memory figure is skipped
until `cargo bench --bench incubator_benchmark` has written its series.
"""

import argparse
import os

import matplotlib.pyplot as plt
import pandas as pd

# House style, matching mstlo/experiments/data_analysis.
plt.rcParams.update({
    "font.family": "serif",
    "font.size": 11,
    "axes.labelsize": 11,
    "axes.linewidth": 1.2,
    "xtick.labelsize": 10,
    "ytick.labelsize": 10,
    "xtick.direction": "in",
    "ytick.direction": "in",
    "xtick.major.width": 1.1,
    "ytick.major.width": 1.1,
    "xtick.major.size": 4.5,
    "ytick.major.size": 4.5,
    "legend.fontsize": 10,
    "mathtext.fontset": "cm",
    "figure.dpi": 150,
})

# One size for both figures, saved as-is: a tight bounding box would crop each
# canvas to its own content, and the two would then no longer share an aspect
# ratio, so side by side at 0.49\textwidth they would differ in height.
FIG_SIZE = (4, 2.9)

MEMORY_CSV = "benchmark_rust_memory.csv"

SPECS = {"phi_hi": ("#9b59b6", r"$\varphi_{hi}$"),
         "phi_lo": ("#1b7f79", r"$\varphi_{lo}$")}

# The specs are named where their lines run, so nothing has to be matched back
# to a legend box; that only works if the name is comfortably readable once the
# figure is scaled down to 0.49\textwidth, hence the size.
LABEL_SIZE = 14

TEMP_COLOR = "#3976af"
HEATER_COLOR = "#e07b39"
BAND_COLOR = "#857c89"
VIOL_COLOR = "#c0392b"

GRID = {"linestyle": "--", "linewidth": 0.8, "alpha": 0.55}


def style_axis(ax):
    ax.grid(True, which="major", **GRID)
    ax.tick_params(which="both", top=True, right=True, width=1.1)


def label_spec(ax, spec, x, y, dy):
    """Name a line at (x, y), offset by `dy` points clear of the trace."""
    color, label = SPECS[spec]
    ax.annotate(label, xy=(x, y), xytext=(0, dy), textcoords="offset points",
                color=color, fontsize=LABEL_SIZE, ha="right",
                va="bottom" if dy > 0 else "top", zorder=5)


def last_sample(t, values):
    """The last point that was actually drawn, which is where the label goes."""
    drawn = values.notna()
    return t[drawn].iloc[-1], values[drawn].iloc[-1]


def plot_temperature(ax, df):
    """Temperature with the controller band, plus the heater on a right axis."""
    # The first sample of the session predates the controller's first state
    # message, so its thresholds are blank; they are constant afterwards.
    max_t, min_t = df["max_T"].dropna().iloc[0], df["min_T"].dropna().iloc[0]

    ax.plot(df["t"], df["temperature"], color=TEMP_COLOR, lw=1.3, zorder=3)
    for value in (max_t, min_t):
        ax.axhline(value, color=BAND_COLOR, ls="--", lw=1.0, zorder=2)

    lo = min(df["temperature"].min(), min_t)
    hi = max(df["temperature"].max(), max_t)
    pad = 0.06 * (hi - lo)
    ax.set_ylim(lo - pad, hi + pad)

    # The thresholds are read off the axis itself; in-plot labels collide with
    # the trace at this figure size.
    ticks = [v for v in range(int((lo - pad) // 5) * 5, int(hi + pad) + 1, 5)
             if v < min_t - 1.5]
    ax.set_yticks(sorted(set(ticks + [min_t, max_t])))
    ax.set_ylabel(r"$T$ [$^\circ$C]", labelpad=2)
    style_axis(ax)

    # The heater spans the full right axis, so it reads as a background square
    # wave behind the temperature rather than as a second trace.
    heater = ax.twinx()
    heater.fill_between(df["t"], 0, df["heater_on"], step="post",
                        color=HEATER_COLOR, alpha=0.08, lw=0, zorder=1)
    heater.step(df["t"], df["heater_on"], where="post", color=HEATER_COLOR,
                lw=0.8, alpha=0.65, zorder=1)
    heater.set_ylim(-0.04, 1.04)
    heater.set_yticks([0, 1])
    heater.set_yticklabels(["off", "on"], fontsize=8)
    heater.set_ylabel("heater", color=HEATER_COLOR, labelpad=1, fontsize=9)
    heater.tick_params(axis="y", colors=HEATER_COLOR, direction="in",
                       width=1.1, pad=1)
    heater.spines["right"].set_color(HEATER_COLOR)


def plot_robustness(ax, df):
    """Both specifications, with the violating half-plane shaded."""
    values = pd.concat([df[spec] for spec in SPECS]).dropna()
    lo, hi = min(values.min(), 0.0), max(values.max(), 0.0)
    pad = 0.10 * (hi - lo)

    ax.axhspan(lo - pad, 0.0, color=VIOL_COLOR, alpha=0.10, lw=0, zorder=0)
    ax.axhline(0.0, color="black", lw=0.9, zorder=2)
    for spec, (color, _) in SPECS.items():
        ax.plot(df["t"], df[spec], color=color, lw=1.5, zorder=3)

    # Extra headroom at the top so the label above the upper trace has somewhere
    # to sit; the traces separate by the end of the run, so naming them there
    # keeps both names clear of everything else.
    ax.set_ylim(lo - pad, hi + 2.4 * pad)
    ax.set_ylabel(r"$\rho$", labelpad=2)
    for spec in SPECS:
        t, value = last_sample(df["t"], df[spec])
        label_spec(ax, spec, t, value, 3)
    style_axis(ax)


def plot_memory(ax, mem):
    """Each monitor's total footprint, one line per specification.

    Drawn from zero rather than from the first sample: what the figure is about
    is how far the footprint grows before it settles, and a zoomed baseline
    would make a bounded plateau look like unbounded growth.
    """
    # The two plateaus end within a kilobyte of each other, so the names are
    # pushed apart rather than both being set above their line.
    offsets = {"phi_hi": -4, "phi_lo": 4}
    for spec, (color, _) in SPECS.items():
        rows = mem[mem["spec_name"] == spec]
        size = rows["total_size_bytes"] / 1024
        ax.plot(rows["t"], size, color=color, lw=1.5, zorder=3)
        label_spec(ax, spec, *last_sample(rows["t"], size), offsets[spec])

    # Headroom above the plateau, so the "lid opened" label has clear space to
    # sit in instead of landing on top of the lines.
    ax.set_ylim(0, 1.32 * mem["total_size_bytes"].max() / 1024)
    ax.set_ylabel("memory [KiB]", labelpad=2)
    style_axis(ax)


def mark_lid_open(df, axes, label_ax=None):
    """The one event in the run, drawn identically on every figure."""
    if not df["lid_open"].any():
        return
    lid_t = df.loc[df["lid_open"] == 1, "t"].iloc[0]
    for ax in axes:
        ax.axvline(lid_t, color="black", ls=(0, (4, 3)), lw=1.1, zorder=4)
    if label_ax is not None:
        label_ax.annotate("lid opened", xy=(lid_t, 1.0),
                          xycoords=("data", "axes fraction"), xytext=(3, -11),
                          textcoords="offset points", fontsize=8, va="top")


def pack(fig):
    """Squeeze the margins without letting anything spill off the canvas.

    The figures are saved at their nominal size rather than cropped to their
    content, so that the two land in LaTeX with the same aspect ratio and hence
    the same height; that leaves the layout engine, not the bounding box, to
    keep the labels on the page.
    """
    fig.get_layout_engine().set(w_pad=0.01, h_pad=0.01, hspace=0.02, wspace=0.0)


def save(fig, figdir, stem):
    os.makedirs(figdir, exist_ok=True)
    for fmt in ("pdf", "png"):
        out = os.path.join(figdir, f"{stem}.{fmt}")
        fig.savefig(out, dpi=600)
        print(f"wrote {out}")
    plt.close(fig)


def run_figure(df):
    """Temperature over robustness, sharing the time axis."""
    fig, (top, bottom) = plt.subplots(2, 1, figsize=FIG_SIZE, sharex=True,
                                      layout="constrained")
    plot_temperature(top, df)
    plot_robustness(bottom, df)
    mark_lid_open(df, (top, bottom), label_ax=top)

    bottom.set_xlabel("time [s]", labelpad=2)
    fig.align_ylabels((top, bottom))
    pack(fig)
    return fig


def memory_figure(mem, df):
    """The monitors' footprint on the same time axis as the run figure."""
    fig, ax = plt.subplots(figsize=FIG_SIZE, layout="constrained")
    plot_memory(ax, mem)
    mark_lid_open(df, (ax,), label_ax=ax)

    ax.set_xlabel("time [s]", labelpad=2)
    pack(fig)
    return fig


def load_memory(datadir, start, end):
    """The per-step footprint series, or None if it has not been recorded.

    `total_size()` is only reachable from Rust, so this comes out of the native
    benchmark rather than the replay; it is legitimately absent on a checkout
    where that benchmark has not been run.
    """
    path = os.path.join(datadir, MEMORY_CSV)
    if not os.path.exists(path):
        return None

    mem = pd.read_csv(path)
    if start is not None:
        mem = mem[mem["t"] >= start]
    if end is not None:
        mem = mem[mem["t"] <= end]
    return mem.reset_index(drop=True) if not mem.empty else None


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--datadir", default="data")
    parser.add_argument("--figdir", default="figures")
    parser.add_argument("--name", default="incubator", help="output file stem")
    parser.add_argument("--start", type=float, default=None,
                        help="first time to plot (default: start of the run)")
    parser.add_argument("--end", type=float, default=None,
                        help="last time to plot (default: end of the run)")
    args = parser.parse_args()

    df = pd.read_csv(os.path.join(args.datadir, "dataset.csv"))
    if args.start is not None:
        df = df[df["t"] >= args.start]
    if args.end is not None:
        df = df[df["t"] <= args.end]
    df = df.reset_index(drop=True)
    print(f"plotting {len(df)} samples, t = {df['t'].iloc[0]:.0f} .. "
          f"{df['t'].iloc[-1]:.0f} s")

    save(run_figure(df), args.figdir, args.name)

    mem = load_memory(args.datadir, args.start, args.end)
    if mem is None:
        print(f"no {MEMORY_CSV} in {args.datadir} -- skipping the memory figure "
              "(run the native incubator benchmark to record it)")
        return

    peak = mem.groupby("spec_name")["total_size_bytes"].max()
    print("peak monitor footprint: "
          + ", ".join(f"{spec} {value / 1024:.1f} KiB"
                      for spec, value in peak.items()))
    save(memory_figure(mem, df), args.figdir, f"{args.name}_memory")


if __name__ == "__main__":
    main()
