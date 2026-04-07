//! Runtime parser for STL formulas.
//!
//! This module provides a runtime parser that accepts the same DSL syntax as the `stl!` macro,
//! enabling Python bindings and other runtime use cases to use the same formula syntax.
//!
//! # Syntax
//!
//! The parser accepts the following syntax (matching the `stl!` macro):
//!
//! ## Predicates
//! - `signal > value` - Signal greater than value
//! - `signal < value` - Signal less than value
//! - `signal >= value` - Signal greater than or equal to value
//! - `signal <= value` - Signal less than or equal to value
//!
//! ## Boolean Constants
//! - `true` - Always true
//! - `false` - Always false
//!
//! ## Unary Operators
//! - `!(sub)` or `not(sub)` - Negation
//! - `G[start, end](sub)` or `globally[start, end](sub)` - Globally (always)
//! - `F[start, end](sub)` or `eventually[start, end](sub)` - Eventually (finally)
//!
//! ## Binary Operators
//! - `left && right` or `left and right` - Conjunction
//! - `left || right` or `left or right` - Disjunction
//! - `left -> right` or `left implies right` - Implication
//! - `left U[start, end] right` or `left until[start, end] right` - Until
//!
//! ## Operator Precedence (lowest to highest)
//! 1. Implication (`->`, `implies`) - right-associative
//! 2. Or (`||`, `or`)
//! 3. And (`&&`, `and`)
//! 4. Until (`U`, `until`)
//! 5. Unary operators and atoms
//!
//! # Example
//!
//! ```
//! use mstlo::stl::parser::parse_stl;
//!
//! let formula = parse_stl("G[0, 10](x > 5)").unwrap();
//! let complex = parse_stl("G[0, 10](x > 5) && F[0, 5](y < 3)").unwrap();
//! ```

use crate::stl::core::TimeInterval;
use crate::stl::formula_definition::FormulaDefinition;
use std::time::Duration;

/// Error type for STL formula parsing.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// Human-readable error message.
    pub message: String,
    /// Position in the input string where the error occurred.
    pub position: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Parse error at position {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// Parse an STL formula from a string.
///
/// This function accepts the same syntax as the `stl!` macro, allowing runtime
/// parsing of STL formulas (e.g., from Python or configuration files).
///
/// # Arguments
///
/// * `input` - A string containing an STL formula
///
/// # Returns
///
/// * `Ok(FormulaDefinition)` - The parsed formula
/// * `Err(ParseError)` - If the input cannot be parsed
///
/// # Example
///
/// ```
/// use mstlo::stl::parser::parse_stl;
///
/// let formula = parse_stl("G[0, 10](x > 5)").unwrap();
/// let nested = parse_stl("G[0, 10](F[0, 5](x > 0))").unwrap();
/// let compound = parse_stl("x > 0 && y < 10").unwrap();
/// ```
pub fn parse_stl(input: &str) -> Result<FormulaDefinition, ParseError> {
    let mut parser = Parser::new(input);
    let result = parser.parse_formula()?;
    parser.skip_whitespace();
    if parser.pos < parser.input.len() {
        return Err(ParseError {
            message: format!(
                "Unexpected trailing characters: '{}'",
                &parser.input[parser.pos..]
            ),
            position: parser.pos,
        });
    }
    Ok(result)
}

/// Internal parser state.
struct Parser<'a> {
    /// Full input expression being parsed.
    input: &'a str,
    /// Current byte offset into `input`.
    pos: usize,
}

impl<'a> Parser<'a> {
    /// Creates a parser at position `0`.
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    /// Advances past ASCII/Unicode whitespace.
    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            if let Some(c) = self.input[self.pos..].chars().next() {
                if c.is_whitespace() {
                    self.pos += c.len_utf8();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    /// Returns the current character without consuming it.
    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Returns the unparsed suffix.
    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }

    /// Consumes one expected character after skipping whitespace.
    fn consume_char(&mut self, expected: char) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.peek() == Some(expected) {
            self.pos += expected.len_utf8();
            Ok(())
        } else {
            Err(ParseError {
                message: format!("Expected '{}', found {:?}", expected, self.peek()),
                position: self.pos,
            })
        }
    }

    /// Parses an identifier: `[A-Za-z_][A-Za-z0-9_]*`.
    fn parse_identifier(&mut self) -> Result<String, ParseError> {
        self.skip_whitespace();
        let start = self.pos;

        // First character must be alphabetic or underscore
        match self.peek() {
            Some(c) if c.is_alphabetic() || c == '_' => {
                self.pos += c.len_utf8();
            }
            _ => {
                return Err(ParseError {
                    message: "Expected identifier".to_string(),
                    position: self.pos,
                });
            }
        }

        // Rest can be alphanumeric or underscore
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }

        Ok(self.input[start..self.pos].to_string())
    }

    /// Parses a signed decimal number.
    ///
    /// Supported forms include integer and fractional literals (for example
    /// `5`, `-2`, `0.75`, `-1.25`).
    fn parse_number(&mut self) -> Result<f64, ParseError> {
        self.skip_whitespace();
        let start = self.pos;

        // Optional negative sign
        if self.peek() == Some('-') {
            self.pos += 1;
        }

        let mut has_digits = false;

        // Integer part
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                has_digits = true;
                self.pos += 1;
            } else {
                break;
            }
        }

        // Decimal part
        if self.peek() == Some('.') {
            self.pos += 1;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    has_digits = true;
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }

        if !has_digits {
            return Err(ParseError {
                message: "Expected number".to_string(),
                position: start,
            });
        }

        self.input[start..self.pos].parse().map_err(|_| ParseError {
            message: "Invalid number".to_string(),
            position: start,
        })
    }

    /// Parses an interval literal `[start, end]` into [`TimeInterval`].
    ///
    /// Interval bounds must be non-negative and satisfy `start <= end`.
    fn parse_interval(&mut self) -> Result<TimeInterval, ParseError> {
        self.consume_char('[')?;
        let start = self.parse_number()?;
        self.skip_whitespace();
        self.consume_char(',')?;
        let end = self.parse_number()?;
        self.consume_char(']')?;

        if start < 0.0 || end < 0.0 {
            return Err(ParseError {
                message: "Time interval bounds must be non-negative".to_string(),
                position: self.pos,
            });
        }

        if start > end {
            return Err(ParseError {
                message: format!("Invalid interval: start ({}) > end ({})", start, end),
                position: self.pos,
            });
        }

        Ok(TimeInterval {
            start: Duration::from_secs_f64(start),
            end: Duration::from_secs_f64(end),
        })
    }

    /// Parses a full formula using top-level precedence entrypoint.
    fn parse_formula(&mut self) -> Result<FormulaDefinition, ParseError> {
        self.parse_implication()
    }

    // Precedence level 1: Implication (lowest, right-associative)
    fn parse_implication(&mut self) -> Result<FormulaDefinition, ParseError> {
        let left = self.parse_or()?;
        self.skip_whitespace();

        // Try "->" operator
        if self.remaining().starts_with("->") {
            self.pos += 2;
            // Right-associative: parse full implication chain on the right
            let right = self.parse_implication()?;
            return Ok(FormulaDefinition::Implies(Box::new(left), Box::new(right)));
        }

        // Try "implies" keyword
        if self.try_consume_keyword("implies") {
            let right = self.parse_implication()?;
            return Ok(FormulaDefinition::Implies(Box::new(left), Box::new(right)));
        }

        Ok(left)
    }

    // Precedence level 2: Or
    fn parse_or(&mut self) -> Result<FormulaDefinition, ParseError> {
        let mut left = self.parse_and()?;

        loop {
            self.skip_whitespace();

            // Try "||" operator
            if self.remaining().starts_with("||") {
                self.pos += 2;
                let right = self.parse_and()?;
                left = FormulaDefinition::Or(Box::new(left), Box::new(right));
                continue;
            }

            // Try "or" keyword
            if self.try_consume_keyword("or") {
                let right = self.parse_and()?;
                left = FormulaDefinition::Or(Box::new(left), Box::new(right));
                continue;
            }

            break;
        }

        Ok(left)
    }

    // Precedence level 3: And
    fn parse_and(&mut self) -> Result<FormulaDefinition, ParseError> {
        let mut left = self.parse_until()?;

        loop {
            self.skip_whitespace();

            // Try "&&" operator
            if self.remaining().starts_with("&&") {
                self.pos += 2;
                let right = self.parse_until()?;
                left = FormulaDefinition::And(Box::new(left), Box::new(right));
                continue;
            }

            // Try "and" keyword (but not "and" as part of another identifier)
            if self.try_consume_keyword("and") {
                let right = self.parse_until()?;
                left = FormulaDefinition::And(Box::new(left), Box::new(right));
                continue;
            }

            break;
        }

        Ok(left)
    }

    // Precedence level 4: Until
    fn parse_until(&mut self) -> Result<FormulaDefinition, ParseError> {
        let mut left = self.parse_unary()?;

        loop {
            self.skip_whitespace();

            // Try "U[" operator
            if self.remaining().starts_with("U[") {
                self.pos += 1; // consume 'U'
                let interval = self.parse_interval()?;
                let right = self.parse_unary()?;
                left = FormulaDefinition::Until(interval, Box::new(left), Box::new(right));
                continue;
            }

            // Try "until[" keyword
            if self.remaining().starts_with("until[") {
                self.pos += 5; // consume "until"
                let interval = self.parse_interval()?;
                let right = self.parse_unary()?;
                left = FormulaDefinition::Until(interval, Box::new(left), Box::new(right));
                continue;
            }

            break;
        }

        Ok(left)
    }

    // Precedence level 5: Unary operators and atoms
    fn parse_unary(&mut self) -> Result<FormulaDefinition, ParseError> {
        self.skip_whitespace();

        // Negation: !
        if self.peek() == Some('!') {
            self.pos += 1;
            let inner = self.parse_unary()?;
            return Ok(FormulaDefinition::Not(Box::new(inner)));
        }

        // Check for keywords
        let checkpoint = self.pos;
        if let Ok(ident) = self.parse_identifier() {
            match ident.as_str() {
                // Negation: not
                "not" => {
                    self.skip_whitespace();
                    let inner = if self.peek() == Some('(') {
                        self.consume_char('(')?;
                        let inner = self.parse_formula()?;
                        self.consume_char(')')?;
                        inner
                    } else {
                        self.parse_unary()?
                    };
                    Ok(FormulaDefinition::Not(Box::new(inner)))
                }

                // Globally: G or globally
                "G" | "globally" => {
                    let interval = self.parse_interval()?;
                    self.consume_char('(')?;
                    let inner = self.parse_formula()?;
                    self.consume_char(')')?;
                    Ok(FormulaDefinition::Globally(interval, Box::new(inner)))
                }

                // Eventually: F or eventually
                "F" | "eventually" => {
                    let interval = self.parse_interval()?;
                    self.consume_char('(')?;
                    let inner = self.parse_formula()?;
                    self.consume_char(')')?;
                    Ok(FormulaDefinition::Eventually(interval, Box::new(inner)))
                }

                // Boolean constants
                "true" => Ok(FormulaDefinition::True),
                "false" => Ok(FormulaDefinition::False),

                // Otherwise, this might be a signal name - check for comparison
                _ => {
                    // We have a potential signal name, need to keep the original String
                    let signal = ident;
                    self.skip_whitespace();

                    // Parse comparison operator
                    let op = if self.remaining().starts_with(">=") {
                        self.pos += 2;
                        ">="
                    } else if self.remaining().starts_with("<=") {
                        self.pos += 2;
                        "<="
                    } else if self.remaining().starts_with(">") {
                        self.pos += 1;
                        ">"
                    } else if self.remaining().starts_with("<") {
                        self.pos += 1;
                        "<"
                    } else {
                        // Not a predicate, restore position
                        self.pos = checkpoint;
                        return self.parse_primary();
                    };

                    // Convert signal to FormulaDefinition
                    // We need to leak the string to get 'static lifetime
                    let signal_static: &'static str = Box::leak(signal.into_boxed_str());

                    // Try to parse the threshold - either a number or a variable (identifier)
                    self.skip_whitespace();

                    // Check if it's a variable reference ($ prefix or identifier)
                    let is_variable = self.peek() == Some('$');
                    if is_variable {
                        self.pos += 1; // consume '$'
                    }

                    // Try parsing a number first (if not explicitly a variable)
                    if !is_variable {
                        let num_start = self.pos;
                        if let Ok(value) = self.parse_number() {
                            return match op {
                                ">" => Ok(FormulaDefinition::GreaterThan(signal_static, value)),
                                "<" => Ok(FormulaDefinition::LessThan(signal_static, value)),
                                ">=" => {
                                    // x >= v is equivalent to !(x < v)
                                    Ok(FormulaDefinition::Not(Box::new(
                                        FormulaDefinition::LessThan(signal_static, value),
                                    )))
                                }
                                "<=" => {
                                    // x <= v is equivalent to !(x > v)
                                    Ok(FormulaDefinition::Not(Box::new(
                                        FormulaDefinition::GreaterThan(signal_static, value),
                                    )))
                                }
                                _ => unreachable!(),
                            };
                        }
                        // Not a number, restore position and try as variable
                        self.pos = num_start;
                    }

                    // Parse as variable identifier
                    let var_name = self.parse_identifier()?;
                    let var_static: &'static str = Box::leak(var_name.into_boxed_str());

                    match op {
                        ">" => Ok(FormulaDefinition::GreaterThanVar(signal_static, var_static)),
                        "<" => Ok(FormulaDefinition::LessThanVar(signal_static, var_static)),
                        ">=" => {
                            // x >= var is equivalent to !(x < var)
                            Ok(FormulaDefinition::Not(Box::new(
                                FormulaDefinition::LessThanVar(signal_static, var_static),
                            )))
                        }
                        "<=" => {
                            // x <= var is equivalent to !(x > var)
                            Ok(FormulaDefinition::Not(Box::new(
                                FormulaDefinition::GreaterThanVar(signal_static, var_static),
                            )))
                        }
                        _ => unreachable!(),
                    }
                }
            }
        } else {
            self.parse_primary()
        }
    }

    /// Parses atomic primary expressions.
    ///
    /// Currently this accepts parenthesized sub-formulas.
    fn parse_primary(&mut self) -> Result<FormulaDefinition, ParseError> {
        self.skip_whitespace();

        // Parenthesized expression
        if self.peek() == Some('(') {
            self.consume_char('(')?;
            let inner = self.parse_formula()?;
            self.consume_char(')')?;
            return Ok(inner);
        }

        Err(ParseError {
            message: format!(
                "Unexpected token: {:?}",
                self.peek()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "end of input".to_string())
            ),
            position: self.pos,
        })
    }

    /// Try to consume a keyword (must not be followed by alphanumeric chars).
    fn try_consume_keyword(&mut self, keyword: &str) -> bool {
        self.skip_whitespace();
        if self.remaining().starts_with(keyword) {
            let after = &self.remaining()[keyword.len()..];
            // Check that keyword is not part of a longer identifier
            if after
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_')
            {
                self.pos += keyword.len();
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_greater_than() {
        let result = parse_stl("x > 5").unwrap();
        assert_eq!(result, FormulaDefinition::GreaterThan("x", 5.0));
    }

    #[test]
    fn test_simple_less_than() {
        let result = parse_stl("y < 3.5").unwrap();
        assert_eq!(result, FormulaDefinition::LessThan("y", 3.5));
    }

    #[test]
    fn test_greater_equal() {
        let result = parse_stl("x >= 5").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::LessThan("x", 5.0)))
        );
    }

    #[test]
    fn test_less_equal() {
        let result = parse_stl("x <= 5").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::GreaterThan("x", 5.0)))
        );
    }

    #[test]
    fn test_boolean_true() {
        let result = parse_stl("true").unwrap();
        assert!(matches!(result, FormulaDefinition::True));
    }

    #[test]
    fn test_boolean_false() {
        let result = parse_stl("false").unwrap();
        assert!(matches!(result, FormulaDefinition::False));
    }

    #[test]
    fn test_globally() {
        let result = parse_stl("G[0, 10](x > 5)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Globally(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(10)
                },
                Box::new(FormulaDefinition::GreaterThan("x", 5.0))
            )
        );
    }

    #[test]
    fn test_globally_keyword() {
        let result = parse_stl("globally[0, 10](x > 5)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Globally(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(10)
                },
                Box::new(FormulaDefinition::GreaterThan("x", 5.0))
            )
        );
    }

    #[test]
    fn test_eventually() {
        let result = parse_stl("F[0, 5](y < 3)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Eventually(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(5)
                },
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_eventually_keyword() {
        let result = parse_stl("eventually[0, 5](y < 3)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Eventually(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(5)
                },
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_and_symbols() {
        let result = parse_stl("x > 5 && y < 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::And(
                Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_and_keyword() {
        let result = parse_stl("x > 5 and y < 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::And(
                Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_or_symbols() {
        let result = parse_stl("x > 5 || y < 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Or(
                Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_or_keyword() {
        let result = parse_stl("x > 5 or y < 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Or(
                Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_not_symbol() {
        let result = parse_stl("!(x > 5)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::GreaterThan("x", 5.0)))
        );
    }

    #[test]
    fn test_not_keyword() {
        let result = parse_stl("not(x > 5)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::GreaterThan("x", 5.0)))
        );
    }

    #[test]
    fn test_implies_symbol() {
        let result = parse_stl("x > 5 -> y < 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Implies(
                Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_implies_keyword() {
        let result = parse_stl("x > 5 implies y < 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Implies(
                Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_until_symbol() {
        let result = parse_stl("x > 5 U[0, 10] y < 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Until(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(10)
                },
                Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_until_keyword() {
        let result = parse_stl("x > 5 until[0, 10] y < 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Until(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(10)
                },
                Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
                Box::new(FormulaDefinition::LessThan("y", 3.0))
            )
        );
    }

    #[test]
    fn test_nested_temporal() {
        let result = parse_stl("G[0, 10](F[0, 5](x > 0))").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Globally(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(10)
                },
                Box::new(FormulaDefinition::Eventually(
                    TimeInterval {
                        start: Duration::from_secs(0),
                        end: Duration::from_secs(5)
                    },
                    Box::new(FormulaDefinition::GreaterThan("x", 0.0))
                ))
            )
        );
    }

    #[test]
    fn test_complex_formula() {
        let result = parse_stl("G[0, 10](x > 5) && F[0, 5](y < 3)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::And(
                Box::new(FormulaDefinition::Globally(
                    TimeInterval {
                        start: Duration::from_secs(0),
                        end: Duration::from_secs(10)
                    },
                    Box::new(FormulaDefinition::GreaterThan("x", 5.0))
                )),
                Box::new(FormulaDefinition::Eventually(
                    TimeInterval {
                        start: Duration::from_secs(0),
                        end: Duration::from_secs(5)
                    },
                    Box::new(FormulaDefinition::LessThan("y", 3.0))
                ))
            )
        );
    }

    #[test]
    fn test_precedence_and_or() {
        // "and" binds tighter than "or": a || b && c == a || (b && c)
        let result = parse_stl("x > 1 || y > 2 && z > 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Or(
                Box::new(FormulaDefinition::GreaterThan("x", 1.0)),
                Box::new(FormulaDefinition::And(
                    Box::new(FormulaDefinition::GreaterThan("y", 2.0)),
                    Box::new(FormulaDefinition::GreaterThan("z", 3.0))
                ))
            )
        );
    }

    #[test]
    fn test_precedence_implies() {
        // Implies has lowest precedence: a && b -> c == (a && b) -> c
        let result = parse_stl("x > 1 && y > 2 -> z > 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Implies(
                Box::new(FormulaDefinition::And(
                    Box::new(FormulaDefinition::GreaterThan("x", 1.0)),
                    Box::new(FormulaDefinition::GreaterThan("y", 2.0))
                )),
                Box::new(FormulaDefinition::GreaterThan("z", 3.0))
            )
        );
    }

    #[test]
    fn test_parentheses_override_precedence() {
        let result = parse_stl("(x > 1 || y > 2) && z > 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::And(
                Box::new(FormulaDefinition::Or(
                    Box::new(FormulaDefinition::GreaterThan("x", 1.0)),
                    Box::new(FormulaDefinition::GreaterThan("y", 2.0))
                )),
                Box::new(FormulaDefinition::GreaterThan("z", 3.0))
            )
        );
    }

    #[test]
    fn test_chained_and() {
        // Left associative: a && b && c == (a && b) && c
        let result = parse_stl("x > 1 && y > 2 && z > 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::And(
                Box::new(FormulaDefinition::And(
                    Box::new(FormulaDefinition::GreaterThan("x", 1.0)),
                    Box::new(FormulaDefinition::GreaterThan("y", 2.0))
                )),
                Box::new(FormulaDefinition::GreaterThan("z", 3.0))
            )
        );
    }

    #[test]
    fn test_implies_right_associative() {
        // Right associative: a -> b -> c == a -> (b -> c)
        let result = parse_stl("x > 1 -> y > 2 -> z > 3").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Implies(
                Box::new(FormulaDefinition::GreaterThan("x", 1.0)),
                Box::new(FormulaDefinition::Implies(
                    Box::new(FormulaDefinition::GreaterThan("y", 2.0)),
                    Box::new(FormulaDefinition::GreaterThan("z", 3.0))
                ))
            )
        );
    }

    #[test]
    fn test_negative_number() {
        let result = parse_stl("x > -5").unwrap();
        assert_eq!(result, FormulaDefinition::GreaterThan("x", -5.0));
    }

    #[test]
    fn test_whitespace_tolerance() {
        let result = parse_stl("  G  [  0  ,  10  ]  (  x  >  5  )  ").unwrap();
        assert!(matches!(result, FormulaDefinition::Globally(..)));
    }

    #[test]
    fn test_error_empty_input() {
        let result = parse_stl("");
        assert!(result.is_err());
    }

    #[test]
    fn test_error_trailing_chars() {
        let result = parse_stl("x > 5 extra");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("trailing"));
    }

    #[test]
    fn test_error_missing_interval() {
        let result = parse_stl("G(x > 5)");
        assert!(result.is_err());
    }

    #[test]
    fn test_error_missing_subformula() {
        let result = parse_stl("G[0, 10]()");
        assert!(result.is_err());
    }

    #[test]
    fn test_decimal_interval() {
        let result = parse_stl("G[0.5, 10.5](x > 5)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Globally(
                TimeInterval {
                    start: Duration::from_secs_f64(0.5),
                    end: Duration::from_secs_f64(10.5)
                },
                Box::new(FormulaDefinition::GreaterThan("x", 5.0))
            )
        );
    }

    #[test]
    fn test_signal_with_underscore() {
        let result = parse_stl("my_signal > 5").unwrap();
        assert_eq!(result, FormulaDefinition::GreaterThan("my_signal", 5.0));
    }

    #[test]
    fn test_variable_greater_than_with_dollar() {
        let result = parse_stl("x > $threshold").unwrap();
        assert_eq!(result, FormulaDefinition::GreaterThanVar("x", "threshold"));
    }

    #[test]
    fn test_variable_less_than_with_dollar() {
        let result = parse_stl("temperature < $MAX_TEMP").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::LessThanVar("temperature", "MAX_TEMP")
        );
    }

    #[test]
    fn test_variable_without_dollar() {
        // Variables can also be used without $ prefix
        let result = parse_stl("x > A").unwrap();
        assert_eq!(result, FormulaDefinition::GreaterThanVar("x", "A"));
    }

    #[test]
    fn test_variable_in_temporal_formula() {
        let result = parse_stl("F[0,10](x > $threshold)").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Eventually(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(10)
                },
                Box::new(FormulaDefinition::GreaterThanVar("x", "threshold"))
            )
        );
    }

    #[test]
    fn test_variable_mixed_with_constant() {
        // Formula with both a variable and a constant
        let result = parse_stl("x > $A && y < 10").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::And(
                Box::new(FormulaDefinition::GreaterThanVar("x", "A")),
                Box::new(FormulaDefinition::LessThan("y", 10.0))
            )
        );
    }

    #[test]
    fn test_variable_identifiers() {
        let result = parse_stl("x > $threshold && y < $limit").unwrap();
        let vars = result.get_variable_identifiers();
        assert_eq!(vars.len(), 2);
        assert!(vars.contains(&"threshold"));
        assert!(vars.contains(&"limit"));
    }

    #[test]
    fn test_parse_error_display() {
        let result = parse_stl("G[0, 10](x > 5");
        assert!(result.is_err());
        let error = format!("{}", result.err().unwrap());
        assert!(error.contains("Expected ')'"));
    }

    #[test]
    fn test_parse_error_negative_interval() {
        let result = parse_stl("G[-1, 10](x > 5)");
        assert!(result.is_err());
        let error = format!("{}", result.err().unwrap());
        assert!(error.contains("non-negative"));
    }

    #[test]
    fn test_parse_error_interval_bounds() {
        let result = parse_stl("G[10, 5](x > 5)");
        assert!(result.is_err());
        let error = format!("{}", result.err().unwrap());
        assert!(error.contains("Invalid interval"));
    }

    #[test]
    fn test_not_keyword_without_parens() {
        // "not" followed by an atom without parentheses
        let result = parse_stl("not x > 5").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::GreaterThan("x", 5.0)))
        );
    }

    #[test]
    fn test_greater_equal_variable() {
        // x >= $var → Not(LessThanVar(x, var))
        let result = parse_stl("x >= $threshold").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::LessThanVar("x", "threshold")))
        );
    }

    #[test]
    fn test_less_equal_variable() {
        // x <= $var → Not(GreaterThanVar(x, var))
        let result = parse_stl("x <= $limit").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::GreaterThanVar("x", "limit")))
        );
    }

    #[test]
    fn test_greater_equal_variable_without_dollar() {
        // x >= VAR → Not(LessThanVar(x, VAR))
        let result = parse_stl("x >= THRESHOLD").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::LessThanVar("x", "THRESHOLD")))
        );
    }

    #[test]
    fn test_less_equal_variable_without_dollar() {
        // x <= VAR → Not(GreaterThanVar(x, VAR))
        let result = parse_stl("x <= LIMIT").unwrap();
        assert_eq!(
            result,
            FormulaDefinition::Not(Box::new(FormulaDefinition::GreaterThanVar("x", "LIMIT")))
        );
    }

    #[test]
    fn test_parse_error_invalid_number() {
        // A standalone "." should fail to parse as a number
        let result = parse_stl("x > .");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_not_an_identifier() {
        // Starting with a digit is not a valid identifier
        let result = parse_stl("123abc > 5");
        assert!(result.is_err());
    }
}
