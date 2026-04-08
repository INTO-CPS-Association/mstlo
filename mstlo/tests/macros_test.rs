use mstlo::formula_definition::FormulaDefinition;
#[cfg(test)]
use mstlo::stl;

#[test]
fn test_stl_macro() {
    let formula: FormulaDefinition = stl!(
        G[0,5]((signal>5) and ((x>0)U[0,2](true)))
    );
    println!("{}", formula.to_tree_string(2))
}
#[test]
fn test_stl_macro_aliases() {
    let formula: FormulaDefinition = stl! {
        globally[0,5]((signal>5) and ((x>0)until[0,2](true)))
    };
    println!("{}", formula.to_tree_string(2))
}
#[test]
fn test_stl_macro_2() {
    let formula: FormulaDefinition = stl! {
        eventually [0, 2] (x > 0) and (globally [0, 2] (x > 0))
    };
    println!("{}", formula.to_tree_string(2))
}
#[test]
fn test_stl_macro_2_aliases() {
    let formula: FormulaDefinition = stl! {
       (F [0, 2] (x > 5)) && (G[0, 2] (x > 0))
    };
    println!("{}", formula.to_tree_string(2))
}
#[test]
fn test_stl_object_in_stl_macro() {
    let formula_1: FormulaDefinition = stl! {
        (eventually [0,5] (x > 5)) and (globally [0, 1] x >= 0)
    };
    let formula_2: FormulaDefinition = stl! {
        (formula_1) or (false)
    };
    println!("{}", formula_2.to_tree_string(2))
}

// New tests for relaxed parenthesis syntax
#[test]
fn test_stl_no_parens_and() {
    // Simple conjunction without parentheses
    let formula: FormulaDefinition = stl!(x > 0 && y < 10);
    println!("{}", formula.to_tree_string(2));
    // Verify it's an And with the right structure
    match formula {
        FormulaDefinition::And(left, right) => {
            assert!(matches!(*left, FormulaDefinition::GreaterThan("x", _)));
            assert!(matches!(*right, FormulaDefinition::LessThan("y", _)));
        }
        _ => panic!("Expected And formula"),
    }
}

#[test]
fn test_stl_no_parens_or() {
    // Simple disjunction without parentheses
    let formula: FormulaDefinition = stl!(x > 0 || y < 10);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::Or(left, right) => {
            assert!(matches!(*left, FormulaDefinition::GreaterThan("x", _)));
            assert!(matches!(*right, FormulaDefinition::LessThan("y", _)));
        }
        _ => panic!("Expected Or formula"),
    }
}

#[test]
fn test_stl_no_parens_keyword() {
    // Keyword operators without parentheses
    let formula: FormulaDefinition = stl!(x > 0 and y < 10);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::And(left, right) => {
            assert!(matches!(*left, FormulaDefinition::GreaterThan("x", _)));
            assert!(matches!(*right, FormulaDefinition::LessThan("y", _)));
        }
        _ => panic!("Expected And formula"),
    }
}

#[test]
fn test_stl_no_parens_chained() {
    // Chained operators (left-associative for same precedence)
    let formula: FormulaDefinition = stl!(x > 0 && y < 10 && z > 5);
    println!("{}", formula.to_tree_string(2));
    // Should be ((x > 0) && (y < 10)) && (z > 5)
    match formula {
        FormulaDefinition::And(left, right) => {
            assert!(matches!(*right, FormulaDefinition::GreaterThan("z", _)));
            match *left {
                FormulaDefinition::And(l2, r2) => {
                    assert!(matches!(*l2, FormulaDefinition::GreaterThan("x", _)));
                    assert!(matches!(*r2, FormulaDefinition::LessThan("y", _)));
                }
                _ => panic!("Expected nested And"),
            }
        }
        _ => panic!("Expected And formula"),
    }
}

#[test]
fn test_stl_no_parens_mixed_precedence() {
    // && has higher precedence than ||
    // x > 0 || y < 10 && z > 5  should parse as  x > 0 || (y < 10 && z > 5)
    let formula: FormulaDefinition = stl!(x > 0 || y < 10 && z > 5);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::Or(left, right) => {
            assert!(matches!(*left, FormulaDefinition::GreaterThan("x", _)));
            match *right {
                FormulaDefinition::And(l2, r2) => {
                    assert!(matches!(*l2, FormulaDefinition::LessThan("y", _)));
                    assert!(matches!(*r2, FormulaDefinition::GreaterThan("z", _)));
                }
                _ => panic!("Expected And on right"),
            }
        }
        _ => panic!("Expected Or formula"),
    }
}

#[test]
fn test_stl_no_parens_with_temporal() {
    // Temporal operators inside without parens
    let formula: FormulaDefinition = stl!(G[0, 5](x > 0 && y < 10));
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::Globally(_, sub) => match *sub {
            FormulaDefinition::And(left, right) => {
                assert!(matches!(*left, FormulaDefinition::GreaterThan("x", _)));
                assert!(matches!(*right, FormulaDefinition::LessThan("y", _)));
            }
            _ => panic!("Expected And inside Globally"),
        },
        _ => panic!("Expected Globally formula"),
    }
}

#[test]
fn test_stl_no_parens_implies() {
    // Implication without parentheses (right-associative)
    let formula: FormulaDefinition = stl!(x > 0 -> y < 10);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::Implies(left, right) => {
            assert!(matches!(*left, FormulaDefinition::GreaterThan("x", _)));
            assert!(matches!(*right, FormulaDefinition::LessThan("y", _)));
        }
        _ => panic!("Expected Implies formula"),
    }
}

#[test]
fn test_stl_no_parens_until() {
    // Until without parentheses
    let formula: FormulaDefinition = stl!(x > 0 U[0, 5] y < 10);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::Until(_, left, right) => {
            assert!(matches!(*left, FormulaDefinition::GreaterThan("x", _)));
            assert!(matches!(*right, FormulaDefinition::LessThan("y", _)));
        }
        _ => panic!("Expected Until formula"),
    }
}

#[test]
fn test_stl_macro_variable_greater_than() {
    // Variable reference with $ prefix
    let formula: FormulaDefinition = stl!(x > $threshold);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::GreaterThanVar(signal, var) => {
            assert_eq!(signal, "x");
            assert_eq!(var, "threshold");
        }
        _ => panic!("Expected GreaterThanVar formula"),
    }
}

#[test]
fn test_stl_macro_variable_less_than() {
    // Variable reference with $ prefix
    let formula: FormulaDefinition = stl!(temp < $limit);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::LessThanVar(signal, var) => {
            assert_eq!(signal, "temp");
            assert_eq!(var, "limit");
        }
        _ => panic!("Expected LessThanVar formula"),
    }
}

#[test]
fn test_stl_macro_variable_in_temporal() {
    // Variable inside temporal operator
    let formula: FormulaDefinition = stl!(G[0, 5](signal > $threshold));
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::Globally(_, sub) => match *sub {
            FormulaDefinition::GreaterThanVar(signal, var) => {
                assert_eq!(signal, "signal");
                assert_eq!(var, "threshold");
            }
            _ => panic!("Expected GreaterThanVar inside Globally"),
        },
        _ => panic!("Expected Globally formula"),
    }
}

#[test]
fn test_stl_macro_variable_combined() {
    // Mix of constant and variable predicates
    let formula: FormulaDefinition = stl!(x > 5 && y < $limit);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::And(left, right) => {
            assert!(matches!(*left, FormulaDefinition::GreaterThan("x", _)));
            match *right {
                FormulaDefinition::LessThanVar(signal, var) => {
                    assert_eq!(signal, "y");
                    assert_eq!(var, "limit");
                }
                _ => panic!("Expected LessThanVar on right"),
            }
        }
        _ => panic!("Expected And formula"),
    }
}

#[test]
fn test_stl_macro_variable_greater_equal() {
    // >= with variable (syntactic sugar for !(x < $var))
    let formula: FormulaDefinition = stl!(x >= $threshold);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::Not(inner) => match *inner {
            FormulaDefinition::LessThanVar(signal, var) => {
                assert_eq!(signal, "x");
                assert_eq!(var, "threshold");
            }
            _ => panic!("Expected LessThanVar inside Not"),
        },
        _ => panic!("Expected Not formula for >= sugar"),
    }
}

#[test]
fn test_stl_macro_variable_less_equal() {
    // <= with variable (syntactic sugar for !(x > $var))
    let formula: FormulaDefinition = stl!(x <= $threshold);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::Not(inner) => match *inner {
            FormulaDefinition::GreaterThanVar(signal, var) => {
                assert_eq!(signal, "x");
                assert_eq!(var, "threshold");
            }
            _ => panic!("Expected GreaterThanVar inside Not"),
        },
        _ => panic!("Expected Not formula for <= sugar"),
    }
}

#[test]
fn test_stl_macro_variable_equal() {
    // == with variable (syntactic sugar for !(x < $var) && !(x > $var))
    let formula: FormulaDefinition = stl!(x == $target);
    println!("{}", formula.to_tree_string(2));
    match formula {
        FormulaDefinition::And(left, right) => {
            match *left {
                FormulaDefinition::Not(inner) => match *inner {
                    FormulaDefinition::LessThanVar(signal, var) => {
                        assert_eq!(signal, "x");
                        assert_eq!(var, "target");
                    }
                    _ => panic!("Expected LessThanVar inside Not on left"),
                },
                _ => panic!("Expected Not on left side of =="),
            }
            match *right {
                FormulaDefinition::Not(inner) => match *inner {
                    FormulaDefinition::GreaterThanVar(signal, var) => {
                        assert_eq!(signal, "x");
                        assert_eq!(var, "target");
                    }
                    _ => panic!("Expected GreaterThanVar inside Not on right"),
                },
                _ => panic!("Expected Not on right side of =="),
            }
        }
        _ => panic!("Expected And formula for == sugar"),
    }
}
