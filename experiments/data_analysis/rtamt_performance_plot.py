import argparse
import re
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd

plt.rcParams.update(
    {
        "font.family": "serif",
        "font.size": 11,
        "axes.labelsize": 11,
        "axes.titlesize": 12,
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
        "legend.title_fontsize": 8,
        "figure.dpi": 150,
    }
)

FIG_SIZE = (6.6, 4.2)

FORMULA_OPERATOR = {5: "U", 6: "G", 7: "F"}
OPERATOR_MARKERS = {"G": "s", "F": "o", "U": "^"}

SEMANTICS_COLORS = {
    "dense-time-python-online": "#3976af",
    "dense-time-python-offline": "#5ba85a",
    "discrete-time-cpp-online": "#e07b39",
    "discrete-time-python-online": "#857c89",
    "discrete-time-python-offline": "#9b59b6",
}

DENSE_TIME_SEMANTICS = {"dense-time-python-online", "dense-time-python-offline"}


def _extract_interval_len(spec: str) -> float:
    m = re.search(r"\[0,(\d+(?:\.\d+)?)\]", spec)
    return float(m.group(1)) if m else 0.0


def parse_args() -> argparse.Namespace:
    root = Path(__file__).resolve().parent
    all_semantics = list(SEMANTICS_COLORS)
    parser = argparse.ArgumentParser(
        description="Plot RTAMT performance scaling with b"
    )
    parser.add_argument(
        "--benchmark-csv",
        type=Path,
        default=root.parent / "results" / "rtamt_benchmark_results_dense.csv",
    )
    parser.add_argument("--output", type=Path, default=root / "rtamt_performance.pdf")
    parser.add_argument("--fig-width", type=float, default=FIG_SIZE[0])
    parser.add_argument("--fig-height", type=float, default=FIG_SIZE[1])
    parser.add_argument("--plot-std", action="store_true", default=False)
    parser.add_argument(
        "--semantics",
        nargs="+",
        choices=all_semantics,
        default=all_semantics,
        metavar="SEM",
        help=f"Semantics to plot (default: all). Choices: {all_semantics}",
    )
    parser.add_argument(
        "--operators",
        nargs="+",
        choices=list(OPERATOR_MARKERS.keys()),
        default=list(OPERATOR_MARKERS.keys()),
        metavar="OP",
        help=(
            f"Operators to plot (default: all). Choices: {list(OPERATOR_MARKERS.keys())}"
        ),
    )
    parser.add_argument(
        "--log-scale",
        action=argparse.BooleanOptionalAction,
        default=False,
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()

    df = pd.read_csv(args.benchmark_csv)
    df = df[df["formula_id"].isin(FORMULA_OPERATOR)].copy()
    df["operator"] = df["formula_id"].map(FORMULA_OPERATOR)
    df["semantics"] = df["monitor_type"] + "-" + df["mode"]
    df["interval_len"] = df["spec"].apply(_extract_interval_len)

    df = df[df["semantics"].isin(args.semantics)]
    df = df[df["operator"].isin(args.operators)]
    # dense-time does not support Until
    df = df[~((df["semantics"].isin(DENSE_TIME_SEMANTICS)) & (df["operator"] == "U"))]

    has_std = "std_per_sample_us" in df.columns
    _, ax = plt.subplots(figsize=(args.fig_width, args.fig_height))

    for (semantics, operator), group in df.groupby(["semantics", "operator"]):
        g = group.sort_values("interval_len")
        color = SEMANTICS_COLORS[semantics]
        marker = OPERATOR_MARKERS[operator]
        label = f"{semantics} ({operator})"

        ax.plot(
            g["interval_len"],
            g["avg_per_sample_us"],
            color=color,
            linewidth=1.6,
            zorder=2,
        )
        ax.scatter(
            g["interval_len"],
            g["avg_per_sample_us"],
            color=color,
            marker=marker,
            s=35,
            zorder=3,
            linewidths=0.7,
            edgecolors="white",
            alpha=0.9,
            label=label,
        )

        if args.plot_std and has_std:
            ax.fill_between(
                g["interval_len"],
                g["avg_per_sample_us"] - g["std_per_sample_us"],
                g["avg_per_sample_us"] + g["std_per_sample_us"],
                color=color,
                alpha=0.15,
                zorder=1,
            )

    if args.log_scale:
        ax.set_yscale("log")

    ax.set_xlabel("Temporal upper bound ($b$)", labelpad=5)
    ax.set_ylabel(
        (
            "Average time per sample (µs, log scale)"
            if args.log_scale
            else "Average time per sample (µs)"
        ),
        labelpad=5,
    )
    ax.grid(True, which="major", linestyle="--", linewidth=0.8, alpha=0.55)
    ax.tick_params(which="both", top=True, right=True, width=1.1)
    ax.legend(title="Semantics (operator)", framealpha=0.85)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    plt.tight_layout()
    plt.savefig(args.output, dpi=600, bbox_inches="tight")
    print(f"Plot saved successfully to: {args.output}")


if __name__ == "__main__":
    main()
