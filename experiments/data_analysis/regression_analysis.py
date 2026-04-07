"""Regression analysis for scalability benchmarks (native Rust and Python bindings).

This script fits regression models to execution-time vs temporal-depth data and
reports goodness-of-fit ($R^2$) for each (source, semantics, operator) group.

Model assumptions:
- Non-RoSI semantics: constant for G/F, linear for U.
- RoSI semantics: linear for G/F, quadratic for U.

Usage:
    python regression_analysis.py
    python regression_analysis.py --output regression_fit_results.csv
"""

from __future__ import annotations

import argparse
import csv
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

import numpy as np
import pandas as pd


ROOT = Path(__file__).resolve().parent.parent / "paper_results" / "outputs"
NATIVE_CSV = ROOT / "performance_results_M=50.csv"
PYTHON_CSV = ROOT / "python_performance_results_M=50.csv"
DEFAULT_OUTPUT_CSV = "regression_fit_results.csv"

_OPERATOR_FROM_NATIVE_FID = {
    "4": "U",
    "5": "G",
    "6": "F",
}

_SIMPLE_SPEC_PATTERN = re.compile(r"^(?P<op>[GF])\[0,\s*[-+]?\d+(?:\.\d+)?\]\s*\(.*\)$")
_SIMPLE_UNTIL_PATTERN = re.compile(r"^\(.*\)\s*U\[0,\s*[-+]?\d+(?:\.\d+)?\]\s*\(.*\)$")


@dataclass(frozen=True)
class DataPoint:
    source: str
    semantics: str
    operator: str
    temporal_depth: float
    time_us: float


@dataclass(frozen=True)
class RegressionResult:
    source: str
    semantics: str
    operator: str
    model_degree: int
    model_name: str
    n_points: int
    intercept: float
    coef_b: float
    coef_b2: float
    r2: float
    adjusted_r2: float
    rmse: float

    def as_csv_row(self) -> dict[str, str | int | float]:
        return {
            "source": self.source,
            "semantics": self.semantics,
            "operator": self.operator,
            "model_degree": self.model_degree,
            "model_name": self.model_name,
            "n_points": self.n_points,
            "intercept": self.intercept,
            "coef_b": self.coef_b,
            "coef_b2": self.coef_b2,
            "r2": self.r2,
            "adjusted_r2": self.adjusted_r2,
            "rmse": self.rmse,
        }


def _dataframe_to_points(df: pd.DataFrame, source: str) -> list[DataPoint]:
    """Convert normalized dataframe rows to DataPoint list."""
    return [
        DataPoint(
            source=source,
            semantics=semantics,
            operator=operator,
            temporal_depth=float(temporal_depth),
            time_us=float(time_us),
        )
        for semantics, operator, temporal_depth, time_us in df[
            ["semantics", "operator", "temporal_depth", "time_us"]
        ].itertuples(index=False, name=None)
    ]


def _load_native_points(path: Path) -> list[DataPoint]:
    """Load native Rust scalability datapoints (formula IDs 4/5/6)."""
    df = pd.read_csv(path, dtype={"formula_id": "string"})

    df["formula_id"] = df["formula_id"].astype("string").str.strip()
    df = df[df["formula_id"].isin(_OPERATOR_FROM_NATIVE_FID)]

    df["operator"] = df["formula_id"].map(_OPERATOR_FROM_NATIVE_FID)
    df["temporal_depth"] = pd.to_numeric(df.get("interval_len"), errors="coerce")
    df["time_us"] = pd.to_numeric(df.get("avg_per_sample_us"), errors="coerce")
    df["semantics"] = df["semantics"].astype(str).str.strip()

    df = df.dropna(subset=["operator", "temporal_depth", "time_us"])
    return _dataframe_to_points(df, source="native")


def _load_python_points(path: Path) -> list[DataPoint]:
    """Load Python-binding scalability datapoints (simple G/F/U formulas only)."""
    df = pd.read_csv(path)

    spec = (
        df.get("spec", pd.Series("", index=df.index)).fillna("").astype(str).str.strip()
    )
    simple_mask = spec.str.match(_SIMPLE_SPEC_PATTERN) | spec.str.match(
        _SIMPLE_UNTIL_PATTERN
    )

    operators = np.select(
        [
            spec.str.contains(r"\sU\[", regex=True),
            spec.str.startswith("G[") & ~spec.str.contains("->", regex=False),
            spec.str.startswith("F[") & ~spec.str.contains("->", regex=False),
        ],
        ["U", "G", "F"],
        default="",
    )

    df = df.assign(
        semantics=df["semantics"].astype(str).str.strip(),
        operator=operators,
        temporal_depth=pd.to_numeric(df.get("temporal_depth"), errors="coerce"),
        time_us=pd.to_numeric(df.get("avg_per_sample_us"), errors="coerce"),
    )

    df = df[simple_mask & (df["operator"] != "")]
    df = df.dropna(subset=["temporal_depth", "time_us"])
    return _dataframe_to_points(df, source="python")


def _select_model_degree(semantics: str, operator: str) -> int:
    """Select polynomial degree according to the fixed modeling assumptions."""
    # RoSI: G/F linear, U quadratic
    if semantics == "Rosi":
        return 2 if operator == "U" else 1

    # Non-RoSI: G/F constant, U linear
    if operator in {"F", "G"}:
        return 0

    return 1


def _fit_polynomial(
    x: np.ndarray, y: np.ndarray, degree: int
) -> tuple[np.ndarray, np.ndarray]:
    """Fit polynomial with least squares and return (coeffs_high_to_low, y_hat)."""
    coeffs = np.polyfit(x, y, degree)
    y_hat = np.polyval(coeffs, x)
    return coeffs, y_hat


def _r2(y_true: np.ndarray, y_pred: np.ndarray) -> float:
    """Compute coefficient of determination $R^2$."""
    ss_res = float(np.sum((y_true - y_pred) ** 2))
    ss_tot = float(np.sum((y_true - np.mean(y_true)) ** 2))
    if np.isclose(ss_tot, 0.0):
        return float("nan")
    return 1.0 - ss_res / ss_tot


def _adjusted_r2(r2: float, n: int, p: int) -> float:
    """Compute adjusted $R^2$ with p predictors (excluding intercept)."""
    if np.isnan(r2) or n <= p + 1:
        return float("nan")
    return 1.0 - (1.0 - r2) * (n - 1) / (n - p - 1)


def _rmse(y_true: np.ndarray, y_pred: np.ndarray) -> float:
    """Compute root mean squared error."""
    return float(np.sqrt(np.mean((y_true - y_pred) ** 2)))


def _group_points(
    points: Iterable[DataPoint],
) -> dict[tuple[str, str, str], list[DataPoint]]:
    """Group datapoints by (source, semantics, operator)."""
    grouped: dict[tuple[str, str, str], list[DataPoint]] = {}
    for p in points:
        key = (p.source, p.semantics, p.operator)
        grouped.setdefault(key, []).append(p)
    return grouped


def load_scalability_points(
    native_csv: Path, python_csv: Path | None
) -> list[DataPoint]:
    """Load and combine native + optional Python scalability points used for regression."""
    points = _load_native_points(native_csv)
    if python_csv is not None and python_csv.exists():
        points += _load_python_points(python_csv)
    return points


def run_regression(
    native_csv: Path,
    python_csv: Path | None,
) -> list[RegressionResult]:
    """Run regression analysis for native and Python benchmark datasets."""
    points = load_scalability_points(native_csv, python_csv)
    grouped = _group_points(points)

    results: list[RegressionResult] = []

    for (source, semantics, operator), group in sorted(grouped.items()):
        # Sort by temporal depth for stable fitting/output
        group_sorted = sorted(group, key=lambda d: d.temporal_depth)
        x = np.array([p.temporal_depth for p in group_sorted], dtype=float)
        y = np.array([p.time_us for p in group_sorted], dtype=float)

        degree = _select_model_degree(semantics, operator)
        n = int(x.size)

        # Require at least degree+1 points to fit
        if n < degree + 1:
            continue

        coeffs_h2l, y_hat = _fit_polynomial(x, y, degree)
        r2 = _r2(y, y_hat)
        adj_r2 = _adjusted_r2(r2, n=n, p=degree)
        rmse = _rmse(y, y_hat)

        # Normalize to y = intercept + coef_b*x + coef_b2*x^2
        if degree == 0:
            intercept = float(coeffs_h2l[0])
            coef_b = 0.0
            coef_b2 = 0.0
            model_name = "constant"
        elif degree == 1:
            coef_b, intercept = float(coeffs_h2l[0]), float(coeffs_h2l[1])
            coef_b2 = 0.0
            model_name = "linear"
        else:
            coef_b2, coef_b, intercept = map(float, coeffs_h2l)
            model_name = "quadratic"

        results.append(
            RegressionResult(
                source=source,
                semantics=semantics,
                operator=operator,
                model_degree=degree,
                model_name=model_name,
                n_points=n,
                intercept=intercept,
                coef_b=coef_b,
                coef_b2=coef_b2,
                r2=r2,
                adjusted_r2=adj_r2,
                rmse=rmse,
            )
        )

    return results


def save_results_csv(results: list[RegressionResult], output_csv: Path) -> None:
    """Write regression results to CSV."""
    output_csv.parent.mkdir(parents=True, exist_ok=True)

    fieldnames = [
        "source",
        "semantics",
        "operator",
        "model_degree",
        "model_name",
        "n_points",
        "intercept",
        "coef_b",
        "coef_b2",
        "r2",
        "adjusted_r2",
        "rmse",
    ]

    with output_csv.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        for result in results:
            writer.writerow(result.as_csv_row())


def _build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Fit benchmark regressions and export coefficients with R^2 statistics."
    )
    parser.add_argument(
        "--native-csv",
        type=Path,
        default=NATIVE_CSV,
        help=f"Path to native benchmark CSV (default: {NATIVE_CSV})",
    )
    parser.add_argument(
        "--python-csv",
        type=Path,
        default=None,
        help="Optional path to Python benchmark CSV (omit to run native-only regression)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT_CSV,
        help=f"Output CSV path (default: {DEFAULT_OUTPUT_CSV})",
    )
    return parser


def main() -> None:
    args = _build_arg_parser().parse_args()

    python_csv = args.python_csv
    if python_csv is None and PYTHON_CSV.exists():
        python_csv = PYTHON_CSV

    results = run_regression(native_csv=args.native_csv, python_csv=python_csv)
    save_results_csv(results, args.output)

    print(f"Saved {len(results)} regression rows to: {args.output}")


if __name__ == "__main__":
    main()
