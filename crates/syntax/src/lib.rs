//! Scanner, parser, AST, and syntax-service entry points.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use cue_rust_source::{
    ByteOffset, Diagnostic, DiagnosticReport, Severity, SourceError, SourceFile, SourceId,
    SourceLimits, Span,
};

mod ast;
mod parser;

pub use ast::{
    AstFile, ComprehensionClause, ComprehensionDecl, Decl, Expr, FieldDecl, FieldMarker,
    ImportDecl, Label, LetDecl, PackageClause, StringPart,
};
pub use parser::{ParseResult, parse_bytes};

const SCANNER_SOURCE_ID: u32 = 1;

/// Default maximum recursive parse depth for expressions and nested values.
pub const DEFAULT_MAX_PARSE_DEPTH: u32 = 256;

/// Parser mode requested by the SDK or CLI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ParseMode {
    /// Parse a complete CUE source file.
    #[default]
    File,
}

/// Parser configuration shared by syntax entry points.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseConfig {
    mode: ParseMode,
    limits: SourceLimits,
    include_comments: bool,
    max_depth: u32,
}

impl Default for ParseConfig {
    fn default() -> Self {
        Self {
            mode: ParseMode::default(),
            limits: SourceLimits::default(),
            include_comments: false,
            max_depth: DEFAULT_MAX_PARSE_DEPTH,
        }
    }
}

impl ParseConfig {
    /// Creates parser configuration from a mode and source limits.
    #[must_use]
    pub fn new(mode: ParseMode, limits: SourceLimits) -> Self {
        Self {
            mode,
            limits,
            include_comments: false,
            max_depth: DEFAULT_MAX_PARSE_DEPTH,
        }
    }

    /// Returns the parser mode.
    #[must_use]
    pub fn mode(self) -> ParseMode {
        self.mode
    }

    /// Returns the source limits.
    #[must_use]
    pub fn limits(self) -> SourceLimits {
        self.limits
    }

    /// Returns whether comment tokens are retained.
    #[must_use]
    pub fn include_comments(self) -> bool {
        self.include_comments
    }

    /// Returns the maximum recursive parse depth.
    #[must_use]
    pub fn max_depth(self) -> u32 {
        self.max_depth
    }

    /// Returns a copy of this config with comment retention enabled or disabled.
    #[must_use]
    pub fn with_comments(mut self, include_comments: bool) -> Self {
        self.include_comments = include_comments;
        self
    }

    /// Returns a copy of this config with an explicit maximum parse depth.
    #[must_use]
    pub fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }
}

/// Minimal syntax parse result used by the SDK facade.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedSource {
    source: SourceFile,
}

impl ParsedSource {
    /// Creates a parsed-source shell around a validated source.
    #[must_use]
    pub fn new(source: SourceFile) -> Self {
        Self { source }
    }

    /// Returns the source backing this parsed result.
    #[must_use]
    pub fn source(&self) -> &SourceFile {
        &self.source
    }
}

/// Scanner token kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TokenKind {
    /// Identifier or keyword-like identifier not otherwise classified.
    Identifier,
    /// `package` keyword.
    Package,
    /// `import` keyword.
    Import,
    /// `let` keyword.
    Let,
    /// Number literal.
    Number,
    /// String literal.
    String,
    /// Line or block comment.
    Comment,
    /// `@name(...)` style attribute marker.
    Attribute,
    /// `{`.
    LeftBrace,
    /// `}`.
    RightBrace,
    /// `[`.
    LeftBracket,
    /// `]`.
    RightBracket,
    /// `(`.
    LeftParen,
    /// `)`.
    RightParen,
    /// `:`.
    Colon,
    /// `,` or inserted comma.
    Comma,
    /// `.`.
    Dot,
    /// `...`.
    Ellipsis,
    /// `*`.
    Star,
    /// Operator token.
    Operator,
    /// Invalid token retained for recovery.
    Bad,
    /// End of file marker.
    Eof,
}

/// Scanner token with source span and text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    kind: TokenKind,
    span: Span,
    text: String,
    inserted: bool,
}

impl Token {
    /// Creates a token.
    #[must_use]
    pub fn new(kind: TokenKind, span: Span, text: impl Into<String>, inserted: bool) -> Self {
        Self {
            kind,
            span,
            text: text.into(),
            inserted,
        }
    }

    /// Returns the token kind.
    #[must_use]
    pub fn kind(&self) -> TokenKind {
        self.kind
    }

    /// Returns the token span.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }

    /// Returns the source text for this token.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns true when the scanner inserted this comma.
    #[must_use]
    pub fn inserted(&self) -> bool {
        self.inserted
    }
}

/// Result of scanner execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanResult {
    source: Option<SourceFile>,
    tokens: Vec<Token>,
    diagnostics: DiagnosticReport,
}

impl ScanResult {
    /// Returns the validated source when UTF-8 and source limits succeeded.
    #[must_use]
    pub fn source(&self) -> Option<&SourceFile> {
        self.source.as_ref()
    }

    /// Returns scanned tokens.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// Returns scanner diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &DiagnosticReport {
        &self.diagnostics
    }
}

/// Scans raw source bytes into tokens and diagnostics without panicking.
#[must_use]
pub fn scan_bytes(name: impl Into<String>, bytes: &[u8], config: ParseConfig) -> ScanResult {
    let name = name.into();
    match SourceFile::named_bytes(name, bytes, config.limits()) {
        Ok(source) => scan_source(source, config),
        Err(error) => {
            let mut diagnostics = DiagnosticReport::new();
            diagnostics.push(Diagnostic::new(
                Severity::Error,
                "cue.source.invalid",
                error.to_string(),
                None,
            ));
            ScanResult {
                source: None,
                tokens: Vec::new(),
                diagnostics,
            }
        }
    }
}

/// Scans a validated source into tokens and diagnostics.
#[must_use]
pub fn scan_source(source: SourceFile, config: ParseConfig) -> ScanResult {
    let source_id = match SourceId::new(SCANNER_SOURCE_ID) {
        Ok(source_id) => source_id,
        Err(error) => {
            let mut diagnostics = DiagnosticReport::new();
            diagnostics.push(Diagnostic::new(
                Severity::Error,
                "cue.source.invalid_id",
                error.to_string(),
                None,
            ));
            return ScanResult {
                source: Some(source),
                tokens: Vec::new(),
                diagnostics,
            };
        }
    };

    let mut scanner = Scanner::new(source.content(), source_id, config.include_comments());
    let (tokens, diagnostics) = scanner.scan_all();
    ScanResult {
        source: Some(source),
        tokens,
        diagnostics,
    }
}

#[derive(Debug)]
struct Scanner<'src> {
    input: &'src str,
    offset: usize,
    source: SourceId,
    include_comments: bool,
    tokens: Vec<Token>,
    diagnostics: DiagnosticReport,
    insert_comma: bool,
}

impl<'src> Scanner<'src> {
    fn new(input: &'src str, source: SourceId, include_comments: bool) -> Self {
        Self {
            input,
            offset: 0,
            source,
            include_comments,
            tokens: Vec::new(),
            diagnostics: DiagnosticReport::new(),
            insert_comma: false,
        }
    }

    fn scan_all(&mut self) -> (Vec<Token>, DiagnosticReport) {
        self.scan_bom();
        while let Some(byte) = self.peek_byte() {
            match byte {
                b' ' | b'\t' | b'\r' => self.advance_one(),
                b'\n' => self.scan_newline(),
                0 => self.scan_nul(),
                b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'#' => self.scan_identifier(),
                b'0'..=b'9' => self.scan_number(),
                b'"' | b'\'' => self.scan_string(byte),
                b'/' => self.scan_slash(),
                b'@' => self.scan_attribute(),
                b'{' => self.scan_single(TokenKind::LeftBrace),
                b'}' => self.scan_closing(TokenKind::RightBrace),
                b'[' => self.scan_single(TokenKind::LeftBracket),
                b']' => self.scan_closing(TokenKind::RightBracket),
                b'(' => self.scan_single(TokenKind::LeftParen),
                b')' => self.scan_closing(TokenKind::RightParen),
                b':' => self.scan_single(TokenKind::Colon),
                b',' | b';' => self.scan_explicit_comma(),
                b'.' => self.scan_dot(),
                b'*' => self.scan_single(TokenKind::Star),
                b'+' | b'-' | b'=' | b'!' | b'<' | b'>' | b'&' | b'|' | b'~' | b'?' => {
                    self.scan_operator();
                }
                _ => self.scan_bad_byte(),
            }
        }
        self.push_token(TokenKind::Eof, self.offset, self.offset, "", false);
        (
            std::mem::take(&mut self.tokens),
            std::mem::take(&mut self.diagnostics),
        )
    }

    fn scan_bom(&mut self) {
        if self.input.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.offset = 3;
        }
    }

    fn scan_newline(&mut self) {
        let newline_offset = self.offset;
        self.advance_one();
        if self.insert_comma {
            self.push_token(TokenKind::Comma, newline_offset, newline_offset, "", true);
            self.insert_comma = false;
        }
    }

    fn scan_nul(&mut self) {
        let start = self.offset;
        self.advance_one();
        self.push_diagnostic(
            "cue.scan.nul",
            "NUL bytes are not valid in CUE source",
            start,
            self.offset,
        );
        self.push_token(TokenKind::Bad, start, self.offset, "\0", false);
    }

    fn scan_identifier(&mut self) {
        let start = self.offset;
        self.advance_while(|byte| {
            matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'#'
            )
        });
        let text = self.text(start, self.offset).to_owned();
        let kind = match text.as_str() {
            "package" => TokenKind::Package,
            "import" => TokenKind::Import,
            "let" => TokenKind::Let,
            _ => TokenKind::Identifier,
        };
        self.push_token(kind, start, self.offset, text, false);
        self.insert_comma = true;
    }

    fn scan_number(&mut self) {
        let start = self.offset;
        self.advance_digits();
        if self.peek_byte() == Some(b'.') && self.peek_next_byte() != Some(b'.') {
            self.advance_one();
            self.advance_digits();
        }
        if matches!(self.peek_byte(), Some(b'e' | b'E')) {
            self.advance_one();
            if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                self.advance_one();
            }
            self.advance_digits();
        }
        let text = self.text(start, self.offset).to_owned();
        if !is_valid_number_literal(&text) {
            self.push_diagnostic(
                "cue.scan.invalid_number",
                format!("invalid number literal `{text}`"),
                start,
                self.offset,
            );
        }
        self.push_token(TokenKind::Number, start, self.offset, text, false);
        self.insert_comma = true;
    }

    fn advance_digits(&mut self) {
        self.advance_while(|byte| matches!(byte, b'0'..=b'9' | b'_'));
    }

    fn scan_string(&mut self, quote: u8) {
        let start = self.offset;
        self.advance_one();
        let mut escaped = false;
        let mut terminated = false;

        while let Some(byte) = self.peek_byte() {
            if escaped {
                escaped = false;
                self.advance_one();
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                self.advance_one();
                continue;
            }
            self.advance_one();
            if byte == quote {
                terminated = true;
                break;
            }
            if byte == 0 {
                self.push_diagnostic(
                    "cue.scan.nul",
                    "NUL bytes are not valid in string literals",
                    self.offset.saturating_sub(1),
                    self.offset,
                );
            }
        }

        if !terminated {
            self.push_diagnostic(
                "cue.scan.unterminated_string",
                "unterminated string literal",
                start,
                self.offset,
            );
        }

        let text = self.text(start, self.offset).to_owned();
        self.push_token(TokenKind::String, start, self.offset, text, false);
        self.insert_comma = true;
    }

    fn scan_slash(&mut self) {
        let start = self.offset;
        self.advance_one();
        match self.peek_byte() {
            Some(b'/') => {
                self.advance_while(|byte| byte != b'\n');
                if self.include_comments {
                    let text = self.text(start, self.offset).to_owned();
                    self.push_token(TokenKind::Comment, start, self.offset, text, false);
                }
            }
            Some(b'*') => {
                self.advance_one();
                let mut terminated = false;
                while let Some(byte) = self.peek_byte() {
                    self.advance_one();
                    if byte == b'*' && self.peek_byte() == Some(b'/') {
                        self.advance_one();
                        terminated = true;
                        break;
                    }
                }
                if !terminated {
                    self.push_diagnostic(
                        "cue.scan.unterminated_comment",
                        "unterminated block comment",
                        start,
                        self.offset,
                    );
                }
                if self.include_comments {
                    let text = self.text(start, self.offset).to_owned();
                    self.push_token(TokenKind::Comment, start, self.offset, text, false);
                }
            }
            _ => {
                let text = self.text(start, self.offset).to_owned();
                self.push_token(TokenKind::Operator, start, self.offset, text, false);
                self.insert_comma = false;
            }
        }
    }

    fn scan_attribute(&mut self) {
        let start = self.offset;
        self.advance_one();
        self.advance_while(|byte| {
            matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'
            )
        });
        let text = self.text(start, self.offset).to_owned();
        self.push_token(TokenKind::Attribute, start, self.offset, text, false);
        self.insert_comma = true;
    }

    fn scan_single(&mut self, kind: TokenKind) {
        let start = self.offset;
        self.advance_one();
        let text = self.text(start, self.offset).to_owned();
        self.push_token(kind, start, self.offset, text, false);
        self.insert_comma = false;
    }

    fn scan_closing(&mut self, kind: TokenKind) {
        let start = self.offset;
        self.advance_one();
        let text = self.text(start, self.offset).to_owned();
        self.push_token(kind, start, self.offset, text, false);
        self.insert_comma = true;
    }

    fn scan_explicit_comma(&mut self) {
        let start = self.offset;
        self.advance_one();
        let text = self.text(start, self.offset).to_owned();
        self.push_token(TokenKind::Comma, start, self.offset, text, false);
        self.insert_comma = false;
    }

    fn scan_dot(&mut self) {
        let start = self.offset;
        self.advance_one();
        if self.peek_byte() == Some(b'.') && self.peek_next_byte() == Some(b'.') {
            self.advance_one();
            self.advance_one();
            self.push_token(TokenKind::Ellipsis, start, self.offset, "...", false);
            self.insert_comma = true;
            return;
        }
        self.push_token(TokenKind::Dot, start, self.offset, ".", false);
        self.insert_comma = false;
    }

    fn scan_operator(&mut self) {
        let start = self.offset;
        self.advance_while(|byte| {
            matches!(
                byte,
                b'+' | b'-' | b'=' | b'!' | b'<' | b'>' | b'&' | b'|' | b'~' | b'?'
            )
        });
        let text = self.text(start, self.offset).to_owned();
        self.push_token(TokenKind::Operator, start, self.offset, text, false);
        self.insert_comma = false;
    }

    fn scan_bad_byte(&mut self) {
        let start = self.offset;
        self.advance_one();
        let text = self.text(start, self.offset).to_owned();
        self.push_diagnostic(
            "cue.scan.unexpected_byte",
            "unexpected byte in source",
            start,
            self.offset,
        );
        self.push_token(TokenKind::Bad, start, self.offset, text, false);
    }

    fn peek_byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.offset).copied()
    }

    fn peek_next_byte(&self) -> Option<u8> {
        self.input
            .as_bytes()
            .get(self.offset.saturating_add(1))
            .copied()
    }

    fn advance_one(&mut self) {
        self.offset = self.offset.saturating_add(1);
    }

    fn advance_while(&mut self, mut predicate: impl FnMut(u8) -> bool) {
        while let Some(byte) = self.peek_byte() {
            if !predicate(byte) {
                break;
            }
            self.advance_one();
        }
    }

    fn text(&self, start: usize, end: usize) -> &str {
        self.input.get(start..end).unwrap_or("")
    }

    fn push_token(
        &mut self,
        kind: TokenKind,
        start: usize,
        end: usize,
        text: impl Into<String>,
        inserted: bool,
    ) {
        if let Ok(span) = self.span(start, end) {
            self.tokens.push(Token::new(kind, span, text, inserted));
        }
    }

    fn push_diagnostic(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        start: usize,
        end: usize,
    ) {
        let span = self.span(start, end).ok();
        self.diagnostics
            .push(Diagnostic::new(Severity::Error, code, message, span));
    }

    fn span(&self, start: usize, end: usize) -> Result<Span, SourceError> {
        let start = ByteOffset(u32::try_from(start).map_err(|_| SourceError::OffsetTooLarge)?);
        let end = ByteOffset(u32::try_from(end).map_err(|_| SourceError::OffsetTooLarge)?);
        Span::new(self.source, start, end)
    }
}

fn is_valid_number_literal(text: &str) -> bool {
    let (mantissa, exponent) = split_exponent(text);
    if let Some(exponent) = exponent {
        let exponent_digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if !is_valid_digit_run(exponent_digits) {
            return false;
        }
    }

    if let Some((whole, fraction)) = mantissa.split_once('.') {
        return is_valid_digit_run(whole) && is_valid_digit_run(fraction);
    }

    is_valid_digit_run(mantissa)
}

fn split_exponent(text: &str) -> (&str, Option<&str>) {
    let Some(index) = text.find(['e', 'E']) else {
        return (text, None);
    };
    let exponent_start = index.saturating_add(1);
    (&text[..index], text.get(exponent_start..))
}

fn is_valid_digit_run(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    let mut seen_digit = false;
    let mut previous_was_underscore = false;
    for byte in text.bytes() {
        match byte {
            b'0'..=b'9' => {
                seen_digit = true;
                previous_was_underscore = false;
            }
            b'_' if !seen_digit || previous_was_underscore => return false,
            b'_' => previous_was_underscore = true,
            _ => return false,
        }
    }

    !previous_was_underscore
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::{ParseConfig, Token, TokenKind, scan_bytes};

    #[rstest]
    #[case(
        "package x\nfoo: 1",
        vec![
            TokenKind::Package,
            TokenKind::Identifier,
            TokenKind::Comma,
            TokenKind::Identifier
        ]
    )]
    #[case(
        "foo: \"bar\"",
        vec![TokenKind::Identifier, TokenKind::Colon, TokenKind::String]
    )]
    #[case(
        "a: 1 // comment\nb: 2",
        vec![
            TokenKind::Identifier,
            TokenKind::Colon,
            TokenKind::Number,
            TokenKind::Comma
        ]
    )]
    fn test_should_scan_core_tokens(#[case] input: &str, #[case] prefix: Vec<TokenKind>) {
        let result = scan_bytes("test.cue", input.as_bytes(), ParseConfig::default());
        assert!(!result.diagnostics().has_errors());
        let kinds: Vec<TokenKind> = result.tokens().iter().map(Token::kind).collect();
        assert!(kinds.starts_with(&prefix));
    }

    #[test]
    fn test_should_scan_number_operators_separately() {
        let result = scan_bytes("test.cue", b"x: 1+2\ny: 1e-2\n", ParseConfig::default());
        assert!(!result.diagnostics().has_errors());
        let tokens = result.tokens();
        let number_texts = tokens
            .iter()
            .filter(|token| token.kind() == TokenKind::Number)
            .map(Token::text)
            .collect::<Vec<_>>();
        assert_eq!(vec!["1", "2", "1e-2"], number_texts);
        assert!(tokens.iter().any(|token| token.text() == "+"));
    }

    #[rstest]
    #[case("1_000")]
    #[case("1_000.5_0")]
    #[case("1e1_0")]
    #[case("1_0e+2")]
    #[case("1_0E-2")]
    fn test_should_accept_valid_underscored_number_literals(#[case] literal: &str) {
        let source = format!("x: {literal}\n");
        let result = scan_bytes("test.cue", source.as_bytes(), ParseConfig::default());

        assert!(!result.diagnostics().has_errors(), "{literal}");
    }

    #[rstest]
    #[case("1_")]
    #[case("1__2")]
    #[case("1e")]
    #[case("1e_")]
    #[case("1._2")]
    #[case("1e+")]
    #[case("1e+_2")]
    fn test_should_report_invalid_number_literals(#[case] literal: &str) {
        let source = format!("x: {literal}\n");
        let result = scan_bytes("test.cue", source.as_bytes(), ParseConfig::default());
        let codes = result
            .diagnostics()
            .diagnostics()
            .iter()
            .map(cue_rust_source::Diagnostic::code)
            .collect::<Vec<_>>();

        assert!(codes.contains(&"cue.scan.invalid_number"), "{literal}");
    }

    #[test]
    fn test_should_scan_regex_operators() {
        let result = scan_bytes(
            "test.cue",
            br#"x: "foo" =~ "f.*"
y: !~"bar"
"#,
            ParseConfig::default(),
        );
        assert!(!result.diagnostics().has_errors());
        let operators = result
            .tokens()
            .iter()
            .filter(|token| token.kind() == TokenKind::Operator)
            .map(Token::text)
            .collect::<Vec<_>>();
        assert!(operators.contains(&"=~"));
        assert!(operators.contains(&"!~"));
    }

    #[test]
    fn test_should_report_nul_bytes() {
        let result = scan_bytes("bad.cue", b"a: \0", ParseConfig::default());
        let first_code = result
            .diagnostics()
            .diagnostics()
            .first()
            .map(cue_rust_source::Diagnostic::code);
        assert!(result.diagnostics().has_errors());
        assert_eq!(Some("cue.scan.nul"), first_code);
    }

    #[test]
    fn test_should_reject_invalid_utf8_without_panic() {
        let result = scan_bytes("bad.cue", &[0xFF], ParseConfig::default());
        let first_code = result
            .diagnostics()
            .diagnostics()
            .first()
            .map(cue_rust_source::Diagnostic::code);
        assert!(result.diagnostics().has_errors());
        assert_eq!(Some("cue.source.invalid"), first_code);
    }

    proptest! {
        #[test]
        fn test_should_not_panic_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let _result = scan_bytes("fuzz.cue", &bytes, ParseConfig::default());
        }
    }
}
