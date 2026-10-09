"""Live STL monitoring demo — Bokeh server app.

Two signals (``temp`` and ``pressure``) are streamed through four monitors —
one per semantics — and plotted live in a sliding window so the traces scroll
instead of squishing. The data loop runs forever.

Run with:  uv run bokeh serve live_demo.py --show
"""

import heapq
import math
import random
import threading
import time

import mstlo_python as mstlo
from bokeh.io import curdoc
from bokeh.layouts import column, gridplot, row
from bokeh.models import Band, ColumnDataSource, Div, Range1d, Slider, Span
from bokeh.plotting import figure

# --------------------------------------------------------------------------- #
# Configuration
# --------------------------------------------------------------------------- #

FORMULA = (
    "G[0, 10]((temp >= $TRIGGER_TEMP) -> "
    "F[0, 10]((pressure >= $PRESS_LOW) and (pressure <= $PRESS_HIGH)))"
)
TRIGGER_TEMP = 170.0
PRESS_LOW = 90.0
PRESS_HIGH = 110.0

# Sliding window width (seconds). Old data scrolls out of view.
WINDOW = 30.0

# How many seconds of history to keep in memory (older data is pruned).
KEEP = 2 * WINDOW

# Robustness bounds are clipped to this range for plotting (RoSI can be +/-inf).
CLIP = 30.0

# Virtual seconds advanced per wall-clock second (2.0 = double speed).
TIME_SCALE = 2.0

# Wall-clock milliseconds between ticks.
TICK_MS = 100

# Smoothing factor for the running-mean latency readout (0..1). Higher reacts
# faster to changes; lower damps spikes more.
LATENCY_ALPHA = 0.1

# Latency is measured by replaying a synthetic trace through fresh monitors in
# a tight loop on a dedicated thread. Timing the live loop directly would be
# misleading: it sleeps between samples, so the CPU idles and the first update
# after each nap pays a "cold start" penalty that has nothing to do with the
# semantics being measured.
BENCH_INTERVAL = 1.0  # seconds between latency benchmark runs
BENCH_SAMPLES = 1000  # synthetic samples replayed per benchmark run
BENCH_WARMUP = 20  # leading samples discarded before timing

SEMANTICS = [
    ("DelayedQualitative", "#1f77b4"),
    ("EagerQualitative", "#ff7f0e"),
    ("DelayedQuantitative", "#2ca02c"),
    ("Rosi", "#9467bd"),
]

GOOD = "#2ca02c"
BAD = "#d62728"
UNKNOWN = "#7f7f7f"
NOW_COLOR = "#999999"

# --------------------------------------------------------------------------- #
# Infinite signal streams
# --------------------------------------------------------------------------- #


def clip(v: float) -> float:
    if math.isinf(v):
        return CLIP if v > 0 else -CLIP
    return v


def spike(
    t: float, offset: float, amplitude: float, period: float, width: float, *, math=math
) -> float:
    phase = (t - offset) % period
    if phase < width:
        return amplitude * math.sin(math.pi * phase / width)
    return 0.0


def temp_stream(*, random=random, math=math):
    rng = random.Random(7)
    t = 0.0
    while True:
        # Normal operating temperature ~150°C, spiking to ~185°C every 20s.
        base = 150.0 + 6.0 * math.sin(0.25 * t)
        peak = spike(t, 0.0, 35.0, 20.0, 5.0, math=math)
        yield ("temp", base + peak + rng.uniform(-1.0, 1.0), t)
        t += 0.8 + rng.uniform(0.0, 0.3)


def pressure_stream(*, random=random, math=math):
    rng = random.Random(8)
    t = 0.15
    while True:
        # Normal pressure ~100 (range 90–110). After each temperature
        # spike, pressure briefly spikes too: it recovers fast on even
        # cycles, but on odd cycles it stays high and misses the 10s
        # deadline (a fault).
        base = 100.0 + 3.0 * math.sin(0.22 * t + 1.0)
        cycle = int((t - 0.5) // 20.0)
        if cycle % 2 == 0:
            peak = spike(t, 0.5, 25.0, 20.0, 3.0, math=math)
        else:
            peak = spike(t, 0.5, 30.0, 20.0, 15.0, math=math)
        yield ("pressure", base + peak + rng.uniform(-0.8, 0.8), t)
        t += 0.9 + rng.uniform(0.0, 0.3)


# --------------------------------------------------------------------------- #
# Shared state (guarded by a lock — the monitors are not thread-safe, so all
# monitoring happens on one dedicated worker thread and only this plain state
# crosses the thread boundary).
# --------------------------------------------------------------------------- #

state_lock = threading.Lock()

# Current values of the formula's runtime variables. The UI sliders write here
# (under state_lock) and the worker applies them to its own Variables object on
# each sample. mstlo's Variables is not thread-safe, so each thread keeps its
# own copy and only this plain dict crosses the thread boundary.
var_values = {
    "TRIGGER_TEMP": TRIGGER_TEMP,
    "PRESS_LOW": PRESS_LOW,
    "PRESS_HIGH": PRESS_HIGH,
}

signals = {"temp": [], "pressure": []}
verdicts = {name: {} for name, _ in SEMANTICS}
latencies = {name: 0.0 for name, _ in SEMANTICS}

# Per-semantics map of verdict-timestamp -> stream time at which that verdict
# first appeared. Used to check that EagerQualitative never reports a given
# timestamp *later* than DelayedQualitative (the whole point of "eager" is that
# it decides earlier). Kept pruned in step with `verdicts`.
first_emit = {name: {} for name, _ in SEMANTICS}

# Running count of verdict timestamps that DelayedQualitative produced but
# EagerQualitative never did, once the timestamp aged out of the keep window.
backend_dropped = {"count": 0}


def worker(
    *,
    mstlo=mstlo,
    time=time,
    heapq=heapq,
    temp_stream=temp_stream,
    pressure_stream=pressure_stream,
    formula_str=FORMULA,
    trigger_temp=TRIGGER_TEMP,
    press_low=PRESS_LOW,
    press_high=PRESS_HIGH,
    var_values=var_values,
    semantics=SEMANTICS,
    time_scale=TIME_SCALE,
    lock=state_lock,
    sigs=signals,
    verdict_map=verdicts,
    first_emit=first_emit,
    backend_dropped=backend_dropped,
    keep=KEEP,
) -> None:
    """Stream the trace through the monitors on a single dedicated thread.

    Dependencies are bound as default arguments so the thread keeps working
    even if Bokeh garbage-collects the (temporary) app module, which would
    otherwise wipe the module-level globals.
    """

    trace = heapq.merge(temp_stream(), pressure_stream(), key=lambda s: s[2])

    variables = mstlo.Variables()
    variables.set("TRIGGER_TEMP", trigger_temp)
    variables.set("PRESS_LOW", press_low)
    variables.set("PRESS_HIGH", press_high)
    formula = mstlo.parse_formula(formula_str)

    monitors = {
        name: mstlo.Monitor(
            formula,
            semantics=name,
            variables=variables,
            init_signals={"temp": 150.0, "pressure": 100.0},
        )
        for name, _ in semantics
    }

    prev_t = None
    for signal, value, t in trace:
        if prev_t is not None:
            time.sleep((t - prev_t) / time_scale)
        prev_t = t

        with lock:
            # Apply any variable updates from the UI sliders.
            variables.set("TRIGGER_TEMP", var_values["TRIGGER_TEMP"])
            variables.set("PRESS_LOW", var_values["PRESS_LOW"])
            variables.set("PRESS_HIGH", var_values["PRESS_HIGH"])

            sigs[signal].append((t, value))
            for name, _ in semantics:
                out = monitors[name].update(signal, value, t)
                for ts, val in out.verdicts():
                    verdict_map[name][ts] = val
                    first_emit[name].setdefault(ts, t)

            # Prune history older than the keep window to bound memory.
            cutoff = t - keep
            for sig in ("temp", "pressure"):
                pts = sigs[sig]
                i = 0
                while i < len(pts) and pts[i][0] < cutoff:
                    i += 1
                if i:
                    del pts[:i]

            # A timestamp that has aged out is long past any legitimate eager
            # delay, so if Delayed produced it and Eager never did, Eager
            # dropped it outright. Count before pruning the emit maps.
            dq_fe = first_emit["DelayedQualitative"]
            eq_fe = first_emit["EagerQualitative"]
            for k in [k for k in dq_fe if k < cutoff]:
                if k not in eq_fe:
                    backend_dropped["count"] += 1

            for name, _ in semantics:
                data = verdict_map[name]
                for k in [k for k in data if k < cutoff]:
                    del data[k]
                fe = first_emit[name]
                for k in [k for k in fe if k < cutoff]:
                    del fe[k]


def benchmark(
    *,
    mstlo=mstlo,
    time=time,
    heapq=heapq,
    temp_stream=temp_stream,
    pressure_stream=pressure_stream,
    formula_str=FORMULA,
    trigger_temp=TRIGGER_TEMP,
    press_low=PRESS_LOW,
    press_high=PRESS_HIGH,
    semantics=SEMANTICS,
    lock=state_lock,
    latency_map=latencies,
    latency_alpha=LATENCY_ALPHA,
    interval=BENCH_INTERVAL,
    samples=BENCH_SAMPLES,
    warmup=BENCH_WARMUP,
) -> None:
    """Measure each semantics' per-update latency on a dedicated thread.

    The live loop sleeps between samples, which lets the CPU idle and makes
    the first update after each nap look artificially slow — that cost is the
    CPU waking back up, not the semantics. So instead of timing the live loop,
    we replay a synthetic trace through fresh monitors in a tight loop, where
    the CPU stays warm and the numbers reflect the per-update work itself.
    """

    variables = mstlo.Variables()
    variables.set("TRIGGER_TEMP", trigger_temp)
    variables.set("PRESS_LOW", press_low)
    variables.set("PRESS_HIGH", press_high)
    formula = mstlo.parse_formula(formula_str)

    monitors = {
        name: mstlo.Monitor(
            formula,
            semantics=name,
            variables=variables,
            init_signals={"temp": 150.0, "pressure": 100.0},
        )
        for name, _ in semantics
    }

    merged = heapq.merge(temp_stream(), pressure_stream(), key=lambda s: s[2])
    trace = [next(merged) for _ in range(samples)]

    while True:
        time.sleep(interval)

        for monitor in monitors.values():
            monitor.reset()

        total = {name: 0.0 for name, _ in semantics}
        count = 0
        for signal, value, t in trace:
            count += 1
            for name, _ in semantics:
                t0 = time.perf_counter()
                monitors[name].update(signal, value, t)
                if count > warmup:
                    total[name] += time.perf_counter() - t0

        timed = count - warmup
        if timed <= 0:
            continue

        with lock:
            for name, _ in semantics:
                mean = total[name] / timed
                latency_map[name] += latency_alpha * (mean - latency_map[name])


# --------------------------------------------------------------------------- #
# Data sources
# --------------------------------------------------------------------------- #

sig_sources = {
    "temp": ColumnDataSource({"t": [], "y": []}),
    "pressure": ColumnDataSource({"t": [], "y": []}),
}

sources = {}
for name, _ in SEMANTICS:
    if name == "Rosi":
        sources[name] = ColumnDataSource({"t": [], "lo": [], "hi": []})
    elif name in ("DelayedQualitative", "EagerQualitative"):
        sources[name] = ColumnDataSource({"t": [], "y": [], "color": []})
    else:
        sources[name] = ColumnDataSource({"t": [], "y": []})

# RoSI intervals that have fully settled (lower == upper) — drawn as a line.
rosi_settled = ColumnDataSource({"t": [], "y": []})

# --------------------------------------------------------------------------- #
# Figures
# --------------------------------------------------------------------------- #

x_range = Range1d(start=0, end=WINDOW)
TOOLS = "pan,wheel_zoom,xwheel_zoom,reset,save"

sig_fig = figure(
    title="signals (zero-order hold)",
    x_range=x_range,
    y_range=(70, 200),
    width=1220,
    height=300,
    tools=TOOLS,
)
sig_fig.step(
    "t",
    "y",
    source=sig_sources["temp"],
    mode="after",
    line_width=2,
    color="#1f77b4",
    legend_label="temp",
)
sig_fig.step(
    "t",
    "y",
    source=sig_sources["pressure"],
    mode="after",
    line_width=2,
    color="#ff7f0e",
    legend_label="pressure",
)
trigger_span = Span(
    location=TRIGGER_TEMP,
    dimension="width",
    line_dash="dashed",
    line_color="#1f77b4",
    line_alpha=0.6,
)
sig_fig.add_layout(trigger_span)
press_high_span = Span(
    location=PRESS_HIGH,
    dimension="width",
    line_dash="dashed",
    line_color="#ff7f0e",
    line_alpha=0.6,
)
sig_fig.add_layout(press_high_span)
press_low_span = Span(
    location=PRESS_LOW,
    dimension="width",
    line_dash="dotted",
    line_color="#ff7f0e",
    line_alpha=0.5,
)
sig_fig.add_layout(press_low_span)
sig_fig.legend.location = "top_right"

sem_figs = {}
status_divs = {}
now_spans = {}

for name, color in SEMANTICS:
    if name == "Rosi":
        fig = figure(
            title=name,
            x_range=x_range,
            y_range=(-CLIP, CLIP),
            width=600,
            height=280,
            tools=TOOLS,
        )
        fig.add_layout(
            Band(
                base="t",
                lower="lo",
                upper="hi",
                source=sources[name],
                fill_color=color,
                fill_alpha=0.2,
                line_alpha=0,
            )
        )
        fig.step("t", "y", source=rosi_settled, mode="after", line_width=2, color=color)
        fig.add_layout(
            Span(
                location=0, dimension="width", line_dash="dashed", line_color="#888888"
            )
        )
    elif name in ("DelayedQualitative", "EagerQualitative"):
        fig = figure(
            title=name,
            x_range=x_range,
            y_range=(-0.2, 1.2),
            width=600,
            height=280,
            tools=TOOLS,
        )
        fig.step(
            "t", "y", source=sources[name], mode="after", line_width=2, color=color
        )
        fig.scatter(
            "t", "y", source=sources[name], size=6, fill_color="color", line_color=None
        )
        fig.yaxis.ticker = [0, 1]
        fig.yaxis.major_label_overrides = {0: "false", 1: "true"}
        fig.add_layout(
            Span(
                location=0.5,
                dimension="width",
                line_dash="dashed",
                line_color="#cccccc",
            )
        )
    else:
        fig = figure(
            title=name,
            x_range=x_range,
            y_range=(-30, 30),
            width=600,
            height=280,
            tools=TOOLS,
        )
        fig.step(
            "t", "y", source=sources[name], mode="after", line_width=2, color=color
        )
        fig.scatter("t", "y", source=sources[name], size=4, color=color)
        fig.add_layout(
            Span(
                location=0, dimension="width", line_dash="dashed", line_color="#888888"
            )
        )

    now_span = Span(
        location=0, dimension="height", line_dash="dotted", line_color=NOW_COLOR
    )
    fig.add_layout(now_span)
    now_spans[name] = now_span
    sem_figs[name] = fig
    status_divs[name] = Div(text="<b>PENDING</b>", width=600, height=28)

now_spans["signals"] = Span(
    location=0, dimension="height", line_dash="dotted", line_color=NOW_COLOR
)
sig_fig.add_layout(now_spans["signals"])

# --------------------------------------------------------------------------- #
# Controls & layout
# --------------------------------------------------------------------------- #

# Sliders for the formula's runtime variables. Moving one writes into the
# shared `var_values` dict, which the worker applies on its next sample.
temp_slider = Slider(
    title="TRIGGER_TEMP (°C)", start=120, end=200, value=TRIGGER_TEMP, step=1, width=380
)
press_low_slider = Slider(
    title="PRESS_LOW", start=70, end=120, value=PRESS_LOW, step=1, width=380
)
press_high_slider = Slider(
    title="PRESS_HIGH", start=90, end=140, value=PRESS_HIGH, step=1, width=380
)


def header_text(trigger, low, high):
    display_formula = (
        FORMULA.replace("$TRIGGER_TEMP", f"{trigger:g}")
        .replace("$PRESS_LOW", f"{low:g}")
        .replace("$PRESS_HIGH", f"{high:g}")
        .replace(" -> ", " → ")
        .replace(" and ", " ∧ ")
    )
    description = (
        f"Once the temperature reaches {trigger:g}°C, the pressure must "
        f"return to the normal range ({low:g}–{high:g}) within 10 seconds."
    )
    return (
        "<div style='font-family:monospace;font-size:15px;"
        "background:#f6f8fa;padding:10px 14px;border:1px solid #d0d7de;"
        "border-radius:6px;'>"
        "<b>STL formula:</b> "
        f"{display_formula}"
        "<div style='font-family:sans-serif;font-size:13px;color:#333;"
        "margin-top:6px;'>"
        f"{description}"
        "</div>"
        "</div>"
    )


header = Div(text=header_text(TRIGGER_TEMP, PRESS_LOW, PRESS_HIGH), width=1220)


def on_var_change(attr, old, new):
    with state_lock:
        var_values["TRIGGER_TEMP"] = temp_slider.value
        var_values["PRESS_LOW"] = press_low_slider.value
        var_values["PRESS_HIGH"] = press_high_slider.value
    trigger_span.location = temp_slider.value
    press_low_span.location = press_low_slider.value
    press_high_span.location = press_high_slider.value
    header.text = header_text(
        temp_slider.value, press_low_slider.value, press_high_slider.value
    )


temp_slider.on_change("value", on_var_change)
press_low_slider.on_change("value", on_var_change)
press_high_slider.on_change("value", on_var_change)

controls = row(temp_slider, press_low_slider, press_high_slider)

panels = [column(status_divs[name], sem_figs[name]) for name, _ in SEMANTICS]
bottom = gridplot(
    [[panels[0], panels[1]], [panels[2], panels[3]]], toolbar_location=None
)

layout = column(header, controls, sig_fig, bottom)
curdoc().title = "mstlo — live online STL monitoring"
curdoc().add_root(layout)

# --------------------------------------------------------------------------- #
# Rendering (called from the Bokeh callback, under the document lock)
# --------------------------------------------------------------------------- #


def status(name: str):
    data = verdicts[name]
    if not data:
        return "PENDING", UNKNOWN
    val = data[max(data)]
    if name in ("DelayedQualitative", "EagerQualitative"):
        return ("SATISFIED", GOOD) if val else ("VIOLATED", BAD)
    if name == "DelayedQuantitative":
        if val > 0:
            return "SATISFIED", GOOD
        if val < 0:
            return "VIOLATED", BAD
        return "UNDECIDED", UNKNOWN
    if name == "Rosi":
        closed = {t: v for t, v in data.items() if math.isfinite(v[0]) and v[0] == v[1]}
        if not closed:
            return "UNDECIDED", UNKNOWN
        val = closed[max(closed)]
        if val[0] > 0:
            return "SATISFIED", GOOD
        if val[0] < 0:
            return "VIOLATED", BAD
        return "UNDECIDED", UNKNOWN
    lo, hi = val
    if lo > 0:
        return "SATISFIED", GOOD
    if hi < 0:
        return "VIOLATED", BAD
    return "UNDECIDED", UNKNOWN


def backend_check():
    """Check the EagerQualitative backend against its contract.

    Eager is supposed to report the truth value of a timestamp *before* the
    delayed semantics can. If the backend is correct, it never emits a verdict
    for a timestamp after DelayedQualitative does, and it never silently drops
    a timestamp that Delayed reports. This isolates the backend from the
    plotting: `rebuild`/`status` only draw whatever verdicts the monitor
    actually produced, so any violation reported here is mstlo's doing.
    """
    dq = first_emit["DelayedQualitative"]
    eq = first_emit["EagerQualitative"]
    common = set(dq) & set(eq)
    late = [eq[ts] - dq[ts] for ts in common if eq[ts] > dq[ts] + 1e-9]
    dropped = backend_dropped["count"]

    if late:
        text = (
            "EagerQualitative backend: BUG — "
            f"{len(late)} verdict(s) emitted later than DelayedQualitative "
            f"(max {max(late):.2f}s); {dropped} dropped"
        )
        return text, BAD
    if dropped:
        text = (
            "EagerQualitative backend: BUG — "
            f"ordering holds but {dropped} verdict(s) were dropped"
        )
        return text, BAD
    return "EagerQualitative backend: OK — never later than Delayed", GOOD


def rebuild() -> None:
    for sig in ("temp", "pressure"):
        pts = signals[sig]
        sig_sources[sig].data = {
            "t": [p[0] for p in pts],
            "y": [p[1] for p in pts],
        }

    for name, _ in SEMANTICS:
        data = verdicts[name]
        ts = sorted(data)
        if name == "Rosi":
            xs, los, his = [], [], []
            settled_t, settled_y = [], []
            for i, t in enumerate(ts):
                raw_lo, raw_hi = data[t][0], data[t][1]
                lo, hi = clip(raw_lo), clip(raw_hi)
                xs.append(t)
                los.append(lo)
                his.append(hi)
                if i + 1 < len(ts):
                    xs.append(ts[i + 1])
                    los.append(lo)
                    his.append(hi)
                if math.isfinite(raw_lo) and raw_lo == raw_hi:
                    settled_t.append(t)
                    settled_y.append(lo)
            sources[name].data = {"t": xs, "lo": los, "hi": his}
            rosi_settled.data = {"t": settled_t, "y": settled_y}
        elif name in ("DelayedQualitative", "EagerQualitative"):
            ys = [1.0 if data[t] else 0.0 for t in ts]
            sources[name].data = {
                "t": ts,
                "y": ys,
                "color": [GOOD if v else BAD for v in ys],
            }
        else:
            sources[name].data = {"t": ts, "y": [data[t] for t in ts]}


def tick() -> None:
    # The window slides at a constant rate, derived from wall-clock time, so it
    # never jumps when a new sample happens to arrive.
    virtual_now = (time.monotonic() - start_time) * TIME_SCALE

    with state_lock:
        rebuild()
        statuses = {name: status(name) for name, _ in SEMANTICS}
        latency_snapshot = {name: latencies[name] for name, _ in SEMANTICS}

    end = max(virtual_now, WINDOW)
    x_range.start = end - WINDOW
    x_range.end = end
    for span in now_spans.values():
        span.location = virtual_now

    for name, _ in SEMANTICS:
        text, color = statuses[name]
        status_divs[name].text = (
            f"<b style='font-size:14px;color:{color}'>{text}</b>"
            f"<span style='color:#555;font-size:16px;margin-left:14px'>"
            f"{latency_snapshot[name] * 1_000_000:.1f} µs per step</span>"
        )


# --------------------------------------------------------------------------- #
# Start
# --------------------------------------------------------------------------- #

start_time = time.monotonic()
threading.Thread(target=worker, daemon=True).start()
threading.Thread(target=benchmark, daemon=True).start()
curdoc().add_periodic_callback(tick, TICK_MS)
