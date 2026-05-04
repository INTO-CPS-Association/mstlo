"""
Online Signal Temporal Logic (STL) monitoring library.

This library provides efficient online monitoring of STL formulas with multiple semantics:

* DelayedQualitative/EagerQualitative: classic true/false evaluation
* DelayedQuantitative: robustness as a single float value
* Rosi: robustness as an interval (min, max)

Examples using `parse_formula` with Rust-style DSL syntax:

    >>> import mstlo_python
    >>> # Parse formula using the same DSL syntax as Rust's stl! macro
    >>> phi = mstlo_python.parse_formula('G[0, 5](x > 0.5)')
    >>> # Create monitor with DelayedQuantitative semantics
    >>> monitor = mstlo_python.Monitor(phi, semantics='DelayedQuantitative')
    >>> # Feed data
    >>> output = monitor.update('x', 1.0, 0.0)
    >>> # Print using Rust's Display formatting
    >>> print(output)
    >>> # Access verdicts
    >>> for ts, val in output.verdicts():
    ...     print(f"t={ts}: {val}")
    >>> # Access structured data
    >>> print(output.to_dict())

Examples using Formula builder methods:

    >>> import mstlo_python
    >>> # Create formula: Always[0,5](x > 0.5)
    >>> phi = mstlo_python.Formula.always(0, 5, mstlo_python.Formula.gt('x', 0.5))
    >>> # Create monitor with Rosi semantics
    >>> monitor = mstlo_python.Monitor(phi, semantics='Rosi')
    >>> # Feed data
    >>> output = monitor.update('x', 1.0, 0.0)
    >>> # Use __str__ and __repr__ for Rust-style formatting
    >>> print(str(output))   # Display format
    >>> print(repr(output))  # Debug format
"""

from typing import Literal, TypedDict, Union, List, Tuple

__version__: str

def parse_formula(formula_str: str) -> "Formula":
    """
    Parse an STL formula from a string using the same DSL syntax as Rust's `stl!` macro.

    This allows you to write formulas using the same syntax as Rust, making it easy
    to port formulas between Python and Rust code.

    **Syntax:**

    *Predicates:*

    * `signal > value` - Signal greater than value
    * `signal < value` - Signal less than value
    * `signal >= value` - Signal greater than or equal
    * `signal <= value` - Signal less than or equal

    *Variable Predicates (for runtime-updateable thresholds):*

    * `signal > $variable` - Signal greater than variable
    * `signal < $variable` - Signal less than variable
    * `signal >= $variable` - Signal greater than or equal to variable
    * `signal <= $variable` - Signal less than or equal to variable

    *Boolean Constants:*

    * `true` - Always true
    * `false` - Always false

    *Unary Operators:*

    * `!(sub)` or `not(sub)` - Negation
    * `G[start, end](sub)` or `globally[start, end](sub)` - Globally (always)
    * `F[start, end](sub)` or `eventually[start, end](sub)` - Eventually (finally)

    *Binary Operators:*

    * `left && right` or `left and right` - Conjunction
    * `left || right` or `left or right` - Disjunction
    * `left -> right` or `left implies right` - Implication
    * `left U[start, end] right` or `left until[start, end] right` - Until

    Args:
        formula_str: A string containing an STL formula

    Returns:
        The parsed Formula object

    Raises:
        ValueError: If the formula string cannot be parsed

    Examples:
        >>> # Simple predicate
        >>> f = parse_formula("x > 5")
        >>> # Globally operator
        >>> f = parse_formula("G[0, 10](x > 5)")
        >>> # Complex formula
        >>> f = parse_formula("G[0, 10](x > 5) && F[0, 5](y < 3)")
        >>> # Using keyword syntax
        >>> f = parse_formula("globally[0, 10](x > 5) and eventually[0, 5](y < 3)")
        >>> # Using variables for runtime-updateable thresholds
        >>> f = parse_formula("G[0, 10](x > $threshold)")
        >>> f = parse_formula("x > $min && x < $max")
    """
    ...

class OutputDict(TypedDict):
    """A single output verdict."""

    timestamp: float
    """The timestamp this verdict is for."""
    value: Union[bool, float, Tuple[float, float]]
    """The verdict value. Type depends on monitor semantics:

    * bool for qualitative semantics
    * float for quantitative semantics
    * tuple[float, float] for Rosi semantics (min, max)
    """

class EvaluationDict(TypedDict):
    """Result of evaluating a synchronized step."""

    sync_step_signal: str
    """Signal name of the synchronized step (may be interpolated)."""
    sync_step_timestamp: float
    """Timestamp of the synchronized step."""
    sync_step_value: float
    """Value of the synchronized step."""
    outputs: List[OutputDict]
    """List of output verdicts produced by evaluating this sync step."""

class MonitorOutputDict(TypedDict):
    """Output from a monitor update operation."""

    input_signal: str
    """The signal name that was updated."""
    input_timestamp: float
    """The timestamp of the input that triggered this update."""
    evaluations: List[EvaluationDict]
    """List of evaluations, one for each synchronized step. May be empty if data is buffered."""

class Variables:
    """
    Container for runtime variable values.

    Variables allow you to define STL formulas with dynamic thresholds that
    can be updated at runtime. Use the `$variable` syntax in formulas to
    reference variables.

    Examples:
        >>> # Create variables and set initial values
        >>> vars = Variables()
        >>> vars.set("threshold", 5.0)
        >>> vars.set("limit", 10.0)
        >>>
        >>> # Parse a formula using variables
        >>> formula = parse_formula("x > $threshold && y < $limit")
        >>>
        >>> # Create a monitor with variables
        >>> monitor = Monitor(formula, variables=vars)
        >>>
        >>> # Update a variable value at runtime
        >>> vars.set("threshold", 7.0)  # Affects future evaluations

    Note:
        Variable predicates require the Incremental algorithm.
        Using variables with the Naive algorithm will raise an error.
    """

    def __init__(self) -> None:
        """
        Create a new empty Variables container.

        Examples:
            >>> vars = Variables()
        """
        ...

    def set(self, name: str, value: float) -> None:
        """
        Set a variable to a value.

        Args:
            name: The variable name (without $ prefix)
            value: The variable value

        Examples:
            >>> vars = Variables()
            >>> vars.set("threshold", 5.0)
        """
        ...

    def get(self, name: str) -> Union[float, None]:
        """
        Get a variable's value.

        Args:
            name: The variable name (without $ prefix)

        Returns:
            The variable's value, or None if not set.

        Examples:
            >>> vars = Variables()
            >>> vars.set("x", 5.0)
            >>> print(vars.get("x"))  # prints 5.0
            >>> print(vars.get("y"))  # prints None
        """
        ...

    def contains(self, name: str) -> bool:
        """
        Check if a variable exists.

        Args:
            name: The variable name (without $ prefix)

        Returns:
            True if the variable exists, False otherwise.

        Examples:
            >>> vars = Variables()
            >>> vars.set("x", 5.0)
            >>> vars.contains("x")  # True
            >>> vars.contains("y")  # False
        """
        ...

    def names(self) -> List[str]:
        """
        Get a list of all variable names.

        Returns:
            A list of variable names.

        Examples:
            >>> vars = Variables()
            >>> vars.set("a", 1.0)
            >>> vars.set("b", 2.0)
            >>> vars.names()  # ['a', 'b']
        """
        ...

    def remove(self, name: str) -> Union[float, None]:
        """
        Remove a variable.

        Args:
            name: The variable name to remove (without $ prefix)

        Returns:
            The variable's previous value, or None if it didn't exist.

        Examples:
            >>> vars = Variables()
            >>> vars.set("x", 5.0)
            >>> vars.remove("x")  # returns 5.0
            >>> vars.remove("x")  # returns None
        """
        ...

    def clear(self) -> None:
        """
        Remove all variables.

        Examples:
            >>> vars = Variables()
            >>> vars.set("x", 5.0)
            >>> vars.clear()
            >>> vars.names()  # []
        """
        ...

    def __str__(self) -> str:
        """Return string representation of the Variables."""
        ...

    def __repr__(self) -> str:
        """Return detailed representation of the Variables."""
        ...

class Formula:
    """
    Signal Temporal Logic (STL) formula.

    Use static methods to construct formulas. Formulas can be composed using
    boolean and temporal operators.

    Examples:
        >>> # G[0,5](x > 0.5)
        >>> phi = Formula.always(0, 5, Formula.gt('x', 0.5))
        >>> # (x > 0.3) AND F[0,3](y < 0.8)
        >>> phi2 = Formula.and_(
        ...     Formula.gt('x', 0.3),
        ...     Formula.eventually(0, 3, Formula.lt('y', 0.8))
        ... )
        >>> # Using variables for dynamic thresholds
        >>> phi3 = Formula.gt_var('x', 'threshold')
    """

    @staticmethod
    def gt(signal: str, value: float) -> "Formula":
        """
        Create a greater-than atomic predicate.

        Args:
            signal: Name of the signal to compare
            value: Threshold value to compare against

        Returns:
            Formula representing: `signal > value`

        Examples:
            >>> Formula.gt('x', 0.5)  # x > 0.5
        """
        ...

    @staticmethod
    def lt(signal: str, value: float) -> "Formula":
        """
        Create a less-than atomic predicate.

        Args:
            signal: Name of the signal to compare
            value: Threshold value to compare against

        Returns:
            Formula representing: `signal < value`

        Examples:
            >>> Formula.lt('x', 0.5)  # x < 0.5
        """
        ...

    @staticmethod
    def gt_var(signal: str, variable: str) -> "Formula":
        """
        Create a greater-than atomic predicate with a variable threshold.

        The variable value is looked up at runtime from a Variables object,
        allowing dynamic thresholds that can be updated on-the-fly.

        Args:
            signal: Name of the signal to compare
            variable: Name of the variable (without $ prefix)

        Returns:
            Formula representing: `signal > $variable`

        Note:
            Requires Incremental algorithm.
            The Naive algorithm does not support variables.

        Examples:
            >>> Formula.gt_var('x', 'threshold')  # x > $threshold
        """
        ...

    @staticmethod
    def lt_var(signal: str, variable: str) -> "Formula":
        """
        Create a less-than atomic predicate with a variable threshold.

        The variable value is looked up at runtime from a Variables object,
        allowing dynamic thresholds that can be updated on-the-fly.

        Args:
            signal: Name of the signal to compare
            variable: Name of the variable (without $ prefix)

        Returns:
            Formula representing: `signal < $variable`

        Note:
            Requires Incremental algorithm.
            The Naive algorithm does not support variables.

        Examples:
            >>> Formula.lt_var('y', 'limit')  # y < $limit
        """
        ...

    @staticmethod
    def true_() -> "Formula":
        """
        Create a constant true formula.

        Returns:
            Formula that is always satisfied (⊤)

        Examples:
            >>> Formula.true_()
        """
        ...

    @staticmethod
    def false_() -> "Formula":
        """
        Create a constant false formula.

        Returns:
            Formula that is never satisfied (⊥)

        Examples:
            >>> Formula.false_()
        """
        ...

    @staticmethod
    def and_(left: "Formula", right: "Formula") -> "Formula":
        """
        Create a conjunction (AND) of two formulas.

        Args:
            left: First formula
            right: Second formula

        Returns:
            Formula representing: `left ∧ right`

        Examples:
            >>> Formula.and_(Formula.gt('x', 0.5), Formula.lt('x', 1.0))
        """
        ...

    @staticmethod
    def or_(left: "Formula", right: "Formula") -> "Formula":
        """
        Create a disjunction (OR) of two formulas.

        Args:
            left: First formula
            right: Second formula

        Returns:
            Formula representing: `left ∨ right`

        Examples:
            >>> Formula.or_(Formula.gt('x', 1.0), Formula.lt('x', 0.0))
        """
        ...

    @staticmethod
    def not_(child: "Formula") -> "Formula":
        """
        Create a negation (NOT) of a formula.

        Args:
            child: Formula to negate

        Returns:
            Formula representing: `¬child`

        Examples:
            >>> Formula.not_(Formula.gt('x', 0.5))  # x <= 0.5
        """
        ...

    @staticmethod
    def implies(left: "Formula", right: "Formula") -> "Formula":
        """
        Create an implication between two formulas.

        Args:
            left: Antecedent (condition)
            right: Consequent (result)

        Returns:
            Formula representing: `left → right` (if left then right)

        Examples:
            >>> # If x > 2.0, then y < 1.0
            >>> Formula.implies(Formula.gt('x', 2.0), Formula.lt('y', 1.0))
        """
        ...

    @staticmethod
    def always(start: float, end: float, child: "Formula") -> "Formula":
        """
        Create a globally (always) temporal formula.

        The formula must hold at all time points in the interval [start, end]
        relative to the current time.

        Args:
            start: Start of time interval (seconds)
            end: End of time interval (seconds)
            child: Formula that must hold throughout the interval

        Returns:
            Formula representing: `G[start,end](child)`

        Examples:
            >>> # For the next 5 seconds, x must be > 0.5
            >>> Formula.always(0, 5, Formula.gt('x', 0.5))
        """
        ...

    @staticmethod
    def eventually(start: float, end: float, child: "Formula") -> "Formula":
        """
        Create an eventually (finally) temporal formula.

        The formula must hold at some time point in the interval [start, end]
        relative to the current time.

        Args:
            start: Start of time interval (seconds)
            end: End of time interval (seconds)
            child: Formula that must hold at some point in the interval

        Returns:
            Formula representing: `F[start,end](child)`

        Examples:
            >>> # Within 3 seconds, y must drop below 0.8
            >>> Formula.eventually(0, 3, Formula.lt('y', 0.8))
        """
        ...

    @staticmethod
    def until(start: float, end: float, left: "Formula", right: "Formula") -> "Formula":
        """
        Create an until temporal formula.

        The left formula must hold until the right formula becomes true,
        and the right formula must become true within the interval [start, end].

        Args:
            start: Start of time interval (seconds)
            end: End of time interval (seconds)
            left: Formula that must hold until right becomes true
            right: Formula that must eventually become true

        Returns:
            Formula representing: `left U[start,end] right`

        Examples:
            >>> # x > 0 must hold until y < 0.5 (within 5 seconds)
            >>> Formula.until(0, 5, Formula.gt('x', 0), Formula.lt('y', 0.5))
        """
        ...

    def __str__(self) -> str:
        """Return string representation of the formula."""
        ...

    def __repr__(self) -> str:
        """Return detailed representation of the formula."""
        ...

SemanticsType = Literal[
    "DelayedQualitative", "EagerQualitative", "DelayedQuantitative", "Rosi"
]
AlgorithmType = Literal["Incremental", "Naive"]
SynchronizationType = Literal["ZeroOrderHold", "Linear", "None"]

class MonitorOutput:
    """
    Output from a monitor update operation.

    This class wraps the Rust MonitorOutput structure and provides:

    * Rust-style Display formatting via `__str__()`
    * Rust-style Debug formatting via `__repr__()`
    * Structured data access via `to_dict()`
    * Convenient properties and methods

    The string representation shows verdicts in the format:
    `t={timestamp}: {value}`

    For multiple verdicts, they are shown on separate lines.
    If no verdicts are available, it shows "No verdicts available".

    Examples:
        >>> output = monitor.update('x', 1.0, 0.0)
        >>> # Rust-style Display
        >>> print(output)
        t=5s: 0.5
        >>> # Access properties
        >>> print(output.input_signal, output.input_timestamp)
        >>> # Get verdicts
        >>> for ts, val in output.verdicts():
        ...     print(f"Verdict at {ts}: {val}")
    """

    @property
    def input_signal(self) -> str:
        """The name of the signal that was updated."""
        ...

    @property
    def input_timestamp(self) -> float:
        """The timestamp (in seconds) of the input that triggered this update."""
        ...

    @property
    def input_value(self) -> float:
        """The value of the input signal."""
        ...

    def has_verdicts(self) -> bool:
        """
        Check if there are any output verdicts.

        Returns:
            True if there is at least one output verdict, False otherwise.
        """
        ...

    def total_raw_outputs(self) -> int:
        """
        Get the total number of raw output verdicts across all evaluations.

        Returns:
            The total count of raw output verdicts.
        """
        ...

    def is_pending(self) -> bool:
        """
        Check if the evaluations list is empty (i.e., data is being buffered).

        Returns:
            True if no evaluations were triggered, False otherwise.
        """
        ...

    def verdicts(self) -> List[Tuple[float, Union[bool, float, Tuple[float, float]]]]:
        """
        Get the finalized verdicts as a list of (timestamp, value) tuples.

        This returns the latest verdict for each unique timestamp.

        Returns:
            List of (timestamp, value) tuples. The value type depends on the
            monitor semantics:

            * bool for qualitative semantics
            * float for robustness semantics
            * tuple[float, float] for Rosi semantics
        """
        ...

    def has_outputs(self) -> bool:
        """
        .. deprecated::
            Use :meth:`has_verdicts` instead.

        Check if there are any output verdicts.
        """
        ...

    def total_outputs(self) -> int:
        """
        .. deprecated::
            Use :meth:`total_raw_outputs` instead.

        Get the total number of output verdicts.
        """
        ...

    def is_empty(self) -> bool:
        """
        .. deprecated::
            Use :meth:`is_pending` instead.

        Check if the evaluations list is empty.
        """
        ...

    def finalize(self) -> List[Tuple[float, Union[bool, float, Tuple[float, float]]]]:
        """
        .. deprecated::
            Use :meth:`verdicts` instead.

        Get the finalized verdicts as a list of (timestamp, value) tuples.
        """
        ...

    def to_dict(self) -> MonitorOutputDict:
        """
        Convert the monitor output to a dictionary.

        Returns:
            Dictionary containing:
            * 'input_signal': the signal name
            * 'input_timestamp': the input timestamp
            * 'input_value': the input value
            * 'evaluations': list of evaluation dictionaries

        Examples:
            >>> output = monitor.update('x', 1.0, 0.0)
            >>> d = output.to_dict()
            >>> print(d['input_signal'], d['input_timestamp'])
        """
        ...

    def __str__(self) -> str:
        """
        Return Rust-style Display representation of the output.
        Shows verdicts in the format: `t={timestamp}: {value}`
        For multiple verdicts, they are shown on separate lines.
        If no verdicts are available, returns "No verdicts available".
        """
        ...

    def __repr__(self) -> str:
        """
        Return Rust-style Debug representation of the output.
        Shows the full internal structure of the MonitorOutput,
        matching Rust's Debug formatting.
        """
        ...

class Monitor:
    """
    Online STL monitor.

    Monitors signal traces against an STL formula and produces verdicts.
    Supports multiple semantics (DelayedQualitative, EagerQualitative, DelayedQuantitative, Rosi),
    algorithms (Incremental, Naive), and synchronization strategies (ZeroOrderHold, Linear, None).

    The monitor processes signals incrementally and produces verdicts when
    sufficient information is available.

    Examples:
        >>> phi = Formula.always(0, 5, Formula.gt('x', 0.5))
        >>> monitor = Monitor(phi, semantics='Rosi')
        >>> for t in range(10):
        ...     result = monitor.update('x', 0.8, float(t))
        ...     print(result)
    """

    def __init__(
        self,
        formula: Formula,
        semantics: SemanticsType = "DelayedQuantitative",
        algorithm: AlgorithmType = "Incremental",
        synchronization: SynchronizationType = "ZeroOrderHold",
        variables: Union[Variables, None] = None,
    ) -> None:
        """
        Create a new STL monitor.

        Args:
            formula: The STL formula to monitor
            semantics: Output semantics. Options:

                * "DelayedQualitative": Returns True/False with delayed evaluation
                * "EagerQualitative": Returns True/False with eager evaluation
                * "DelayedQuantitative": Returns float robustness value (+ = satisfied, - = violated, default)
                * "Rosi": Returns (min, max) robustness interval

            algorithm: Monitoring algorithm. Options:

                * "Incremental": Efficient online monitoring with sliding windows (default)
                * "Naive": Simple baseline implementation

            synchronization: Signal synchronization method. Options:

                * "ZeroOrderHold": Zero-order hold (default)
                * "Linear": Linear interpolation
                * "None": No interpolation

            variables: A Variables object containing runtime variable values.
                Required if the formula contains variable predicates (e.g., `x > $threshold`).
                Note: Variable predicates require the Incremental algorithm.

        Raises:
            ValueError: If invalid semantics, algorithm, or synchronization is specified
            ValueError: If Naive algorithm is used with EagerQualitative (not supported)
            ValueError: If Naive algorithm is used with variable predicates (not supported)

        Note:
            For single-signal formulas, signal synchronization is automatically disabled
            for better performance.

        Examples:
            >>> # DelayedQualitative monitoring
            >>> m1 = Monitor(phi, semantics="DelayedQualitative", algorithm="Incremental")
            >>>
            >>> # DelayedQuantitative (quantitative)
            >>> m2 = Monitor(phi, semantics="DelayedQuantitative")
            >>>
            >>> # Rosi intervals
            >>> m3 = Monitor(phi, semantics="Rosi")
            >>>
            >>> # Using variables for dynamic thresholds
            >>> vars = Variables()
            >>> vars.set("threshold", 5.0)
            >>> phi = parse_formula("G[0, 5](x > $threshold)")
            >>> m4 = Monitor(phi, variables=vars)
        """
        ...

    def update(self, signal: str, value: float, timestamp: float) -> MonitorOutput:
        """
        Update the monitor with a new signal value.

        Processes the new data point and produces verdicts when sufficient
        information is available. Each update may trigger multiple evaluations,
        one for each synchronized step (which may include interpolated values).

        Args:
            signal: Name of the signal being updated
            value: Signal value at this timestamp
            timestamp: Timestamp in seconds (must be monotonically increasing)

        Returns:
            MonitorOutput object containing:

            * Display/Debug formatting via `__str__()` and `__repr__()`
            * Properties: input_signal, input_timestamp, input_value
            * Methods: has_verdicts(), total_raw_outputs(), is_pending(), verdicts()
            * Structured data access via `to_dict()`

        Note:
            * Evaluations list may be empty if data is being buffered
            * For multi-signal formulas, the synchronizer may produce interpolated steps
            * Each evaluation corresponds to a specific synchronized step
            * Multiple outputs may be produced for each synchronized step

        Examples:
            >>> output = monitor.update('x', 0.8, 1.0)
            >>> # Use Rust-style Display formatting
            >>> print(output)
            >>> # Access properties
            >>> print(f"Input: {output.input_signal} at t={output.input_timestamp}")
            >>> # Get verdicts
            >>> for ts, val in output.verdicts():
            ...     print(f"Verdict at {ts}: {val}")
            >>> # Access as dictionary
            >>> d = output.to_dict()
            >>> for eval in d['evaluations']:
            ...     for out in eval['outputs']:
            ...         print(f"  t={out['timestamp']}: {out['value']}")
        """
        ...

    def get_signal_identifiers(self) -> set[str]:
        """
        Get the set of signal identifiers used in the monitor's formula.

        Returns:
            Set[str]: A set of signal names (identifiers) used in the formula.

        Examples:
            >>> phi = parse_formula("G[0,5](x > 5) && F[0,3](y < 10)")
            >>> monitor = Monitor(phi)
            >>> signals = monitor.get_signal_identifiers()
            >>> print(signals)  # {'x', 'y'}
        """
        ...

    def get_variables(self) -> Variables:
        """
        Get the Variables object associated with this monitor.

        This allows you to update variable values at runtime, which will
        affect future evaluations of the formula.

        Returns:
            The Variables object containing runtime variable values.

        Examples:
            >>> vars = Variables()
            >>> vars.set("threshold", 5.0)
            >>> monitor = Monitor(formula, variables=vars)
            >>> # Update a variable at runtime
            >>> monitor.get_variables().set("threshold", 10.0)
        """
        ...

    def get_specification(self) -> str:
        """
        Get the STL specification as a string representation.

        Returns:
            String representation of the STL formula being monitored.

        Examples:
            >>> phi = parse_formula("G[0,5](x > 5)")
            >>> monitor = Monitor(phi)
            >>> print(monitor.get_specification())  # "G[0s,5s](x > 5)"
        """
        ...

    def get_algorithm(self) -> str:
        """
        Get the algorithm used by this monitor.

        Returns:
            "Incremental" or "Naive"

        Examples:
            >>> monitor = Monitor(phi, algorithm="Incremental")
            >>> print(monitor.get_algorithm())  # "Incremental"
        """
        ...

    def get_semantics(self) -> str:
        """
        Get the semantics used by this monitor.

        Returns:
            One of: "DelayedQualitative", "EagerQualitative", "DelayedQuantitative", or "Rosi"

        Examples:
            >>> monitor = Monitor(phi, semantics="DelayedQuantitative")
            >>> print(monitor.get_semantics())  # "DelayedQuantitative"
        """
        ...

    def get_synchronization_strategy(self) -> str:
        """
        Get the synchronization strategy used by this monitor.

        Returns:
            One of: "ZeroOrderHold", "Linear", or "None"

        Examples:
            >>> monitor = Monitor(phi, synchronization="Linear")
            >>> print(monitor.get_synchronization_strategy())  # "Linear"
        """
        ...

    def get_temporal_depth(self) -> float:
        """
        Get the maximum lookahead required by the formula (temporal depth).

        This corresponds to the maximum time interval in any temporal operator.

        Returns:
            The temporal depth in seconds.

        Examples:
            >>> phi = parse_formula("G[0,5](x > 5) && F[0,3](y < 10)")
            >>> monitor = Monitor(phi)
            >>> print(monitor.get_temporal_depth())  # 5.0
        """
        ...

    def reset(self) -> None:
        """
        Reset the monitor to its initial state.

        Clears all internal caches and evaluation buffers. The formula,
        semantics, algorithm, synchronization strategy, and variables are
        preserved.

        Use this to reuse a monitor across multiple independent traces without
        rebuilding it from scratch.

        Examples:
            >>> monitor = Monitor(formula)
            >>> for signal, value, ts in trace_1:
            ...     monitor.update(signal, value, ts)
            >>> monitor.reset()
            >>> for signal, value, ts in trace_2:
            ...     monitor.update(signal, value, ts)
        """
        ...

    def update_batch(
        self, steps: dict[str, List[Tuple[float, float]]]
    ) -> MonitorOutput:
        """
        Update the monitor with multiple signal values in batch.

        This method processes multiple data points at once. Steps are automatically
        sorted by timestamp in chronological order before processing, which is
        optimal for the Incremental algorithm.

        Args:
            steps: A dictionary mapping signal names to lists of (value, timestamp)
                tuples. Examples: `{"x": [(1.0, 0.0), (2.0, 1.0)], "y": [(5.0, 0.5)]}`

        Returns:
            A single MonitorOutput containing all evaluation results from processing
            the batch. The input metadata reflects the last step processed.

        Raises:
            ValueError: If the steps dictionary is empty or contains no steps.

        Note:
            Steps are processed in chronological order (sorted by timestamp) regardless
            of the order in which signals appear in the dictionary. If you need a
            specific processing order, use `update()` directly.

        Examples:
            >>> steps = {
            ...     "temperature": [(25.0, 1.0), (26.0, 2.0)],
            ...     "pressure": [(101.3, 1.5)]
            ... }
            >>> output = monitor.update_batch(steps)
            >>> print(output)  # Display all finalized verdicts
        """
        ...

    def __repr__(self) -> str:
        """
        Return a brief representation of the monitor.

        Returns:
            String in the format: "Monitor(semantics='...', algorithm='...', synchronization='...')"
        """
        ...

    def __str__(self) -> str:
        """
        Return the complete monitor configuration using Rust's Display trait.

        This provides a detailed view of all monitor configuration including:
        - STL specification formula
        - Algorithm (Naive/Incremental)
        - Semantics (DelayedQuantitative/Rosi/DelayedQualitative/EagerQualitative)
        - Synchronization strategy
        - Temporal depth
        - Variables (if any are defined)

        Returns:
            Formatted string showing complete monitor configuration.

        Examples:
            >>> phi = parse_formula("G[0,5](x > $threshold)")
            >>> vars = Variables()
            >>> vars.set("threshold", 10.0)
            >>> monitor = Monitor(phi, semantics="DelayedQuantitative", variables=vars)
            >>> print(monitor)
            STL Monitor Configuration:
              Specification: G[0s,5s](x > 10)
              Algorithm: Incremental
              Semantics: DelayedQuantitative
              Synchronization: ZeroOrderHold
              Temporal Depth: 5s
              Variables:
                $threshold = 10
        """
        ...
