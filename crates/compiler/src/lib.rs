//! Compiler boundary between parsed CUE syntax and semantic ADT values.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::collections::{BTreeMap, BTreeSet};

use cue_rust_adt::{
    AdtError, BaseValue, Bottom, Comprehension, Conjunct, Environment, ExprId, Feature,
    FeatureKind, FieldExpr, FieldLabel, FieldMetadata, Runtime, SemanticExpr, StringSegment,
    StructMember, Vertex, VertexId,
};
use cue_rust_loader::BuildInstance;
use cue_rust_source::{Diagnostic, DiagnosticReport, Severity, Span};
use cue_rust_syntax::{
    AstFile, ComprehensionClause as AstComprehensionClause, Decl, Expr, FieldDecl, FieldMarker,
    Label, ParseConfig, StringPart,
};
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
    scopes: Vec<Scope>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Scope {
    fields: BTreeMap<String, String>,
    imports: BTreeMap<String, String>,
    lets: BTreeMap<String, ExprId>,
    dynamic_bindings: BTreeSet<String>,
}

impl<'runtime> Compiler<'runtime> {
    /// Creates a compiler over a runtime.
    #[must_use]
    pub fn new(runtime: &'runtime mut Runtime) -> Self {
        Self {
            runtime,
            diagnostics: DiagnosticReport::new(),
            scopes: Vec::new(),
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
        let root = self
            .runtime
            .add_vertex(Vertex::new(None, None, BaseValue::Struct))?;
        let root_env = self.runtime.add_environment(Environment {
            parent: None,
            vertex: root,
        })?;

        self.lower_imports(instance.imports(), root, root_env)?;

        let root_scope = Self::collect_scope(instance.files());
        self.scopes.push(root_scope);
        for file in instance.files() {
            self.lower_file(instance.imports(), file, root, root_env)?;
        }
        self.scopes.pop();

        Ok(CompiledInstance::new(root, self.diagnostics))
    }

    fn collect_scope(files: &[AstFile]) -> Scope {
        let mut scope = Scope::default();
        for file in files {
            for declaration in &file.declarations {
                if let Decl::Field(field) = declaration {
                    register_field_bindings(&mut scope, field);
                }
            }
        }
        scope
    }

    fn lower_file(
        &mut self,
        imports: &BTreeMap<String, Vec<AstFile>>,
        file: &AstFile,
        root: VertexId,
        environment: cue_rust_adt::EnvironmentId,
    ) -> Result<(), CompileError> {
        for import in &file.imports {
            let path = unquote_string(&import.path);
            if is_supported_builtin_import(&path) {
                if let Some(scope) = self.scopes.last_mut() {
                    let local_name = import
                        .alias
                        .clone()
                        .unwrap_or_else(|| import_name(&path).to_owned());
                    scope.imports.insert(local_name, path);
                }
            } else if imports.contains_key(&path) {
                if let Some(scope) = self.scopes.last_mut() {
                    let local_name = import
                        .alias
                        .clone()
                        .unwrap_or_else(|| import_name(&path).to_owned());
                    scope.imports.insert(local_name, path);
                }
            } else {
                self.runtime.add_expression(SemanticExpr::ImportReference {
                    path: import.path.clone(),
                })?;
                self.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "cue.compile.unsupported_import",
                    format!("import {} is not loaded by the current loader", import.path),
                    Some(import.span),
                ));
            }
        }
        for declaration in &file.declarations {
            if let Decl::Let(let_decl) = declaration {
                let expression = self.lower_expr(&let_decl.value)?;
                if let Some(scope) = self.scopes.last_mut() {
                    scope.lets.insert(let_decl.name.clone(), expression);
                }
            }
        }
        for declaration in &file.declarations {
            self.lower_decl(declaration, root, environment)?;
        }
        Ok(())
    }

    fn lower_imports(
        &mut self,
        imports: &BTreeMap<String, Vec<AstFile>>,
        root: VertexId,
        environment: cue_rust_adt::EnvironmentId,
    ) -> Result<(), CompileError> {
        for (path, files) in imports {
            self.lower_import_files(path, files, imports, root, environment)?;
        }
        Ok(())
    }

    fn lower_import_files(
        &mut self,
        path: &str,
        files: &[AstFile],
        imports: &BTreeMap<String, Vec<AstFile>>,
        root: VertexId,
        environment: cue_rust_adt::EnvironmentId,
    ) -> Result<(), CompileError> {
        let scope = Self::collect_qualified_import_scope(path, files);
        self.scopes.push(scope);
        for file in files {
            self.register_file_imports(imports, file)?;
        }
        for file in files {
            for declaration in &file.declarations {
                if let Decl::Let(let_decl) = declaration {
                    let expression = self.lower_expr(&let_decl.value)?;
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.lets.insert(let_decl.name.clone(), expression);
                    }
                }
            }
        }
        for file in files {
            for declaration in &file.declarations {
                match declaration {
                    Decl::Field(field) => {
                        self.lower_import_field(path, field, root, environment)?;
                    }
                    Decl::Let(_) | Decl::Ellipsis(_) => {}
                    Decl::Bad(span) => self.diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        "cue.compile.bad_import_decl",
                        "cannot compile recovered import declaration",
                        Some(*span),
                    )),
                    _ => self.diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        "cue.compile.unsupported_import_decl",
                        format!("unsupported declaration in import `{path}`"),
                        None,
                    )),
                }
            }
        }
        self.scopes.pop();
        Ok(())
    }

    fn register_file_imports(
        &mut self,
        imports: &BTreeMap<String, Vec<AstFile>>,
        file: &AstFile,
    ) -> Result<(), CompileError> {
        for import in &file.imports {
            let path = unquote_string(&import.path);
            if is_supported_builtin_import(&path) || imports.contains_key(&path) {
                if let Some(scope) = self.scopes.last_mut() {
                    let local_name = import
                        .alias
                        .clone()
                        .unwrap_or_else(|| import_name(&path).to_owned());
                    scope.imports.insert(local_name, path);
                }
            } else {
                self.runtime.add_expression(SemanticExpr::ImportReference {
                    path: import.path.clone(),
                })?;
                self.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "cue.compile.unsupported_import",
                    format!("import {} is not loaded by the current loader", import.path),
                    Some(import.span),
                ));
            }
        }
        Ok(())
    }

    fn collect_qualified_import_scope(path: &str, files: &[AstFile]) -> Scope {
        let mut scope = Scope::default();
        for file in files {
            for declaration in &file.declarations {
                if let Decl::Field(field) = declaration {
                    register_qualified_import_bindings(&mut scope, path, field);
                }
            }
        }
        scope
    }

    fn lower_import_field(
        &mut self,
        path: &str,
        field: &FieldDecl,
        root: VertexId,
        environment: cue_rust_adt::EnvironmentId,
    ) -> Result<(), CompileError> {
        let Some(label) = static_import_label(&field.label) else {
            self.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "cue.compile.unsupported_import_label",
                format!("unsupported dynamic import field in `{path}`"),
                Some(field.span),
            ));
            return Ok(());
        };
        let qualified = qualify_import_field(path, &label);
        let feature = self.feature_for_name(&qualified);
        let metadata = FieldMetadata::regular(false, true);
        let child = self.ensure_child(root, feature, metadata)?;
        let expression = self.lower_expr(&field.value)?;
        let conjunct = Conjunct {
            environment,
            expression,
            span: Some(field.span),
        };
        self.runtime.add_conjunct(child, conjunct)?;
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
                let metadata = Self::metadata_for_field(&field.label, field.marker);
                let expression = self.lower_expr(&field.value)?;
                let label = self.lower_label(&field.label)?;
                let FieldLabel::Static(feature) = label else {
                    let struct_expression =
                        self.runtime.add_expression(SemanticExpr::Struct(vec![
                            StructMember::Field(FieldExpr {
                                label,
                                metadata,
                                expression,
                                span: Some(field.span),
                            }),
                        ]))?;
                    let conjunct = Conjunct {
                        environment,
                        expression: struct_expression,
                        span: Some(field.span),
                    };
                    self.runtime.add_conjunct(parent, conjunct)?;
                    return Ok(());
                };
                let child = self.ensure_child(parent, feature, metadata)?;
                let conjunct = Conjunct {
                    environment,
                    expression,
                    span: Some(field.span),
                };
                self.runtime.add_conjunct(child, conjunct)?;
            }
            Decl::Comprehension(comprehension) => {
                let expression = self.lower_comprehension_decl(comprehension)?;
                let conjunct = Conjunct {
                    environment,
                    expression,
                    span: Some(comprehension.span),
                };
                self.runtime.add_conjunct(parent, conjunct)?;
            }
            Decl::Let(_) | Decl::Ellipsis(_) => {}
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
        metadata: FieldMetadata,
    ) -> Result<VertexId, CompileError> {
        if let Some(existing) = self
            .runtime
            .vertex(parent)?
            .arcs
            .get(&feature)
            .map(|arc| arc.target)
        {
            self.runtime
                .vertex_mut(parent)?
                .arcs
                .entry(feature)
                .and_modify(|arc| {
                    arc.metadata.merge(metadata);
                });
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
                metadata,
            },
        )?;
        Ok(child)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "AST expression lowering is a single exhaustive dispatch over the public syntax \
                  enum"
    )]
    fn lower_expr(&mut self, expression: &Expr) -> Result<ExprId, CompileError> {
        let lowered = match expression {
            Expr::Identifier(name, span) => self.lower_identifier(name, *span),
            Expr::Number(value, _) => SemanticExpr::Base(BaseValue::Number(value.clone())),
            Expr::String(value, _) => SemanticExpr::Base(BaseValue::String(unquote_string(value))),
            Expr::InterpolatedString { parts, .. } => {
                SemanticExpr::InterpolatedString(self.lower_string_parts(parts)?)
            }
            Expr::Bytes(value, _) => SemanticExpr::Base(BaseValue::Bytes(unquote_bytes(value))),
            Expr::Bool(value, _) => SemanticExpr::Base(BaseValue::Bool(*value)),
            Expr::Null(_) => SemanticExpr::Base(BaseValue::Null),
            Expr::Struct(declarations, _) => self.lower_struct_expr(declarations)?,
            Expr::List { items, tail, .. } => {
                let mut lowered_items = Vec::with_capacity(items.len());
                for item in items {
                    lowered_items.push(self.lower_expr(item)?);
                }
                let lowered_tail = tail
                    .as_ref()
                    .map(|tail| self.lower_list_tail(tail))
                    .transpose()?;
                SemanticExpr::List {
                    items: lowered_items,
                    tail: lowered_tail,
                }
            }
            Expr::Selector { base, field, .. } => {
                if let Some(path) = self.import_path_for_selector_base(base) {
                    if !is_supported_builtin_import(&path) {
                        let name = qualify_import_field(&path, field);
                        let feature = self.feature_for_name(&name);
                        return Ok(self.runtime.add_expression(SemanticExpr::FieldReference {
                            feature,
                            up_count: 0,
                        })?);
                    }
                    return Ok(self.runtime.add_expression(SemanticExpr::Base(
                        BaseValue::Builtin(format!("{path}.{field}")),
                    ))?);
                }
                let base = self.lower_expr(base)?;
                let feature = self.feature_for_name(field);
                SemanticExpr::Selector { base, feature }
            }
            Expr::Index { base, index, .. } => {
                let base = self.lower_expr(base)?;
                let index = self.lower_expr(index)?;
                SemanticExpr::Index { base, index }
            }
            Expr::Slice {
                base, start, end, ..
            } => {
                let base = self.lower_expr(base)?;
                let start = start
                    .as_ref()
                    .map(|start| self.lower_expr(start))
                    .transpose()?;
                let end = end.as_ref().map(|end| self.lower_expr(end)).transpose()?;
                SemanticExpr::Slice { base, start, end }
            }
            Expr::Call { callee, args, .. } => {
                let callee = self.lower_expr(callee)?;
                let mut lowered_args = Vec::with_capacity(args.len());
                for arg in args {
                    lowered_args.push(self.lower_expr(arg)?);
                }
                SemanticExpr::Call {
                    callee,
                    args: lowered_args,
                }
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
            Expr::Comprehension { clauses, body, .. } => {
                let comprehension = self.lower_comprehension_expr(clauses, body)?;
                SemanticExpr::Comprehension(comprehension)
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

    fn lower_list_tail(&mut self, tail: &Expr) -> Result<ExprId, CompileError> {
        match tail {
            Expr::Ellipsis(_) => Ok(self
                .runtime
                .add_expression(SemanticExpr::Base(BaseValue::Top))?),
            _ => self.lower_expr(tail),
        }
    }

    fn lower_struct_expr(&mut self, declarations: &[Decl]) -> Result<SemanticExpr, CompileError> {
        let mut scope = Scope::default();
        for declaration in declarations {
            if let Decl::Field(field) = declaration {
                register_field_bindings(&mut scope, field);
            }
        }

        let mut members = Vec::new();
        self.scopes.push(scope);
        for declaration in declarations {
            if let Decl::Let(let_decl) = declaration {
                let expression = self.lower_expr(&let_decl.value)?;
                if let Some(scope) = self.scopes.last_mut() {
                    scope.lets.insert(let_decl.name.clone(), expression);
                }
            }
        }
        for declaration in declarations {
            if let Decl::Field(field) = declaration {
                let expression = self.lower_expr(&field.value)?;
                let label = self.lower_label(&field.label)?;
                let metadata = Self::metadata_for_field(&field.label, field.marker);
                members.push(StructMember::Field(FieldExpr {
                    label,
                    metadata,
                    expression,
                    span: Some(field.span),
                }));
            } else if let Decl::Comprehension(comprehension) = declaration {
                members.push(StructMember::Comprehension(
                    self.lower_comprehension_decl_value(comprehension)?,
                ));
            }
        }
        self.scopes.pop();
        Ok(SemanticExpr::Struct(members))
    }

    fn lower_identifier(&mut self, name: &str, span: Span) -> SemanticExpr {
        if let Some(expression) = self.resolve_let(name) {
            return SemanticExpr::LetReference { expression };
        }
        if self.resolve_dynamic_binding(name) {
            return SemanticExpr::DynamicReference {
                name: name.to_owned(),
            };
        }
        if let Some(path) = self.resolve_import(name) {
            return SemanticExpr::ImportReference { path };
        }
        if let Some(label) = self.resolve_field(name) {
            let feature = self.feature_for_name(&label);
            return SemanticExpr::FieldReference {
                feature,
                up_count: 0,
            };
        }
        if is_builtin_name(name) {
            return SemanticExpr::Base(BaseValue::Builtin(name.to_owned()));
        }
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

    fn lower_label(&mut self, label: &Label) -> Result<FieldLabel, CompileError> {
        match label {
            Label::String(value, _) if interpolation_body(value).is_some() => {
                let expression = self.lower_interpolated_literal(value)?;
                Ok(FieldLabel::Dynamic(expression))
            }
            Label::Dynamic(expression, _) => Ok(FieldLabel::Dynamic(self.lower_expr(expression)?)),
            Label::Pattern(expression, _) => Ok(FieldLabel::Pattern(self.lower_expr(expression)?)),
            _ => Ok(FieldLabel::Static(self.feature_for_label(label))),
        }
    }

    fn lower_string_parts(
        &mut self,
        parts: &[StringPart],
    ) -> Result<Vec<StringSegment>, CompileError> {
        parts
            .iter()
            .map(|part| match part {
                StringPart::Text(value) => Ok(StringSegment::Text(unescape_string_segment(value))),
                StringPart::Expr(expression) => {
                    self.lower_expr(expression).map(StringSegment::Expr)
                }
                _ => Ok(StringSegment::Text(String::new())),
            })
            .collect()
    }

    fn lower_interpolated_literal(&mut self, value: &str) -> Result<ExprId, CompileError> {
        let parts = match interpolation_body(value) {
            Some(body) => match parse_interpolated_literal_parts(body) {
                Ok(parts) => parts,
                Err(diagnostic) => {
                    self.diagnostics.push(diagnostic.clone());
                    return Ok(self
                        .runtime
                        .add_expression(SemanticExpr::Bottom(Bottom::new(
                            diagnostic.code(),
                            diagnostic.message().to_owned(),
                            diagnostic.primary_span(),
                            false,
                        )))?);
                }
            },
            None => vec![StringPart::Text(unquote_string(value))],
        };
        let segments = self.lower_string_parts(&parts)?;
        Ok(self
            .runtime
            .add_expression(SemanticExpr::InterpolatedString(segments))?)
    }

    fn lower_comprehension_decl(
        &mut self,
        comprehension: &cue_rust_syntax::ComprehensionDecl,
    ) -> Result<ExprId, CompileError> {
        let value = self.lower_comprehension_decl_value(comprehension)?;
        Ok(self
            .runtime
            .add_expression(SemanticExpr::Comprehension(value))?)
    }

    fn lower_comprehension_decl_value(
        &mut self,
        comprehension: &cue_rust_syntax::ComprehensionDecl,
    ) -> Result<Comprehension, CompileError> {
        self.scopes.push(Scope::default());
        let clauses = self.lower_comprehension_clauses(&comprehension.clauses)?;
        let body = self.lower_struct_expr(&comprehension.body)?;
        let body = self.runtime.add_expression(body)?;
        self.scopes.pop();
        Ok(Comprehension { clauses, body })
    }

    fn lower_comprehension_expr(
        &mut self,
        clauses: &[AstComprehensionClause],
        body: &Expr,
    ) -> Result<Comprehension, CompileError> {
        self.scopes.push(Scope::default());
        let clauses = self.lower_comprehension_clauses(clauses)?;
        let body = self.lower_expr(body)?;
        self.scopes.pop();
        Ok(Comprehension { clauses, body })
    }

    fn lower_comprehension_clauses(
        &mut self,
        clauses: &[AstComprehensionClause],
    ) -> Result<Vec<cue_rust_adt::ComprehensionClause>, CompileError> {
        let mut lowered = Vec::with_capacity(clauses.len());
        for clause in clauses {
            match clause {
                AstComprehensionClause::For {
                    key, value, source, ..
                } => {
                    let source = self.lower_expr(source)?;
                    if let Some(scope) = self.scopes.last_mut() {
                        if let Some(key) = key {
                            scope.dynamic_bindings.insert(key.clone());
                        }
                        scope.dynamic_bindings.insert(value.clone());
                    }
                    lowered.push(cue_rust_adt::ComprehensionClause::For {
                        key: key.clone(),
                        value: value.clone(),
                        source,
                    });
                }
                AstComprehensionClause::If { condition, .. } => {
                    let condition = self.lower_expr(condition)?;
                    lowered.push(cue_rust_adt::ComprehensionClause::If { condition });
                }
                _ => {}
            }
        }
        Ok(lowered)
    }

    fn feature_for_label(&mut self, label: &Label) -> Feature {
        let name = label_name(label);
        let kind = match label {
            Label::Identifier(_, _) => feature_kind_for_label(&name),
            _ => FeatureKind::String,
        };
        self.runtime.features.intern(kind, &name)
    }

    fn feature_for_name(&mut self, name: &str) -> Feature {
        let kind = feature_kind_for_label(name);
        self.runtime.features.intern(kind, name)
    }

    fn metadata_for_field(label: &Label, marker: FieldMarker) -> FieldMetadata {
        let name = label_name(label);
        let is_identifier = matches!(label, Label::Identifier(_, _));
        let hidden = is_identifier && is_hidden_label(&name);
        let definition = is_identifier && is_definition_label(&name);
        match marker {
            FieldMarker::Optional => FieldMetadata::optional(definition, hidden),
            FieldMarker::Required => FieldMetadata::required(definition, hidden),
            _ => FieldMetadata::regular(definition, hidden),
        }
    }

    fn resolve_let(&self, name: &str) -> Option<ExprId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.lets.get(name).copied())
    }

    fn resolve_import(&self, name: &str) -> Option<String> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.imports.get(name).cloned())
    }

    fn resolve_field(&self, name: &str) -> Option<String> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.fields.get(name).cloned())
    }

    fn resolve_dynamic_binding(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .any(|scope| scope.dynamic_bindings.contains(name))
    }

    fn import_path_for_selector_base(&self, base: &Expr) -> Option<String> {
        let Expr::Identifier(name, _) = base else {
            return None;
        };
        self.resolve_import(name)
    }
}

fn qualify_import_field(path: &str, field: &str) -> String {
    format!("{path}.{field}")
}

fn unquote_string(value: &str) -> String {
    let unquoted = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);
    String::from_utf8_lossy(&unescape_literal_bytes(unquoted)).into_owned()
}

fn interpolation_body(value: &str) -> Option<&str> {
    let body = value.strip_prefix('"')?.strip_suffix('"')?;
    body.contains("\\(").then_some(body)
}

fn parse_interpolated_literal_parts(body: &str) -> Result<Vec<StringPart>, Diagnostic> {
    let mut parts = Vec::new();
    let mut literal_start = 0_usize;
    let mut cursor = 0_usize;
    while let Some(relative_start) = body.get(cursor..).and_then(|value| value.find("\\(")) {
        let Some(start) = cursor.checked_add(relative_start) else {
            break;
        };
        if start > literal_start
            && let Some(text) = body.get(literal_start..start)
        {
            parts.push(StringPart::Text(text.to_owned()));
        }
        let Some(expression_start) = start.checked_add(2) else {
            break;
        };
        let Some(expression_end) = interpolation_end(body, expression_start) else {
            return Err(Diagnostic::new(
                Severity::Error,
                "cue.compile.invalid_interpolation",
                "string interpolation is missing a closing parenthesis",
                None,
            ));
        };
        let expression_text = body
            .get(expression_start..expression_end)
            .unwrap_or_default()
            .trim();
        parts.push(StringPart::Expr(Box::new(parse_interpolation_expression(
            expression_text,
        )?)));
        let Some(next_cursor) = expression_end.checked_add(1) else {
            break;
        };
        cursor = next_cursor;
        literal_start = cursor;
    }
    if literal_start < body.len()
        && let Some(text) = body.get(literal_start..)
    {
        parts.push(StringPart::Text(text.to_owned()));
    }
    Ok(parts)
}

fn parse_interpolation_expression(expression: &str) -> Result<Expr, Diagnostic> {
    let source = format!("__cue_rs_interp: ({expression})\n");
    let parsed = cue_rust_syntax::parse_bytes(
        "__cue_rs_interp.cue",
        source.as_bytes(),
        ParseConfig::default(),
    );
    if parsed.diagnostics().has_errors() {
        return Err(Diagnostic::new(
            Severity::Error,
            "cue.compile.invalid_interpolation",
            "string interpolation contains an invalid expression",
            None,
        ));
    }
    parsed
        .ast()
        .and_then(|ast| ast.declarations.first())
        .and_then(|decl| match decl {
            Decl::Field(field) => Some(field.value.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            Diagnostic::new(
                Severity::Error,
                "cue.compile.invalid_interpolation",
                "string interpolation did not produce an expression",
                None,
            )
        })
}

fn interpolation_end(body: &str, expression_start: usize) -> Option<usize> {
    let mut depth = 1_u32;
    let mut quote = None;
    let mut escaped = false;
    for (offset, byte) in body.bytes().enumerate().skip(expression_start) {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn unescape_string_segment(value: &str) -> String {
    String::from_utf8_lossy(&unescape_literal_bytes(value)).into_owned()
}

fn import_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn label_name(label: &Label) -> String {
    match label {
        Label::Identifier(name, _) => name.clone(),
        Label::String(name, _) => unquote_string(name),
        Label::Bad(_) => "<bad>".to_owned(),
        _ => label.display_name().to_owned(),
    }
}

fn register_field_bindings(scope: &mut Scope, field: &FieldDecl) {
    let label = label_name(&field.label);
    scope.fields.insert(label.clone(), label.clone());
    if let Some(alias) = &field.alias {
        scope.fields.insert(alias.clone(), label);
    }
}

fn register_qualified_import_bindings(scope: &mut Scope, path: &str, field: &FieldDecl) {
    let Some(label) = static_import_label(&field.label) else {
        return;
    };
    let qualified = qualify_import_field(path, &label);
    scope.fields.insert(label.clone(), qualified.clone());
    if let Some(alias) = &field.alias {
        scope.fields.insert(alias.clone(), qualified);
    }
}

fn static_import_label(label: &Label) -> Option<String> {
    match label {
        Label::Identifier(_, _) | Label::String(_, _) => Some(label_name(label)),
        _ => None,
    }
}

fn feature_kind_for_label(label: &str) -> FeatureKind {
    if is_hidden_label(label) {
        FeatureKind::Hidden
    } else if is_definition_label(label) {
        FeatureKind::Definition
    } else {
        FeatureKind::String
    }
}

fn is_definition_label(label: &str) -> bool {
    label.starts_with('#') || label.starts_with("_#")
}

fn is_hidden_label(label: &str) -> bool {
    label.starts_with('_') && label != "_"
}

fn unquote_bytes(value: &str) -> Vec<u8> {
    let unquoted = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .unwrap_or(value);
    unescape_literal_bytes(unquoted)
}

fn unescape_literal_bytes(value: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            output.push(byte);
            continue;
        }
        let Some(escaped) = bytes.next() else {
            output.push(b'\\');
            break;
        };
        match escaped {
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'"' => output.push(b'"'),
            b'\'' => output.push(b'\''),
            b'\\' => output.push(b'\\'),
            b'x' => push_hex_escape(&mut output, &mut bytes),
            other => {
                output.push(b'\\');
                output.push(other);
            }
        }
    }
    output
}

fn push_hex_escape(output: &mut Vec<u8>, bytes: &mut impl Iterator<Item = u8>) {
    match (
        bytes.next().and_then(hex_value),
        bytes.next().and_then(hex_value),
    ) {
        (Some(high), Some(low)) => output.push((high << 4) | low),
        _ => output.extend_from_slice(b"\\x"),
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_builtin_kind(name: &str) -> bool {
    matches!(
        name,
        "_" | "bool" | "bytes" | "float" | "int" | "null" | "number" | "string"
    )
}

fn is_builtin_name(name: &str) -> bool {
    is_builtin_kind(name)
        || matches!(
            name,
            "and" | "close" | "div" | "len" | "mod" | "or" | "quo" | "rem"
        )
}

fn is_supported_builtin_import(path: &str) -> bool {
    matches!(path, "list" | "math" | "strings")
}

#[cfg(test)]
mod tests {
    use cue_rust_adt::{BaseValue, FeatureKind, Runtime, SemanticExpr};
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

    #[test]
    fn test_should_lower_definition_hidden_and_presence_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_bytes(
            "test.cue",
            b"#Schema: { optional?: string, required!: int, _hidden: true }\nvalue: #Schema & { required: 2 }\n",
            ParseConfig::default(),
        );
        assert!(!parsed.diagnostics().has_errors());
        let files = parsed.ast().map_or_else(Vec::new, |ast| vec![ast.clone()]);
        let instance = BuildInstance::new(None, files);
        let mut runtime = Runtime::default();
        let compiled =
            Compiler::new(&mut runtime).compile_instance(&instance, CompileOptions::default())?;
        let schema_feature = runtime.features.intern(FeatureKind::Definition, "#Schema");
        let root = runtime.vertex(compiled.root())?;
        let schema_arc = root
            .arcs
            .get(&schema_feature)
            .ok_or_else(|| std::io::Error::other("missing #Schema arc"))?;
        assert!(schema_arc.metadata.is_definition());
        assert!(schema_arc.metadata.is_regular());

        let schema = runtime.vertex(schema_arc.target)?;
        let conjunct_id = schema
            .conjuncts
            .first()
            .copied()
            .ok_or_else(|| std::io::Error::other("missing #Schema conjunct"))?;
        let schema_expr = runtime.expression(runtime.conjunct(conjunct_id)?.expression)?;
        let SemanticExpr::Struct(members) = schema_expr else {
            return Err(std::io::Error::other("expected #Schema struct expression").into());
        };
        let optional = members
            .iter()
            .filter_map(static_field_member)
            .find(|(_, feature)| {
                runtime
                    .features
                    .lookup(*feature)
                    .is_some_and(|feature| feature.label == "optional")
            })
            .map(|(field, _)| field)
            .ok_or_else(|| std::io::Error::other("missing optional field"))?;
        assert!(optional.metadata.is_optional());
        assert!(!optional.metadata.is_regular());
        let required = members
            .iter()
            .filter_map(static_field_member)
            .find(|(_, feature)| {
                runtime
                    .features
                    .lookup(*feature)
                    .is_some_and(|feature| feature.label == "required")
            })
            .map(|(field, _)| field)
            .ok_or_else(|| std::io::Error::other("missing required field"))?;
        assert!(required.metadata.is_required());
        assert!(!required.metadata.is_regular());
        let hidden = members
            .iter()
            .filter_map(static_field_member)
            .find(|(_, feature)| {
                runtime
                    .features
                    .lookup(*feature)
                    .is_some_and(|feature| feature.label == "_hidden")
            })
            .map(|(field, _)| field)
            .ok_or_else(|| std::io::Error::other("missing hidden field"))?;
        assert!(hidden.metadata.is_hidden());
        Ok(())
    }

    fn static_field_member(
        member: &cue_rust_adt::StructMember,
    ) -> Option<(&cue_rust_adt::FieldExpr, cue_rust_adt::Feature)> {
        let cue_rust_adt::StructMember::Field(field) = member else {
            return None;
        };
        match &field.label {
            cue_rust_adt::FieldLabel::Static(feature) => Some((field, *feature)),
            _ => None,
        }
    }
}
