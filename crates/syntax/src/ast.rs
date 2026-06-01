//! Source-preserving AST nodes for the supported CUE syntax subset.

use cue_rust_source::Span;

/// Parsed CUE source file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AstFile {
    /// Optional package clause.
    pub package: Option<PackageClause>,
    /// Import declarations.
    pub imports: Vec<ImportDecl>,
    /// Top-level declarations.
    pub declarations: Vec<Decl>,
}

impl AstFile {
    /// Creates an AST file.
    #[must_use]
    pub fn new(
        package: Option<PackageClause>,
        imports: Vec<ImportDecl>,
        declarations: Vec<Decl>,
    ) -> Self {
        Self {
            package,
            imports,
            declarations,
        }
    }

    /// Renders a stable debug tree for CLI snapshots and tests.
    #[must_use]
    pub fn to_debug_tree(&self) -> String {
        let mut lines = Vec::new();
        lines.push("file".to_owned());
        if let Some(package) = &self.package {
            lines.push(format!("  package {}", package.name));
        }
        for import in &self.imports {
            if let Some(alias) = &import.alias {
                lines.push(format!("  import {alias} {}", import.path));
            } else {
                lines.push(format!("  import {}", import.path));
            }
        }
        for declaration in &self.declarations {
            declaration.push_debug(&mut lines, 1);
        }
        lines.join("\n")
    }
}

/// Package clause.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageClause {
    /// Package name.
    pub name: String,
    /// Source span.
    pub span: Span,
}

/// Import declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportDecl {
    /// Optional local import alias.
    pub alias: Option<String>,
    /// Import path literal, including quotes.
    pub path: String,
    /// Source span.
    pub span: Span,
}

/// CUE declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Decl {
    /// Field declaration.
    Field(FieldDecl),
    /// Let declaration.
    Let(LetDecl),
    /// Field or value comprehension declaration.
    Comprehension(ComprehensionDecl),
    /// Ellipsis declaration.
    Ellipsis(Span),
    /// Recovery declaration.
    Bad(Span),
}

impl Decl {
    fn push_debug(&self, lines: &mut Vec<String>, depth: usize) {
        let indent = "  ".repeat(depth);
        match self {
            Self::Field(field) => {
                if let Some(alias) = &field.alias {
                    lines.push(format!(
                        "{indent}field {alias}={}",
                        field.label.display_name(),
                    ));
                } else {
                    lines.push(format!("{indent}field {}", field.label.display_name()));
                }
                field.value.push_debug(lines, depth + 1);
            }
            Self::Let(let_decl) => {
                lines.push(format!("{indent}let {}", let_decl.name));
                let_decl.value.push_debug(lines, depth + 1);
            }
            Self::Comprehension(comprehension) => {
                lines.push(format!("{indent}comprehension"));
                comprehension.push_debug(lines, depth + 1);
            }
            Self::Ellipsis(_) => lines.push(format!("{indent}ellipsis")),
            Self::Bad(_) => lines.push(format!("{indent}bad_decl")),
        }
    }
}

/// Field declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldDecl {
    /// Optional local alias for referencing this field's value.
    pub alias: Option<String>,
    /// Field label.
    pub label: Label,
    /// Field presence marker.
    pub marker: FieldMarker,
    /// Field value expression.
    pub value: Expr,
    /// Source span.
    pub span: Span,
}

/// Field presence marker.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum FieldMarker {
    /// A regular field that participates in concrete output.
    #[default]
    Regular,
    /// An optional field constraint, spelled `?:`.
    Optional,
    /// A required field constraint, spelled `!:`.
    Required,
}

/// Let declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LetDecl {
    /// Let binding name.
    pub name: String,
    /// Bound value expression.
    pub value: Expr,
    /// Source span.
    pub span: Span,
}

/// Field label.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Label {
    /// Identifier label.
    Identifier(String, Span),
    /// String literal label.
    String(String, Span),
    /// Dynamic label, spelled `(expr):`.
    Dynamic(Box<Expr>, Span),
    /// Pattern label, spelled `[expr]:`.
    Pattern(Box<Expr>, Span),
    /// Recovery label.
    Bad(Span),
}

impl Label {
    /// Returns a stable display name for debug trees and diagnostics.
    #[must_use]
    pub fn display_name(&self) -> &str {
        match self {
            Self::Identifier(name, _) | Self::String(name, _) => name,
            Self::Dynamic(_, _) => "<dynamic>",
            Self::Pattern(_, _) => "<pattern>",
            Self::Bad(_) => "<bad>",
        }
    }

    /// Returns the label span.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Identifier(_, span)
            | Self::String(_, span)
            | Self::Dynamic(_, span)
            | Self::Pattern(_, span)
            | Self::Bad(span) => *span,
        }
    }
}

/// Field or value comprehension declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComprehensionDecl {
    /// Ordered comprehension clauses.
    pub clauses: Vec<ComprehensionClause>,
    /// Declarations produced by the comprehension.
    pub body: Vec<Decl>,
    /// Source span.
    pub span: Span,
}

impl ComprehensionDecl {
    fn push_debug(&self, lines: &mut Vec<String>, depth: usize) {
        for clause in &self.clauses {
            clause.push_debug(lines, depth);
        }
        for declaration in &self.body {
            declaration.push_debug(lines, depth);
        }
    }
}

/// One comprehension clause.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ComprehensionClause {
    /// `for key, value in source` or `for value in source`.
    For {
        /// Optional key binding name.
        key: Option<String>,
        /// Value binding name.
        value: String,
        /// Iterated source expression.
        source: Expr,
        /// Source span.
        span: Span,
    },
    /// `if condition`.
    If {
        /// Condition expression.
        condition: Expr,
        /// Source span.
        span: Span,
    },
}

impl ComprehensionClause {
    fn push_debug(&self, lines: &mut Vec<String>, depth: usize) {
        let indent = "  ".repeat(depth);
        match self {
            Self::For {
                key, value, source, ..
            } => {
                if let Some(key) = key {
                    lines.push(format!("{indent}for {key}, {value}"));
                } else {
                    lines.push(format!("{indent}for {value}"));
                }
                source.push_debug(lines, depth + 1);
            }
            Self::If { condition, .. } => {
                lines.push(format!("{indent}if"));
                condition.push_debug(lines, depth + 1);
            }
        }
    }
}

/// CUE expression.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Expr {
    /// Identifier expression.
    Identifier(String, Span),
    /// Number literal expression.
    Number(String, Span),
    /// String literal expression.
    String(String, Span),
    /// Interpolated string expression.
    InterpolatedString {
        /// Interpolation parts.
        parts: Vec<StringPart>,
        /// Source span.
        span: Span,
    },
    /// Bytes literal expression.
    Bytes(String, Span),
    /// Boolean literal expression.
    Bool(bool, Span),
    /// Null literal expression.
    Null(Span),
    /// Struct literal expression.
    Struct(Vec<Decl>, Span),
    /// List literal expression.
    List {
        /// Fixed prefix items.
        items: Vec<Expr>,
        /// Optional open-list tail constraint.
        tail: Option<Box<Expr>>,
        /// Source span.
        span: Span,
    },
    /// Selector expression.
    Selector {
        /// Selected base expression.
        base: Box<Expr>,
        /// Selected field name.
        field: String,
        /// Source span.
        span: Span,
    },
    /// Index expression.
    Index {
        /// Indexed base expression.
        base: Box<Expr>,
        /// Index expression.
        index: Box<Expr>,
        /// Source span.
        span: Span,
    },
    /// Slice expression.
    Slice {
        /// Sliced base expression.
        base: Box<Expr>,
        /// Optional inclusive start bound.
        start: Option<Box<Expr>>,
        /// Optional exclusive end bound.
        end: Option<Box<Expr>>,
        /// Source span.
        span: Span,
    },
    /// Function or builtin call expression.
    Call {
        /// Called expression.
        callee: Box<Expr>,
        /// Call arguments.
        args: Vec<Expr>,
        /// Source span.
        span: Span,
    },
    /// Unary expression.
    Unary {
        /// Operator text.
        op: String,
        /// Operand expression.
        expr: Box<Expr>,
        /// Source span.
        span: Span,
    },
    /// Binary expression.
    Binary {
        /// Operator text.
        op: String,
        /// Left expression.
        left: Box<Expr>,
        /// Right expression.
        right: Box<Expr>,
        /// Source span.
        span: Span,
    },
    /// Default marker expression.
    Default(Box<Expr>, Span),
    /// List or value comprehension expression.
    Comprehension {
        /// Ordered comprehension clauses.
        clauses: Vec<ComprehensionClause>,
        /// Produced expression body.
        body: Box<Expr>,
        /// Source span.
        span: Span,
    },
    /// Ellipsis expression.
    Ellipsis(Span),
    /// Recovery expression.
    Bad(Span),
}

/// One interpolated string segment.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StringPart {
    /// Literal text segment after ordinary string escape decoding.
    Text(String),
    /// Embedded expression segment.
    Expr(Box<Expr>),
}

impl Expr {
    /// Returns the expression span.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Identifier(_, span)
            | Self::Number(_, span)
            | Self::String(_, span)
            | Self::InterpolatedString { span, .. }
            | Self::Bytes(_, span)
            | Self::Bool(_, span)
            | Self::Null(span)
            | Self::Struct(_, span)
            | Self::Default(_, span)
            | Self::Comprehension { span, .. }
            | Self::Ellipsis(span)
            | Self::Bad(span)
            | Self::List { span, .. }
            | Self::Selector { span, .. }
            | Self::Index { span, .. }
            | Self::Slice { span, .. }
            | Self::Call { span, .. }
            | Self::Unary { span, .. }
            | Self::Binary { span, .. } => *span,
        }
    }

    fn push_debug(&self, lines: &mut Vec<String>, depth: usize) {
        let indent = "  ".repeat(depth);
        match self {
            Self::Identifier(name, _) => lines.push(format!("{indent}ident {name}")),
            Self::Number(value, _) => lines.push(format!("{indent}number {value}")),
            Self::String(value, _) => lines.push(format!("{indent}string {value}")),
            Self::InterpolatedString { parts, .. } => {
                lines.push(format!("{indent}interpolated_string"));
                for part in parts {
                    match part {
                        StringPart::Text(value) => lines.push(format!("{indent}  text {value:?}")),
                        StringPart::Expr(expr) => expr.push_debug(lines, depth + 1),
                    }
                }
            }
            Self::Bytes(value, _) => lines.push(format!("{indent}bytes {value}")),
            Self::Bool(value, _) => lines.push(format!("{indent}bool {value}")),
            Self::Null(_) => lines.push(format!("{indent}null")),
            Self::Struct(declarations, _) => {
                lines.push(format!("{indent}struct"));
                for declaration in declarations {
                    declaration.push_debug(lines, depth + 1);
                }
            }
            Self::List { items, tail, .. } => {
                lines.push(format!("{indent}list"));
                for item in items {
                    item.push_debug(lines, depth + 1);
                }
                if let Some(tail) = tail {
                    lines.push(format!("{indent}  ellipsis"));
                    tail.push_debug(lines, depth + 2);
                }
            }
            Self::Selector { base, field, .. } => {
                lines.push(format!("{indent}selector {field}"));
                base.push_debug(lines, depth + 1);
            }
            Self::Index { base, index, .. } => {
                lines.push(format!("{indent}index"));
                base.push_debug(lines, depth + 1);
                index.push_debug(lines, depth + 1);
            }
            Self::Slice {
                base, start, end, ..
            } => {
                lines.push(format!("{indent}slice"));
                base.push_debug(lines, depth + 1);
                if let Some(start) = start {
                    start.push_debug(lines, depth + 1);
                }
                if let Some(end) = end {
                    end.push_debug(lines, depth + 1);
                }
            }
            Self::Call { callee, args, .. } => {
                lines.push(format!("{indent}call"));
                callee.push_debug(lines, depth + 1);
                for arg in args {
                    arg.push_debug(lines, depth + 1);
                }
            }
            Self::Unary { op, expr, .. } => {
                lines.push(format!("{indent}unary {op}"));
                expr.push_debug(lines, depth + 1);
            }
            Self::Binary {
                op, left, right, ..
            } => {
                lines.push(format!("{indent}binary {op}"));
                left.push_debug(lines, depth + 1);
                right.push_debug(lines, depth + 1);
            }
            Self::Default(expr, _) => {
                lines.push(format!("{indent}default"));
                expr.push_debug(lines, depth + 1);
            }
            Self::Comprehension { clauses, body, .. } => {
                lines.push(format!("{indent}comprehension"));
                for clause in clauses {
                    clause.push_debug(lines, depth + 1);
                }
                body.push_debug(lines, depth + 1);
            }
            Self::Ellipsis(_) => lines.push(format!("{indent}ellipsis")),
            Self::Bad(_) => lines.push(format!("{indent}bad_expr")),
        }
    }
}
