"""
Comprehensive test suite for mstlo_python.

Tests all formula types, monitor types, and error handling.
"""

import pytest
import mstlo_python.mstlo_python as mstlo


class TestParseFormula:
    """Test the parse_formula function that uses the same DSL as Rust's stl! macro."""

    def test_simple_greater_than(self):
        """Test parsing a simple greater-than predicate."""
        formula = mstlo.parse_formula("x > 5")
        assert formula is not None
        assert "x" in str(formula)

    def test_simple_less_than(self):
        """Test parsing a simple less-than predicate."""
        formula = mstlo.parse_formula("y < 3.14")
        assert formula is not None
        assert "y" in str(formula)

    def test_greater_equal(self):
        """Test parsing greater-than-or-equal predicate."""
        formula = mstlo.parse_formula("x >= 5")
        assert formula is not None

    def test_less_equal(self):
        """Test parsing less-than-or-equal predicate."""
        formula = mstlo.parse_formula("x <= 5")
        assert formula is not None

    def test_boolean_true(self):
        """Test parsing true constant."""
        formula = mstlo.parse_formula("true")
        assert formula is not None

    def test_boolean_false(self):
        """Test parsing false constant."""
        formula = mstlo.parse_formula("false")
        assert formula is not None

    def test_globally(self):
        """Test parsing globally operator."""
        formula = mstlo.parse_formula("G[0, 10](x > 5)")
        assert formula is not None
        assert "G" in str(formula)

    def test_globally_keyword(self):
        """Test parsing globally keyword syntax."""
        formula = mstlo.parse_formula("globally[0, 10](x > 5)")
        assert formula is not None

    def test_eventually(self):
        """Test parsing eventually operator."""
        formula = mstlo.parse_formula("F[0, 5](y < 3)")
        assert formula is not None
        assert "F" in str(formula)

    def test_eventually_keyword(self):
        """Test parsing eventually keyword syntax."""
        formula = mstlo.parse_formula("eventually[0, 5](y < 3)")
        assert formula is not None

    def test_and_symbols(self):
        """Test parsing conjunction with && symbols."""
        formula = mstlo.parse_formula("x > 5 && y < 3")
        assert formula is not None

    def test_and_keyword(self):
        """Test parsing conjunction with 'and' keyword."""
        formula = mstlo.parse_formula("x > 5 and y < 3")
        assert formula is not None

    def test_or_symbols(self):
        """Test parsing disjunction with || symbols."""
        formula = mstlo.parse_formula("x > 5 || y < 3")
        assert formula is not None

    def test_or_keyword(self):
        """Test parsing disjunction with 'or' keyword."""
        formula = mstlo.parse_formula("x > 5 or y < 3")
        assert formula is not None

    def test_not_symbol(self):
        """Test parsing negation with ! symbol."""
        formula = mstlo.parse_formula("!(x > 5)")
        assert formula is not None

    def test_not_keyword(self):
        """Test parsing negation with 'not' keyword."""
        formula = mstlo.parse_formula("not(x > 5)")
        assert formula is not None

    def test_implies_symbol(self):
        """Test parsing implication with -> symbol."""
        formula = mstlo.parse_formula("x > 5 -> y < 3")
        assert formula is not None

    def test_implies_keyword(self):
        """Test parsing implication with 'implies' keyword."""
        formula = mstlo.parse_formula("x > 5 implies y < 3")
        assert formula is not None

    def test_until_symbol(self):
        """Test parsing until operator with U symbol."""
        formula = mstlo.parse_formula("x > 5 U[0, 10] y < 3")
        assert formula is not None

    def test_until_keyword(self):
        """Test parsing until operator with 'until' keyword."""
        formula = mstlo.parse_formula("x > 5 until[0, 10] y < 3")
        assert formula is not None

    def test_nested_temporal(self):
        """Test parsing nested temporal operators."""
        formula = mstlo.parse_formula("G[0, 10](F[0, 5](x > 0))")
        assert formula is not None

    def test_complex_formula(self):
        """Test parsing complex formula."""
        formula = mstlo.parse_formula("G[0, 10](x > 5) && F[0, 5](y < 3)")
        assert formula is not None

    def test_whitespace_tolerance(self):
        """Test that parser handles extra whitespace."""
        formula = mstlo.parse_formula("  G  [  0  ,  10  ]  (  x  >  5  )  ")
        assert formula is not None

    def test_decimal_interval(self):
        """Test parsing intervals with decimal values."""
        formula = mstlo.parse_formula("G[0.5, 10.5](x > 5)")
        assert formula is not None

    def test_signal_with_underscore(self):
        """Test parsing signal names with underscores."""
        formula = mstlo.parse_formula("my_signal > 5")
        assert formula is not None
        assert "my_signal" in str(formula)

    def test_negative_threshold(self):
        """Test parsing predicates with negative thresholds."""
        formula = mstlo.parse_formula("x > -5")
        assert formula is not None

    def test_error_empty_input(self):
        """Test that empty input raises an error."""
        with pytest.raises(ValueError):
            mstlo.parse_formula("")

    def test_error_invalid_syntax(self):
        """Test that invalid syntax raises an error."""
        with pytest.raises(ValueError):
            mstlo.parse_formula("not a valid formula !!!")

    def test_error_missing_interval(self):
        """Test that missing interval raises an error."""
        with pytest.raises(ValueError):
            mstlo.parse_formula("G(x > 5)")

    def test_with_monitor(self):
        """Test using parse_formula with a monitor."""
        formula = mstlo.parse_formula("G[0, 5](x > 0.5)")
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")
        output = monitor.update("x", 1.0, 0.0)
        # Test new MonitorOutput properties
        assert output.input_signal == "x"
        assert output.input_timestamp == 0.0
        # Test to_dict() still works
        result = output.to_dict()
        assert "evaluations" in result

    def test_complex_with_monitor(self):
        """Test complex parsed formula with a monitor."""
        formula = mstlo.parse_formula("G[0, 5](x > 0) && F[0, 3](y < 10)")
        monitor = mstlo.Monitor(formula, semantics="Rosi")
        output = monitor.update("x", 1.0, 0.0)
        # Test new MonitorOutput properties
        assert output.input_signal == "x"
        # Test to_dict() still works
        result = output.to_dict()
        assert "evaluations" in result


class TestFormulaCreation:
    """Test that all formula types can be created."""

    def test_gt_formula(self):
        """Test greater-than atomic predicate."""
        formula = mstlo.Formula.gt("x", 5.0)
        assert formula is not None
        assert "x" in str(formula)

    def test_lt_formula(self):
        """Test less-than atomic predicate."""
        formula = mstlo.Formula.lt("y", 10.0)
        assert formula is not None
        assert "y" in str(formula)

    def test_true_formula(self):
        """Test constant true formula."""
        formula = mstlo.Formula.true_()
        assert formula is not None

    def test_false_formula(self):
        """Test constant false formula."""
        formula = mstlo.Formula.false_()
        assert formula is not None

    def test_and_formula(self):
        """Test conjunction of formulas."""
        f1 = mstlo.Formula.gt("x", 1.0)
        f2 = mstlo.Formula.lt("x", 10.0)
        formula = mstlo.Formula.and_(f1, f2)
        assert formula is not None

    def test_or_formula(self):
        """Test disjunction of formulas."""
        f1 = mstlo.Formula.gt("x", 5.0)
        f2 = mstlo.Formula.lt("y", 3.0)
        formula = mstlo.Formula.or_(f1, f2)
        assert formula is not None

    def test_not_formula(self):
        """Test negation of formula."""
        f = mstlo.Formula.gt("x", 5.0)
        formula = mstlo.Formula.not_(f)
        assert formula is not None

    def test_implies_formula(self):
        """Test implication."""
        f1 = mstlo.Formula.gt("x", 0.0)
        f2 = mstlo.Formula.lt("y", 100.0)
        formula = mstlo.Formula.implies(f1, f2)
        assert formula is not None

    def test_always_formula(self):
        """Test globally (always) temporal operator."""
        f = mstlo.Formula.gt("x", 5.0)
        formula = mstlo.Formula.always(0.0, 10.0, f)
        assert formula is not None

    def test_eventually_formula(self):
        """Test eventually (finally) temporal operator."""
        f = mstlo.Formula.lt("x", 5.0)
        formula = mstlo.Formula.eventually(0.0, 5.0, f)
        assert formula is not None

    def test_until_formula(self):
        """Test until temporal operator."""
        f1 = mstlo.Formula.gt("x", 0.0)
        f2 = mstlo.Formula.gt("x", 10.0)
        formula = mstlo.Formula.until(0.0, 5.0, f1, f2)
        assert formula is not None

    def test_complex_formula(self):
        """Test complex nested formula."""
        # Always[0,5](x > 0.5 AND Eventually[0,2](y < 0.8))
        f1 = mstlo.Formula.gt("x", 0.5)
        f2 = mstlo.Formula.lt("y", 0.8)
        f3 = mstlo.Formula.eventually(0.0, 2.0, f2)
        f4 = mstlo.Formula.and_(f1, f3)
        formula = mstlo.Formula.always(0.0, 5.0, f4)
        assert formula is not None


class TestMonitorCreation:
    """Test that all monitor types can be created."""

    def test_monitor_default(self):
        """Test monitor with default parameters."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula)
        assert monitor is not None
        assert "DelayedQuantitative" in repr(monitor)

    def test_boolean_monitor_incremental_eager(self):
        """Test boolean monitor with explicit parameters."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor_eager = mstlo.Monitor(
            formula, semantics="EagerQualitative", algorithm="Incremental"
        )
        assert monitor_eager is not None

    def test_boolean_monitor_incremental_strict(self):
        """Test boolean monitor with incremental algorithm and strict semantics."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor_strict = mstlo.Monitor(
            formula, semantics="DelayedQualitative", algorithm="Incremental"
        )
        assert monitor_strict is not None

    def test_boolean_monitor_naive(self):
        """Test boolean monitor with naive algorithm."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(
            formula, semantics="DelayedQualitative", algorithm="Naive"
        )
        assert monitor is not None

    def test_quantitative_monitor(self):
        """Test quantitative monitor."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")
        assert monitor is not None
        assert "DelayedQuantitative" in repr(monitor)

    def test_DelayedQuantitative_monitor(self):
        """Test Rosi monitor."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="Rosi")
        assert monitor is not None
        assert "Rosi" in repr(monitor)

    def test_DelayedQuantitative_monitor_eager(self):
        """Test Rosi monitor."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="Rosi")
        assert monitor is not None


class TestMonitorUpdate:
    """Test that monitor update works for all semantics."""

    def test_boolean_update(self):
        """Test boolean monitor update."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        output = monitor.update("x", 10.0, 0.0)

        # Test new MonitorOutput properties
        assert output.input_signal == "x"
        assert output.input_timestamp == 0.0
        assert output.input_value == 10.0

        # Test to_dict() for structured access
        result = output.to_dict()
        assert "input_signal" in result
        assert result["input_signal"] == "x"
        assert "input_timestamp" in result
        assert result["input_timestamp"] == 0.0
        assert "evaluations" in result
        assert isinstance(result["evaluations"], list)

        # Check structure of evaluations
        if len(result["evaluations"]) > 0:
            eval_dict = result["evaluations"][0]
            assert "sync_step_signal" in eval_dict
            assert "sync_step_timestamp" in eval_dict
            assert "sync_step_value" in eval_dict
            assert "outputs" in eval_dict

            if len(eval_dict["outputs"]) > 0:
                out = eval_dict["outputs"][0]
                assert "timestamp" in out
                assert "value" in out
                assert isinstance(out["value"], bool)

    def test_quantitative_update(self):
        """Test quantitative monitor update."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

        output = monitor.update("x", 10.0, 0.0)

        # Test new MonitorOutput properties
        assert output.input_signal == "x"

        # Test to_dict() for structured access
        result = output.to_dict()
        assert "input_signal" in result
        assert "evaluations" in result

        if len(result["evaluations"]) > 0:
            eval_dict = result["evaluations"][0]
            if len(eval_dict["outputs"]) > 0:
                out = eval_dict["outputs"][0]
                assert isinstance(out["value"], float)

    def test_Rosi_update(self):
        """Test Rosi monitor update."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="Rosi")

        output = monitor.update("x", 10.0, 0.0)

        # Test new MonitorOutput properties
        assert output.input_signal == "x"

        # Test to_dict() for structured access
        result = output.to_dict()
        assert "input_signal" in result
        assert "evaluations" in result

        if len(result["evaluations"]) > 0:
            eval_dict = result["evaluations"][0]
            if len(eval_dict["outputs"]) > 0:
                out = eval_dict["outputs"][0]
                assert isinstance(out["value"], tuple)
                assert len(out["value"]) == 2
                assert isinstance(out["value"][0], float)
                assert isinstance(out["value"][1], float)

    def test_multiple_updates(self):
        """Test multiple sequential updates."""
        formula = mstlo.Formula.always(0.0, 2.0, mstlo.Formula.gt("x", 5.0))
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        for t in range(5):
            output = monitor.update("x", 10.0, float(t))
            assert output is not None
            assert output.input_timestamp == float(t)

    def test_multi_signal_update(self):
        """Test updates with multiple signals."""
        f1 = mstlo.Formula.gt("x", 5.0)
        f2 = mstlo.Formula.lt("y", 10.0)
        formula = mstlo.Formula.and_(f1, f2)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        output1 = monitor.update("x", 10.0, 0.0)
        assert output1.input_signal == "x"

        output2 = monitor.update("y", 5.0, 0.0)
        assert output2.input_signal == "y"


class TestBatchUpdate:
    """Test batch update functionality."""

    def test_batch_update_single_signal(self):
        """Test batch update with a single signal."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        steps = {"x": [(5.0, 0.0), (15.0, 1.0), (8.0, 2.0)]}  # (value, timestamp)

        output = monitor.update_batch(steps)

        # Check output properties
        assert output is not None
        assert output.input_timestamp == 2.0  # Last step's timestamp
        assert output.input_signal == "x"

        # Check verdicts
        verdicts = output.verdicts()
        assert len(verdicts) == 3
        # 5.0 > 10.0 = False, 15.0 > 10.0 = True, 8.0 > 10.0 = False
        assert verdicts[0][1] is False
        assert verdicts[1][1] is True
        assert verdicts[2][1] is False

    def test_batch_update_DelayedQuantitative(self):
        """Test batch update with DelayedQuantitative semantics."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

        steps = {"x": [(5.0, 0.0), (15.0, 1.0), (8.0, 2.0)]}

        output = monitor.update_batch(steps)

        # Check verdicts
        verdicts = output.verdicts()
        assert len(verdicts) == 3
        # DelayedQuantitative: value - threshold
        assert verdicts[0][1] == -5.0  # 5.0 - 10.0
        assert verdicts[1][1] == 5.0  # 15.0 - 10.0
        assert verdicts[2][1] == -2.0  # 8.0 - 10.0

    def test_batch_update_rosi(self):
        """Test batch update with Rosi semantics."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="Rosi")

        steps = {"x": [(5.0, 0.0), (15.0, 1.0)]}

        output = monitor.update_batch(steps)
        verdicts = output.verdicts()
        assert len(verdicts) >= 1
        # RoSI returns tuples (min, max)
        for ts, val in verdicts:
            assert isinstance(val, tuple)
            assert len(val) == 2

    def test_batch_update_multi_signal(self):
        """Test batch update with multiple signals."""
        f1 = mstlo.Formula.gt("x", 5.0)
        f2 = mstlo.Formula.lt("y", 10.0)
        formula = mstlo.Formula.and_(f1, f2)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        steps = {"x": [(10.0, 0.0), (8.0, 2.0)], "y": [(5.0, 1.0), (3.0, 3.0)]}

        output = monitor.update_batch(steps)
        assert output is not None
        # Input metadata should reflect last chronological step
        assert output.input_timestamp == 3.0

    def test_batch_update_out_of_order_timestamps(self):
        """Test that batch update handles out-of-order timestamps correctly."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        # Provide steps out of chronological order
        steps = {"x": [(15.0, 2.0), (5.0, 0.0), (8.0, 1.0)]}  # Out of order

        output = monitor.update_batch(steps)

        # Should still work - steps are sorted internally
        assert output is not None
        verdicts = output.verdicts()
        assert len(verdicts) == 3
        # Verdicts should be in chronological order
        assert verdicts[0][0] == 0.0
        assert verdicts[1][0] == 1.0
        assert verdicts[2][0] == 2.0

    def test_batch_update_empty_raises_error(self):
        """Test that empty batch raises ValueError."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        with pytest.raises(ValueError, match="at least one step"):
            monitor.update_batch({})

    def test_batch_update_empty_lists_raises_error(self):
        """Test that batch with empty lists raises ValueError."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        with pytest.raises(ValueError, match="at least one step"):
            monitor.update_batch({"x": []})

    def test_batch_update_to_dict(self):
        """Test that batch update output can be converted to dict."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        steps = {"x": [(15.0, 0.0), (5.0, 1.0)]}
        output = monitor.update_batch(steps)

        result = output.to_dict()
        assert "input_signal" in result
        assert "input_timestamp" in result
        assert "evaluations" in result
        assert isinstance(result["evaluations"], list)

    def test_batch_update_has_verdicts(self):
        """Test has_verdicts and total_raw_outputs methods on batch output."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        steps = {"x": [(15.0, 0.0), (5.0, 1.0), (20.0, 2.0)]}
        output = monitor.update_batch(steps)

        assert output.has_verdicts() is True
        assert output.total_raw_outputs() == 3

    def test_batch_update_display(self):
        """Test that batch output can be displayed via __str__."""
        formula = mstlo.Formula.gt("x", 10.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        steps = {"x": [(15.0, 0.0)]}
        output = monitor.update_batch(steps)

        # Should not raise and should contain some content
        display_str = str(output)
        assert display_str is not None
        assert len(display_str) > 0


class TestErrorHandling:
    """Test that invalid inputs are properly caught."""

    def test_invalid_semantics(self):
        """Test that invalid semantics raises ValueError."""
        formula = mstlo.Formula.gt("x", 5.0)

        with pytest.raises(ValueError, match="Invalid semantics"):
            mstlo.Monitor(formula, semantics="invalid")

    def test_invalid_algorithm(self):
        """Test that invalid algorithm raises ValueError."""
        formula = mstlo.Formula.gt("x", 5.0)

        with pytest.raises(ValueError, match="Invalid algorithm"):
            mstlo.Monitor(formula, semantics="DelayedQualitative", algorithm="invalid")

    def test_naive_algorithm_with_eager(self):
        """Test that naive algorithm with eager semantics raises error."""
        formula = mstlo.Formula.gt("x", 5.0)

        with pytest.raises(ValueError, match="Naive algorithm does not support"):
            mstlo.Monitor(formula, semantics="EagerQualitative", algorithm="Naive")

    def test_invalid_synchronization(self):
        """Test that invalid synchronization raises ValueError."""
        formula = mstlo.Formula.gt("x", 5.0)

        with pytest.raises(ValueError, match="Invalid synchronization"):
            mstlo.Monitor(
                formula, semantics="DelayedQualitative", synchronization="invalid"
            )


class TestsynchronizationStrategies:
    """Test that all synchronization strategies can be created."""

    def test_zoh_synchronization(self):
        """Test zero-order hold synchronization."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(
            formula, semantics="DelayedQualitative", synchronization="ZeroOrderHold"
        )
        assert monitor is not None
        assert "ZeroOrderHold" in repr(monitor)

    def test_linear_synchronization(self):
        """Test linear synchronization."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(
            formula, semantics="DelayedQualitative", synchronization="Linear"
        )
        assert monitor is not None
        assert "Linear" in repr(monitor)

    def test_none_synchronization(self):
        """Test no synchronization."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(
            formula, semantics="DelayedQualitative", synchronization="None"
        )
        assert monitor is not None
        assert "None" in repr(monitor)

    def test_synchronization_with_quantitative(self):
        """Test synchronization strategies work with quantitative semantics."""
        formula = mstlo.Formula.gt("x", 5.0)

        monitor_zoh = mstlo.Monitor(
            formula, semantics="DelayedQuantitative", synchronization="ZeroOrderHold"
        )
        assert monitor_zoh is not None

        monitor_linear = mstlo.Monitor(
            formula, semantics="DelayedQuantitative", synchronization="Linear"
        )
        assert monitor_linear is not None

        monitor_none = mstlo.Monitor(
            formula, semantics="DelayedQuantitative", synchronization="None"
        )
        assert monitor_none is not None

    def test_synchronization_with_rosi(self):
        """Test synchronization strategies work with Rosi semantics."""
        formula = mstlo.Formula.gt("x", 5.0)

        monitor_zoh = mstlo.Monitor(
            formula, semantics="Rosi", synchronization="ZeroOrderHold"
        )
        assert monitor_zoh is not None

        monitor_linear = mstlo.Monitor(
            formula, semantics="Rosi", synchronization="Linear"
        )
        assert monitor_linear is not None

        monitor_none = mstlo.Monitor(formula, semantics="Rosi", synchronization="None")
        assert monitor_none is not None


class TestTemporalFormulas:
    """Test temporal formulas produce correct structure."""

    def test_always_produces_verdicts(self):
        """Test that Always formula eventually produces verdicts."""
        formula = mstlo.Formula.always(0.0, 2.0, mstlo.Formula.gt("x", 5.0))
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        # Feed data points
        outputs = []
        for t in range(5):
            output = monitor.update("x", 10.0, float(t))
            outputs.append(output)

        # At least some outputs should have evaluations
        total_evals = sum(len(o.to_dict()["evaluations"]) for o in outputs)
        assert total_evals > 0

    def test_eventually_produces_verdicts(self):
        """Test that Eventually formula produces verdicts."""
        formula = mstlo.Formula.eventually(0.0, 2.0, mstlo.Formula.gt("x", 5.0))
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        outputs = []
        for t in range(5):
            output = monitor.update("x", 10.0, float(t))
            outputs.append(output)

        total_evals = sum(len(o.to_dict()["evaluations"]) for o in outputs)
        assert total_evals > 0

    def test_until_produces_verdicts(self):
        """Test that Until formula produces verdicts."""
        f1 = mstlo.Formula.gt("x", 0.0)
        f2 = mstlo.Formula.gt("x", 10.0)
        formula = mstlo.Formula.until(0.0, 3.0, f1, f2)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        outputs = []
        # Start with low value, then high
        for t in range(2):
            output = monitor.update("x", 5.0, float(t))
            outputs.append(output)
        for t in range(2, 5):
            output = monitor.update("x", 15.0, float(t))
            outputs.append(output)

        total_evals = sum(len(o.to_dict()["evaluations"]) for o in outputs)
        assert total_evals > 0


class TestBooleanOperators:
    """Test boolean operators produce correct behavior."""

    def test_and_operator(self):
        """Test AND operator."""
        f1 = mstlo.Formula.gt("x", 5.0)
        f2 = mstlo.Formula.lt("x", 15.0)
        formula = mstlo.Formula.and_(f1, f2)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        # Value in range should satisfy
        result = monitor.update("x", 10.0, 0.0)
        assert result is not None

    def test_or_operator(self):
        """Test OR operator."""
        f1 = mstlo.Formula.gt("x", 100.0)
        f2 = mstlo.Formula.lt("x", 5.0)
        formula = mstlo.Formula.or_(f1, f2)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        result = monitor.update("x", 2.0, 0.0)
        assert result is not None

    def test_not_operator(self):
        """Test NOT operator."""
        f = mstlo.Formula.gt("x", 5.0)
        formula = mstlo.Formula.not_(f)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        result = monitor.update("x", 2.0, 0.0)
        assert result is not None

    def test_implies_operator(self):
        """Test IMPLIES operator."""
        f1 = mstlo.Formula.gt("x", 5.0)
        f2 = mstlo.Formula.lt("y", 10.0)
        formula = mstlo.Formula.implies(f1, f2)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        result = monitor.update("x", 10.0, 0.0)
        assert result is not None


class TestEvaluationStructure:
    """Test the structure of evaluation results."""

    def test_evaluation_has_sync_step_info(self):
        """Test that evaluations include sync step information."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        output = monitor.update("x", 10.0, 0.0)
        result = output.to_dict()

        if len(result["evaluations"]) > 0:
            eval_dict = result["evaluations"][0]

            # Check all required fields are present
            assert "sync_step_signal" in eval_dict
            assert "sync_step_timestamp" in eval_dict
            assert "sync_step_value" in eval_dict
            assert "outputs" in eval_dict

            # Check types
            assert isinstance(eval_dict["sync_step_signal"], str)
            assert isinstance(eval_dict["sync_step_timestamp"], float)
            assert isinstance(eval_dict["sync_step_value"], float)
            assert isinstance(eval_dict["outputs"], list)

    def test_output_structure(self):
        """Test that outputs have correct structure."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="Rosi")

        output = monitor.update("x", 10.0, 0.0)
        result = output.to_dict()

        if len(result["evaluations"]) > 0:
            eval_dict = result["evaluations"][0]
            if len(eval_dict["outputs"]) > 0:
                out = eval_dict["outputs"][0]

                assert "timestamp" in out
                assert "value" in out
                assert isinstance(out["timestamp"], float)


class TestMonitorOutputFormatting:
    """Test MonitorOutput __str__ and __repr__ methods."""

    def test_str_with_verdicts(self):
        """Test __str__ when verdicts are available."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

        output = monitor.update("x", 10.0, 0.0)

        # __str__ should return Rust-style Display formatting
        str_repr = str(output)
        # The format should be "t={timestamp}: {value}" or "No verdicts available"
        assert isinstance(str_repr, str)
        # If there are verdicts, they should contain 't='
        if output.has_verdicts():
            assert "t=" in str_repr

    def test_str_without_verdicts(self):
        """Test __str__ when no verdicts are available yet."""
        formula = mstlo.Formula.always(0.0, 10.0, mstlo.Formula.gt("x", 5.0))
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

        # First update may not produce verdicts yet
        output = monitor.update("x", 10.0, 0.0)

        str_repr = str(output)
        assert isinstance(str_repr, str)
        # If no verdicts, should say so
        if not output.has_verdicts():
            assert "No verdicts available" in str_repr

    def test_repr_format(self):
        """Test __repr__ returns Rust Debug format."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

        output = monitor.update("x", 10.0, 0.0)

        # __repr__ should return Rust-style Debug formatting
        repr_str = repr(output)
        assert isinstance(repr_str, str)
        # Debug format typically includes struct name
        assert "MonitorOutput" in repr_str

    def test_str_multiple_verdicts(self):
        """Test __str__ with multiple verdicts produces multiple lines."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        # Multiple updates
        for t in range(5):
            output = monitor.update("x", 10.0, float(t))

        # The final output should have multiple verdicts in verdicts()
        verdicts = output.verdicts()
        str_repr = str(output)

        # str representation should contain 't=' for each verdict
        assert isinstance(str_repr, str)

    def test_verdicts_method(self):
        """Test verdicts() method returns list of (timestamp, value) tuples."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

        output = monitor.update("x", 10.0, 0.0)

        verdicts = output.verdicts()
        assert isinstance(verdicts, list)

        for item in verdicts:
            assert isinstance(item, tuple)
            assert len(item) == 2
            # First element is timestamp (float)
            assert isinstance(item[0], float)
            # Second element is value (float for DelayedQuantitative)
            assert isinstance(item[1], float)

    def test_verdicts_with_bool_semantics(self):
        """Test verdicts() with boolean semantics."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        output = monitor.update("x", 10.0, 0.0)

        verdicts = output.verdicts()
        for item in verdicts:
            assert isinstance(item, tuple)
            assert isinstance(item[0], float)  # timestamp
            assert isinstance(item[1], bool)  # value

    def test_verdicts_with_rosi_semantics(self):
        """Test verdicts() with Rosi semantics."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="Rosi")

        output = monitor.update("x", 10.0, 0.0)

        verdicts = output.verdicts()
        for item in verdicts:
            assert isinstance(item, tuple)
            assert isinstance(item[0], float)  # timestamp
            # Value should be a tuple (min, max)
            assert isinstance(item[1], tuple)
            assert len(item[1]) == 2

    def test_helper_methods(self):
        """Test has_verdicts, total_raw_outputs, is_pending methods."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

        output = monitor.update("x", 10.0, 0.0)

        # Test method return types
        assert isinstance(output.has_verdicts(), bool)
        assert isinstance(output.total_raw_outputs(), int)
        assert isinstance(output.is_pending(), bool)

        # If has_verdicts is True, total_raw_outputs should be > 0
        if output.has_verdicts():
            assert output.total_raw_outputs() > 0

    def test_properties(self):
        """Test input_signal, input_timestamp, input_value properties."""
        formula = mstlo.Formula.gt("x", 5.0)
        monitor = mstlo.Monitor(formula, semantics="DelayedQuantitative")

        output = monitor.update("test_signal", 42.5, 1.5)

        assert output.input_signal == "test_signal"
        assert output.input_timestamp == 1.5
        assert output.input_value == 42.5


class TestMonitorGetSignalIdentifiers:
    """Test getting signal identifiers from the monitor."""

    def test_get_signal_identifiers(self):
        """Test that get_signal_identifiers returns correct signals."""
        f1 = mstlo.Formula.gt("x", 5.0)
        f2 = mstlo.Formula.lt("y", 10.0)
        formula = mstlo.Formula.and_(f1, f2)
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative")

        signals = monitor.get_signal_identifiers()
        assert isinstance(signals, set)
        assert "x" in signals
        assert "y" in signals
        assert len(signals) == 2


class TestVariables:
    """Test the Variables class for runtime variable support."""

    def test_create_variables(self):
        """Test creating an empty Variables object."""
        vars = mstlo.Variables()
        assert vars is not None
        assert len(vars.names()) == 0

    def test_set_and_get(self):
        """Test setting and getting variable values."""
        vars = mstlo.Variables()
        vars.set("threshold", 5.0)
        assert vars.get("threshold") == 5.0
        assert vars.get("nonexistent") is None

    def test_contains(self):
        """Test checking if a variable exists."""
        vars = mstlo.Variables()
        vars.set("x", 1.0)
        assert vars.contains("x") == True
        assert vars.contains("y") == False

    def test_names(self):
        """Test getting all variable names."""
        vars = mstlo.Variables()
        vars.set("a", 1.0)
        vars.set("b", 2.0)
        vars.set("c", 3.0)
        names = vars.names()
        assert len(names) == 3
        assert "a" in names
        assert "b" in names
        assert "c" in names

    def test_remove(self):
        """Test removing a variable."""
        vars = mstlo.Variables()
        vars.set("x", 5.0)
        removed = vars.remove("x")
        assert removed == 5.0
        assert vars.get("x") is None
        assert vars.remove("nonexistent") is None

    def test_clear(self):
        """Test clearing all variables."""
        vars = mstlo.Variables()
        vars.set("a", 1.0)
        vars.set("b", 2.0)
        vars.clear()
        assert len(vars.names()) == 0

    def test_update_value(self):
        """Test updating an existing variable's value."""
        vars = mstlo.Variables()
        vars.set("x", 5.0)
        assert vars.get("x") == 5.0
        vars.set("x", 10.0)
        assert vars.get("x") == 10.0

    def test_str_representation(self):
        """Test string representation of Variables."""
        vars = mstlo.Variables()
        assert "Variables" in str(vars)
        vars.set("x", 5.0)
        assert "x" in str(vars)
        assert "5" in str(vars)


class TestVariableFormulas:
    """Test formulas with variable predicates."""

    def test_parse_formula_with_variable_dollar(self):
        """Test parsing a formula with $ variable syntax."""
        formula = mstlo.parse_formula("x > $threshold")
        assert formula is not None
        assert "threshold" in str(formula)

    def test_parse_formula_with_variable_plain(self):
        """Test parsing a formula with plain variable syntax."""
        formula = mstlo.parse_formula("x > A")
        assert formula is not None
        assert "A" in str(formula)

    def test_formula_gt_var(self):
        """Test creating a greater-than-variable formula programmatically."""
        formula = mstlo.Formula.gt_var("x", "threshold")
        assert formula is not None
        assert "x" in str(formula)
        assert "threshold" in str(formula)

    def test_formula_lt_var(self):
        """Test creating a less-than-variable formula programmatically."""
        formula = mstlo.Formula.lt_var("y", "limit")
        assert formula is not None
        assert "y" in str(formula)
        assert "limit" in str(formula)


class TestMonitorWithVariables:
    """Test monitors with variable predicates."""

    def test_monitor_with_variables_strict(self):
        """Test a monitor with variables using strict satisfaction."""
        vars = mstlo.Variables()
        vars.set("threshold", 5.0)

        formula = mstlo.parse_formula("x > $threshold")
        monitor = mstlo.Monitor(
            formula,
            semantics="DelayedQualitative",
            algorithm="Incremental",
            variables=vars,
        )

        # Value above threshold should be True
        output = monitor.update("x", 10.0, 1.0)
        verdicts = output.verdicts()
        assert len(verdicts) == 1
        # verdicts returns (timestamp, value) tuples
        assert verdicts[0][1] == True

        # Update threshold
        vars.set("threshold", 15.0)

        # Same value now below threshold should be False
        output2 = monitor.update("x", 10.0, 2.0)
        verdicts2 = output2.verdicts()
        assert len(verdicts2) == 1
        assert verdicts2[0][1] == False

    def test_monitor_with_variables_DelayedQuantitative(self):
        """Test a monitor with variables using DelayedQuantitative semantics."""
        vars = mstlo.Variables()
        vars.set("threshold", 5.0)

        formula = mstlo.parse_formula("x > $threshold")
        monitor = mstlo.Monitor(
            formula,
            semantics="DelayedQuantitative",
            algorithm="Incremental",
            variables=vars,
        )

        # DelayedQuantitative = 10 - 5 = 5
        output = monitor.update("x", 10.0, 1.0)
        verdicts = output.verdicts()
        assert len(verdicts) == 1
        # verdicts returns (timestamp, value) tuples
        assert verdicts[0][1] == 5.0

        # Update threshold
        vars.set("threshold", 15.0)

        # DelayedQuantitative = 10 - 15 = -5
        output2 = monitor.update("x", 10.0, 2.0)
        verdicts2 = output2.verdicts()
        assert len(verdicts2) == 1
        assert verdicts2[0][1] == -5.0

    def test_get_variables_from_monitor(self):
        """Test getting variables from a monitor."""
        vars = mstlo.Variables()
        vars.set("A", 1.0)
        vars.set("B", 2.0)

        formula = mstlo.parse_formula("x > $A && y < $B")
        monitor = mstlo.Monitor(formula, semantics="DelayedQualitative", variables=vars)

        # Get variables back
        retrieved_vars = monitor.get_variables()
        assert retrieved_vars.get("A") == 1.0
        assert retrieved_vars.get("B") == 2.0

    def test_variables_in_temporal_formula(self):
        """Test variables inside temporal operators."""
        vars = mstlo.Variables()
        vars.set("limit", 5.0)

        formula = mstlo.parse_formula("F[0, 2](x > $limit)")
        monitor = mstlo.Monitor(
            formula, semantics="DelayedQuantitative", variables=vars
        )

        # Feed some data points
        output = monitor.update("x", 3.0, 0.0)  # Below threshold
        output = monitor.update("x", 7.0, 1.0)  # Above threshold
        output = monitor.update("x", 4.0, 2.0)
        output = monitor.update("x", 4.0, 3.0)

        # Should have verdicts
        assert output.has_verdicts()

    def test_variables_not_supported_in_naive(self):
        """Test that variables are not supported in naive algorithm."""
        vars = mstlo.Variables()
        vars.set("threshold", 5.0)

        formula = mstlo.parse_formula("x > $threshold")

        # PanicException is raised when variables are used with the naive algorithm
        with pytest.raises(BaseException) as exc_info:
            monitor = mstlo.Monitor(
                formula,
                semantics="DelayedQualitative",
                algorithm="Naive",
                variables=vars,
            )
        # Check that the error message mentions naive or variable
        assert "naive" in str(exc_info.value).lower() or "Variable" in str(
            exc_info.value
        )


class TestMonitorGetters:
    """Test all the monitor getter methods."""

    def test_get_algorithm(self):
        """Test getting the algorithm from a monitor."""
        formula = mstlo.parse_formula("x > 5")

        monitor_inc = mstlo.Monitor(formula, algorithm="Incremental")
        assert monitor_inc.get_algorithm() == "Incremental"

        monitor_naive = mstlo.Monitor(
            formula, algorithm="Naive", semantics="DelayedQuantitative"
        )
        assert monitor_naive.get_algorithm() == "Naive"

    def test_get_semantics(self):
        """Test getting the semantics from a monitor."""
        formula = mstlo.parse_formula("x > 5")

        monitor_rob = mstlo.Monitor(formula, semantics="DelayedQuantitative")
        assert monitor_rob.get_semantics() == "DelayedQuantitative"

        monitor_strict = mstlo.Monitor(formula, semantics="DelayedQualitative")
        assert monitor_strict.get_semantics() == "DelayedQualitative"

        monitor_eager = mstlo.Monitor(formula, semantics="EagerQualitative")
        assert monitor_eager.get_semantics() == "EagerQualitative"

        monitor_rosi = mstlo.Monitor(formula, semantics="Rosi")
        assert monitor_rosi.get_semantics() == "Rosi"

    def test_get_synchronization_strategy(self):
        """Test getting the synchronization strategy from a monitor."""
        formula = mstlo.parse_formula("x > 5")

        monitor_zoh = mstlo.Monitor(formula, synchronization="ZeroOrderHold")
        assert monitor_zoh.get_synchronization_strategy() == "ZeroOrderHold"

        monitor_linear = mstlo.Monitor(formula, synchronization="Linear")
        assert monitor_linear.get_synchronization_strategy() == "Linear"

        monitor_none = mstlo.Monitor(formula, synchronization="None")
        assert monitor_none.get_synchronization_strategy() == "None"

    def test_get_specification(self):
        """Test getting the specification string from a monitor."""
        formula = mstlo.parse_formula("G[0,5](x > 10)")
        monitor = mstlo.Monitor(formula)

        spec = monitor.get_specification()
        assert "x" in spec
        assert "10" in spec
        assert "G" in spec or "globally" in spec.lower()

    def test_get_temporal_depth(self):
        """Test getting the temporal depth from a monitor."""
        # Single temporal operator
        formula1 = mstlo.parse_formula("G[0,5](x > 10)")
        monitor1 = mstlo.Monitor(formula1)
        assert monitor1.get_temporal_depth() == 5.0

        # Multiple temporal operators - should return the max
        formula2 = mstlo.parse_formula("G[0,5](x > 10) && F[0,3](y < 20)")
        monitor2 = mstlo.Monitor(formula2)
        assert monitor2.get_temporal_depth() == 5.0

        # Nested temporal operators
        formula3 = mstlo.parse_formula("G[0,10](F[0,3](x > 5))")
        monitor3 = mstlo.Monitor(formula3)
        assert monitor3.get_temporal_depth() == 13.0  # 10 + 3

    def test_get_signal_identifiers(self):
        """Test getting signal identifiers from a monitor."""
        # Single signal
        formula1 = mstlo.parse_formula("x > 5")
        monitor1 = mstlo.Monitor(formula1)
        signals1 = monitor1.get_signal_identifiers()
        assert "x" in signals1
        assert len(signals1) == 1

        # Multiple signals
        formula2 = mstlo.parse_formula("G[0,5](x > 10) && F[0,3](y < 20)")
        monitor2 = mstlo.Monitor(formula2)
        signals2 = monitor2.get_signal_identifiers()
        assert "x" in signals2
        assert "y" in signals2
        assert len(signals2) == 2


class TestMonitorDisplay:
    """Test the Display trait implementation (__str__ and __repr__)."""

    def test_monitor_str_basic(self):
        """Test the string representation of a monitor without variables."""
        # Use a multi-signal formula to test synchronization
        formula = mstlo.parse_formula("G[0,5](x > 10) && y < 20")
        monitor = mstlo.Monitor(
            formula,
            semantics="DelayedQuantitative",
            algorithm="Incremental",
            synchronization="Linear",
        )

        display = str(monitor)
        assert "STL Monitor Configuration" in display
        assert "Specification:" in display
        assert "Algorithm: Incremental" in display
        assert "Semantics: DelayedQuantitative" in display
        assert "Synchronization: Linear" in display
        assert "Temporal Depth:" in display
        assert "5s" in display

    def test_monitor_str_with_variables(self):
        """Test the string representation of a monitor with variables."""
        vars = mstlo.Variables()
        vars.set("threshold", 10.0)
        vars.set("limit", 20.0)

        formula = mstlo.parse_formula("G[0,5](x > $threshold) && y < $limit")
        monitor = mstlo.Monitor(
            formula, semantics="DelayedQuantitative", variables=vars
        )

        display = str(monitor)
        assert "STL Monitor Configuration" in display
        assert "Variables:" in display
        assert "$threshold = 10" in display
        assert "$limit = 20" in display

    def test_monitor_repr(self):
        """Test the repr representation of a monitor."""
        formula = mstlo.parse_formula("x > 5")
        monitor = mstlo.Monitor(
            formula,
            semantics="DelayedQualitative",
            algorithm="Naive",
            synchronization="None",
        )

        repr_str = repr(monitor)
        assert "Monitor(" in repr_str
        assert "semantics='DelayedQualitative'" in repr_str
        assert "algorithm='Naive'" in repr_str
        assert "synchronization='None'" in repr_str

    def test_print_monitor(self):
        """Test that print(monitor) works correctly."""
        formula = mstlo.parse_formula("F[0,3](x > 5)")
        monitor = mstlo.Monitor(formula)

        # Should not raise an exception
        output = str(monitor)
        assert len(output) > 0
        assert "Monitor" in output


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
