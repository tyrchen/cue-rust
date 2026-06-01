//! Evaluator, validation, and export profile types.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fmt, mem,
    num::FpCategory,
    str::FromStr,
};

use bigdecimal::{BigDecimal, Zero};
use cue_rust_adt::{
    AdtError, BaseValue, Bottom, Comprehension, ComprehensionClause, EnvironmentId, ExprId,
    Feature, FieldExpr, FieldLabel, FieldMetadata, Runtime, SemanticExpr, StringSegment,
    StructMember, VertexId,
};
use cue_rust_source::{Diagnostic, DiagnosticReport, Severity, Span};
use indexmap::IndexMap;
use regex::{Regex, RegexBuilder};
use thiserror::Error;

const DEFAULT_MAX_EVALUATION_DEPTH: u32 = 128;
const MAX_REGEX_PATTERN_BYTES: usize = 4 * 1024;
const REGEX_SIZE_LIMIT_BYTES: usize = 1024 * 1024;
const REGEX_DFA_SIZE_LIMIT_BYTES: usize = 1024 * 1024;
const MAX_BUILTIN_GENERATED_BYTES: usize = 16 * 1024 * 1024;
const MAX_BUILTIN_GENERATED_ITEMS: usize = 1_000_000;
const MAX_COMPREHENSION_GENERATED_ITEMS: usize = MAX_BUILTIN_GENERATED_ITEMS;
const MAX_SCHEMA_SORT_ITEMS: usize = 100_000;
const MAX_FIXPOINT_ITERATIONS: usize = 64;
const INVALID_DYNAMIC_LABEL_FIELD: &str = "<invalid-dynamic-label>";
const INVALID_PATTERN_LABEL_FIELD: &str = "<invalid-pattern-label>";

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
    /// Struct fields plus pattern constraints that apply to future fields.
    PatternedStruct {
        /// Concrete fields.
        fields: IndexMap<String, EvaluatedValue>,
        /// Pattern constraints.
        patterns: Vec<StructPatternConstraint>,
    },
    /// Closed struct fields in deterministic order.
    ClosedStruct(IndexMap<String, EvaluatedValue>),
    /// Closed struct fields plus pattern constraints that allow matching future fields.
    ClosedPatternedStruct {
        /// Concrete fields.
        fields: IndexMap<String, EvaluatedValue>,
        /// Pattern constraints that define allowed future fields.
        patterns: Vec<StructPatternConstraint>,
    },
    /// Optional field constraint included by an export profile.
    OptionalField(Box<EvaluatedValue>),
    /// List items.
    List(Vec<EvaluatedValue>),
    /// Open list with fixed prefix items and a tail constraint for remaining elements.
    OpenList {
        /// Fixed prefix items.
        items: Vec<EvaluatedValue>,
        /// Tail constraint for all remaining elements.
        tail: Box<EvaluatedValue>,
    },
    /// Builtin kind constraint.
    Kind(ValueKind),
    /// Builtin value or package constant.
    Builtin(String),
    /// Numeric comparison constraint.
    NumericConstraint(Vec<NumericBound>),
    /// Regular-expression string constraint.
    RegexConstraint {
        /// Regex pattern text.
        pattern: String,
        /// Whether the pattern is negated.
        negated: bool,
    },
    /// String rune-count constraints from `strings.MinRunes` and `strings.MaxRunes`.
    StringConstraints(Vec<StringConstraint>),
    /// Combined concrete string constraints.
    StringConstraintSet(StringConstraintSet),
    /// Default-marked value inside a disjunction.
    Default(Box<EvaluatedValue>),
    /// Disjunction alternatives.
    Disjunction(Vec<Disjunct>),
    #[doc(hidden)]
    /// Previous fixpoint iteration value for an active recursive component.
    FixpointPrevious(Box<EvaluatedValue>),
    /// Internal list items generated by a comprehension.
    ComprehensionItems(Vec<EvaluatedValue>),
    /// Semantic bottom.
    Bottom(Bottom),
}

impl EvaluatedValue {
    /// Returns the kind of this evaluated value.
    #[must_use]
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Top | Self::Builtin(_) => ValueKind::Top,
            Self::Null => ValueKind::Null,
            Self::Bool(_) => ValueKind::Bool,
            Self::Number(_) | Self::NumericConstraint(_) => ValueKind::Number,
            Self::String(_)
            | Self::RegexConstraint { .. }
            | Self::StringConstraints(_)
            | Self::StringConstraintSet(_) => ValueKind::String,
            Self::Bytes(_) => ValueKind::Bytes,
            Self::Struct(_)
            | Self::PatternedStruct { .. }
            | Self::ClosedStruct(_)
            | Self::ClosedPatternedStruct { .. } => ValueKind::Struct,
            Self::List(_) | Self::OpenList { .. } | Self::ComprehensionItems(_) => ValueKind::List,
            Self::Kind(kind) => *kind,
            Self::OptionalField(value) | Self::Default(value) | Self::FixpointPrevious(value) => {
                value.kind()
            }
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

/// String constraint operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StringConstraintOp {
    /// Minimum rune count.
    MinRunes,
    /// Maximum rune count.
    MaxRunes,
}

impl StringConstraintOp {
    /// Returns the builtin name for this string constraint.
    #[must_use]
    pub fn builtin_name(self) -> &'static str {
        match self {
            Self::MinRunes => "strings.MinRunes",
            Self::MaxRunes => "strings.MaxRunes",
        }
    }
}

/// One string rune-count constraint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StringConstraint {
    /// Constraint operator.
    pub op: StringConstraintOp,
    /// Rune-count limit.
    pub limit: i128,
}

/// One regex constraint that applies to string values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegexStringConstraint {
    /// Regex pattern text.
    pub pattern: String,
    /// Whether matching the pattern is disallowed.
    pub negated: bool,
}

/// Combined string constraints that must all hold for a concrete string.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StringConstraintSet {
    /// Rune-count constraints.
    pub runes: Vec<StringConstraint>,
    /// Regex constraints.
    pub regexes: Vec<RegexStringConstraint>,
}

/// Struct pattern constraint applied to matching field labels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructPatternConstraint {
    /// Label pattern.
    pub pattern: Box<EvaluatedValue>,
    /// Value constraint for matching labels.
    pub value: Box<EvaluatedValue>,
    /// Optional source span.
    pub span: Option<Span>,
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
            let (EvaluatedValue::Struct(fields)
            | EvaluatedValue::PatternedStruct { fields, .. }
            | EvaluatedValue::ClosedStruct(fields)
            | EvaluatedValue::ClosedPatternedStruct { fields, .. }) = current
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
    local_values: Vec<IndexMap<Feature, EvaluatedValue>>,
    local_value_scopes: Vec<LocalValueScope>,
    local_evaluating: Vec<HashSet<Feature>>,
    local_bindings: Vec<IndexMap<String, EvaluatedValue>>,
    field_evaluation_stack: Vec<FieldEvaluation>,
    vertex_cache: HashMap<VertexId, EvaluatedValue>,
    evaluating_vertices: HashSet<VertexId>,
    vertex_evaluation_stack: Vec<VertexId>,
    vertex_list_self_index_features: Vec<Feature>,
    fixpoint_active_vertices: HashSet<VertexId>,
    fixpoint_forcing_vertices: HashSet<VertexId>,
    fixpoint_previous_values: HashMap<VertexId, EvaluatedValue>,
    cycle_fallback_to_top: u32,
    defer_list_self_indexes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LocalField {
    expression: ExprId,
    metadata: FieldMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FieldEvaluation {
    scope_index: usize,
    feature: Feature,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InProgressField {
    expression: ExprId,
    environment: EnvironmentId,
    span: Option<Span>,
    metadata: FieldMetadata,
    local_fields: IndexMap<Feature, LocalField>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalValueScope {
    Struct,
    Overlay,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EvaluatedFieldLabel {
    Concrete(String),
    Pattern,
    Invalid(Bottom),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComprehensionControl {
    Continue,
    Stop,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct StructAccumulator {
    fields: IndexMap<String, EvaluatedValue>,
    patterns: Vec<StructPatternConstraint>,
    closed: bool,
}

impl<'runtime> Evaluator<'runtime> {
    fn new(runtime: &'runtime Runtime, options: EvalOptions) -> Self {
        Self {
            runtime,
            diagnostics: DiagnosticReport::new(),
            options,
            export_options: None,
            local_fields: Vec::new(),
            local_values: Vec::new(),
            local_value_scopes: Vec::new(),
            local_evaluating: Vec::new(),
            local_bindings: Vec::new(),
            field_evaluation_stack: Vec::new(),
            vertex_cache: HashMap::new(),
            evaluating_vertices: HashSet::new(),
            vertex_evaluation_stack: Vec::new(),
            vertex_list_self_index_features: Vec::new(),
            fixpoint_active_vertices: HashSet::new(),
            fixpoint_forcing_vertices: HashSet::new(),
            fixpoint_previous_values: HashMap::new(),
            cycle_fallback_to_top: 0,
            defer_list_self_indexes: 0,
        }
    }

    fn with_export_options(mut self, options: ExportOptions) -> Self {
        self.export_options = Some(options);
        self
    }

    fn evaluate_vertex(&mut self, vertex_id: VertexId) -> Result<EvaluatedValue, EvalError> {
        self.evaluate_reachable_fixpoints(vertex_id)?;
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
        if let Some(value) = self.fixpoint_reentry_value(vertex_id) {
            return Ok(value);
        }
        if let Some(value) = self.vertex_cache.get(&vertex_id) {
            return Ok(value.clone());
        }
        if !self.evaluating_vertices.insert(vertex_id) {
            if self.defer_list_self_indexes > 0
                && self
                    .vertex_evaluation_stack
                    .last()
                    .is_some_and(|current| *current == vertex_id)
            {
                return Ok(EvaluatedValue::Bottom(structural_cycle_bottom()));
            }
            if self.cycle_fallback_to_top > 0 {
                return Ok(EvaluatedValue::Top);
            }
            return Ok(if self.is_structural_vertex_cycle(vertex_id)? {
                EvaluatedValue::Bottom(structural_cycle_bottom())
            } else {
                EvaluatedValue::Top
            });
        }
        if depth > self.options.max_depth {
            self.evaluating_vertices.remove(&vertex_id);
            return Ok(depth_limit_bottom());
        }

        let vertex = self.runtime.vertex(vertex_id)?;
        if let Some(bottom) = &vertex.bottom {
            self.evaluating_vertices.remove(&vertex_id);
            return Ok(EvaluatedValue::Bottom(bottom.clone()));
        }
        self.vertex_evaluation_stack.push(vertex_id);

        let result: Result<EvaluatedValue, EvalError> = (|| {
            let mut value = value_from_base(&vertex.base);
            for conjunct_id in &vertex.conjuncts {
                let conjunct = self.runtime.conjunct(*conjunct_id)?;
                let self_list_feature = if self.is_list_expr(conjunct.expression)? {
                    vertex.feature
                } else {
                    None
                };
                if let Some(feature) = self_list_feature {
                    self.vertex_list_self_index_features.push(feature);
                }
                let expression_result =
                    self.evaluate_expr_at(conjunct.expression, conjunct.environment, depth + 1);
                if self_list_feature.is_some() {
                    self.vertex_list_self_index_features.pop();
                }
                let expression = expression_result?;
                value = unify_values(value, expression, conjunct.span);
            }

            if !vertex.arcs.is_empty() {
                let mut accumulator = match value {
                    EvaluatedValue::Top
                    | EvaluatedValue::Struct(_)
                    | EvaluatedValue::PatternedStruct { .. }
                    | EvaluatedValue::ClosedStruct(_)
                    | EvaluatedValue::ClosedPatternedStruct { .. } => {
                        into_struct_accumulator(value)
                    }
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
                    merge_field_value(label, child, None, &mut accumulator.fields);
                }
                apply_struct_patterns(&accumulator.patterns, &mut accumulator.fields);
                value = accumulator.into_value();
            }

            if self.is_definition_vertex(vertex_id)? {
                value = close_recursive(value);
            }

            Ok(value)
        })();
        self.vertex_evaluation_stack.pop();
        self.evaluating_vertices.remove(&vertex_id);
        let value = strip_fixpoint_previous_values(result?);
        self.vertex_cache.insert(vertex_id, value.clone());
        Ok(value)
    }

    fn fixpoint_reentry_value(&self, vertex_id: VertexId) -> Option<EvaluatedValue> {
        if !self.fixpoint_active_vertices.contains(&vertex_id) {
            return None;
        }
        if self.defer_list_self_indexes > 0 && self.fixpoint_forcing_vertices.contains(&vertex_id) {
            return Some(EvaluatedValue::Bottom(structural_cycle_bottom()));
        }
        (self.evaluating_vertices.contains(&vertex_id)
            || !self.fixpoint_forcing_vertices.contains(&vertex_id))
        .then(|| {
            EvaluatedValue::FixpointPrevious(Box::new(
                self.fixpoint_previous_values
                    .get(&vertex_id)
                    .cloned()
                    .unwrap_or(EvaluatedValue::Top),
            ))
        })
    }

    fn is_structural_vertex_cycle(&self, vertex_id: VertexId) -> Result<bool, EvalError> {
        let Some(current) = self.vertex_evaluation_stack.last().copied() else {
            return Ok(false);
        };
        if current == vertex_id && !self.field_evaluation_stack.is_empty() {
            return Ok(true);
        }
        let mut parent = self.runtime.vertex(current)?.parent;
        while let Some(parent_id) = parent {
            if parent_id == vertex_id {
                return Ok(true);
            }
            parent = self.runtime.vertex(parent_id)?.parent;
        }
        Ok(false)
    }

    fn is_definition_vertex(&self, vertex_id: VertexId) -> Result<bool, EvalError> {
        let vertex = self.runtime.vertex(vertex_id)?;
        let (Some(parent), Some(feature)) = (vertex.parent, vertex.feature) else {
            return Ok(false);
        };
        let parent = self.runtime.vertex(parent)?;
        Ok(parent
            .arcs
            .get(&feature)
            .is_some_and(|arc| arc.metadata.is_definition()))
    }

    fn evaluate_reachable_fixpoints(&mut self, root: VertexId) -> Result<(), EvalError> {
        if !self.fixpoint_active_vertices.is_empty() {
            return Ok(());
        }
        let vertices = self.collect_fixpoint_vertices(root)?;
        let graph = self.reference_dependency_graph(&vertices)?;
        let components = cyclic_graph_components(&graph);
        if components.is_empty() {
            return Ok(());
        }
        for component in components {
            if self.choice_vertex_count(&component)? > 1 {
                self.cache_cycle_bottoms(&component);
            } else {
                self.evaluate_fixpoint_vertices(component)?;
            }
        }
        Ok(())
    }

    fn collect_fixpoint_vertices(&self, root: VertexId) -> Result<Vec<VertexId>, EvalError> {
        let mut seen = HashSet::from([root]);
        let mut stack = vec![root];
        while let Some(vertex_id) = stack.pop() {
            self.push_arc_vertices(vertex_id, &mut seen, &mut stack)?;
            self.push_reference_vertices(vertex_id, &mut seen, &mut stack)?;
        }
        Ok(sorted_vertices(seen))
    }

    fn push_arc_vertices(
        &self,
        vertex_id: VertexId,
        seen: &mut HashSet<VertexId>,
        stack: &mut Vec<VertexId>,
    ) -> Result<(), EvalError> {
        for arc in self.runtime.vertex(vertex_id)?.arcs.values() {
            if seen.insert(arc.target) {
                stack.push(arc.target);
            }
        }
        Ok(())
    }

    fn push_reference_vertices(
        &self,
        vertex_id: VertexId,
        seen: &mut HashSet<VertexId>,
        stack: &mut Vec<VertexId>,
    ) -> Result<(), EvalError> {
        let mut dependencies = HashSet::new();
        self.collect_vertex_reference_targets(vertex_id, &mut dependencies)?;
        for dependency in dependencies {
            if seen.insert(dependency) {
                stack.push(dependency);
            }
        }
        Ok(())
    }

    fn reference_dependency_graph(
        &self,
        vertices: &[VertexId],
    ) -> Result<HashMap<VertexId, HashSet<VertexId>>, EvalError> {
        let vertex_set = vertices.iter().copied().collect::<HashSet<_>>();
        let mut graph = vertices
            .iter()
            .copied()
            .map(|vertex| (vertex, HashSet::new()))
            .collect::<HashMap<_, _>>();
        for vertex_id in vertices {
            let mut dependencies = HashSet::new();
            self.collect_vertex_reference_targets(*vertex_id, &mut dependencies)?;
            dependencies.retain(|dependency| vertex_set.contains(dependency));
            if let Some(edges) = graph.get_mut(vertex_id) {
                edges.extend(dependencies);
            }
        }
        Ok(graph)
    }

    fn collect_vertex_reference_targets(
        &self,
        vertex_id: VertexId,
        targets: &mut HashSet<VertexId>,
    ) -> Result<(), EvalError> {
        let vertex = self.runtime.vertex(vertex_id)?;
        for conjunct_id in &vertex.conjuncts {
            let conjunct = self.runtime.conjunct(*conjunct_id)?;
            let mut visited = HashSet::new();
            self.collect_expr_reference_targets(
                conjunct.expression,
                conjunct.environment,
                targets,
                &mut visited,
            )?;
        }
        if self.vertex_has_direct_self_list_index(vertex_id)? {
            targets.remove(&vertex_id);
        }
        Ok(())
    }

    fn vertex_has_direct_self_list_index(&self, vertex_id: VertexId) -> Result<bool, EvalError> {
        let vertex = self.runtime.vertex(vertex_id)?;
        for conjunct_id in &vertex.conjuncts {
            let conjunct = self.runtime.conjunct(*conjunct_id)?;
            if self.expr_has_direct_self_list_index(
                conjunct.expression,
                conjunct.environment,
                vertex_id,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn expr_has_direct_self_list_index(
        &self,
        expression: ExprId,
        environment: EnvironmentId,
        vertex_id: VertexId,
    ) -> Result<bool, EvalError> {
        let expression = self.skip_group_exprs(expression)?;
        match self.runtime.expression(expression)? {
            SemanticExpr::List { items, .. } => {
                for item in items {
                    if self.expr_is_direct_self_index(*item, environment, vertex_id)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            SemanticExpr::Binary { op, left, right } if op == "&" => Ok(self
                .expr_has_direct_self_list_index(*left, environment, vertex_id)?
                || self.expr_has_direct_self_list_index(*right, environment, vertex_id)?),
            _ => Ok(false),
        }
    }

    fn expr_is_direct_self_index(
        &self,
        expression: ExprId,
        environment: EnvironmentId,
        vertex_id: VertexId,
    ) -> Result<bool, EvalError> {
        let expression = self.skip_group_exprs(expression)?;
        let SemanticExpr::Index { base, .. } = self.runtime.expression(expression)? else {
            return Ok(false);
        };
        let base = self.skip_group_exprs(*base)?;
        let SemanticExpr::FieldReference { feature, up_count } = self.runtime.expression(base)?
        else {
            return Ok(false);
        };
        Ok(
            self.field_reference_target_vertex(environment, *feature, *up_count)?
                == Some(vertex_id),
        )
    }

    fn collect_expr_reference_targets(
        &self,
        expression: ExprId,
        environment: EnvironmentId,
        targets: &mut HashSet<VertexId>,
        visited: &mut HashSet<ExprId>,
    ) -> Result<(), EvalError> {
        if !visited.insert(expression) {
            return Ok(());
        }
        match self.runtime.expression(expression)? {
            SemanticExpr::Struct(members) => {
                self.collect_struct_reference_targets(members, environment, targets, visited)?;
            }
            SemanticExpr::List { items, tail } => {
                for item in items {
                    self.collect_expr_reference_targets(*item, environment, targets, visited)?;
                }
                if let Some(tail) = tail {
                    self.collect_expr_reference_targets(*tail, environment, targets, visited)?;
                }
            }
            SemanticExpr::FieldReference { feature, up_count } => {
                if let Some(target) =
                    self.field_reference_target_vertex(environment, *feature, *up_count)?
                {
                    targets.insert(target);
                }
            }
            SemanticExpr::LetReference { expression } | SemanticExpr::Default(expression) => {
                self.collect_expr_reference_targets(*expression, environment, targets, visited)?;
            }
            SemanticExpr::Selector { base, feature } => {
                if let Some(target) =
                    self.selector_reference_target_vertex(*base, *feature, environment)?
                {
                    targets.insert(target);
                }
                self.collect_expr_reference_targets(*base, environment, targets, visited)?;
            }
            SemanticExpr::Index { base, index } => {
                self.collect_expr_reference_targets(*base, environment, targets, visited)?;
                self.collect_expr_reference_targets(*index, environment, targets, visited)?;
            }
            SemanticExpr::Slice { base, start, end } => {
                self.collect_expr_reference_targets(*base, environment, targets, visited)?;
                if let Some(start) = start {
                    self.collect_expr_reference_targets(*start, environment, targets, visited)?;
                }
                if let Some(end) = end {
                    self.collect_expr_reference_targets(*end, environment, targets, visited)?;
                }
            }
            SemanticExpr::Call { callee, args } => {
                self.collect_expr_reference_targets(*callee, environment, targets, visited)?;
                for arg in args {
                    self.collect_expr_reference_targets(*arg, environment, targets, visited)?;
                }
            }
            SemanticExpr::Unary { expr, .. } => {
                self.collect_expr_reference_targets(*expr, environment, targets, visited)?;
            }
            SemanticExpr::Binary { left, right, .. } => {
                self.collect_expr_reference_targets(*left, environment, targets, visited)?;
                self.collect_expr_reference_targets(*right, environment, targets, visited)?;
            }
            SemanticExpr::InterpolatedString(segments) => {
                for segment in segments {
                    if let StringSegment::Expr(expression) = segment {
                        self.collect_expr_reference_targets(
                            *expression,
                            environment,
                            targets,
                            visited,
                        )?;
                    }
                }
            }
            SemanticExpr::Comprehension(comprehension) => {
                self.collect_comprehension_reference_targets(
                    comprehension,
                    environment,
                    targets,
                    visited,
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    fn collect_struct_reference_targets(
        &self,
        members: &[StructMember],
        environment: EnvironmentId,
        targets: &mut HashSet<VertexId>,
        visited: &mut HashSet<ExprId>,
    ) -> Result<(), EvalError> {
        for member in members {
            match member {
                StructMember::Field(field) => {
                    match &field.label {
                        FieldLabel::Dynamic(expression) | FieldLabel::Pattern(expression) => {
                            self.collect_expr_reference_targets(
                                *expression,
                                environment,
                                targets,
                                visited,
                            )?;
                        }
                        _ => {}
                    }
                    self.collect_expr_reference_targets(
                        field.expression,
                        environment,
                        targets,
                        visited,
                    )?;
                }
                StructMember::Comprehension(comprehension) => {
                    self.collect_comprehension_reference_targets(
                        comprehension,
                        environment,
                        targets,
                        visited,
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn collect_comprehension_reference_targets(
        &self,
        comprehension: &Comprehension,
        environment: EnvironmentId,
        targets: &mut HashSet<VertexId>,
        visited: &mut HashSet<ExprId>,
    ) -> Result<(), EvalError> {
        for clause in &comprehension.clauses {
            match clause {
                ComprehensionClause::For { source, .. } => {
                    self.collect_expr_reference_targets(*source, environment, targets, visited)?;
                }
                ComprehensionClause::If { condition } => {
                    self.collect_expr_reference_targets(*condition, environment, targets, visited)?;
                }
                _ => {}
            }
        }
        self.collect_expr_reference_targets(comprehension.body, environment, targets, visited)
    }

    fn selector_reference_target_vertex(
        &self,
        base: ExprId,
        feature: Feature,
        environment: EnvironmentId,
    ) -> Result<Option<VertexId>, EvalError> {
        let base = self.skip_group_exprs(base)?;
        let SemanticExpr::FieldReference {
            feature: base_feature,
            up_count,
        } = self.runtime.expression(base)?
        else {
            return Ok(None);
        };
        let Some(base_vertex) =
            self.field_reference_target_vertex(environment, *base_feature, *up_count)?
        else {
            return Ok(None);
        };
        Ok(self
            .runtime
            .vertex(base_vertex)?
            .arcs
            .get(&feature)
            .map(|arc| arc.target))
    }

    fn choice_vertex_count(&self, vertices: &HashSet<VertexId>) -> Result<usize, EvalError> {
        let mut count = 0_usize;
        for vertex in vertices {
            if self.vertex_contains_choice(*vertex)? {
                count = count.saturating_add(1);
            }
        }
        Ok(count)
    }

    fn vertex_contains_choice(&self, vertex_id: VertexId) -> Result<bool, EvalError> {
        let vertex = self.runtime.vertex(vertex_id)?;
        for conjunct_id in &vertex.conjuncts {
            let conjunct = self.runtime.conjunct(*conjunct_id)?;
            let mut visited = HashSet::new();
            if self.expr_contains_choice(conjunct.expression, &mut visited)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn expr_contains_choice(
        &self,
        expression: ExprId,
        visited: &mut HashSet<ExprId>,
    ) -> Result<bool, EvalError> {
        if !visited.insert(expression) {
            return Ok(false);
        }
        Ok(match self.runtime.expression(expression)? {
            SemanticExpr::Default(_) => true,
            SemanticExpr::Binary { op, left, right } => {
                op == "|"
                    || self.expr_contains_choice(*left, visited)?
                    || self.expr_contains_choice(*right, visited)?
            }
            SemanticExpr::Unary { expr, .. } | SemanticExpr::LetReference { expression: expr } => {
                self.expr_contains_choice(*expr, visited)?
            }
            SemanticExpr::Struct(members) => self.struct_contains_choice(members, visited)?,
            SemanticExpr::List { items, tail } => {
                self.exprs_contain_choice(items.iter().copied(), visited)?
                    || self.optional_expr_contains_choice(*tail, visited)?
            }
            SemanticExpr::Selector { base, .. } => self.expr_contains_choice(*base, visited)?,
            SemanticExpr::Index { base, index } => {
                self.expr_contains_choice(*base, visited)?
                    || self.expr_contains_choice(*index, visited)?
            }
            SemanticExpr::Slice { base, start, end } => {
                self.expr_contains_choice(*base, visited)?
                    || self.optional_expr_contains_choice(*start, visited)?
                    || self.optional_expr_contains_choice(*end, visited)?
            }
            SemanticExpr::Call { callee, args } => {
                self.expr_contains_choice(*callee, visited)?
                    || self.exprs_contain_choice(args.iter().copied(), visited)?
            }
            SemanticExpr::InterpolatedString(segments) => {
                let mut has_choice = false;
                for segment in segments {
                    if let StringSegment::Expr(expression) = segment {
                        has_choice |= self.expr_contains_choice(*expression, visited)?;
                    }
                }
                has_choice
            }
            SemanticExpr::Comprehension(comprehension) => {
                self.comprehension_contains_choice(comprehension, visited)?
            }
            _ => false,
        })
    }

    fn optional_expr_contains_choice(
        &self,
        expression: Option<ExprId>,
        visited: &mut HashSet<ExprId>,
    ) -> Result<bool, EvalError> {
        expression.map_or(Ok(false), |expression| {
            self.expr_contains_choice(expression, visited)
        })
    }

    fn exprs_contain_choice(
        &self,
        expressions: impl IntoIterator<Item = ExprId>,
        visited: &mut HashSet<ExprId>,
    ) -> Result<bool, EvalError> {
        for expression in expressions {
            if self.expr_contains_choice(expression, visited)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn struct_contains_choice(
        &self,
        members: &[StructMember],
        visited: &mut HashSet<ExprId>,
    ) -> Result<bool, EvalError> {
        for member in members {
            match member {
                StructMember::Field(field) => {
                    let label_has_choice = match &field.label {
                        FieldLabel::Dynamic(expression) | FieldLabel::Pattern(expression) => {
                            self.expr_contains_choice(*expression, visited)?
                        }
                        _ => false,
                    };
                    if label_has_choice || self.expr_contains_choice(field.expression, visited)? {
                        return Ok(true);
                    }
                }
                StructMember::Comprehension(comprehension)
                    if self.comprehension_contains_choice(comprehension, visited)? =>
                {
                    return Ok(true);
                }
                _ => {}
            }
        }
        Ok(false)
    }

    fn comprehension_contains_choice(
        &self,
        comprehension: &Comprehension,
        visited: &mut HashSet<ExprId>,
    ) -> Result<bool, EvalError> {
        for clause in &comprehension.clauses {
            let has_choice = match clause {
                ComprehensionClause::For { source, .. } => {
                    self.expr_contains_choice(*source, visited)?
                }
                ComprehensionClause::If { condition } => {
                    self.expr_contains_choice(*condition, visited)?
                }
                _ => false,
            };
            if has_choice {
                return Ok(true);
            }
        }
        self.expr_contains_choice(comprehension.body, visited)
    }

    fn cache_cycle_bottoms(&mut self, vertices: &HashSet<VertexId>) {
        for vertex in vertices {
            self.vertex_cache
                .insert(*vertex, EvaluatedValue::Bottom(cycle_bottom()));
        }
    }

    fn evaluate_fixpoint_vertices(
        &mut self,
        active_vertices: HashSet<VertexId>,
    ) -> Result<(), EvalError> {
        let ordered_vertices = sorted_vertices(active_vertices.iter().copied());
        let previous_values = ordered_vertices
            .iter()
            .map(|vertex| {
                (
                    *vertex,
                    self.vertex_cache
                        .get(vertex)
                        .cloned()
                        .unwrap_or(EvaluatedValue::Top),
                )
            })
            .collect::<HashMap<_, _>>();
        let saved_active = mem::replace(&mut self.fixpoint_active_vertices, active_vertices);
        let saved_previous = mem::replace(&mut self.fixpoint_previous_values, previous_values);
        let saved_forcing = mem::take(&mut self.fixpoint_forcing_vertices);
        let result = self.iterate_fixpoint_vertices(&ordered_vertices);
        self.fixpoint_forcing_vertices = saved_forcing;
        self.fixpoint_previous_values = saved_previous;
        self.fixpoint_active_vertices = saved_active;
        result
    }

    fn iterate_fixpoint_vertices(
        &mut self,
        ordered_vertices: &[VertexId],
    ) -> Result<(), EvalError> {
        let mut final_values = HashMap::new();
        let mut converged = false;
        for _ in 0..MAX_FIXPOINT_ITERATIONS {
            self.clear_fixpoint_cache_entries(ordered_vertices);
            let next_values = self.evaluate_fixpoint_iteration(ordered_vertices)?;
            if next_values == self.fixpoint_previous_values {
                final_values = next_values;
                converged = true;
                break;
            }
            self.fixpoint_previous_values.clone_from(&next_values);
            final_values = next_values;
        }
        if !converged {
            final_values = ordered_vertices
                .iter()
                .copied()
                .map(|vertex| (vertex, cycle_fixpoint_limit_bottom()))
                .collect();
        }
        self.clear_fixpoint_cache_entries(ordered_vertices);
        self.vertex_cache.extend(final_values);
        Ok(())
    }

    fn clear_fixpoint_cache_entries(&mut self, ordered_vertices: &[VertexId]) {
        for vertex in ordered_vertices {
            self.vertex_cache.remove(vertex);
        }
    }

    fn evaluate_fixpoint_iteration(
        &mut self,
        ordered_vertices: &[VertexId],
    ) -> Result<HashMap<VertexId, EvaluatedValue>, EvalError> {
        let mut next_values = HashMap::with_capacity(ordered_vertices.len());
        for vertex in ordered_vertices {
            self.fixpoint_forcing_vertices.insert(*vertex);
            let value = self.evaluate_vertex_at(*vertex, 0)?;
            self.fixpoint_forcing_vertices.remove(vertex);
            next_values.insert(*vertex, value);
        }
        Ok(next_values)
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
            SemanticExpr::List { items, tail } => {
                self.evaluate_list_expr(items, *tail, environment, depth + 1)?
            }
            SemanticExpr::FieldReference { feature, up_count } => {
                self.evaluate_field_reference(environment, *feature, *up_count, depth + 1)?
            }
            SemanticExpr::DynamicReference { name } => self.evaluate_dynamic_reference(name),
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
                self.evaluate_selector_expr(*base, *feature, environment, depth + 1)?
            }
            SemanticExpr::Index { base, index } => {
                let base_value = self.evaluate_expr_at(*base, environment, depth + 1)?;
                let index_value = self.evaluate_expr_at(*index, environment, depth + 1)?;
                evaluate_index(
                    resolve_default_operand_value(base_value),
                    index_value,
                    self.defer_list_self_indexes > 0,
                )
            }
            SemanticExpr::Slice { base, start, end } => {
                let base_value = self.evaluate_expr_at(*base, environment, depth + 1)?;
                let start_value = start
                    .map(|start| self.evaluate_expr_at(start, environment, depth + 1))
                    .transpose()?;
                let end_value = end
                    .map(|end| self.evaluate_expr_at(end, environment, depth + 1))
                    .transpose()?;
                evaluate_slice(
                    resolve_default_operand_value(base_value),
                    start_value.map(resolve_default_operand_value),
                    end_value.map(resolve_default_operand_value),
                )
            }
            SemanticExpr::Call { callee, args } => {
                let callee = *callee;
                let args = args.clone();
                self.evaluate_call(callee, &args, environment, depth + 1)?
            }
            SemanticExpr::Unary { op, expr } => {
                let value = self.evaluate_expr_at(*expr, environment, depth + 1)?;
                evaluate_unary(op, resolve_default_operand_value(value))
            }
            SemanticExpr::Binary { op, left, right } => {
                self.evaluate_binary_expr(op, *left, *right, environment, depth + 1)?
            }
            SemanticExpr::Default(expr) => EvaluatedValue::Default(Box::new(
                self.evaluate_expr_at(*expr, environment, depth + 1)?,
            )),
            SemanticExpr::InterpolatedString(segments) => {
                self.evaluate_interpolated_string(segments, environment, depth + 1)?
            }
            SemanticExpr::Comprehension(comprehension) => {
                self.evaluate_comprehension(comprehension, environment, depth + 1, false)?
            }
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
        members: &[StructMember],
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        let mut values = IndexMap::new();
        let mut patterns = Vec::new();
        let local_fields = static_local_fields(members);
        self.local_fields.push(local_fields);
        self.local_values.push(IndexMap::new());
        self.local_value_scopes.push(LocalValueScope::Struct);
        self.local_evaluating.push(HashSet::new());
        let result = (|| {
            for member in members {
                match member {
                    StructMember::Field(field) => {
                        self.evaluate_field_member(field, environment, depth, &mut values)?;
                    }
                    StructMember::Comprehension(comprehension) => {
                        let value =
                            self.evaluate_comprehension(comprehension, environment, depth, true)?;
                        Self::merge_struct_member_value(value, None, &mut values);
                    }
                    _ => {}
                }
            }
            for member in members {
                let StructMember::Field(field) = member else {
                    continue;
                };
                if matches!(&field.label, FieldLabel::Pattern(_))
                    && let Some(pattern) =
                        self.evaluate_pattern_field(field, environment, depth, &mut values)?
                {
                    patterns.push(pattern);
                }
            }
            apply_struct_patterns(&patterns, &mut values);
            if patterns.is_empty() {
                Ok(EvaluatedValue::Struct(values))
            } else {
                Ok(EvaluatedValue::PatternedStruct {
                    fields: values,
                    patterns,
                })
            }
        })();
        self.local_evaluating.pop();
        self.local_value_scopes.pop();
        self.local_values.pop();
        self.local_fields.pop();
        result
    }

    fn evaluate_field_member(
        &mut self,
        field: &FieldExpr,
        environment: EnvironmentId,
        depth: u32,
        values: &mut IndexMap<String, EvaluatedValue>,
    ) -> Result<(), EvalError> {
        if !self.should_emit_field(field.metadata) {
            return Ok(());
        }
        let label = match self.evaluate_field_label(&field.label, environment, depth)? {
            EvaluatedFieldLabel::Concrete(label) => label,
            EvaluatedFieldLabel::Pattern => return Ok(()),
            EvaluatedFieldLabel::Invalid(bottom) => {
                merge_field_value(
                    INVALID_DYNAMIC_LABEL_FIELD.to_owned(),
                    EvaluatedValue::Bottom(bottom),
                    field.span,
                    values,
                );
                return Ok(());
            }
        };
        let static_feature = match &field.label {
            FieldLabel::Static(feature) => Some(*feature),
            _ => None,
        };
        if let Some(feature) = static_feature {
            self.mark_local_feature_evaluating(feature);
            self.field_evaluation_stack.push(FieldEvaluation {
                scope_index: self.current_local_struct_scope_index(),
                feature,
            });
        }
        let value_result = self.evaluate_expr_at(field.expression, environment, depth);
        if let Some(feature) = static_feature {
            self.field_evaluation_stack.pop();
            self.unmark_local_feature_evaluating(feature);
        }
        let mut value = value_result?;
        if self.export_options.is_some() && is_optional_constraint(field.metadata) {
            value = EvaluatedValue::OptionalField(Box::new(value));
        }
        merge_field_value(label, value, field.span, values);
        if let Some(feature) = static_feature
            && !is_optional_constraint(field.metadata)
            && let Some(value) = values.get(&self.feature_label(feature)).cloned()
            && let Some(local_values) = self.local_values.last_mut()
        {
            local_values.insert(feature, value);
        }
        Ok(())
    }

    fn mark_local_feature_evaluating(&mut self, feature: Feature) {
        if let Some(features) = self.local_evaluating.last_mut() {
            features.insert(feature);
        }
    }

    fn unmark_local_feature_evaluating(&mut self, feature: Feature) {
        if let Some(features) = self.local_evaluating.last_mut() {
            features.remove(&feature);
        }
    }

    fn current_local_struct_scope_index(&self) -> usize {
        self.local_evaluating.len().saturating_sub(1)
    }

    fn evaluate_with_cycle_fallback<T>(
        &mut self,
        evaluate: impl FnOnce(&mut Self) -> Result<T, EvalError>,
    ) -> Result<T, EvalError> {
        self.cycle_fallback_to_top = self.cycle_fallback_to_top.saturating_add(1);
        let result = evaluate(self);
        self.cycle_fallback_to_top = self.cycle_fallback_to_top.saturating_sub(1);
        result
    }

    fn evaluate_field_label(
        &mut self,
        label: &FieldLabel,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedFieldLabel, EvalError> {
        match label {
            FieldLabel::Static(feature) => {
                Ok(EvaluatedFieldLabel::Concrete(self.feature_label(*feature)))
            }
            FieldLabel::Dynamic(expression) => {
                let value = self.evaluate_expr_at(*expression, environment, depth)?;
                Ok(dynamic_label_value(value))
            }
            FieldLabel::Pattern(_) => Ok(EvaluatedFieldLabel::Pattern),
            _ => Ok(EvaluatedFieldLabel::Invalid(Bottom::new(
                "cue.eval.unsupported_field_label",
                "unsupported field label",
                None,
                false,
            ))),
        }
    }

    fn evaluate_binary_expr(
        &mut self,
        op: &str,
        left: ExprId,
        right: ExprId,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        let left = if op == "&" {
            self.evaluate_cycle_fallback_operand(left, environment, depth)?
        } else {
            self.evaluate_expr_at(left, environment, depth)?
        };
        let right = if op == "&" {
            self.evaluate_cycle_fallback_operand(right, environment, depth)?
        } else {
            self.evaluate_expr_at(right, environment, depth)?
        };
        Ok(evaluate_binary_with_default_operands(op, left, right))
    }

    fn evaluate_cycle_fallback_operand(
        &mut self,
        expression: ExprId,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        let self_list_feature = if self.is_list_expr(expression)? {
            self.current_top_level_vertex_feature()?
        } else {
            None
        };
        if let Some(feature) = self_list_feature {
            self.vertex_list_self_index_features.push(feature);
        }
        let result = self.evaluate_with_cycle_fallback(|evaluator| {
            evaluator.evaluate_expr_at(expression, environment, depth)
        });
        if self_list_feature.is_some() {
            self.vertex_list_self_index_features.pop();
        }
        result
    }

    fn current_top_level_vertex_feature(&self) -> Result<Option<Feature>, EvalError> {
        if !self.field_evaluation_stack.is_empty() {
            return Ok(None);
        }
        let Some(vertex_id) = self.vertex_evaluation_stack.last().copied() else {
            return Ok(None);
        };
        Ok(self.runtime.vertex(vertex_id)?.feature)
    }

    fn evaluate_selector_expr(
        &mut self,
        base: ExprId,
        feature: Feature,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        if let Some(value) =
            self.evaluate_selector_from_in_progress_vertex(base, feature, environment, depth)?
        {
            return Ok(value);
        }
        let base_value = self.evaluate_expr_at(base, environment, depth)?;
        Ok(self.select_field(base_value, feature))
    }

    fn evaluate_selector_from_in_progress_vertex(
        &mut self,
        base: ExprId,
        feature: Feature,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<Option<EvaluatedValue>, EvalError> {
        let base = self.skip_group_exprs(base)?;
        let SemanticExpr::FieldReference {
            feature: base_feature,
            up_count,
        } = self.runtime.expression(base)?
        else {
            return Ok(None);
        };
        if *up_count == 0 && self.has_local_reference_candidate(*base_feature) {
            return Ok(None);
        }
        let Some(vertex_id) =
            self.field_reference_target_vertex(environment, *base_feature, *up_count)?
        else {
            return Ok(None);
        };
        if !self.evaluating_vertices.contains(&vertex_id) {
            return Ok(None);
        }
        let Some(value) = self.evaluate_in_progress_vertex_field(vertex_id, feature, depth)? else {
            let label = self.feature_label(feature);
            return Ok(Some(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.missing_field",
                format!("field `{label}` does not exist"),
                None,
                true,
            ))));
        };
        Ok(Some(value))
    }

    fn evaluate_in_progress_vertex_field(
        &mut self,
        vertex_id: VertexId,
        feature: Feature,
        depth: u32,
    ) -> Result<Option<EvaluatedValue>, EvalError> {
        if self
            .vertex_evaluation_stack
            .last()
            .is_some_and(|current| *current == vertex_id)
            && self
                .field_evaluation_stack
                .last()
                .is_some_and(|current| current.feature == feature)
        {
            return Ok(Some(EvaluatedValue::Top));
        }

        let mut value = None;
        let target = self.runtime.vertex(vertex_id)?;
        if let Some(arc) = target.arcs.get(&feature).cloned() {
            if is_optional_constraint(arc.metadata) {
                let label = self.feature_label(feature);
                value = Some(optional_reference_bottom(&label));
            } else {
                value = Some(self.evaluate_vertex_at(arc.target, depth + 1)?);
            }
        }

        let fields = self.in_progress_struct_field_exprs(vertex_id, feature)?;
        for field in fields {
            let next = if is_optional_constraint(field.metadata) {
                let label = self.feature_label(feature);
                optional_reference_bottom(&label)
            } else {
                self.evaluate_in_progress_static_field(feature, &field, depth + 1)?
            };
            value = Some(match value {
                Some(value) => unify_values(value, next, field.span),
                None => next,
            });
        }
        if let Some(generated) =
            self.evaluate_in_progress_generated_field(vertex_id, feature, depth + 1)?
        {
            value = Some(match value {
                Some(value) => unify_values(value, generated, None),
                None => generated,
            });
        }
        value
            .map(|value| self.apply_in_progress_patterns_to_field(vertex_id, feature, value, depth))
            .transpose()
    }

    fn in_progress_struct_field_exprs(
        &self,
        vertex_id: VertexId,
        feature: Feature,
    ) -> Result<Vec<InProgressField>, EvalError> {
        let mut fields = Vec::new();
        let target = self.runtime.vertex(vertex_id)?;
        for conjunct_id in &target.conjuncts {
            let conjunct = self.runtime.conjunct(*conjunct_id)?;
            self.collect_in_progress_struct_field_exprs(
                conjunct.expression,
                conjunct.environment,
                conjunct.span,
                feature,
                &mut fields,
            )?;
        }
        Ok(fields)
    }

    fn collect_in_progress_struct_field_exprs(
        &self,
        expression: ExprId,
        environment: EnvironmentId,
        span: Option<Span>,
        feature: Feature,
        fields: &mut Vec<InProgressField>,
    ) -> Result<(), EvalError> {
        let expression = self.skip_group_exprs(expression)?;
        match self.runtime.expression(expression)? {
            SemanticExpr::Struct(members) => {
                let local_fields = static_local_fields(members);
                for member in members {
                    let StructMember::Field(field) = member else {
                        continue;
                    };
                    if field.label == FieldLabel::Static(feature) {
                        fields.push(InProgressField {
                            expression: field.expression,
                            environment,
                            span: field.span.or(span),
                            metadata: field.metadata,
                            local_fields: local_fields.clone(),
                        });
                    }
                }
            }
            SemanticExpr::Binary { op, left, right } if op == "&" => {
                self.collect_in_progress_struct_field_exprs(
                    *left,
                    environment,
                    span,
                    feature,
                    fields,
                )?;
                self.collect_in_progress_struct_field_exprs(
                    *right,
                    environment,
                    span,
                    feature,
                    fields,
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    fn evaluate_in_progress_static_field(
        &mut self,
        feature: Feature,
        field: &InProgressField,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        self.local_fields.push(field.local_fields.clone());
        self.local_values.push(IndexMap::new());
        self.local_value_scopes.push(LocalValueScope::Struct);
        self.local_evaluating.push(HashSet::new());
        self.mark_local_feature_evaluating(feature);
        self.field_evaluation_stack.push(FieldEvaluation {
            scope_index: self.current_local_struct_scope_index(),
            feature,
        });
        let result = self.evaluate_expr_at(field.expression, field.environment, depth);
        self.field_evaluation_stack.pop();
        self.unmark_local_feature_evaluating(feature);
        self.local_evaluating.pop();
        self.local_value_scopes.pop();
        self.local_values.pop();
        self.local_fields.pop();
        result
    }

    fn evaluate_in_progress_generated_field(
        &mut self,
        vertex_id: VertexId,
        feature: Feature,
        depth: u32,
    ) -> Result<Option<EvaluatedValue>, EvalError> {
        let conjuncts = self.runtime.vertex(vertex_id)?.conjuncts.clone();
        let mut value = None;
        for conjunct_id in conjuncts {
            let conjunct = self.runtime.conjunct(conjunct_id)?;
            let next = self.evaluate_in_progress_generated_field_expr(
                conjunct.expression,
                conjunct.environment,
                conjunct.span,
                feature,
                depth,
            )?;
            if let Some(next) = next {
                value = Some(match value {
                    Some(value) => unify_values(value, next, conjunct.span),
                    None => next,
                });
            }
        }
        Ok(value)
    }

    fn evaluate_in_progress_generated_field_expr(
        &mut self,
        expression: ExprId,
        environment: EnvironmentId,
        span: Option<Span>,
        feature: Feature,
        depth: u32,
    ) -> Result<Option<EvaluatedValue>, EvalError> {
        let expression = self.skip_group_exprs(expression)?;
        match self.runtime.expression(expression)?.clone() {
            SemanticExpr::Struct(members) => self.evaluate_in_progress_generated_struct_fields(
                &members,
                environment,
                feature,
                depth + 1,
            ),
            SemanticExpr::Binary { op, left, right } if op == "&" => {
                let left = self.evaluate_in_progress_generated_field_expr(
                    left,
                    environment,
                    span,
                    feature,
                    depth + 1,
                )?;
                let right = self.evaluate_in_progress_generated_field_expr(
                    right,
                    environment,
                    span,
                    feature,
                    depth + 1,
                )?;
                Ok(match (left, right) {
                    (Some(left), Some(right)) => Some(unify_values(left, right, span)),
                    (Some(value), None) | (None, Some(value)) => Some(value),
                    (None, None) => None,
                })
            }
            _ => Ok(None),
        }
    }

    fn evaluate_in_progress_generated_struct_fields(
        &mut self,
        members: &[StructMember],
        environment: EnvironmentId,
        feature: Feature,
        depth: u32,
    ) -> Result<Option<EvaluatedValue>, EvalError> {
        self.local_fields.push(static_local_fields(members));
        self.local_values.push(IndexMap::new());
        self.local_value_scopes.push(LocalValueScope::Struct);
        self.local_evaluating.push(HashSet::new());
        let result = (|| {
            let mut values = IndexMap::new();
            for member in members {
                match member {
                    StructMember::Field(field)
                        if !matches!(
                            field.label,
                            FieldLabel::Static(_) | FieldLabel::Pattern(_)
                        ) =>
                    {
                        self.evaluate_field_member(field, environment, depth, &mut values)?;
                    }
                    StructMember::Comprehension(comprehension) => {
                        let value =
                            self.evaluate_comprehension(comprehension, environment, depth, true)?;
                        Self::merge_struct_member_value(value, None, &mut values);
                    }
                    _ => {}
                }
            }
            Ok(values.get(&self.feature_label(feature)).cloned())
        })();
        self.local_evaluating.pop();
        self.local_value_scopes.pop();
        self.local_values.pop();
        self.local_fields.pop();
        result
    }

    fn apply_in_progress_patterns_to_field(
        &mut self,
        vertex_id: VertexId,
        feature: Feature,
        value: EvaluatedValue,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        let patterns =
            self.evaluate_in_progress_pattern_constraints(vertex_id, feature, &value, depth + 1)?;
        let label = self.feature_label(feature);
        let mut value = value;
        for pattern in patterns {
            if let Ok(true) = pattern_label_matches(&pattern.pattern, &label) {
                value = unify_field_values(value, (*pattern.value).clone(), pattern.span);
            }
        }
        Ok(value)
    }

    fn evaluate_in_progress_pattern_constraints(
        &mut self,
        vertex_id: VertexId,
        feature: Feature,
        value: &EvaluatedValue,
        depth: u32,
    ) -> Result<Vec<StructPatternConstraint>, EvalError> {
        let conjuncts = self.runtime.vertex(vertex_id)?.conjuncts.clone();
        let mut patterns = Vec::new();
        for conjunct_id in conjuncts {
            let conjunct = self.runtime.conjunct(conjunct_id)?;
            self.collect_in_progress_pattern_constraints(
                conjunct.expression,
                conjunct.environment,
                feature,
                value,
                depth,
                &mut patterns,
            )?;
        }
        Ok(patterns)
    }

    fn collect_in_progress_pattern_constraints(
        &mut self,
        expression: ExprId,
        environment: EnvironmentId,
        feature: Feature,
        value: &EvaluatedValue,
        depth: u32,
        patterns: &mut Vec<StructPatternConstraint>,
    ) -> Result<(), EvalError> {
        let expression = self.skip_group_exprs(expression)?;
        match self.runtime.expression(expression)?.clone() {
            SemanticExpr::Struct(members) => self.collect_in_progress_struct_patterns(
                &members,
                environment,
                feature,
                value,
                depth + 1,
                patterns,
            ),
            SemanticExpr::Binary { op, left, right } if op == "&" => {
                self.collect_in_progress_pattern_constraints(
                    left,
                    environment,
                    feature,
                    value,
                    depth + 1,
                    patterns,
                )?;
                self.collect_in_progress_pattern_constraints(
                    right,
                    environment,
                    feature,
                    value,
                    depth + 1,
                    patterns,
                )
            }
            _ => Ok(()),
        }
    }

    fn collect_in_progress_struct_patterns(
        &mut self,
        members: &[StructMember],
        environment: EnvironmentId,
        feature: Feature,
        value: &EvaluatedValue,
        depth: u32,
        patterns: &mut Vec<StructPatternConstraint>,
    ) -> Result<(), EvalError> {
        let mut local_values = IndexMap::new();
        local_values.insert(feature, value.clone());
        self.local_fields.push(static_local_fields(members));
        self.local_values.push(local_values);
        self.local_value_scopes.push(LocalValueScope::Struct);
        self.local_evaluating.push(HashSet::new());
        let result = (|| {
            let mut invalid_pattern_values = IndexMap::new();
            for member in members {
                let StructMember::Field(field) = member else {
                    continue;
                };
                if matches!(&field.label, FieldLabel::Pattern(_))
                    && let Some(pattern) = self.evaluate_pattern_field(
                        field,
                        environment,
                        depth,
                        &mut invalid_pattern_values,
                    )?
                {
                    patterns.push(pattern);
                }
            }
            Ok(())
        })();
        self.local_evaluating.pop();
        self.local_value_scopes.pop();
        self.local_values.pop();
        self.local_fields.pop();
        result
    }

    fn evaluate_list_item(
        &mut self,
        item: ExprId,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        if self.is_direct_self_list_index(item)? {
            self.defer_list_self_indexes = self.defer_list_self_indexes.saturating_add(1);
            let value = self.evaluate_expr_at(item, environment, depth);
            self.defer_list_self_indexes = self.defer_list_self_indexes.saturating_sub(1);
            return value;
        }
        self.evaluate_expr_at(item, environment, depth)
    }

    fn is_direct_self_list_index(&self, item: ExprId) -> Result<bool, EvalError> {
        let item = self.skip_group_exprs(item)?;
        let SemanticExpr::Index { base, .. } = self.runtime.expression(item)? else {
            return Ok(false);
        };
        let base = self.skip_group_exprs(*base)?;
        let SemanticExpr::FieldReference { feature, up_count } = self.runtime.expression(base)?
        else {
            return Ok(false);
        };
        Ok(*up_count == 0
            && (self
                .field_evaluation_stack
                .last()
                .is_some_and(|current| current.feature == *feature)
                || self
                    .vertex_list_self_index_features
                    .last()
                    .is_some_and(|current| current == feature)))
    }

    fn skip_group_exprs(&self, expression: ExprId) -> Result<ExprId, EvalError> {
        let mut current = expression;
        loop {
            let SemanticExpr::Unary { op, expr } = self.runtime.expression(current)? else {
                return Ok(current);
            };
            if op != "group" {
                return Ok(current);
            }
            current = *expr;
        }
    }

    fn is_list_expr(&self, expression: ExprId) -> Result<bool, EvalError> {
        let expression = self.skip_group_exprs(expression)?;
        Ok(matches!(
            self.runtime.expression(expression)?,
            SemanticExpr::List { .. }
        ))
    }

    fn evaluate_list_expr(
        &mut self,
        items: &[ExprId],
        tail: Option<ExprId>,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            match self.evaluate_list_item(*item, environment, depth)? {
                EvaluatedValue::ComprehensionItems(items) => values.extend(items),
                value => values.push(value),
            }
        }
        let tail_value = tail
            .map(|tail| self.evaluate_expr_at(tail, environment, depth))
            .transpose()?;
        resolve_deferred_list_indexes(&mut values, tail_value.as_ref());
        Ok(if tail.is_some() {
            EvaluatedValue::OpenList {
                items: values,
                tail: Box::new(tail_value.unwrap_or_else(|| {
                    EvaluatedValue::Bottom(Bottom::new(
                        "cue.eval.invalid_open_list",
                        "open list tail did not evaluate",
                        None,
                        false,
                    ))
                })),
            }
        } else {
            EvaluatedValue::List(values)
        })
    }

    fn evaluate_pattern_field(
        &mut self,
        field: &FieldExpr,
        environment: EnvironmentId,
        depth: u32,
        values: &mut IndexMap<String, EvaluatedValue>,
    ) -> Result<Option<StructPatternConstraint>, EvalError> {
        if !self.should_emit_field(field.metadata) {
            return Ok(None);
        }
        let FieldLabel::Pattern(pattern) = &field.label else {
            return Ok(None);
        };
        let pattern = self.evaluate_expr_at(*pattern, environment, depth + 1)?;
        if let EvaluatedValue::Bottom(bottom) = pattern {
            merge_field_value(
                INVALID_PATTERN_LABEL_FIELD.to_owned(),
                EvaluatedValue::Bottom(bottom),
                field.span,
                values,
            );
            return Ok(None);
        }
        let value = self.evaluate_expr_at(field.expression, environment, depth + 1)?;
        Ok(Some(StructPatternConstraint {
            pattern: Box::new(pattern),
            value: Box::new(value),
            span: field.span,
        }))
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

        if matches!(
            name.as_str(),
            "list.Sort" | "list.SortStable" | "list.IsSorted"
        ) {
            return self.evaluate_list_schema_call(&name, args, environment, depth + 1);
        }

        let mut evaluated_args = Vec::with_capacity(args.len());
        for arg in args {
            evaluated_args.push(self.evaluate_expr_at(*arg, environment, depth + 1)?);
        }
        Ok(evaluate_builtin_call(&name, evaluated_args))
    }

    fn evaluate_list_schema_call(
        &mut self,
        name: &str,
        args: &[ExprId],
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        if args.len() != 2 {
            return Ok(invalid_list_builtin_arg(name));
        }
        let Some(list_expr) = args.first().copied() else {
            return Ok(invalid_list_builtin_arg(name));
        };
        let Some(comparator_expr) = args.get(1).copied() else {
            return Ok(invalid_list_builtin_arg(name));
        };
        let list_value = self.evaluate_expr_at(list_expr, environment, depth + 1)?;
        let EvaluatedValue::List(items) = resolve_default_operand_value(list_value) else {
            return Ok(invalid_list_builtin_arg(name));
        };
        if let Some(direction) =
            self.evaluate_sort_direction(comparator_expr, environment, depth + 1)?
        {
            return Ok(match name {
                "list.Sort" | "list.SortStable" => sort_with_direction(items, direction, true),
                "list.IsSorted" => is_sorted_with_direction(&items, direction),
                _ => invalid_list_builtin_arg(name),
            });
        }
        Ok(match name {
            "list.Sort" | "list.SortStable" => {
                self.sort_with_schema(&items, comparator_expr, environment, depth, true)?
            }
            "list.IsSorted" => {
                self.is_sorted_with_schema(&items, comparator_expr, environment, depth)?
            }
            _ => invalid_list_builtin_arg(name),
        })
    }

    fn evaluate_sort_direction(
        &mut self,
        comparator: ExprId,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<Option<SortDirection>, EvalError> {
        let comparator = self.evaluate_expr_at(comparator, environment, depth + 1)?;
        match resolve_default_operand_value(comparator) {
            EvaluatedValue::Builtin(name) if name == "list.Ascending" => {
                Ok(Some(SortDirection::Ascending))
            }
            EvaluatedValue::Builtin(name) if name == "list.Descending" => {
                Ok(Some(SortDirection::Descending))
            }
            _ => Ok(None),
        }
    }

    fn sort_with_schema(
        &mut self,
        items: &[EvaluatedValue],
        comparator: ExprId,
        environment: EnvironmentId,
        depth: u32,
        stable: bool,
    ) -> Result<EvaluatedValue, EvalError> {
        if items.len() > MAX_SCHEMA_SORT_ITEMS {
            return Ok(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.sort_limit",
                "schema sort item limit exceeded",
                None,
                false,
            )));
        }
        let mut sorted = items.to_vec();
        let mut bottom = None;
        let mut error = None;
        let mut compare = |left: &EvaluatedValue, right: &EvaluatedValue| {
            if bottom.is_some() || error.is_some() {
                return Ordering::Equal;
            }
            match self.compare_with_schema(comparator, environment, depth + 1, left, right) {
                Ok(Ok(ordering)) => ordering,
                Ok(Err(failure)) => {
                    bottom = Some(failure);
                    Ordering::Equal
                }
                Err(failure) => {
                    error = Some(failure);
                    Ordering::Equal
                }
            }
        };
        if stable {
            sorted.sort_by(&mut compare);
        } else {
            sorted.sort_unstable_by(&mut compare);
        }
        if let Some(error) = error {
            return Err(error);
        }
        if let Some(bottom) = bottom {
            return Ok(EvaluatedValue::Bottom(bottom));
        }
        Ok(EvaluatedValue::List(sorted))
    }

    fn is_sorted_with_schema(
        &mut self,
        items: &[EvaluatedValue],
        comparator: ExprId,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        for window in items.windows(2) {
            let Some(left) = window.first() else {
                continue;
            };
            let Some(right) = window.get(1) else {
                continue;
            };
            match self.compare_with_schema(comparator, environment, depth + 1, left, right)? {
                Ok(Ordering::Greater) => return Ok(EvaluatedValue::Bool(false)),
                Ok(_) => {}
                Err(bottom) => return Ok(EvaluatedValue::Bottom(bottom)),
            }
        }
        Ok(EvaluatedValue::Bool(true))
    }

    fn compare_with_schema(
        &mut self,
        comparator: ExprId,
        environment: EnvironmentId,
        depth: u32,
        left: &EvaluatedValue,
        right: &EvaluatedValue,
    ) -> Result<Result<Ordering, Bottom>, EvalError> {
        let less_lr = self.evaluate_schema_less(comparator, environment, depth, left, right)?;
        let less_rl = self.evaluate_schema_less(comparator, environment, depth, right, left)?;
        Ok(match (less_lr, less_rl) {
            (EvaluatedValue::Bottom(bottom), _) | (_, EvaluatedValue::Bottom(bottom)) => {
                Err(bottom)
            }
            (EvaluatedValue::Bool(left), EvaluatedValue::Bool(right)) => Ok(match (left, right) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => Ordering::Equal,
            }),
            (left, _) => Err(Bottom::new(
                "cue.eval.invalid_sort_comparator",
                format!(
                    "sort comparator less field must evaluate to bool, got {}",
                    left.kind()
                ),
                None,
                false,
            )),
        })
    }

    fn evaluate_schema_less(
        &mut self,
        comparator: ExprId,
        environment: EnvironmentId,
        depth: u32,
        left: &EvaluatedValue,
        right: &EvaluatedValue,
    ) -> Result<EvaluatedValue, EvalError> {
        let comparator = self.resolve_comparator_schema(comparator, environment)?;
        let SemanticExpr::Struct(members) = self.runtime.expression(comparator)? else {
            return Ok(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.invalid_sort_comparator",
                "sort comparator must be a struct with x, y, and less fields",
                None,
                false,
            )));
        };
        let mut values = IndexMap::new();
        let mut has_x = false;
        let mut has_y = false;
        let mut less = None;
        for member in members {
            let StructMember::Field(field) = member else {
                continue;
            };
            let FieldLabel::Static(feature) = &field.label else {
                continue;
            };
            let label = self.feature_label(*feature);
            match label.as_str() {
                "x" => {
                    has_x = true;
                    let schema = self.evaluate_expr_at(field.expression, environment, depth + 1)?;
                    let existing = values.shift_remove(feature).unwrap_or_else(|| left.clone());
                    let value = unify_values(existing, schema, field.span);
                    if let EvaluatedValue::Bottom(bottom) = value {
                        return Ok(EvaluatedValue::Bottom(bottom));
                    }
                    values.insert(*feature, value);
                }
                "y" => {
                    has_y = true;
                    let schema = self.evaluate_expr_at(field.expression, environment, depth + 1)?;
                    let existing = values
                        .shift_remove(feature)
                        .unwrap_or_else(|| right.clone());
                    let value = unify_values(existing, schema, field.span);
                    if let EvaluatedValue::Bottom(bottom) = value {
                        return Ok(EvaluatedValue::Bottom(bottom));
                    }
                    values.insert(*feature, value);
                }
                "less" => less = Some(field.expression),
                _ => {}
            }
        }
        if !has_x || !has_y {
            return Ok(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.invalid_sort_comparator",
                "sort comparator must define x and y fields",
                None,
                false,
            )));
        }
        let Some(less) = less else {
            return Ok(EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.invalid_sort_comparator",
                "sort comparator must define a less field",
                None,
                false,
            )));
        };
        self.local_values.push(values);
        self.local_value_scopes.push(LocalValueScope::Overlay);
        let evaluated = self.evaluate_expr_at(less, environment, depth + 1);
        self.local_value_scopes.pop();
        self.local_values.pop();
        evaluated
    }

    fn resolve_comparator_schema(
        &self,
        comparator: ExprId,
        environment: EnvironmentId,
    ) -> Result<ExprId, EvalError> {
        const MAX_COMPARATOR_SCHEMA_RESOLUTION_DEPTH: usize = 32;
        let mut current = comparator;
        let mut seen = Vec::new();
        for _ in 0..MAX_COMPARATOR_SCHEMA_RESOLUTION_DEPTH {
            if seen.contains(&current) {
                return Ok(current);
            }
            seen.push(current);
            match self.runtime.expression(current)? {
                SemanticExpr::LetReference { expression } => current = *expression,
                SemanticExpr::FieldReference { feature, up_count } => {
                    let Some(next) =
                        self.resolve_field_expression(*feature, *up_count, environment)?
                    else {
                        return Ok(current);
                    };
                    current = next;
                }
                _ => return Ok(current),
            }
        }
        Ok(current)
    }

    fn resolve_field_expression(
        &self,
        feature: Feature,
        up_count: u32,
        environment: EnvironmentId,
    ) -> Result<Option<ExprId>, EvalError> {
        if up_count == 0
            && let Some(field) = self
                .local_fields
                .iter()
                .rev()
                .find_map(|fields| fields.get(&feature).copied())
        {
            return Ok(Some(field.expression));
        }

        let mut environment_id = environment;
        for _ in 0..up_count {
            let environment = self.runtime.environment(environment_id)?;
            let Some(parent) = environment.parent else {
                return Ok(None);
            };
            environment_id = parent;
        }

        let environment = self.runtime.environment(environment_id)?;
        let vertex = self.runtime.vertex(environment.vertex)?;
        let Some(arc) = vertex.arcs.get(&feature) else {
            return Ok(None);
        };
        let target = self.runtime.vertex(arc.target)?;
        let Some(conjunct_id) = target.conjuncts.first().copied() else {
            return Ok(None);
        };
        Ok(Some(self.runtime.conjunct(conjunct_id)?.expression))
    }

    fn evaluate_field_reference(
        &mut self,
        environment: EnvironmentId,
        feature: Feature,
        up_count: u32,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        if up_count == 0
            && let Some(value) = self.evaluate_local_field_reference(feature, environment, depth)?
        {
            return Ok(value);
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

    fn field_reference_target_vertex(
        &self,
        environment: EnvironmentId,
        feature: Feature,
        up_count: u32,
    ) -> Result<Option<VertexId>, EvalError> {
        let mut environment_id = environment;
        for _ in 0..up_count {
            let environment = self.runtime.environment(environment_id)?;
            let Some(parent) = environment.parent else {
                return Ok(None);
            };
            environment_id = parent;
        }

        let environment = self.runtime.environment(environment_id)?;
        let vertex = self.runtime.vertex(environment.vertex)?;
        Ok(vertex.arcs.get(&feature).map(|arc| arc.target))
    }

    fn has_local_reference_candidate(&self, feature: Feature) -> bool {
        let mut struct_scope_index = self.local_fields.len();
        for (scope, values) in self
            .local_value_scopes
            .iter()
            .zip(self.local_values.iter())
            .rev()
        {
            match scope {
                LocalValueScope::Overlay => {
                    if values.contains_key(&feature) {
                        return true;
                    }
                }
                LocalValueScope::Struct => {
                    struct_scope_index = struct_scope_index.saturating_sub(1);
                    if values.contains_key(&feature)
                        || self
                            .local_fields
                            .get(struct_scope_index)
                            .is_some_and(|fields| fields.contains_key(&feature))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn evaluate_local_field_reference(
        &mut self,
        feature: Feature,
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<Option<EvaluatedValue>, EvalError> {
        let mut struct_scope_index = self.local_fields.len();
        for (scope, values) in self
            .local_value_scopes
            .iter()
            .zip(self.local_values.iter())
            .rev()
        {
            match scope {
                LocalValueScope::Overlay => {
                    if let Some(value) = values.get(&feature) {
                        return Ok(Some(value.clone()));
                    }
                }
                LocalValueScope::Struct => {
                    struct_scope_index = struct_scope_index.saturating_sub(1);
                    let Some(fields) = self.local_fields.get(struct_scope_index) else {
                        continue;
                    };
                    let Some(field) = fields.get(&feature).copied() else {
                        continue;
                    };
                    if let Some(value) = values.get(&feature) {
                        return Ok(Some(value.clone()));
                    }
                    if self
                        .local_evaluating
                        .get(struct_scope_index)
                        .is_some_and(|features| features.contains(&feature))
                    {
                        if self.defer_list_self_indexes > 0
                            && self.is_current_local_field(struct_scope_index, feature)
                        {
                            return Ok(Some(EvaluatedValue::Bottom(structural_cycle_bottom())));
                        }
                        if self.cycle_fallback_to_top > 0 {
                            return Ok(Some(EvaluatedValue::Top));
                        }
                        return Ok(Some(
                            if self.is_structural_ancestor_cycle(struct_scope_index) {
                                EvaluatedValue::Bottom(structural_cycle_bottom())
                            } else {
                                EvaluatedValue::Top
                            },
                        ));
                    }
                    if is_optional_constraint(field.metadata) {
                        let label = self.feature_label(feature);
                        return Ok(Some(optional_reference_bottom(&label)));
                    }
                    return self
                        .evaluate_expr_at(field.expression, environment, depth + 1)
                        .map(Some);
                }
            }
        }
        Ok(None)
    }

    fn is_current_local_field(&self, scope_index: usize, feature: Feature) -> bool {
        self.field_evaluation_stack
            .last()
            .is_some_and(|current| current.scope_index == scope_index && current.feature == feature)
    }

    fn is_structural_ancestor_cycle(&self, scope_index: usize) -> bool {
        self.field_evaluation_stack
            .last()
            .is_some_and(|current| current.scope_index > scope_index)
    }

    fn evaluate_dynamic_reference(&self, name: &str) -> EvaluatedValue {
        self.local_bindings
            .iter()
            .rev()
            .find_map(|bindings| bindings.get(name).cloned())
            .unwrap_or_else(|| {
                EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.missing_binding",
                    format!("comprehension binding `{name}` does not exist"),
                    None,
                    true,
                ))
            })
    }

    fn evaluate_interpolated_string(
        &mut self,
        segments: &[StringSegment],
        environment: EnvironmentId,
        depth: u32,
    ) -> Result<EvaluatedValue, EvalError> {
        let mut output = String::new();
        for segment in segments {
            match segment {
                StringSegment::Text(value) => output.push_str(value),
                StringSegment::Expr(expression) => {
                    let value = self.evaluate_expr_at(*expression, environment, depth + 1)?;
                    let Some(rendered) = interpolation_value(value) else {
                        return Ok(EvaluatedValue::Bottom(Bottom::new(
                            "cue.eval.invalid_interpolation",
                            "string interpolation requires concrete scalar values",
                            None,
                            false,
                        )));
                    };
                    output.push_str(&rendered);
                }
                _ => {}
            }
        }
        Ok(EvaluatedValue::String(output))
    }

    fn evaluate_comprehension(
        &mut self,
        comprehension: &Comprehension,
        environment: EnvironmentId,
        depth: u32,
        merge_struct_results: bool,
    ) -> Result<EvaluatedValue, EvalError> {
        let mut results = Vec::new();
        self.evaluate_comprehension_clause(comprehension, environment, depth, 0, &mut results)?;
        let body_is_struct = matches!(
            self.runtime.expression(comprehension.body)?,
            SemanticExpr::Struct(_)
        );
        if merge_struct_results
            && body_is_struct
            && results.iter().all(|value| {
                matches!(
                    value,
                    EvaluatedValue::Struct(_)
                        | EvaluatedValue::PatternedStruct { .. }
                        | EvaluatedValue::ClosedStruct(_)
                        | EvaluatedValue::ClosedPatternedStruct { .. }
                )
            })
        {
            let mut fields = IndexMap::new();
            let mut patterns = Vec::new();
            for value in results {
                if let EvaluatedValue::PatternedStruct {
                    patterns: new_patterns,
                    ..
                } = &value
                {
                    patterns.extend(new_patterns.clone());
                }
                Self::merge_struct_member_value(value, None, &mut fields);
            }
            apply_struct_patterns(&patterns, &mut fields);
            if patterns.is_empty() {
                return Ok(EvaluatedValue::Struct(fields));
            }
            return Ok(EvaluatedValue::PatternedStruct { fields, patterns });
        }
        Ok(EvaluatedValue::ComprehensionItems(results))
    }

    fn evaluate_comprehension_clause(
        &mut self,
        comprehension: &Comprehension,
        environment: EnvironmentId,
        depth: u32,
        clause_index: usize,
        results: &mut Vec<EvaluatedValue>,
    ) -> Result<ComprehensionControl, EvalError> {
        let Some(clause) = comprehension.clauses.get(clause_index) else {
            if results.len() >= MAX_COMPREHENSION_GENERATED_ITEMS {
                results.push(EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.comprehension_limit",
                    "comprehension generated item limit exceeded",
                    None,
                    false,
                )));
                return Ok(ComprehensionControl::Stop);
            }
            results.push(self.evaluate_expr_at(comprehension.body, environment, depth + 1)?);
            return Ok(ComprehensionControl::Continue);
        };
        match clause {
            ComprehensionClause::If { condition } => {
                let condition = self.evaluate_expr_at(*condition, environment, depth + 1)?;
                match resolve_default_operand_value(condition) {
                    EvaluatedValue::Bool(true) => {
                        return self.evaluate_comprehension_clause(
                            comprehension,
                            environment,
                            depth + 1,
                            clause_index.saturating_add(1),
                            results,
                        );
                    }
                    EvaluatedValue::Bool(false) => {}
                    EvaluatedValue::Bottom(bottom) => {
                        results.push(EvaluatedValue::Bottom(bottom));
                        return Ok(ComprehensionControl::Stop);
                    }
                    _ => {
                        results.push(EvaluatedValue::Bottom(Bottom::new(
                            "cue.eval.invalid_comprehension_if",
                            "if comprehension condition must be bool",
                            None,
                            false,
                        )));
                        return Ok(ComprehensionControl::Stop);
                    }
                }
            }
            ComprehensionClause::For { key, value, source } => {
                let source = self.evaluate_expr_at(*source, environment, depth + 1)?;
                let source = resolve_default_operand_value(source);
                let items = match comprehension_items(source) {
                    Ok(items) => items,
                    Err(bottom) => {
                        results.push(EvaluatedValue::Bottom(bottom));
                        return Ok(ComprehensionControl::Stop);
                    }
                };
                for (key_value, item) in items {
                    let mut bindings = IndexMap::new();
                    if let Some(key) = key {
                        bindings.insert(key.clone(), key_value);
                    }
                    bindings.insert(value.clone(), item);
                    self.local_bindings.push(bindings);
                    let control = self.evaluate_comprehension_clause(
                        comprehension,
                        environment,
                        depth + 1,
                        clause_index.saturating_add(1),
                        results,
                    )?;
                    self.local_bindings.pop();
                    if control == ComprehensionControl::Stop {
                        return Ok(ComprehensionControl::Stop);
                    }
                }
            }
            _ => {}
        }
        Ok(ComprehensionControl::Continue)
    }

    fn merge_struct_member_value(
        value: EvaluatedValue,
        span: Option<Span>,
        fields: &mut IndexMap<String, EvaluatedValue>,
    ) {
        let (EvaluatedValue::Struct(new_fields)
        | EvaluatedValue::PatternedStruct {
            fields: new_fields, ..
        }
        | EvaluatedValue::ClosedStruct(new_fields)
        | EvaluatedValue::ClosedPatternedStruct {
            fields: new_fields, ..
        }) = value
        else {
            return;
        };
        for (label, value) in new_fields {
            if let Some(existing) = fields.shift_remove(&label) {
                fields.insert(label, unify_values(existing, value, span));
            } else {
                fields.insert(label, value);
            }
        }
    }

    fn select_field(&self, base: EvaluatedValue, feature: Feature) -> EvaluatedValue {
        let label = self.feature_label(feature);
        match resolve_default_operand_value(base) {
            EvaluatedValue::Struct(fields)
            | EvaluatedValue::PatternedStruct { fields, .. }
            | EvaluatedValue::ClosedStruct(fields)
            | EvaluatedValue::ClosedPatternedStruct { fields, .. } => {
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

fn structural_cycle_bottom() -> Bottom {
    Bottom::new("cue.eval.structural_cycle", "structural cycle", None, false)
}

fn deferred_list_index(index: EvaluatedValue) -> EvaluatedValue {
    let EvaluatedValue::Number(index) = index else {
        return invalid_list_index_type();
    };
    let Some(index) = parse_list_index(&index) else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_index",
            format!("invalid list index `{index}`"),
            None,
            false,
        ));
    };
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.deferred_list_index",
        index.to_string(),
        None,
        true,
    ))
}

fn merge_field_value(
    label: String,
    value: EvaluatedValue,
    span: Option<Span>,
    values: &mut IndexMap<String, EvaluatedValue>,
) {
    if let Some(existing) = values.shift_remove(&label) {
        values.insert(label, unify_values(existing, value, span));
    } else {
        values.insert(label, value);
    }
}

fn apply_struct_patterns(
    patterns: &[StructPatternConstraint],
    values: &mut IndexMap<String, EvaluatedValue>,
) {
    for pattern in patterns {
        let labels = values.keys().cloned().collect::<Vec<_>>();
        for label in labels {
            match pattern_label_matches(&pattern.pattern, &label) {
                Ok(true) => {
                    let Some(existing) = values.shift_remove(&label) else {
                        continue;
                    };
                    values.insert(
                        label,
                        unify_field_values(existing, (*pattern.value).clone(), pattern.span),
                    );
                }
                Ok(false) => {}
                Err(bottom) => {
                    merge_field_value(
                        INVALID_PATTERN_LABEL_FIELD.to_owned(),
                        EvaluatedValue::Bottom(bottom),
                        pattern.span,
                        values,
                    );
                    return;
                }
            }
        }
    }
}

fn pattern_value_for_label(
    patterns: &[StructPatternConstraint],
    label: &str,
) -> Result<Option<EvaluatedValue>, Bottom> {
    let mut value = None;
    for pattern in patterns {
        if pattern_label_matches(&pattern.pattern, label)? {
            let next = (*pattern.value).clone();
            value = Some(match value {
                Some(existing) => unify_field_values(existing, next, pattern.span),
                None => next,
            });
        }
    }
    Ok(value)
}

fn dynamic_label_value(value: EvaluatedValue) -> EvaluatedFieldLabel {
    match resolve_default_value(value) {
        EvaluatedValue::String(value) => EvaluatedFieldLabel::Concrete(value),
        EvaluatedValue::Bottom(bottom) => EvaluatedFieldLabel::Invalid(bottom),
        other => EvaluatedFieldLabel::Invalid(Bottom::new(
            "cue.eval.invalid_dynamic_label",
            format!(
                "dynamic field label must resolve to string, got {}",
                other.kind()
            ),
            None,
            false,
        )),
    }
}

fn interpolation_value(value: EvaluatedValue) -> Option<String> {
    match resolve_default_value(value) {
        EvaluatedValue::String(value) | EvaluatedValue::Number(value) => Some(value),
        EvaluatedValue::Bool(value) => Some(value.to_string()),
        EvaluatedValue::Null => Some("null".to_owned()),
        EvaluatedValue::Bytes(value) => Some(String::from_utf8_lossy(&value).into_owned()),
        _ => None,
    }
}

fn pattern_label_matches(pattern: &EvaluatedValue, label: &str) -> Result<bool, Bottom> {
    match resolve_default_value(pattern.clone()) {
        EvaluatedValue::Top | EvaluatedValue::Kind(ValueKind::String) => Ok(true),
        EvaluatedValue::String(value) => Ok(value == label),
        EvaluatedValue::StringConstraints(constraints) => Ok(matches!(
            unify_string_constraints(label.to_owned(), &constraints),
            EvaluatedValue::String(_)
        )),
        EvaluatedValue::StringConstraintSet(constraints) => Ok(matches!(
            unify_string_constraint_set(label.to_owned(), &constraints),
            EvaluatedValue::String(_)
        )),
        EvaluatedValue::RegexConstraint { pattern, negated } => match compile_regex(&pattern) {
            Ok(regex) => Ok(regex.is_match(label) != negated),
            Err(bottom) => Err(bottom),
        },
        EvaluatedValue::Bottom(bottom) => Err(bottom),
        other => Err(Bottom::new(
            "cue.eval.unsupported_pattern_label",
            format!(
                "pattern field label must be a string constraint, got {}",
                other.kind()
            ),
            None,
            false,
        )),
    }
}

fn comprehension_items(
    source: EvaluatedValue,
) -> Result<Vec<(EvaluatedValue, EvaluatedValue)>, Bottom> {
    match source {
        EvaluatedValue::List(items) if items.len() <= MAX_COMPREHENSION_GENERATED_ITEMS => {
            Ok(items
                .into_iter()
                .enumerate()
                .map(|(index, value)| (EvaluatedValue::Number(index.to_string()), value))
                .collect())
        }
        EvaluatedValue::Struct(fields)
        | EvaluatedValue::PatternedStruct { fields, .. }
        | EvaluatedValue::ClosedStruct(fields)
        | EvaluatedValue::ClosedPatternedStruct { fields, .. }
            if fields.len() <= MAX_COMPREHENSION_GENERATED_ITEMS =>
        {
            Ok(fields
                .into_iter()
                .map(|(label, value)| (EvaluatedValue::String(label), value))
                .collect())
        }
        EvaluatedValue::List(_)
        | EvaluatedValue::Struct(_)
        | EvaluatedValue::PatternedStruct { .. }
        | EvaluatedValue::ClosedStruct(_)
        | EvaluatedValue::ClosedPatternedStruct { .. } => Err(Bottom::new(
            "cue.eval.comprehension_limit",
            "comprehension generated item limit exceeded",
            None,
            false,
        )),
        EvaluatedValue::Bottom(bottom) => Err(bottom),
        other => Err(Bottom::new(
            "cue.eval.invalid_comprehension_source",
            format!(
                "for comprehension source must be list or struct, got {}",
                other.kind()
            ),
            None,
            false,
        )),
    }
}

fn sorted_vertices(vertices: impl IntoIterator<Item = VertexId>) -> Vec<VertexId> {
    let mut vertices = vertices.into_iter().collect::<Vec<_>>();
    vertices.sort_unstable();
    vertices
}

fn cyclic_graph_components(graph: &HashMap<VertexId, HashSet<VertexId>>) -> Vec<HashSet<VertexId>> {
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    for vertex in sorted_vertices(graph.keys().copied()) {
        visit_dependency_order(vertex, graph, &mut visited, &mut order);
    }
    let reverse = reverse_dependency_graph(graph);
    let mut components = Vec::new();
    visited.clear();
    while let Some(vertex) = order.pop() {
        if visited.contains(&vertex) {
            continue;
        }
        let mut component = HashSet::new();
        visit_dependency_component(vertex, &reverse, &mut visited, &mut component);
        components.push(component);
    }
    components.into_iter().filter(is_cyclic_component).collect()
}

fn visit_dependency_order(
    vertex: VertexId,
    graph: &HashMap<VertexId, HashSet<VertexId>>,
    visited: &mut HashSet<VertexId>,
    order: &mut Vec<VertexId>,
) {
    if !visited.insert(vertex) {
        return;
    }
    let neighbors = graph
        .get(&vertex)
        .map(|edges| sorted_vertices(edges.iter().copied()))
        .unwrap_or_default();
    for neighbor in neighbors {
        visit_dependency_order(neighbor, graph, visited, order);
    }
    order.push(vertex);
}

fn reverse_dependency_graph(
    graph: &HashMap<VertexId, HashSet<VertexId>>,
) -> HashMap<VertexId, HashSet<VertexId>> {
    let mut reverse = graph
        .keys()
        .copied()
        .map(|vertex| (vertex, HashSet::new()))
        .collect::<HashMap<_, _>>();
    for (source, targets) in graph {
        for target in targets {
            reverse.entry(*target).or_default().insert(*source);
        }
    }
    reverse
}

fn visit_dependency_component(
    vertex: VertexId,
    graph: &HashMap<VertexId, HashSet<VertexId>>,
    visited: &mut HashSet<VertexId>,
    component: &mut HashSet<VertexId>,
) {
    if !visited.insert(vertex) {
        return;
    }
    component.insert(vertex);
    let neighbors = graph
        .get(&vertex)
        .map(|edges| sorted_vertices(edges.iter().copied()))
        .unwrap_or_default();
    for neighbor in neighbors {
        visit_dependency_component(neighbor, graph, visited, component);
    }
}

fn is_cyclic_component(component: &HashSet<VertexId>) -> bool {
    component.len() > 1
}

fn cycle_fixpoint_limit_bottom() -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.cycle_fixpoint_limit",
        "cycle fixpoint did not converge",
        None,
        false,
    ))
}

fn cycle_bottom() -> Bottom {
    Bottom::new(
        "cue.eval.cycle",
        "cycle has unresolved disjunctions",
        None,
        false,
    )
}

fn depth_limit_bottom() -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.depth_limit",
        "evaluation depth limit exceeded",
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
        BaseValue::Builtin(name) => builtin_kind(name).map_or_else(
            || EvaluatedValue::Builtin(name.clone()),
            EvaluatedValue::Kind,
        ),
        _ => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_base",
            "unsupported base value",
            None,
            false,
        )),
    }
}

fn into_struct_accumulator(value: EvaluatedValue) -> StructAccumulator {
    match value {
        EvaluatedValue::Struct(fields) => StructAccumulator {
            fields,
            patterns: Vec::new(),
            closed: false,
        },
        EvaluatedValue::PatternedStruct { fields, patterns } => StructAccumulator {
            fields,
            patterns,
            closed: false,
        },
        EvaluatedValue::ClosedStruct(fields) => StructAccumulator {
            fields,
            patterns: Vec::new(),
            closed: true,
        },
        EvaluatedValue::ClosedPatternedStruct { fields, patterns } => StructAccumulator {
            fields,
            patterns,
            closed: true,
        },
        _ => StructAccumulator::default(),
    }
}

impl StructAccumulator {
    fn into_value(self) -> EvaluatedValue {
        match (self.closed, self.patterns.is_empty()) {
            (false, true) => EvaluatedValue::Struct(self.fields),
            (false, false) => EvaluatedValue::PatternedStruct {
                fields: self.fields,
                patterns: self.patterns,
            },
            (true, true) => EvaluatedValue::ClosedStruct(self.fields),
            (true, false) => EvaluatedValue::ClosedPatternedStruct {
                fields: self.fields,
                patterns: self.patterns,
            },
        }
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
        ("<" | "<=" | ">" | ">=", value) => {
            if let (Some(op), Some(value)) = (numeric_bound_op(op), numeric_bound_literal(&value)) {
                EvaluatedValue::NumericConstraint(vec![NumericBound { op, value }])
            } else {
                invalid_unary(op, &value)
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

fn numeric_bound_literal(value: &EvaluatedValue) -> Option<String> {
    match value {
        EvaluatedValue::Number(value) => Some(value.clone()),
        EvaluatedValue::Default(value) => numeric_bound_literal(value),
        EvaluatedValue::Disjunction(disjuncts) => {
            let mut defaults = disjuncts
                .iter()
                .filter(|disjunct| disjunct.default)
                .filter_map(|disjunct| numeric_bound_literal(&disjunct.value));
            let value = defaults.next()?;
            defaults.next().is_none().then_some(value)
        }
        _ => None,
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
        "list.Concat" => evaluate_list_concat(args),
        "list.Contains" => evaluate_list_contains(args),
        "list.Drop" => evaluate_list_drop(args),
        "list.FlattenN" => evaluate_list_flatten_n(args),
        "list.IsSortedStrings" => evaluate_list_is_sorted_strings(args),
        "list.MaxItems" => evaluate_list_max_items(args),
        "list.Max" => evaluate_list_numeric_aggregate("list.Max", args, NumericAggregate::Max),
        "list.MinItems" => evaluate_list_min_items(args),
        "list.Min" => evaluate_list_numeric_aggregate("list.Min", args, NumericAggregate::Min),
        "list.Avg" => evaluate_list_numeric_aggregate("list.Avg", args, NumericAggregate::Avg),
        "list.Product" => {
            evaluate_list_numeric_aggregate("list.Product", args, NumericAggregate::Product)
        }
        "list.Range" => evaluate_list_range(args),
        "list.Repeat" => evaluate_list_repeat(args),
        "list.Reverse" => evaluate_list_reverse(args),
        "list.Slice" => evaluate_list_slice(args),
        "list.IsSorted" => evaluate_list_is_sorted(args),
        "list.Sort" | "list.SortStable" => evaluate_list_sort(args, true),
        "list.SortStrings" => evaluate_list_sort_strings(args),
        "list.Sum" => evaluate_list_numeric_aggregate("list.Sum", args, NumericAggregate::Sum),
        "list.Take" => evaluate_list_take(args),
        "list.UniqueItems" => evaluate_list_unique_items(args),
        "or" => evaluate_or(args),
        "strings.ByteAt" => evaluate_strings_byte_at(args),
        "strings.ByteSlice" => evaluate_strings_byte_slice(args),
        "strings.Compare" => evaluate_strings_compare(args),
        "strings.Contains" => evaluate_strings_contains(args),
        "strings.ContainsAny" => evaluate_strings_contains_any(args),
        "strings.Count" => evaluate_strings_count(args),
        "strings.Fields" => evaluate_strings_fields(args),
        "strings.HasPrefix" => evaluate_strings_has_prefix(args),
        "strings.HasSuffix" => evaluate_strings_has_suffix(args),
        "strings.Index" => evaluate_strings_index(args),
        "strings.IndexAny" => evaluate_strings_index_any(args),
        "strings.Join" => evaluate_strings_join(args),
        "strings.LastIndex" => evaluate_strings_last_index(args),
        "strings.LastIndexAny" => evaluate_strings_last_index_any(args),
        "strings.MaxRunes" => evaluate_strings_max_runes(args),
        "strings.MinRunes" => evaluate_strings_min_runes(args),
        "strings.Replace" => evaluate_strings_replace(args),
        "strings.Repeat" => evaluate_strings_repeat(args),
        "strings.Runes" => evaluate_strings_runes(args),
        "strings.SliceRunes" => evaluate_strings_slice_runes(args),
        "strings.Split" => evaluate_strings_split(args),
        "strings.SplitAfter" => evaluate_strings_split_after(args),
        "strings.SplitAfterN" => evaluate_strings_split_after_n(args),
        "strings.SplitN" => evaluate_strings_split_n(args),
        "strings.ToCamel" => evaluate_strings_to_camel(args),
        "strings.ToLower" => evaluate_strings_to_lower(args),
        "strings.ToTitle" => evaluate_strings_to_title(args),
        "strings.ToUpper" => evaluate_strings_to_upper(args),
        "strings.Trim" => evaluate_strings_trim(args),
        "strings.TrimLeft" => evaluate_strings_trim_left(args),
        "strings.TrimPrefix" => evaluate_strings_trim_prefix(args),
        "strings.TrimRight" => evaluate_strings_trim_right(args),
        "strings.TrimSpace" => evaluate_strings_trim_space(args),
        "strings.TrimSuffix" => evaluate_strings_trim_suffix(args),
        _ => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.unsupported_builtin",
            format!("unsupported builtin `{name}`"),
            None,
            false,
        )),
    }
}

fn evaluate_strings_contains(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_bool("strings.Contains", args, |value, needle| {
        value.contains(needle)
    })
}

fn evaluate_strings_contains_any(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_bool("strings.ContainsAny", args, |value, chars| {
        value.chars().any(|character| chars.contains(character))
    })
}

fn evaluate_strings_compare(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((left, right)) = two_string_args("strings.Compare", args) else {
        return invalid_string_builtin_args("strings.Compare");
    };
    let ordering = match left.cmp(&right) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    };
    EvaluatedValue::Number(ordering.to_string())
}

fn evaluate_strings_has_prefix(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_bool("strings.HasPrefix", args, |value, prefix| {
        value.starts_with(prefix)
    })
}

fn evaluate_strings_has_suffix(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_bool("strings.HasSuffix", args, |value, suffix| {
        value.ends_with(suffix)
    })
}

fn evaluate_strings_count(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, needle)) = two_string_args("strings.Count", args) else {
        return invalid_string_builtin_args("strings.Count");
    };
    let count = if needle.is_empty() {
        value.chars().count().checked_add(1)
    } else {
        Some(value.matches(&needle).count())
    };
    count.map_or_else(
        || builtin_resource_exhausted("strings.Count"),
        |count| EvaluatedValue::Number(count.to_string()),
    )
}

fn evaluate_strings_index(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, needle)) = two_string_args("strings.Index", args) else {
        return invalid_string_builtin_args("strings.Index");
    };
    byte_index_result(value.find(&needle))
}

fn evaluate_strings_last_index(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, needle)) = two_string_args("strings.LastIndex", args) else {
        return invalid_string_builtin_args("strings.LastIndex");
    };
    byte_index_result(value.rfind(&needle))
}

fn evaluate_strings_index_any(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, chars)) = two_string_args("strings.IndexAny", args) else {
        return invalid_string_builtin_args("strings.IndexAny");
    };
    byte_index_result(value.find(|character| chars.contains(character)))
}

fn evaluate_strings_last_index_any(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, chars)) = two_string_args("strings.LastIndexAny", args) else {
        return invalid_string_builtin_args("strings.LastIndexAny");
    };
    byte_index_result(value.rfind(|character| chars.contains(character)))
}

fn evaluate_strings_join(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, separator)) = string_list_and_string_args("strings.Join", args) else {
        return invalid_string_builtin_args("strings.Join");
    };
    if !joined_string_fits(&items, &separator) {
        return builtin_resource_exhausted("strings.Join");
    }
    EvaluatedValue::String(items.join(&separator))
}

fn evaluate_strings_repeat(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, count)) = string_and_count_args("strings.Repeat", args) else {
        return invalid_string_builtin_args("strings.Repeat");
    };
    let Some(size) = value.len().checked_mul(count) else {
        return builtin_resource_exhausted("strings.Repeat");
    };
    if size > MAX_BUILTIN_GENERATED_BYTES {
        return builtin_resource_exhausted("strings.Repeat");
    }
    EvaluatedValue::String(value.repeat(count))
}

fn evaluate_strings_split(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, separator)) = two_string_args("strings.Split", args) else {
        return invalid_string_builtin_args("strings.Split");
    };
    evaluate_split("strings.Split", &value, &separator, -1, false)
}

fn evaluate_strings_split_after(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, separator)) = two_string_args("strings.SplitAfter", args) else {
        return invalid_string_builtin_args("strings.SplitAfter");
    };
    evaluate_split("strings.SplitAfter", &value, &separator, -1, true)
}

fn evaluate_strings_split_n(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, separator, count)) = two_string_string_integer_args("strings.SplitN", args)
    else {
        return invalid_string_builtin_args("strings.SplitN");
    };
    evaluate_split("strings.SplitN", &value, &separator, count, false)
}

fn evaluate_strings_split_after_n(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, separator, count)) =
        two_string_string_integer_args("strings.SplitAfterN", args)
    else {
        return invalid_string_builtin_args("strings.SplitAfterN");
    };
    evaluate_split("strings.SplitAfterN", &value, &separator, count, true)
}

fn evaluate_strings_fields(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(value) = single_string_arg("strings.Fields", args) else {
        return invalid_string_builtin_args("strings.Fields");
    };
    if !generated_string_fits(&value) {
        return builtin_resource_exhausted("strings.Fields");
    }
    let items = value
        .split_whitespace()
        .map(|item| EvaluatedValue::String(item.to_owned()))
        .collect::<Vec<_>>();
    if items.len() > MAX_BUILTIN_GENERATED_ITEMS {
        return builtin_resource_exhausted("strings.Fields");
    }
    EvaluatedValue::List(items)
}

fn evaluate_strings_to_lower(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(value) = single_string_arg("strings.ToLower", args) else {
        return invalid_string_builtin_args("strings.ToLower");
    };
    evaluate_case_mapping("strings.ToLower", &value, str::to_lowercase)
}

fn evaluate_strings_to_upper(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(value) = single_string_arg("strings.ToUpper", args) else {
        return invalid_string_builtin_args("strings.ToUpper");
    };
    evaluate_case_mapping("strings.ToUpper", &value, str::to_uppercase)
}

fn evaluate_strings_to_title(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(value) = single_string_arg("strings.ToTitle", args) else {
        return invalid_string_builtin_args("strings.ToTitle");
    };
    let mut previous_was_space = true;
    evaluate_char_mapping("strings.ToTitle", &value, |character| {
        let mapped = if previous_was_space {
            character.to_uppercase().collect::<String>()
        } else {
            character.to_string()
        };
        previous_was_space = character.is_whitespace();
        mapped
    })
}

fn evaluate_strings_to_camel(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(value) = single_string_arg("strings.ToCamel", args) else {
        return invalid_string_builtin_args("strings.ToCamel");
    };
    let mut previous_was_space = true;
    evaluate_char_mapping("strings.ToCamel", &value, |character| {
        let mapped = if previous_was_space {
            character.to_lowercase().collect::<String>()
        } else {
            character.to_string()
        };
        previous_was_space = character.is_whitespace();
        mapped
    })
}

fn evaluate_strings_trim_space(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(value) = single_string_arg("strings.TrimSpace", args) else {
        return invalid_string_builtin_args("strings.TrimSpace");
    };
    if !generated_string_fits(&value) {
        return builtin_resource_exhausted("strings.TrimSpace");
    }
    EvaluatedValue::String(value.trim().to_owned())
}

fn evaluate_strings_trim(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_string("strings.Trim", args, |value, cutset| {
        value
            .trim_matches(|character| cutset.contains(character))
            .to_owned()
    })
}

fn evaluate_strings_trim_left(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_string("strings.TrimLeft", args, |value, cutset| {
        value
            .trim_start_matches(|character| cutset.contains(character))
            .to_owned()
    })
}

fn evaluate_strings_trim_right(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_string("strings.TrimRight", args, |value, cutset| {
        value
            .trim_end_matches(|character| cutset.contains(character))
            .to_owned()
    })
}

fn evaluate_strings_trim_prefix(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_string("strings.TrimPrefix", args, |value, prefix| {
        value.strip_prefix(prefix).unwrap_or(value).to_owned()
    })
}

fn evaluate_strings_trim_suffix(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    evaluate_two_string_args_string("strings.TrimSuffix", args, |value, suffix| {
        value.strip_suffix(suffix).unwrap_or(value).to_owned()
    })
}

fn evaluate_strings_replace(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, old, new, count)) = three_string_integer_args("strings.Replace", args) else {
        return invalid_string_builtin_args("strings.Replace");
    };
    let replaced = if count < 0 {
        value.replace(&old, &new)
    } else {
        let Some(count) = usize::try_from(count).ok() else {
            return builtin_resource_exhausted("strings.Replace");
        };
        value.replacen(&old, &new, count)
    };
    if !generated_string_fits(&replaced) {
        return builtin_resource_exhausted("strings.Replace");
    }
    EvaluatedValue::String(replaced)
}

fn evaluate_strings_runes(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(value) = single_string_arg("strings.Runes", args) else {
        return invalid_string_builtin_args("strings.Runes");
    };
    if value.chars().count() > MAX_BUILTIN_GENERATED_ITEMS {
        return builtin_resource_exhausted("strings.Runes");
    }
    EvaluatedValue::List(
        value
            .chars()
            .map(|character| {
                let codepoint = u32::from(character);
                EvaluatedValue::Number(codepoint.to_string())
            })
            .collect(),
    )
}

fn evaluate_strings_min_runes(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    if let Some(limit) = single_signed_count_arg("strings.MinRunes", &args) {
        return EvaluatedValue::StringConstraints(vec![StringConstraint {
            op: StringConstraintOp::MinRunes,
            limit,
        }]);
    }
    let Some((value, limit)) = string_and_signed_count_args("strings.MinRunes", args) else {
        return invalid_string_builtin_args("strings.MinRunes");
    };
    EvaluatedValue::Bool(rune_count_satisfies(
        value.chars().count(),
        limit,
        Ordering::Greater,
    ))
}

fn evaluate_strings_max_runes(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    if let Some(limit) = single_signed_count_arg("strings.MaxRunes", &args) {
        return EvaluatedValue::StringConstraints(vec![StringConstraint {
            op: StringConstraintOp::MaxRunes,
            limit,
        }]);
    }
    let Some((value, limit)) = string_and_signed_count_args("strings.MaxRunes", args) else {
        return invalid_string_builtin_args("strings.MaxRunes");
    };
    EvaluatedValue::Bool(rune_count_satisfies(
        value.chars().count(),
        limit,
        Ordering::Less,
    ))
}

fn evaluate_strings_slice_runes(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, start, end)) = string_two_index_args("strings.SliceRunes", args) else {
        return invalid_string_builtin_args("strings.SliceRunes");
    };
    let chars = value.chars().collect::<Vec<_>>();
    if start > end || end > chars.len() {
        return invalid_string_index("strings.SliceRunes");
    }
    let Some(slice) = chars.get(start..end) else {
        return invalid_string_index("strings.SliceRunes");
    };
    let sliced = slice.iter().collect::<String>();
    EvaluatedValue::String(sliced)
}

fn evaluate_strings_byte_at(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((bytes, index)) = bytes_like_and_index_args("strings.ByteAt", args) else {
        return invalid_string_builtin_args("strings.ByteAt");
    };
    let Some(byte) = bytes.get(index) else {
        return invalid_string_index("strings.ByteAt");
    };
    EvaluatedValue::Number(byte.to_string())
}

fn evaluate_strings_byte_slice(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((value, start, end)) = bytes_like_two_index_args("strings.ByteSlice", args) else {
        return invalid_string_builtin_args("strings.ByteSlice");
    };
    let len = value.byte_len();
    if start > end || end > len {
        return invalid_string_index("strings.ByteSlice");
    }
    match value {
        BytesLikeValue::String(value) => {
            let Some(slice) = value.as_bytes().get(start..end) else {
                return invalid_string_index("strings.ByteSlice");
            };
            EvaluatedValue::Bytes(slice.to_vec())
        }
        BytesLikeValue::Bytes(bytes) => {
            let Some(slice) = bytes.get(start..end) else {
                return invalid_string_index("strings.ByteSlice");
            };
            EvaluatedValue::Bytes(slice.to_vec())
        }
    }
}

fn evaluate_case_mapping(
    name: &str,
    value: &str,
    mapping: impl Fn(&str) -> String,
) -> EvaluatedValue {
    if !generated_string_fits(value) {
        return builtin_resource_exhausted(name);
    }
    let mapped = mapping(value);
    if !generated_string_fits(&mapped) {
        return builtin_resource_exhausted(name);
    }
    EvaluatedValue::String(mapped)
}

fn evaluate_char_mapping(
    name: &str,
    value: &str,
    mut mapping: impl FnMut(char) -> String,
) -> EvaluatedValue {
    if !generated_string_fits(value) {
        return builtin_resource_exhausted(name);
    }
    let mut mapped = String::with_capacity(value.len());
    for character in value.chars() {
        mapped.push_str(&mapping(character));
        if mapped.len() > MAX_BUILTIN_GENERATED_BYTES {
            return builtin_resource_exhausted(name);
        }
    }
    EvaluatedValue::String(mapped)
}

fn evaluate_two_string_args_bool(
    name: &str,
    args: Vec<EvaluatedValue>,
    predicate: impl Fn(&str, &str) -> bool,
) -> EvaluatedValue {
    let Some((value, other)) = two_string_args(name, args) else {
        return invalid_string_builtin_args(name);
    };
    EvaluatedValue::Bool(predicate(&value, &other))
}

fn evaluate_two_string_args_string(
    name: &str,
    args: Vec<EvaluatedValue>,
    mapping: impl Fn(&str, &str) -> String,
) -> EvaluatedValue {
    let Some((value, other)) = two_string_args(name, args) else {
        return invalid_string_builtin_args(name);
    };
    let mapped = mapping(&value, &other);
    if !generated_string_fits(&mapped) {
        return builtin_resource_exhausted(name);
    }
    EvaluatedValue::String(mapped)
}

fn evaluate_list_contains(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return invalid_list_builtin_arg("list.Contains");
    }
    let mut args = args.into_iter();
    let Some(EvaluatedValue::List(items)) = args.next() else {
        return invalid_list_builtin_arg("list.Contains");
    };
    let Some(needle) = args.next() else {
        return invalid_list_builtin_arg("list.Contains");
    };
    for item in items {
        match values_equal(&item, &needle) {
            Ok(true) => return EvaluatedValue::Bool(true),
            Ok(false) => {}
            Err(bottom) => return EvaluatedValue::Bottom(bottom),
        }
    }
    EvaluatedValue::Bool(false)
}

fn evaluate_list_repeat(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, count)) = list_and_count_args("list.Repeat", args) else {
        return invalid_list_builtin_arg("list.Repeat");
    };
    let Some(capacity) = items.len().checked_mul(count) else {
        return builtin_resource_exhausted("list.Repeat");
    };
    if capacity > MAX_BUILTIN_GENERATED_ITEMS {
        return builtin_resource_exhausted("list.Repeat");
    }
    let mut repeated = Vec::with_capacity(capacity);
    for _ in 0..count {
        repeated.extend(items.iter().cloned());
    }
    EvaluatedValue::List(repeated)
}

fn evaluate_list_concat(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(lists) = nested_list_arg("list.Concat", args) else {
        return invalid_list_builtin_arg("list.Concat");
    };
    let mut capacity = 0_usize;
    for items in &lists {
        let Some(next_capacity) = capacity.checked_add(items.len()) else {
            return builtin_resource_exhausted("list.Concat");
        };
        if next_capacity > MAX_BUILTIN_GENERATED_ITEMS {
            return builtin_resource_exhausted("list.Concat");
        }
        capacity = next_capacity;
    }
    let mut concatenated = Vec::with_capacity(capacity);
    for items in lists {
        concatenated.extend(items);
    }
    EvaluatedValue::List(concatenated)
}

fn evaluate_list_drop(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, count)) = list_and_count_args("list.Drop", args) else {
        return invalid_list_builtin_arg("list.Drop");
    };
    let dropped = if count > items.len() {
        Vec::new()
    } else {
        items.into_iter().skip(count).collect()
    };
    EvaluatedValue::List(dropped)
}

fn evaluate_list_take(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, count)) = list_and_count_args("list.Take", args) else {
        return invalid_list_builtin_arg("list.Take");
    };
    EvaluatedValue::List(items.into_iter().take(count).collect())
}

fn evaluate_list_slice(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, start, end)) = list_two_index_args("list.Slice", args) else {
        return invalid_list_builtin_arg("list.Slice");
    };
    if start > end || end > items.len() {
        return invalid_list_index("list.Slice");
    }
    EvaluatedValue::List(items.into_iter().skip(start).take(end - start).collect())
}

fn evaluate_list_reverse(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(mut items) = single_list_arg(args) else {
        return invalid_list_builtin_arg("list.Reverse");
    };
    items.reverse();
    EvaluatedValue::List(items)
}

fn evaluate_list_flatten_n(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, depth)) = list_and_signed_count_args("list.FlattenN", args) else {
        return invalid_list_builtin_arg("list.FlattenN");
    };
    let mut flattened = Vec::new();
    if !flatten_items(&items, depth, &mut flattened) {
        return builtin_resource_exhausted("list.FlattenN");
    }
    EvaluatedValue::List(flattened)
}

fn evaluate_list_unique_items(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(items) = single_list_arg(args) else {
        return invalid_list_builtin_arg("list.UniqueItems");
    };
    for (index, item) in items.iter().enumerate() {
        for other in items.iter().skip(index.saturating_add(1)) {
            match values_equal(item, other) {
                Ok(true) => return EvaluatedValue::Bool(false),
                Ok(false) => {}
                Err(bottom) => return EvaluatedValue::Bottom(bottom),
            }
        }
    }
    EvaluatedValue::Bool(true)
}

fn evaluate_list_min_items(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, count)) = list_and_count_args("list.MinItems", args) else {
        return invalid_list_builtin_arg("list.MinItems");
    };
    EvaluatedValue::Bool(items.len() >= count)
}

fn evaluate_list_max_items(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, count)) = list_and_count_args("list.MaxItems", args) else {
        return invalid_list_builtin_arg("list.MaxItems");
    };
    EvaluatedValue::Bool(items.len() <= count)
}

fn evaluate_list_sort(args: Vec<EvaluatedValue>, stable: bool) -> EvaluatedValue {
    let Some((mut items, direction)) = list_and_ordering_args(args) else {
        return invalid_list_builtin_arg(if stable {
            "list.SortStable"
        } else {
            "list.Sort"
        });
    };
    let mut failed = None;
    if stable {
        items.sort_by(
            |left, right| match compare_list_values(left, right, direction) {
                Ok(ordering) => ordering,
                Err(bottom) => {
                    failed = Some(bottom);
                    Ordering::Equal
                }
            },
        );
    } else {
        items.sort_unstable_by(
            |left, right| match compare_list_values(left, right, direction) {
                Ok(ordering) => ordering,
                Err(bottom) => {
                    failed = Some(bottom);
                    Ordering::Equal
                }
            },
        );
    }
    failed.map_or(EvaluatedValue::List(items), EvaluatedValue::Bottom)
}

fn sort_with_direction(
    mut items: Vec<EvaluatedValue>,
    direction: SortDirection,
    stable: bool,
) -> EvaluatedValue {
    let mut failed = None;
    if stable {
        items.sort_by(
            |left, right| match compare_list_values(left, right, direction) {
                Ok(ordering) => ordering,
                Err(bottom) => {
                    failed = Some(bottom);
                    Ordering::Equal
                }
            },
        );
    } else {
        items.sort_unstable_by(
            |left, right| match compare_list_values(left, right, direction) {
                Ok(ordering) => ordering,
                Err(bottom) => {
                    failed = Some(bottom);
                    Ordering::Equal
                }
            },
        );
    }
    failed.map_or(EvaluatedValue::List(items), EvaluatedValue::Bottom)
}

fn evaluate_list_is_sorted(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((items, direction)) = list_and_ordering_args(args) else {
        return invalid_list_builtin_arg("list.IsSorted");
    };
    for window in items.windows(2) {
        let Some(left) = window.first() else {
            continue;
        };
        let Some(right) = window.get(1) else {
            continue;
        };
        match compare_list_values(left, right, direction) {
            Ok(Ordering::Greater) => return EvaluatedValue::Bool(false),
            Ok(Ordering::Less | Ordering::Equal) => {}
            Err(bottom) => return EvaluatedValue::Bottom(bottom),
        }
    }
    EvaluatedValue::Bool(true)
}

fn is_sorted_with_direction(items: &[EvaluatedValue], direction: SortDirection) -> EvaluatedValue {
    for window in items.windows(2) {
        let Some(left) = window.first() else {
            continue;
        };
        let Some(right) = window.get(1) else {
            continue;
        };
        match compare_list_values(left, right, direction) {
            Ok(Ordering::Greater) => return EvaluatedValue::Bool(false),
            Ok(Ordering::Less | Ordering::Equal) => {}
            Err(bottom) => return EvaluatedValue::Bottom(bottom),
        }
    }
    EvaluatedValue::Bool(true)
}

fn evaluate_list_sort_strings(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(mut items) = single_string_list_arg("list.SortStrings", args) else {
        return invalid_list_builtin_arg("list.SortStrings");
    };
    items.sort();
    EvaluatedValue::List(items.into_iter().map(EvaluatedValue::String).collect())
}

fn evaluate_list_is_sorted_strings(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some(items) = single_string_list_arg("list.IsSortedStrings", args) else {
        return invalid_list_builtin_arg("list.IsSortedStrings");
    };
    EvaluatedValue::Bool(items.windows(2).all(|window| {
        let Some(left) = window.first() else {
            return true;
        };
        let Some(right) = window.get(1) else {
            return true;
        };
        left <= right
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SortDirection {
    Ascending,
    Descending,
}

fn list_and_ordering_args(
    args: Vec<EvaluatedValue>,
) -> Option<(Vec<EvaluatedValue>, SortDirection)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return None;
    }
    let comparator = args.pop()?;
    let list = args.pop()?;
    let EvaluatedValue::List(items) = list else {
        return None;
    };
    let direction = match comparator {
        EvaluatedValue::Builtin(name) if name == "list.Ascending" => SortDirection::Ascending,
        EvaluatedValue::Builtin(name) if name == "list.Descending" => SortDirection::Descending,
        _ => return None,
    };
    Some((items, direction))
}

fn compare_list_values(
    left: &EvaluatedValue,
    right: &EvaluatedValue,
    direction: SortDirection,
) -> Result<Ordering, Bottom> {
    let ordering = match (
        resolve_default_value(left.clone()),
        resolve_default_value(right.clone()),
    ) {
        (EvaluatedValue::Number(left), EvaluatedValue::Number(right)) => {
            let Some(left) = parse_decimal_number(&left) else {
                return Err(invalid_sort_value());
            };
            let Some(right) = parse_decimal_number(&right) else {
                return Err(invalid_sort_value());
            };
            compare_decimal_numbers(&left, &right)
        }
        (EvaluatedValue::String(left), EvaluatedValue::String(right)) => left.cmp(&right),
        _ => return Err(invalid_sort_value()),
    };
    Ok(match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    })
}

fn invalid_sort_value() -> Bottom {
    Bottom::new(
        "cue.eval.invalid_builtin_arg",
        "list comparator requires comparable number or string elements",
        None,
        false,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NumericAggregate {
    Avg,
    Max,
    Min,
    Product,
    Sum,
}

fn evaluate_list_numeric_aggregate(
    name: &str,
    args: Vec<EvaluatedValue>,
    aggregate: NumericAggregate,
) -> EvaluatedValue {
    let Some(values) = single_number_list_arg(name, args) else {
        return invalid_list_builtin_arg(name);
    };
    match aggregate {
        NumericAggregate::Avg | NumericAggregate::Max | NumericAggregate::Min
            if values.is_empty() =>
        {
            empty_numeric_list(name)
        }
        NumericAggregate::Avg => {
            let sum = values
                .iter()
                .fold(BigDecimal::zero(), |sum, value| sum + value);
            let Some(count) = i64::try_from(values.len()).ok().map(BigDecimal::from) else {
                return builtin_resource_exhausted(name);
            };
            evaluated_decimal_number(name, &(sum / count))
        }
        NumericAggregate::Max => values.into_iter().reduce(BigDecimal::max).map_or_else(
            || empty_numeric_list(name),
            |value| evaluated_decimal_number(name, &value),
        ),
        NumericAggregate::Min => values.into_iter().reduce(BigDecimal::min).map_or_else(
            || empty_numeric_list(name),
            |value| evaluated_decimal_number(name, &value),
        ),
        NumericAggregate::Product => evaluated_decimal_number(
            name,
            &values
                .into_iter()
                .fold(BigDecimal::from(1), |product, value| product * value),
        ),
        NumericAggregate::Sum => evaluated_decimal_number(
            name,
            &values
                .into_iter()
                .fold(BigDecimal::zero(), |sum, value| sum + value),
        ),
    }
}

fn evaluate_list_range(args: Vec<EvaluatedValue>) -> EvaluatedValue {
    let Some((start, limit, step)) = three_number_args("list.Range", args) else {
        return invalid_list_builtin_arg("list.Range");
    };
    if step.is_zero() {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_builtin_arg",
            "list.Range step must be non-zero",
            None,
            false,
        ));
    }
    let zero = BigDecimal::zero();
    if (step > zero && start > limit) || (step < zero && start < limit) {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_builtin_arg",
            "list.Range limit is incompatible with step direction",
            None,
            false,
        ));
    }
    let mut values = Vec::new();
    let mut next = start;
    while (step > zero && next < limit) || (step < zero && next > limit) {
        if values.len() >= MAX_BUILTIN_GENERATED_ITEMS {
            return builtin_resource_exhausted("list.Range");
        }
        let Some(number) = format_decimal_number(&next) else {
            return builtin_resource_exhausted("list.Range");
        };
        values.push(EvaluatedValue::Number(number));
        next += &step;
    }
    EvaluatedValue::List(values)
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
            EvaluatedValue::PatternedStruct { fields, patterns }
            | EvaluatedValue::ClosedPatternedStruct { fields, patterns } => {
                EvaluatedValue::ClosedPatternedStruct { fields, patterns }
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
        format!("{name} received invalid arguments"),
        None,
        false,
    ))
}

fn invalid_list_index(name: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_builtin_arg",
        format!("{name} received invalid list indexes"),
        None,
        false,
    ))
}

fn single_string_arg(_name: &str, args: Vec<EvaluatedValue>) -> Option<String> {
    let args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 1 {
        return None;
    }
    match args.into_iter().next()? {
        EvaluatedValue::String(value) => Some(value),
        _ => None,
    }
}

fn two_string_args(_name: &str, args: Vec<EvaluatedValue>) -> Option<(String, String)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return None;
    }
    let second = args.pop()?;
    let first = args.pop()?;
    match (first, second) {
        (EvaluatedValue::String(first), EvaluatedValue::String(second)) => Some((first, second)),
        _ => None,
    }
}

fn string_list_and_string_args(
    _name: &str,
    args: Vec<EvaluatedValue>,
) -> Option<(Vec<String>, String)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return None;
    }
    let separator = args.pop()?;
    let list = args.pop()?;
    let EvaluatedValue::String(separator) = separator else {
        return None;
    };
    let items = string_list_value(list)?;
    Some((items, separator))
}

fn string_and_count_args(_name: &str, args: Vec<EvaluatedValue>) -> Option<(String, usize)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return None;
    }
    let count = args.pop()?;
    let value = args.pop()?;
    match (value, count) {
        (EvaluatedValue::String(value), EvaluatedValue::Number(count)) => {
            Some((value, non_negative_count(&count)?))
        }
        _ => None,
    }
}

fn string_and_signed_count_args(_name: &str, args: Vec<EvaluatedValue>) -> Option<(String, i128)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return None;
    }
    let count = args.pop()?;
    let value = args.pop()?;
    match (value, count) {
        (EvaluatedValue::String(value), EvaluatedValue::Number(count)) => {
            Some((value, parse_integer(&count)?))
        }
        _ => None,
    }
}

fn single_signed_count_arg(_name: &str, args: &[EvaluatedValue]) -> Option<i128> {
    if args.len() != 1 {
        return None;
    }
    let value = args.first().cloned().map(resolve_default_value)?;
    let EvaluatedValue::Number(count) = value else {
        return None;
    };
    parse_integer(&count)
}

fn string_two_index_args(_name: &str, args: Vec<EvaluatedValue>) -> Option<(String, usize, usize)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return None;
    }
    let end = args.pop()?;
    let start = args.pop()?;
    let value = args.pop()?;
    match (value, start, end) {
        (
            EvaluatedValue::String(value),
            EvaluatedValue::Number(start),
            EvaluatedValue::Number(end),
        ) => Some((
            value,
            non_negative_count(&start)?,
            non_negative_count(&end)?,
        )),
        _ => None,
    }
}

fn two_string_string_integer_args(
    _name: &str,
    args: Vec<EvaluatedValue>,
) -> Option<(String, String, i128)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return None;
    }
    let count = args.pop()?;
    let separator = args.pop()?;
    let value = args.pop()?;
    match (value, separator, count) {
        (
            EvaluatedValue::String(value),
            EvaluatedValue::String(separator),
            EvaluatedValue::Number(count),
        ) => Some((value, separator, parse_integer(&count)?)),
        _ => None,
    }
}

fn three_string_integer_args(
    _name: &str,
    args: Vec<EvaluatedValue>,
) -> Option<(String, String, String, i128)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 4 {
        return None;
    }
    let count = args.pop()?;
    let new = args.pop()?;
    let old = args.pop()?;
    let value = args.pop()?;
    match (value, old, new, count) {
        (
            EvaluatedValue::String(value),
            EvaluatedValue::String(old),
            EvaluatedValue::String(new),
            EvaluatedValue::Number(count),
        ) => Some((value, old, new, parse_integer(&count)?)),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BytesLikeValue {
    Bytes(Vec<u8>),
    String(String),
}

impl BytesLikeValue {
    fn byte_len(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.len(),
            Self::String(value) => value.len(),
        }
    }
}

fn bytes_like_and_index_args(_name: &str, args: Vec<EvaluatedValue>) -> Option<(Vec<u8>, usize)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return None;
    }
    let index = args.pop()?;
    let value = args.pop()?;
    let bytes = match value {
        EvaluatedValue::String(value) => value.into_bytes(),
        EvaluatedValue::Bytes(bytes) => bytes,
        _ => return None,
    };
    let EvaluatedValue::Number(index) = index else {
        return None;
    };
    Some((bytes, non_negative_count(&index)?))
}

fn bytes_like_two_index_args(
    _name: &str,
    args: Vec<EvaluatedValue>,
) -> Option<(BytesLikeValue, usize, usize)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return None;
    }
    let end = args.pop()?;
    let start = args.pop()?;
    let value = args.pop()?;
    let value = match value {
        EvaluatedValue::String(value) => BytesLikeValue::String(value),
        EvaluatedValue::Bytes(bytes) => BytesLikeValue::Bytes(bytes),
        _ => return None,
    };
    match (start, end) {
        (EvaluatedValue::Number(start), EvaluatedValue::Number(end)) => Some((
            value,
            non_negative_count(&start)?,
            non_negative_count(&end)?,
        )),
        _ => None,
    }
}

fn list_and_count_args(
    _name: &str,
    args: Vec<EvaluatedValue>,
) -> Option<(Vec<EvaluatedValue>, usize)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return None;
    }
    let count = args.pop()?;
    let list = args.pop()?;
    match (list, count) {
        (EvaluatedValue::List(items), EvaluatedValue::Number(count)) => {
            Some((items, non_negative_count(&count)?))
        }
        _ => None,
    }
}

fn list_and_signed_count_args(
    _name: &str,
    args: Vec<EvaluatedValue>,
) -> Option<(Vec<EvaluatedValue>, i128)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 2 {
        return None;
    }
    let count = args.pop()?;
    let list = args.pop()?;
    match (list, count) {
        (EvaluatedValue::List(items), EvaluatedValue::Number(count)) => {
            Some((items, parse_integer(&count)?))
        }
        _ => None,
    }
}

fn list_two_index_args(
    _name: &str,
    args: Vec<EvaluatedValue>,
) -> Option<(Vec<EvaluatedValue>, usize, usize)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return None;
    }
    let end = args.pop()?;
    let start = args.pop()?;
    let list = args.pop()?;
    match (list, start, end) {
        (
            EvaluatedValue::List(items),
            EvaluatedValue::Number(start),
            EvaluatedValue::Number(end),
        ) => Some((
            items,
            non_negative_count(&start)?,
            non_negative_count(&end)?,
        )),
        _ => None,
    }
}

fn nested_list_arg(_name: &str, args: Vec<EvaluatedValue>) -> Option<Vec<Vec<EvaluatedValue>>> {
    let args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 1 {
        return None;
    }
    let EvaluatedValue::List(lists) = args.into_iter().next()? else {
        return None;
    };
    lists
        .into_iter()
        .map(|value| match resolve_default_value(value) {
            EvaluatedValue::List(items) => Some(items),
            _ => None,
        })
        .collect()
}

fn single_string_list_arg(_name: &str, args: Vec<EvaluatedValue>) -> Option<Vec<String>> {
    let items = single_list_arg(args)?;
    items
        .into_iter()
        .map(|item| match resolve_default_value(item) {
            EvaluatedValue::String(value) => Some(value),
            _ => None,
        })
        .collect()
}

fn single_number_list_arg(_name: &str, args: Vec<EvaluatedValue>) -> Option<Vec<BigDecimal>> {
    let items = single_list_arg(args)?;
    items
        .into_iter()
        .map(|item| match resolve_default_value(item) {
            EvaluatedValue::Number(value) => parse_exact_decimal(&value),
            _ => None,
        })
        .collect()
}

fn string_list_value(value: EvaluatedValue) -> Option<Vec<String>> {
    let EvaluatedValue::List(items) = value else {
        return None;
    };
    items
        .into_iter()
        .map(|item| match resolve_default_value(item) {
            EvaluatedValue::String(value) => Some(value),
            _ => None,
        })
        .collect()
}

fn three_number_args(
    _name: &str,
    args: Vec<EvaluatedValue>,
) -> Option<(BigDecimal, BigDecimal, BigDecimal)> {
    let mut args = args
        .into_iter()
        .map(resolve_default_value)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return None;
    }
    let step = args.pop()?;
    let limit = args.pop()?;
    let start = args.pop()?;
    match (start, limit, step) {
        (
            EvaluatedValue::Number(start),
            EvaluatedValue::Number(limit),
            EvaluatedValue::Number(step),
        ) => Some((
            parse_exact_decimal(&start)?,
            parse_exact_decimal(&limit)?,
            parse_exact_decimal(&step)?,
        )),
        _ => None,
    }
}

fn flatten_items(items: &[EvaluatedValue], depth: i128, output: &mut Vec<EvaluatedValue>) -> bool {
    for item in items {
        if output.len() >= MAX_BUILTIN_GENERATED_ITEMS {
            return false;
        }
        match resolve_default_value(item.clone()) {
            EvaluatedValue::List(nested) if depth != 0 => {
                let next_depth = if depth < 0 { depth } else { depth - 1 };
                if !flatten_items(&nested, next_depth, output) {
                    return false;
                }
            }
            value => output.push(value),
        }
    }
    true
}

fn empty_numeric_list(name: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_builtin_arg",
        format!("{name} received an empty list"),
        None,
        false,
    ))
}

fn joined_string_fits(items: &[String], separator: &str) -> bool {
    let mut size = 0_usize;
    for item in items {
        let Some(next_size) = size.checked_add(item.len()) else {
            return false;
        };
        size = next_size;
    }
    let separator_count = items.len().saturating_sub(1);
    let Some(separator_size) = separator.len().checked_mul(separator_count) else {
        return false;
    };
    size.checked_add(separator_size)
        .is_some_and(|size| size <= MAX_BUILTIN_GENERATED_BYTES)
}

fn evaluate_split(
    name: &str,
    value: &str,
    separator: &str,
    count: i128,
    after: bool,
) -> EvaluatedValue {
    if !generated_string_fits(value) {
        return builtin_resource_exhausted(name);
    }
    if count == 0 {
        return EvaluatedValue::List(Vec::new());
    }
    let limit = if count < 0 {
        None
    } else {
        let Some(limit) = usize::try_from(count).ok() else {
            return builtin_resource_exhausted(name);
        };
        Some(limit)
    };
    let items = if separator.is_empty() {
        split_empty_separator(value, limit)
    } else if after {
        split_after_separator(value, separator, limit)
    } else {
        split_before_separator(value, separator, limit)
    };
    let Some(items) = items else {
        return builtin_resource_exhausted(name);
    };
    EvaluatedValue::List(items.into_iter().map(EvaluatedValue::String).collect())
}

fn split_empty_separator(value: &str, limit: Option<usize>) -> Option<Vec<String>> {
    let limit = limit.unwrap_or(usize::MAX);
    if limit == 0 || value.is_empty() {
        return Some(Vec::new());
    }
    let mut items = Vec::new();
    for character in value.chars() {
        if items.len().checked_add(1)? >= limit {
            let remaining = value
                .char_indices()
                .nth(items.len())
                .and_then(|(offset, _)| value.get(offset..))
                .unwrap_or("");
            items.push(remaining.to_owned());
            return bounded_split_items(items);
        }
        items.push(character.to_string());
    }
    bounded_split_items(items)
}

fn split_before_separator(
    value: &str,
    separator: &str,
    limit: Option<usize>,
) -> Option<Vec<String>> {
    let items = match limit {
        Some(limit) => value
            .splitn(limit, separator)
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        None => value
            .split(separator)
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    };
    bounded_split_items(items)
}

fn split_after_separator(
    value: &str,
    separator: &str,
    limit: Option<usize>,
) -> Option<Vec<String>> {
    let limit = limit.unwrap_or(usize::MAX);
    if limit == 0 {
        return Some(Vec::new());
    }
    let mut items = Vec::new();
    let mut start = 0_usize;
    while items.len().checked_add(1)? < limit {
        let haystack = value.get(start..)?;
        let Some(relative) = haystack.find(separator) else {
            break;
        };
        let end = start.checked_add(relative)?.checked_add(separator.len())?;
        items.push(value.get(start..end)?.to_owned());
        start = end;
    }
    if let Some(rest) = value.get(start..) {
        items.push(rest.to_owned());
    }
    bounded_split_items(items)
}

fn bounded_split_items(items: Vec<String>) -> Option<Vec<String>> {
    if items.len() > MAX_BUILTIN_GENERATED_ITEMS {
        return None;
    }
    let bytes = items
        .iter()
        .try_fold(0_usize, |size, item| size.checked_add(item.len()))?;
    (bytes <= MAX_BUILTIN_GENERATED_BYTES).then_some(items)
}

fn generated_string_fits(value: &str) -> bool {
    value.len() <= MAX_BUILTIN_GENERATED_BYTES
}

fn byte_index_result(index: Option<usize>) -> EvaluatedValue {
    let index = index.map_or(-1_i128, |index| i128::try_from(index).unwrap_or(i128::MAX));
    EvaluatedValue::Number(index.to_string())
}

fn rune_count_satisfies(count: usize, limit: i128, direction: Ordering) -> bool {
    if limit < 0 {
        return matches!(direction, Ordering::Greater);
    }
    let Ok(limit) = usize::try_from(limit) else {
        return matches!(direction, Ordering::Less);
    };
    match direction {
        Ordering::Greater => count >= limit,
        Ordering::Less => count <= limit,
        Ordering::Equal => count == limit,
    }
}

fn invalid_string_index(name: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_builtin_arg",
        format!("{name} received an out-of-range index"),
        None,
        false,
    ))
}

fn non_negative_count(value: &str) -> Option<usize> {
    let count = parse_integer(value)?;
    if count < 0 {
        return None;
    }
    usize::try_from(count).ok()
}

fn invalid_string_builtin_args(name: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_builtin_arg",
        format!("{name} received invalid arguments"),
        None,
        false,
    ))
}

fn builtin_resource_exhausted(name: &str) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.resource_exhausted",
        format!("{name} result is too large"),
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
        EvaluatedValue::Struct(fields)
        | EvaluatedValue::PatternedStruct { fields, .. }
        | EvaluatedValue::ClosedStruct(fields)
        | EvaluatedValue::ClosedPatternedStruct { fields, .. } => {
            EvaluatedValue::Number(fields.len().to_string())
        }
        EvaluatedValue::Bottom(bottom) => EvaluatedValue::Bottom(bottom),
        value => EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_builtin_arg",
            format!("len cannot accept {}", value.kind()),
            None,
            false,
        )),
    }
}

fn evaluate_binary_with_default_operands(
    op: &str,
    left: EvaluatedValue,
    right: EvaluatedValue,
) -> EvaluatedValue {
    let (left, right) = if binary_consumes_default_operands(op) {
        (
            resolve_default_operand_value(left),
            resolve_default_operand_value(right),
        )
    } else {
        (
            strip_fixpoint_previous_values(left),
            strip_fixpoint_previous_values(right),
        )
    };
    if should_distribute_binary(op) && (is_choice_value(&left) || is_choice_value(&right)) {
        return evaluate_choice_binary(op, left, right);
    }
    evaluate_plain_binary(op, left, right)
}

fn binary_consumes_default_operands(op: &str) -> bool {
    matches!(
        op,
        "&&" | "||" | "==" | "!=" | "=~" | "!~" | "+" | "*" | "-" | "/" | "<" | "<=" | ">" | ">="
    )
}

fn evaluate_plain_binary(op: &str, left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    if let Some(value) = evaluate_incomplete_binary(op, &left, &right) {
        return value;
    }
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

fn should_distribute_binary(op: &str) -> bool {
    matches!(op, "+" | "*" | "-" | "/" | "<" | "<=" | ">" | ">=")
}

fn is_choice_value(value: &EvaluatedValue) -> bool {
    matches!(
        value,
        EvaluatedValue::Default(_) | EvaluatedValue::Disjunction(_)
    )
}

fn evaluate_choice_binary(op: &str, left: EvaluatedValue, right: EvaluatedValue) -> EvaluatedValue {
    let left_disjuncts = disjuncts_from(left);
    let right_disjuncts = disjuncts_from(right);
    let mut results =
        Vec::with_capacity(left_disjuncts.len().saturating_mul(right_disjuncts.len()));
    for left in &left_disjuncts {
        for right in &right_disjuncts {
            let value = evaluate_plain_binary(
                op,
                left.value.as_ref().clone(),
                right.value.as_ref().clone(),
            );
            if !matches!(value, EvaluatedValue::Bottom(_)) {
                results.push(Disjunct {
                    value: Box::new(value),
                    default: left.default || right.default,
                });
            }
        }
    }
    collapse_disjunction(EvaluatedValue::Disjunction(unique_disjuncts(results)))
}

fn evaluate_incomplete_binary(
    op: &str,
    left: &EvaluatedValue,
    right: &EvaluatedValue,
) -> Option<EvaluatedValue> {
    if should_distribute_binary(op)
        && (is_incomplete_binary_operand(left) || is_incomplete_binary_operand(right))
    {
        return Some(EvaluatedValue::Top);
    }
    None
}

fn is_incomplete_binary_operand(value: &EvaluatedValue) -> bool {
    matches!(value, EvaluatedValue::Top)
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
            EvaluatedValue::Struct(left)
            | EvaluatedValue::PatternedStruct { fields: left, .. }
            | EvaluatedValue::ClosedStruct(left)
            | EvaluatedValue::ClosedPatternedStruct { fields: left, .. },
            EvaluatedValue::Struct(right)
            | EvaluatedValue::PatternedStruct { fields: right, .. }
            | EvaluatedValue::ClosedStruct(right)
            | EvaluatedValue::ClosedPatternedStruct { fields: right, .. },
        ) => structs_equal(left, right),
        (EvaluatedValue::List(left), EvaluatedValue::List(right)) => lists_equal(left, right),
        (
            EvaluatedValue::OpenList {
                items: left_items,
                tail: left_tail,
            },
            EvaluatedValue::OpenList {
                items: right_items,
                tail: right_tail,
            },
        ) => lists_equal(left_items, right_items).and_then(|equal| {
            if equal {
                values_equal(left_tail, right_tail)
            } else {
                Ok(false)
            }
        }),
        (EvaluatedValue::Top, EvaluatedValue::Top)
        | (EvaluatedValue::Null, EvaluatedValue::Null) => Ok(true),
        (EvaluatedValue::Bool(left), EvaluatedValue::Bool(right)) => Ok(left == right),
        (EvaluatedValue::String(left), EvaluatedValue::String(right))
        | (EvaluatedValue::Builtin(left), EvaluatedValue::Builtin(right)) => Ok(left == right),
        (EvaluatedValue::Bytes(left), EvaluatedValue::Bytes(right)) => Ok(left == right),
        (EvaluatedValue::Kind(left), EvaluatedValue::Kind(right)) => Ok(left == right),
        (EvaluatedValue::NumericConstraint(left), EvaluatedValue::NumericConstraint(right)) => {
            Ok(left == right)
        }
        (EvaluatedValue::StringConstraints(left), EvaluatedValue::StringConstraints(right)) => {
            Ok(left == right)
        }
        (EvaluatedValue::StringConstraintSet(left), EvaluatedValue::StringConstraintSet(right)) => {
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
        (EvaluatedValue::FixpointPrevious(left), right) => values_equal(left, right),
        (left, EvaluatedValue::FixpointPrevious(right)) => values_equal(left, right),
        (EvaluatedValue::Disjunction(left), EvaluatedValue::Disjunction(right)) => {
            disjunctions_equal(left, right)
        }
        (EvaluatedValue::ComprehensionItems(left), EvaluatedValue::ComprehensionItems(right)) => {
            lists_equal(left, right)
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
        EvaluatedValue::Default(value) if matches!(value.as_ref(), EvaluatedValue::Bottom(_)) => {
            Vec::new()
        }
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
        EvaluatedValue::Default(value) | EvaluatedValue::FixpointPrevious(value) => {
            resolve_default_value(*value)
        }
        EvaluatedValue::Struct(fields) => EvaluatedValue::Struct(
            fields
                .into_iter()
                .map(|(label, value)| (label, resolve_default_value(value)))
                .collect(),
        ),
        EvaluatedValue::PatternedStruct { fields, patterns } => EvaluatedValue::PatternedStruct {
            fields: fields
                .into_iter()
                .map(|(label, value)| (label, resolve_default_value(value)))
                .collect(),
            patterns: patterns
                .into_iter()
                .map(|pattern| StructPatternConstraint {
                    pattern: Box::new(resolve_default_value(*pattern.pattern)),
                    value: Box::new(resolve_default_value(*pattern.value)),
                    span: pattern.span,
                })
                .collect(),
        },
        EvaluatedValue::ClosedStruct(fields) => EvaluatedValue::ClosedStruct(
            fields
                .into_iter()
                .map(|(label, value)| (label, resolve_default_value(value)))
                .collect(),
        ),
        EvaluatedValue::ClosedPatternedStruct { fields, patterns } => {
            EvaluatedValue::ClosedPatternedStruct {
                fields: fields
                    .into_iter()
                    .map(|(label, value)| (label, resolve_default_value(value)))
                    .collect(),
                patterns: patterns
                    .into_iter()
                    .map(|pattern| StructPatternConstraint {
                        pattern: Box::new(resolve_default_value(*pattern.pattern)),
                        value: Box::new(resolve_default_value(*pattern.value)),
                        span: pattern.span,
                    })
                    .collect(),
            }
        }
        EvaluatedValue::OptionalField(value) => {
            EvaluatedValue::OptionalField(Box::new(resolve_default_value(*value)))
        }
        EvaluatedValue::List(items) => {
            EvaluatedValue::List(items.into_iter().map(resolve_default_value).collect())
        }
        EvaluatedValue::OpenList { items, tail } => EvaluatedValue::OpenList {
            items: items.into_iter().map(resolve_default_value).collect(),
            tail: Box::new(resolve_default_value(*tail)),
        },
        EvaluatedValue::ComprehensionItems(items) => EvaluatedValue::ComprehensionItems(
            items.into_iter().map(resolve_default_value).collect(),
        ),
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

fn resolve_default_operand_value(value: EvaluatedValue) -> EvaluatedValue {
    match value {
        EvaluatedValue::FixpointPrevious(value) => strip_fixpoint_previous_values(*value),
        value => resolve_default_value(value),
    }
}

fn strip_fixpoint_previous_values(value: EvaluatedValue) -> EvaluatedValue {
    match value {
        EvaluatedValue::FixpointPrevious(value) => strip_fixpoint_previous_values(*value),
        EvaluatedValue::Struct(fields) => EvaluatedValue::Struct(
            fields
                .into_iter()
                .map(|(label, value)| (label, strip_fixpoint_previous_values(value)))
                .collect(),
        ),
        EvaluatedValue::PatternedStruct { fields, patterns } => EvaluatedValue::PatternedStruct {
            fields: fields
                .into_iter()
                .map(|(label, value)| (label, strip_fixpoint_previous_values(value)))
                .collect(),
            patterns: patterns
                .into_iter()
                .map(|pattern| StructPatternConstraint {
                    pattern: Box::new(strip_fixpoint_previous_values(*pattern.pattern)),
                    value: Box::new(strip_fixpoint_previous_values(*pattern.value)),
                    span: pattern.span,
                })
                .collect(),
        },
        EvaluatedValue::ClosedStruct(fields) => EvaluatedValue::ClosedStruct(
            fields
                .into_iter()
                .map(|(label, value)| (label, strip_fixpoint_previous_values(value)))
                .collect(),
        ),
        EvaluatedValue::ClosedPatternedStruct { fields, patterns } => {
            EvaluatedValue::ClosedPatternedStruct {
                fields: fields
                    .into_iter()
                    .map(|(label, value)| (label, strip_fixpoint_previous_values(value)))
                    .collect(),
                patterns: patterns
                    .into_iter()
                    .map(|pattern| StructPatternConstraint {
                        pattern: Box::new(strip_fixpoint_previous_values(*pattern.pattern)),
                        value: Box::new(strip_fixpoint_previous_values(*pattern.value)),
                        span: pattern.span,
                    })
                    .collect(),
            }
        }
        EvaluatedValue::OptionalField(value) => {
            EvaluatedValue::OptionalField(Box::new(strip_fixpoint_previous_values(*value)))
        }
        EvaluatedValue::List(items) => EvaluatedValue::List(
            items
                .into_iter()
                .map(strip_fixpoint_previous_values)
                .collect(),
        ),
        EvaluatedValue::OpenList { items, tail } => EvaluatedValue::OpenList {
            items: items
                .into_iter()
                .map(strip_fixpoint_previous_values)
                .collect(),
            tail: Box::new(strip_fixpoint_previous_values(*tail)),
        },
        EvaluatedValue::ComprehensionItems(items) => EvaluatedValue::ComprehensionItems(
            items
                .into_iter()
                .map(strip_fixpoint_previous_values)
                .collect(),
        ),
        EvaluatedValue::Disjunction(disjuncts) => EvaluatedValue::Disjunction(
            disjuncts
                .into_iter()
                .map(|disjunct| Disjunct {
                    value: Box::new(strip_fixpoint_previous_values(*disjunct.value)),
                    default: disjunct.default,
                })
                .collect(),
        ),
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

fn parse_exact_decimal(value: &str) -> Option<BigDecimal> {
    let parsed = parse_decimal_number(value)?;
    if !decimal_plain_string_fits(&parsed) {
        return None;
    }
    BigDecimal::from_str(&value.replace('_', "")).ok()
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

fn evaluated_decimal_number(name: &str, value: &BigDecimal) -> EvaluatedValue {
    format_decimal_number(value)
        .map_or_else(|| builtin_resource_exhausted(name), EvaluatedValue::Number)
}

fn format_decimal_number(value: &BigDecimal) -> Option<String> {
    let normalized = value.normalized();
    let compact = normalized.to_string();
    let parsed = parse_decimal_number(&compact)?;
    if !decimal_plain_string_fits(&parsed) {
        return None;
    }
    let plain = normalized.to_plain_string();
    (plain.len() <= MAX_BUILTIN_GENERATED_BYTES).then_some(plain)
}

fn decimal_plain_string_fits(value: &DecimalNumber) -> bool {
    decimal_plain_string_len(value).is_some_and(|len| len <= MAX_BUILTIN_GENERATED_BYTES)
}

fn decimal_plain_string_len(value: &DecimalNumber) -> Option<usize> {
    let digits_len = value.digits.len();
    let unsigned_len = if value.scale <= 0 {
        let zeroes = usize::try_from(value.scale.checked_neg()?).ok()?;
        digits_len.checked_add(zeroes)?
    } else {
        let scale = usize::try_from(value.scale).ok()?;
        if digits_len > scale {
            digits_len.checked_add(1)?
        } else {
            scale.checked_add(2)?
        }
    };
    if value.sign < 0 {
        unsigned_len.checked_add(1)
    } else {
        Some(unsigned_len)
    }
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

fn evaluate_index(
    base: EvaluatedValue,
    index: EvaluatedValue,
    defer_list_self_index: bool,
) -> EvaluatedValue {
    match base {
        EvaluatedValue::Bottom(bottom)
            if bottom.code == "cue.eval.structural_cycle"
                && defer_list_self_index
                && matches!(index, EvaluatedValue::Number(_)) =>
        {
            deferred_list_index(index)
        }
        EvaluatedValue::Bottom(bottom)
            if bottom.code == "cue.eval.structural_cycle"
                && defer_list_self_index
                && !matches!(index, EvaluatedValue::Number(_)) =>
        {
            invalid_list_index_type()
        }
        EvaluatedValue::Bottom(bottom) => EvaluatedValue::Bottom(bottom),
        EvaluatedValue::List(items) => evaluate_list_index(&items, index),
        EvaluatedValue::OpenList { items, tail } => evaluate_open_list_index(&items, &tail, index),
        EvaluatedValue::Struct(fields)
        | EvaluatedValue::PatternedStruct { fields, .. }
        | EvaluatedValue::ClosedStruct(fields)
        | EvaluatedValue::ClosedPatternedStruct { fields, .. } => {
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
        return invalid_list_index_type();
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

fn invalid_list_index_type() -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.invalid_index",
        "list index must be a non-negative integer",
        None,
        false,
    ))
}

fn resolve_deferred_list_indexes(items: &mut [EvaluatedValue], tail: Option<&EvaluatedValue>) {
    let snapshot = items.to_vec();
    for (index, item) in items.iter_mut().enumerate() {
        *item = resolve_deferred_list_index(index, &snapshot, tail);
    }
}

fn resolve_deferred_list_index(
    index: usize,
    snapshot: &[EvaluatedValue],
    tail: Option<&EvaluatedValue>,
) -> EvaluatedValue {
    let mut next = index;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(next) {
            return EvaluatedValue::Top;
        }
        let Some(next_value) = snapshot.get(next) else {
            return tail.cloned().unwrap_or_else(|| {
                EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.index_out_of_bounds",
                    format!("list index {next} is out of bounds"),
                    None,
                    false,
                ))
            });
        };
        let Some(deferred) = deferred_list_index_target(next_value) else {
            return next_value.clone();
        };
        next = deferred;
    }
}

fn deferred_list_index_target(value: &EvaluatedValue) -> Option<usize> {
    let EvaluatedValue::Bottom(bottom) = value else {
        return None;
    };
    if bottom.code != "cue.eval.deferred_list_index" {
        return None;
    }
    parse_list_index(&bottom.message)
}

fn evaluate_open_list_index(
    items: &[EvaluatedValue],
    tail: &EvaluatedValue,
    index: EvaluatedValue,
) -> EvaluatedValue {
    let EvaluatedValue::Number(index_text) = index else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_index",
            "list index must be a non-negative integer",
            None,
            false,
        ));
    };
    let Some(index) = parse_list_index(&index_text) else {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.invalid_index",
            format!("invalid list index `{index_text}`"),
            None,
            false,
        ));
    };
    items.get(index).cloned().unwrap_or_else(|| tail.clone())
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
    let items = match base {
        EvaluatedValue::List(items) => items,
        EvaluatedValue::OpenList { .. } => {
            return EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.unsupported_open_list_slice",
                "cannot slice open list values",
                None,
                false,
            ));
        }
        _ => {
            return EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.invalid_slice_base",
                "cannot slice non-list value",
                None,
                false,
            ));
        }
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

#[allow(
    clippy::too_many_lines,
    reason = "central CUE unification dispatch stays grouped so each value-pair rule remains \
              auditable"
)]
fn unify_values(left: EvaluatedValue, right: EvaluatedValue, span: Option<Span>) -> EvaluatedValue {
    match (left, right) {
        (EvaluatedValue::Top, value) | (value, EvaluatedValue::Top) => value,
        (EvaluatedValue::Bottom(bottom), _) | (_, EvaluatedValue::Bottom(bottom)) => {
            EvaluatedValue::Bottom(bottom)
        }
        (EvaluatedValue::FixpointPrevious(left), right) => unify_values(*left, right, span),
        (left, EvaluatedValue::FixpointPrevious(right)) => unify_values(left, *right, span),
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
        (EvaluatedValue::StringConstraints(mut left), EvaluatedValue::StringConstraints(right)) => {
            left.extend(right);
            EvaluatedValue::StringConstraints(left)
        }
        (
            EvaluatedValue::StringConstraints(runes),
            EvaluatedValue::RegexConstraint { pattern, negated },
        )
        | (
            EvaluatedValue::RegexConstraint { pattern, negated },
            EvaluatedValue::StringConstraints(runes),
        ) => EvaluatedValue::StringConstraintSet(StringConstraintSet {
            runes,
            regexes: vec![RegexStringConstraint { pattern, negated }],
        }),
        (
            EvaluatedValue::RegexConstraint {
                pattern: left_pattern,
                negated: left_negated,
            },
            EvaluatedValue::RegexConstraint {
                pattern: right_pattern,
                negated: right_negated,
            },
        ) => EvaluatedValue::StringConstraintSet(StringConstraintSet {
            runes: Vec::new(),
            regexes: vec![
                RegexStringConstraint {
                    pattern: left_pattern,
                    negated: left_negated,
                },
                RegexStringConstraint {
                    pattern: right_pattern,
                    negated: right_negated,
                },
            ],
        }),
        (
            EvaluatedValue::StringConstraintSet(mut left),
            EvaluatedValue::StringConstraintSet(right),
        ) => {
            left.runes.extend(right.runes);
            left.regexes.extend(right.regexes);
            EvaluatedValue::StringConstraintSet(left)
        }
        (
            EvaluatedValue::StringConstraintSet(mut constraints),
            EvaluatedValue::StringConstraints(runes),
        )
        | (
            EvaluatedValue::StringConstraints(runes),
            EvaluatedValue::StringConstraintSet(mut constraints),
        ) => {
            constraints.runes.extend(runes);
            EvaluatedValue::StringConstraintSet(constraints)
        }
        (
            EvaluatedValue::StringConstraintSet(mut constraints),
            EvaluatedValue::RegexConstraint { pattern, negated },
        )
        | (
            EvaluatedValue::RegexConstraint { pattern, negated },
            EvaluatedValue::StringConstraintSet(mut constraints),
        ) => {
            constraints
                .regexes
                .push(RegexStringConstraint { pattern, negated });
            EvaluatedValue::StringConstraintSet(constraints)
        }
        (EvaluatedValue::StringConstraints(constraints), EvaluatedValue::String(value))
        | (EvaluatedValue::String(value), EvaluatedValue::StringConstraints(constraints)) => {
            unify_string_constraints(value, &constraints)
        }
        (EvaluatedValue::StringConstraintSet(constraints), EvaluatedValue::String(value))
        | (EvaluatedValue::String(value), EvaluatedValue::StringConstraintSet(constraints)) => {
            unify_string_constraint_set(value, &constraints)
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
        (EvaluatedValue::Builtin(left), EvaluatedValue::Builtin(right)) if left == right => {
            EvaluatedValue::Builtin(left)
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
        (
            EvaluatedValue::ClosedPatternedStruct {
                fields: left,
                patterns: left_patterns,
            },
            EvaluatedValue::ClosedPatternedStruct {
                fields: right,
                patterns: right_patterns,
            },
        ) => unify_closed_patterned_structs(left, &left_patterns, &right, right_patterns, span),
        (
            EvaluatedValue::ClosedPatternedStruct { fields, patterns },
            EvaluatedValue::Struct(other) | EvaluatedValue::ClosedStruct(other),
        )
        | (
            EvaluatedValue::Struct(other) | EvaluatedValue::ClosedStruct(other),
            EvaluatedValue::ClosedPatternedStruct { fields, patterns },
        ) => unify_closed_patterned_struct(fields, patterns, other, Vec::new(), span),
        (
            EvaluatedValue::ClosedPatternedStruct {
                fields,
                patterns: closed_patterns,
            },
            EvaluatedValue::PatternedStruct {
                fields: other,
                patterns: open_patterns,
            },
        )
        | (
            EvaluatedValue::PatternedStruct {
                fields: other,
                patterns: open_patterns,
            },
            EvaluatedValue::ClosedPatternedStruct {
                fields,
                patterns: closed_patterns,
            },
        ) => unify_closed_patterned_struct(fields, closed_patterns, other, open_patterns, span),
        (EvaluatedValue::ClosedStruct(left), EvaluatedValue::ClosedStruct(right)) => {
            unify_closed_structs(left, right, span)
        }
        (
            EvaluatedValue::PatternedStruct {
                fields: left,
                patterns: left_patterns,
            },
            EvaluatedValue::PatternedStruct {
                fields: right,
                patterns: right_patterns,
            },
        ) => unify_patterned_structs(left, left_patterns, right, right_patterns, span),
        (EvaluatedValue::PatternedStruct { fields, patterns }, EvaluatedValue::Struct(other))
        | (EvaluatedValue::Struct(other), EvaluatedValue::PatternedStruct { fields, patterns }) => {
            unify_patterned_structs(fields, patterns, other, Vec::new(), span)
        }
        (
            EvaluatedValue::ClosedStruct(left),
            EvaluatedValue::PatternedStruct {
                fields: right,
                patterns,
            },
        )
        | (
            EvaluatedValue::PatternedStruct {
                fields: right,
                patterns,
            },
            EvaluatedValue::ClosedStruct(left),
        ) => unify_closed_struct(left, fields_after_patterns(right, &patterns), span),
        (EvaluatedValue::ClosedStruct(left), EvaluatedValue::Struct(right))
        | (EvaluatedValue::Struct(right), EvaluatedValue::ClosedStruct(left)) => {
            unify_closed_struct(left, right, span)
        }
        (EvaluatedValue::Struct(left), EvaluatedValue::Struct(right)) => {
            EvaluatedValue::Struct(unify_structs(left, right, span))
        }
        (EvaluatedValue::List(left), EvaluatedValue::List(right)) => {
            unify_closed_lists(left, right, span)
        }
        (EvaluatedValue::ComprehensionItems(left), EvaluatedValue::ComprehensionItems(right)) => {
            EvaluatedValue::ComprehensionItems(
                left.into_iter()
                    .zip(right)
                    .map(|(left, right)| unify_values(left, right, span))
                    .collect(),
            )
        }
        (EvaluatedValue::OpenList { items, tail }, EvaluatedValue::List(closed))
        | (EvaluatedValue::List(closed), EvaluatedValue::OpenList { items, tail }) => {
            unify_open_with_closed_list(&items, &tail, closed, span)
        }
        (
            EvaluatedValue::OpenList {
                items: left_items,
                tail: left_tail,
            },
            EvaluatedValue::OpenList {
                items: right_items,
                tail: right_tail,
            },
        ) => unify_open_lists(&left_items, &left_tail, &right_items, &right_tail, span),
        (left, right) => conflict_bottom(left.kind(), right.kind(), span, "cue.eval.conflict"),
    }
}

fn unify_closed_lists(
    left: Vec<EvaluatedValue>,
    right: Vec<EvaluatedValue>,
    span: Option<Span>,
) -> EvaluatedValue {
    if left.len() != right.len() {
        return list_length_conflict(span);
    }
    EvaluatedValue::List(
        left.into_iter()
            .zip(right)
            .map(|(left, right)| unify_values(left, right, span))
            .collect(),
    )
}

fn unify_open_with_closed_list(
    items: &[EvaluatedValue],
    tail: &EvaluatedValue,
    closed: Vec<EvaluatedValue>,
    span: Option<Span>,
) -> EvaluatedValue {
    if closed.len() < items.len() {
        return list_length_conflict(span);
    }
    let mut unified = Vec::with_capacity(closed.len());
    for (index, value) in closed.into_iter().enumerate() {
        let constraint = items.get(index).cloned().unwrap_or_else(|| tail.clone());
        unified.push(unify_values(constraint, value, span));
    }
    EvaluatedValue::List(unified)
}

fn unify_open_lists(
    left_items: &[EvaluatedValue],
    left_tail: &EvaluatedValue,
    right_items: &[EvaluatedValue],
    right_tail: &EvaluatedValue,
    span: Option<Span>,
) -> EvaluatedValue {
    let prefix_len = left_items.len().max(right_items.len());
    let mut items = Vec::with_capacity(prefix_len);
    for index in 0..prefix_len {
        let left = left_items
            .get(index)
            .cloned()
            .unwrap_or_else(|| left_tail.clone());
        let right = right_items
            .get(index)
            .cloned()
            .unwrap_or_else(|| right_tail.clone());
        items.push(unify_values(left, right, span));
    }
    EvaluatedValue::OpenList {
        items,
        tail: Box::new(unify_values(left_tail.clone(), right_tail.clone(), span)),
    }
}

fn list_length_conflict(span: Option<Span>) -> EvaluatedValue {
    EvaluatedValue::Bottom(Bottom::new(
        "cue.eval.list_length_conflict",
        "incompatible list lengths",
        span,
        false,
    ))
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
            format!(
                "invalid value {value} for numeric constraint {}",
                format_numeric_bounds(bounds),
            ),
            None,
            false,
        )),
        Err(bottom) => EvaluatedValue::Bottom(bottom),
    }
}

fn unify_string_constraints(value: String, constraints: &[StringConstraint]) -> EvaluatedValue {
    let rune_count = value.chars().count();
    if let Some(constraint) = constraints
        .iter()
        .find(|constraint| !string_constraint_satisfied(rune_count, constraint))
    {
        return EvaluatedValue::Bottom(Bottom::new(
            "cue.eval.string_constraint_mismatch",
            format!(
                "invalid value {value:?} for string constraint {}",
                format_string_constraint(constraint),
            ),
            None,
            false,
        ));
    }
    EvaluatedValue::String(value)
}

fn unify_string_constraint_set(value: String, constraints: &StringConstraintSet) -> EvaluatedValue {
    let value = match unify_string_constraints(value, &constraints.runes) {
        EvaluatedValue::String(value) => value,
        bottom @ EvaluatedValue::Bottom(_) => return bottom,
        other => {
            return EvaluatedValue::Bottom(Bottom::new(
                "cue.eval.invalid_string_constraint",
                format!("string constraint produced {}", other.kind()),
                None,
                false,
            ));
        }
    };
    for regex in &constraints.regexes {
        match unify_regex_constraint(
            EvaluatedValue::String(value.clone()),
            &regex.pattern,
            regex.negated,
        ) {
            EvaluatedValue::String(_) => {}
            bottom @ EvaluatedValue::Bottom(_) => return bottom,
            other => {
                return EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.invalid_string_constraint",
                    format!("regex string constraint produced {}", other.kind()),
                    None,
                    false,
                ));
            }
        }
    }
    EvaluatedValue::String(value)
}

fn string_constraint_satisfied(rune_count: usize, constraint: &StringConstraint) -> bool {
    let direction = match constraint.op {
        StringConstraintOp::MinRunes => Ordering::Greater,
        StringConstraintOp::MaxRunes => Ordering::Less,
    };
    rune_count_satisfies(rune_count, constraint.limit, direction)
}

fn format_string_constraint(constraint: &StringConstraint) -> String {
    format!("{}({})", constraint.op.builtin_name(), constraint.limit)
}

fn format_numeric_bounds(bounds: &[NumericBound]) -> String {
    bounds
        .iter()
        .map(|bound| format!("{}{}", bound.op.as_str(), bound.value))
        .collect::<Vec<_>>()
        .join(" & ")
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

fn unify_patterned_structs(
    left: IndexMap<String, EvaluatedValue>,
    mut left_patterns: Vec<StructPatternConstraint>,
    right: IndexMap<String, EvaluatedValue>,
    right_patterns: Vec<StructPatternConstraint>,
    span: Option<Span>,
) -> EvaluatedValue {
    let mut fields = unify_structs(left, right, span);
    left_patterns.extend(right_patterns);
    apply_struct_patterns(&left_patterns, &mut fields);
    if left_patterns.is_empty() {
        EvaluatedValue::Struct(fields)
    } else {
        EvaluatedValue::PatternedStruct {
            fields,
            patterns: left_patterns,
        }
    }
}

fn fields_after_patterns(
    mut fields: IndexMap<String, EvaluatedValue>,
    patterns: &[StructPatternConstraint],
) -> IndexMap<String, EvaluatedValue> {
    apply_struct_patterns(patterns, &mut fields);
    fields
}

fn close_recursive(value: EvaluatedValue) -> EvaluatedValue {
    match resolve_default_value(value) {
        EvaluatedValue::Struct(fields) => EvaluatedValue::ClosedStruct(
            fields
                .into_iter()
                .map(|(label, value)| (label, close_recursive(value)))
                .collect(),
        ),
        EvaluatedValue::PatternedStruct { fields, patterns } => {
            EvaluatedValue::ClosedPatternedStruct {
                fields: fields
                    .into_iter()
                    .map(|(label, value)| (label, close_recursive(value)))
                    .collect(),
                patterns: patterns
                    .into_iter()
                    .map(|pattern| StructPatternConstraint {
                        pattern: Box::new(close_recursive(*pattern.pattern)),
                        value: Box::new(close_recursive(*pattern.value)),
                        span: pattern.span,
                    })
                    .collect(),
            }
        }
        EvaluatedValue::ClosedStruct(fields) => EvaluatedValue::ClosedStruct(
            fields
                .into_iter()
                .map(|(label, value)| (label, close_recursive(value)))
                .collect(),
        ),
        EvaluatedValue::ClosedPatternedStruct { fields, patterns } => {
            EvaluatedValue::ClosedPatternedStruct {
                fields: fields
                    .into_iter()
                    .map(|(label, value)| (label, close_recursive(value)))
                    .collect(),
                patterns: patterns
                    .into_iter()
                    .map(|pattern| StructPatternConstraint {
                        pattern: Box::new(close_recursive(*pattern.pattern)),
                        value: Box::new(close_recursive(*pattern.value)),
                        span: pattern.span,
                    })
                    .collect(),
            }
        }
        EvaluatedValue::List(items) => {
            EvaluatedValue::List(items.into_iter().map(close_recursive).collect())
        }
        EvaluatedValue::OpenList { items, tail } => EvaluatedValue::OpenList {
            items: items.into_iter().map(close_recursive).collect(),
            tail: Box::new(close_recursive(*tail)),
        },
        EvaluatedValue::Disjunction(disjuncts) => EvaluatedValue::Disjunction(
            disjuncts
                .into_iter()
                .map(|disjunct| Disjunct {
                    value: Box::new(close_recursive(*disjunct.value)),
                    default: disjunct.default,
                })
                .collect(),
        ),
        EvaluatedValue::Default(value) => {
            EvaluatedValue::Default(Box::new(close_recursive(*value)))
        }
        EvaluatedValue::OptionalField(value) => {
            EvaluatedValue::OptionalField(Box::new(close_recursive(*value)))
        }
        other => other,
    }
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

fn unify_closed_patterned_struct(
    closed: IndexMap<String, EvaluatedValue>,
    patterns: Vec<StructPatternConstraint>,
    open: IndexMap<String, EvaluatedValue>,
    open_patterns: Vec<StructPatternConstraint>,
    span: Option<Span>,
) -> EvaluatedValue {
    let mut fields = closed;
    for (label, right_value) in open {
        if let Some(left_value) = fields.shift_remove(&label) {
            fields.insert(label, unify_values(left_value, right_value, span));
            continue;
        }
        let pattern_value = match pattern_value_for_label(&patterns, &label) {
            Ok(Some(value)) => value,
            Ok(None) => {
                return EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.closed_struct",
                    format!("field `{label}` not allowed in closed struct"),
                    span,
                    false,
                ));
            }
            Err(bottom) => return EvaluatedValue::Bottom(bottom),
        };
        fields.insert(label, unify_values(pattern_value, right_value, span));
    }
    let mut patterns = patterns;
    patterns.extend(open_patterns);
    EvaluatedValue::ClosedPatternedStruct { fields, patterns }
}

fn unify_closed_patterned_structs(
    left: IndexMap<String, EvaluatedValue>,
    left_patterns: &[StructPatternConstraint],
    right: &IndexMap<String, EvaluatedValue>,
    right_patterns: Vec<StructPatternConstraint>,
    span: Option<Span>,
) -> EvaluatedValue {
    let left_unified = unify_closed_patterned_struct(
        left,
        left_patterns.to_owned(),
        right.clone(),
        Vec::new(),
        span,
    );
    let EvaluatedValue::ClosedPatternedStruct {
        fields,
        patterns: mut combined_patterns,
    } = left_unified
    else {
        return left_unified;
    };
    let labels = fields.keys().cloned().collect::<Vec<_>>();
    let mut fields = fields;
    for label in labels {
        if right.contains_key(&label) {
            continue;
        }
        match pattern_value_for_label(&right_patterns, &label) {
            Ok(Some(pattern_value)) => {
                let Some(existing) = fields.shift_remove(&label) else {
                    continue;
                };
                fields.insert(label, unify_values(existing, pattern_value, span));
            }
            Ok(None) => {
                return EvaluatedValue::Bottom(Bottom::new(
                    "cue.eval.closed_struct",
                    format!("field `{label}` not allowed in closed struct"),
                    span,
                    false,
                ));
            }
            Err(bottom) => return EvaluatedValue::Bottom(bottom),
        }
    }
    combined_patterns.extend(right_patterns);
    EvaluatedValue::ClosedPatternedStruct {
        fields,
        patterns: combined_patterns,
    }
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
        | (
            ValueKind::String,
            EvaluatedValue::String(_)
            | EvaluatedValue::RegexConstraint { .. }
            | EvaluatedValue::StringConstraints(_)
            | EvaluatedValue::StringConstraintSet(_),
        )
        | (ValueKind::Bytes, EvaluatedValue::Bytes(_))
        | (
            ValueKind::Struct,
            EvaluatedValue::Struct(_)
            | EvaluatedValue::PatternedStruct { .. }
            | EvaluatedValue::ClosedStruct(_)
            | EvaluatedValue::ClosedPatternedStruct { .. },
        )
        | (ValueKind::List, EvaluatedValue::List(_) | EvaluatedValue::OpenList { .. })
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
        | EvaluatedValue::Builtin(_)
        | EvaluatedValue::NumericConstraint(_)
        | EvaluatedValue::StringConstraints(_)
        | EvaluatedValue::StringConstraintSet(_)
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
        EvaluatedValue::Struct(fields)
        | EvaluatedValue::PatternedStruct { fields, .. }
        | EvaluatedValue::ClosedStruct(fields)
        | EvaluatedValue::ClosedPatternedStruct { fields, .. } => {
            for (label, field) in fields {
                let field_path = format!("{path}.{label}");
                validate_value(field, options, &field_path, report);
                if report.has_errors() && !options.all_errors {
                    return;
                }
            }
        }
        EvaluatedValue::List(items) | EvaluatedValue::ComprehensionItems(items) => {
            for (index, item) in items.iter().enumerate() {
                let item_path = format!("{path}[{index}]");
                validate_value(item, options, &item_path, report);
                if report.has_errors() && !options.all_errors {
                    return;
                }
            }
        }
        EvaluatedValue::OpenList { items, tail } => {
            let inner_options = ValidateOptions {
                concrete: false,
                ..options
            };
            for (index, item) in items.iter().enumerate() {
                let item_path = format!("{path}[{index}]");
                validate_value(item, inner_options, &item_path, report);
                if report.has_errors() && !options.all_errors {
                    return;
                }
            }
            validate_value(tail, inner_options, path, report);
            if report.has_errors() && !options.all_errors {
                return;
            }
            if options.concrete {
                report.push(Diagnostic::new(
                    Severity::Error,
                    "cue.eval.incomplete_open_list",
                    format!("{path}: open list is not concrete data"),
                    None,
                ));
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
        "_" => Some(ValueKind::Top),
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

fn static_local_fields(members: &[StructMember]) -> IndexMap<Feature, LocalField> {
    members
        .iter()
        .filter_map(|member| {
            let StructMember::Field(field) = member else {
                return None;
            };
            let FieldLabel::Static(feature) = &field.label else {
                return None;
            };
            Some((
                *feature,
                LocalField {
                    expression: field.expression,
                    metadata: field.metadata,
                },
            ))
        })
        .collect()
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
