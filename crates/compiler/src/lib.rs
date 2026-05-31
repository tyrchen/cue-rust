//! Compiler boundary between parsed CUE syntax and semantic ADT values.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::collections::BTreeSet;

use cue_rust_adt::{
    AdtError, BaseValue, Bottom, Conjunct, Environment, ExprId, Feature, FieldExpr, Runtime,
    SemanticExpr, Vertex, VertexId,
};
use cue_rust_loader::BuildInstance;
use cue_rust_source::{Diagnostic, DiagnosticReport, Severity, Span};
use cue_rust_syntax::{AstFile, Decl, Expr, Label};
use thiserror::Error;

/// Compiler options shared by lowering passes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct CompileOptions {
    /// Whether experimental syntax and semantic features are allowed.
    pub allow_experimental: bool,
}

/// Infrastructure errors from compilation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CompileError {
    /// ADT runtime construction failed.
    #[error(transparent)]
    Adt(#[from] AdtError),
}

/// Compiled build instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledInstance {
    root: VertexId,
    diagnostics: DiagnosticReport,
}

impl CompiledInstance {
    /// Creates a compiled instance.
    #[must_use]
    pub fn new(root: VertexId, diagnostics: DiagnosticReport) -> Self {
        Self { root, diagnostics }
    }

    /// Returns the root vertex id.
    #[must_use]
    pub fn root(&self) -> VertexId {
        self.root
    }

    /// Returns compiler diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &DiagnosticReport {
        &self.diagnostics
    }
}

/// AST-to-ADT compiler.
#[derive(Debug)]
pub struct Compiler<'runtime> {
    runtime: &'runtime mut Runtime,
    diagnostics: DiagnosticReport,
    scope: BTreeSet<String>,
}

impl<'runtime> Compiler<'runtime> {
    /// Creates a compiler over a runtime.
    #[must_use]
    pub fn new(runtime: &'runtime mut Runtime) -> Self {
        Self {
            runtime,
            diagnostics: DiagnosticReport::new(),
            scope: BTreeSet::new(),
        }
    }

    /// Compiles a build instance into a root vertex.
    ///
    /// # Errors
    ///
    /// Returns [`CompileError`] when ADT arena construction fails.
    pub fn compile_instance(
        mut self,
        instance: &BuildInstance,
        _options: CompileOptions,
    ) -> Result<CompiledInstance, CompileError> {
        self.diagnostics
            .extend(instance.diagnostics().diagnostics().iter().cloned());
        self.collect_scope(instance.files());

        let root = self
            .runtime
            .add_vertex(Vertex::new(None, None, BaseValue::Struct))?;
        let root_env = self.runtime.add_environment(Environment {
            parent: None,
            vertex: root,
        })?;

        for file in instance.files() {
            self.lower_file(file, root, root_env)?;
        }

        Ok(CompiledInstance::new(root, self.diagnostics))
    }

    fn collect_scope(&mut self, files: &[AstFile]) {
        for file in files {
            for declaration in &file.declarations {
                if let Decl::Field(field) = declaration {
                    self.scope.insert(field.label.display_name().to_owned());
                }
            }
        }
    }

    fn lower_file(
        &mut self,
        file: &AstFile,
        root: VertexId,
        environment: cue_rust_adt::EnvironmentId,
    ) -> Result<(), CompileError> {
        for import in &file.imports {
            let _expr = self.runtime.add_expression(SemanticExpr::ImportReference {
                path: import.path.clone(),
            })?;
        }
        for declaration in &file.declarations {
            self.lower_decl(declaration, root, environment)?;
        }
        Ok(())
    }

    fn lower_decl(
        &mut self,
        declaration: &Decl,
        parent: VertexId,
        environment: cue_rust_adt::EnvironmentId,
    ) -> Result<(), CompileError> {
        match declaration {
            Decl::Field(field) => {
                let feature = self.feature_for_label(&field.label);
                let child = self.ensure_child(parent, feature)?;
                let expression = self.lower_expr(&field.value)?;
                let conjunct = Conjunct {
                    environment,
                    expression,
                    span: Some(field.span),
                };
                self.runtime.add_conjunct(child, conjunct)?;
            }
            Decl::Let(let_decl) => {
                let expression = self.lower_expr(&let_decl.value)?;
                let conjunct = Conjunct {
                    environment,
                    expression,
                    span: Some(let_decl.span),
                };
                self.runtime.add_conjunct(parent, conjunct)?;
            }
            Decl::Ellipsis(_) => {}
            Decl::Bad(span) => self.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "cue.compile.bad_decl",
                "cannot compile recovered declaration",
                Some(*span),
            )),
            _ => self.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "cue.compile.unsupported_decl",
                "unsupported declaration",
                None,
            )),
        }
        Ok(())
    }

    fn ensure_child(
        &mut self,
        parent: VertexId,
        feature: Feature,
    ) -> Result<VertexId, CompileError> {
        if let Some(existing) = self
            .runtime
            .vertex(parent)?
            .arcs
            .get(&feature)
            .map(|arc| arc.target)
        {
            return Ok(existing);
        }
        let child =
            self.runtime
                .add_vertex(Vertex::new(Some(parent), Some(feature), BaseValue::Top))?;
        self.runtime.add_arc(
            parent,
            cue_rust_adt::Arc {
                feature,
                target: child,
                optional: false,
                definition: false,
                hidden: false,
            },
        )?;
        Ok(child)
    }

    fn lower_expr(&mut self, expression: &Expr) -> Result<ExprId, CompileError> {
        let lowered = match expression {
            Expr::Identifier(name, span) => self.lower_identifier(name, *span),
            Expr::Number(value, _) => SemanticExpr::Base(BaseValue::Number(value.clone())),
            Expr::String(value, _) => SemanticExpr::Base(BaseValue::String(unquote_string(value))),
            Expr::Bool(value, _) => SemanticExpr::Base(BaseValue::Bool(*value)),
            Expr::Null(_) => SemanticExpr::Base(BaseValue::Null),
            Expr::Struct(declarations, _) => {
                let mut fields = Vec::new();
                for declaration in declarations {
                    if let Decl::Field(field) = declaration {
                        let expression = self.lower_expr(&field.value)?;
                        let feature = self.feature_for_label(&field.label);
                        fields.push(FieldExpr {
                            feature,
                            expression,
                            span: Some(field.span),
                        });
                    }
                }
                SemanticExpr::Struct(fields)
            }
            Expr::List(items, _) => {
                let mut lowered_items = Vec::with_capacity(items.len());
                for item in items {
                    lowered_items.push(self.lower_expr(item)?);
                }
                SemanticExpr::List(lowered_items)
            }
            Expr::Selector { base, field, .. } => {
                let base = self.lower_expr(base)?;
                let feature = self.runtime.features.string(field);
                SemanticExpr::Selector { base, feature }
            }
            Expr::Unary { op, expr, .. } => {
                let expr = self.lower_expr(expr)?;
                SemanticExpr::Unary {
                    op: op.clone(),
                    expr,
                }
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let left = self.lower_expr(left)?;
                let right = self.lower_expr(right)?;
                SemanticExpr::Binary {
                    op: op.clone(),
                    left,
                    right,
                }
            }
            Expr::Default(expr, _) => {
                let expr = self.lower_expr(expr)?;
                SemanticExpr::Default(expr)
            }
            Expr::Ellipsis(_) => SemanticExpr::Base(BaseValue::Top),
            Expr::Bad(span) => SemanticExpr::Bottom(Bottom::new(
                "cue.compile.bad_expr",
                "cannot compile recovered expression",
                Some(*span),
                false,
            )),
            _ => SemanticExpr::Bottom(Bottom::new(
                "cue.compile.unsupported_expr",
                "unsupported expression",
                None,
                false,
            )),
        };
        Ok(self.runtime.add_expression(lowered)?)
    }

    fn lower_identifier(&mut self, name: &str, span: Span) -> SemanticExpr {
        if is_builtin_kind(name) {
            return SemanticExpr::Base(BaseValue::Builtin(name.to_owned()));
        }
        if self.scope.contains(name) {
            let feature = self.runtime.features.string(name);
            SemanticExpr::FieldReference {
                feature,
                up_count: 0,
            }
        } else {
            self.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "cue.compile.unresolved_identifier",
                format!("unresolved identifier `{name}`"),
                Some(span),
            ));
            SemanticExpr::Bottom(Bottom::new(
                "cue.compile.unresolved_identifier",
                format!("unresolved identifier `{name}`"),
                Some(span),
                false,
            ))
        }
    }

    fn feature_for_label(&mut self, label: &Label) -> Feature {
        self.runtime.features.string(label.display_name())
    }
}

fn unquote_string(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .map_or_else(|| value.to_owned(), ToOwned::to_owned)
}

fn is_builtin_kind(name: &str) -> bool {
    matches!(name, "_" | "bool" | "int" | "null" | "number" | "string")
}

#[cfg(test)]
mod tests {
    use cue_rust_adt::{BaseValue, Runtime, SemanticExpr};
    use cue_rust_loader::BuildInstance;
    use cue_rust_syntax::{ParseConfig, parse_bytes};

    use super::{CompileOptions, Compiler};

    #[test]
    fn test_should_lower_duplicate_fields_to_child_conjuncts()
    -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_bytes("test.cue", b"x: 1\nx: 2\n", ParseConfig::default());
        assert!(!parsed.diagnostics().has_errors());
        let files = parsed.ast().map_or_else(Vec::new, |ast| vec![ast.clone()]);
        let instance = BuildInstance::new(None, files);
        let mut runtime = Runtime::default();
        let compiled =
            Compiler::new(&mut runtime).compile_instance(&instance, CompileOptions::default())?;
        let graph = runtime.debug_graph(compiled.root())?;
        assert!(graph.contains("arc x -> 2"));
        assert!(graph.contains("conjuncts=2"));
        Ok(())
    }

    #[test]
    fn test_should_report_unresolved_identifier() -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_bytes("test.cue", b"x: y\n", ParseConfig::default());
        let files = parsed.ast().map_or_else(Vec::new, |ast| vec![ast.clone()]);
        let instance = BuildInstance::new(None, files);
        let mut runtime = Runtime::default();
        let compiled =
            Compiler::new(&mut runtime).compile_instance(&instance, CompileOptions::default())?;
        assert!(compiled.diagnostics().has_errors());
        Ok(())
    }

    #[test]
    fn test_should_lower_field_reference() -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_bytes("test.cue", b"x: 1\ny: x\n", ParseConfig::default());
        let files = parsed.ast().map_or_else(Vec::new, |ast| vec![ast.clone()]);
        let instance = BuildInstance::new(None, files);
        let mut runtime = Runtime::default();
        let _compiled =
            Compiler::new(&mut runtime).compile_instance(&instance, CompileOptions::default())?;
        let has_reference = (1..=16).any(|id| {
            cue_rust_adt::ExprId::new(id)
                .ok()
                .and_then(|expr_id| runtime.expression(expr_id).ok())
                .is_some_and(|expr| matches!(expr, SemanticExpr::FieldReference { .. }))
        });
        assert!(has_reference);
        let has_number = (1..=16).any(|id| {
            cue_rust_adt::ExprId::new(id)
                .ok()
                .and_then(|expr_id| runtime.expression(expr_id).ok())
                .is_some_and(|expr| matches!(expr, SemanticExpr::Base(BaseValue::Number(value)) if value == "1"))
        });
        assert!(has_number);
        Ok(())
    }
}
