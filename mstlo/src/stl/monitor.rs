//! High-level STL monitor API.
//!
//! This module provides the user-facing monitor abstraction ([`StlMonitor`]) and
//! its builder ([`StlMonitorBuilder`]). It bridges:
//! - formula definitions ([`FormulaDefinition`]),
//! - executable operator trees (incremental is strictly recommended, naive is mostly for testing), and
//! - optional multi-signal synchronization.
//!
//! It also defines output containers ([`MonitorOutput`], [`SyncStepResult`]) and
//! semantic selection markers used for type-driven output inference.

use crate::ring_buffer::{RingBuffer, Step};
use crate::stl::core::{
    RobustnessSemantics, SignalIdentifier, StlOperatorAndSignalIdentifier, StlOperatorTrait,
};
use crate::stl::formula_definition::FormulaDefinition;
use crate::stl::naive_operators::{StlFormula, StlOperator};
use crate::stl::operators::atomic_operators::Atomic;
use crate::stl::operators::binary_operators::{And, Or};
use crate::stl::operators::not_operator::Not;
use crate::stl::operators::unary_temporal_operators::{Eventually, Globally};
use crate::stl::operators::until_operator::Until;
use crate::synchronizer::{Interpolatable, SynchronizationStrategy, Synchronizer};
use std::fmt::Debug;
use std::fmt::Display;
use std::time::Duration;

/// Defines the monitoring strategy.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Algorithm {
    /// Recursive naive evaluator (`naive_operators`) without incremental caches. (not recommended).
    Naive,
    /// Incremental streaming evaluator (default).
    #[default]
    Incremental,
}

/// Monitoring semantics and evaluation mode.
///
/// This controls both the output domain and short-circuit/refinement behavior.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Semantics {
    /// Robustness Satisfaction interval (RoSI). (`RobustnessInterval`) with refinements.
    RobustnessInterval,
    /// Delayed quantitative robustness (`f64`, default).
    #[default]
    DelayedQuantitative,
    /// Delayed qualitative verdicts (`bool`).
    DelayedQualitative,
    /// Eager qualitative verdicts (`bool`) with short-circuiting.
    EagerQualitative,
}

/// Marker traits and structs for Type-Driven Semantics
pub mod semantic_markers {
    use super::Semantics;
    use crate::stl::core::RobustnessInterval;

    /// Marker trait mapping a type-level semantic marker to runtime [`Semantics`]
    /// and a concrete output type.
    pub trait SemanticType {
        /// Concrete monitor output domain for this semantic family.
        type Output: super::RobustnessSemantics + 'static;
        /// Runtime enum representation used in builder dispatch.
        fn as_enum() -> Semantics;
    }

    /// Marker for RoSI interval semantics.
    #[derive(Debug, Clone, Copy)]
    pub struct Rosi;
    impl SemanticType for Rosi {
        type Output = RobustnessInterval;
        fn as_enum() -> Semantics {
            Semantics::RobustnessInterval
        }
    }

    /// Marker for delayed quantitative semantics (`f64`).
    #[derive(Debug, Clone, Copy)]
    pub struct DelayedQuantitative;
    impl SemanticType for DelayedQuantitative {
        type Output = f64;
        fn as_enum() -> Semantics {
            Semantics::DelayedQuantitative
        }
    }

    /// Marker for delayed qualitative semantics (`bool`).
    #[derive(Debug, Clone, Copy)]
    pub struct DelayedQualitative;
    impl SemanticType for DelayedQualitative {
        type Output = bool;
        fn as_enum() -> Semantics {
            Semantics::DelayedQualitative
        }
    }

    /// Marker for eager qualitative semantics (`bool`).
    #[derive(Debug, Clone, Copy)]
    pub struct EagerQualitative;
    impl SemanticType for EagerQualitative {
        type Output = bool;
        fn as_enum() -> Semantics {
            Semantics::EagerQualitative
        }
    }
}

// Re-export markers for easier access like `monitor::DelayedQualitative`
pub use semantic_markers::{DelayedQualitative, DelayedQuantitative, EagerQualitative, Rosi};

/// Represents the output of a single monitor update operation.
///
/// # Accessing results
///
/// The most common way to access results is via [`verdicts()`](Self::verdicts),
/// which returns one finalized verdict per timestamp (deduplicating refinements).
///
/// You can also iterate directly:
/// ```ignore
/// for verdict in &output {
///     println!("t={:?}: {:?}", verdict.timestamp, verdict.value);
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorOutput<T, Y> {
    /// Signal name of the original input step that triggered this output object.
    pub input_signal: &'static str,
    /// Timestamp of the original input step.
    pub input_timestamp: Duration,
    /// Value of the original input step.
    pub input_value: T,
    /// Internal per-synchronized-step evaluations (synchronization detail).
    evaluations: Vec<SyncStepResult<T, Y>>,
}

impl<T, Y> Display for MonitorOutput<T, Y>
where
    Y: Debug + Clone,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let finalized = self.verdicts();

        if finalized.is_empty() {
            return write!(f, "No verdicts available");
        }

        for (i, step) in finalized.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "t={:?}: {:?}", step.timestamp, step.value)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
/// Result of evaluating one synchronized step through the root operator.
pub struct SyncStepResult<T, Y> {
    /// Synchronized input step presented to the operator tree.
    pub sync_step: Step<T>,
    /// Output steps emitted for this synchronized input step.
    pub outputs: Vec<Step<Y>>,
}

impl<T, Y> SyncStepResult<T, Y> {
    /// Creates a new synchronized-step evaluation result.
    pub fn new(sync_step: Step<T>, outputs: Vec<Step<Y>>) -> Self {
        SyncStepResult { sync_step, outputs }
    }

    /// Returns `true` if this synchronized step produced any outputs.
    pub fn has_outputs(&self) -> bool {
        !self.outputs.is_empty()
    }
}

impl<T, Y> MonitorOutput<T, Y> {
    /// Creates a monitor output from an original input step and collected evaluations.
    pub fn new(input: &Step<T>, evaluations: Vec<SyncStepResult<T, Y>>) -> Self
    where
        T: Clone,
    {
        MonitorOutput {
            input_signal: input.signal,
            input_timestamp: input.timestamp,
            input_value: input.value.clone(),
            evaluations,
        }
    }

    // ── Primary API (what most users need) ──────────────────────────

    /// Returns one finalized verdict per timestamp.
    ///
    /// When multiple outputs exist at the same timestamp (e.g. due to
    /// interval refinement in RoSI mode), the last (most refined) value
    /// is retained.
    ///
    /// This is the **recommended** way to consume monitor results.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let output = monitor.update(&step);
    /// for verdict in output.verdicts() {
    ///     println!("t={:?}: {:?}", verdict.timestamp, verdict.value);
    /// }
    /// ```
    pub fn verdicts(&self) -> Vec<Step<Y>>
    where
        Y: Clone,
    {
        let mut latest_map = std::collections::BTreeMap::new();
        for output in self.raw_outputs() {
            latest_map.insert(output.timestamp, output.value.clone());
        }
        latest_map
            .into_iter()
            .map(|(ts, val)| Step::new(self.input_signal, val, ts))
            .collect()
    }

    /// Consuming version of [`verdicts()`](Self::verdicts) — avoids cloning
    /// when you own the output.
    pub fn into_verdicts(self) -> Vec<Step<Y>> {
        let signal = self.input_signal;
        let mut latest_map = std::collections::BTreeMap::new();
        for eval in self.evaluations {
            for output in eval.outputs {
                latest_map.insert(output.timestamp, output.value);
            }
        }
        latest_map
            .into_iter()
            .map(|(ts, val)| Step::new(signal, val, ts))
            .collect()
    }

    /// Returns `true` if at least one verdict was produced.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let output = monitor.update(&step);
    /// if output.has_verdicts() {
    ///     println!("Got results!");
    /// }
    /// ```
    pub fn has_verdicts(&self) -> bool {
        self.evaluations.iter().any(|e| !e.outputs.is_empty())
    }

    /// Returns the latest verdict at the given timestamp, if present.
    ///
    /// When refinement produces multiple values at the same timestamp,
    /// the last (most refined) value is returned.
    pub fn verdict_at(&self, timestamp: Duration) -> Option<&Y> {
        self.raw_outputs()
            .filter(|s| s.timestamp == timestamp)
            .last()
            .map(|s| &s.value)
    }

    /// Returns the most recent verdict by timestamp, if any.
    pub fn latest_verdict(&self) -> Option<&Step<Y>> {
        self.raw_outputs().last()
    }

    /// Returns `true` if no synchronized evaluations occurred
    /// (e.g. a multi-signal formula waiting for the other signal).
    pub fn is_pending(&self) -> bool {
        self.evaluations.is_empty()
    }

    // ── Advanced / diagnostic API ───────────────────────────────────

    /// Iterates over all emitted output steps in evaluation order,
    /// **including** intermediate refinements.
    ///
    /// Most users should prefer [`verdicts()`](Self::verdicts) instead.
    pub fn raw_outputs(&self) -> impl Iterator<Item = &Step<Y>> {
        self.evaluations.iter().flat_map(|e| e.outputs.iter())
    }

    /// Returns the total number of raw output steps (including refinements).
    pub fn total_raw_outputs(&self) -> usize {
        self.evaluations.iter().map(|e| e.outputs.len()).sum()
    }

    /// Returns all raw emitted output steps as a flat vector.
    ///
    /// Most users should prefer [`verdicts()`](Self::verdicts) instead.
    pub fn all_raw_outputs(&self) -> Vec<Step<Y>>
    where
        Y: Clone,
    {
        self.evaluations
            .iter()
            .flat_map(|e| e.outputs.clone())
            .collect()
    }

    /// Provides read-only access to the per-sync-step evaluation results.
    ///
    /// Each [`SyncStepResult`] pairs a synchronized input step with
    /// the output(s) produced by the formula for that step.
    /// This is useful for advanced diagnostics or when building custom
    /// output formats (e.g., the Python bindings' `to_dict()`).
    pub fn sync_evaluations(&self) -> &[SyncStepResult<T, Y>] {
        &self.evaluations
    }
}

// ── IntoIterator implementations ────────────────────────────────────

/// Iterating over `&MonitorOutput` yields references to the raw output steps
/// (equivalent to calling [`raw_outputs()`](MonitorOutput::raw_outputs)).
impl<'a, T, Y> IntoIterator for &'a MonitorOutput<T, Y> {
    type Item = &'a Step<Y>;
    type IntoIter = std::iter::FlatMap<
        std::slice::Iter<'a, SyncStepResult<T, Y>>,
        std::slice::Iter<'a, Step<Y>>,
        fn(&'a SyncStepResult<T, Y>) -> std::slice::Iter<'a, Step<Y>>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.evaluations
            .iter()
            .flat_map(|e| e.outputs.iter() as std::slice::Iter<'a, Step<Y>>)
    }
}

/// Iterating over an owned `MonitorOutput` yields owned output steps
/// (consuming the output).
impl<T, Y> IntoIterator for MonitorOutput<T, Y> {
    type Item = Step<Y>;
    type IntoIter = std::iter::FlatMap<
        std::vec::IntoIter<SyncStepResult<T, Y>>,
        std::vec::IntoIter<Step<Y>>,
        fn(SyncStepResult<T, Y>) -> std::vec::IntoIter<Step<Y>>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.evaluations
            .into_iter()
            .flat_map(|e| e.outputs.into_iter())
    }
}

/// The final monitor struct that handles the input stream.
/// We do not constrain Y on the struct definition to allow flexible builder patterns,
/// though valid monitors will always have Y: RobustnessSemantics.
pub struct StlMonitor<T: Clone + Interpolatable, Y> {
    root_operator: Box<dyn StlOperatorTrait<T, Output = Y>>,
    synchronizer: Synchronizer<T>,
    variables: Variables,
    algorithm: Algorithm,
    semantics: Semantics,
}

/// Entry point for the builder.
/// This impl block enables `StlMonitor::builder()` to return a builder with default T=f64 and Y=f64.
impl StlMonitor<f64, f64> {
    /// Creates a builder with default input/output types (`T = f64`, `Y = f64`).
    ///
    /// Use [`StlMonitorBuilder::semantics`] to switch the output type.
    pub fn builder() -> StlMonitorBuilder<f64, f64> {
        StlMonitorBuilder {
            formula: None,
            algorithm: Algorithm::default(),
            semantics: Semantics::DelayedQuantitative, // Default, but will be overwritten if semantics() is called
            synchronization_strategy: SynchronizationStrategy::default(),
            variables: Variables::new(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T: Clone + Interpolatable, Y> StlMonitor<T, Y> {
    /// Processes a single input step through the monitor's evaluation pipeline.
    ///
    /// This is the core evaluation logic shared by both single and batch updates.
    /// It synchronizes the input, evaluates all pending synchronized steps, and
    /// collects the results.
    fn process_step(&mut self, step: &Step<T>) -> MonitorOutput<T, Y>
    where
        Y: RobustnessSemantics + Debug,
    {
        self.synchronizer.evaluate(step.clone());

        let evaluations = std::iter::from_fn(|| self.synchronizer.pending.pop_front())
            .map(|sync_step| {
                let op_res = self.root_operator.update(&sync_step);
                SyncStepResult::new(sync_step, op_res)
            })
            .collect();

        MonitorOutput::new(step, evaluations)
    }

    /// Updates the monitor with a single input step and returns the evaluation output.
    ///
    /// This method processes the input through synchronization and evaluates all
    /// pending synchronized steps against the STL formula.
    ///
    /// # Arguments
    ///
    /// * `step` - The input step containing signal name, value, and timestamp.
    ///
    /// # Returns
    ///
    /// A [`MonitorOutput`] containing the input metadata and all evaluation results
    /// produced by this update.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let step = Step::new("temperature", 25.0, Duration::from_secs(1));
    /// let output = monitor.update(&step);
    /// for verdict in output.verdicts() {
    ///     println!("t={:?}: {:?}", verdict.timestamp, verdict.value);
    /// }
    /// // or simply:
    /// for verdict in &output {
    ///     println!("t={:?}: {:?}", verdict.timestamp, verdict.value);
    /// }
    /// ```
    pub fn update(&mut self, step: &Step<T>) -> MonitorOutput<T, Y>
    where
        Y: RobustnessSemantics + Debug,
    {
        self.process_step(step)
    }

    /// Updates the monitor with multiple input steps organized by signal name.
    ///
    /// This method processes each step sequentially through the monitor and
    /// aggregates all evaluations into a single [`MonitorOutput`]. This is useful
    /// when you have batched data from multiple signals that need to be evaluated
    /// together and you want a unified result.
    ///
    /// # Arguments
    ///
    /// * `steps` - A map from signal names to vectors of steps for that signal.
    ///
    /// # Returns
    ///
    /// A single [`MonitorOutput`] containing all evaluation results from processing
    /// the batch. The input metadata reflects the last step processed.
    ///
    /// # Panics
    ///
    /// Panics if `steps` is empty (no steps to process).
    ///
    /// # Note
    ///
    /// Steps are processed in chronological order to optimize performance for Incremental algorithms.
    /// If another ordering is required, consider updating steps individually.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut steps = HashMap::new();
    /// steps.insert("temperature", vec![
    ///     Step::new("temperature", 25.0, Duration::from_secs(1)),
    ///     Step::new("temperature", 26.0, Duration::from_secs(2)),
    /// ]);
    /// steps.insert("pressure", vec![
    ///     Step::new("pressure", 101.3, Duration::from_secs(1)),
    /// ]);
    /// let output = monitor.update_batch(&steps);
    /// println!("{}", output); // Display all finalized verdicts
    /// ```
    pub fn update_batch(
        &mut self,
        steps: &std::collections::HashMap<&'static str, Vec<Step<T>>>,
    ) -> MonitorOutput<T, Y>
    where
        Y: RobustnessSemantics + Debug,
    {
        let mut all_steps: Vec<_> = steps
            .values()
            .flat_map(|step_list| step_list.iter())
            .collect();

        all_steps.sort_by_key(|step| step.timestamp);

        assert!(
            !all_steps.is_empty(),
            "update_batch requires at least one step"
        );

        let mut all_evaluations = Vec::new();
        let first_step = all_steps[0];
        let mut last_step = first_step;

        for step in all_steps {
            let output = self.process_step(step);
            all_evaluations.extend(output.evaluations);
            last_step = step;
        }

        MonitorOutput {
            input_signal: last_step.signal,
            input_timestamp: last_step.timestamp,
            input_value: last_step.value,
            evaluations: all_evaluations,
        }
    }

    /// Returns the STL specification as a string representation.
    pub fn specification(&self) -> String {
        self.root_operator.to_string()
    }

    /// Returns the algorithm used by this monitor (Naive or Incremental).
    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    /// Returns the semantics used by this monitor.
    pub fn semantics(&self) -> Semantics {
        self.semantics
    }

    /// Returns the synchronization strategy used by this monitor.
    pub fn synchronization_strategy(&self) -> SynchronizationStrategy {
        self.synchronizer.strategy()
    }

    /// Returns the signal identifiers used in the formula.
    pub fn signal_identifiers(&mut self) -> std::collections::HashSet<&'static str> {
        self.root_operator.get_signal_identifiers()
    }

    /// Returns the maximum lookahead required by the formula (temporal depth).
    pub fn temporal_depth(&self) -> Duration {
        self.root_operator.get_max_lookahead()
    }

    /// Returns a clone of the variables context used by this monitor.
    /// This allows reading and updating variable values at runtime.
    pub fn variables(&self) -> Variables {
        self.variables.clone()
    }
}

impl<T: Clone + Interpolatable, Y> Display for StlMonitor<T, Y> {
    /// Formats monitor configuration details for debugging and inspection.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "STL Monitor Configuration:")?;
        writeln!(f, "  Specification: {}", self.root_operator)?;
        writeln!(f, "  Algorithm: {:?}", self.algorithm)?;
        writeln!(f, "  Semantics: {:?}", self.semantics)?;
        writeln!(f, "  Synchronization: {:?}", self.synchronizer.strategy())?;
        writeln!(
            f,
            "  Temporal Depth: {:?}",
            self.root_operator.get_max_lookahead()
        )?;

        if !self.variables.is_empty() {
            writeln!(f, "  Variables:")?;
            for (name, value) in self.variables.iter() {
                writeln!(f, "    ${} = {}", name, value)?;
            }
        }

        Ok(())
    }
}

use crate::stl::core::Variables;

/// The Builder pattern struct for StlMonitor.
pub struct StlMonitorBuilder<T, Y> {
    formula: Option<FormulaDefinition>,
    algorithm: Algorithm,
    /// We store the enum value for logic, and use Y for type safety
    semantics: Semantics,
    synchronization_strategy: SynchronizationStrategy,
    variables: Variables,
    _phantom: std::marker::PhantomData<(T, Y)>,
}

impl<T, Y> StlMonitorBuilder<T, Y> {
    /// Sets the formula definition to be monitored.
    pub fn formula(mut self, formula: FormulaDefinition) -> Self {
        self.formula = Some(formula);
        self
    }

    /// Configures the monitoring algorithm (Naive or Incremental).
    pub fn algorithm(mut self, algorithm: Algorithm) -> Self {
        self.algorithm = algorithm;
        self
    }

    /// Configures the synchronization strategy for signal synchronization.
    pub fn synchronization_strategy(mut self, strategy: SynchronizationStrategy) -> Self {
        self.synchronization_strategy = strategy;
        self
    }

    /// Sets the variables context for formulas that use variable thresholds.
    ///
    /// Variables can be updated at runtime via the returned context.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let vars = Variables::new();
    /// vars.set("threshold", 5.0);
    /// let monitor = StlMonitor::builder()
    ///     .formula(parse_stl("x > $threshold").unwrap())
    ///     .variables(vars.clone())
    ///     .build()
    ///     .unwrap();
    /// // Later: vars.set("threshold", 10.0);
    /// ```
    pub fn variables(mut self, vars: Variables) -> Self {
        self.variables = vars;
        self
    }

    /// Applies the semantics, switching the Builder's generic type `Y` to match the semantics.
    /// This allows inference of the output type (bool, f64, RobustnessInterval).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let monitor = StlMonitor::builder()
    ///     .formula(formula)
    ///     .semantics(DelayedQualitative) // Y becomes bool
    ///     .build()?;
    /// ```
    pub fn semantics<S: semantic_markers::SemanticType>(
        self,
        _marker: S,
    ) -> StlMonitorBuilder<T, S::Output> {
        StlMonitorBuilder {
            formula: self.formula,
            algorithm: self.algorithm,
            semantics: S::as_enum(),
            synchronization_strategy: self.synchronization_strategy,
            variables: self.variables,
            _phantom: std::marker::PhantomData,
        }
    }
}

// The build method is only available when Y implements RobustnessSemantics.
// This prevents building when Y is still `Unset`.
impl<T, Y> StlMonitorBuilder<T, Y>
where
    T: Into<f64> + Copy + Interpolatable + 'static,
    Y: RobustnessSemantics + Copy + 'static + std::fmt::Debug,
{
    /// Initializes the operator by collecting signal identifiers and preparing internal state.
    /// This is called automatically during the build process for incremental operators.
    fn initialize_operator(
        &self,
        mut operator: Box<dyn StlOperatorAndSignalIdentifier<T, Y>>,
    ) -> Box<dyn StlOperatorAndSignalIdentifier<T, Y>> {
        operator.get_signal_identifiers();
        operator
    }

    /// Builds a fully configured [`StlMonitor`].
    ///
    /// Returns an error if no formula was provided or when an unsupported
    /// algorithm/semantics combination is requested.
    pub fn build(self) -> Result<StlMonitor<T, Y>, &'static str> {
        let mut formula_def = self
            .formula
            .clone()
            .ok_or("Formula definition is required")?;

        let root_operator = match (self.algorithm, self.semantics) {
            (Algorithm::Incremental, _) => {
                let operator = build_incremental_operator::<T, Y>(
                    formula_def.clone(),
                    self.semantics,
                    self.variables.clone(),
                );
                self.initialize_operator(operator)
            }
            (Algorithm::Naive, Semantics::DelayedQualitative | Semantics::DelayedQuantitative) => {
                self.build_naive_operator(formula_def.clone(), self.semantics)
            }
            (Algorithm::Naive, Semantics::EagerQualitative | Semantics::RobustnessInterval) => {
                return Err("Naive algorithm does not support RoSI/Eaver evaluation");
            }
        };

        let synchronizer = if formula_def.get_signal_identifiers().len() <= 1 {
            Synchronizer::new(SynchronizationStrategy::None)
        } else {
            Synchronizer::new(self.synchronization_strategy)
        };

        Ok(StlMonitor {
            root_operator,
            synchronizer,
            variables: self.variables.clone(),
            algorithm: self.algorithm,
            semantics: self.semantics,
        })
    }

    fn build_naive_operator(
        &self,
        formula: FormulaDefinition,
        _semantics: Semantics,
    ) -> Box<dyn StlOperatorTrait<T, Output = Y>>
    where
        T: Into<f64> + Copy + 'static,
        Y: RobustnessSemantics + 'static,
    {
        let formula_enum = build_naive_formula(formula);

        Box::new(StlFormula::<T, RingBuffer<T>, Y>::new(
            formula_enum,
            RingBuffer::new(),
        ))
    }
}

/// Recursively builds the lightweight StlOperator enum from the FormulaDefinition.
fn build_naive_formula(formula: FormulaDefinition) -> StlOperator {
    match formula {
        FormulaDefinition::GreaterThan(s, c) => StlOperator::GreaterThan(s, c),
        FormulaDefinition::LessThan(s, c) => StlOperator::LessThan(s, c),
        FormulaDefinition::GreaterThanVar(_, _) | FormulaDefinition::LessThanVar(_, _) => {
            panic!(
                "Variable predicates are not supported in the naive algorithm. Use Algorithm::Incremental instead."
            )
        }
        FormulaDefinition::True => StlOperator::True,
        FormulaDefinition::False => StlOperator::False,
        FormulaDefinition::And(l, r) => StlOperator::And(
            Box::new(build_naive_formula(*l)),
            Box::new(build_naive_formula(*r)),
        ),
        FormulaDefinition::Or(l, r) => StlOperator::Or(
            Box::new(build_naive_formula(*l)),
            Box::new(build_naive_formula(*r)),
        ),
        FormulaDefinition::Not(op) => StlOperator::Not(Box::new(build_naive_formula(*op))),
        FormulaDefinition::Implies(l, r) => StlOperator::Implies(
            Box::new(build_naive_formula(*l)),
            Box::new(build_naive_formula(*r)),
        ),
        FormulaDefinition::Eventually(i, op) => {
            StlOperator::Eventually(i, Box::new(build_naive_formula(*op)))
        }
        FormulaDefinition::Globally(i, op) => {
            StlOperator::Globally(i, Box::new(build_naive_formula(*op)))
        }
        FormulaDefinition::Until(i, l, r) => StlOperator::Until(
            i,
            Box::new(build_naive_formula(*l)),
            Box::new(build_naive_formula(*r)),
        ),
    }
}

fn build_incremental_operator<T, Y>(
    formula: FormulaDefinition,
    semantics: Semantics,
    variables: Variables,
) -> Box<dyn StlOperatorAndSignalIdentifier<T, Y>>
where
    T: Into<f64> + Copy + 'static,
    Y: RobustnessSemantics + Copy + 'static + std::fmt::Debug,
{
    // Determine configuration flags
    let is_eager = matches!(semantics, Semantics::EagerQualitative);
    let is_rosi = matches!(semantics, Semantics::RobustnessInterval);

    // We use `$( $arg:expr ),*` to capture arguments as a list.
    // We explicitly define <T, RingBuffer<Option<Y>>, Y...> to allow passing 'None' for caches.
    macro_rules! dispatch_operator {
        ($OpType:ident, $( $arg:expr ),* ) => {
            match (is_eager, is_rosi) {
                (true, true) => Box::new($OpType::<T, RingBuffer<Y>, Y, true, true>::new( $( $arg ),* )),
                (true, false) => Box::new($OpType::<T, RingBuffer<Y>, Y, true, false>::new( $( $arg ),* )),
                (false, true) => Box::new($OpType::<T, RingBuffer<Y>, Y, false, true>::new( $( $arg ),* )),
                (false, false) => Box::new($OpType::<T, RingBuffer<Y>, Y, false, false>::new( $( $arg ),* )),
            }
        };
    }

    match formula {
        FormulaDefinition::GreaterThan(s, c) => Box::new(Atomic::new_greater_than(s, c)),
        FormulaDefinition::LessThan(s, c) => Box::new(Atomic::new_less_than(s, c)),
        FormulaDefinition::GreaterThanVar(s, var) => {
            Box::new(Atomic::new_greater_than_var(s, var, variables.clone()))
        }
        FormulaDefinition::LessThanVar(s, var) => {
            Box::new(Atomic::new_less_than_var(s, var, variables.clone()))
        }
        FormulaDefinition::True => Box::new(Atomic::new_true()),
        FormulaDefinition::False => Box::new(Atomic::new_false()),

        FormulaDefinition::Not(op) => {
            let child = build_incremental_operator(*op, semantics, variables);
            Box::new(Not::new(child))
        }

        FormulaDefinition::And(l, r) => {
            let left = build_incremental_operator(*l, semantics, variables.clone());
            let right = build_incremental_operator(*r, semantics, variables);
            dispatch_operator!(And, left, right, None, None)
        }

        FormulaDefinition::Or(l, r) => {
            let left = build_incremental_operator(*l, semantics, variables.clone());
            let right = build_incremental_operator(*r, semantics, variables);
            dispatch_operator!(Or, left, right, None, None)
        }

        FormulaDefinition::Implies(l, r) => {
            let not_left = Box::new(Not::new(build_incremental_operator(
                *l,
                semantics,
                variables.clone(),
            )));
            let right = build_incremental_operator(*r, semantics, variables);
            dispatch_operator!(Or, not_left, right, None, None)
        }

        FormulaDefinition::Eventually(i, op) => {
            let child = build_incremental_operator(*op, semantics, variables);
            dispatch_operator!(Eventually, i, child, None, None)
        }

        FormulaDefinition::Globally(i, op) => {
            let child = build_incremental_operator(*op, semantics, variables);
            dispatch_operator!(Globally, i, child, None, None)
        }

        FormulaDefinition::Until(i, l, r) => {
            let left = build_incremental_operator(*l, semantics, variables.clone());
            let right = build_incremental_operator(*r, semantics, variables);
            dispatch_operator!(Until, i, left, right, None, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stl;
    use crate::stl::core::TimeInterval;
    use crate::stl::monitor::{Algorithm, StlMonitor};
    use std::time::Duration;

    #[test]
    fn test_builder_type_inference() {
        let formula = FormulaDefinition::GreaterThan("x", 5.0);

        // This is the syntax you requested:
        // T is defaulted to f64 by StlMonitor::builder()
        // Y is inferred as `bool` because of DelayedQualitative
        let mut monitor = StlMonitor::builder()
            .formula(formula)
            .semantics(DelayedQualitative) // Use the marker struct
            .algorithm(Algorithm::Incremental)
            .build()
            .unwrap();

        // The compiler knows this is f64 input, bool output
        let step = Step::new("x", 10.0, Duration::from_secs(1));
        let output = monitor.update(&step);

        // We can assert types in the test
        assert_eq!(output.input_value, 10.0); // T=f64
        // output.latest_verdict_at(...) returns Option<&Step<Option<bool>>>
    }

    #[test]
    fn test_build_1() {
        let formula = FormulaDefinition::And(
            Box::new(FormulaDefinition::GreaterThan("x", 5.0)),
            Box::new(FormulaDefinition::Eventually(
                TimeInterval {
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(10),
                },
                Box::new(FormulaDefinition::LessThan("y", 10.0)),
            )),
        );

        // Usage with marker struct implies Y = f64
        let mut monitor = StlMonitor::builder()
            .formula(formula.clone())
            .algorithm(Algorithm::Incremental)
            .semantics(DelayedQuantitative)
            .build()
            .unwrap();

        let mut monitor_naive = StlMonitor::builder()
            .formula(formula)
            .algorithm(Algorithm::Naive)
            .semantics(DelayedQuantitative)
            .build()
            .unwrap();

        let spec = monitor.specification();
        let spec_naive = monitor_naive.specification();

        let naive_ids = monitor_naive.signal_identifiers();
        let inc_ids = monitor.signal_identifiers();
        assert_eq!(naive_ids, inc_ids);
        assert!(naive_ids.contains("x"));
        assert!(naive_ids.contains("y"));
        assert!(inc_ids.contains("x"));
        assert!(inc_ids.contains("y"));
        assert_eq!(spec, spec_naive);
    }

    #[test]
    fn test_monitor_with_variables() {
        use crate::stl::parse_stl;

        // Parse a formula with a variable threshold
        let formula = parse_stl("x > $threshold").unwrap();

        // Create variables and set the threshold
        let variables = Variables::new();
        variables.set("threshold", 5.0);

        // Build monitor with variables
        let mut monitor = StlMonitor::builder()
            .formula(formula)
            .semantics(DelayedQualitative)
            .algorithm(Algorithm::Incremental)
            .variables(variables.clone())
            .build()
            .unwrap();

        // Test with value above threshold
        let step = Step::new("x", 10.0, Duration::from_secs(1));
        let output = monitor.update(&step);
        let verdicts = output.verdicts();
        assert_eq!(verdicts.len(), 1);
        assert!(verdicts[0].value);

        // Update the threshold
        variables.set("threshold", 15.0);

        // Now the same value should be below threshold
        let step2 = Step::new("x", 10.0, Duration::from_secs(2));
        let output2 = monitor.update(&step2);
        let verdicts2 = output2.verdicts();
        assert_eq!(verdicts2.len(), 1);
        assert!(!verdicts2[0].value);
    }

    #[test]
    fn test_monitor_with_variables_robustness() {
        use crate::stl::parse_stl;

        // Parse a formula with a variable threshold
        let formula = parse_stl("x > $threshold").unwrap();

        // Create variables and set the threshold
        let variables = Variables::new();
        variables.set("threshold", 5.0);

        // Build monitor with robustness semantics
        let mut monitor: StlMonitor<f64, f64> = StlMonitor::builder()
            .formula(formula)
            .semantics(DelayedQuantitative)
            .algorithm(Algorithm::Incremental)
            .variables(variables.clone())
            .build()
            .unwrap();

        // Test with value 10, threshold 5: robustness = 10 - 5 = 5
        let step = Step::new("x", 10.0, Duration::from_secs(1));
        let output = monitor.update(&step);
        let verdicts = output.verdicts();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].value, 5.0);

        // Update the threshold to 15
        variables.set("threshold", 15.0);

        // Now robustness = 10 - 15 = -5
        let step2 = Step::new("x", 10.0, Duration::from_secs(2));
        let output2 = monitor.update(&step2);
        let verdicts2 = output2.verdicts();
        assert_eq!(verdicts2.len(), 1);
        assert_eq!(verdicts2[0].value, -5.0);
    }

    #[test]
    fn test_monitor_get_variables() {
        use crate::stl::parse_stl;

        let formula = parse_stl("x > $A && y < $B").unwrap();
        let variables = Variables::new();
        variables.set("A", 1.0);
        variables.set("B", 2.0);

        let monitor: StlMonitor<f64, bool> = StlMonitor::builder()
            .formula(formula)
            .semantics(DelayedQualitative)
            .algorithm(Algorithm::Incremental)
            .variables(variables)
            .build()
            .unwrap();

        // Get variables back from monitor
        let vars = monitor.variables();
        assert_eq!(vars.get("A"), Some(1.0));
        assert_eq!(vars.get("B"), Some(2.0));
    }

    #[test]
    #[should_panic(expected = "Variable predicates are not supported in the naive algorithm")]
    fn test_variables_not_supported_in_naive() {
        use crate::stl::parse_stl;

        let formula = parse_stl("x > $threshold").unwrap();
        let variables = Variables::new();
        variables.set("threshold", 5.0);

        // This should panic because naive algorithm doesn't support variables
        let _monitor: StlMonitor<f64, bool> = StlMonitor::builder()
            .formula(formula)
            .semantics(DelayedQualitative)
            .algorithm(Algorithm::Naive)
            .variables(variables)
            .build()
            .unwrap();
    }

    #[test]
    fn test_batch_update() {
        // Test batch update with a simple atomic predicate
        let formula = stl!(G[0,2] (x > 10.0) && F[0,3] (y < 20.0));

        let mut monitor = StlMonitor::builder()
            .formula(formula)
            .semantics(Rosi)
            .algorithm(Algorithm::Incremental)
            .build()
            .unwrap();

        let mut steps = std::collections::HashMap::new();
        steps.insert(
            "x",
            vec![
                Step::new("x", 5.0, Duration::from_secs(0)),
                Step::new("x", 15.0, Duration::from_secs(2)),
                Step::new("x", 8.0, Duration::from_secs(4)),
            ],
        );
        steps.insert(
            "y",
            vec![
                Step::new("y", 25.0, Duration::from_secs(0)),
                Step::new("y", 15.0, Duration::from_secs(3)),
                Step::new("y", 30.0, Duration::from_secs(5)),
            ],
        );

        let output = monitor.update_batch(&steps);
        let verdicts = output.verdicts();

        assert_eq!(verdicts.len(), 4);
        assert!(verdicts[0].timestamp == Duration::from_secs(0));
        assert!(verdicts[0].value.0 == verdicts[0].value.1); // final
        assert!(verdicts[1].timestamp == Duration::from_secs(2));
        assert!(verdicts[1].value.0 == verdicts[1].value.1); // final
        assert!(verdicts[2].timestamp == Duration::from_secs(3));
        assert!(verdicts[2].value.0 != verdicts[2].value.1); // non-final
        assert!(verdicts[3].timestamp == Duration::from_secs(4));
        assert!(verdicts[3].value.0 != verdicts[3].value.1); // non-final
    }

    #[test]
    #[should_panic(expected = "update_batch requires at least one step")]
    fn test_batch_update_empty() {
        let formula = stl!(G[0,2] (x > 10.0));
        let mut monitor = StlMonitor::builder().formula(formula).build().unwrap();
        let steps: std::collections::HashMap<&'static str, Vec<Step<f64>>> =
            std::collections::HashMap::new();
        monitor.update_batch(&steps);
    }

    #[test]
    fn test_monitor_display_and_getters() {
        use crate::stl::parse_stl;

        // Create a formula with variables
        let formula = parse_stl("G[0,5] (x > $threshold) && F[1,3] (y < 20.0)").unwrap();
        let variables = Variables::new();
        variables.set("threshold", 10.0);

        let mut monitor = StlMonitor::builder()
            .formula(formula)
            .semantics(DelayedQuantitative)
            .algorithm(Algorithm::Incremental)
            .synchronization_strategy(SynchronizationStrategy::Linear)
            .variables(variables)
            .build()
            .unwrap();

        // Test getters
        assert_eq!(monitor.algorithm(), Algorithm::Incremental);
        assert_eq!(monitor.semantics(), Semantics::DelayedQuantitative);
        assert_eq!(
            monitor.synchronization_strategy(),
            SynchronizationStrategy::Linear
        );
        assert_eq!(monitor.temporal_depth(), Duration::from_secs(5));

        // Test specification getter
        let spec = monitor.specification();
        assert!(spec.contains("x"));
        assert!(spec.contains("y"));

        // Test variables getter
        let vars = monitor.variables();
        assert_eq!(vars.get("threshold"), Some(10.0));

        // Test signal identifiers
        let signals = monitor.signal_identifiers();
        assert!(signals.contains("x"));
        assert!(signals.contains("y"));
        assert_eq!(signals.len(), 2);

        // Test Display implementation
        let display_output = format!("{}", monitor);
        assert!(display_output.contains("STL Monitor Configuration"));
        assert!(display_output.contains("Algorithm: Incremental"));
        assert!(display_output.contains("Semantics: DelayedQuantitative"));
        assert!(display_output.contains("Synchronization: Linear"));
        assert!(display_output.contains("Temporal Depth: 5s"));
        assert!(display_output.contains("Variables:"));
        assert!(display_output.contains("$threshold = 10"));
    }

    #[test]
    fn test_monitor_display_no_variables() {
        let formula = FormulaDefinition::GreaterThan("x", 5.0);

        let monitor = StlMonitor::builder()
            .formula(formula)
            .semantics(DelayedQualitative)
            .algorithm(Algorithm::Incremental)
            .build()
            .unwrap();

        // Test Display implementation without variables
        let display_output = format!("{}", monitor);
        assert!(display_output.contains("STL Monitor Configuration"));
        assert!(display_output.contains("Algorithm: Incremental"));
        assert!(display_output.contains("Semantics: DelayedQualitative"));
        assert!(!display_output.contains("Variables:")); // Should not appear
    }

    mod monitor_output_tests {

        use super::*;

        #[test]
        fn test_monitor_output_display() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step1 = Step::new("output", true, Duration::from_secs(1));
            let output_step2 = Step::new("output", false, Duration::from_secs(2));

            let sync_result =
                SyncStepResult::new(sync_step.clone(), vec![output_step1, output_step2]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };
            let monitor_output_empty: MonitorOutput<f64, bool> = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![],
            };

            let display_str = format!("{}", monitor_output);
            assert!(display_str.contains("t=1s: true"));
            assert!(display_str.contains("t=2s: false"));

            let display_empty_str = format!("{}", monitor_output_empty);
            assert!(display_empty_str.contains("No verdicts available"));
        }

        #[test]
        fn test_monitor_has_outputs() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step = Step::new("output", true, Duration::from_secs(1));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            assert!(monitor_output.has_verdicts());
        }

        #[test]
        fn test_monitor_total_outputs() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step1 = Step::new("output", true, Duration::from_secs(1));
            let output_step2 = Step::new("output", false, Duration::from_secs(2));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step1, output_step2]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            assert_eq!(monitor_output.total_raw_outputs(), 2);
        }

        #[test]
        fn test_monitor_is_empty() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let sync_result: SyncStepResult<f64, bool> = SyncStepResult::new(sync_step, vec![]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            assert!(!monitor_output.is_pending());

            // exhaust the iter
            let mut iter = monitor_output.raw_outputs();
            assert!(iter.next().is_none());
        }

        #[test]
        fn test_monitor_latest_verdict_at() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step1 = Step::new("output", true, Duration::from_secs(1));
            let output_step2 = Step::new("output", false, Duration::from_secs(2));
            let output_step2_ = Step::new("output", true, Duration::from_secs(2));
            let sync_result =
                SyncStepResult::new(sync_step, vec![output_step1, output_step2, output_step2_]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            let verdict = monitor_output.verdict_at(Duration::from_secs(1));
            assert!(verdict.is_some());
            assert!(*verdict.unwrap());

            let verdict2 = monitor_output.verdict_at(Duration::from_secs(2));
            assert!(verdict2.is_some());
            assert!(*verdict2.unwrap()); // should return the latest verdict at t=2, which is true

            let verdict3 = monitor_output.verdict_at(Duration::from_secs(3));
            assert!(verdict3.is_none());
        }

        #[test]
        fn test_into_verdicts() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step1 = Step::new("output", true, Duration::from_secs(1));
            let output_step2 = Step::new("output", false, Duration::from_secs(2));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step1, output_step2]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            let verdicts = monitor_output.into_verdicts();
            assert_eq!(verdicts.len(), 2);
            assert!(verdicts[0].value);
            assert!(!verdicts[1].value);
        }

        #[test]
        fn test_latest_verdict() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step1 = Step::new("output", true, Duration::from_secs(1));
            let output_step2 = Step::new("output", false, Duration::from_secs(2));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step1, output_step2]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            let latest = monitor_output.latest_verdict().unwrap();
            assert!(!latest.value);
            assert_eq!(latest.timestamp, Duration::from_secs(2));
        }

        #[test]
        fn test_latest_verdict_empty() {
            let monitor_output: MonitorOutput<f64, bool> = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![],
            };
            assert!(monitor_output.latest_verdict().is_none());
        }

        #[test]
        fn test_sync_evaluations() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step = Step::new("output", true, Duration::from_secs(1));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            let evals = monitor_output.sync_evaluations();
            assert_eq!(evals.len(), 1);
            assert!(evals[0].has_outputs());
        }

        #[test]
        fn test_into_iterator_ref() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step1 = Step::new("output", true, Duration::from_secs(1));
            let output_step2 = Step::new("output", false, Duration::from_secs(2));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step1, output_step2]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            let steps: Vec<_> = (&monitor_output).into_iter().collect();
            assert_eq!(steps.len(), 2);
            assert!(steps[0].value);
            assert!(!steps[1].value);
        }

        #[test]
        fn test_into_iterator_owned() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step1 = Step::new("output", true, Duration::from_secs(1));
            let output_step2 = Step::new("output", false, Duration::from_secs(2));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step1, output_step2]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            let steps: Vec<Step<bool>> = monitor_output.into_iter().collect();
            assert_eq!(steps.len(), 2);
            assert!(steps[0].value);
            assert!(!steps[1].value);
        }

        #[test]
        fn test_is_pending_true() {
            let monitor_output: MonitorOutput<f64, bool> = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![],
            };
            assert!(monitor_output.is_pending());
        }

        #[test]
        fn test_for_loop_iteration() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step1 = Step::new("output", 5.0_f64, Duration::from_secs(1));
            let output_step2 = Step::new("output", -3.0_f64, Duration::from_secs(2));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step1, output_step2]);
            let monitor_output = MonitorOutput {
                input_signal: "x",
                input_timestamp: Duration::from_secs(1),
                input_value: 10.0,
                evaluations: vec![sync_result],
            };

            let mut count = 0;
            for step in &monitor_output {
                count += 1;
                assert_eq!(step.signal, "output");
            }
            assert_eq!(count, 2);
        }
    }

    mod syncstep_result_tests {
        use super::*;

        #[test]
        fn test_sync_step_has_outputs() {
            let sync_step = Step::new("x", 10.0, Duration::from_secs(1));
            let output_step = Step::new("output", true, Duration::from_secs(1));
            let sync_result = SyncStepResult::new(sync_step, vec![output_step]);

            assert!(sync_result.has_outputs());
        }
    }
}
