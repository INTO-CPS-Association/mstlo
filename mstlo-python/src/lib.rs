use mstlo::monitor::{
    Algorithm, DelayedQualitative, DelayedQuantitative, EagerQualitative, MonitorOutput, Rosi,
    StlMonitor,
};
use mstlo::parse_stl;
use mstlo::{FormulaDefinition, RobustnessInterval, TimeInterval, Variables};
use mstlo::{SignalInterpolation, Step, intern};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyList, PyTuple};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

#[pyfunction]
#[pyo3(name = "parse_formula")]
fn py_parse_formula(formula_str: &str) -> PyResult<Formula> {
    let formula = parse_stl(formula_str)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("{}", e)))?;
    Ok(Formula { inner: formula })
}
#[pyclass(name = "Formula", module = "mstlo_python.mstlo_python", from_py_object)]
#[derive(Clone)]
struct Formula {
    inner: FormulaDefinition,
}

#[pymethods]
impl Formula {
    // --- Atomic Propositions ---
    #[staticmethod]
    // #[pyo3(text_signature = "(signal, value)")]
    fn gt(signal: String, value: f64) -> Self {
        // Intern the name to get the `&'static str` mstlo stores: allocated
        // once per distinct name rather than on every call.
        Formula {
            inner: FormulaDefinition::GreaterThan(intern(&signal), value),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(signal, value)")]
    fn lt(signal: String, value: f64) -> Self {
        Formula {
            inner: FormulaDefinition::LessThan(intern(&signal), value),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "()")]
    fn true_() -> Self {
        Formula {
            inner: FormulaDefinition::True,
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "()")]
    fn false_() -> Self {
        Formula {
            inner: FormulaDefinition::False,
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(signal, variable)")]
    fn gt_var(signal: String, variable: String) -> Self {
        Formula {
            inner: FormulaDefinition::GreaterThanVar(intern(&signal), intern(&variable)),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(signal, variable)")]
    fn lt_var(signal: String, variable: String) -> Self {
        Formula {
            inner: FormulaDefinition::LessThanVar(intern(&signal), intern(&variable)),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(left, right)")]
    fn and_(left: &Formula, right: &Formula) -> Self {
        Formula {
            inner: FormulaDefinition::And(
                Box::new(left.inner.clone()),
                Box::new(right.inner.clone()),
            ),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(left, right)")]
    fn or_(left: &Formula, right: &Formula) -> Self {
        Formula {
            inner: FormulaDefinition::Or(
                Box::new(left.inner.clone()),
                Box::new(right.inner.clone()),
            ),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(child)")]
    fn not_(child: &Formula) -> Self {
        Formula {
            inner: FormulaDefinition::Not(Box::new(child.inner.clone())),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(left, right)")]
    fn implies(left: &Formula, right: &Formula) -> Self {
        Formula {
            inner: FormulaDefinition::Implies(
                Box::new(left.inner.clone()),
                Box::new(right.inner.clone()),
            ),
        }
    }

    // --- Temporal Logic ---

    #[staticmethod]
    #[pyo3(text_signature = "(start, end, child)")]
    fn always(start: f64, end: f64, child: &Formula) -> Self {
        let interval = TimeInterval {
            start: Duration::from_secs_f64(start),
            end: Duration::from_secs_f64(end),
        };
        Formula {
            inner: FormulaDefinition::Globally(interval, Box::new(child.inner.clone())),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(start, end, child)")]
    fn eventually(start: f64, end: f64, child: &Formula) -> Self {
        let interval = TimeInterval {
            start: Duration::from_secs_f64(start),
            end: Duration::from_secs_f64(end),
        };
        Formula {
            inner: FormulaDefinition::Eventually(interval, Box::new(child.inner.clone())),
        }
    }

    #[staticmethod]
    #[pyo3(text_signature = "(start, end, left, right)")]
    fn until(start: f64, end: f64, left: &Formula, right: &Formula) -> Self {
        let interval = TimeInterval {
            start: Duration::from_secs_f64(start),
            end: Duration::from_secs_f64(end),
        };
        Formula {
            inner: FormulaDefinition::Until(
                interval,
                Box::new(left.inner.clone()),
                Box::new(right.inner.clone()),
            ),
        }
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Formula({})", self.inner)
    }
}

// -----------------------------------------------------------------------------
// 3. MonitorOutput Wrapper
// -----------------------------------------------------------------------------
#[derive(Clone)]
enum InnerMonitorOutput {
    Bool(MonitorOutput<f64, bool>),
    Float(MonitorOutput<f64, f64>),
    Interval(MonitorOutput<f64, RobustnessInterval>),
}
#[pyclass(
    name = "MonitorOutput",
    module = "mstlo_python.mstlo_python",
    from_py_object
)]
#[derive(Clone)]
struct PyMonitorOutput {
    inner: InnerMonitorOutput,
}

#[pymethods]
impl PyMonitorOutput {
    fn to_dict(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| match &self.inner {
            InnerMonitorOutput::Bool(output) => convert_output_to_dict(py, output.clone(), |val| {
                PyBool::new(py, val).to_owned().into_any().unbind()
            }),
            InnerMonitorOutput::Float(output) => {
                convert_output_to_dict(py, output.clone(), |val| {
                    PyFloat::new(py, val).to_owned().into_any().unbind()
                })
            }
            InnerMonitorOutput::Interval(output) => {
                convert_output_to_dict(py, output.clone(), |val| {
                    PyTuple::new(py, [val.0, val.1])
                        .unwrap()
                        .into_any()
                        .unbind()
                })
            }
        })
    }

    /// Signal name of the last input step, or `None` for an empty batch.
    #[getter]
    fn input_signal(&self) -> Option<&'static str> {
        match &self.inner {
            InnerMonitorOutput::Bool(o) => o.input_signal(),
            InnerMonitorOutput::Float(o) => o.input_signal(),
            InnerMonitorOutput::Interval(o) => o.input_signal(),
        }
    }

    /// Timestamp of the last input step, or `None` for an empty batch.
    #[getter]
    fn input_timestamp(&self) -> Option<f64> {
        match &self.inner {
            InnerMonitorOutput::Bool(o) => o.input_timestamp(),
            InnerMonitorOutput::Float(o) => o.input_timestamp(),
            InnerMonitorOutput::Interval(o) => o.input_timestamp(),
        }
        .map(|timestamp| timestamp.as_secs_f64())
    }

    /// Value of the last input step, or `None` for an empty batch.
    #[getter]
    fn input_value(&self) -> Option<f64> {
        match &self.inner {
            InnerMonitorOutput::Bool(o) => o.input_value(),
            InnerMonitorOutput::Float(o) => o.input_value(),
            InnerMonitorOutput::Interval(o) => o.input_value(),
        }
        .copied()
    }

    fn has_verdicts(&self) -> bool {
        match &self.inner {
            InnerMonitorOutput::Bool(o) => o.has_verdicts(),
            InnerMonitorOutput::Float(o) => o.has_verdicts(),
            InnerMonitorOutput::Interval(o) => o.has_verdicts(),
        }
    }

    fn total_raw_outputs(&self) -> usize {
        match &self.inner {
            InnerMonitorOutput::Bool(o) => o.total_raw_outputs(),
            InnerMonitorOutput::Float(o) => o.total_raw_outputs(),
            InnerMonitorOutput::Interval(o) => o.total_raw_outputs(),
        }
    }

    fn is_pending(&self) -> bool {
        match &self.inner {
            InnerMonitorOutput::Bool(o) => o.is_pending(),
            InnerMonitorOutput::Float(o) => o.is_pending(),
            InnerMonitorOutput::Interval(o) => o.is_pending(),
        }
    }

    /// Deprecated: use has_verdicts() instead.
    fn has_outputs(&self) -> bool {
        self.has_verdicts()
    }

    /// Deprecated: use total_raw_outputs() instead.
    fn total_outputs(&self) -> usize {
        self.total_raw_outputs()
    }

    /// Deprecated: use is_pending() instead.
    fn is_empty(&self) -> bool {
        self.is_pending()
    }

    fn verdicts(&self) -> PyResult<Py<PyList>> {
        Python::attach(|py| {
            let list = match &self.inner {
                InnerMonitorOutput::Bool(o) => {
                    let verdicts = o.verdicts();
                    let items: Vec<_> = verdicts
                        .iter()
                        .map(|step| {
                            PyTuple::new(
                                py,
                                [
                                    step.timestamp
                                        .as_secs_f64()
                                        .into_pyobject(py)
                                        .unwrap()
                                        .into_any(),
                                    PyBool::new(py, step.value).to_owned().into_any(),
                                ],
                            )
                            .unwrap()
                        })
                        .collect();
                    PyList::new(py, items).unwrap()
                }
                InnerMonitorOutput::Float(o) => {
                    let verdicts = o.verdicts();
                    let items: Vec<_> = verdicts
                        .iter()
                        .map(|step| {
                            PyTuple::new(
                                py,
                                [
                                    step.timestamp
                                        .as_secs_f64()
                                        .into_pyobject(py)
                                        .unwrap()
                                        .into_any(),
                                    PyFloat::new(py, step.value).to_owned().into_any(),
                                ],
                            )
                            .unwrap()
                        })
                        .collect();
                    PyList::new(py, items).unwrap()
                }
                InnerMonitorOutput::Interval(o) => {
                    let verdicts = o.verdicts();
                    let items: Vec<_> = verdicts
                        .iter()
                        .map(|step| {
                            let interval = PyTuple::new(py, [step.value.0, step.value.1]).unwrap();
                            PyTuple::new(
                                py,
                                [
                                    step.timestamp
                                        .as_secs_f64()
                                        .into_pyobject(py)
                                        .unwrap()
                                        .into_any(),
                                    interval.into_any(),
                                ],
                            )
                            .unwrap()
                        })
                        .collect();
                    PyList::new(py, items).unwrap()
                }
            };
            Ok(list.unbind())
        })
    }

    /// Deprecated: use verdicts() instead.
    fn finalize(&self) -> PyResult<Py<PyList>> {
        self.verdicts()
    }

    fn __str__(&self) -> String {
        match &self.inner {
            InnerMonitorOutput::Bool(o) => format!("{}", o),
            InnerMonitorOutput::Float(o) => format!("{}", o),
            InnerMonitorOutput::Interval(o) => format!("{}", o),
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            InnerMonitorOutput::Bool(o) => format!("{:?}", o),
            InnerMonitorOutput::Float(o) => format!("{:?}", o),
            InnerMonitorOutput::Interval(o) => format!("{:?}", o),
        }
    }
}

// -----------------------------------------------------------------------------
// 3.5 Variables Wrapper
// -----------------------------------------------------------------------------
#[pyclass(
    name = "Variables",
    module = "mstlo_python.mstlo_python",
    unsendable,
    from_py_object
)]
#[derive(Clone)]
struct PyVariables {
    inner: Variables,
}

#[pymethods]
impl PyVariables {
    #[new]
    fn new() -> Self {
        PyVariables {
            inner: Variables::new(),
        }
    }

    fn set(&self, name: String, value: f64) {
        self.inner.set(&name, value);
    }

    fn get(&self, name: String) -> Option<f64> {
        self.inner.get(&name)
    }

    fn contains(&self, name: String) -> bool {
        self.inner.contains(&name)
    }

    fn names(&self) -> Vec<String> {
        self.inner.names().iter().map(|s| s.to_string()).collect()
    }

    fn remove(&self, name: String) -> Option<f64> {
        self.inner.remove(&name)
    }

    fn clear(&self) {
        self.inner.clear();
    }

    fn __str__(&self) -> String {
        let names = self.inner.names();
        if names.is_empty() {
            return "Variables({})".to_string();
        }
        let pairs: Vec<String> = names
            .iter()
            .map(|n| {
                let val = self
                    .inner
                    .get(n)
                    .map_or("None".to_string(), |v| v.to_string());
                format!("{}: {}", n, val)
            })
            .collect();
        format!("Variables({{{}}})", pairs.join(", "))
    }

    fn __repr__(&self) -> String {
        self.__str__()
    }
}

// -----------------------------------------------------------------------------
// 4. Monitor Wrapper
// -----------------------------------------------------------------------------
enum InnerMonitor {
    DelayedQualitative(StlMonitor<f64, bool>),
    EagerQualitative(StlMonitor<f64, bool>),
    Robustness(StlMonitor<f64, f64>),
    Rosi(StlMonitor<f64, RobustnessInterval>),
}

// Note: Monitor is marked unsendable because it contains Variables which uses Rc<RefCell>.
// Python's GIL ensures thread safety across Python threads.

#[pyclass(module = "mstlo_python.mstlo_python", unsendable)]
struct Monitor {
    inner: InnerMonitor,
    semantics: String,
    algorithm: String,
    signal_interpolation: String,
    variables: PyVariables,
    /// Cache mapping signal name → the interned `&'static str`, so the global
    /// interner is consulted at most once per unique signal name per monitor
    /// rather than on every update call.
    signal_name_cache: HashMap<String, &'static str>,
}

#[pymethods]
impl Monitor {
    #[new]
    #[pyo3(signature = (formula, semantics="DelayedQuantitative", algorithm="Incremental", synchronization=None, signal_interpolation=None, variables=None))]
    fn new(
        py: Python<'_>,
        formula: &Formula,
        semantics: &str,
        algorithm: &str,
        synchronization: Option<&str>,
        signal_interpolation: Option<&str>,
        variables: Option<&PyVariables>,
    ) -> PyResult<Self> {
        // Parse algorithm
        let algo = match algorithm {
            "Incremental" => Algorithm::Incremental,
            "Naive" => Algorithm::Naive,
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Invalid algorithm. Use 'Incremental' or 'Naive'",
                ));
            }
        };

        // `synchronization` asked how several signals are aligned onto a common timeline,
        // which turned out not to be a question with an answer. Each of its values is now
        // read as the interpolation of the same name, with "None" meaning "ZeroOrderHold".
        let from_synchronization = match synchronization {
            None => Option::None,
            Some(name) => {
                let selected = match name {
                    "ZeroOrderHold" | "None" => SignalInterpolation::ZeroOrderHold,
                    "Linear" => SignalInterpolation::Linear,
                    _ => {
                        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                            "Invalid synchronization. Use 'ZeroOrderHold', 'Linear', or 'None'",
                        ));
                    }
                };
                let message = std::ffi::CString::new(format!(
                    "synchronization='{name}' is deprecated; use signal_interpolation='{}' instead",
                    match selected {
                        SignalInterpolation::ZeroOrderHold => "ZeroOrderHold",
                        SignalInterpolation::Linear => "Linear",
                    }
                ))
                .map_err(|_| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid synchronization")
                })?;
                PyErr::warn(
                    py,
                    &py.get_type::<pyo3::exceptions::PyDeprecationWarning>(),
                    message.as_c_str(),
                    1,
                )?;
                Some(selected)
            }
        };

        // How a signal behaves *between its own samples*. Given explicitly it wins, so a
        // caller porting off `synchronization` can pass both during the transition.
        let interpolation = match signal_interpolation {
            None => from_synchronization.unwrap_or_default(),
            Some("ZeroOrderHold") => SignalInterpolation::ZeroOrderHold,
            Some("Linear") => SignalInterpolation::Linear,
            Some(_) => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Invalid signal_interpolation. Use 'ZeroOrderHold' or 'Linear'",
                ));
            }
        };
        let interpolation_name = match interpolation {
            SignalInterpolation::ZeroOrderHold => "ZeroOrderHold",
            SignalInterpolation::Linear => "Linear",
        };

        // Get or create variables
        let vars = variables.cloned().unwrap_or_else(PyVariables::new);

        // Build monitor based on semantics
        match semantics {
            "DelayedQualitative" => {
                let m = StlMonitor::builder()
                    .formula(formula.inner.clone())
                    .algorithm(algo)
                    .semantics(DelayedQualitative)
                    .signal_interpolation(interpolation)
                    .variables(vars.inner.clone())
                    .build()
                    .map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?;
                Ok(Monitor {
                    inner: InnerMonitor::DelayedQualitative(m),
                    semantics: semantics.to_string(),
                    algorithm: algorithm.to_string(),
                    signal_interpolation: interpolation_name.to_string(),
                    variables: vars,
                    signal_name_cache: HashMap::new(),
                })
            }
            "EagerQualitative" => {
                let m = StlMonitor::builder()
                    .formula(formula.inner.clone())
                    .algorithm(algo)
                    .semantics(EagerQualitative)
                    .signal_interpolation(interpolation)
                    .variables(vars.inner.clone())
                    .build()
                    .map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?;
                Ok(Monitor {
                    inner: InnerMonitor::EagerQualitative(m),
                    semantics: semantics.to_string(),
                    algorithm: algorithm.to_string(),
                    signal_interpolation: interpolation_name.to_string(),
                    variables: vars,
                    signal_name_cache: HashMap::new(),
                })
            }
            "DelayedQuantitative" => {
                let m = StlMonitor::builder()
                    .formula(formula.inner.clone())
                    .algorithm(algo)
                    .semantics(DelayedQuantitative)
                    .signal_interpolation(interpolation)
                    .variables(vars.inner.clone())
                    .build()
                    .map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?;
                Ok(Monitor {
                    inner: InnerMonitor::Robustness(m),
                    semantics: semantics.to_string(),
                    algorithm: algorithm.to_string(),
                    signal_interpolation: interpolation_name.to_string(),
                    variables: vars,
                    signal_name_cache: HashMap::new(),
                })
            }
            "Rosi" => {
                let m = StlMonitor::builder()
                    .formula(formula.inner.clone())
                    .algorithm(algo)
                    .semantics(Rosi)
                    .signal_interpolation(interpolation)
                    .variables(vars.inner.clone())
                    .build()
                    .map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?;
                Ok(Monitor {
                    inner: InnerMonitor::Rosi(m),
                    semantics: semantics.to_string(),
                    algorithm: algorithm.to_string(),
                    signal_interpolation: interpolation_name.to_string(),
                    variables: vars,
                    signal_name_cache: HashMap::new(),
                })
            }
            _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Invalid semantics. Use 'DelayedQualitative', 'EagerQualitative', 'DelayedQuantitative', or 'Rosi'",
            )),
        }
    }

    fn update(&mut self, signal: String, value: f64, timestamp: f64) -> PyMonitorOutput {
        let sig_ref = self.intern_signal_name(signal);
        let step = Step::new(sig_ref, value, Duration::from_secs_f64(timestamp));

        match &mut self.inner {
            InnerMonitor::Robustness(m) => {
                let output = m.update(&step);
                PyMonitorOutput {
                    inner: InnerMonitorOutput::Float(output),
                }
            }
            InnerMonitor::EagerQualitative(m) | InnerMonitor::DelayedQualitative(m) => {
                let output = m.update(&step);
                PyMonitorOutput {
                    inner: InnerMonitorOutput::Bool(output),
                }
            }
            InnerMonitor::Rosi(m) => {
                let output = m.update(&step);
                PyMonitorOutput {
                    inner: InnerMonitorOutput::Interval(output),
                }
            }
        }
    }

    fn get_signal_identifiers(&mut self) -> HashSet<&'static str> {
        match &mut self.inner {
            InnerMonitor::DelayedQualitative(m) => m.signal_identifiers(),
            InnerMonitor::EagerQualitative(m) => m.signal_identifiers(),
            InnerMonitor::Robustness(m) => m.signal_identifiers(),
            InnerMonitor::Rosi(m) => m.signal_identifiers(),
        }
    }

    fn get_variables(&self) -> PyVariables {
        self.variables.clone()
    }

    fn get_specification(&self) -> String {
        match &self.inner {
            InnerMonitor::DelayedQualitative(m) => m.specification(),
            InnerMonitor::EagerQualitative(m) => m.specification(),
            InnerMonitor::Robustness(m) => m.specification(),
            InnerMonitor::Rosi(m) => m.specification(),
        }
    }

    fn get_algorithm(&self) -> String {
        self.algorithm.clone()
    }

    fn get_semantics(&self) -> String {
        self.semantics.clone()
    }

    /// Deprecated alias for [`Monitor::get_signal_interpolation`].
    ///
    /// Reports the interpolation in force, so it answers "ZeroOrderHold" or "Linear" and
    /// never "None", whichever spelling the monitor was built with.
    fn get_synchronization_strategy(&self, py: Python<'_>) -> PyResult<String> {
        PyErr::warn(
            py,
            &py.get_type::<pyo3::exceptions::PyDeprecationWarning>(),
            c"get_synchronization_strategy() is deprecated; use get_signal_interpolation()",
            1,
        )?;
        Ok(self.signal_interpolation.clone())
    }

    fn get_signal_interpolation(&self) -> String {
        self.signal_interpolation.clone()
    }

    fn get_temporal_depth(&self) -> f64 {
        let duration = match &self.inner {
            InnerMonitor::DelayedQualitative(m) => m.temporal_depth(),
            InnerMonitor::EagerQualitative(m) => m.temporal_depth(),
            InnerMonitor::Robustness(m) => m.temporal_depth(),
            InnerMonitor::Rosi(m) => m.temporal_depth(),
        };
        duration.as_secs_f64()
    }

    fn update_batch(&mut self, steps: &Bound<'_, PyAny>) -> PyResult<PyMonitorOutput> {
        let rust_steps = self.collect_batch_steps(steps)?;

        match &mut self.inner {
            InnerMonitor::Robustness(m) => {
                let output = m.update_batch(&rust_steps);
                Ok(PyMonitorOutput {
                    inner: InnerMonitorOutput::Float(output),
                })
            }
            InnerMonitor::EagerQualitative(m) | InnerMonitor::DelayedQualitative(m) => {
                let output = m.update_batch(&rust_steps);
                Ok(PyMonitorOutput {
                    inner: InnerMonitorOutput::Bool(output),
                })
            }
            InnerMonitor::Rosi(m) => {
                let output = m.update_batch(&rust_steps);
                Ok(PyMonitorOutput {
                    inner: InnerMonitorOutput::Interval(output),
                })
            }
        }
    }

    /// Resets the monitor to its initial state, clearing all internal caches and
    /// evaluation buffers. The formula, semantics, algorithm, signal
    /// interpolation, and variables are preserved.
    ///
    /// Use this to reuse a monitor across multiple independent traces without
    /// rebuilding it from scratch.
    ///
    /// Example:
    ///     >>> monitor = Monitor(formula)
    ///     >>> for signal, value, ts in trace_1:
    ///     ...     monitor.update(signal, value, ts)
    ///     >>> monitor.reset()
    ///     >>> for signal, value, ts in trace_2:
    ///     ...     monitor.update(signal, value, ts)
    fn reset(&mut self) {
        match &mut self.inner {
            InnerMonitor::DelayedQualitative(m) => m.reset(),
            InnerMonitor::EagerQualitative(m) => m.reset(),
            InnerMonitor::Robustness(m) => m.reset(),
            InnerMonitor::Rosi(m) => m.reset(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Monitor(semantics='{}', algorithm='{}', signal_interpolation='{}')",
            self.semantics, self.algorithm, self.signal_interpolation
        )
    }

    fn __str__(&self) -> String {
        // Use the Rust Display implementation
        match &self.inner {
            InnerMonitor::DelayedQualitative(m) => format!("{}", m),
            InnerMonitor::EagerQualitative(m) => format!("{}", m),
            InnerMonitor::Robustness(m) => format!("{}", m),
            InnerMonitor::Rosi(m) => format!("{}", m),
        }
    }
}

impl Monitor {
    /// Converts either accepted batch form into a flat list of steps.
    ///
    /// Accepted forms:
    /// - signal-major: `{"x": [(value, timestamp), ..], ..}`
    /// - flat: `[("x", value, timestamp), ..]`
    fn collect_batch_steps(&mut self, steps: &Bound<'_, PyAny>) -> PyResult<Vec<Step<f64>>> {
        let mut rust_steps = Vec::new();

        if let Ok(mapping) = steps.cast::<PyDict>() {
            for (key, samples) in mapping.iter() {
                let signal: String = key.extract().map_err(|_| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>("Signal names must be strings")
                })?;
                let sig_ref = self.intern_signal_name(signal);

                for sample in samples.try_iter()? {
                    let (value, timestamp) = extract_pair(&sample?, "(value, timestamp)")?;
                    rust_steps.push(Step::new(
                        sig_ref,
                        value,
                        Duration::from_secs_f64(timestamp),
                    ));
                }
            }
        } else {
            for entry in steps.try_iter().map_err(|_| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Batch must be a dict of {signal: [(value, timestamp), ..]} or an iterable of (signal, value, timestamp) tuples",
                )
            })? {
                let entry = entry?;
                let tuple: Bound<'_, PyTuple> = entry.extract().map_err(|_| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        "Each step must be a tuple of (signal, value, timestamp)",
                    )
                })?;
                if tuple.len() != 3 {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        "Each step must be a tuple of (signal, value, timestamp)",
                    ));
                }

                let signal: String = tuple.get_item(0)?.extract()?;
                let sig_ref = self.intern_signal_name(signal);
                let value: f64 = tuple.get_item(1)?.extract()?;
                let timestamp: f64 = tuple.get_item(2)?.extract()?;
                rust_steps.push(Step::new(
                    sig_ref,
                    value,
                    Duration::from_secs_f64(timestamp),
                ));
            }
        }

        Ok(rust_steps)
    }

    /// Returns the `&'static str` mstlo stores for `signal`.
    ///
    /// `update` is called once per streamed sample, so the per-monitor cache is
    /// consulted first to keep the hot path off the global interner's lock; the
    /// interner is only reached the first time a monitor sees a given name.
    fn intern_signal_name(&mut self, signal: String) -> &'static str {
        if let Some(&cached) = self.signal_name_cache.get(&signal) {
            return cached;
        }
        let interned = intern(&signal);
        self.signal_name_cache.insert(signal, interned);
        interned
    }
}

/// Extracts a `(f64, f64)` pair from a Python 2-tuple.
fn extract_pair(item: &Bound<'_, PyAny>, expected: &str) -> PyResult<(f64, f64)> {
    let tuple: Bound<'_, PyTuple> = item.extract().map_err(|_| {
        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Each step must be a {expected}"))
    })?;
    if tuple.len() != 2 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "Each step must be a {expected}"
        )));
    }

    Ok((tuple.get_item(0)?.extract()?, tuple.get_item(1)?.extract()?))
}

fn convert_output_to_dict<Y: Clone, F>(
    py: Python,
    output: MonitorOutput<f64, Y>,
    val_mapper: F,
) -> PyResult<Py<PyAny>>
where
    F: Fn(Y) -> Py<PyAny>,
{
    let dict = PyDict::new(py);
    dict.set_item("input_signal", output.input_signal())?;
    dict.set_item(
        "input_timestamp",
        output.input_timestamp().map(|ts| ts.as_secs_f64()),
    )?;
    dict.set_item("input_value", output.input_value().copied())?;

    // Preserve the structure of individual evaluations/sync steps
    let mut evaluations_list = Vec::new();

    // Iterate over all evaluations triggered by this input
    for eval in output.sync_evaluations() {
        let eval_dict = PyDict::new(py);

        // Add sync step information
        eval_dict.set_item("sync_step_signal", eval.sync_step.signal)?;
        eval_dict.set_item(
            "sync_step_timestamp",
            eval.sync_step.timestamp.as_secs_f64(),
        )?;
        eval_dict.set_item("sync_step_value", eval.sync_step.value)?;

        let mut outputs_list = Vec::new();

        for out_step in &eval.outputs {
            let val = out_step.value.clone();
            let output_dict = PyDict::new(py);
            output_dict.set_item("timestamp", out_step.timestamp.as_secs_f64())?;
            output_dict.set_item("value", val_mapper(val))?;
            outputs_list.push(output_dict);
        }

        eval_dict.set_item("outputs", outputs_list)?;
        evaluations_list.push(eval_dict);
    }

    dict.set_item("evaluations", evaluations_list)?;
    Ok(dict.into_any().unbind())
}

// -----------------------------------------------------------------------------
// 4. Module Definition
// -----------------------------------------------------------------------------

#[pymodule]
fn mstlo_python(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("__doc__", "Online Signal Temporal Logic (STL) monitoring library.\n\n\
        This library provides efficient online monitoring of STL formulas with multiple semantics:\n\
        - DelayedQualitative/EagerQualitative: true/false evaluation\n\
        - DelayedQuantitative: robustness as a single float value\n\
        - Rosi: robustness as an interval (min, max)\n\n\
        Example using parse_formula (recommended):\n\
        -----------------------------------------\n\
        >>> import mstlo_python\n\
        >>> # Parse formula using the same DSL syntax as Rust's stl! macro\n\
        >>> phi = mstlo_python.parse_formula('G[0, 5](x > 0.5)')\n\
        >>> # Create monitor with DelayedQuantitative semantics\n\
        >>> monitor = mstlo_python.Monitor(phi, semantics='DelayedQuantitative')\n\
        >>> # Feed data\n\
        >>> output = monitor.update('x', 1.0, 0.0)\n\
        >>> # Print using Rust's Display formatting\n\
        >>> print(output)\n\
        >>> # Access structured data\n\
        >>> print(output.to_dict())\n\n\
        Example using Formula builder methods:\n\
        --------------------------------------\n\
        >>> import mstlo_python\n\
        >>> # Create formula: Always[0,5](x > 0.5)\n\
        >>> phi = mstlo_python.Formula.always(0, 5, mstlo_python.Formula.gt('x', 0.5))\n\
        >>> # Create monitor with DelayedQuantitative semantics\n\
        >>> monitor = mstlo_python.Monitor(phi, semantics='DelayedQuantitative')\n\
        >>> # Feed data\n\
        >>> output = monitor.update('x', 1.0, 0.0)\n\
        >>> # Use __str__ and __repr__ for Rust-style formatting\n\
        >>> print(str(output))  # Display format\n\
        >>> print(repr(output)) # Debug format\n\
    ")?;

    m.add_function(wrap_pyfunction!(py_parse_formula, m)?)?;
    m.add_class::<Formula>()?;
    m.add_class::<PyVariables>()?;
    m.add_class::<PyMonitorOutput>()?;
    m.add_class::<Monitor>()?;

    Ok(())
}
