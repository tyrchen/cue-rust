//! Semantic graph and runtime data structures for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::num::NonZeroU32;

use cue_rust_source::Span;
use indexmap::IndexMap;
use thiserror::Error;

/// Runtime configuration shared by semantic graph construction.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeConfig {
    deterministic_export: bool,
}

impl RuntimeConfig {
    /// Creates a runtime configuration.
    #[must_use]
    pub fn new(deterministic_export: bool) -> Self {
        Self {
            deterministic_export,
        }
    }

    /// Returns whether exports should preserve deterministic field ordering.
    #[must_use]
    pub fn deterministic_export(&self) -> bool {
        self.deterministic_export
    }
}

/// Errors produced by semantic runtime construction.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum AdtError {
    /// Arena ids currently fit in `u32`.
    #[error("ADT arena exhausted")]
    ArenaExhausted,
    /// A requested id does not exist in its arena.
    #[error("missing ADT id {id}")]
    MissingId {
        /// Missing one-based id.
        id: u32,
    },
}

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(NonZeroU32);

        impl $name {
            /// Creates an id from a one-based integer.
            ///
            /// # Errors
            ///
            /// Returns [`AdtError::MissingId`] when `value` is zero.
            pub fn new(value: u32) -> Result<Self, AdtError> {
                NonZeroU32::new(value)
                    .map(Self)
                    .ok_or(AdtError::MissingId { id: value })
            }

            /// Returns the underlying one-based integer.
            #[must_use]
            pub fn get(self) -> u32 {
                self.0.get()
            }

            fn from_index(index: usize) -> Result<Self, AdtError> {
                let one_based = index.checked_add(1).ok_or(AdtError::ArenaExhausted)?;
                let value = u32::try_from(one_based).map_err(|_| AdtError::ArenaExhausted)?;
                Self::new(value)
            }

            /// Returns the zero-based arena index for this id.
            ///
            /// # Errors
            ///
            /// Returns [`AdtError::MissingId`] if the id cannot be converted.
            pub fn to_index(self) -> Result<usize, AdtError> {
                let zero_based = self
                    .get()
                    .checked_sub(1)
                    .ok_or(AdtError::MissingId { id: self.get() })?;
                usize::try_from(zero_based).map_err(|_| AdtError::MissingId { id: self.get() })
            }
        }
    };
}

id_type!(Feature, "A compact interned CUE feature label.");
id_type!(VertexId, "A stable id for a semantic vertex.");
id_type!(ExprId, "A stable id for a semantic expression.");
id_type!(EnvironmentId, "A stable id for a lexical environment.");
id_type!(ConjunctId, "A stable id for a source conjunct.");

/// Kind of interned feature.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum FeatureKind {
    /// Regular string field.
    String,
    /// Integer list index.
    Int,
    /// Definition field.
    Definition,
    /// Hidden field.
    Hidden,
    /// Let binding.
    Let,
}

/// Interned feature metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InternedFeature {
    /// Feature id.
    pub feature: Feature,
    /// Feature kind.
    pub kind: FeatureKind,
    /// Display label.
    pub label: String,
}

/// Context-local feature interner.
#[derive(Clone, Debug)]
pub struct FeatureInterner {
    by_key: IndexMap<(FeatureKind, String), Feature>,
    features: Vec<InternedFeature>,
    any: Feature,
}

impl Default for FeatureInterner {
    fn default() -> Self {
        let any = Feature::from_index(0).unwrap_or(Feature(NonZeroU32::MIN));
        let mut interner = Self {
            by_key: IndexMap::new(),
            features: Vec::new(),
            any,
        };
        let _inserted = interner.intern(FeatureKind::String, "_");
        interner
    }
}

impl FeatureInterner {
    /// Creates a feature interner with the reserved `_` feature.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the reserved `_` feature.
    #[must_use]
    pub fn any(&self) -> Feature {
        self.any
    }

    /// Interns a string feature.
    #[must_use]
    pub fn string(&mut self, label: &str) -> Feature {
        self.intern(FeatureKind::String, label)
    }

    /// Interns a feature by kind and label.
    #[must_use]
    pub fn intern(&mut self, kind: FeatureKind, label: &str) -> Feature {
        let key = (kind, label.to_owned());
        if let Some(feature) = self.by_key.get(&key).copied() {
            return feature;
        }
        let feature = Feature::from_index(self.features.len()).unwrap_or(self.any);
        self.features.push(InternedFeature {
            feature,
            kind,
            label: label.to_owned(),
        });
        self.by_key.insert(key, feature);
        feature
    }

    /// Looks up interned feature metadata.
    #[must_use]
    pub fn lookup(&self, feature: Feature) -> Option<&InternedFeature> {
        feature
            .to_index()
            .ok()
            .and_then(|index| self.features.get(index))
    }
}

/// Scalar or marker value stored in semantic expressions and vertices.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BaseValue {
    /// Top value.
    Top,
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Exact numeric literal text.
    Number(String),
    /// String value.
    String(String),
    /// Bytes value.
    Bytes(Vec<u8>),
    /// Struct marker.
    Struct,
    /// List marker.
    List,
    /// Builtin marker.
    Builtin(String),
}

/// Semantic expression stored in the expression arena.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SemanticExpr {
    /// Base value expression.
    Base(BaseValue),
    /// Struct expression represented by field conjunct ids.
    Struct(Vec<ConjunctId>),
    /// List expression represented by item expression ids.
    List(Vec<ExprId>),
    /// Field reference with lexical up-count.
    FieldReference {
        /// Referenced feature.
        feature: Feature,
        /// Lexical environment hops.
        up_count: u32,
    },
    /// Bottom expression.
    Bottom(Bottom),
}

/// Vertex evaluation status.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum VertexStatus {
    /// Vertex has not been finalized.
    #[default]
    Unevaluated,
    /// Vertex is currently being finalized.
    Evaluating,
    /// Vertex has been finalized.
    Finalized,
}

/// Arc from a vertex to a child vertex.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Arc {
    /// Feature label for this arc.
    pub feature: Feature,
    /// Child vertex id.
    pub target: VertexId,
    /// Whether the arc is optional.
    pub optional: bool,
    /// Whether the arc is a definition.
    pub definition: bool,
    /// Whether the arc is hidden.
    pub hidden: bool,
}

/// Semantic graph vertex.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Vertex {
    /// Optional parent vertex.
    pub parent: Option<VertexId>,
    /// Optional feature from the parent.
    pub feature: Option<Feature>,
    /// Child arcs keyed by feature.
    pub arcs: IndexMap<Feature, Arc>,
    /// Source conjuncts contributing to this vertex.
    pub conjuncts: Vec<ConjunctId>,
    /// Base value or marker.
    pub base: BaseValue,
    /// Evaluation status.
    pub status: VertexStatus,
    /// Semantic bottom, if this vertex is invalid.
    pub bottom: Option<Bottom>,
}

impl Vertex {
    /// Creates an empty vertex.
    #[must_use]
    pub fn new(parent: Option<VertexId>, feature: Option<Feature>, base: BaseValue) -> Self {
        Self {
            parent,
            feature,
            arcs: IndexMap::new(),
            conjuncts: Vec::new(),
            base,
            status: VertexStatus::Unevaluated,
            bottom: None,
        }
    }
}

/// Lexical environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Environment {
    /// Parent lexical environment.
    pub parent: Option<EnvironmentId>,
    /// Current vertex.
    pub vertex: VertexId,
}

/// Source conjunct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conjunct {
    /// Lexical environment.
    pub environment: EnvironmentId,
    /// Semantic expression.
    pub expression: ExprId,
    /// Optional source span.
    pub span: Option<Span>,
}

/// Semantic bottom value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bottom {
    /// Stable diagnostic code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Optional source span.
    pub span: Option<Span>,
    /// Whether this bottom represents incompleteness rather than conflict.
    pub incomplete: bool,
}

impl Bottom {
    /// Creates a bottom value.
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        span: Option<Span>,
        incomplete: bool,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            span,
            incomplete,
        }
    }
}

/// Builtin package registry entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Builtin {
    /// Builtin name.
    pub name: String,
}

/// Builtin package registry skeleton.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BuiltinRegistry {
    builtins: IndexMap<String, Builtin>,
}

impl BuiltinRegistry {
    /// Registers a builtin name.
    pub fn register(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.builtins.insert(name.clone(), Builtin { name });
    }

    /// Looks up a builtin.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Builtin> {
        self.builtins.get(name)
    }
}

/// Semantic runtime with context-local arenas.
#[derive(Clone, Debug)]
pub struct Runtime {
    /// Runtime configuration.
    pub config: RuntimeConfig,
    /// Feature interner.
    pub features: FeatureInterner,
    expressions: Vec<SemanticExpr>,
    vertices: Vec<Vertex>,
    environments: Vec<Environment>,
    conjuncts: Vec<Conjunct>,
    /// Builtin registry.
    pub builtins: BuiltinRegistry,
}

impl Runtime {
    /// Creates an empty runtime.
    #[must_use]
    pub fn new(config: RuntimeConfig) -> Self {
        Self {
            config,
            features: FeatureInterner::new(),
            expressions: Vec::new(),
            vertices: Vec::new(),
            environments: Vec::new(),
            conjuncts: Vec::new(),
            builtins: BuiltinRegistry::default(),
        }
    }

    /// Adds a semantic expression to the arena.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError::ArenaExhausted`] if the arena id cannot fit in `u32`.
    pub fn add_expression(&mut self, expression: SemanticExpr) -> Result<ExprId, AdtError> {
        let id = ExprId::from_index(self.expressions.len())?;
        self.expressions.push(expression);
        Ok(id)
    }

    /// Adds a vertex to the arena.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError::ArenaExhausted`] if the arena id cannot fit in `u32`.
    pub fn add_vertex(&mut self, vertex: Vertex) -> Result<VertexId, AdtError> {
        let id = VertexId::from_index(self.vertices.len())?;
        self.vertices.push(vertex);
        Ok(id)
    }

    /// Adds an environment to the arena.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError::ArenaExhausted`] if the arena id cannot fit in `u32`.
    pub fn add_environment(&mut self, environment: Environment) -> Result<EnvironmentId, AdtError> {
        let id = EnvironmentId::from_index(self.environments.len())?;
        self.environments.push(environment);
        Ok(id)
    }

    /// Adds a conjunct and attaches it to a vertex.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError`] if the target vertex is missing or arena ids are exhausted.
    pub fn add_conjunct(
        &mut self,
        vertex: VertexId,
        conjunct: Conjunct,
    ) -> Result<ConjunctId, AdtError> {
        let id = ConjunctId::from_index(self.conjuncts.len())?;
        self.conjuncts.push(conjunct);
        self.vertex_mut(vertex)?.conjuncts.push(id);
        Ok(id)
    }

    /// Adds or replaces a child arc on a vertex.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError::MissingId`] if the parent vertex is missing.
    pub fn add_arc(&mut self, parent: VertexId, arc: Arc) -> Result<(), AdtError> {
        self.vertex_mut(parent)?.arcs.insert(arc.feature, arc);
        Ok(())
    }

    /// Returns a vertex.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError::MissingId`] if the vertex does not exist.
    pub fn vertex(&self, id: VertexId) -> Result<&Vertex, AdtError> {
        self.vertices
            .get(id.to_index()?)
            .ok_or(AdtError::MissingId { id: id.get() })
    }

    /// Returns a mutable vertex.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError::MissingId`] if the vertex does not exist.
    pub fn vertex_mut(&mut self, id: VertexId) -> Result<&mut Vertex, AdtError> {
        self.vertices
            .get_mut(id.to_index()?)
            .ok_or(AdtError::MissingId { id: id.get() })
    }

    /// Returns an expression.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError::MissingId`] if the expression does not exist.
    pub fn expression(&self, id: ExprId) -> Result<&SemanticExpr, AdtError> {
        self.expressions
            .get(id.to_index()?)
            .ok_or(AdtError::MissingId { id: id.get() })
    }

    /// Returns a stable debug rendering of a graph rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`AdtError::MissingId`] if any traversed vertex is missing.
    pub fn debug_graph(&self, root: VertexId) -> Result<String, AdtError> {
        let mut lines = Vec::new();
        self.push_vertex_debug(root, 0, &mut lines)?;
        Ok(lines.join("\n"))
    }

    fn push_vertex_debug(
        &self,
        vertex_id: VertexId,
        depth: usize,
        lines: &mut Vec<String>,
    ) -> Result<(), AdtError> {
        let vertex = self.vertex(vertex_id)?;
        let indent = "  ".repeat(depth);
        lines.push(format!(
            "{indent}vertex {} base={:?} conjuncts={}",
            vertex_id.get(),
            vertex.base,
            vertex.conjuncts.len()
        ));
        for arc in vertex.arcs.values() {
            let label = self
                .features
                .lookup(arc.feature)
                .map_or("<unknown>", |feature| feature.label.as_str());
            lines.push(format!("{indent}  arc {label} -> {}", arc.target.get()));
            self.push_vertex_debug(arc.target, depth + 2, lines)?;
        }
        Ok(())
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new(RuntimeConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Arc, BaseValue, Conjunct, Environment, FeatureInterner, Runtime, RuntimeConfig,
        SemanticExpr, Vertex,
    };

    #[test]
    fn test_should_round_trip_feature_interning() {
        let mut interner = FeatureInterner::new();
        let first = interner.string("service");
        let second = interner.string("service");
        assert_eq!(first, second);
        let label = interner.lookup(first).map(|feature| feature.label.as_str());
        assert_eq!(Some("service"), label);
    }

    #[test]
    fn test_should_represent_duplicate_fields_as_multiple_conjuncts() -> Result<(), super::AdtError>
    {
        let mut runtime = Runtime::new(RuntimeConfig::default());
        let root = runtime.add_vertex(Vertex::new(None, None, BaseValue::Struct))?;
        let environment = runtime.add_environment(Environment {
            parent: None,
            vertex: root,
        })?;
        let first_expr =
            runtime.add_expression(SemanticExpr::Base(BaseValue::Number("1".into())))?;
        let second_expr =
            runtime.add_expression(SemanticExpr::Base(BaseValue::Number("2".into())))?;
        let first = Conjunct {
            environment,
            expression: first_expr,
            span: None,
        };
        let second = Conjunct {
            environment,
            expression: second_expr,
            span: None,
        };
        runtime.add_conjunct(root, first)?;
        runtime.add_conjunct(root, second)?;
        assert_eq!(2, runtime.vertex(root)?.conjuncts.len());
        Ok(())
    }

    #[test]
    fn test_should_export_debug_graph() -> Result<(), super::AdtError> {
        let mut runtime = Runtime::default();
        let feature = runtime.features.string("x");
        let root = runtime.add_vertex(Vertex::new(None, None, BaseValue::Struct))?;
        let child = runtime.add_vertex(Vertex::new(Some(root), Some(feature), BaseValue::Top))?;
        runtime.add_arc(
            root,
            Arc {
                feature,
                target: child,
                optional: false,
                definition: false,
                hidden: false,
            },
        )?;
        let graph = runtime.debug_graph(root)?;
        assert!(graph.contains("arc x -> 2"));
        Ok(())
    }
}
