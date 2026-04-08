//! Procedural macro for STL (Signal Temporal Logic) formula definitions.
//!
//! This crate provides the `stl!` macro for defining STL formulas with a DSL syntax.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitBool, LitInt, Result, Token, bracketed, parenthesized};

/// Convenience macro to construct a `::mstlo::ring_buffer::Step`.
///
/// # Syntax
///
/// ```ignore
/// step!("x", 5.5, 10ms);
/// step!("x", 5.5, 1s);
/// step!("x", 5.5, std::time::Duration::from_millis(10));
/// ```
///
/// Supported integer duration suffixes are: `ns`, `us`, `ms`, and `s`.
#[proc_macro]
pub fn step(input: TokenStream) -> TokenStream {
    let parsed = syn::parse_macro_input!(input as StepMacroInput);

    let signal = parsed.signal;
    let value = parsed.value;

    let ts_tokens = match parsed.timestamp {
        StepTimestamp::Suffixed(lit) => match duration_expr_from_lit(&lit) {
            Ok(tokens) => tokens,
            Err(err) => return err.to_compile_error().into(),
        },
        StepTimestamp::Expr(expr) => quote! { #expr },
    };

    quote! {
        ::mstlo::ring_buffer::Step::new(#signal, #value, #ts_tokens)
    }
    .into()
}

struct StepMacroInput {
    signal: Expr,
    value: Expr,
    timestamp: StepTimestamp,
}

enum StepTimestamp {
    Suffixed(LitInt),
    Expr(Expr),
}

impl Parse for StepMacroInput {
    fn parse(input: ParseStream) -> Result<Self> {
        let signal = input.parse::<Expr>()?;
        input.parse::<Token![,]>()?;
        let value = input.parse::<Expr>()?;
        input.parse::<Token![,]>()?;

        let timestamp = if input.peek(LitInt) {
            StepTimestamp::Suffixed(input.parse::<LitInt>()?)
        } else {
            StepTimestamp::Expr(input.parse::<Expr>()?)
        };

        if !input.is_empty() {
            return Err(input.error("unexpected tokens after timestamp argument"));
        }

        Ok(Self {
            signal,
            value,
            timestamp,
        })
    }
}

fn duration_expr_from_lit(lit: &LitInt) -> Result<TokenStream2> {
    let value = lit.base10_parse::<u64>()?;
    let suffix = lit.suffix();

    match suffix {
        "ns" => Ok(quote! { ::std::time::Duration::from_nanos(#value) }),
        "us" => Ok(quote! { ::std::time::Duration::from_micros(#value) }),
        "ms" => Ok(quote! { ::std::time::Duration::from_millis(#value) }),
        "s" => Ok(quote! { ::std::time::Duration::from_secs(#value) }),
        _ => Err(syn::Error::new(
            lit.span(),
            "unsupported duration suffix; use one of: ns, us, ms, s",
        )),
    }
}

/// The main entry point for the `stl!` procedural macro.
///
/// # Syntax
///
/// ## Atomics
/// - `true` - Always true
/// - `false` - Always false
///
/// ## Predicates
/// - `signal > value` - Signal greater than value
/// - `signal < value` - Signal less than value
/// - `signal >= value` - Signal greater than or equal to value (sugar for `!(signal < value)`)
/// - `signal <= value` - Signal less than or equal to value (sugar for `!(signal > value)`)
/// - `signal == value` - Signal equal to value (sugar for `(signal >= value) && (signal <= value)`)
///
/// ## Variable Predicates
/// Use `$variable` syntax to reference runtime variables that can be updated at runtime:
/// - `signal > $threshold` - Signal greater than runtime variable `threshold`
/// - `signal < $limit` - Signal less than runtime variable `limit`
/// - `signal >= $min` - Signal greater than or equal to runtime variable `min`
/// - `signal <= $max` - Signal less than or equal to runtime variable `max`
/// - `signal == $target` - Signal equal to runtime variable `target`
///
/// ## Unary Operators
/// - `!(sub)` or `not(sub)` - Negation
/// - `G[start, end](sub)` or `globally[start, end](sub)` - Globally
/// - `F[start, end](sub)` or `eventually[start, end](sub)` - Eventually
///
/// ## Binary Operators (parentheses optional for simple operands)
/// - `left && right` or `left and right` - Conjunction
/// - `left || right` or `left or right` - Disjunction  
/// - `left -> right` or `left implies right` - Implication
/// - `left U[start, end] right` or `left until[start, end] right` - Until
///
/// ## Operator Precedence
/// Operators have the following precedence (from lowest to highest):
/// - `Implication (->, implies)` : Lowest (1, right-associative)
/// - `Or (||, or)`               : (2)
/// - `And (&&, and)`             : (3)
/// - `Until (U, until)`          : (4)
/// - `Unary (F, G) and Atoms`               : Highest (5)
///
/// # Examples
///
/// ```ignore
/// // With parentheses
/// let formula = stl!(G[0, 5]((signal > 5) and ((x > 0) U[0, 2] (true))));
/// // Without parentheses using operator precedence
/// let formula = stl!(x > 0 && y < 10);
/// let formula = stl!(G[0, 5](x > 0 && y < 10));
/// // With runtime variables
/// let formula = stl!(G[0, 5](signal > $threshold));
/// let formula = stl!(x > 5 && y < $limit);
/// ```
#[proc_macro]
pub fn stl(input: TokenStream) -> TokenStream {
    let input2 = TokenStream2::from(input);

    match parse_stl_formula(input2) {
        Ok(output) => output.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn parse_stl_formula(input: TokenStream2) -> Result<TokenStream2> {
    // since the Parse trait is implemented for StlFormula, we can use syn to parse it
    let formula: StlFormula = syn::parse2(input)?;
    Ok(formula.to_token_stream())
}

/// Represents a predicate value - either a constant expression or a variable reference
enum PredicateValue {
    /// A constant expression (e.g., `5`, `threshold`, `(x + 1)`)
    Const(Expr),
    /// A variable reference using $ syntax (e.g., `$threshold`)
    Var(Ident),
}

/// Represents a parsed STL formula.
enum StlFormula {
    True,
    False,
    GreaterThan(Ident, Expr),
    LessThan(Ident, Expr),
    /// Variable-based greater-than: signal > $variable
    GreaterThanVar(Ident, Ident),
    /// Variable-based less-than: signal < $variable
    LessThanVar(Ident, Ident),
    Not(Box<StlFormula>),
    And(Box<StlFormula>, Box<StlFormula>),
    Or(Box<StlFormula>, Box<StlFormula>),
    Implies(Box<StlFormula>, Box<StlFormula>),
    Globally(Expr, Expr, Box<StlFormula>),
    Eventually(Expr, Expr, Box<StlFormula>),
    Until(Expr, Expr, Box<StlFormula>, Box<StlFormula>),
    /// Interpolation: an arbitrary expression that evaluates to FormulaDefinition
    Interpolated(Expr),
}

impl StlFormula {
    fn to_token_stream(&self) -> TokenStream2 {
        match self {
            StlFormula::True => quote! {
                ::mstlo::formula_definition::FormulaDefinition::True
            },
            StlFormula::False => quote! {
                ::mstlo::formula_definition::FormulaDefinition::False
            },
            StlFormula::GreaterThan(signal, val) => {
                let signal_str = signal.to_string();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::GreaterThan(#signal_str, #val as f64)
                }
            }
            StlFormula::LessThan(signal, val) => {
                let signal_str = signal.to_string();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::LessThan(#signal_str, #val as f64)
                }
            }
            StlFormula::GreaterThanVar(signal, var) => {
                let signal_str = signal.to_string();
                let var_str = var.to_string();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::GreaterThanVar(#signal_str, #var_str)
                }
            }
            StlFormula::LessThanVar(signal, var) => {
                let signal_str = signal.to_string();
                let var_str = var.to_string();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::LessThanVar(#signal_str, #var_str)
                }
            }
            StlFormula::Not(sub) => {
                let sub_tokens = sub.to_token_stream();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::Not(
                        Box::new(#sub_tokens)
                    )
                }
            }
            StlFormula::And(left, right) => {
                let left_tokens = left.to_token_stream();
                let right_tokens = right.to_token_stream();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::And(
                        Box::new(#left_tokens),
                        Box::new(#right_tokens)
                    )
                }
            }
            StlFormula::Or(left, right) => {
                let left_tokens = left.to_token_stream();
                let right_tokens = right.to_token_stream();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::Or(
                        Box::new(#left_tokens),
                        Box::new(#right_tokens)
                    )
                }
            }
            StlFormula::Implies(left, right) => {
                let left_tokens = left.to_token_stream();
                let right_tokens = right.to_token_stream();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::Implies(
                        Box::new(#left_tokens),
                        Box::new(#right_tokens)
                    )
                }
            }
            StlFormula::Globally(start, end, sub) => {
                let sub_tokens = sub.to_token_stream();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::Globally(
                        ::mstlo::core::TimeInterval {
                            start: ::std::time::Duration::from_secs(#start as u64),
                            end: ::std::time::Duration::from_secs(#end as u64),
                        },
                        Box::new(#sub_tokens)
                    )
                }
            }
            StlFormula::Eventually(start, end, sub) => {
                let sub_tokens = sub.to_token_stream();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::Eventually(
                        ::mstlo::core::TimeInterval {
                            start: ::std::time::Duration::from_secs(#start as u64),
                            end: ::std::time::Duration::from_secs(#end as u64),
                        },
                        Box::new(#sub_tokens)
                    )
                }
            }
            StlFormula::Until(start, end, left, right) => {
                let left_tokens = left.to_token_stream();
                let right_tokens = right.to_token_stream();
                quote! {
                    ::mstlo::formula_definition::FormulaDefinition::Until(
                        ::mstlo::core::TimeInterval {
                            start: ::std::time::Duration::from_secs(#start as u64),
                            end: ::std::time::Duration::from_secs(#end as u64),
                        },
                        Box::new(#left_tokens),
                        Box::new(#right_tokens)
                    )
                }
            }
            StlFormula::Interpolated(expr) => {
                quote! { #expr }
            }
        }
    }
}

impl Parse for StlFormula {
    fn parse(input: ParseStream) -> Result<Self> {
        parse_expr(input, 0) // Start parsing with minimum precedence 0
    }
}

/// Operator precedence levels (higher = binds tighter)
/// Implication (->, implies) : 1 (lowest, right-associative)
/// Or (||, or)               : 2
/// And (&&, and)             : 3
/// Until (U, until)          : 4
/// Unary/Atoms               : 5 (highest)
fn get_binary_op_precedence(input: ParseStream) -> Option<u8> {
    if input.peek(Token![->]) {
        return Some(1);
    }
    if input.peek(Token![||]) {
        return Some(2);
    }
    if input.peek(Token![&&]) {
        return Some(3);
    }
    if input.peek(Ident) {
        let fork = input.fork();
        if let Ok(ident) = fork.parse::<Ident>() {
            match ident.to_string().as_str() {
                "implies" => return Some(1),
                "or" => return Some(2),
                "and" => return Some(3),
                "U" | "until" => return Some(4),
                _ => {}
            }
        }
    }
    None
}

/// Parse an expression with precedence climbing (Pratt parser)
fn parse_expr(input: ParseStream, min_prec: u8) -> Result<StlFormula> {
    let mut left = parse_primary(input)?;

    while let Some(prec) = get_binary_op_precedence(input) {
        if prec < min_prec {
            break;
        }

        left = parse_binary_op(input, left, prec)?;
    }

    Ok(left)
}

/// Parse a binary operator and its right operand
fn parse_binary_op(input: ParseStream, left: StlFormula, prec: u8) -> Result<StlFormula> {
    // Parse the operator
    if input.peek(Token![&&]) {
        let op_token = input.parse::<Token![&&]>()?;
        if input.is_empty() {
            return Err(syn::Error::new(
                op_token.spans[0],
                "missing right operand after '&&'\n  help: conjunction requires a formula on both sides, e.g., `x > 0 && y < 10`",
            ));
        }
        let right = parse_expr(input, prec + 1)?;
        return Ok(StlFormula::And(Box::new(left), Box::new(right)));
    }

    if input.peek(Token![||]) {
        let op_token = input.parse::<Token![||]>()?;
        if input.is_empty() {
            return Err(syn::Error::new(
                op_token.spans[0],
                "missing right operand after '||'\n  help: disjunction requires a formula on both sides, e.g., `x > 0 || y < 10`",
            ));
        }
        let right = parse_expr(input, prec + 1)?;
        return Ok(StlFormula::Or(Box::new(left), Box::new(right)));
    }

    if input.peek(Token![->]) {
        let op_token = input.parse::<Token![->]>()?;
        if input.is_empty() {
            return Err(syn::Error::new(
                op_token.spans[0],
                "missing right operand after '->'\n  help: implication requires a formula on both sides, e.g., `x > 0 -> y < 10`",
            ));
        }
        // Implication is right-associative, so use same precedence
        let right = parse_expr(input, prec)?;
        return Ok(StlFormula::Implies(Box::new(left), Box::new(right)));
    }

    if input.peek(Ident) {
        let fork = input.fork();
        let ident: Ident = fork.parse()?;
        let ident_str = ident.to_string();

        match ident_str.as_str() {
            "and" => {
                let op_ident = input.parse::<Ident>()?;
                if input.is_empty() {
                    return Err(syn::Error::new(
                        op_ident.span(),
                        "missing right operand after 'and'\n  help: conjunction requires a formula on both sides, e.g., `(x > 0) and (y < 10)`",
                    ));
                }
                let right = parse_expr(input, prec + 1)?;
                return Ok(StlFormula::And(Box::new(left), Box::new(right)));
            }
            "or" => {
                let op_ident = input.parse::<Ident>()?;
                if input.is_empty() {
                    return Err(syn::Error::new(
                        op_ident.span(),
                        "missing right operand after 'or'\n  help: disjunction requires a formula on both sides, e.g., `(x > 0) or (y < 10)`",
                    ));
                }
                let right = parse_expr(input, prec + 1)?;
                return Ok(StlFormula::Or(Box::new(left), Box::new(right)));
            }
            "implies" => {
                let op_ident = input.parse::<Ident>()?;
                if input.is_empty() {
                    return Err(syn::Error::new(
                        op_ident.span(),
                        "missing right operand after 'implies'\n  help: implication requires a formula on both sides, e.g., `(x > 0) implies (y < 10)`",
                    ));
                }
                // Right-associative
                let right = parse_expr(input, prec)?;
                return Ok(StlFormula::Implies(Box::new(left), Box::new(right)));
            }
            "U" | "until" => {
                let op_ident = input.parse::<Ident>()?;

                if !input.peek(syn::token::Bracket) {
                    return Err(syn::Error::new(
                        op_ident.span(),
                        format!(
                            "missing time interval after '{}'\n  \
                            help: the '{}' operator requires a time interval, e.g., `left {}[0, 10] right`",
                            ident_str, ident_str, ident_str
                        ),
                    ));
                }

                let (start, end) = parse_time_interval(input)?;

                if input.is_empty() {
                    return Err(syn::Error::new(
                        op_ident.span(),
                        format!(
                            "missing right operand after '{}[_, _]'\n  \
                            help: until requires a formula on both sides, e.g., `(x > 0) {}[0, 10] (y < 10)`",
                            ident_str, ident_str
                        ),
                    ));
                }

                let right = parse_expr(input, prec + 1)?;
                return Ok(StlFormula::Until(
                    start,
                    end,
                    Box::new(left),
                    Box::new(right),
                ));
            }
            _ => {}
        }
    }

    Err(syn::Error::new(
        input.span(),
        "expected binary operator (&&, ||, ->, and, or, implies, U[_, _], until[_, _])",
    ))
}

/// Parse a primary expression (atoms, unary operators, parenthesized expressions)
fn parse_primary(input: ParseStream) -> Result<StlFormula> {
    // Check for Rust boolean literals first
    if input.peek(LitBool) {
        let lit: LitBool = input.parse()?;
        return if lit.value {
            Ok(StlFormula::True)
        } else {
            Ok(StlFormula::False)
        };
    }

    // Check for negation: !
    if input.peek(Token![!]) {
        return parse_not(input);
    }

    // Check for keyword operators and predicates
    if input.peek(Ident) {
        let fork = input.fork();
        let ident: Ident = fork.parse()?;
        let ident_str = ident.to_string();

        match ident_str.as_str() {
            "not" => return parse_not_keyword(input),
            "G" | "globally" => return parse_globally(input),
            "F" | "eventually" => return parse_eventually(input),
            _ => {
                // Try to parse as a predicate
                if let Ok(predicate) = try_parse_predicate(input) {
                    return Ok(predicate);
                }
            }
        }
    }

    // Check for parenthesized expression
    if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);
        let inner = parse_expr(&content, 0)?;

        if !content.is_empty() {
            return Err(content.error("unexpected tokens inside parentheses"));
        }

        return Ok(inner);
    }

    // Fallback: parse as an arbitrary expression (interpolation)
    let expr: Expr = input.parse()?;
    Ok(StlFormula::Interpolated(expr))
}

/// Parse negation: !(sub) or !sub for simple atoms
fn parse_not(input: ParseStream) -> Result<StlFormula> {
    let not_token = input.parse::<Token![!]>()?;

    if input.is_empty() {
        return Err(syn::Error::new(
            not_token.span,
            "missing operand after '!'\n  \
            help: negation requires a subformula, e.g., `!(x > 5)` or `!true`",
        ));
    }

    // Allow both !(sub) with parentheses and !sub without for simple expressions
    if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);

        if content.is_empty() {
            return Err(content.error(
                "empty subformula in negation\n  \
                help: negation requires a subformula, e.g., `!(x > 5)`",
            ));
        }

        let sub = parse_expr(&content, 0)?;

        if !content.is_empty() {
            return Err(content.error(
                "unexpected tokens after negation subformula\n  \
                help: if you want to negate a complex formula, wrap it in parentheses: `!(a && b)`",
            ));
        }

        Ok(StlFormula::Not(Box::new(sub)))
    } else {
        // Parse the next primary expression
        let sub = parse_primary(input)?;
        Ok(StlFormula::Not(Box::new(sub)))
    }
}

/// Parse negation keyword: not(sub) or not sub
fn parse_not_keyword(input: ParseStream) -> Result<StlFormula> {
    let ident: Ident = input.parse()?;
    if ident != "not" {
        return Err(syn::Error::new(ident.span(), "expected 'not'"));
    }

    if input.is_empty() {
        return Err(syn::Error::new(
            ident.span(),
            "missing operand after 'not'\n  \
            help: negation requires a subformula, e.g., `not(x > 5)` or `not true`",
        ));
    }

    // Allow both not(sub) with parentheses and not sub without
    if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);

        if content.is_empty() {
            return Err(content.error(
                "empty subformula in negation\n  \
                help: negation requires a subformula, e.g., `not(x > 5)`",
            ));
        }

        let sub = parse_expr(&content, 0)?;

        if !content.is_empty() {
            return Err(content.error(
                "unexpected tokens after negation subformula\n  \
                help: if you want to negate a complex formula, wrap it in parentheses: `not(a and b)`"
            ));
        }

        Ok(StlFormula::Not(Box::new(sub)))
    } else {
        let sub = parse_primary(input)?;
        Ok(StlFormula::Not(Box::new(sub)))
    }
}

/// Parse time interval: [start, end]
fn parse_time_interval(input: ParseStream) -> Result<(Expr, Expr)> {
    let content;
    bracketed!(content in input);

    if content.is_empty() {
        return Err(content.error(
            "empty time interval\n  \
            help: time intervals require two values [start, end], e.g., [0, 10]",
        ));
    }

    let start: Expr = content.parse().map_err(|_| {
        syn::Error::new(
            content.span(),
            "invalid start value in time interval\n  \
            help: expected a numeric expression for the interval start, e.g., [0, 10]",
        )
    })?;

    if !content.peek(Token![,]) {
        return Err(syn::Error::new(
            content.span(),
            "missing comma in time interval\n  \
            help: time intervals must have two comma-separated values [start, end], e.g., [0, 10]",
        ));
    }
    content.parse::<Token![,]>()?;

    if content.is_empty() {
        return Err(content.error(
            "missing end value in time interval\n  \
            help: time intervals require two values [start, end], e.g., [0, 10]",
        ));
    }

    let end: Expr = content.parse().map_err(|_| {
        syn::Error::new(
            content.span(),
            "invalid end value in time interval\n  \
            help: expected a numeric expression for the interval end, e.g., [0, 10]",
        )
    })?;

    if !content.is_empty() {
        return Err(content.error(
            "too many values in time interval\n  \
            help: time intervals take exactly two values [start, end], e.g., [0, 10]",
        ));
    }

    Ok((start, end))
}

/// Parse globally: G[start, end]\(sub\) or G[start, end] sub
fn parse_globally(input: ParseStream) -> Result<StlFormula> {
    let ident: Ident = input.parse()?;
    let ident_str = ident.to_string();

    if ident_str != "G" && ident_str != "globally" {
        return Err(syn::Error::new(
            ident.span(),
            format!("expected 'G' or 'globally', found '{}'", ident_str),
        ));
    }

    if !input.peek(syn::token::Bracket) {
        return Err(syn::Error::new(
            ident.span(),
            format!(
                "missing time interval after '{}'\n  \
                help: the globally operator requires a time interval, e.g., `{}[0, 10](x > 5)`\n  \
                note: '{}' means \"always\" within the given time window",
                ident_str, ident_str, ident_str
            ),
        ));
    }

    let (start, end) = parse_time_interval(input)?;

    // Allow both G[a,b](sub) with parentheses and G[a,b] sub without
    let sub = if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);

        if content.is_empty() {
            return Err(content.error(
                "empty subformula in globally operator\n  \
                help: globally requires a subformula, e.g., `G[0, 10](x > 5)`",
            ));
        }

        let sub = parse_expr(&content, 0)?;

        if !content.is_empty() {
            return Err(content.error(
                "unexpected tokens after globally subformula\n  \
                help: if you want to combine formulas, use binary operators outside the parentheses"
            ));
        }
        sub
    } else if input.is_empty() {
        return Err(syn::Error::new(
            ident.span(),
            format!(
                "missing subformula after '{}[_, _]'\n  \
                help: globally requires a subformula, e.g., `{}[0, 10](x > 5)` or `{}[0, 10] x > 5`",
                ident_str, ident_str, ident_str
            ),
        ));
    } else {
        parse_primary(input)?
    };

    Ok(StlFormula::Globally(start, end, Box::new(sub)))
}

/// Parse eventually: F[start, end]\(sub\) or F[start, end] sub  
fn parse_eventually(input: ParseStream) -> Result<StlFormula> {
    let ident: Ident = input.parse()?;
    let ident_str = ident.to_string();

    if ident_str != "F" && ident_str != "eventually" {
        return Err(syn::Error::new(
            ident.span(),
            format!("expected 'F' or 'eventually', found '{}'", ident_str),
        ));
    }

    if !input.peek(syn::token::Bracket) {
        return Err(syn::Error::new(
            ident.span(),
            format!(
                "missing time interval after '{}'\n  \
                help: the eventually operator requires a time interval, e.g., `{}[0, 10](x > 5)`\n  \
                note: '{}' means \"at some point\" within the given time window",
                ident_str, ident_str, ident_str
            ),
        ));
    }

    let (start, end) = parse_time_interval(input)?;

    // Allow both F[a,b](sub) with parentheses and F[a,b] sub without
    let sub = if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);

        if content.is_empty() {
            return Err(content.error(
                "empty subformula in eventually operator\n  \
                help: eventually requires a subformula, e.g., `F[0, 10](x > 5)`",
            ));
        }

        let sub = parse_expr(&content, 0)?;

        if !content.is_empty() {
            return Err(content.error(
                "unexpected tokens after eventually subformula\n  \
                help: if you want to combine formulas, use binary operators outside the parentheses"
            ));
        }
        sub
    } else if input.is_empty() {
        return Err(syn::Error::new(
            ident.span(),
            format!(
                "missing subformula after '{}[_, _]'\n  \
                help: eventually requires a subformula, e.g., `{}[0, 10](x > 5)` or `{}[0, 10] x > 5`",
                ident_str, ident_str, ident_str
            ),
        ));
    } else {
        parse_primary(input)?
    };

    Ok(StlFormula::Eventually(start, end, Box::new(sub)))
}

/// Try to parse a predicate: signal > val, signal < val, signal >= val, signal <= val, signal == val
fn try_parse_predicate(input: ParseStream) -> Result<StlFormula> {
    let fork = input.fork();

    // Must start with an identifier (signal name)
    if !fork.peek(Ident) {
        return Err(syn::Error::new(input.span(), "expected signal identifier"));
    }

    let signal: Ident = fork.parse()?;
    let signal_str = signal.to_string();

    // Check it's not a keyword
    if matches!(
        signal_str.as_str(),
        "true"
            | "false"
            | "not"
            | "and"
            | "or"
            | "implies"
            | "G"
            | "F"
            | "U"
            | "globally"
            | "eventually"
            | "until"
    ) {
        return Err(syn::Error::new(
            signal.span(),
            format!(
                "'{}' is a reserved keyword and cannot be used as a signal name\n  \
                help: use a different identifier for your signal",
                signal_str
            ),
        ));
    }

    // Parse comparison operator
    if fork.peek(Token![>]) && !fork.peek(Token![>=]) {
        fork.parse::<Token![>]>()?;
        let val = parse_predicate_value(&fork).map_err(|_| {
            syn::Error::new(
                fork.span(),
                format!(
                    "invalid value after '>' in predicate\n  \
                    help: expected a numeric value, e.g., `{} > 5` or `{} > $threshold`",
                    signal_str, signal_str
                ),
            )
        })?;

        // Commit the parse
        input.parse::<Ident>()?;
        input.parse::<Token![>]>()?;
        let _: PredicateValue = parse_predicate_value(input)?;

        return match val {
            PredicateValue::Const(expr) => Ok(StlFormula::GreaterThan(signal, expr)),
            PredicateValue::Var(var_ident) => Ok(StlFormula::GreaterThanVar(signal, var_ident)),
        };
    }

    if fork.peek(Token![<]) && !fork.peek(Token![<=]) {
        fork.parse::<Token![<]>()?;
        let val = parse_predicate_value(&fork).map_err(|_| {
            syn::Error::new(
                fork.span(),
                format!(
                    "invalid value after '<' in predicate\n  \
                    help: expected a numeric value, e.g., `{} < 5` or `{} < $threshold`",
                    signal_str, signal_str
                ),
            )
        })?;

        // Commit the parse
        input.parse::<Ident>()?;
        input.parse::<Token![<]>()?;
        let _: PredicateValue = parse_predicate_value(input)?;

        return match val {
            PredicateValue::Const(expr) => Ok(StlFormula::LessThan(signal, expr)),
            PredicateValue::Var(var_ident) => Ok(StlFormula::LessThanVar(signal, var_ident)),
        };
    }

    if fork.peek(Token![>=]) {
        fork.parse::<Token![>=]>()?;
        let val = parse_predicate_value(&fork).map_err(|_| {
            syn::Error::new(
                fork.span(),
                format!(
                    "invalid value after '>=' in predicate\n  \
                    help: expected a numeric value, e.g., `{} >= 5` or `{} >= $threshold`",
                    signal_str, signal_str
                ),
            )
        })?;

        // Commit the parse
        input.parse::<Ident>()?;
        input.parse::<Token![>=]>()?;
        let _: PredicateValue = parse_predicate_value(input)?;

        // >= is sugar for !(signal < val)
        return match val {
            PredicateValue::Const(expr) => Ok(StlFormula::Not(Box::new(StlFormula::LessThan(
                signal, expr,
            )))),
            PredicateValue::Var(var_ident) => Ok(StlFormula::Not(Box::new(
                StlFormula::LessThanVar(signal, var_ident),
            ))),
        };
    }

    if fork.peek(Token![<=]) {
        fork.parse::<Token![<=]>()?;
        let val = parse_predicate_value(&fork).map_err(|_| {
            syn::Error::new(
                fork.span(),
                format!(
                    "invalid value after '<=' in predicate\n  \
                    help: expected a numeric value, e.g., `{} <= 5` or `{} <= $threshold`",
                    signal_str, signal_str
                ),
            )
        })?;

        // Commit the parse
        input.parse::<Ident>()?;
        input.parse::<Token![<=]>()?;
        let _: PredicateValue = parse_predicate_value(input)?;

        // <= is sugar for !(signal > val)
        return match val {
            PredicateValue::Const(expr) => Ok(StlFormula::Not(Box::new(StlFormula::GreaterThan(
                signal, expr,
            )))),
            PredicateValue::Var(var_ident) => Ok(StlFormula::Not(Box::new(
                StlFormula::GreaterThanVar(signal, var_ident),
            ))),
        };
    }

    if fork.peek(Token![==]) {
        fork.parse::<Token![==]>()?;
        let val = parse_predicate_value(&fork).map_err(|_| {
            syn::Error::new(
                fork.span(),
                format!(
                    "invalid value after '==' in predicate\n  \
                    help: expected a numeric value, e.g., `{} == 5` or `{} == $threshold`",
                    signal_str, signal_str
                ),
            )
        })?;

        // Commit the parse
        input.parse::<Ident>()?;
        input.parse::<Token![==]>()?;
        let _: PredicateValue = parse_predicate_value(input)?;

        // == is sugar for (signal >= val) && (signal <= val)
        // Which is: !(signal < val) && !(signal > val)
        return match val {
            PredicateValue::Const(expr) => {
                let gte =
                    StlFormula::Not(Box::new(StlFormula::LessThan(signal.clone(), expr.clone())));
                let lte = StlFormula::Not(Box::new(StlFormula::GreaterThan(signal, expr)));
                Ok(StlFormula::And(Box::new(gte), Box::new(lte)))
            }
            PredicateValue::Var(var_ident) => {
                let gte = StlFormula::Not(Box::new(StlFormula::LessThanVar(
                    signal.clone(),
                    var_ident.clone(),
                )));
                let lte = StlFormula::Not(Box::new(StlFormula::GreaterThanVar(signal, var_ident)));
                Ok(StlFormula::And(Box::new(gte), Box::new(lte)))
            }
        };
    }

    Err(syn::Error::new(
        signal.span(),
        format!(
            "expected comparison operator after signal '{}'\n  \
            help: predicates require a comparison: `{} > value`, `{} < value`, `{} >= value`, `{} <= value`, or `{} == value`",
            signal_str, signal_str, signal_str, signal_str, signal_str, signal_str
        ),
    ))
}

/// Parse a predicate value - stops at binary operators to avoid consuming them
/// Returns either a constant expression or a variable reference ($var)
fn parse_predicate_value(input: ParseStream) -> Result<PredicateValue> {
    // We need to parse a numeric literal or expression, but stop before binary STL operators
    // Use syn's expression parsing but be careful with what we consume

    // Check for variable reference with $ prefix: $threshold
    if input.peek(Token![$]) {
        input.parse::<Token![$]>()?;
        if !input.peek(Ident) {
            return Err(syn::Error::new(
                input.span(),
                "expected identifier after '$'\n  \
                help: variable references should be written as `$variable_name`, e.g., `x > $threshold`",
            ));
        }
        let var_ident: Ident = input.parse()?;
        return Ok(PredicateValue::Var(var_ident));
    }

    // Check for negative numbers: -5, -3.14, etc.
    if input.peek(Token![-]) {
        let neg: Token![-] = input.parse()?;
        if input.peek(syn::LitInt) {
            let lit: syn::LitInt = input.parse()?;
            return Ok(PredicateValue::Const(
                syn::parse_quote_spanned!(neg.span=> -#lit),
            ));
        }
        if input.peek(syn::LitFloat) {
            let lit: syn::LitFloat = input.parse()?;
            return Ok(PredicateValue::Const(
                syn::parse_quote_spanned!(neg.span=> -#lit),
            ));
        }
        // Parenthesized negative expression
        if input.peek(syn::token::Paren) {
            let content;
            parenthesized!(content in input);
            let inner: Expr = content.parse()?;
            return Ok(PredicateValue::Const(
                syn::parse_quote_spanned!(neg.span=> -(#inner)),
            ));
        }
        return Err(syn::Error::new(
            neg.span,
            "invalid negative value\n  \
            help: expected a number after '-', e.g., `-5`, `-3.14`, or `-(expr)`",
        ));
    }

    // Simple literals
    if input.peek(syn::LitInt) {
        let lit: syn::LitInt = input.parse()?;
        return Ok(PredicateValue::Const(Expr::Lit(syn::ExprLit {
            attrs: vec![],
            lit: syn::Lit::Int(lit),
        })));
    }

    if input.peek(syn::LitFloat) {
        let lit: syn::LitFloat = input.parse()?;
        return Ok(PredicateValue::Const(Expr::Lit(syn::ExprLit {
            attrs: vec![],
            lit: syn::Lit::Float(lit),
        })));
    }

    // Parenthesized expression for the value
    if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);
        if content.is_empty() {
            return Err(content.error(
                "empty parentheses in value position\n  \
                help: expected an expression inside parentheses, e.g., `(x + 1)`",
            ));
        }
        let inner: Expr = content.parse()?;
        return Ok(PredicateValue::Const(syn::parse_quote!((#inner))));
    }

    // Variable reference (identifier)
    if input.peek(Ident) {
        let ident: Ident = input.parse()?;
        let ident_str = ident.to_string();

        // Check if this looks like an STL keyword used in value position
        if matches!(
            ident_str.as_str(),
            "and"
                | "or"
                | "implies"
                | "not"
                | "G"
                | "F"
                | "U"
                | "globally"
                | "eventually"
                | "until"
        ) {
            return Err(syn::Error::new(
                ident.span(),
                format!(
                    "'{}' is an STL keyword and cannot be used as a value\n  \
                    help: did you forget the comparison value? e.g., `signal > 5 {} ...`",
                    ident_str, ident_str
                ),
            ));
        }

        return Ok(PredicateValue::Const(Expr::Path(syn::ExprPath {
            attrs: vec![],
            qself: None,
            path: ident.into(),
        })));
    }

    Err(syn::Error::new(
        input.span(),
        "expected a value (number, variable, $var reference, or parenthesized expression)\n  \
        help: valid values include: `5`, `3.14`, `-10`, `threshold`, `$threshold`, `(x + 1)`",
    ))
}
