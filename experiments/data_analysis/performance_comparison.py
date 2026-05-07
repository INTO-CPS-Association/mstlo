import argparse
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
from sklearn.linear_model import LinearRegression

# ── Academic style ────────────────────────────────────────────────────────────
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

# Default tuned for single-column page layout where the figure should span most
# of the horizontal text area without becoming cluttered.
FIG_SIZE = (6.6, 4.2)

FORMULA_OPERATOR = {4: "U", 5: "G", 6: "F"}


def _build_plot_dataframe(
    df: pd.DataFrame, fg_mode: str
) -> tuple[pd.DataFrame, list[str]]:
    """Build plot dataframe according to F/G mode selection."""
    df_u = df[df["operator"] == "U"].copy()

    if fg_mode == "average":
        df_fg = (
            df[df["operator"].isin(["F", "G"])]
            .groupby(["semantics", "interval_len"], as_index=False)["avg_per_sample_us"]
            .mean()
        )
        df_fg["operator"] = "F/G"
        return pd.concat([df_u, df_fg], ignore_index=True), ["U", "F/G"]

    if fg_mode == "both":
        df_fg = df[df["operator"].isin(["F", "G"])].copy()
        return pd.concat([df_u, df_fg], ignore_index=True), ["U", "F", "G"]

    if fg_mode == "eventually":
        df_f = df[df["operator"] == "F"].copy()
        return pd.concat([df_u, df_f], ignore_index=True), ["U", "F"]

    # global
    df_g = df[df["operator"] == "G"].copy()
    return pd.concat([df_u, df_g], ignore_index=True), ["U", "G"]


def _build_fit_params(
    fits_orig: pd.DataFrame,
    df_plot: pd.DataFrame,
    operators_to_plot: list[str],
    fg_mode: str,
) -> dict[tuple[str, str], dict | pd.Series]:
    """Build fit parameter lookup keyed by (semantics, operator)."""
    fit_params: dict[tuple[str, str], dict | pd.Series] = {}

    for _, row in fits_orig[fits_orig["operator"] == "U"].iterrows():
        fit_params[(row["semantics"], "U")] = row

    if fg_mode == "average":
        df_fg = df_plot[df_plot["operator"] == "F/G"].copy()
        for sem in df_plot["semantics"].unique():
            fg_rows = fits_orig[
                (fits_orig["semantics"] == sem)
                & (fits_orig["operator"].isin(["F", "G"]))
            ]
            model_names = fg_rows["model_name"].unique()

            g_avg = df_fg[df_fg["semantics"] == sem].sort_values("interval_len")
            x_raw = g_avg["interval_len"].values
            y = g_avg["avg_per_sample_us"].values

            if len(model_names) == 1 and model_names[0] == "constant":
                fit_params[(sem, "F/G")] = {
                    "model_name": "constant",
                    "intercept": fg_rows["intercept"].mean(),
                    "coef_b": 0.0,
                    "coef_b2": 0.0,
                }
            else:
                reg = LinearRegression().fit(x_raw.reshape(-1, 1), y)
                fit_params[(sem, "F/G")] = {
                    "model_name": "linear",
                    "intercept": reg.intercept_,
                    "coef_b": reg.coef_[0],
                    "coef_b2": 0.0,
                }
        return fit_params

    fg_ops = [op for op in operators_to_plot if op in {"F", "G"}]
    for _, row in fits_orig[fits_orig["operator"].isin(fg_ops)].iterrows():
        fit_params[(row["semantics"], row["operator"])] = row

    return fit_params


def parse_args() -> argparse.Namespace:
    root = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description="Create final runtime regression plot")
    parser.add_argument(
        "--benchmark-csv",
        type=Path,
        default=root.parent / "results" / "paper_native_benchmark_results_final.csv",
        help="Path to native benchmark CSV",
    )
    parser.add_argument(
        "--regression-csv",
        type=Path,
        default=root / "regression_fit_results.csv",
        help="Path to regression-fit CSV",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=root / "final_plot.pdf",
        help="Output plot path",
    )
    parser.add_argument(
        "--fg-mode",
        choices=["global", "eventually", "average", "both"],
        default="global",
        help=(
            "How to handle formulas F/G: "
            "'global' shows only G, "
            "'eventually' shows only F, "
            "'average' shows F/G average, "
            "'both' shows F and G separately"
        ),
    )
    parser.add_argument(
        "--fig-width",
        type=float,
        default=FIG_SIZE[0],
        help="Figure width in inches (default tuned for one-column page-width fit)",
    )
    parser.add_argument(
        "--fig-height",
        type=float,
        default=FIG_SIZE[1],
        help="Figure height in inches",
    )
    parser.add_argument(
        "--plot-std",
        action="store_true",
        default=False,
        help="Overlay ±1 std deviation band around each series",
    )
    parser.add_argument(
        "--plot-operators",
        nargs="+",
        choices=["U", "F", "G", "F/G"],
        default=None,
        metavar="OP",
        help="Operators to plot (default: all determined by --fg-mode). Choices: U F G F/G",
    )
    parser.add_argument(
        "--plot-semantics",
        nargs="+",
        choices=["delqual", "delquant", "eagerqual", "rosi"],
        default=None,
        metavar="SEM",
        help=(
            "Semantics to plot (default: all). "
            "Choices: delqual (DelayedQualitative), delquant (DelayedQuantitative), "
            "eagerqual (EagerQualitative), rosi (Rosi)"
        ),
    )
    parser.add_argument(
        "--log-scale",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Use log scale on y-axis (default: true)",
    )
    return parser.parse_args()


def eval_fit(p, x):
    return p["intercept"] + p["coef_b"] * x + p["coef_b2"] * x**2


def adjust_label_positions(y_values, y_min, y_max, min_frac=0.10, log_scale=True):
    """Spread label y positions on the axis to reduce overlaps.

    min_frac: minimum separation between adjacent labels as a fraction of the
              total axis span (measured in log-decades or linear units depending
              on log_scale).
    """
    if len(y_values) == 0:
        return np.array([])

    arr = np.asarray(y_values, dtype=float)
    if log_scale:
        arr = np.log10(arr)
        lo, hi = np.log10(float(y_min)), np.log10(float(y_max))
    else:
        lo, hi = float(y_min), float(y_max)

    span = hi - lo
    min_delta = min_frac * span
    margin = span * 0.02

    order = np.argsort(arr)
    pos = arr[order].copy()
    lo_m, hi_m = lo + margin, hi - margin

    for i in range(1, len(pos)):
        pos[i] = max(pos[i], pos[i - 1] + min_delta)

    if len(pos) > 0 and pos[-1] > hi_m:
        pos[-1] = hi_m
        for i in range(len(pos) - 2, -1, -1):
            pos[i] = min(pos[i], pos[i + 1] - min_delta)

    if len(pos) > 0 and pos[0] < lo_m:
        pos[0] = lo_m
        for i in range(1, len(pos)):
            pos[i] = max(pos[i], pos[i - 1] + min_delta)

    adjusted = np.empty_like(arr)
    adjusted[order] = pos
    return 10**adjusted if log_scale else adjusted


def main() -> None:
    args = parse_args()

    df = pd.read_csv(args.benchmark_csv)
    df = df[df["formula_id"].isin(FORMULA_OPERATOR)].copy()
    df["operator"] = df["formula_id"].map(FORMULA_OPERATOR)

    fits_orig = pd.read_csv(args.regression_csv)
    fits_orig = fits_orig[fits_orig["source"] == "native"]

    SEMANTICS_ALIASES = {
        "delqual": "DelayedQualitative",
        "delquant": "DelayedQuantitative",
        "eagerqual": "EagerQualitative",
        "rosi": "Rosi",
    }

    df_plot, operators_to_plot = _build_plot_dataframe(df, args.fg_mode)

    if args.plot_operators is not None:
        df_plot = df_plot[df_plot["operator"].isin(args.plot_operators)]
        operators_to_plot = [op for op in operators_to_plot if op in args.plot_operators]

    if args.plot_semantics is not None:
        requested = {SEMANTICS_ALIASES[s] for s in args.plot_semantics}
        df_plot = df_plot[df_plot["semantics"].isin(requested)]

    fit_params = _build_fit_params(fits_orig, df_plot, operators_to_plot, args.fg_mode)

    semantics_colors = {
        "DelayedQuantitative": "#64baaa",
        "DelayedQualitative": "#6bae48",
        "EagerQualitative": "#e52740",
        "Rosi": "#3c0701",
    }
    semantics_display = {
        "DelayedQuantitative": "Del. Quant.",
        "DelayedQualitative": "Del. Qual.",
        "EagerQualitative": "Eager Qual.",
        "Rosi": "RoSI",
    }
    operator_markers = {"U": "^", "F": "o", "G": "s", "F/G": "D"}
    operator_display = {"U": "U", "F": "F", "G": "G", "F/G": "F/G"}

    fig, ax = plt.subplots(figsize=(args.fig_width, args.fig_height))
    direct_labels = []

    has_std = "std_per_sample_us" in df_plot.columns

    for (semantics, operator), group in df_plot.groupby(["semantics", "operator"]):
        g = group.sort_values("interval_len")
        color = semantics_colors[semantics]
        marker = operator_markers[operator]
        fit = fit_params.get((semantics, operator))

        if args.plot_std and has_std:
            ax.fill_between(
                g["interval_len"],
                g["avg_per_sample_us"] - g["std_per_sample_us"],
                g["avg_per_sample_us"] + g["std_per_sample_us"],
                color=color,
                alpha=0.15,
                zorder=1,
            )

        ax.scatter(
            g["interval_len"],
            g["avg_per_sample_us"],
            color=color,
            marker=marker,
            s=40,
            zorder=3,
            linewidths=0.7,
            edgecolors="white",
            alpha=0.9,
        )

        if fit is not None:
            x_min, x_max = g["interval_len"].min(), g["interval_len"].max()
            if fit["model_name"] == "constant":
                ax.plot(
                    [x_min, x_max],
                    [fit["intercept"], fit["intercept"]],
                    color=color,
                    linewidth=1.8,
                    linestyle=":",
                    alpha=0.6,
                    zorder=2,
                )
            else:
                x_fit = np.linspace(x_min, x_max, 500)
                y_fit = eval_fit(fit, x_fit)
                y_data_min = g["avg_per_sample_us"].min()
                mask = y_fit >= y_data_min
                if mask.any():
                    ax.plot(
                        x_fit[mask],
                        y_fit[mask],
                        color=color,
                        linewidth=1.8,
                        linestyle=":",
                        alpha=0.6,
                        zorder=2,
                    )

        x_last = g["interval_len"].iloc[-1]
        y_last = g["avg_per_sample_us"].iloc[-1]
        direct_labels.append(
            {
                "x": x_last,
                "y": y_last,
                "semantics": semantics,
                "operator": operator,
                "label": f"{semantics_display[semantics]}, {operator_display[operator]}",
                "color": color,
            }
        )

    if args.log_scale:
        ax.set_yscale("log")

    ax.set_xlabel("Temporal upper bound ($b$)", labelpad=5)
    y_label = "Average time per sample (\u00b5s, log scale)" if args.log_scale else "Average time per sample (\u00b5s)"
    ax.set_ylabel(y_label, labelpad=5)
    # ax.set_title("Performance scaling of temporal operators", pad=5)
    ax.grid(True, which="major", linestyle="--", linewidth=0.8, alpha=0.55)
    if args.log_scale:
        ax.grid(True, which="minor", linestyle=":", linewidth=0.6, alpha=0.4)
    ax.tick_params(which="both", top=True, right=True, width=1.1)

    x_data_max = df_plot["interval_len"].max()
    x_label = x_data_max * 1.015
    x_right = x_data_max * 1.35
    ax.set_xlim(right=x_right)

    # Force matplotlib to finalise autoscaling so get_ylim() returns the real limits.
    fig.canvas.draw()
    y_min, y_max = ax.get_ylim()

    # Spread non-RoSI labels per operator to avoid near-overlapping stacks.
    y_targets = {i: info["y"] for i, info in enumerate(direct_labels)}
    non_rosi_indices = [
        i for i, info in enumerate(direct_labels) if info["semantics"] != "RoSI"
    ]
    operators = sorted({direct_labels[i]["operator"] for i in non_rosi_indices})
    for op in operators:
        idx = [i for i in non_rosi_indices if direct_labels[i]["operator"] == op]
        if not idx:
            continue
        y_vals = [direct_labels[i]["y"] for i in idx]
        min_frac = 0.13 if op == "G" else 0.10
        adjusted = adjust_label_positions(
            y_vals, y_min, y_max, min_frac=min_frac, log_scale=args.log_scale
        )
        for i, y_adj in zip(idx, adjusted):
            y_targets[i] = y_adj

    for i, label_info in enumerate(direct_labels):
        y_target = y_targets[i]
        if label_info["semantics"] == "Rosi":
            ax.annotate(
                label_info["label"],
                xy=(label_info["x"], label_info["y"]),
                xytext=(5, 0),
                textcoords="offset points",
                color=label_info["color"],
                fontsize=11,
                va="center",
                ha="left",
                clip_on=False,
                bbox={
                    "boxstyle": "round,pad=0.1",
                    "fc": "white",
                    "ec": "none",
                    "alpha": 0.75,
                },
            )
        else:
            ax.annotate(
                label_info["label"],
                xy=(label_info["x"], label_info["y"]),
                xytext=(x_label, y_target),
                textcoords="data",
                color=label_info["color"],
                fontsize=11,
                va="center",
                ha="left",
                clip_on=False,
                bbox={
                    "boxstyle": "round,pad=0.1",
                    "fc": "white",
                    "ec": "none",
                    "alpha": 0.75,
                },
                arrowprops={
                    "arrowstyle": "-",
                    "lw": 1.0,
                    "color": label_info["color"],
                    "alpha": 0.8,
                    "shrinkA": 0,
                    "shrinkB": 0,
                },
            )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    plt.tight_layout()
    plt.savefig(args.output, dpi=600, bbox_inches="tight")
    print(f"Plot saved successfully to: {args.output}")


if __name__ == "__main__":
    main()
