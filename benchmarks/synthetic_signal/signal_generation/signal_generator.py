import argparse
import csv
from pathlib import Path

import numpy as np


def _write_signal_csv(t: np.ndarray, signal_values: np.ndarray, output_path: str) -> None:
    path = Path(output_path)
    path.parent.mkdir(parents=True, exist_ok=True)

    with path.open("w", newline="") as csvfile:
        writer = csv.writer(csvfile)
        writer.writerow(["timestep", "value"])
        writer.writerows(zip(t, signal_values))


def generate_sine_wave_signal(num_samples: int, sampling_rate: float, frequency: float, filename: str) -> None:
    t = np.arange(num_samples, dtype=float) / sampling_rate
    signal_values = np.sin(2 * np.pi * frequency * t)
    _write_signal_csv(t, signal_values, filename)


def generate_chirp_signal(
    num_samples: int,
    sampling_rate: float,
    start_frequency: float,
    end_frequency: float,
    filename: str,
) -> None:
    from scipy.signal import chirp

    t = np.arange(num_samples, dtype=float) / sampling_rate
    duration = 0.0 if num_samples == 0 else num_samples / sampling_rate
    if duration == 0.0:
        signal_values = np.array([], dtype=float)
    else:
        signal_values = chirp(t, f0=start_frequency, f1=end_frequency, t1=duration, method="linear")

    _write_signal_csv(t, signal_values, filename)


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Generate benchmark signal CSV files")
    parser.add_argument("--num-samples", type=int, default=None, help="Number of samples to generate")
    parser.add_argument(
        "--num-signals",
        type=int,
        default=None,
        help="Compatibility alias used by bench scripts (treated as effective sample count when provided)",
    )
    parser.add_argument("--output-path", type=str, default=None, help="Output CSV path")
    parser.add_argument(
        "--signal-type",
        type=str,
        choices=["sine", "chirp"],
        default="sine",
        help="Type of signal to generate",
    )
    parser.add_argument("--sampling-rate", type=float, default=1.0, help="Sampling rate in Hz")
    parser.add_argument("--frequency", type=float, default=0.01, help="Sine frequency in Hz")
    parser.add_argument("--start-frequency", type=float, default=0.01, help="Chirp start frequency in Hz")
    parser.add_argument("--end-frequency", type=float, default=0.0001, help="Chirp end frequency in Hz")
    return parser.parse_args()


def _run_cli(args: argparse.Namespace) -> None:
    if args.output_path is None:
        raise SystemExit("--output-path is required when using CLI mode.")

    if args.num_signals is not None:
        effective_num_samples = args.num_signals
    elif args.num_samples is not None:
        effective_num_samples = args.num_samples
    else:
        raise SystemExit("Provide --num-signals or --num-samples.")

    if effective_num_samples < 0:
        raise SystemExit("Sample count must be >= 0")

    if args.signal_type == "sine":
        generate_sine_wave_signal(
            num_samples=effective_num_samples,
            sampling_rate=args.sampling_rate,
            frequency=args.frequency,
            filename=args.output_path,
        )
    else:
        generate_chirp_signal(
            num_samples=effective_num_samples,
            sampling_rate=args.sampling_rate,
            start_frequency=args.start_frequency,
            end_frequency=args.end_frequency,
            filename=args.output_path,
        )

    print(f"Signal generated and saved to {args.output_path}")


if __name__ == "__main__":
    args = _parse_args()

    # CLI mode (used by benchmark shell scripts)
    if args.output_path is not None or args.num_samples is not None or args.num_signals is not None:
        _run_cli(args)
    else:
        # Backward-compatible batch mode
        durations = np.arange(0, 1001, 100)
        for duration in durations:
            output_filename = f"signals/signal_{duration}_sine.csv"
            generate_sine_wave_signal(
                num_samples=int(duration),
                sampling_rate=1.0,
                frequency=0.01,
                filename=output_filename,
            )
            print(f"Signal generated and saved to {output_filename}")
