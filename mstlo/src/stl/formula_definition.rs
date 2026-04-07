//! Abstract syntax tree definition for Signal Temporal Logic (STL) formulas.
//!
//! [`FormulaDefinition`] models predicate, boolean, and temporal operators used
//! by the monitor and parser layers.

use crate::stl::core::{SignalIdentifier, TimeInterval};

use std::fmt::Display;

/// An enum representing the definition of an STL formula.
///
/// Predicates can use either constant values (e.g., `x > 5.0`) or variables
/// (e.g., `x > A` where `A` is a named variable that can be updated at runtime).
#[derive(Clone, Debug, PartialEq)]
pub enum FormulaDefinition {
    /// Signal greater than a constant: signal > value
    GreaterThan(&'static str, f64),
    /// Signal less than a constant: signal < value
    LessThan(&'static str, f64),
    /// Signal greater than a variable: signal > var_name
    GreaterThanVar(&'static str, &'static str),
    /// Signal less than a variable: signal < var_name
    LessThanVar(&'static str, &'static str),
    /// Boolean constant `True`.
    True,
    /// Boolean constant `False`.
    False,
    /// Boolean conjunction: `lhs ∧ rhs`.
    And(Box<FormulaDefinition>, Box<FormulaDefinition>),
    /// Boolean disjunction: `lhs ∨ rhs`.
    Or(Box<FormulaDefinition>, Box<FormulaDefinition>),
    /// Boolean negation: `¬f`.
    Not(Box<FormulaDefinition>),
    /// Boolean implication: `lhs → rhs`.
    Implies(Box<FormulaDefinition>, Box<FormulaDefinition>),
    /// Temporal eventually operator: `F[a,b] f`.
    Eventually(TimeInterval, Box<FormulaDefinition>),
    /// Temporal globally operator: `G[a,b] f`.
    Globally(TimeInterval, Box<FormulaDefinition>),
    /// Temporal until operator: `lhs U[a,b] rhs`.
    Until(TimeInterval, Box<FormulaDefinition>, Box<FormulaDefinition>),
}

/// Renders formulas using compact mathematical notation.
///
/// Temporal operators are printed as `F[start, end](...)`, `G[start, end](...)`,
/// and `(...) U[start, end] (...)`, where interval bounds are shown in seconds.
impl Display for FormulaDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                FormulaDefinition::True => "True".to_string(),
                FormulaDefinition::False => "False".to_string(),
                FormulaDefinition::Not(f) => format!("¬({f})"),
                FormulaDefinition::And(f1, f2) => format!("({f1}) ∧ ({f2})"),
                FormulaDefinition::Or(f1, f2) => format!("({f1}) v ({f2})"),
                FormulaDefinition::Globally(interval, f) => format!(
                    "G[{}, {}]({})",
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64(),
                    f
                ),
                FormulaDefinition::Eventually(interval, f) => format!(
                    "F[{}, {}]({})",
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64(),
                    f
                ),
                FormulaDefinition::Until(interval, f1, f2) => format!(
                    "({}) U[{}, {}] ({})",
                    f1,
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64(),
                    f2
                ),
                FormulaDefinition::Implies(f1, f2) => format!("({f1}) → ({f2})"),
                FormulaDefinition::GreaterThan(s, val) => format!("{s} > {val}"),
                FormulaDefinition::LessThan(s, val) => format!("{s} < {val}"),
                FormulaDefinition::GreaterThanVar(s, var) => format!("{s} > ${var}"),
                FormulaDefinition::LessThanVar(s, var) => format!("{s} < ${var}"),
            }
        )
    }
}

impl SignalIdentifier for FormulaDefinition {
    /// Returns all signal identifiers referenced by the formula.
    ///
    /// The traversal is recursive and collects unique signal names from both
    /// constant and variable predicates.
    fn get_signal_identifiers(&mut self) -> std::collections::HashSet<&'static str> {
        let mut signals = std::collections::HashSet::new();
        fn collect_signals(
            node: &FormulaDefinition,
            signals: &mut std::collections::HashSet<&'static str>,
        ) {
            match node {
                FormulaDefinition::GreaterThan(s, _)
                | FormulaDefinition::LessThan(s, _)
                | FormulaDefinition::GreaterThanVar(s, _)
                | FormulaDefinition::LessThanVar(s, _) => {
                    signals.insert(*s);
                }
                FormulaDefinition::True | FormulaDefinition::False => {}
                FormulaDefinition::Not(f) => {
                    collect_signals(f, signals);
                }
                FormulaDefinition::And(f1, f2)
                | FormulaDefinition::Or(f1, f2)
                | FormulaDefinition::Implies(f1, f2) => {
                    collect_signals(f1, signals);
                    collect_signals(f2, signals);
                }
                FormulaDefinition::Eventually(_, f) | FormulaDefinition::Globally(_, f) => {
                    collect_signals(f, signals);
                }
                FormulaDefinition::Until(_, f1, f2) => {
                    collect_signals(f1, signals);
                    collect_signals(f2, signals);
                }
            }
        }
        collect_signals(self, &mut signals);
        signals
    }
}

impl FormulaDefinition {
    /// Collects all variable identifiers used in the formula.
    ///
    /// Only variable predicates (`GreaterThanVar`/`LessThanVar`) contribute.
    pub fn get_variable_identifiers(&self) -> std::collections::HashSet<&'static str> {
        let mut variables = std::collections::HashSet::new();
        fn collect_variables(
            node: &FormulaDefinition,
            variables: &mut std::collections::HashSet<&'static str>,
        ) {
            match node {
                FormulaDefinition::GreaterThanVar(_, var)
                | FormulaDefinition::LessThanVar(_, var) => {
                    variables.insert(*var);
                }
                FormulaDefinition::GreaterThan(_, _)
                | FormulaDefinition::LessThan(_, _)
                | FormulaDefinition::True
                | FormulaDefinition::False => {}
                FormulaDefinition::Not(f) => {
                    collect_variables(f, variables);
                }
                FormulaDefinition::And(f1, f2)
                | FormulaDefinition::Or(f1, f2)
                | FormulaDefinition::Implies(f1, f2) => {
                    collect_variables(f1, variables);
                    collect_variables(f2, variables);
                }
                FormulaDefinition::Eventually(_, f) | FormulaDefinition::Globally(_, f) => {
                    collect_variables(f, variables);
                }
                FormulaDefinition::Until(_, f1, f2) => {
                    collect_variables(f1, variables);
                    collect_variables(f2, variables);
                }
            }
        }
        collect_variables(self, &mut variables);
        variables
    }

    /// Builds an ASCII/Unicode tree representation of the formula Abastract Syntax Tree (AST).
    ///
    /// # Arguments
    /// * `indent` - Number of leading spaces for the root node.
    ///
    /// # Returns
    /// A multi-line string with `tree`-style connectors (`├──`, `└──`, `│`).
    ///
    /// # Example
    /// A conjunction may look like:
    ///
    /// ```text
    /// And
    /// ├── x > 1
    /// └── y < 2
    /// ```
    pub fn to_tree_string(&self, indent: usize) -> String {
        // Produce a tree-like multi-line representation using characters similar to `tree`:
        // ├──  branch
        // └──  last branch
        // │    vertical continuation
        let padding = " ".repeat(indent);

        fn label(node: &FormulaDefinition) -> String {
            match node {
                FormulaDefinition::True => "True".to_string(),
                FormulaDefinition::False => "False".to_string(),
                FormulaDefinition::Not(_) => "Not".to_string(),
                FormulaDefinition::And(_, _) => "And".to_string(),
                FormulaDefinition::Or(_, _) => "Or".to_string(),
                FormulaDefinition::Globally(interval, _) => format!(
                    "Globally[{},{}]",
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64()
                ),
                FormulaDefinition::Eventually(interval, _) => format!(
                    "Eventually[{},{}]",
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64()
                ),
                FormulaDefinition::Until(interval, _, _) => format!(
                    "Until[{},{}]",
                    interval.start.as_secs_f64(),
                    interval.end.as_secs_f64()
                ),
                FormulaDefinition::Implies(_, _) => "Implies".to_string(),
                FormulaDefinition::GreaterThan(s, val) => format!("{} > {}", s, val),
                FormulaDefinition::LessThan(s, val) => format!("{} < {}", s, val),
                FormulaDefinition::GreaterThanVar(s, var) => format!("{} > ${}", s, var),
                FormulaDefinition::LessThanVar(s, var) => format!("{} < ${}", s, var),
            }
        }

        fn write_node(
            node: &FormulaDefinition,
            prefix: &str,
            is_root: bool,
            is_last: bool,
            out: &mut String,
        ) {
            if is_root {
                out.push_str(&format!("{}{}\n", prefix, label(node)));
            } else {
                let connector = if is_last { "└── " } else { "├── " };
                out.push_str(&format!("{}{}{}\n", prefix, connector, label(node)));
            }

            // prepare prefix for children
            let child_prefix = if is_root {
                // root's children prefix depends on whether root had initial padding
                if prefix.trim().is_empty() {
                    if is_last {
                        format!("{}    ", prefix)
                    } else {
                        format!("{}│   ", prefix)
                    }
                } else if is_last {
                    format!("{}    ", prefix)
                } else {
                    format!("{}│   ", prefix)
                }
            } else if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };

            match node {
                FormulaDefinition::Not(child) => {
                    write_node(child, &child_prefix, false, true, out);
                }
                FormulaDefinition::And(l, r)
                | FormulaDefinition::Or(l, r)
                | FormulaDefinition::Implies(l, r)
                | FormulaDefinition::Until(_, l, r) => {
                    write_node(l, &child_prefix, false, false, out);
                    write_node(r, &child_prefix, false, true, out);
                }
                FormulaDefinition::Eventually(_, child) | FormulaDefinition::Globally(_, child) => {
                    write_node(child, &child_prefix, false, true, out);
                }
                // leaves: True, False, GreaterThan, LessThan, GreaterThanVar, LessThanVar - nothing to do
                FormulaDefinition::True
                | FormulaDefinition::False
                | FormulaDefinition::GreaterThan(_, _)
                | FormulaDefinition::LessThan(_, _)
                | FormulaDefinition::GreaterThanVar(_, _)
                | FormulaDefinition::LessThanVar(_, _) => {}
            }
        }

        let mut out = String::new();
        write_node(self, &padding, true, true, &mut out);
        // trim trailing newline for tidiness
        out.trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stl::core::TimeInterval;
    use std::time::Duration;

    #[test]
    fn test_true_false_display_and_tree() {
        let t = FormulaDefinition::True;
        let f = FormulaDefinition::False;
        assert_eq!(format!("{}", t), "True");
        assert_eq!(t.to_tree_string(0), "True");
        assert_eq!(format!("{}", f), "False");
        assert_eq!(f.to_tree_string(0), "False");
    }

    #[test]
    fn test_predicates_and_not() {
        let gt = FormulaDefinition::GreaterThan("x", 5.0);
        let lt = FormulaDefinition::LessThan("y", 2.5);
        let not = FormulaDefinition::Not(Box::new(FormulaDefinition::GreaterThan("z", 1.0)));

        assert_eq!(format!("{}", gt), "x > 5");
        assert_eq!(gt.to_tree_string(0), "x > 5");

        assert_eq!(format!("{}", lt), "y < 2.5");
        assert_eq!(lt.to_tree_string(0), "y < 2.5");

        assert_eq!(format!("{}", not), "¬(z > 1)");
        assert_eq!(not.to_tree_string(0), "Not\n    └── z > 1");
    }

    #[test]
    fn test_logical_binary_and_or_implies() {
        let a = FormulaDefinition::GreaterThan("x", 1.0);
        let b = FormulaDefinition::LessThan("y", 2.0);

        let and = FormulaDefinition::And(Box::new(a.clone()), Box::new(b.clone()));
        let or = FormulaDefinition::Or(Box::new(a.clone()), Box::new(b.clone()));
        let imp = FormulaDefinition::Implies(Box::new(a.clone()), Box::new(b.clone()));

        assert_eq!(format!("{}", and), "(x > 1) ∧ (y < 2)");
        assert_eq!(and.to_tree_string(0), "And\n    ├── x > 1\n    └── y < 2");

        assert_eq!(format!("{}", or), "(x > 1) v (y < 2)");
        assert_eq!(or.to_tree_string(0), "Or\n    ├── x > 1\n    └── y < 2");

        assert_eq!(format!("{}", imp), "(x > 1) → (y < 2)");
        assert_eq!(
            imp.to_tree_string(0),
            "Implies\n    ├── x > 1\n    └── y < 2"
        );
    }

    #[test]
    fn test_temporal_and_until() {
        let interval = TimeInterval {
            start: Duration::from_secs(0),
            end: Duration::from_secs(2),
        };
        let ev = FormulaDefinition::Eventually(interval, Box::new(FormulaDefinition::True));
        let gl = FormulaDefinition::Globally(
            TimeInterval {
                start: Duration::from_secs(1),
                end: Duration::from_secs(3),
            },
            Box::new(FormulaDefinition::False),
        );
        let until = FormulaDefinition::Until(
            TimeInterval {
                start: Duration::from_secs(0),
                end: Duration::from_secs(5),
            },
            Box::new(FormulaDefinition::GreaterThan("a", 0.0)),
            Box::new(FormulaDefinition::LessThan("b", 10.0)),
        );

        assert_eq!(format!("{}", ev), "F[0, 2](True)");
        assert_eq!(ev.to_tree_string(0), "Eventually[0,2]\n    └── True");

        assert_eq!(format!("{}", gl), "G[1, 3](False)");
        assert_eq!(gl.to_tree_string(0), "Globally[1,3]\n    └── False");

        assert_eq!(format!("{}", until), "(a > 0) U[0, 5] (b < 10)");
        assert_eq!(
            until.to_tree_string(0),
            "Until[0,5]\n    ├── a > 0\n    └── b < 10"
        );
    }

    #[test]
    fn test_variable_predicates() {
        let gt_var = FormulaDefinition::GreaterThanVar("x", "A");
        let lt_var = FormulaDefinition::LessThanVar("y", "B");

        assert_eq!(format!("{}", gt_var), "x > $A");
        assert_eq!(gt_var.to_tree_string(0), "x > $A");

        assert_eq!(format!("{}", lt_var), "y < $B");
        assert_eq!(lt_var.to_tree_string(0), "y < $B");
    }

    #[test]
    fn test_get_variables_nested() {
        // Test that get_variable_identifiers traverses all nesting levels:
        // Not, And, Or, Implies, Eventually, Globally, Until
        let formula = FormulaDefinition::And(
            Box::new(FormulaDefinition::Not(Box::new(
                FormulaDefinition::GreaterThanVar("x", "A"),
            ))),
            Box::new(FormulaDefinition::Or(
                Box::new(FormulaDefinition::Implies(
                    Box::new(FormulaDefinition::LessThanVar("y", "B")),
                    Box::new(FormulaDefinition::GreaterThanVar("z", "C")),
                )),
                Box::new(FormulaDefinition::Eventually(
                    TimeInterval {
                        start: Duration::from_secs(0),
                        end: Duration::from_secs(1),
                    },
                    Box::new(FormulaDefinition::Globally(
                        TimeInterval {
                            start: Duration::from_secs(0),
                            end: Duration::from_secs(2),
                        },
                        Box::new(FormulaDefinition::Until(
                            TimeInterval {
                                start: Duration::from_secs(0),
                                end: Duration::from_secs(3),
                            },
                            Box::new(FormulaDefinition::LessThanVar("a", "D")),
                            Box::new(FormulaDefinition::GreaterThanVar("b", "E")),
                        )),
                    )),
                )),
            )),
        );

        let vars = formula.get_variable_identifiers();
        assert_eq!(vars.len(), 5);
        assert!(vars.contains(&"A"));
        assert!(vars.contains(&"B"));
        assert!(vars.contains(&"C"));
        assert!(vars.contains(&"D"));
        assert!(vars.contains(&"E"));
    }

    #[test]
    fn test_deeply_nested_tree_string() {
        // Test deeply nested formula to exercise tree prefix formatting logic
        let formula = FormulaDefinition::And(
            Box::new(FormulaDefinition::Or(
                Box::new(FormulaDefinition::GreaterThan("x", 1.0)),
                Box::new(FormulaDefinition::LessThan("y", 2.0)),
            )),
            Box::new(FormulaDefinition::Not(Box::new(
                FormulaDefinition::GreaterThan("z", 3.0),
            ))),
        );

        let tree = formula.to_tree_string(0);
        assert!(tree.contains("And"));
        assert!(tree.contains("Or"));
        assert!(tree.contains("Not"));
        assert!(tree.contains("x > 1"));
        assert!(tree.contains("y < 2"));
        assert!(tree.contains("z > 3"));
    }

    #[test]
    fn test_get_variables_no_variables() {
        // Formula with no variable predicates
        let formula = FormulaDefinition::And(
            Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
            Box::new(FormulaDefinition::LessThan("y", 10.0)),
        );
        let vars = formula.get_variable_identifiers();
        assert!(vars.is_empty());
    }
}
