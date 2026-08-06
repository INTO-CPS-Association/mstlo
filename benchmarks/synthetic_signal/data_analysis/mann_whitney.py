"""Mann-Whitney U test between two benchmark datasets.

Typical usage — native Rust vs rtamt, grouped by formula_id:

    python mann_whitney.py \
        --csv-a  ../BENCH_RESULTS/outputs/performance_results_M=50_raw.csv \
        --csv-b  ../BENCH_RESULTS/outputs/rtamt_benchmark_results_raw.csv \
        --label-a native --label-b rtamt \
        --group-by formula_id \
        --output mwu_native_vs_rtamt.csv
"""

import argparse
import json
from pathlib import Path

import pandas as pd
from scipy.stats import mannwhitneyu


def parse_args() -> argparse.Namespace:
    root = Path(__file__).resolve().parent
    p = argparse.ArgumentParser(description="Mann-Whitney U test between two datasets")
    p.add_argument("--csv-a", type=Path, required=True, help="First dataset CSV")
    p.add_argument("--csv-b", type=Path, required=True, help="Second dataset CSV")
    p.add_argument("--label-a", default="A", help="Name for first dataset")
    p.add_argument("--label-b", default="B", help="Name for second dataset")
    p.add_argument(
        "--column",
        default="per_sample_us",
        help="Numeric column to compare (default: per_sample_us)",
    )
    p.add_argument(
        "--filter-a",
        default=None,
        metavar="EXPR",
        help="pandas query string to filter dataset A before testing",
    )
    p.add_argument(
        "--filter-b",
        default=None,
        metavar="EXPR",
        help="pandas query string to filter dataset B before testing",
    )
    p.add_argument(
        "--group-by",
        nargs="+",
        default=None,
        metavar="COL",
        help="Run a separate test per combination of these columns (must exist in both CSVs)",
    )
    p.add_argument(
        "--alternative",
        choices=["two-sided", "less", "greater"],
        default="two-sided",
        help="Alternative hypothesis (default: two-sided)",
    )
    p.add_argument(
        "--output",
        type=Path,
        default=None,
        help="Output CSV path (default: <label-a>_vs_<label-b>_mwu.csv next to this script)",
    )
    return p.parse_args()


def run_test(
    a: pd.Series,
    b: pd.Series,
    label_a: str,
    label_b: str,
    alternative: str,
) -> dict:
    stat, p_val = mannwhitneyu(a, b, alternative=alternative)
    n_a, n_b = len(a), len(b)
    # rank-biserial correlation as effect size
    r = 1 - (2 * stat) / (n_a * n_b)
    return {
        "label_a": label_a,
        "label_b": label_b,
        "n_a": n_a,
        "n_b": n_b,
        "median_a": a.median(),
        "median_b": b.median(),
        "mean_a": a.mean(),
        "mean_b": b.mean(),
        "U_statistic": stat,
        "p_value": p_val,
        "effect_size_r": r,
        "alternative": alternative,
        "significant_0.05": p_val < 0.05,
    }


def main() -> None:
    args = parse_args()

    df_a = pd.read_csv(args.csv_a)
    df_b = pd.read_csv(args.csv_b)

    if args.filter_a:
        df_a = df_a.query(args.filter_a)
    if args.filter_b:
        df_b = df_b.query(args.filter_b)

    if args.column not in df_a.columns:
        raise ValueError(f"Column '{args.column}' not found in {args.csv_a}")
    if args.column not in df_b.columns:
        raise ValueError(f"Column '{args.column}' not found in {args.csv_b}")

    output = args.output
    if output is None:
        output = Path(__file__).resolve().parent / f"{args.label_a}_vs_{args.label_b}_mwu.csv"

    if args.group_by is None:
        result = run_test(
            df_a[args.column].dropna(),
            df_b[args.column].dropna(),
            args.label_a,
            args.label_b,
            args.alternative,
        )
        rows = [result]
    else:
        missing_a = [c for c in args.group_by if c not in df_a.columns]
        missing_b = [c for c in args.group_by if c not in df_b.columns]
        if missing_a:
            raise ValueError(f"Group-by columns missing from A: {missing_a}")
        if missing_b:
            raise ValueError(f"Group-by columns missing from B: {missing_b}")

        groups_a = set(map(tuple, df_a[args.group_by].drop_duplicates().values.tolist()))
        groups_b = set(map(tuple, df_b[args.group_by].drop_duplicates().values.tolist()))
        common = sorted(groups_a & groups_b)

        if not common:
            raise ValueError(
                f"No common groups between datasets for columns {args.group_by}. "
                f"A groups: {sorted(groups_a)}, B groups: {sorted(groups_b)}"
            )

        rows = []
        for key in common:
            mask_a = (df_a[args.group_by] == list(key)).all(axis=1)
            mask_b = (df_b[args.group_by] == list(key)).all(axis=1)
            a_vals = df_a.loc[mask_a, args.column].dropna()
            b_vals = df_b.loc[mask_b, args.column].dropna()
            if a_vals.empty or b_vals.empty:
                continue
            result = run_test(a_vals, b_vals, args.label_a, args.label_b, args.alternative)
            for col, val in zip(args.group_by, key):
                result[col] = val
            rows.append(result)

    results_df = pd.DataFrame(rows)

    # Reorder: group columns first, then stats
    group_cols = args.group_by or []
    stat_cols = [c for c in results_df.columns if c not in group_cols]
    results_df = results_df[group_cols + stat_cols]

    output.parent.mkdir(parents=True, exist_ok=True)
    results_df.to_csv(output, index=False, float_format="%.6g")
    print(f"Results saved to: {output}")
    print(results_df.to_string(index=False))


if __name__ == "__main__":
    main()
