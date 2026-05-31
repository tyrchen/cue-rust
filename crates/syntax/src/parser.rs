//! Tolerant parser for the supported CUE syntax subset.

use cue_rust_source::{Diagnostic, DiagnosticReport, Severity, Span};

use crate::{
    AstFile, Decl, Expr, FieldDecl, ImportDecl, Label, LetDecl, PackageClause, ParseConfig,
    ScanResult, Token, TokenKind, scan_bytes,
};

/// Result of parsing source bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseResult {
    source: ScanResult,
    ast: Option<AstFile>,
    diagnostics: DiagnosticReport,
}

impl ParseResult {
    /// Creates a parse result.
    #[must_use]
    pub fn new(source: ScanResult, ast: Option<AstFile>, diagnostics: DiagnosticReport) -> Self {
        Self {
            source,
            ast,
            diagnostics,
        }
    }

    /// Returns the scanner result.
    #[must_use]
    pub fn source(&self) -> &ScanResult {
        &self.source
    }

    /// Returns the parsed AST when source validation succeeded.
    #[must_use]
    pub fn ast(&self) -> Option<&AstFile> {
        self.ast.as_ref()
    }

    /// Returns parse and scanner diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &DiagnosticReport {
        &self.diagnostics
    }
}

/// Parses raw CUE source bytes into a tolerant AST.
///
/// # Examples
///
/// ```
/// use cue_rust_syntax::{ParseConfig, parse_bytes};
///
/// let parsed = parse_bytes("example.cue", b"package demo\nx: 1\n", ParseConfig::default());
/// assert!(!parsed.diagnostics().has_errors());
/// ```
#[must_use]
pub fn parse_bytes(name: impl Into<String>, bytes: &[u8], config: ParseConfig) -> ParseResult {
    let scan = scan_bytes(name, bytes, config);
    let mut diagnostics = scan.diagnostics().clone();
    let ast = if scan.source().is_some() {
        let mut parser = Parser::new(scan.tokens());
        let ast = parser.parse_file();
        diagnostics.extend(parser.diagnostics.diagnostics().iter().cloned());
        Some(ast)
    } else {
        None
    };
    ParseResult::new(scan, ast, diagnostics)
}

#[derive(Debug)]
struct Parser<'tokens> {
    tokens: &'tokens [Token],
    cursor: usize,
    diagnostics: DiagnosticReport,
}

impl<'tokens> Parser<'tokens> {
    fn new(tokens: &'tokens [Token]) -> Self {
        Self {
            tokens,
            cursor: 0,
            diagnostics: DiagnosticReport::new(),
        }
    }

    fn parse_file(&mut self) -> AstFile {
        let package = self.parse_package();
        let mut imports = Vec::new();
        let mut declarations = Vec::new();

        while !self.at(TokenKind::Eof) {
            self.skip_separators();
            if self.at(TokenKind::Import) {
                imports.extend(self.parse_imports());
            } else if !self.at(TokenKind::Eof) {
                declarations.push(self.parse_decl());
            }
            self.skip_separators();
        }

        AstFile::new(package, imports, declarations)
    }

    fn parse_package(&mut self) -> Option<PackageClause> {
        self.skip_separators();
        if !self.at(TokenKind::Package) {
            return None;
        }
        let start = self.bump()?.span();
        let name = if self.at(TokenKind::Identifier) {
            self.bump()?.text().to_owned()
        } else {
            self.error_here(
                "cue.parse.expected_package_name",
                "expected package name after package keyword",
            );
            "<bad>".to_owned()
        };
        let end = self.previous_span().unwrap_or(start);
        Some(PackageClause {
            name,
            span: merge_span(start, end),
        })
    }

    fn parse_imports(&mut self) -> Vec<ImportDecl> {
        let start = match self.bump() {
            Some(token) => token.span(),
            None => return Vec::new(),
        };
        if self.at(TokenKind::LeftParen) {
            self.bump();
            let mut imports = Vec::new();
            while !self.at(TokenKind::RightParen) && !self.at(TokenKind::Eof) {
                self.skip_separators();
                if self.at(TokenKind::String) {
                    imports.push(self.import_from_string(start));
                } else {
                    self.error_here("cue.parse.expected_import", "expected import string");
                    self.recover_to_separator();
                }
                self.skip_separators();
            }
            self.expect_kind(
                TokenKind::RightParen,
                "cue.parse.expected_import_close",
                "expected ')' after import list",
            );
            imports
        } else if self.at(TokenKind::String) {
            vec![self.import_from_string(start)]
        } else {
            self.error_here("cue.parse.expected_import", "expected import string");
            Vec::new()
        }
    }

    fn import_from_string(&mut self, start: Span) -> ImportDecl {
        let Some(token) = self.bump() else {
            return ImportDecl {
                path: "\"<bad>\"".to_owned(),
                span: start,
            };
        };
        ImportDecl {
            path: token.text().to_owned(),
            span: merge_span(start, token.span()),
        }
    }

    fn parse_decl(&mut self) -> Decl {
        if self.at(TokenKind::Ellipsis) {
            return self.bump().map_or_else(
                || self.bad_decl_at_current(),
                |token| Decl::Ellipsis(token.span()),
            );
        }
        if self.at(TokenKind::Let) {
            return self.parse_let();
        }
        if self.at_label_start() {
            return self.parse_field();
        }
        self.error_here("cue.parse.expected_decl", "expected declaration");
        let bad = self.current_span();
        self.recover_to_separator();
        Decl::Bad(bad)
    }

    fn parse_let(&mut self) -> Decl {
        let start = match self.bump() {
            Some(token) => token.span(),
            None => return self.bad_decl_at_current(),
        };
        let name = if self.at(TokenKind::Identifier) {
            self.bump().map_or("<bad>", Token::text).to_owned()
        } else {
            self.error_here("cue.parse.expected_let_name", "expected let binding name");
            "<bad>".to_owned()
        };
        self.expect_operator(
            "=",
            "cue.parse.expected_let_eq",
            "expected '=' in let declaration",
        );
        let value = self.parse_expr(0);
        let span = merge_span(start, value.span());
        Decl::Let(LetDecl { name, value, span })
    }

    fn parse_field(&mut self) -> Decl {
        let label = self.parse_label();
        self.expect_kind(
            TokenKind::Colon,
            "cue.parse.expected_colon",
            "expected ':' after field label",
        );
        let value = self.parse_expr(0);
        let span = merge_span(label.span(), value.span());
        Decl::Field(FieldDecl { label, value, span })
    }

    fn parse_label(&mut self) -> Label {
        if self.at(TokenKind::Identifier) {
            return self.bump().map_or_else(
                || Label::Bad(self.current_span()),
                |token| Label::Identifier(token.text().to_owned(), token.span()),
            );
        }
        if self.at(TokenKind::String) {
            return self.bump().map_or_else(
                || Label::Bad(self.current_span()),
                |token| Label::String(token.text().to_owned(), token.span()),
            );
        }
        self.error_here("cue.parse.expected_label", "expected field label");
        Label::Bad(self.current_span())
    }

    fn parse_expr(&mut self, min_bp: u8) -> Expr {
        let mut left = self.parse_prefix();

        loop {
            if self.at(TokenKind::Dot) {
                let dot = self.bump().map_or_else(|| left.span(), Token::span);
                if self.at(TokenKind::Identifier)
                    && let Some(field) = self.bump()
                {
                    let span = merge_span(left.span(), field.span());
                    left = Expr::Selector {
                        base: Box::new(left),
                        field: field.text().to_owned(),
                        span,
                    };
                    continue;
                }
                self.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "cue.parse.expected_selector",
                    "expected selector after '.'",
                    Some(dot),
                ));
                left = Expr::Bad(dot);
                continue;
            }

            let Some((op, left_bp, right_bp)) = self.infix_binding_power() else {
                break;
            };
            if left_bp < min_bp {
                break;
            }
            let Some(operator) = self.bump() else {
                break;
            };
            let right = self.parse_expr(right_bp);
            let span = merge_span(left.span(), right.span());
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
            if operator.kind() == TokenKind::Eof {
                break;
            }
        }

        left
    }

    fn parse_prefix(&mut self) -> Expr {
        if self.at(TokenKind::Star) {
            let star = self.bump().map_or_else(|| self.current_span(), Token::span);
            let expr = self.parse_prefix();
            return Expr::Default(Box::new(expr), star);
        }
        if self.at(TokenKind::Operator) && matches!(self.peek_text(), Some("-" | "+" | "!")) {
            let Some(operator) = self.bump() else {
                return Expr::Bad(self.current_span());
            };
            let expr = self.parse_prefix();
            let span = merge_span(operator.span(), expr.span());
            return Expr::Unary {
                op: operator.text().to_owned(),
                expr: Box::new(expr),
                span,
            };
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek_kind() {
            Some(TokenKind::Identifier) => self.parse_identifier_or_literal(),
            Some(TokenKind::Number) => self.bump().map_or_else(
                || Expr::Bad(self.current_span()),
                |token| Expr::Number(token.text().to_owned(), token.span()),
            ),
            Some(TokenKind::String) => self.bump().map_or_else(
                || Expr::Bad(self.current_span()),
                |token| Expr::String(token.text().to_owned(), token.span()),
            ),
            Some(TokenKind::LeftBrace) => self.parse_struct(),
            Some(TokenKind::LeftBracket) => self.parse_list(),
            Some(TokenKind::LeftParen) => self.parse_parenthesized(),
            Some(TokenKind::Ellipsis) => self.bump().map_or_else(
                || Expr::Bad(self.current_span()),
                |token| Expr::Ellipsis(token.span()),
            ),
            _ => {
                self.error_here("cue.parse.expected_expr", "expected expression");
                let span = self.current_span();
                self.recover_to_separator();
                Expr::Bad(span)
            }
        }
    }

    fn parse_identifier_or_literal(&mut self) -> Expr {
        let Some(token) = self.bump() else {
            return Expr::Bad(self.current_span());
        };
        match token.text() {
            "true" => Expr::Bool(true, token.span()),
            "false" => Expr::Bool(false, token.span()),
            "null" => Expr::Null(token.span()),
            _ => Expr::Identifier(token.text().to_owned(), token.span()),
        }
    }

    fn parse_struct(&mut self) -> Expr {
        let start = match self.bump() {
            Some(token) => token.span(),
            None => return Expr::Bad(self.current_span()),
        };
        let mut declarations = Vec::new();
        while !self.at(TokenKind::RightBrace) && !self.at(TokenKind::Eof) {
            self.skip_separators();
            if !self.at(TokenKind::RightBrace) {
                declarations.push(self.parse_decl());
            }
            self.skip_separators();
        }
        let end = self
            .expect_kind(
                TokenKind::RightBrace,
                "cue.parse.expected_struct_close",
                "expected '}' after struct",
            )
            .unwrap_or(start);
        Expr::Struct(declarations, merge_span(start, end))
    }

    fn parse_list(&mut self) -> Expr {
        let start = match self.bump() {
            Some(token) => token.span(),
            None => return Expr::Bad(self.current_span()),
        };
        let mut items = Vec::new();
        while !self.at(TokenKind::RightBracket) && !self.at(TokenKind::Eof) {
            self.skip_separators();
            if !self.at(TokenKind::RightBracket) {
                items.push(self.parse_expr(0));
            }
            self.skip_separators();
        }
        let end = self
            .expect_kind(
                TokenKind::RightBracket,
                "cue.parse.expected_list_close",
                "expected ']' after list",
            )
            .unwrap_or(start);
        Expr::List(items, merge_span(start, end))
    }

    fn parse_parenthesized(&mut self) -> Expr {
        let start = match self.bump() {
            Some(token) => token.span(),
            None => return Expr::Bad(self.current_span()),
        };
        let expr = self.parse_expr(0);
        let end = self
            .expect_kind(
                TokenKind::RightParen,
                "cue.parse.expected_paren_close",
                "expected ')' after expression",
            )
            .unwrap_or(start);
        let span = merge_span(start, end);
        Expr::Unary {
            op: "group".to_owned(),
            expr: Box::new(expr),
            span,
        }
    }

    fn infix_binding_power(&self) -> Option<(String, u8, u8)> {
        match self.peek_kind()? {
            TokenKind::Star => Some(("*".to_owned(), 7, 8)),
            TokenKind::Operator => match self.peek_text()? {
                "&" | "&&" => Some((self.peek_text()?.to_owned(), 3, 4)),
                "|" | "||" => Some((self.peek_text()?.to_owned(), 1, 2)),
                "==" | "!=" | "<" | "<=" | ">" | ">=" => Some((self.peek_text()?.to_owned(), 5, 6)),
                "+" | "-" => Some((self.peek_text()?.to_owned(), 7, 8)),
                _ => None,
            },
            _ => None,
        }
    }

    fn expect_kind(
        &mut self,
        kind: TokenKind,
        code: &'static str,
        message: &'static str,
    ) -> Option<Span> {
        if self.at(kind) {
            return self.bump().map(Token::span);
        }
        self.error_here(code, message);
        None
    }

    fn expect_operator(&mut self, op: &str, code: &'static str, message: &'static str) {
        if self.at(TokenKind::Operator) && self.peek_text() == Some(op) {
            self.bump();
        } else {
            self.error_here(code, message);
        }
    }

    fn skip_separators(&mut self) {
        while matches!(
            self.peek_kind(),
            Some(TokenKind::Comma | TokenKind::Comment)
        ) {
            self.bump();
        }
    }

    fn recover_to_separator(&mut self) {
        while !matches!(
            self.peek_kind(),
            Some(
                TokenKind::Comma | TokenKind::RightBrace | TokenKind::RightBracket | TokenKind::Eof
            )
        ) {
            self.bump();
        }
    }

    fn at_label_start(&self) -> bool {
        matches!(
            self.peek_kind(),
            Some(TokenKind::Identifier | TokenKind::String)
        )
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.peek_kind() == Some(kind)
    }

    fn peek_kind(&self) -> Option<TokenKind> {
        self.tokens.get(self.cursor).map(Token::kind)
    }

    fn peek_text(&self) -> Option<&str> {
        self.tokens.get(self.cursor).map(Token::text)
    }

    fn bump(&mut self) -> Option<&'tokens Token> {
        let token = self.tokens.get(self.cursor);
        if token.is_some() {
            self.cursor = self.cursor.saturating_add(1);
        }
        token
    }

    fn previous_span(&self) -> Option<Span> {
        self.cursor
            .checked_sub(1)
            .and_then(|index| self.tokens.get(index))
            .map(Token::span)
    }

    fn current_span(&self) -> Span {
        self.tokens
            .get(self.cursor)
            .or_else(|| self.tokens.last())
            .map_or_else(fallback_span, Token::span)
    }

    fn error_here(&mut self, code: &'static str, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(
            Severity::Error,
            code,
            message,
            Some(self.current_span()),
        ));
    }

    fn bad_decl_at_current(&self) -> Decl {
        Decl::Bad(self.current_span())
    }
}

fn merge_span(start: Span, end: Span) -> Span {
    Span::new(start.source(), start.start(), end.end()).unwrap_or(start)
}

fn fallback_span() -> Span {
    Span::point(
        cue_rust_source::SourceId::FIRST,
        cue_rust_source::ByteOffset(0),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_bytes;
    use crate::{Decl, Expr, ParseConfig};

    #[test]
    fn test_should_parse_package_and_field() {
        let result = parse_bytes("test.cue", b"package demo\nx: 1\n", ParseConfig::default());
        assert!(!result.diagnostics().has_errors());
        let tree = result.ast().map(crate::AstFile::to_debug_tree);
        assert_eq!(
            Some("file\n  package demo\n  field x\n    number 1".to_owned()),
            tree,
        );
    }

    #[test]
    fn test_should_recover_bad_declaration() {
        let result = parse_bytes("test.cue", b"@bad\nx: 1\n", ParseConfig::default());
        assert!(result.diagnostics().has_errors());
        let has_bad = result.ast().is_some_and(|ast| {
            ast.declarations
                .iter()
                .any(|declaration| matches!(declaration, Decl::Bad(_)))
        });
        assert!(has_bad);
    }

    #[test]
    fn test_should_parse_nested_struct_list_and_binary() {
        let result = parse_bytes(
            "test.cue",
            b"x: { y: [1, 2 & 3] }\n",
            ParseConfig::default(),
        );
        assert!(!result.diagnostics().has_errors());
        let is_struct = result
            .ast()
            .and_then(|ast| ast.declarations.first())
            .and_then(|declaration| match declaration {
                Decl::Field(field) => Some(matches!(field.value, Expr::Struct(_, _))),
                _ => None,
            })
            .unwrap_or(false);
        assert!(is_struct);
    }
}
