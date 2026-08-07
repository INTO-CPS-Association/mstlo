"""The two STL monitors used in this experiment.

  phi_hi = G[0,30]( (T >= $max_T) -> F[0,360]( T <= $max_T ) )
      "an overshoot above the controller's max temperature is cleared within
       360 s".

  phi_lo = G[0,30]( (T <= $min_T) -> F[0,360]( T >= $min_T ) )
      "once the temperature falls to the controller's min temperature, the
       heater brings it back up within 360 s".

Opening the lid raises the heat loss beyond what the 18 W heater can supply,
so the box settles around 23 C: phi_lo becomes permanently violated, while
phi_hi stays satisfied because the temperature never exceeds max_T again.

The monitors use delayed quantitative semantics.  The verdict carried by
timestamp ``t`` is the robustness of the formula evaluated over the window
starting at ``t``, and it is emitted once -- when the temporal depth has
elapsed and the value is final.  Nothing is refined afterwards, so a window
either has its verdict or does not have one yet.
"""

import mstlo_python as mstlo

SEMANTICS = "DelayedQuantitative"
GLOBALLY_WINDOW = 30  # a
EVENTUALLY_WINDOW = 360  # b

SPEC_HI = (
    f"G[0,{GLOBALLY_WINDOW}]((T >= $max_T) -> "
    f"F[0,{EVENTUALLY_WINDOW}](T <= $max_T))"
)
SPEC_LO = (
    f"G[0,{GLOBALLY_WINDOW}]((T <= $min_T) -> "
    f"F[0,{EVENTUALLY_WINDOW}](T >= $min_T))"
)

TIME_DECIMALS = 6


def time_key(timestamp):
    return round(timestamp, TIME_DECIMALS)


class SpecMonitor:
    """One mstlo monitor plus the accumulated robustness trace."""

    def __init__(self, name, spec, param_name, param_value):
        self.name = name
        self.spec_str = spec
        self.param_name = param_name
        self.param_value = param_value

        self._vars = mstlo.Variables()
        self._vars.set(param_name, param_value)
        self._formula = mstlo.parse_formula(spec)
        self._monitor = mstlo.Monitor(
            formula=self._formula, semantics=SEMANTICS, variables=self._vars
        )

        # timestamp -> (lower, upper), refined in place as the monitor learns more
        self.trace = {}
        # timestamp -> time at which that verdict first became conclusive
        # (upper < 0 => violated, lower > 0 => satisfied, regardless of the future)
        self.decided_at = {}
        # timestamp -> time at which the interval collapsed to a point
        self.finalised_at = {}

    @property
    def temporal_depth(self):
        return self._monitor.get_temporal_depth()

    def update(self, timestamp, temperature, param_value):
        """Feed one sample; return the verdicts the monitor emitted for it.

        The return value is the monitor's actual online output: a vector of
        `(timestamp, (lower, upper))` covering the sliding window of verdicts
        the monitor is still refining, *not* a single verdict for the sample
        just fed.  The verdict for the newest timestamp is always
        `(-inf, inf)` -- no information about it exists yet -- so logging only
        that value records nothing.  Callers should log the whole vector.
        """
        # The threshold is a runtime parameter, exactly as in the course service,
        # so a reconfiguration of the controller is picked up by the monitor.
        self._vars.set(self.param_name, param_value)

        now = time_key(timestamp)
        output = self._monitor.update(signal="T", value=temperature, timestamp=now)

        emitted = []
        for raw_t, verdict in output.verdicts():
            # Delayed quantitative reports a single number; RoSI reports an
            # interval.  Both are stored as (lower, upper) so everything
            # downstream reads the same columns.
            lower, upper = verdict if isinstance(verdict, tuple) else (verdict, verdict)
            t = time_key(raw_t)
            self.trace[t] = (lower, upper)
            emitted.append((t, lower, upper))

            # `now` is the current time: the verdict for the window starting at
            # t is being reported while the monitor is at now >= t, so now - t
            # is the detection delay.
            if t not in self.decided_at and (upper < 0 or lower > 0):
                self.decided_at[t] = now
            if t not in self.finalised_at and lower == upper:
                self.finalised_at[t] = now

        return emitted

    def get(self, timestamp):
        """Final interval reported for `timestamp`, or the unbounded interval."""
        return self.trace.get(time_key(timestamp), (float("-inf"), float("inf")))

    def decision_time(self, timestamp):
        """When the verdict for `timestamp` became conclusive, or None."""
        return self.decided_at.get(time_key(timestamp))

    def first_violation(self):
        """(attributed_time, detected_at, final_interval) of the earliest violation.

        Returns None if no window was ever conclusively violated.  The two times
        differ: a violation is attributed to the window in which the requirement
        was breached, but can only be detected once enough future signal has
        arrived to rule out a recovery.
        """
        violated = sorted(
            t
            for t, (_, upper) in self.trace.items()
            if upper < 0 and t in self.decided_at
        )
        if not violated:
            return None
        t = violated[0]
        return t, self.decided_at[t], self.trace[t]

    def finalised_trace(self):
        """Timestamps whose robustness interval has collapsed to a point."""
        return {t: v for t, v in self.trace.items() if v[0] == v[1]}


def build_monitors(max_T, min_T):
    """The two monitors fed by the experiment."""
    return [
        SpecMonitor("phi_hi", SPEC_HI, "max_T", max_T),
        SpecMonitor("phi_lo", SPEC_LO, "min_T", min_T),
    ]
