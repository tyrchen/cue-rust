//! Evaluator, validation, and export profile types.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{cmp::Ordering, fmt, num::FpCategory};

use cue_rust_adt::{
    AdtError, BaseValue, Bottom, EnvironmentId, ExprId, Feature, FieldExpr, FieldMetadata, Runtime,
    SemanticExpr, VertexId,
};
use cue_rust_source::{Diagnostic, DiagnosticReport, Severity, Span};
use indexmap::IndexMap;
use regex::{Regex, RegexBuilder};
use thiserror::Error;

const DEFAULT_MAX_EVALUATION_DEPTH: u32 = 128;
const MAX_REGEX_PATTERN_BYTES: usize = 4 * 1024;
const REGEX_SIZE_LIMIT_BYTES: usize = 1024 * 1024;
const REGEX_DFA_SIZE_LIMIT_BYTES: usize = 1024 * 1024;

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

/// Field visibility controls used when producing concrete export trees.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct ExportOptions {
    /// Include definition fields such as `#Schema`.
    pub include_definitions: bool,
    /// Include hidden fields such as `_scratch`.
    pub include_hidden: bool,
    /// Include optional fields such as `field?: value`.
    pub include_optional: bool,
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
    /// Integer number constraint.
    Int,
    /// Floating-point number constraint.
    Float,
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
    /// Closed struct fields in deterministic order.
    ClosedStruct(IndexMap<String, EvaluatedValue>),
    /// Optional field constraint included by an export profile.
    OptionalField(Box<EvaluatedValue>),
    /// List items.
    List(Vec<EvaluatedValue>),
    /// Builtin kind constraint.
    Kind(ValueKind),
    /// Numeric comparison constraint.
    NumericConstraint(Vec<NumericBound>),
    /// Regular-expression string constraint.
    RegexConstraint {
        /// Regex pattern text.
        pattern: String,
        /// Whether the pattern is negated.
        negated: bool,
    },
    /// Default-marked value inside a disjunction.
    Default(Box<EvaluatedValue>),
    /// Disjunction alternatives.
    Disjunction(Vec<Disjunct>),
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
            Self::Number(_) | Self::NumericConstraint(_) => ValueKind::Number,
            Self::String(_) | Self::RegexConstraint { .. } => ValueKind::String,
            Self::Bytes(_) => ValueKind::Bytes,
            Self::Struct(_) | Self::ClosedStruct(_) => ValueKind::Struct,
            Self::List(_) => ValueKind::List,
            Self::Kind(kind) => *kind,
            Self::OptionalField(value) | Self::Default(value) => value.kind(),
            Self::Disjunction(disjuncts) => disjunction_kind(disjuncts),
            Self::Bottom(_) => ValueKind::Bottom,
        }
    }

    /// Resolves a value to its default alternative when exactly one default is available.
    #[must_use]
    pub fn resolve_defaults(self) -> Self {
        resolve_default_value(self)
    }
}

/// Numeric comparison operator used by a bound constraint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NumericBoundOp {
    /// Less than.
    LessThan,
    /// Less than or equal.
    LessThanOrEqual,
    /// Greater than.
    GreaterThan,
    /// Greater than or equal.
    GreaterThanOrEqual,
}

impl NumericBoundOp {
    /// Returns the CUE operator spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
        }
    }
}

/// One numeric comparison bound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NumericBound {
    /// Comparison operator.
    pub op: NumericBoundOp,
    /// Bound number literal text.
    pub value: String,
}

/// One disjunction alternative.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Disjunct {
    /// Alternative value.
    pub value: Box<EvaluatedValue>,
    /// Whether this alternative is marked as a default.
    pub default: bool,
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

    /// Evaluates this value with export field visibility rules.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError`] if ADT lookup fails or evaluation reports diagnostics.
    pub fn evaluate_export(&self, options: ExportOptions) -> Result<EvaluatedValue, EvalError> {
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
        let mut evaluator =
            Evaluator::new(&self.runtime, EvalOptions::default()).with_export_options(options);
        let value = evaluator.evaluate_vertex(root)?;
        evaluator.finish(value)
    }

    /// Validates this value with the provided options.
    ///
    /// # Errors
    ///
    /// Returns [`EvalError::Diagnostics`] when the value is invalid.
    pub fn validate(&self, options: ValidateOptions) -> Result<(), EvalError> {
        let value = self.evaluate_export(ExportOptions::default())?;
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
        let options = ExportOptions {
            include_optional: true,
            ..ExportOptions::default()
        };
        let left = self.evaluate_export(options)?;
        let right = other.evaluate_export(options)?;
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
            current = current.resolve_defaults();
            let (EvaluatedValue::Struct(fields) | EvaluatedValue::ClosedStruct(fields)) = current
            else {
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
    export_options: Option<ExportOptions>,
    local_fields: Vec<IndexMap<Feature, LocalField>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LocalField {
    expression: ExprId,
    metadata: FieldMetadata,
}

impl<'runtime> Evaluator<'runtime> {
    fn new(runtime: &'runtime Runtime, options: EvalOptions) -> Self {
        Self {
            runtime,
            diagnostics: DiagnosticReport::new(),
            options,
            export_options: None,
            local_fields: Vec::new(),
        }
    }

    fn with_export_options(mut self, options: ExportOptions) -> Self {
        self.export_options = Some(options);
        self
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
                if !self.should_emit_field(arc.metadata) {
                    continue;
                }
                let label = self.feature_label(arc.feature);
                let mut child = self.evaluate_vertex_at(arc.target, depth + 1)?;
                if self.export_options.is_some() && is_optional_constraint(arc.metadata) {
                    child = EvaluatedValue::OptionalField(Box::new(child));
                }
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
                self.evaluate_struct_fields(fields, environment, depth + 1)?
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
            SemanticExpr::Call { callee, args } => {
                let callee = *callee;
                let args = args.clone();
                self.evaluate_call(callee, &args, environment, depth + 1)?
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
            SemanticExpr::Default(expr) => EvaluatedValue::Default(Box::new(
                self.evaluate_expr_at(*expr, environment, depth + 1)?,
            )),
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

    fn evaluate_struct_fields(
        &mut self,
        fields: &[FieldExpr],
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        let mut values = IndexMap::new();
        let local_fields = fields
            .iter()
            .map(|field| {
                (
                    field.feature,
                    LocalField {
                        expression: field.expression,
                        metadata: field.metadata,
                    },
                )
            })
            .collect();
        self.local_fields.push(local_fields);
        for field in fields {
            if !self.should_emit_field(field.metadata) {
                continue;
            }
            let label = self.feature_label(field.feature);
            let mut value = self.evaluate_expr_at(field.expression, environment, depth)?;
            if self.export_options.is_some() && is_optional_constraint(field.metadata) {
                value = EvaluatedValue::OptionalField(Box::new(value));
            }
            if let Some(existing) = values.shift_remove(&label) {
                values.insert(label, unify_values(existing, value, field.span));
            } else {
                values.insert(label, value);
            }
        }
        self.local_fields.pop();
        Ok(EvaluatedValue::Struct(values))
    }

    fn evaluate_call(
        &mut self,
        callee: ExprId,
        args: &[ExprId],
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        let builtin_name = match self.runtime.expression(callee)? {
            SemanticExpr::Base(BaseValue::Builtin(name)) => Some(name.clone()),
            _ => None,
        };
        let Some(name) = builtin_name else {
            return Ok(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.unsupported_call",
                "only builtin calls are supported",
                None,
                false,
            )));
        };

        let mut evaluated_args = Vec::with_capacity(args.len());
        for arg in args {
            evaluated_args.push(self.evaluate_expr_at(*arg, environment, depth + 1)?);
        }
        Ok(evaluate_builtin_call(&name, evaluated_args))
    }

    fn evaluate_field_reference(
        &mut self,
        environment: EnvironmentId,
        feature: Feature,
        up_count: u32,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        if up_count == 0
            && let Some(field) = self
                .local_fields
                .iter()
                .rev()
                .find_map(|fields| fields.get(&feature).copied())
        {
            if is_optional_constraint(field.metadata) {
                let label = self.feature_label(feature);
                return Ok(optional_reference_bottom(&label));
            }
            return self.evaluate_expr_at(field.expression, environment, depth + 1);
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
        if is_optional_constraint(arc.metadata) {
            let label = self.feature_label(feature);
            return Ok(optional_reference_bottom(&label));
        }
        self.evaluate_vertex_at(arc.target, depth + 1)
    }

    fn select_field(&self, base: EvaluatedValue, feature: Feature) -> EvaluatedValue {
        let label = self.feature_label(feature);
        match base.resolve_defaults() {
            EvaluatedValue::Struct(fields) | EvaluatedValue::ClosedStruct(fields) => {
                fields.get(&label).cloned().unwrap_or_else(|| {
                    EvaluatedValue::Bottom(Bottom::new(
                        "cue.eval.missing_field",
                        format!("field `{label}` does not exist"),
                        None,
                        true,
                    ))
                })
            }
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

    fn should_emit_field(&self, metadata: FieldMetadata) -> bool {
        let Some(options) = self.export_options else {
            return true;
        };
        if metadata.is_definition() && !options.include_definitions {
            return false;
        }
        if metadata.is_hidden() && !options.include_hidden {
            return false;
        }
        if is_optional_constraint(metadata) && !options.include_optional {
            return false;
        }
        true
    }
}

fn is_optional_constraint(metadata: FieldMetadata) -> bool {
    metadata.is_optional() && !metadata.is_regular() && !metadata.is_required()
}

fn optional_reference_bottom(label: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.optional_field_reference",
        format!("cannot reference optional field `{label}`"),
        None,
        false,
    ))
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
        EvaluatedValue::Struct(fields) | EvaluatedValue::ClosedStruct(fields) => fields,
        _ => IndexMap::new(),
    }
}

fn evaluate_unary(op: &str, value: EvaluatedValue) -> EvaluatedValue {
    match (op, value) {
        ("group", value) => value,
        ("+", EvaluatedValue::Number(value)) => EvaluatedValue::Number(value),
        ("-", EvaluatedValue::Number(value)) if value.starts_with('-') => {
            EvaluatedValue::Number(value.trim_start_matches('-').to_owned())
        }
        ("-", EvaluatedValue::Number(value)) => EvaluatedValue::Number(format!("-{value}")),
        ("!", EvaluatedValue::Bool(value)) => EvaluatedValue::Bool(!value),
        ("=~" | "!~", EvaluatedValue::String(pattern)) => EvaluatedValue::RegexConstraint {
            pattern,
            negated: op == "!~",
        },
        ("<" | "<=" | ">" | ">=", EvaluatedValue::Number(value)) => {
            if let Some(op) = numeric_bound_op(op) {
                EvaluatedValue::NumericConstraint(vec![NumericBound { op, value }])
            } else {
                invalid_unary(op, &EvaluatedValue::Number(value))
            }
        }
        (op, value) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_unary",
            format!("cannot apply unary `{op}` to {}", value.kind()),
            None,
            false,
        )),
    }
}

fn invalid_unary(op: &str, value: &EvaluatedValue) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_unary",
        format!("cannot apply unary `{op}` to {}", value.kind()),
        None,
        false,
    ))
}

fn evaluate_builtin_call(name: &str, args: Vec<EvaluatedValue>) -> EvaluatedValue {
    match name {
        "and" => evaluate_and(args),
        "close" => evaluate_close(args),
        "div" | "mod" | "quo" | "rem" => evaluate_integer_builtin(name, args),
        "len" => evaluate_len(args),
        "or" => evaluate_or(args),
        _ => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_builtin",
            format!("unsupported builtin `{name}`"),
            None,
            false,
        )),
    }
}

fn evaluate_integer_builtin(name: &str, args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_builtin_arity",
            format!("{name} expects 2 arguments, got {}", args.len()),
            None,
            false,
        ));
    }
    let mut args = args.into_iter();
    let Some(left) = args.next().and_then(integer_arg) else {
        return invalid_integer_builtin_arg(name);
    };
    let Some(right) = args.next().and_then(integer_arg) else {
        return invalid_integer_builtin_arg(name);
    };
    if right == 0 {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.division_by_zero",
            "division by zero",
            None,
            false,
        ));
    }

    let result = match name {
        "quo" => left.checked_div(right),
        "rem" => left.checked_rem(right),
        "div" => floor_div(left, right),
        "mod" => floor_div(left, right).and_then(|quotient| {
            quotient
                .checked_mul(right)
                .and_then(|product| left.checked_sub(product))
        }),
        _ => None,
    };
    result.map_or_else(
        || {
            EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.integer_overflow",
                format!("{name} overflowed integer range"),
                None,
                false,
            ))
        },
        |value| EvaluatedValue::Number(value.to_string()),
    )
}

fn evaluate_and(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(items) = single_list_arg(args) else {
        return invalid_list_builtin_arg("and");
    };
    items.into_iter().fold(EvaluatedValue::Top, |left, right| {
        unify_values(left, right, None)
    })
}

fn evaluate_or(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(items) = single_list_arg(args) else {
        return invalid_list_builtin_arg("or");
    };
    items.into_iter().reduce(evaluate_disjunction).map_or(
        EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.empty_disjunction",
            "or requires at least one alternative",
            None,
            false,
        )),
        collapse_disjunction,
    )
}

fn evaluate_close(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    if args.len() != 1 {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_builtin_arity",
            format!("close expects 1 argument, got {}", args.len()),
            None,
            false,
        ));
    }
    args.into_iter().next().map_or_else(
        || {
            EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.invalid_builtin_arity",
                "close expects 1 argument, got 0",
                None,
                false,
            ))
        },
        |value| match resolve_default_value(value) {
            EvaluatedValue::Struct(fields) | EvaluatedValue::ClosedStruct(fields) => {
                EvaluatedValue::ClosedStruct(fields)
            }
            value => value,
        },
    )
}

fn single_list_arg(args: Vec<EvaluatedValue>) -> Option<Vec<EvaluatedValue>> {
    if args.len() != 1 {
        return None;
    }
    let value = args.into_iter().next().map(resolve_default_value)?;
    match value {
        EvaluatedValue::List(items) => Some(items),
        _ => None,
    }
}

fn invalid_list_builtin_arg(name: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_builtin_arg",
        format!("{name} expects a single list argument"),
        None,
        false,
    ))
}

fn floor_div(left: i128, right: i128) -> Option<i128> {
    let divisor = right.checked_abs()?;
    let quotient = left.checked_div(divisor)?;
    let remainder = left.checked_rem(divisor)?;
    let unsigned_quotient = if remainder < 0 {
        quotient.checked_sub(1)?
    } else {
        quotient
    };
    if right < 0 {
        unsigned_quotient.checked_neg()
    } else {
        Some(unsigned_quotient)
    }
}

fn integer_arg(value: EvaluatedValue) -> Option<i128> {
    let EvaluatedValue::Number(value) = value else {
        return None;
    };
    parse_integer(&value)
}

fn invalid_integer_builtin_arg(name: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_builtin_arg",
        format!("{name} arguments must be integers"),
        None,
        false,
    ))
}

fn evaluate_len(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 1 {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_builtin_arity",
            format!("len expects 1 argument, got {}", args.len()),
            None,
            false,
        ));
    }
    let mut args = args.into_iter();
    let Some(value) = args.next() else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_builtin_arity",
            "len expects 1 argument, got 0",
            None,
            false,
        ));
    };
    match value {
        EvaluatedValue::String(value) => EvaluatedValue::Number(value.len().to_string()),
        EvaluatedValue::Bytes(value) => EvaluatedValue::Number(value.len().to_string()),
        EvaluatedValue::List(values) => EvaluatedValue::Number(values.len().to_string()),
        EvaluatedValue::Struct(fields) => EvaluatedValue::Number(fields.len().to_string()),
        EvaluatedValue::Bottom(bottom) => EvaluatedValue::Bottom(bottom),
        value => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_builtin_arg",
            format!("len cannot accept {}", value.kind()),
            None,
            false,
        )),
    }
}

fn evaluate_binary(op: &str, left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    match op {
        "&&" => evaluate_bool_binary("&&", left, right),
        "||" => evaluate_bool_binary("||", left, right),
        "&" => unify_values(left, right, None),
        "|" => evaluate_disjunction(left, right),
        "==" => evaluate_equality(&left, &right, false),
        "!=" => evaluate_equality(&left, &right, true),
        "=~" => evaluate_regex_binary(left, right, false),
        "!~" => evaluate_regex_binary(left, right, true),
        "+" => evaluate_add(left, right),
        "*" => evaluate_multiply(left, right),
        "-" | "/" | "<" | "<=" | ">" | ">=" => evaluate_numeric_binary(op, left, right),
        _ => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_binary",
            format!("unsupported binary operator `{op}`"),
            None,
            false,
        )),
    }
}

fn evaluate_bool_binary(op: &str, left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    match (op, left, right) {
        ("&&", EvaluatedValue::Bool(left), EvaluatedValue::Bool(right)) => {
            EvaluatedValue::Bool(left && right)
        }
        ("||", EvaluatedValue::Bool(left), EvaluatedValue::Bool(right)) => {
            EvaluatedValue::Bool(left || right)
        }
        (op, left, right) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_bool_operand",
            format!(
                "operator `{op}` cannot accept {} and {}",
                left.kind(),
                right.kind()
            ),
            None,
            false,
        )),
    }
}

fn evaluate_numeric_binary(
    op: &str,
    left: EvaluatedValue,
    right: EvaluatedValue,
) -> EvaluatedValue {
    let (left, right) = match (number_operand(left), number_operand(right)) {
        (Ok(left), Ok(right)) => (left, right),
        (Err(bottom), _) | (_, Err(bottom)) => return EvaluatedValue::Bottom(bottom),
    };

    match op {
        "-" => EvaluatedValue::Number(format_number(left - right)),
        "*" => EvaluatedValue::Number(format_number(left * right)),
        "/" if is_zero(right) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.division_by_zero",
            "division by zero",
            None,
            false,
        )),
        "/" => EvaluatedValue::Number(format_number(left / right)),
        "<" => EvaluatedValue::Bool(compare_numbers(left, right, Ordering::Less)),
        "<=" => EvaluatedValue::Bool(
            compare_numbers(left, right, Ordering::Less)
                || compare_numbers(left, right, Ordering::Equal),
        ),
        ">" => EvaluatedValue::Bool(compare_numbers(left, right, Ordering::Greater)),
        ">=" => EvaluatedValue::Bool(
            compare_numbers(left, right, Ordering::Greater)
                || compare_numbers(left, right, Ordering::Equal),
        ),
        _ => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_binary",
            format!("unsupported binary operator `{op}`"),
            None,
            false,
        )),
    }
}

fn evaluate_equality(
    left: &EvaluatedValue,
    right: &EvaluatedValue,
    negated: bool,
) -> EvaluatedValue {
    let equal = match values_equal(left, right) {
        Ok(equal) => equal,
        Err(bottom) => return EvaluatedValue::Bottom(bottom),
    };
    EvaluatedValue::Bool(if negated { !equal } else { equal })
}

fn values_equal(left: &EvaluatedValue, right: &EvaluatedValue) -> Result<bool, Bottom> {
    match (left, right) {
        (EvaluatedValue::Bottom(bottom), _) | (_, EvaluatedValue::Bottom(bottom)) => {
            Err(bottom.clone())
        }
        (EvaluatedValue::Number(left), EvaluatedValue::Number(right)) => {
            Ok(equal_numbers(left, right))
        }
        (
            EvaluatedValue::Struct(left) | EvaluatedValue::ClosedStruct(left),
            EvaluatedValue::Struct(right) | EvaluatedValue::ClosedStruct(right),
        ) => structs_equal(left, right),
        (EvaluatedValue::List(left), EvaluatedValue::List(right)) => lists_equal(left, right),
        (EvaluatedValue::Top, EvaluatedValue::Top)
        | (EvaluatedValue::Null, EvaluatedValue::Null) => Ok(true),
        (EvaluatedValue::Bool(left), EvaluatedValue::Bool(right)) => Ok(left == right),
        (EvaluatedValue::String(left), EvaluatedValue::String(right)) => Ok(left == right),
        (EvaluatedValue::Bytes(left), EvaluatedValue::Bytes(right)) => Ok(left == right),
        (EvaluatedValue::Kind(left), EvaluatedValue::Kind(right)) => Ok(left == right),
        (EvaluatedValue::NumericConstraint(left), EvaluatedValue::NumericConstraint(right)) => {
            Ok(left == right)
        }
        (
            EvaluatedValue::RegexConstraint {
                pattern: left_pattern,
                negated: left_negated,
            },
            EvaluatedValue::RegexConstraint {
                pattern: right_pattern,
                negated: right_negated,
            },
        ) => Ok(left_pattern == right_pattern && left_negated == right_negated),
        (EvaluatedValue::Default(left), EvaluatedValue::Default(right))
        | (EvaluatedValue::OptionalField(left), EvaluatedValue::OptionalField(right)) => {
            values_equal(left, right)
        }
        (EvaluatedValue::Disjunction(left), EvaluatedValue::Disjunction(right)) => {
            disjunctions_equal(left, right)
        }
        _ => Ok(false),
    }
}

fn equal_numbers(left: &str, right: &str) -> bool {
    match (parse_decimal_number(left), parse_decimal_number(right)) {
        (Some(left), Some(right)) => compare_decimal_numbers(&left, &right) == Ordering::Equal,
        _ => left == right,
    }
}

fn lists_equal(left: &[EvaluatedValue], right: &[EvaluatedValue]) -> Result<bool, Bottom> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.iter().zip(right) {
        if !values_equal(left, right)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn structs_equal(
    left: &IndexMap<String, EvaluatedValue>,
    right: &IndexMap<String, EvaluatedValue>,
) -> Result<bool, Bottom> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (label, left_value) in left {
        let Some(right_value) = right.get(label) else {
            return Ok(false);
        };
        if !values_equal(left_value, right_value)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn disjunctions_equal(left: &[Disjunct], right: &[Disjunct]) -> Result<bool, Bottom> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for left_disjunct in left {
        let mut matched = false;
        for right_disjunct in right {
            if left_disjunct.default == right_disjunct.default
                && values_equal(&left_disjunct.value, &right_disjunct.value)?
            {
                matched = true;
                break;
            }
        }
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn compare_numbers(left: f64, right: f64, ordering: Ordering) -> bool {
    left.partial_cmp(&right)
        .is_some_and(|actual_ordering| actual_ordering == ordering)
}

fn is_zero(value: f64) -> bool {
    matches!(value.classify(), FpCategory::Zero)
}

#[derive(Debug, Eq, PartialEq)]
struct DecimalNumber {
    sign: i8,
    digits: String,
    scale: i64,
}

impl DecimalNumber {
    fn zero() -> Self {
        Self {
            sign: 0,
            digits: "0".to_owned(),
            scale: 0,
        }
    }
}

fn evaluate_disjunction(left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    collapse_disjunction(EvaluatedValue::Disjunction(unique_disjuncts(
        disjuncts_from(left)
            .into_iter()
            .chain(disjuncts_from(right))
            .collect(),
    )))
}

fn disjuncts_from(value: EvaluatedValue) -> Vec<Disjunct> {
    match value {
        EvaluatedValue::Bottom(_) => Vec::new(),
        EvaluatedValue::Disjunction(disjuncts) => disjuncts,
        EvaluatedValue::Default(value) => vec![Disjunct {
            value,
            default: true,
        }],
        value => vec![Disjunct {
            value: Box::new(value),
            default: false,
        }],
    }
}

fn unique_disjuncts(disjuncts: Vec<Disjunct>) -> Vec<Disjunct> {
    let mut unique: Vec<Disjunct> = Vec::new();
    'outer: for disjunct in disjuncts {
        for existing in &mut unique {
            if values_equal(existing.value.as_ref(), disjunct.value.as_ref()).unwrap_or(false) {
                existing.default |= disjunct.default;
                continue 'outer;
            }
        }
        unique.push(disjunct);
    }
    unique
}

fn collapse_disjunction(value: EvaluatedValue) -> EvaluatedValue {
    let EvaluatedValue::Disjunction(disjuncts) = value else {
        return value;
    };
    match disjuncts.len() {
        0 => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.empty_disjunction",
            "disjunction has no valid alternatives",
            None,
            false,
        )),
        1 => disjuncts.into_iter().next().map_or_else(
            || {
                EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.empty_disjunction",
                    "disjunction has no valid alternatives",
                    None,
                    false,
                ))
            },
            |disjunct| *disjunct.value,
        ),
        _ => EvaluatedValue::Disjunction(disjuncts),
    }
}

fn resolve_default_value(value: EvaluatedValue) -> EvaluatedValue {
    match value {
        EvaluatedValue::Default(value) => resolve_default_value(*value),
        EvaluatedValue::Struct(fields) => EvaluatedValue::Struct(
            fields
                .into_iter()
                .map(|(label, value)| (label, resolve_default_value(value)))
                .collect(),
        ),
        EvaluatedValue::ClosedStruct(fields) => EvaluatedValue::ClosedStruct(
            fields
                .into_iter()
                .map(|(label, value)| (label, resolve_default_value(value)))
                .collect(),
        ),
        EvaluatedValue::OptionalField(value) => {
            EvaluatedValue::OptionalField(Box::new(resolve_default_value(*value)))
        }
        EvaluatedValue::List(items) => {
            EvaluatedValue::List(items.into_iter().map(resolve_default_value).collect())
        }
        EvaluatedValue::Disjunction(disjuncts) => {
            let defaults = disjuncts
                .iter()
                .filter(|disjunct| disjunct.default)
                .collect::<Vec<_>>();
            if defaults.len() == 1
                && let Some(disjunct) = defaults.first()
            {
                return resolve_default_value((*disjunct.value).clone());
            }
            EvaluatedValue::Disjunction(disjuncts)
        }
        value => value,
    }
}

fn evaluate_regex_binary(
    left: EvaluatedValue,
    right: EvaluatedValue,
    negated: bool,
) -> EvaluatedValue {
    let EvaluatedValue::String(pattern) = right else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_regex_pattern",
            "regex operator requires a string pattern",
            None,
            false,
        ));
    };
    let Some(input) = regex_input(left) else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_regex_input",
            "regex operator requires string or bytes input",
            None,
            false,
        ));
    };
    match compile_regex(&pattern) {
        Ok(regex) => EvaluatedValue::Bool(regex.is_match(&input) != negated),
        Err(bottom) => EvaluatedValue::Bottom(bottom),
    }
}

fn regex_input(value: EvaluatedValue) -> Option<String> {
    match value {
        EvaluatedValue::String(value) => Some(value),
        EvaluatedValue::Bytes(value) => String::from_utf8(value).ok(),
        _ => None,
    }
}

fn compile_regex(pattern: &str) -> Result<Regex, Bottom> {
    if pattern.len() > MAX_REGEX_PATTERN_BYTES {
        return Err(Bottom::new(
            "cue.eval.regex_too_large",
            format!("regex pattern exceeds {MAX_REGEX_PATTERN_BYTES} byte limit"),
            None,
            false,
        ));
    }
    RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT_BYTES)
        .dfa_size_limit(REGEX_DFA_SIZE_LIMIT_BYTES)
        .build()
        .map_err(|error| {
            Bottom::new(
                "cue.eval.invalid_regex",
                format!("invalid regex pattern: {error}"),
                None,
                false,
            )
        })
}

fn evaluate_add(left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    match (left, right) {
        (EvaluatedValue::String(left), EvaluatedValue::String(right)) => {
            EvaluatedValue::String(format!("{left}{right}"))
        }
        (EvaluatedValue::Bytes(mut left), EvaluatedValue::Bytes(right)) => {
            left.extend(right);
            EvaluatedValue::Bytes(left)
        }
        (EvaluatedValue::Number(left), EvaluatedValue::Number(right)) => {
            match (parse_finite_f64(&left), parse_finite_f64(&right)) {
                (Some(left), Some(right)) => EvaluatedValue::Number(format_number(left + right)),
                _ => EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.invalid_number",
                    "invalid numeric operand",
                    None,
                    false,
                )),
            }
        }
        (left, right) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_add",
            format!("cannot add {} and {}", left.kind(), right.kind()),
            None,
            false,
        )),
    }
}

fn evaluate_multiply(left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    match (left, right) {
        (EvaluatedValue::Number(left), EvaluatedValue::Number(right)) => {
            match (parse_finite_f64(&left), parse_finite_f64(&right)) {
                (Some(left), Some(right)) => EvaluatedValue::Number(format_number(left * right)),
                _ => EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.invalid_number",
                    "invalid numeric operand",
                    None,
                    false,
                )),
            }
        }
        (EvaluatedValue::String(value), EvaluatedValue::Number(count))
        | (EvaluatedValue::Number(count), EvaluatedValue::String(value)) => {
            repeat_string(&value, &count)
        }
        (EvaluatedValue::Bytes(value), EvaluatedValue::Number(count))
        | (EvaluatedValue::Number(count), EvaluatedValue::Bytes(value)) => {
            repeat_bytes(&value, &count)
        }
        (left, right) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_multiply",
            format!("cannot multiply {} and {}", left.kind(), right.kind()),
            None,
            false,
        )),
    }
}

fn repeat_string(value: &str, count: &str) -> EvaluatedValue {
    let Some(count) = parse_list_index(count) else {
        return invalid_repeat_count();
    };
    EvaluatedValue::String(value.repeat(count))
}

fn repeat_bytes(value: &[u8], count: &str) -> EvaluatedValue {
    let Some(count) = parse_list_index(count) else {
        return invalid_repeat_count();
    };
    EvaluatedValue::Bytes(value.repeat(count))
}

fn invalid_repeat_count() -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_repeat_count",
        "repeat count must be a non-negative integer",
        None,
        false,
    ))
}

fn number_operand(value: EvaluatedValue) -> Result<f64, Bottom> {
    let EvaluatedValue::Number(value) = value else {
        return Err(Bottom::new(
            "cue.eval.invalid_numeric_operand",
            "numeric operator requires number operands",
            None,
            false,
        ));
    };
    parse_finite_f64(&value).ok_or_else(|| {
        Bottom::new(
            "cue.eval.invalid_number",
            format!("invalid numeric operand `{value}`"),
            None,
            false,
        )
    })
}

fn parse_finite_f64(value: &str) -> Option<f64> {
    value
        .replace('_', "")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_integer(value: &str) -> Option<i128> {
    if value.contains(['.', 'e', 'E']) {
        return None;
    }
    value.replace('_', "").parse::<i128>().ok()
}

fn parse_float(value: &str) -> Option<f64> {
    if !value.contains(['.', 'e', 'E']) {
        return None;
    }
    parse_finite_f64(value)
}

fn parse_decimal_number(value: &str) -> Option<DecimalNumber> {
    let value = value.replace('_', "");
    let (sign, unsigned) = value
        .strip_prefix('-')
        .map_or((1, value.as_str()), |value| (-1, value));
    let unsigned = unsigned.strip_prefix('+').unwrap_or(unsigned);
    let (mantissa, exponent) = split_exponent(unsigned)?;
    let (whole, fractional) = mantissa
        .split_once('.')
        .map_or((mantissa, ""), |(whole, fractional)| (whole, fractional));
    if whole.is_empty() && fractional.is_empty() {
        return None;
    }
    if !whole
        .bytes()
        .chain(fractional.bytes())
        .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let mut digits = String::with_capacity(whole.len().saturating_add(fractional.len()));
    digits.push_str(whole);
    digits.push_str(fractional);
    let mut digits = digits.trim_start_matches('0').to_owned();
    if digits.is_empty() {
        return Some(DecimalNumber::zero());
    }
    let fractional_len = i64::try_from(fractional.len()).ok()?;
    let mut scale = fractional_len.checked_sub(exponent)?;
    while digits.ends_with('0') {
        digits.pop();
        scale = scale.checked_sub(1)?;
    }
    Some(DecimalNumber {
        sign,
        digits,
        scale,
    })
}

fn split_exponent(value: &str) -> Option<(&str, i64)> {
    let Some(index) = value.find(['e', 'E']) else {
        return Some((value, 0));
    };
    let mantissa = value.get(..index)?;
    let exponent = value.get(index.saturating_add(1)..)?.parse::<i64>().ok()?;
    Some((mantissa, exponent))
}

fn compare_decimal_numbers(left: &DecimalNumber, right: &DecimalNumber) -> Ordering {
    match left.sign.cmp(&right.sign) {
        Ordering::Equal if left.sign == 0 => Ordering::Equal,
        Ordering::Equal if left.sign > 0 => compare_decimal_magnitude(left, right),
        Ordering::Equal => compare_decimal_magnitude(right, left),
        ordering => ordering,
    }
}

fn compare_decimal_magnitude(left: &DecimalNumber, right: &DecimalNumber) -> Ordering {
    let left_magnitude = decimal_magnitude(left);
    let right_magnitude = decimal_magnitude(right);
    match left_magnitude.cmp(&right_magnitude) {
        Ordering::Equal => compare_scaled_digits(left, right),
        ordering => ordering,
    }
}

fn decimal_magnitude(value: &DecimalNumber) -> i64 {
    i64::try_from(value.digits.len())
        .ok()
        .and_then(|len| len.checked_sub(value.scale))
        .unwrap_or(i64::MAX)
}

fn compare_scaled_digits(left: &DecimalNumber, right: &DecimalNumber) -> Ordering {
    let common_scale = left.scale.max(right.scale);
    let left_scaled = scaled_digits(left, common_scale);
    let right_scaled = scaled_digits(right, common_scale);
    match (left_scaled, right_scaled) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.digits.cmp(&right.digits),
    }
}

fn scaled_digits(value: &DecimalNumber, common_scale: i64) -> Option<String> {
    let zeros = common_scale.checked_sub(value.scale)?;
    let zeros = usize::try_from(zeros).ok()?;
    let mut digits = String::with_capacity(value.digits.len().checked_add(zeros)?);
    digits.push_str(&value.digits);
    digits.extend(std::iter::repeat_n('0', zeros));
    Some(digits)
}

fn format_number(value: f64) -> String {
    if is_zero(value.fract()) {
        return format!("{value:.0}");
    }
    value.to_string()
}

fn numeric_bound_op(op: &str) -> Option<NumericBoundOp> {
    match op {
        "<" => Some(NumericBoundOp::LessThan),
        "<=" => Some(NumericBoundOp::LessThanOrEqual),
        ">" => Some(NumericBoundOp::GreaterThan),
        ">=" => Some(NumericBoundOp::GreaterThanOrEqual),
        _ => None,
    }
}

fn number_satisfies_bounds(value: &str, bounds: &[NumericBound]) -> Result<bool, Bottom> {
    let Some(number) = parse_decimal_number(value) else {
        return Err(Bottom::new(
            "cue.eval.invalid_number",
            format!("invalid numeric operand `{value}`"),
            None,
            false,
        ));
    };
    for bound in bounds {
        let Some(limit) = parse_decimal_number(&bound.value) else {
            return Err(Bottom::new(
                "cue.eval.invalid_number",
                format!("invalid numeric bound `{}`", bound.value),
                None,
                false,
            ));
        };
        let ordering = compare_decimal_numbers(&number, &limit);
        let satisfied = match bound.op {
            NumericBoundOp::LessThan => ordering == Ordering::Less,
            NumericBoundOp::LessThanOrEqual => {
                ordering == Ordering::Less || ordering == Ordering::Equal
            }
            NumericBoundOp::GreaterThan => ordering == Ordering::Greater,
            NumericBoundOp::GreaterThanOrEqual => {
                ordering == Ordering::Greater || ordering == Ordering::Equal
            }
        };
        if !satisfied {
            return Ok(false);
        }
    }
    Ok(true)
}

fn evaluate_index(base: EvaluatedValue, index: EvaluatedValue) -> EvaluatedValue {
    match base {
        EvaluatedValue::List(items) => evaluate_list_index(&items, index),
        EvaluatedValue::Struct(fields) | EvaluatedValue::ClosedStruct(fields) => {
            evaluate_struct_index(&fields, index)
        }
        _ => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_index_base",
            "cannot index non-list or non-struct value",
            None,
            false,
        )),
    }
}

fn evaluate_list_index(items: &[EvaluatedValue], index: EvaluatedValue) -> EvaluatedValue {
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

fn evaluate_struct_index(
    fields: &IndexMap<String, EvaluatedValue>,
    index: EvaluatedValue,
) -> EvaluatedValue {
    let EvaluatedValue::String(index) = index else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_index",
            "struct index must be a string",
            None,
            false,
        ));
    };
    fields.get(&index).cloned().unwrap_or_else(|| {
        EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.missing_field",
            format!("field `{index}` does not exist"),
            None,
            true,
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
        (EvaluatedValue::Default(left), right) => unify_values(*left, right, span),
        (left, EvaluatedValue::Default(right)) => unify_values(left, *right, span),
        (EvaluatedValue::Disjunction(left), EvaluatedValue::Disjunction(right)) => {
            unify_disjunctions(left, &right, span)
        }
        (EvaluatedValue::Disjunction(disjuncts), value)
        | (value, EvaluatedValue::Disjunction(disjuncts)) => {
            unify_disjunction_with_value(disjuncts, &value, span)
        }
        (EvaluatedValue::NumericConstraint(mut left), EvaluatedValue::NumericConstraint(right)) => {
            left.extend(right);
            EvaluatedValue::NumericConstraint(left)
        }
        (EvaluatedValue::NumericConstraint(bounds), EvaluatedValue::Number(value))
        | (EvaluatedValue::Number(value), EvaluatedValue::NumericConstraint(bounds)) => {
            unify_numeric_constraint(value, &bounds)
        }
        (
            EvaluatedValue::RegexConstraint { pattern, negated },
            value @ (EvaluatedValue::String(_) | EvaluatedValue::Bytes(_)),
        )
        | (
            value @ (EvaluatedValue::String(_) | EvaluatedValue::Bytes(_)),
            EvaluatedValue::RegexConstraint { pattern, negated },
        ) => unify_regex_constraint(value, &pattern, negated),
        (EvaluatedValue::Kind(left), EvaluatedValue::Kind(right)) => {
            if let Some(kind) = intersect_kinds(left, right) {
                EvaluatedValue::Kind(kind)
            } else {
                conflict_bottom(left, right, span, "cue.eval.kind_conflict")
            }
        }
        (EvaluatedValue::Kind(kind), value) | (value, EvaluatedValue::Kind(kind)) => {
            if kind_accepts_value(kind, &value) {
                value
            } else {
                conflict_bottom(kind, value.kind(), span, "cue.eval.kind_conflict")
            }
        }
        (EvaluatedValue::Null, EvaluatedValue::Null) => EvaluatedValue::Null,
        (EvaluatedValue::Bool(left), EvaluatedValue::Bool(right)) if left == right => {
            EvaluatedValue::Bool(left)
        }
        (EvaluatedValue::Number(left), EvaluatedValue::Number(right))
            if equal_numbers(&left, &right) =>
        {
            EvaluatedValue::Number(left)
        }
        (EvaluatedValue::String(left), EvaluatedValue::String(right)) if left == right => {
            EvaluatedValue::String(left)
        }
        (EvaluatedValue::Bytes(left), EvaluatedValue::Bytes(right)) if left == right => {
            EvaluatedValue::Bytes(left)
        }
        (EvaluatedValue::ClosedStruct(left), EvaluatedValue::ClosedStruct(right)) => {
            unify_closed_structs(left, right, span)
        }
        (EvaluatedValue::ClosedStruct(left), EvaluatedValue::Struct(right))
        | (EvaluatedValue::Struct(right), EvaluatedValue::ClosedStruct(left)) => {
            unify_closed_struct(left, right, span)
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

fn unify_disjunctions(
    left: Vec<Disjunct>,
    right: &[Disjunct],
    span: Option<Span>,
) -> EvaluatedValue {
    let mut unified = Vec::new();
    for left_disjunct in left {
        for right_disjunct in right {
            let value = unify_values(
                (*left_disjunct.value).clone(),
                (*right_disjunct.value).clone(),
                span,
            );
            if !matches!(value, EvaluatedValue::Bottom(_)) {
                unified.push(Disjunct {
                    value: Box::new(value),
                    default: left_disjunct.default || right_disjunct.default,
                });
            }
        }
    }
    collapse_disjunction(EvaluatedValue::Disjunction(unique_disjuncts(unified)))
}

fn unify_disjunction_with_value(
    disjuncts: Vec<Disjunct>,
    value: &EvaluatedValue,
    span: Option<Span>,
) -> EvaluatedValue {
    let mut unified = Vec::new();
    for disjunct in disjuncts {
        let value = unify_values((*disjunct.value).clone(), value.clone(), span);
        if !matches!(value, EvaluatedValue::Bottom(_)) {
            unified.push(Disjunct {
                value: Box::new(value),
                default: disjunct.default,
            });
        }
    }
    collapse_disjunction(EvaluatedValue::Disjunction(unique_disjuncts(unified)))
}

fn unify_numeric_constraint(value: String, bounds: &[NumericBound]) -> EvaluatedValue {
    match number_satisfies_bounds(&value, bounds) {
        Ok(true) => EvaluatedValue::Number(value),
        Ok(false) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.numeric_bound_mismatch",
            format!("invalid value {value} for numeric constraint"),
            None,
            false,
        )),
        Err(bottom) => EvaluatedValue::Bottom(bottom),
    }
}

fn unify_structs(
    mut left: IndexMap<String, EvaluatedValue>,
    right: IndexMap<String, EvaluatedValue>,
    span: Option<Span>,
) -> IndexMap<String, EvaluatedValue> {
    for (label, right_value) in right {
        if let Some(left_value) = left.shift_remove(&label) {
            left.insert(label, unify_field_values(left_value, right_value, span));
        } else {
            left.insert(label, right_value);
        }
    }
    left.retain(|_, value| !matches!(value, EvaluatedValue::OptionalField(_)));
    left
}

fn unify_field_values(
    left: EvaluatedValue,
    right: EvaluatedValue,
    span: Option<Span>,
) -> EvaluatedValue {
    match (left, right) {
        (EvaluatedValue::OptionalField(left), EvaluatedValue::OptionalField(right)) => {
            EvaluatedValue::OptionalField(Box::new(unify_values(*left, *right, span)))
        }
        (EvaluatedValue::OptionalField(left), right) => unify_values(*left, right, span),
        (left, EvaluatedValue::OptionalField(right)) => unify_values(left, *right, span),
        (left, right) => unify_values(left, right, span),
    }
}

fn unify_closed_struct(
    mut closed: IndexMap<String, EvaluatedValue>,
    open: IndexMap<String, EvaluatedValue>,
    span: Option<Span>,
) -> EvaluatedValue {
    for (label, right_value) in open {
        let Some(left_value) = closed.shift_remove(&label) else {
            return EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.closed_struct",
                format!("field `{label}` not allowed in closed struct"),
                span,
                false,
            ));
        };
        closed.insert(label, unify_values(left_value, right_value, span));
    }
    EvaluatedValue::ClosedStruct(closed)
}

fn unify_closed_structs(
    left: IndexMap<String, EvaluatedValue>,
    right: IndexMap<String, EvaluatedValue>,
    span: Option<Span>,
) -> EvaluatedValue {
    if left.len() != right.len() || left.keys().any(|label| !right.contains_key(label)) {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.closed_struct",
            "closed structs have incompatible fields",
            span,
            false,
        ));
    }
    EvaluatedValue::ClosedStruct(unify_structs(left, right, span))
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

fn intersect_kinds(left: ValueKind, right: ValueKind) -> Option<ValueKind> {
    match (left, right) {
        (left, right) if left == right => Some(left),
        (ValueKind::Number, ValueKind::Int) | (ValueKind::Int, ValueKind::Number) => {
            Some(ValueKind::Int)
        }
        (ValueKind::Number, ValueKind::Float) | (ValueKind::Float, ValueKind::Number) => {
            Some(ValueKind::Float)
        }
        _ => None,
    }
}

fn kind_accepts_value(kind: ValueKind, value: &EvaluatedValue) -> bool {
    match (kind, value) {
        (ValueKind::Number, EvaluatedValue::Number(value)) => parse_decimal_number(value).is_some(),
        (ValueKind::Int, EvaluatedValue::Number(value)) => parse_integer(value).is_some(),
        (ValueKind::Float, EvaluatedValue::Number(value)) => parse_float(value).is_some(),
        (
            ValueKind::Number | ValueKind::Int | ValueKind::Float,
            EvaluatedValue::NumericConstraint(_),
        )
        | (ValueKind::Top, _)
        | (ValueKind::Null, EvaluatedValue::Null)
        | (ValueKind::Bool, EvaluatedValue::Bool(_))
        | (ValueKind::String, EvaluatedValue::String(_) | EvaluatedValue::RegexConstraint { .. })
        | (ValueKind::Bytes, EvaluatedValue::Bytes(_))
        | (ValueKind::Struct, EvaluatedValue::Struct(_) | EvaluatedValue::ClosedStruct(_))
        | (ValueKind::List, EvaluatedValue::List(_))
        | (ValueKind::Bottom, EvaluatedValue::Bottom(_)) => true,
        _ => false,
    }
}

fn disjunction_kind(disjuncts: &[Disjunct]) -> ValueKind {
    let mut kinds = disjuncts
        .iter()
        .map(|disjunct| disjunct.value.kind())
        .filter(|kind| *kind != ValueKind::Bottom);
    let Some(first) = kinds.next() else {
        return ValueKind::Bottom;
    };
    if kinds.all(|kind| kind == first) {
        first
    } else {
        ValueKind::Top
    }
}

fn unify_regex_constraint(value: EvaluatedValue, pattern: &str, negated: bool) -> EvaluatedValue {
    let Some(input) = regex_input(value.clone()) else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_regex_input",
            "regex constraint requires string or bytes input",
            None,
            false,
        ));
    };
    match compile_regex(pattern) {
        Ok(regex) if regex.is_match(&input) != negated => value,
        Ok(_) => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.regex_mismatch",
            format!("invalid value {input:?} for regex constraint"),
            None,
            false,
        )),
        Err(bottom) => EvaluatedValue::Bottom(bottom),
    }
}

fn validate_value(
    value: &EvaluatedValue,
    options: ValidateOptions,
    path: &str,
    report: &mut DiagnosticReport,
) {
    match value {
        EvaluatedValue::Top
        | EvaluatedValue::Kind(_)
        | EvaluatedValue::NumericConstraint(_)
        | EvaluatedValue::RegexConstraint { .. }
            if options.concrete =>
        {
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
        EvaluatedValue::Default(value) => validate_value(value, options, path, report),
        EvaluatedValue::OptionalField(value) => {
            if options.concrete {
                report.push(Diagnostic::new(
                    Severity::Error,
                    "cue.eval.incomplete_optional",
                    format!("{path}: optional field constraint is not concrete data"),
                    None,
                ));
            } else {
                validate_value(value, options, path, report);
            }
        }
        EvaluatedValue::Disjunction(disjuncts) => {
            validate_disjunction(disjuncts, options, path, report);
        }
        EvaluatedValue::Struct(fields) | EvaluatedValue::ClosedStruct(fields) => {
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

fn validate_disjunction(
    disjuncts: &[Disjunct],
    options: ValidateOptions,
    path: &str,
    report: &mut DiagnosticReport,
) {
    if !options.concrete {
        return;
    }
    let defaults = disjuncts
        .iter()
        .filter(|disjunct| disjunct.default)
        .collect::<Vec<_>>();
    if defaults.len() == 1
        && let Some(disjunct) = defaults.first()
    {
        validate_value(&disjunct.value, options, path, report);
        return;
    }
    report.push(Diagnostic::new(
        Severity::Error,
        "cue.eval.incomplete",
        format!("{path}: incomplete disjunction"),
        None,
    ));
}

fn builtin_kind(name: &str) -> Option<ValueKind> {
    match name {
        "bool" => Some(ValueKind::Bool),
        "bytes" => Some(ValueKind::Bytes),
        "float" => Some(ValueKind::Float),
        "int" => Some(ValueKind::Int),
        "null" => Some(ValueKind::Null),
        "number" => Some(ValueKind::Number),
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
            Self::Int => "int",
            Self::Float => "float",
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
