"""Plot the monitors' memory footprint over time for phi1..phi4.

    python memory_profile.py --csv <memory_profile_N=20000.csv>

Reads the per-step series written by the `memory_profile_benchmark` Rust bench
and draws one panel per formula, with the four semantics as lines: time on x,
the monitor's `total_size()` on y.
"""

import argparse
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd
from matplotlib.lines import Line2D

# House style, matching the other scripts in this directory.
plt.rcParams.update(
    {
        "font.family": "serif",
        "font.size": 11,
        "axes.labelsize": 11,
        "axes.titlesize": 9,
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
    }
)

FIG_SIZE = (6.6, 4.6)

# Colours match performance_comparison.py so a semantics keeps its identity
# across the figures in the paper.
SEMANTICS = {
    "DelayedQuantitative": ("#64baaa", "Del. Quant.", "-"),
    "DelayedQualitative": ("#6bae48", "Del. Qual.", (0, (5, 1.5))),
    "EagerQualitative": ("#e52740", "Eager Qual.", (0, (1.5, 1.5))),
    "Rosi": ("#3c0701", "RoSI", (0, (4, 1.5, 1, 1.5))),
}

TITLES = {
    1: r"$\varphi_1:\ (x<0.5) \wedge (x>-0.5)$",
    2: r"$\varphi_2:\ G_{[0,1000]}\,\left((x>0.5) \rightarrow F_{[0,100]}\,(x<0)\right)$",
    3: r"$\varphi_3:\ (x<0.5)\ U_{[0,1000]}\ (x<0)$",
    4: r"$\varphi_4:\ G_{[0,100]}(x<0.5) \vee G_{[100,150]}(x>0)$",
}

GRID = {"linestyle": "--", "linewidth": 0.8, "alpha": 0.55}


def plot_formula(ax, rows, formula_id):
    """One panel: every semantics recorded for this formula."""
    for semantics, (color, _, dashes) in SEMANTICS.items():
        series = rows[rows["semantics"] == semantics]
        if series.empty:
            continue
        ax.plot(
            series["t"],
            series["total_size_bytes"] / 1024,
            color=color,
            ls=dashes,
            lw=1.3,
            zorder=3,
        )

    ax.set_title(TITLES.get(formula_id, rf"$\varphi_{{{formula_id}}}$"), pad=4)
    ax.set_ylim(0, 1.15 * rows["total_size_bytes"].max() / 1024)
    ax.set_xlim(rows["t"].min(), rows["t"].max())
    ax.grid(True, which="major", **GRID)
    ax.tick_params(which="both", top=True, right=True, width=1.1)


def make_figure(df):
    fig, axes = plt.subplots(2, 2, figsize=FIG_SIZE, sharex=True)

    for ax, formula_id in zip(axes.flat, sorted(df["formula_id"].unique())):
        plot_formula(ax, df[df["formula_id"] == formula_id], formula_id)

    for ax in axes[:, 0]:
        ax.set_ylabel("memory [KiB]", labelpad=2)
    for ax in axes[-1, :]:
        ax.set_xlabel("time [s]", labelpad=2)
    fig.align_ylabels(axes[:, 0])

    handles = [
        Line2D([], [], color=color, ls=dashes, lw=1.3, label=label)
        for color, label, dashes in SEMANTICS.values()
    ]
    fig.legend(
        handles=handles,
        loc="lower center",
        ncol=4,
        frameon=False,
        handlelength=2.2,
        columnspacing=1.2,
        handletextpad=0.5,
        bbox_to_anchor=(0.5, 0.005),
    )
    fig.tight_layout(pad=0.4, h_pad=0.8, w_pad=1.0, rect=(0, 0.07, 1, 1))
    return fig


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--csv",
        default="../../../mstlo/benches/results/memory_profile_N=20000.csv",
        help="per-step series from the memory_profile_benchmark bench",
    )
    parser.add_argument("--output", default="memory_profile.pdf")
    args = parser.parse_args()

    df = pd.read_csv(args.csv)
    peak = df.groupby(["formula_id", "semantics"])["total_size_bytes"].max()
    print(f"{len(df)} samples, peak footprint [KiB]:")
    print((peak / 1024).round(1).to_string())

    out = Path(args.output)
    out.parent.mkdir(parents=True, exist_ok=True)
    fig = make_figure(df)
    for path in (out, out.with_suffix(".png")):
        fig.savefig(path, dpi=600, bbox_inches="tight")
        print(f"wrote {path}")
    plt.close(fig)


if __name__ == "__main__":
    main()
