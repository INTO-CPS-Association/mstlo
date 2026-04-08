//! Integration tests to ensure the runtime parser produces identical formulas
//! to the compile-time `stl!` macro.
//!
//! This test suite verifies that both parsing methods (macro and runtime parser)
//! produce structurally identical `FormulaDefinition` instances for the same input.

use mstlo::stl;
use mstlo::formula_definition::FormulaDefinition;
use mstlo::parse_stl;

/// Helper macro to test that the macro and runtime parser produce identical results.
/// The macro input must be valid for both the `stl!` macro and `parse_stl`.
macro_rules! assert_parser_parity {
    ($formula_str:expr, $($macro_input:tt)+) => {{
        let macro_formula: FormulaDefinition = stl!($($macro_input)+);
        let parsed_formula = parse_stl($formula_str)
            .expect(&format!("Failed to parse: {}", $formula_str));

        assert_eq!(
            macro_formula, parsed_formula,
            "Parser parity failed for: {}\n\nMacro produced:\n{:#?}\n\nParser produced:\n{:#?}",
            $formula_str, macro_formula, parsed_formula
        );
    }};
}

#[cfg(test)]
// =========================================================================
// Atomic Predicates
// =========================================================================
#[test]
fn test_parity_greater_than() {
    assert_parser_parity!("x > 5", x > 5);
}

#[test]
fn test_parity_less_than() {
    assert_parser_parity!("y < 10", y < 10);
}

#[test]
fn test_parity_greater_than_negative() {
    assert_parser_parity!("temp > -5", temp > -5);
}

#[test]
fn test_parity_less_than_decimal() {
    assert_parser_parity!("speed < 3.5", speed < 3.5);
}

#[test]
fn test_parity_greater_equal() {
    // >= is sugar for !(x < v)
    assert_parser_parity!("x >= 5", x >= 5);
}

#[test]
fn test_parity_less_equal() {
    // <= is sugar for !(x > v)
    assert_parser_parity!("x <= 5", x <= 5);
}

// =========================================================================
// Boolean Constants
// =========================================================================

#[test]
fn test_parity_true() {
    assert_parser_parity!("true", true);
}

#[test]
fn test_parity_false() {
    assert_parser_parity!("false", false);
}

// =========================================================================
// Unary Operators
// =========================================================================

#[test]
fn test_parity_not_symbol() {
    assert_parser_parity!("!(x > 5)", !(x > 5));
}

#[test]
fn test_parity_not_keyword() {
    assert_parser_parity!("not(x > 5)", not(x > 5));
}

#[test]
fn test_parity_globally() {
    assert_parser_parity!("G[0, 10](x > 5)", G[0, 10](x > 5));
}

#[test]
fn test_parity_globally_keyword() {
    assert_parser_parity!("globally[0, 10](x > 5)", globally[0, 10](x > 5));
}

#[test]
fn test_parity_eventually() {
    assert_parser_parity!("F[0, 5](y < 3)", F[0, 5](y < 3));
}

#[test]
fn test_parity_eventually_keyword() {
    assert_parser_parity!("eventually[0, 5](y < 3)", eventually[0, 5](y < 3));
}

// =========================================================================
// Binary Boolean Operators
// =========================================================================

#[test]
fn test_parity_and_symbols() {
    assert_parser_parity!("x > 5 && y < 3", x > 5 && y < 3);
}

#[test]
fn test_parity_and_keyword() {
    assert_parser_parity!("x > 5 and y < 3", x > 5 and y < 3);
}

#[test]
fn test_parity_or_symbols() {
    assert_parser_parity!("x > 5 || y < 3", x > 5 || y < 3);
}

#[test]
fn test_parity_or_keyword() {
    assert_parser_parity!("x > 5 or y < 3", x > 5 or y < 3);
}

#[test]
fn test_parity_implies_symbol() {
    assert_parser_parity!("x > 5 -> y < 3", x > 5 -> y < 3);
}

#[test]
fn test_parity_implies_keyword() {
    assert_parser_parity!("x > 5 implies y < 3", x > 5 implies y < 3);
}

// =========================================================================
// Until Operator
// =========================================================================

#[test]
fn test_parity_until_symbol() {
    assert_parser_parity!("x > 5 U[0, 10] y < 3", x > 5 U[0, 10] y < 3);
}

#[test]
fn test_parity_until_keyword() {
    assert_parser_parity!("x > 5 until[0, 10] y < 3", x > 5 until[0, 10] y < 3);
}

// =========================================================================
// Operator Precedence
// =========================================================================

#[test]
fn test_parity_precedence_and_or() {
    // OR has lower precedence than AND: a || b && c == a || (b && c)
    assert_parser_parity!("x > 1 || y > 2 && z > 3", x > 1 || y > 2 && z > 3);
}

#[test]
fn test_parity_precedence_implies_lowest() {
    // Implies has lowest precedence: a && b -> c == (a && b) -> c
    assert_parser_parity!("x > 1 && y > 2 -> z > 3", x > 1 && y > 2 -> z > 3);
}

#[test]
fn test_parity_chained_and() {
    // Left associative: a && b && c == (a && b) && c
    assert_parser_parity!("x > 1 && y > 2 && z > 3", x > 1 && y > 2 && z > 3);
}

#[test]
fn test_parity_chained_or() {
    // Left associative: a || b || c == (a || b) || c
    assert_parser_parity!("x > 1 || y > 2 || z > 3", x > 1 || y > 2 || z > 3);
}

#[test]
fn test_parity_implies_right_associative() {
    // Right associative: a -> b -> c == a -> (b -> c)
    assert_parser_parity!("x > 1 -> y > 2 -> z > 3", x > 1 -> y > 2 -> z > 3);
}

#[test]
fn test_parity_parentheses_override() {
    // Parentheses override precedence
    assert_parser_parity!("(x > 1 || y > 2) && z > 3", (x > 1 || y > 2) && z > 3);
}

// =========================================================================
// Nested Temporal Operators
// =========================================================================

#[test]
fn test_parity_nested_globally_eventually() {
    assert_parser_parity!("G[0, 10](F[0, 5](x > 0))", G[0, 10](F[0, 5](x > 0)));
}

#[test]
fn test_parity_nested_eventually_globally() {
    assert_parser_parity!("F[0, 5](G[0, 10](x > 0))", F[0, 5](G[0, 10](x > 0)));
}

// =========================================================================
// Complex Formulas
// =========================================================================

#[test]
fn test_parity_complex_and_temporal() {
    assert_parser_parity!(
        "G[0, 10](x > 5) && F[0, 5](y < 3)",
        G[0, 10](x > 5) && F[0, 5](y < 3)
    );
}

#[test]
fn test_parity_complex_or_temporal() {
    assert_parser_parity!(
        "G[0, 10](x > 5) || F[0, 5](y < 3)",
        G[0, 10](x > 5) || F[0, 5](y < 3)
    );
}

#[test]
fn test_parity_complex_implies_temporal() {
    assert_parser_parity!(
        "G[0, 10](x > 5) -> F[0, 5](y < 3)",
        G[0, 10](x > 5) -> F[0, 5](y < 3)
    );
}

#[test]
fn test_parity_complex_nested_boolean() {
    assert_parser_parity!(
        "G[0, 10](x > 5 && y < 10)",
        G[0, 10](x > 5 && y < 10)
    );
}

#[test]
fn test_parity_complex_mixed_keywords_symbols() {
    assert_parser_parity!(
        "G[0, 5](x > 0) and F[0, 3](y < 10)",
        G[0, 5](x > 0) and F[0, 3](y < 10)
    );
}

#[test]
fn test_parity_very_complex() {
    // A complex formula combining multiple operators
    assert_parser_parity!(
        "G[0, 10](x > 0 && y < 100) -> F[0, 5](z > 50)",
        G[0, 10](x > 0 && y < 100) -> F[0, 5](z > 50)
    );
}

#[test]
fn test_parity_deeply_nested() {
    assert_parser_parity!(
        "G[0, 10](F[0, 5](x > 0 && y < 10))",
        G[0, 10](F[0, 5](x > 0 && y < 10))
    );
}

#[test]
fn test_parity_until_in_globally() {
    assert_parser_parity!(
        "G[0, 10](x > 0 U[0, 5] y < 10)",
        G[0, 10](x > 0 U[0, 5] y < 10)
    );
}

// =========================================================================
// Whitespace Tolerance
// =========================================================================

#[test]
fn test_parity_extra_whitespace() {
    // Parser should handle extra whitespace the same way
    let parsed = parse_stl("  G  [  0  ,  10  ]  (  x  >  5  )  ").unwrap();
    let macro_formula: FormulaDefinition = stl!(G[0, 10](x > 5));
    assert_eq!(macro_formula, parsed);
}

#[test]
fn test_parity_no_whitespace() {
    let parsed = parse_stl("G[0,10](x>5)").unwrap();
    let macro_formula: FormulaDefinition = stl!(G[0, 10](x > 5));
    assert_eq!(macro_formula, parsed);
}

// =========================================================================
// Signal Names
// =========================================================================

#[test]
fn test_parity_underscore_signal() {
    assert_parser_parity!("my_signal > 5", my_signal > 5);
}

#[test]
fn test_parity_long_signal_name() {
    assert_parser_parity!("temperature_sensor_1 > 100", temperature_sensor_1 > 100);
}

#[test]
fn test_parity_signal_with_numbers() {
    assert_parser_parity!("sensor1 > 0", sensor1 > 0);
}
