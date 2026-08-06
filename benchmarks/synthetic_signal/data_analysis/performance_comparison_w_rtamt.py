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

FORMULA_OPERATOR = {5: "U", 6: "G", 7: "F"}

# RTAMT overlays are held in a separate namespace so their rows never collide with
# ours, but they implement delayed quantitative semantics -- the same as our
# DelayedQuantitative series -- and are coloured accordingly. Each entry maps a CLI
# name onto the (monitor_type, mode) pair identifying it in the RTAMT benchmark CSV,
# plus the name used on its direct label.
RTAMT_SEMANTICS = "DelayedQuantitative"
RTAMT_VARIANTS = {
    "dense-online": ("dense-time-python", "online", "RTAMT dense"),
    "discrete-online": ("discrete-time-python", "online", "RTAMT discrete"),
    "discrete-cpp-online": ("discrete-time-cpp", "online", "RTAMT C++"),
    "dense-offline": ("dense-time-python", "offline", "RTAMT dense off."),
    "discrete-offline": ("discrete-time-python", "offline", "RTAMT discrete off."),
}


def _rtamt_semantics(variant: str) -> str:
    """Namespaced semantics key so RTAMT rows never collide with our own."""
    return f"RTAMT:{variant}"


def _load_rtamt(path: Path, variants: list[str]) -> pd.DataFrame:
    """Load the requested RTAMT variants into benchmark-CSV shape.

    Not every variant implements every operator: dense-time RTAMT has no Until at
    all, and discrete-time only has Until for the smaller bounds. Missing rows are
    simply absent, so those series stop early rather than being special-cased.
    """
    df = pd.read_csv(path)
    df = df[df["formula_id"].isin(FORMULA_OPERATOR)].copy()
    df["operator"] = df["formula_id"].map(FORMULA_OPERATOR)
    df["interval_len"] = (
        df["spec"].str.extract(r"\[0,(\d+(?:\.\d+)?)\]", expand=False).astype(float)
    )

    frames = []
    for variant in variants:
        monitor_type, mode, _ = RTAMT_VARIANTS[variant]
        sub = df[(df["monitor_type"] == monitor_type) & (df["mode"] == mode)].copy()
        sub["semantics"] = _rtamt_semantics(variant)
        frames.append(sub)
    return pd.concat(frames, ignore_index=True)


def _rtamt_fits(fits_all: pd.DataFrame, variants: list[str]) -> pd.DataFrame:
    """Re-key the RTAMT regression fits onto our namespaced semantics labels.

    The fit table identifies RTAMT rows by monitor_type alone, with no mode column,
    so the online and offline variants of a monitor share a fit.
    """
    fits = fits_all[fits_all["source"] == "rtamt"]
    frames = []
    for variant in variants:
        monitor_type, _, _ = RTAMT_VARIANTS[variant]
        rows = fits[fits["semantics"] == monitor_type].copy()
        rows["semantics"] = _rtamt_semantics(variant)
        frames.append(rows)
    return pd.concat(frames, ignore_index=True) if frames else fits.iloc[:0]


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
        "--rtamt-csv",
        type=Path,
        default=None,
        help=(
            "Path to the RTAMT benchmark CSV. When given, the variants selected by "
            "--rtamt-variants are overlaid in grey as baselines, following --fg-mode."
        ),
    )
    parser.add_argument(
        "--rtamt-variants",
        nargs="+",
        choices=list(RTAMT_VARIANTS),
        default=["dense-online", "discrete-cpp-online"],
        metavar="VARIANT",
        help=(
            "RTAMT variants to overlay, requires --rtamt-csv "
            "(default: dense-online discrete-cpp-online). "
            f"Choices: {' '.join(RTAMT_VARIANTS)}"
        ),
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
    parser.add_argument(
        "--log-x",
        action=argparse.BooleanOptionalAction,
        default=False,
        help="Use log scale on x-axis too, i.e. a log-log plot (default: false)",
    )
    parser.add_argument(
        "--x-min",
        type=float,
        default=None,
        help=(
            "Drop points with interval_len < this value, curves included. Useful with "
            "--log-x, where the sparsely sampled low-b region is stretched over "
            "several empty decades (e.g. --x-min 100)."
        ),
    )
    return parser.parse_args()


def _fit_labels_inside(fig, ax, annotations, log_x, pad_px=4.0, max_passes=5):
    """Grow the right x-limit until every direct label sits inside the axes.

    Labels are drawn with clip_on=False and anchored just past the last data point,
    so how far they reach depends on the rendered text width -- which no fixed
    padding can predict. Measure it and widen the data range to match; the text
    stays at a fixed data x, so each pass moves it further from the spine.
    """
    for _ in range(max_passes):
        fig.canvas.draw()
        axes_x1 = ax.get_window_extent().x1
        label_x1 = max((a.get_window_extent().x1 for a in annotations), default=axes_x1)
        overflow = label_x1 - axes_x1 + pad_px
        if overflow <= 0:
            return

        lo, hi = ax.get_xlim()
        if log_x:
            lo, hi = np.log10(lo), np.log10(hi)
        # Convert the pixel overflow into the same units the limits are held in.
        hi += overflow / ax.get_window_extent().width * (hi - lo)
        ax.set_xlim(right=10**hi if log_x else hi)


def _axis_fraction(x, lo, hi, log_x):
    """Where x sits along the axis, as a 0-1 fraction of the drawn span."""
    if log_x:
        lo, hi, x = np.log10(lo), np.log10(hi), np.log10(x)
    return 0.0 if hi == lo else (x - lo) / (hi - lo)


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

    fits_all = pd.read_csv(args.regression_csv)
    fits_orig = fits_all[fits_all["source"] == "native"]

    SEMANTICS_ALIASES = {
        "delqual": "DelayedQualitative",
        "delquant": "DelayedQuantitative",
        "eagerqual": "EagerQualitative",
        "rosi": "Rosi",
    }

    df_plot, operators_to_plot = _build_plot_dataframe(df, args.fg_mode)

    rtamt_variants = args.rtamt_variants if args.rtamt_csv is not None else []
    rtamt_semantics = {_rtamt_semantics(v) for v in rtamt_variants}

    if rtamt_variants:
        # Same fg-mode collapsing as our own results, so the F/G averaging in
        # particular stays consistent between the two sources.
        df_rtamt, _ = _build_plot_dataframe(
            _load_rtamt(args.rtamt_csv, rtamt_variants), args.fg_mode
        )
        df_plot = pd.concat([df_plot, df_rtamt], ignore_index=True)
        # Feed the RTAMT fits through the same lookup, so the baselines get the same
        # regression treatment (including the F/G averaging) as our own series.
        fits_orig = pd.concat(
            [fits_orig, _rtamt_fits(fits_all, rtamt_variants)], ignore_index=True
        )

    if args.plot_operators is not None:
        df_plot = df_plot[df_plot["operator"].isin(args.plot_operators)]
        operators_to_plot = [
            op for op in operators_to_plot if op in args.plot_operators
        ]

    if args.plot_semantics is not None:
        # RTAMT is a baseline, not one of our semantics, so --plot-semantics does
        # not filter it out; drop --rtamt-csv to hide it.
        requested = {
            SEMANTICS_ALIASES[s] for s in args.plot_semantics
        } | rtamt_semantics
        df_plot = df_plot[df_plot["semantics"].isin(requested)]

    if args.x_min is not None:
        df_plot = df_plot[df_plot["interval_len"] >= args.x_min]

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
    for variant in rtamt_variants:
        _, _, name = RTAMT_VARIANTS[variant]
        # RTAMT implements delayed quantitative semantics, so it shares that colour;
        # the implementation is encoded by marker fill and line style instead.
        semantics_colors[_rtamt_semantics(variant)] = semantics_colors[RTAMT_SEMANTICS]
        # With a single variant on the plot there is nothing to disambiguate, so
        # keep the label short.
        semantics_display[_rtamt_semantics(variant)] = (
            "RTAMT" if len(rtamt_variants) == 1 else name
        )
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

        is_rtamt = semantics in rtamt_semantics

        # Colour carries the semantics and the marker carries the operator, so the
        # implementation is left to encode: RTAMT draws hollow and dashed, ours
        # filled and dotted.
        ax.scatter(
            g["interval_len"],
            g["avg_per_sample_us"],
            facecolors="none" if is_rtamt else color,
            edgecolors=color if is_rtamt else "white",
            marker=marker,
            s=15 if is_rtamt else 25,
            zorder=3,
            linewidths=0.6 if is_rtamt else 0.7,
            alpha=0.5 if is_rtamt else 0.9,
        )

        if fit is not None:
            x_min, x_max = g["interval_len"].min(), g["interval_len"].max()
            if fit["model_name"] == "constant":
                x_fit = np.array([x_min, x_max])
                y_fit = np.full(2, fit["intercept"])
            else:
                # Sample in log space on a log x-axis, otherwise nearly all the points
                # land in the last decade and the low-x part renders as a polyline.
                space = np.logspace if args.log_x else np.linspace
                lo, hi = (
                    (np.log10(x_min), np.log10(x_max)) if args.log_x else (x_min, x_max)
                )
                x_fit = space(lo, hi, 500)
                y_fit = eval_fit(fit, x_fit)

            # A log y-axis cannot show non-positive predictions, and several fits have
            # negative intercepts.
            if args.log_scale:
                keep = y_fit > 0
                x_fit, y_fit = x_fit[keep], y_fit[keep]

            if len(x_fit):
                ax.plot(
                    x_fit,
                    y_fit,
                    color=color,
                    linewidth=1.4 if is_rtamt else 1.8,
                    linestyle=(0, (5, 2)) if is_rtamt else ":",
                    alpha=0.45 if is_rtamt else 0.6,
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
    if args.log_x:
        ax.set_xscale("log")

    ax.set_xlabel("Temporal upper bound ($b$)", labelpad=5)
    y_label = (
        "Average time per sample (\u00b5s, log scale)"
        if args.log_scale
        else "Average time per sample (\u00b5s)"
    )
    ax.set_ylabel(y_label, labelpad=5)
    # ax.set_title("Performance scaling of temporal operators", pad=5)
    ax.grid(True, which="major", linestyle="--", linewidth=0.8, alpha=0.55)
    if args.log_scale:
        ax.grid(True, which="minor", linestyle=":", linewidth=0.6, alpha=0.4)
    ax.tick_params(which="both", top=True, right=True, width=1.1)

    x_data_max = df_plot["interval_len"].max()
    x_data_min = df_plot["interval_len"].min()
    if args.log_x:
        # Reserve the label gutter as a fraction of the *decades* on screen; a fixed
        # multiplicative padding is a negligible slice of a log axis.
        decades = np.log10(x_data_max / max(x_data_min, 1e-12))
        x_label = x_data_max * 10 ** (0.01 * decades)
        x_right = x_data_max * 10 ** (0.26 * decades)
    else:
        x_label = x_data_max * 1.015
        x_right = x_data_max * 1.35
    ax.set_xlim(right=x_right)

    # Force matplotlib to finalise autoscaling so get_ylim() returns the real limits.
    fig.canvas.draw()
    y_min, y_max = ax.get_ylim()

    # Series that stop well short of the right edge — RTAMT's discrete-time Until is
    # only benchmarked at small bounds — are labelled inline at their last point. A
    # leader line running to the gutter would otherwise be longer than the curve and
    # would point at empty space.
    for info in direct_labels:
        info["inline"] = info["semantics"] == "Rosi" or (
            _axis_fraction(info["x"], x_data_min, x_data_max, args.log_x) < 0.6
        )

    y_targets = {i: info["y"] for i, info in enumerate(direct_labels)}
    gutter_indices = [i for i, info in enumerate(direct_labels) if not info["inline"]]
    # Spread them as one group rather than per operator: labels from different
    # operators sit at similar y and would otherwise be allowed to overlap.
    if gutter_indices:
        adjusted = adjust_label_positions(
            [direct_labels[i]["y"] for i in gutter_indices],
            y_min,
            y_max,
            min_frac=0.075,
            log_scale=args.log_scale,
        )
        for i, y_adj in zip(gutter_indices, adjusted):
            y_targets[i] = y_adj

    annotations = []
    for i, label_info in enumerate(direct_labels):
        y_target = y_targets[i]
        if label_info["inline"]:
            annotations.append(
                ax.annotate(
                    label_info["label"],
                    xy=(label_info["x"], label_info["y"]),
                    xytext=(5, 0),
                    textcoords="offset points",
                    color=label_info["color"],
                    fontsize=11,
                    fontweight="bold",
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
            )
        else:
            annotations.append(
                ax.annotate(
                    label_info["label"],
                    xy=(label_info["x"], label_info["y"]),
                    xytext=(x_label, y_target),
                    textcoords="data",
                    color=label_info["color"],
                    fontsize=11,
                    fontweight="bold",
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
            )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Settle the layout first: _fit_labels_inside measures pixels, so the axes must
    # already be at its final size.
    plt.tight_layout()
    _fit_labels_inside(fig, ax, annotations, args.log_x)
    plt.savefig(args.output, dpi=600)
    print(f"Plot saved successfully to: {args.output}")


if __name__ == "__main__":
    main()
