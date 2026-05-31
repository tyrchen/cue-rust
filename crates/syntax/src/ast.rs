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
            lines.push(format!("  import {}", import.path));
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
                lines.push(format!("{indent}field {}", field.label.display_name()));
                field.value.push_debug(lines, depth + 1);
            }
            Self::Let(let_decl) => {
                lines.push(format!("{indent}let {}", let_decl.name));
                let_decl.value.push_debug(lines, depth + 1);
            }
            Self::Ellipsis(_) => lines.push(format!("{indent}ellipsis")),
            Self::Bad(_) => lines.push(format!("{indent}bad_decl")),
        }
    }
}

/// Field declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldDecl {
    /// Field label.
    pub label: Label,
    /// Field value expression.
    pub value: Expr,
    /// Source span.
    pub span: Span,
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
    /// Recovery label.
    Bad(Span),
}

impl Label {
    /// Returns a stable display name for debug trees and diagnostics.
    #[must_use]
    pub fn display_name(&self) -> &str {
        match self {
            Self::Identifier(name, _) | Self::String(name, _) => name,
            Self::Bad(_) => "<bad>",
        }
    }

    /// Returns the label span.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Identifier(_, span) | Self::String(_, span) | Self::Bad(span) => *span,
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
    /// Boolean literal expression.
    Bool(bool, Span),
    /// Null literal expression.
    Null(Span),
    /// Struct literal expression.
    Struct(Vec<Decl>, Span),
    /// List literal expression.
    List(Vec<Expr>, Span),
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
    /// Ellipsis expression.
    Ellipsis(Span),
    /// Recovery expression.
    Bad(Span),
}

impl Expr {
    /// Returns the expression span.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Identifier(_, span)
            | Self::Number(_, span)
            | Self::String(_, span)
            | Self::Bool(_, span)
            | Self::Null(span)
            | Self::Struct(_, span)
            | Self::List(_, span)
            | Self::Default(_, span)
            | Self::Ellipsis(span)
            | Self::Bad(span)
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
            Self::Bool(value, _) => lines.push(format!("{indent}bool {value}")),
            Self::Null(_) => lines.push(format!("{indent}null")),
            Self::Struct(declarations, _) => {
                lines.push(format!("{indent}struct"));
                for declaration in declarations {
                    declaration.push_debug(lines, depth + 1);
                }
            }
            Self::List(items, _) => {
                lines.push(format!("{indent}list"));
                for item in items {
                    item.push_debug(lines, depth + 1);
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
            Self::Ellipsis(_) => lines.push(format!("{indent}ellipsis")),
            Self::Bad(_) => lines.push(format!("{indent}bad_expr")),
        }
    }
}
