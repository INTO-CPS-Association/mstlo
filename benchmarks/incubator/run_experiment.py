"""Gather incubator temperature traces together with STL robustness.

This drives the *real* course services over RabbitMQ:

  * ``5-IncubatorPTEmulator/pt_emulator_service.py``  -- the plant
  * ``2-Controller-Modelling/controller.py``          -- the thermostat

Both are launched as subprocesses from their own folders, exactly as the course
notebooks instruct.  This script acts as the monitoring client: it subscribes to
the emulator state stream and the controller state stream, feeds the monitored
part of the run to two ``mstlo`` monitors, and writes everything to CSV.

One continuous session produces the whole dataset, so every phase shares
identical conditions.  *All* of it is recorded, tagged by a ``phase`` column, so
nothing has to be gathered twice:

  1. ``preheat``   the emulator always starts the box at 30 C, and the
                   thermostat takes ~15 min to bring it into the control band.
                   Nothing is done to speed this up -- the run is left entirely
                   to the course services -- but the climb is recorded.
  2. ``warmup``    run on until the thermostat has completed ``--warmup-cycles``
                   cycles under normal room conditions, so the start-up
                   transient is not mistaken for a violation.
  3. ``normal``    undisturbed steady-state operation, ``--normal-cycles``
                   complete thermostat cycles.
  4. ``lid_open``  the lid is opened and recording continues for
                   ``--lid-duration`` seconds.

Only phases 3 and 4 are fed to the monitors; ``replay.py`` reproduces exactly
that by filtering on ``phase``.  The earlier phases are on disk for plotting and
for sizing the specification windows.

Because the emulator runs in real time at one sample every 3 s, the full
session takes roughly 70 minutes of wall-clock time.

Prerequisites: the RabbitMQ container must be running (start it from
``6-PuttingItAllTogether`` with ``python start_influxdb_rabbitmq.py``).

Usage (from the rv-example folder):

    python run_experiment.py
    python run_experiment.py --normal-cycles 3 --lid-duration 1200
"""

import argparse
import csv
import os
import subprocess
import sys
import time

from pyhocon import ConfigFactory

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INCUBATOR_DT_SOFTWARE = os.path.join(REPO_ROOT, "incubator_dt", "software")
sys.path.append(INCUBATOR_DT_SOFTWARE)

from incubator.communication.server.rabbitmq import Rabbitmq
from incubator.communication.shared.protocol import ROUTING_KEY_STATE

from stl_monitors import build_monitors

# Published by 2-Controller-Modelling/controller.py
ROUTING_KEY_CONTROLLER_STATE = "incubator.record.dtcourse.controller.state"
# Consumed by 5-IncubatorPTEmulator/pt_emulator_service.py
ROUTING_KEY_LID = "routing.key.lid"

SERVICES = [
    ("emulator", "5-IncubatorPTEmulator", "pt_emulator_service.py"),
    ("controller", "2-Controller-Modelling", "controller.py"),
]

# Phases whose samples are fed to the monitors; the rest are recorded only.
MONITORED_PHASES = ("normal", "lid_open")

# What the plant and controller published, one row per sample.
SIGNAL_FIELDS = [
    "t",
    "phase",
    "timestamp_ns",
    "temperature",
    "heater_on",
    "lid_open",
    "max_T",
    "min_T",
]

# How long each monitor.update() call took.  One row per call, so the cost of
# monitoring is measured by the same pass that feeds the monitors.
LATENCY_FIELDS = ["t", "spec", "update_us"]

# The box temperature at which the initial climb is considered over.  Only a
# label for the recorded data: nothing is done to the plant either way.
PREHEAT_TARGET_T = 36.0


def start_services(logdir):
    """Launch the real emulator and controller, each from its own folder."""
    os.makedirs(logdir, exist_ok=True)
    processes = []

    for name, folder, script in SERVICES:
        cwd = os.path.join(REPO_ROOT, folder)
        path = os.path.join(cwd, script)
        if not os.path.exists(path):
            raise FileNotFoundError(
                f"{path} not found -- run the notebook that generates it first"
            )

        logfile = open(os.path.join(logdir, f"{name}.log"), "w")
        proc = subprocess.Popen(
            [sys.executable, script], cwd=cwd, stdout=logfile, stderr=subprocess.STDOUT
        )
        processes.append((name, proc, logfile))
        print(f"started {name}: {folder}/{script} (pid {proc.pid})")
        # Give the emulator a moment to bind its queues before the controller
        # starts publishing heater commands.
        time.sleep(2)

    return processes


def stop_services(processes):
    for name, proc, logfile in processes:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
        logfile.close()
        print(f"stopped {name}")


class Recorder:
    """Writes every sample of the session, plus the monitor update cost.

    The signal file spans the whole session -- pre-heat included -- with a
    ``phase`` column saying which part of the run each sample belongs to.  The
    monitors are only fed the phases in ``MONITORED_PHASES``, so ``latency.csv``
    still measures the cost on the monitored segment alone.
    """

    def __init__(self, outdir, max_T, min_T):
        os.makedirs(outdir, exist_ok=True)
        self.signal_path = os.path.join(outdir, "signal.csv")
        self.latency_path = os.path.join(outdir, "latency.csv")

        self._signal_file = open(self.signal_path, "w", newline="", encoding="utf-8")
        self._signal = csv.DictWriter(self._signal_file, fieldnames=SIGNAL_FIELDS)
        self._signal.writeheader()

        self._latency_file = open(self.latency_path, "w", newline="", encoding="utf-8")
        self._latency = csv.DictWriter(self._latency_file, fieldnames=LATENCY_FIELDS)
        self._latency.writeheader()

        self._monitors = build_monitors(max_T=max_T, min_T=min_T)
        self._t0_ns = None
        self.samples = 0

    def elapsed(self, timestamp_ns):
        """Seconds since the first recorded sample, on the plant's own clock."""
        if self._t0_ns is None:
            self._t0_ns = timestamp_ns
        return round((timestamp_ns - self._t0_ns) / 1e9, 6)

    def write(self, phase, state, controller):
        fields = state["fields"]
        t = self.elapsed(state["time"])

        # The controller has not necessarily published yet during pre-heat.
        max_T = controller["fields"]["max_temperature"] if controller else ""
        min_T = controller["fields"]["min_temperature"] if controller else ""

        self._signal.writerow(
            {
                "t": t,
                "phase": phase,
                "timestamp_ns": state["time"],
                "temperature": fields["average_temperature"],
                "heater_on": int(bool(fields["heater_on"])),
                "lid_open": int(bool(fields["lid_open"])),
                "max_T": max_T,
                "min_T": min_T,
            }
        )
        self.samples += 1

        if phase in MONITORED_PHASES:
            self._update_monitors(t, fields["average_temperature"], max_T, min_T)

        self._signal_file.flush()
        self._latency_file.flush()
        return t

    def _update_monitors(self, t, temperature, max_T, min_T):
        # Each update is timed here, so the cost is measured on the running
        # system rather than reconstructed afterwards.
        params = {"max_T": max_T, "min_T": min_T}
        for monitor in self._monitors:
            started = time.perf_counter()
            monitor.update(t, temperature, params[monitor.param_name])
            self._latency.writerow(
                {
                    "t": t,
                    "spec": monitor.name,
                    "update_us": (time.perf_counter() - started) * 1e6,
                }
            )

    def close(self):
        self._signal_file.close()
        self._latency_file.close()


class ExperimentClient:
    """Subscribes to the running services and monitors what they publish."""

    def __init__(self, rabbitmq_config):
        self._rabbitmq = Rabbitmq(**rabbitmq_config)

        self.latest_state = None  # newest emulator sample
        self.latest_controller = None  # newest controller sample
        self.cycles = 0  # completed thermostat cycles
        self._previous_ctrl_state = None
        self._new_sample = False

    def connect(self):
        self._rabbitmq.connect_to_server()
        self._rabbitmq.subscribe(
            routing_key=ROUTING_KEY_STATE, on_message_callback=self._on_state
        )
        self._rabbitmq.subscribe(
            routing_key=ROUTING_KEY_CONTROLLER_STATE,
            on_message_callback=self._on_controller_state,
        )

    def close(self):
        self._rabbitmq.close()

    def _on_state(self, ch, method, properties, body_json):
        self.latest_state = body_json
        self._new_sample = True

    def _on_controller_state(self, ch, method, properties, body_json):
        self.latest_controller = body_json

        state = body_json["fields"]["current_state"]
        # A cycle completes each time the thermostat falls back to Heating.
        if self._previous_ctrl_state == "Cooling" and state == "Heating":
            self.cycles += 1
        self._previous_ctrl_state = state

    def set_lid_open(self, lid_open):
        self._rabbitmq.send_message(
            routing_key=ROUTING_KEY_LID, message={"lid_open": lid_open}
        )

    def next_sample(self, require_controller=True, timeout=30.0):
        """Block until the emulator publishes a new state, and pair it with the
        latest controller output. Returns (state, controller)."""
        deadline = time.time() + timeout
        self._new_sample = False

        while time.time() < deadline:
            self._rabbitmq.connection.process_data_events(time_limit=0.2)
            if self._new_sample and (
                self.latest_controller is not None or not require_controller
            ):
                return self.latest_state, self.latest_controller

        raise TimeoutError(
            "no state sample received within "
            f"{timeout:.0f} s -- is the emulator (and RabbitMQ) running?"
        )


def preheat(client, recorder, timeout=1800.0):
    """Record the initial climb from 30 C until the box first reaches the band.

    The plant is left completely alone here -- only the thermostat acts on it.
    This phase exists so the start-up transient is recorded and labelled rather
    than discarded, and so it is kept away from the monitors.
    """
    deadline = time.time() + timeout
    print(f"pre-heat: recording the climb to {PREHEAT_TARGET_T:.0f} C")

    while True:
        if time.time() > deadline:
            raise TimeoutError(f"pre-heat did not finish within {timeout:.0f} s")

        state, controller = client.next_sample(require_controller=False)
        recorder.write("preheat", state, controller)
        if state["fields"]["average_temperature"] >= PREHEAT_TARGET_T:
            break

    print(f"pre-heat done: T = {state['fields']['average_temperature']:.2f} C")


def run_cycles(client, recorder, phase, cycles, timeout=7200.0):
    """Record ``phase`` until the thermostat has completed ``cycles`` cycles."""
    deadline = time.time() + timeout
    start = client.cycles
    last_report = 0.0

    print(f"{phase}: recording {cycles} thermostat cycles")

    # Always take at least one sample, so the services are confirmed alive even
    # when no cycles are requested.
    state, controller = client.next_sample()
    recorder.write(phase, state, controller)

    while client.cycles - start < cycles:
        if time.time() > deadline:
            raise TimeoutError(
                f"{phase} did not complete within {timeout:.0f} s "
                f"({client.cycles - start} of {cycles} cycles completed)"
            )

        state, controller = client.next_sample()
        t = recorder.write(phase, state, controller)

        now = time.time()
        if now - last_report > 60:
            print(
                f"  {phase}: t = {t:.0f} s, "
                f"T = {state['fields']['average_temperature']:.2f} C, "
                f"{client.cycles - start}/{cycles} cycles"
            )
            last_report = now

    print(
        f"{phase} complete: T = "
        f"{client.latest_state['fields']['average_temperature']:.2f} C"
    )


def run_lid_open(client, recorder, duration):
    """Open the lid and keep recording for ``duration`` seconds."""
    client.set_lid_open(True)
    state, controller = client.next_sample()
    t0 = recorder.write("lid_open", state, controller)
    print(
        f"lid_open: opened at t = {t0:.0f} s "
        f"(T = {state['fields']['average_temperature']:.2f} C), "
        f"recording {duration:.0f} s"
    )

    last_report = 0.0
    while True:
        state, controller = client.next_sample()
        t = recorder.write("lid_open", state, controller)
        if t - t0 >= duration:
            break

        now = time.time()
        if now - last_report > 60:
            print(
                f"  lid_open: t = {t:.0f} s, "
                f"T = {state['fields']['average_temperature']:.2f} C"
            )
            last_report = now

    client.set_lid_open(False)
    print(f"lid_open complete: T = {state['fields']['average_temperature']:.2f} C")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--warmup-cycles",
        type=int,
        default=1,
        help="thermostat cycles to record as warm-up (not fed to the monitors)",
    )
    parser.add_argument(
        "--normal-cycles",
        type=int,
        default=3,
        help="thermostat cycles of undisturbed operation to record",
    )
    parser.add_argument(
        "--lid-duration",
        type=float,
        default=1200.0,
        help="seconds to keep recording after the lid opens",
    )
    parser.add_argument("--outdir", default="data")
    parser.add_argument("--logdir", default="logs")
    args = parser.parse_args()

    startup_conf = os.path.join(INCUBATOR_DT_SOFTWARE, "startup.conf")
    assert os.path.exists(startup_conf), "startup.conf file not found"
    config = ConfigFactory.parse_file(startup_conf)

    processes = start_services(args.logdir)
    client = ExperimentClient(config["rabbitmq"])
    recorder = None

    try:
        client.connect()
        recorder = Recorder(args.outdir, max_T=39.0, min_T=36.0)

        preheat(client, recorder)
        run_cycles(client, recorder, "warmup", args.warmup_cycles)
        run_cycles(client, recorder, "normal", args.normal_cycles)
        run_lid_open(client, recorder, args.lid_duration)
    finally:
        if recorder is not None:
            recorder.close()
            print(f"captured {recorder.samples} samples -> {recorder.signal_path}")
            print(f"captured update latency  -> {recorder.latency_path}")
        try:
            client.close()
        except Exception:
            pass
        stop_services(processes)


if __name__ == "__main__":
    main()
