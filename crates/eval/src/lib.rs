//! Evaluator, validation, and export profile types.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::fmt;

use cue_rust_adt::{
    AdtError, BaseValue, Bottom, EnvironmentId, ExprId, Feature, Runtime, SemanticExpr, VertexId,
};
use cue_rust_source::{Diagnostic, DiagnosticReport, Severity, Span};
use indexmap::IndexMap;
use thiserror::Error;

const DEFAULT_MAX_EVALUATION_DEPTH: u32 = 128;

/// Evaluation options for a single value operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EvalOptions {
    /// Maximum recursive evaluation depth.
    pub max_depth: u32,
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_EVALUATION_DEPTH,
        }
    }
}

/// Validation options for CUE values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ValidateOptions {
    /// Require concrete values.
    pub concrete: bool,
    /// Report all validation errors instead of stopping at the first one.
    pub all_errors: bool,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        Self {
            concrete: true,
            all_errors: false,
        }
    }
}

/// Errors produced by evaluation and validation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EvalError {
    /// ADT runtime lookup failed.
    #[error(transparent)]
    Adt(#[from] AdtError),
    /// The operation produced diagnostics.
    #[error("evaluation produced diagnostics")]
    Diagnostics(DiagnosticReport),
}

/// Public value kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ValueKind {
    /// Top or otherwise unconstrained value.
    Top,
    /// Null value.
    Null,
    /// Boolean value.
    Bool,
    /// Number value.
    Number,
    /// String value.
    String,
    /// Bytes value.
    Bytes,
    /// Struct value.
    Struct,
    /// List value.
    List,
    /// Bottom value.
    Bottom,
}

/// A concrete evaluated value tree for the currently implemented core subset.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EvaluatedValue {
    /// Top or otherwise unconstrained value.
    Top,
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Number literal text.
    Number(String),
    /// String literal text.
    String(String),
    /// Bytes value.
    Bytes(Vec<u8>),
    /// Struct fields in deterministic order.
    Struct(IndexMap<String, EvaluatedValue>),
    /// List items.
    List(Vec<EvaluatedValue>),
    /// Builtin kind constraint.
    Kind(ValueKind),
    /// Semantic bottom.
    Bottom(Bottom),
}

impl EvaluatedValue {
    /// Returns the kind of this evaluated value.
    #[must_use]
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Top => ValueKind::Top,
            Self::Null => ValueKind::Null,
            Self::Bool(_) => ValueKind::Bool,
            Self::Number(_) => ValueKind::Number,
            Self::String(_) => ValueKind::String,
            Self::Bytes(_) => ValueKind::Bytes,
            Self::Struct(_) => ValueKind::Struct,
            Self::List(_) => ValueKind::List,
            Self::Kind(kind) => *kind,
            Self::Bottom(_) => ValueKind::Bottom,
        }
    }
}

/// Immutable handle to a compiled CUE value.
#[derive(Clone, Debug)]
pub struct Value {
    runtime: Runtime,
    root: Option<VertexId>,
    diagnostics: DiagnosticReport,
    evaluated: Option<EvaluatedValue>,
}

impl Value {
    /// Creates a value handle from a runtime and root vertex.
    #[must_use]
    pub fn new(runtime: Runtime, root: VertexId, diagnostics: DiagnosticReport) -> Self {
        Self {
            runtime,
            root: Some(root),
            diagnostics,
            evaluated: None,
        }
    }

    /// Creates a value handle from an already evaluated tree.
    #[must_use]
    pub fn from_evaluated(evaluated: EvaluatedValue) -> Self {
        let runtime = Runtime::default();
        Self {
            runtime,
            root: None,
            diagnostics: DiagnosticReport::new(),
            evaluated: Some(evaluated),
        }
    }

    /// Returns diagnostics accumulated before evaluation.
    #[must_use]
    pub fn diagnostics(&self) -> &DiagnosticReport {
        &self.diagnostics
    }

    /// Evaluates and returns the value kind.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError`] if ADT lookup fails or evaluation reports diagnostics.
    pub fn kind(&self) -> Result<ValueKind, EvalError> {
        Ok(self.evaluate()?.kind())
    }

    /// Evaluates this value into the core evaluated tree.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError`] if ADT lookup fails or evaluation reports diagnostics.
    pub fn evaluate(&self) -> Result<EvaluatedValue, EvalError> {
        if self.diagnostics.has_errors() {
            return Err(EvalError::Diagnostics(self.diagnostics.clone()));
        }
        if let Some(evaluated) = &self.evaluated {
            return Ok(evaluated.clone());
        }
        let Some(root) = self.root else {
            return Err(EvalError::Diagnostics(single_diagnostic(
                "cue.eval.missing_root",
                "value has no root vertex",
                None,
            )));
        };
        let mut evaluator = Evaluator::new(&self.runtime, EvalOptions::default());
        let value = evaluator.evaluate_vertex(root)?;
        evaluator.finish(value)
    }

    /// Validates this value with the provided options.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::Diagnostics`] when the value is invalid.
    pub fn validate(&self, options: ValidateOptions) -> Result<(), EvalError> {
        let value = self.evaluate()?;
        let mut report = DiagnosticReport::new();
        validate_value(&value, options, "$", &mut report);
        if report.has_errors() {
            return Err(EvalError::Diagnostics(report));
        }
        Ok(())
    }

    /// Unifies this value with another value.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError`] if either value cannot be evaluated.
    pub fn unify(&self, other: &Self) -> Result<Self, EvalError> {
        let left = self.evaluate()?;
        let right = other.evaluate()?;
        let unified = unify_values(left, right, None);
        Ok(Self::from_evaluated(unified))
    }

    /// Looks up a string field path.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::Diagnostics`] when the path does not select a value.
    pub fn lookup_path(&self, path: &[&str]) -> Result<Self, EvalError> {
        let mut current = self.evaluate()?;
        for segment in path {
            let EvaluatedValue::Struct(fields) = current else {
                return Err(EvalError::Diagnostics(single_diagnostic(
                    "cue.eval.invalid_lookup",
                    format!("cannot select `{segment}` from non-struct value"),
                    None,
                )));
            };
            let Some(next) = fields.get(*segment).cloned() else {
                return Err(EvalError::Diagnostics(single_diagnostic(
                    "cue.eval.missing_field",
                    format!("field `{segment}` does not exist"),
                    None,
                )));
            };
            current = next;
        }
        Ok(Self::from_evaluated(current))
    }
}

#[derive(Debug)]
struct Evaluator<'runtime> {
    runtime: &'runtime Runtime,
    diagnostics: DiagnosticReport,
    options: EvalOptions,
    local_fields: Vec<IndexMap<Feature, ExprId>>,
}

impl<'runtime> Evaluator<'runtime> {
    fn new(runtime: &'runtime Runtime, options: EvalOptions) -> Self {
        Self {
            runtime,
            diagnostics: DiagnosticReport::new(),
            options,
            local_fields: Vec::new(),
        }
    }

    fn evaluate_vertex(&mut self, vertex_id: VertexId) -> Result<EvaluatedValue, EvalError> {
        self.evaluate_vertex_at(vertex_id, 0)
    }

    fn finish(self, value: EvaluatedValue) -> Result<EvaluatedValue, EvalError> {
        if self.diagnostics.has_errors() {
            return Err(EvalError::Diagnostics(self.diagnostics));
        }
        Ok(value)
    }

    fn evaluate_vertex_at(
        &mut self,
        vertex_id: VertexId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        if depth > self.options.max_depth {
            return Ok(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.depth_limit",
                "evaluation depth limit exceeded",
                None,
                false,
            )));
        }

        let vertex = self.runtime.vertex(vertex_id)?;
        if let Some(bottom) = &vertex.bottom {
            return Ok(EvaluatedValue::Bottom(bottom.clone()));
        }

        let mut value = value_from_base(&vertex.base);
        for conjunct_id in &vertex.conjuncts {
            let conjunct = self.runtime.conjunct(*conjunct_id)?;
            let expression =
                self.evaluate_expr_at(conjunct.expression, conjunct.environment, depth + 1)?;
            value = unify_values(value, expression, conjunct.span);
        }

        if !vertex.arcs.is_empty() {
            let mut fields = match value {
                EvaluatedValue::Top | EvaluatedValue::Struct(_) => into_fields(value),
                other => {
                    return Ok(conflict_bottom(
                        other.kind(),
                        ValueKind::Struct,
                        None,
                        "cue.eval.struct_conflict",
                    ));
                }
            };
            for arc in vertex.arcs.values() {
                let label = self.feature_label(arc.feature);
                let child = self.evaluate_vertex_at(arc.target, depth + 1)?;
                if let Some(existing) = fields.shift_remove(&label) {
                    fields.insert(label, unify_values(existing, child, None));
                } else {
                    fields.insert(label, child);
                }
            }
            value = EvaluatedValue::Struct(fields);
        }

        Ok(value)
    }

    fn evaluate_expr_at(
        &mut self,
        expr_id: ExprId,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        if depth > self.options.max_depth {
            return Ok(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.depth_limit",
                "evaluation depth limit exceeded",
                None,
                false,
            )));
        }

        let value = match self.runtime.expression(expr_id)? {
            SemanticExpr::Base(base) => value_from_base(base),
            SemanticExpr::Struct(fields) => {
                let mut values = IndexMap::new();
                let local_fields = fields
                    .iter()
                    .map(|field| (field.feature, field.expression))
                    .collect();
                self.local_fields.push(local_fields);
                for field in fields {
                    let label = self.feature_label(field.feature);
                    let value = self.evaluate_expr_at(field.expression, environment, depth + 1)?;
                    if let Some(existing) = values.shift_remove(&label) {
                        values.insert(label, unify_values(existing, value, field.span));
                    } else {
                        values.insert(label, value);
                    }
                }
                self.local_fields.pop();
                EvaluatedValue::Struct(values)
            }
            SemanticExpr::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.evaluate_expr_at(*item, environment, depth + 1)?);
                }
                EvaluatedValue::List(values)
            }
            SemanticExpr::FieldReference { feature, up_count } => {
                self.evaluate_field_reference(environment, *feature, *up_count, depth + 1)?
            }
            SemanticExpr::ImportReference { path } => EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.unsupported_import",
                format!("import `{path}` is not loaded"),
                None,
                true,
            )),
            SemanticExpr::LetReference { expression } => {
                self.evaluate_expr_at(*expression, environment, depth + 1)?
            }
            SemanticExpr::Selector { base, feature } => {
                let base_value = self.evaluate_expr_at(*base, environment, depth + 1)?;
                self.select_field(base_value, *feature)
            }
            SemanticExpr::Index { base, index } => {
                let base_value = self.evaluate_expr_at(*base, environment, depth + 1)?;
                let index_value = self.evaluate_expr_at(*index, environment, depth + 1)?;
                evaluate_index(base_value, index_value)
            }
            SemanticExpr::Slice { base, start, end } => {
                let base_value = self.evaluate_expr_at(*base, environment, depth + 1)?;
                let start_value = start
                    .map(|start| self.evaluate_expr_at(start, environment, depth + 1))
                    .transpose()?;
                let end_value = end
                    .map(|end| self.evaluate_expr_at(end, environment, depth + 1))
                    .transpose()?;
                evaluate_slice(base_value, start_value, end_value)
            }
            SemanticExpr::Unary { op, expr } => {
                let value = self.evaluate_expr_at(*expr, environment, depth + 1)?;
                evaluate_unary(op, value)
            }
            SemanticExpr::Binary { op, left, right } => {
                let left = self.evaluate_expr_at(*left, environment, depth + 1)?;
                let right = self.evaluate_expr_at(*right, environment, depth + 1)?;
                evaluate_binary(op, left, right)
            }
            SemanticExpr::Default(expr) => self.evaluate_expr_at(*expr, environment, depth + 1)?,
            SemanticExpr::Bottom(bottom) => EvaluatedValue::Bottom(bottom.clone()),
            _ => EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.unsupported_expr",
                "unsupported expression",
                None,
                false,
            )),
        };
        Ok(value)
    }

    fn evaluate_field_reference(
        &mut self,
        environment: EnvironmentId,
        feature: Feature,
        up_count: u32,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        if up_count == 0
            && let Some(expr_id) = self
                .local_fields
                .iter()
                .rev()
                .find_map(|fields| fields.get(&feature).copied())
        {
            return self.evaluate_expr_at(expr_id, environment, depth + 1);
        }

        let mut environment_id = environment;
        for _ in 0..up_count {
            let environment = self.runtime.environment(environment_id)?;
            let Some(parent) = environment.parent else {
                return Ok(EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.invalid_reference",
                    "reference escapes lexical environment",
                    None,
                    false,
                )));
            };
            environment_id = parent;
        }

        let environment = self.runtime.environment(environment_id)?;
        let vertex = self.runtime.vertex(environment.vertex)?;
        let Some(arc) = vertex.arcs.get(&feature) else {
            let label = self.feature_label(feature);
            return Ok(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.missing_reference",
                format!("reference `{label}` does not exist"),
                None,
                true,
            )));
        };
        self.evaluate_vertex_at(arc.target, depth + 1)
    }

    fn select_field(&self, base: EvaluatedValue, feature: Feature) -> EvaluatedValue {
        let label = self.feature_label(feature);
        match base {
            EvaluatedValue::Struct(fields) => fields.get(&label).cloned().unwrap_or_else(|| {
                EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.missing_field",
                    format!("field `{label}` does not exist"),
                    None,
                    true,
                ))
            }),
            other => EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.invalid_selector",
                format!("cannot select `{label}` from {}", other.kind()),
                None,
                false,
            )),
        }
    }

    fn feature_label(&self, feature: Feature) -> String {
        self.runtime
            .features
            .lookup(feature)
            .map_or_else(|| "<unknown>".to_owned(), |interned| interned.label.clone())
    }
}

fn value_from_base(base: &BaseValue) -> EvaluatedValue {
    match base {
        BaseValue::Top => EvaluatedValue::Top,
        BaseValue::Null => EvaluatedValue::Null,
        BaseValue::Bool(value) => EvaluatedValue::Bool(*value),
        BaseValue::Number(value) => EvaluatedValue::Number(value.clone()),
        BaseValue::String(value) => EvaluatedValue::String(value.clone()),
        BaseValue::Bytes(value) => EvaluatedValue::Bytes(value.clone()),
        BaseValue::Struct => EvaluatedValue::Struct(IndexMap::new()),
        BaseValue::List => EvaluatedValue::List(Vec::new()),
        BaseValue::Builtin(name) => {
            builtin_kind(name).map_or(EvaluatedValue::Top, EvaluatedValue::Kind)
        }
        _ => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_base",
            "unsupported base value",
            None,
            false,
        )),
    }
}

fn into_fields(value: EvaluatedValue) -> IndexMap<String, EvaluatedValue> {
    match value {
        EvaluatedValue::Struct(fields) => fields,
        _ => IndexMap::new(),
    }
}

fn evaluate_unary(op: &str, value: EvaluatedValue) -> EvaluatedValue {
    match (op, value) {
        ("group" | "+", value) => value,
        ("-", EvaluatedValue::Number(value)) if value.starts_with('-') => {
            EvaluatedValue::Number(value.trim_start_matches('-').to_owned())
        }
        ("-", EvaluatedValue::Number(value)) => EvaluatedValue::Number(format!("-{value}")),
        ("!", EvaluatedValue::Bool(value)) => EvaluatedValue::Bool(!value),
        (op, value) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_unary",
            format!("cannot apply unary `{op}` to {}", value.kind()),
            None,
            false,
        )),
    }
}

fn evaluate_binary(op: &str, left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    match op {
        "&" | "&&" => unify_values(left, right, None),
        "|" | "||" => choose_disjunction(left, right),
        "==" => EvaluatedValue::Bool(left == right),
        "!=" => EvaluatedValue::Bool(left != right),
        "+" => evaluate_add(left, right),
        _ => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_binary",
            format!("unsupported binary operator `{op}`"),
            None,
            false,
        )),
    }
}

fn choose_disjunction(left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    match left {
        EvaluatedValue::Bottom(_) => right,
        value => value,
    }
}

fn evaluate_add(left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    match (left, right) {
        (EvaluatedValue::String(left), EvaluatedValue::String(right)) => {
            EvaluatedValue::String(format!("{left}{right}"))
        }
        (left, right) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_add",
            format!("cannot add {} and {}", left.kind(), right.kind()),
            None,
            false,
        )),
    }
}

fn evaluate_index(base: EvaluatedValue, index: EvaluatedValue) -> EvaluatedValue {
    let EvaluatedValue::List(items) = base else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_index_base",
            "cannot index non-list value",
            None,
            false,
        ));
    };
    let EvaluatedValue::Number(index) = index else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_index",
            "list index must be a non-negative integer",
            None,
            false,
        ));
    };
    let Some(index) = parse_list_index(&index) else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_index",
            format!("invalid list index `{index}`"),
            None,
            false,
        ));
    };
    items.get(index).cloned().unwrap_or_else(|| {
        EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.index_out_of_bounds",
            format!("list index {index} is out of bounds"),
            None,
            false,
        ))
    })
}

fn evaluate_slice(
    base: EvaluatedValue,
    start: Option<EvaluatedValue>,
    end: Option<EvaluatedValue>,
) -> EvaluatedValue {
    let EvaluatedValue::List(items) = base else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_slice_base",
            "cannot slice non-list value",
            None,
            false,
        ));
    };
    let Some(start) = optional_slice_bound(start, 0) else {
        return invalid_slice_bound("start");
    };
    let Some(end) = optional_slice_bound(end, items.len()) else {
        return invalid_slice_bound("end");
    };
    if start > end || end > items.len() {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_slice",
            format!(
                "slice bounds [{start}:{end}] are invalid for list length {}",
                items.len()
            ),
            None,
            false,
        ));
    }
    EvaluatedValue::List(items.into_iter().skip(start).take(end - start).collect())
}

fn optional_slice_bound(bound: Option<EvaluatedValue>, default: usize) -> Option<usize> {
    match bound {
        Some(EvaluatedValue::Number(value)) => parse_list_index(&value),
        Some(_) => None,
        None => Some(default),
    }
}

fn invalid_slice_bound(name: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_slice_bound",
        format!("slice {name} bound must be a non-negative integer"),
        None,
        false,
    ))
}

fn parse_list_index(index: &str) -> Option<usize> {
    if index.starts_with('-') || index.contains(['.', 'e', 'E']) {
        return None;
    }
    let mut value = 0_usize;
    for byte in index.bytes() {
        if byte == b'_' {
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        let digit = usize::from(byte.saturating_sub(b'0'));
        value = value.checked_mul(10)?.checked_add(digit)?;
    }
    Some(value)
}

fn unify_values(left: EvaluatedValue, right: EvaluatedValue, span: Option<Span>) -> EvaluatedValue {
    match (left, right) {
        (EvaluatedValue::Top, value) | (value, EvaluatedValue::Top) => value,
        (EvaluatedValue::Bottom(bottom), _) | (_, EvaluatedValue::Bottom(bottom)) => {
            EvaluatedValue::Bottom(bottom)
        }
        (EvaluatedValue::Kind(kind), value) | (value, EvaluatedValue::Kind(kind)) => {
            if value.kind() == kind {
                value
            } else {
                conflict_bottom(kind, value.kind(), span, "cue.eval.kind_conflict")
            }
        }
        (EvaluatedValue::Null, EvaluatedValue::Null) => EvaluatedValue::Null,
        (EvaluatedValue::Bool(left), EvaluatedValue::Bool(right)) if left == right => {
            EvaluatedValue::Bool(left)
        }
        (EvaluatedValue::Number(left), EvaluatedValue::Number(right)) if left == right => {
            EvaluatedValue::Number(left)
        }
        (EvaluatedValue::String(left), EvaluatedValue::String(right)) if left == right => {
            EvaluatedValue::String(left)
        }
        (EvaluatedValue::Bytes(left), EvaluatedValue::Bytes(right)) if left == right => {
            EvaluatedValue::Bytes(left)
        }
        (EvaluatedValue::Struct(left), EvaluatedValue::Struct(right)) => {
            EvaluatedValue::Struct(unify_structs(left, right, span))
        }
        (EvaluatedValue::List(left), EvaluatedValue::List(right)) if left.len() == right.len() => {
            EvaluatedValue::List(
                left.into_iter()
                    .zip(right)
                    .map(|(left, right)| unify_values(left, right, span))
                    .collect(),
            )
        }
        (left, right) => conflict_bottom(left.kind(), right.kind(), span, "cue.eval.conflict"),
    }
}

fn unify_structs(
    mut left: IndexMap<String, EvaluatedValue>,
    right: IndexMap<String, EvaluatedValue>,
    span: Option<Span>,
) -> IndexMap<String, EvaluatedValue> {
    for (label, right_value) in right {
        if let Some(left_value) = left.shift_remove(&label) {
            left.insert(label, unify_values(left_value, right_value, span));
        } else {
            left.insert(label, right_value);
        }
    }
    left
}

fn conflict_bottom(
    left: ValueKind,
    right: ValueKind,
    span: Option<Span>,
    code: impl Into<String>,
) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        code,
        format!("conflicting values {left} and {right}"),
        span,
        false,
    ))
}

fn validate_value(
    value: &EvaluatedValue,
    options: ValidateOptions,
    path: &str,
    report: &mut DiagnosticReport,
) {
    match value {
        EvaluatedValue::Top | EvaluatedValue::Kind(_) if options.concrete => {
            report.push(Diagnostic::new(
                Severity::Error,
                "cue.eval.incomplete",
                format!("{path}: incomplete value"),
                None,
            ));
        }
        EvaluatedValue::Bottom(bottom) => report.push(Diagnostic::new(
            Severity::Error,
            "cue.eval.bottom",
            format!("{path}: {}", bottom.message),
            bottom.span,
        )),
        EvaluatedValue::Struct(fields) => {
            for (label, field) in fields {
                let field_path = format!("{path}.{label}");
                validate_value(field, options, &field_path, report);
                if report.has_errors() && !options.all_errors {
                    return;
                }
            }
        }
        EvaluatedValue::List(items) => {
            for (index, item) in items.iter().enumerate() {
                let item_path = format!("{path}[{index}]");
                validate_value(item, options, &item_path, report);
                if report.has_errors() && !options.all_errors {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn builtin_kind(name: &str) -> Option<ValueKind> {
    match name {
        "bool" => Some(ValueKind::Bool),
        "int" | "number" => Some(ValueKind::Number),
        "null" => Some(ValueKind::Null),
        "string" => Some(ValueKind::String),
        _ => None,
    }
}

fn single_diagnostic(
    code: &'static str,
    message: impl Into<String>,
    span: Option<Span>,
) -> DiagnosticReport {
    let mut report = DiagnosticReport::new();
    report.push(Diagnostic::new(Severity::Error, code, message, span));
    report
}

impl fmt::Display for ValueKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Top => "top",
            Self::Null => "null",
            Self::Bool => "bool",
            Self::Number => "number",
            Self::String => "string",
            Self::Bytes => "bytes",
            Self::Struct => "struct",
            Self::List => "list",
            Self::Bottom => "bottom",
        })
    }
}

#[cfg(test)]
mod tests {
    use cue_rust_adt::{BaseValue, Conjunct, Environment, Runtime, SemanticExpr, Vertex};
    use cue_rust_source::DiagnosticReport;

    use super::{EvaluatedValue, ValidateOptions, Value, ValueKind};

    #[test]
    fn test_should_report_value_kind() -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = Runtime::default();
        let root = runtime.add_vertex(Vertex::new(None, None, BaseValue::Struct))?;
        let value = Value::new(runtime, root, DiagnosticReport::default());
        assert_eq!(ValueKind::Struct, value.kind()?);
        Ok(())
    }

    #[test]
    fn test_should_unify_duplicate_scalar_conjuncts() -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = Runtime::default();
        let root = runtime.add_vertex(Vertex::new(None, None, BaseValue::Top))?;
        let environment = runtime.add_environment(Environment {
            parent: None,
            vertex: root,
        })?;
        let first = runtime.add_expression(SemanticExpr::Base(BaseValue::Number("1".into())))?;
        let second = runtime.add_expression(SemanticExpr::Base(BaseValue::Number("1".into())))?;
        runtime.add_conjunct(
            root,
            Conjunct {
                environment,
                expression: first,
                span: None,
            },
        )?;
        runtime.add_conjunct(
            root,
            Conjunct {
                environment,
                expression: second,
                span: None,
            },
        )?;
        let value = Value::new(runtime, root, DiagnosticReport::default());
        assert_eq!(EvaluatedValue::Number("1".into()), value.evaluate()?);
        Ok(())
    }

    #[test]
    fn test_should_validate_conflict_as_error() -> Result<(), Box<dyn std::error::Error>> {
        let left = Value::from_evaluated(EvaluatedValue::Number("1".into()));
        let right = Value::from_evaluated(EvaluatedValue::Number("2".into()));
        let value = left.unify(&right)?;
        assert!(value.validate(ValidateOptions::default()).is_err());
        Ok(())
    }

    #[test]
    fn test_should_lookup_path() -> Result<(), Box<dyn std::error::Error>> {
        let mut fields = indexmap::IndexMap::new();
        fields.insert("x".to_owned(), EvaluatedValue::String("ok".into()));
        let value = Value::from_evaluated(EvaluatedValue::Struct(fields));
        assert_eq!(ValueKind::String, value.lookup_path(&["x"])?.kind()?);
        Ok(())
    }
}
