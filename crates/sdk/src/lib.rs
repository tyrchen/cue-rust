//! Public SDK facade for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use camino::Utf8PathBuf;
use cue_rust_adt::Runtime;
pub use cue_rust_compiler::CompiledInstance;
use cue_rust_compiler::{CompileError, CompileOptions, Compiler};
pub use cue_rust_encoding::{
    DecodeError, DecodeOptions, EncodeError, EncodeOptions, Encoding, decode_bytes, encode_value,
};
pub use cue_rust_eval::{
    EvalError, EvalOptions, EvaluatedValue, ExportOptions, ValidateOptions, Value, ValueKind,
};
pub use cue_rust_loader::{BuildInstance, LoadConfig, LoadError, Loader, PackageSelector};
pub use cue_rust_source::{DiagnosticReport, SourceError, SourceFile, SourceLimits};
pub use cue_rust_syntax::{
    AstFile, Decl, Expr, FieldDecl, FieldMarker, ImportDecl, Label, LetDecl, PackageClause,
    ParseConfig, ParseMode, ParseResult, ParsedSource, ScanResult, Token, TokenKind,
};
use thiserror::Error;

/// Current SDK version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Top-level SDK error.
#[derive(Debug, Error)]
pub enum CueError {
    /// Compilation infrastructure failed.
    #[error(transparent)]
    Compile(#[from] CompileError),
    /// Evaluation failed.
    #[error(transparent)]
    Eval(#[from] EvalError),
    /// Data decoding failed.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// Data encoding failed.
    #[error(transparent)]
    Encode(#[from] EncodeError),
    /// Loading failed.
    #[error(transparent)]
    Load(#[from] LoadError),
    /// Source, parse, compile, or validation diagnostics were emitted.
    #[error("operation produced diagnostics")]
    Diagnostics(DiagnosticReport),
}

/// Top-level SDK context.
#[derive(Clone, Debug, Default)]
pub struct Context {
    parse_config: ParseConfig,
}

impl Context {
    /// Creates a context with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a context with an explicit parser configuration.
    #[must_use]
    pub fn with_parse_config(parse_config: ParseConfig) -> Self {
        Self { parse_config }
    }

    /// Parses a named source into a tolerant AST and diagnostics.
    #[must_use]
    pub fn parse_source(&self, name: impl Into<String>, content: impl Into<String>) -> ParseResult {
        let content = content.into();
        cue_rust_syntax::parse_bytes(name, content.as_bytes(), self.parse_config)
    }

    /// Parses raw source bytes into a tolerant AST and diagnostics.
    #[must_use]
    pub fn parse_source_bytes(&self, name: impl Into<String>, bytes: &[u8]) -> ParseResult {
        cue_rust_syntax::parse_bytes(name, bytes, self.parse_config)
    }

    /// Scans raw source bytes into syntax tokens and diagnostics.
    #[must_use]
    pub fn scan_source_bytes(&self, name: impl Into<String>, bytes: &[u8]) -> ScanResult {
        cue_rust_syntax::scan_bytes(name, bytes, self.parse_config)
    }

    /// Loads local package arguments into build instances.
    ///
    /// # Errors
    ///
    /// Returns [`CueError`] when loading fails.
    pub async fn load(
        &self,
        config: LoadConfig,
        args: &[Utf8PathBuf],
    ) -> Result<Vec<BuildInstance>, CueError> {
        Ok(Loader::new(config).load_args(args).await?)
    }

    /// Compiles a named source into a value handle.
    ///
    /// # Errors
    ///
    /// Returns [`CueError`] when parsing or compilation emits errors.
    pub fn compile_source(
        &self,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Value, CueError> {
        let content = content.into();
        self.compile_source_bytes(name, content.as_bytes())
    }

    /// Compiles raw source bytes into a value handle.
    ///
    /// # Errors
    ///
    /// Returns [`CueError`] when parsing or compilation emits errors.
    pub fn compile_source_bytes(
        &self,
        name: impl Into<String>,
        bytes: &[u8],
    ) -> Result<Value, CueError> {
        let parsed = self.parse_source_bytes(name, bytes);
        if parsed.diagnostics().has_errors() {
            return Err(CueError::Diagnostics(parsed.diagnostics().clone()));
        }
        let files = parsed.ast().map_or_else(Vec::new, |ast| vec![ast.clone()]);
        let instance = BuildInstance::new(None, files);
        self.build_instance(&instance)
    }

    /// Builds a parsed instance into a value handle.
    ///
    /// # Errors
    ///
    /// Returns [`CueError`] when compilation emits errors or ADT construction fails.
    pub fn build_instance(&self, instance: &BuildInstance) -> Result<Value, CueError> {
        let mut runtime = Runtime::default();
        let compiled =
            Compiler::new(&mut runtime).compile_instance(instance, CompileOptions::default())?;
        let diagnostics = compiled.diagnostics().clone();
        if diagnostics.has_errors() {
            return Err(CueError::Diagnostics(diagnostics));
        }
        Ok(Value::new(runtime, compiled.root(), diagnostics))
    }

    /// Compiles an expression in the lexical context of an existing build instance.
    ///
    /// # Errors
    ///
    /// Returns [`CueError`] when the expression cannot be parsed, compiled, or selected.
    pub fn compile_instance_expression(
        &self,
        instance: &BuildInstance,
        expression: &str,
    ) -> Result<Value, CueError> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err(CueError::Diagnostics(single_diagnostic(
                "cue.sdk.empty_expression",
                "expression must not be empty",
            )));
        }
        let source = format!("__cue_rs_expr: ({expression})\n");
        let parsed = self.parse_source("__cue_rs_expr.cue", source);
        if parsed.diagnostics().has_errors() {
            return Err(CueError::Diagnostics(parsed.diagnostics().clone()));
        }
        let mut files = instance.files().to_vec();
        if let Some(ast) = parsed.ast() {
            files.push(ast.clone());
        }
        let extended = BuildInstance::new(instance.package_name().map(ToOwned::to_owned), files);
        self.build_instance(&extended)?
            .lookup_path(&["__cue_rs_expr"])
            .map_err(CueError::from)
    }
}

fn single_diagnostic(code: &'static str, message: &'static str) -> DiagnosticReport {
    let mut report = DiagnosticReport::new();
    report.push(cue_rust_source::Diagnostic::new(
        cue_rust_source::Severity::Error,
        code,
        message,
        None,
    ));
    report
}

#[cfg(test)]
mod tests {
    use super::{
        Context, CueError, DecodeOptions, EncodeOptions, Encoding, EvaluatedValue, ValidateOptions,
        ValueKind, decode_bytes, encode_value,
    };

    #[test]
    fn test_should_compile_source_and_lookup_value() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: 1\n")?;
        assert_eq!(ValueKind::Number, value.lookup_path(&["x"])?.kind()?);
        Ok(())
    }

    #[test]
    fn test_should_validate_builtin_kind_schema() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let schema = context.compile_source("schema.cue", "name: string\n")?;
        let data = context.compile_source("data.cue", "name: \"cue\"\n")?;
        schema.unify(&data)?.validate(ValidateOptions::default())?;
        Ok(())
    }

    #[test]
    fn test_should_unify_equivalent_numeric_literals() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "x: 2\nx: 2.0\nlarge: 9007199254740993 == 9007199254740992\nbound: 9007199254740993 & \
             >9007199254740992\n",
        )?;
        assert_eq!(
            EvaluatedValue::Number("2".to_owned()),
            value.lookup_path(&["x"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(false),
            value.lookup_path(&["large"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("9007199254740993".to_owned()),
            value.lookup_path(&["bound"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_numeric_builtin_kind_constraints()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "integer: 1 & int\nfloating: 1.0 & float\nnarrowed: number & int\nbad: int & float\n",
        )?;
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["integer"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1.0".to_owned()),
            value.lookup_path(&["floating"])?.evaluate()?,
        );
        assert_eq!(ValueKind::Int, value.lookup_path(&["narrowed"])?.kind()?);
        assert!(
            value
                .lookup_path(&["bad"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_report_invalid_unary_plus_operand() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: +true\n")?;
        assert!(
            value
                .lookup_path(&["x"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_recursive_equality() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "list: [1, {a: 2}] == [1.0, {a: 2}]\nstruct: {a: 1, b: 2} == {b: 2, a: 1}\nmissing: \
             {a: 1} == {a: 1, b: 2}\nerr: [1/0] == [1]\n",
        )?;
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["list"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["struct"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(false),
            value.lookup_path(&["missing"])?.evaluate()?,
        );
        assert!(
            value
                .lookup_path(&["err"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_regex_match_and_constraints() -> Result<(), Box<dyn std::error::Error>>
    {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "match: \"foo\" =~ \"[a-z]{3}\"\nnotMatch: \"foo\" !~ \"[0-9]+\"\nconstrained: \
             =~\"[a-z]+\"\nconstrained: \"cue\"\nbad: =~\"[0-9]+\"\nbad: \"cue\"\n",
        )?;
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["match"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["notMatch"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::String("cue".to_owned()),
            value.lookup_path(&["constrained"])?.evaluate()?,
        );
        assert!(
            value
                .lookup_path(&["bad"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_disjunction_defaults_bounds_and_aggregate_builtins()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "narrow: (1 | 2 | 3) & (>=2 & <=2)\nchosen: *5 | string\nnested: {chosen: *5 | \
             string}\nlenDefault: len(*[1, 2, 3] | 0)\nandValue: and([1, 1])\norValue: or([2, 1, \
             1, 2]) & 1\nclosed: close(*{} | 0)\n",
        )?;
        assert_eq!(
            EvaluatedValue::Number("2".to_owned()),
            value.lookup_path(&["narrow"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("5".to_owned()),
            value
                .lookup_path(&["chosen"])?
                .evaluate()?
                .resolve_defaults(),
        );
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["lenDefault"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["andValue"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["orValue"])?.evaluate()?,
        );
        let EvaluatedValue::ClosedStruct(fields) = value.lookup_path(&["closed"])?.evaluate()?
        else {
            return Err("expected closed struct".into());
        };
        assert!(fields.is_empty());

        let mut options = EncodeOptions::default();
        options.encoding = Encoding::Json;
        let json = encode_value(&value.lookup_path(&["chosen"])?, options)?;
        assert_eq!("5", json);
        let nested_json = encode_value(&value.lookup_path(&["nested"])?, options)?;
        assert!(nested_json.contains("\"chosen\": 5"));
        Ok(())
    }

    #[test]
    fn test_should_reject_extra_fields_for_closed_struct() -> Result<(), Box<dyn std::error::Error>>
    {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: close({a: int}) & {a: 1, b: 2}\n")?;
        assert!(
            value
                .lookup_path(&["x"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_allow_fields_to_shadow_predeclared_builtins()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "or: 1\nand: 2\nx: or + and\n")?;
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["x"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_select_field_through_default_struct() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: *{a: 1} | string\ny: x.a\n")?;
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["y"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["x", "a"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_resolve_nested_field_before_outer_field()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: 1\ny: { x: 2, z: x }\n")?;
        let nested = value.lookup_path(&["y", "z"])?;
        assert_eq!(EvaluatedValue::Number("2".to_owned()), nested.evaluate()?);
        Ok(())
    }

    #[test]
    fn test_should_resolve_let_binding() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "let x = 1\ny: x\n")?;
        let y = value.lookup_path(&["y"])?;
        assert_eq!(EvaluatedValue::Number("1".to_owned()), y.evaluate()?);
        Ok(())
    }

    #[test]
    fn test_should_evaluate_list_index_expression() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: [1, 2, 3][1]\n")?;
        assert_eq!(
            EvaluatedValue::Number("2".to_owned()),
            value.lookup_path(&["x"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_list_slice_expression() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: [1, 2, 3][1:]\ny: [1, 2, 3][:2]\n")?;
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("2".to_owned()),
                EvaluatedValue::Number("3".to_owned()),
            ]),
            value.lookup_path(&["x"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("1".to_owned()),
                EvaluatedValue::Number("2".to_owned()),
            ]),
            value.lookup_path(&["y"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_struct_string_index_expression()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "x: {a: 1, b: {c: 2}}[\"b\"][\"c\"]\ny: {\"quoted\": 3}.quoted\n",
        )?;
        assert_eq!(
            EvaluatedValue::Number("2".to_owned()),
            value.lookup_path(&["x"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["y"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_report_missing_struct_string_index() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: {a: 1}[\"b\"]\n")?;
        assert!(
            value
                .lookup_path(&["x"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_numeric_binary_expressions() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "sum: -1+ +2\nmul: 2*3\nquo: 6/4\neq: 2.0 == 2\nlt: 1 < 2\nge: 2 >= 2.0\n",
        )?;
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["sum"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("6".to_owned()),
            value.lookup_path(&["mul"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1.5".to_owned()),
            value.lookup_path(&["quo"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["eq"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["lt"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["ge"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_len_builtin() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "list: len([1, 2, 3])\nstruct: len({a: 1, b: 2})\nstring: len(\"😂\")\nempty: \
             len(\"\")\n",
        )?;
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["list"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("2".to_owned()),
            value.lookup_path(&["struct"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("4".to_owned()),
            value.lookup_path(&["string"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("0".to_owned()),
            value.lookup_path(&["empty"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_string_and_bytes_operators() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "s0: \"foo\" + \"bar\"\ns1: 3 * \"abc\"\ns2: \"abc\" * 2\nb0: 'foo' + 'bar'\nb1: 3 * \
             'abc'\nb2: 'abc' * 2\n",
        )?;
        assert_eq!(
            EvaluatedValue::String("foobar".to_owned()),
            value.lookup_path(&["s0"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::String("abcabcabc".to_owned()),
            value.lookup_path(&["s1"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::String("abcabc".to_owned()),
            value.lookup_path(&["s2"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bytes(b"foobar".to_vec()),
            value.lookup_path(&["b0"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bytes(b"abcabcabc".to_vec()),
            value.lookup_path(&["b1"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bytes(b"abcabc".to_vec()),
            value.lookup_path(&["b2"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_unescape_string_and_bytes_literals() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "s: \"foo\\nbar\"\nb: 'a\\n\\xff'\n")?;
        assert_eq!(
            EvaluatedValue::String("foo\nbar".to_owned()),
            value.lookup_path(&["s"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bytes(vec![b'a', b'\n', 0xff]),
            value.lookup_path(&["b"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_integer_arithmetic_builtins() -> Result<(), Box<dyn std::error::Error>>
    {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "q: quo(-5, 2)\nr: rem(-5, 2)\nd: div(-5, 2)\nm: mod(-5, 2)\n",
        )?;
        assert_eq!(
            EvaluatedValue::Number("-2".to_owned()),
            value.lookup_path(&["q"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("-1".to_owned()),
            value.lookup_path(&["r"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("-3".to_owned()),
            value.lookup_path(&["d"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["m"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_report_integer_arithmetic_builtin_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let non_integer = context.compile_source("test.cue", "x: quo(2.0, 1)\n")?;
        assert!(
            non_integer
                .lookup_path(&["x"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        let division_by_zero = context.compile_source("test.cue", "x: div(1, 0)\n")?;
        assert!(
            division_by_zero
                .lookup_path(&["x"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_report_len_builtin_arity() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: len()\n")?;
        assert!(
            value
                .lookup_path(&["x"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_report_numeric_division_by_zero() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x: 1 / 0\n")?;
        assert!(
            value
                .lookup_path(&["x"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_export_regular_fields_without_definitions_hidden_or_optional_constraints()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "#Port: int & >=1 & <=65535\n_hidden: \"scratch\"\noptional?: string\nport: #Port & \
             8080\n",
        )?;
        let output = encode_value(&value, EncodeOptions::default())?;
        assert!(output.contains("\"port\": 8080"));
        assert!(!output.contains("#Port"));
        assert!(!output.contains("_hidden"));
        assert!(!output.contains("optional"));
        Ok(())
    }

    #[test]
    fn test_should_report_required_constraint_missing_from_export()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "name!: string\n")?;
        let result = encode_value(&value, EncodeOptions::default());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_export_required_field_when_regular_field_satisfies_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "name!: string\nname: \"cue\"\n")?;
        let output = encode_value(&value, EncodeOptions::default())?;
        assert!(output.contains("\"name\": \"cue\""));
        Ok(())
    }

    #[test]
    fn test_should_reject_optional_field_reference() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x?: 1\ny: x\n")?;
        let result = encode_value(&value, EncodeOptions::default());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_keep_required_constraint_when_optional_constraint_shares_label()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source("test.cue", "x?: string\nx!: string\n")?;
        let result = encode_value(&value, EncodeOptions::default());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_apply_optional_constraint_when_data_field_is_present()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let schema = context.compile_source("schema.cue", "name?: string\n")?;
        let valid = context.compile_source("valid.cue", "name: \"cue\"\n")?;
        schema.unify(&valid)?.validate(ValidateOptions::default())?;
        let invalid = context.compile_source("invalid.cue", "name: 1\n")?;
        assert!(
            schema
                .unify(&invalid)?
                .validate(ValidateOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_preserve_external_data_keys_that_look_like_cue_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let value = decode_bytes(
            Encoding::Json,
            br##"{"_keep":1,"#literal":2,"regular":3}"##,
            DecodeOptions::default(),
        )?;
        let output = encode_value(&value, EncodeOptions::default())?;
        assert!(output.contains("\"_keep\": 1"));
        assert!(output.contains("\"#literal\": 2"));
        assert!(output.contains("\"regular\": 3"));
        Ok(())
    }

    #[test]
    fn test_should_report_import_as_compile_diagnostic() {
        let context = Context::new();
        let result = context.compile_source("test.cue", "import \"strings\"\nx: 1\n");
        assert!(matches!(result, Err(CueError::Diagnostics(_))));
    }
}
