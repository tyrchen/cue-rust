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
    AstFile, ComprehensionClause, ComprehensionDecl, Decl, Expr, FieldDecl, FieldMarker,
    ImportDecl, Label, LetDecl, PackageClause, ParseConfig, ParseMode, ParseResult, ParsedSource,
    ScanResult, StringPart, Token, TokenKind,
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
        let mut extended =
            BuildInstance::new(instance.package_name().map(ToOwned::to_owned), files);
        extended.set_imports(instance.imports().clone());
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
        Context, CueError, DecodeOptions, EncodeOptions, Encoding, EvalError, EvaluatedValue,
        ValidateOptions, Value, ValueKind, decode_bytes, encode_value,
    };

    fn assert_evaluated_path(
        value: &Value,
        label: &str,
        expected: &EvaluatedValue,
    ) -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(expected, &value.lookup_path(&[label])?.evaluate()?);
        Ok(())
    }

    fn string_list(items: &[&str]) -> EvaluatedValue {
        EvaluatedValue::List(
            items
                .iter()
                .map(|item| EvaluatedValue::String((*item).to_owned()))
                .collect(),
        )
    }

    const TOP_LEVEL_CYCLE_SOURCE: &str =
        "ordinary: {x: y, y: x}\nstructural: a: c: a\naTop: 100\nrootListT0: \
         [rootListT0[0]]\nrootListPair: [rootListPair[1], rootListPair[0]]\nrootListGrounded: \
         [rootListGrounded[1], aTop]\nrootListConj: [rootListConj[0]] & [1]\nselectorOk: {b: \
         selectorOk.c, c: 1}\nselectorFixpoint: {b: selectorFixpoint.c, c: c & 1}\nselectorConj: \
         {b: selectorConj.c} & {c: 1}\nselectorConjFixpoint: {b: selectorConjFixpoint.c} & {c: c \
         & 1}\nnotSelfList: {b: [notSelfList[0]], c: notSelfList}\nselectorGenerated: {b: \
         selectorGenerated.c, for k, v in {c: 1} {\"\\(k)\": v}}\nselectorDynamicKey: \
         \"c\"\nselectorDynamic: {b: selectorDynamic.c, (selectorDynamicKey): \
         1}\nselectorGeneratedMerge: {b: selectorGeneratedMerge.c, c: int, for k, v in {c: 1} \
         {\"\\(k)\": v}}\nselectorDynamicMergeKey: \"c\"\nselectorDynamicMerge: {b: \
         selectorDynamicMerge.c, c: int, (selectorDynamicMergeKey): \
         1}\nselectorDynamicConflictKey: \"c\"\nselectorDynamicConflict: {b: \
         selectorDynamicConflict.c, c: 1, (selectorDynamicConflictKey): \
         2}\nselectorPatternConflict: {b: selectorPatternConflict.c, [=~\"^c$\"]: int, c: \
         \"bad\"}\nselectorGeneratedPatternConflict: {b: selectorGeneratedPatternConflict.c, \
         [=~\"^c$\"]: int, for k, v in {c: \"bad\"} {\"\\(k)\": v}}\n";

    fn compile_top_level_cycle_fixture() -> Result<Value, Box<dyn std::error::Error>> {
        Context::new()
            .compile_source("test.cue", TOP_LEVEL_CYCLE_SOURCE)
            .map_err(Into::into)
    }

    fn number_list(items: &[&str]) -> EvaluatedValue {
        EvaluatedValue::List(
            items
                .iter()
                .map(|item| EvaluatedValue::Number((*item).to_owned()))
                .collect(),
        )
    }

    fn assert_number_list(value: &EvaluatedValue, expected: &[&str]) {
        let expected = number_list(expected);
        assert_eq!(&expected, value);
    }

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
    fn test_should_reject_abstract_arithmetic_operands() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "abstract.cue",
            "nested: (int + 1) + (int + 1)\ndisjunct: (int + 1) | (int + 1)\nboundMultiply: (>10 \
             * 2) & 0\nboundCompare: >=100 <= 200\n",
        )?;
        for path in ["nested", "disjunct", "boundMultiply", "boundCompare"] {
            assert!(
                value
                    .lookup_path(&[path])?
                    .validate(ValidateOptions::default())
                    .is_err(),
                "expected {path} to reject abstract arithmetic",
            );
        }
        Ok(())
    }

    #[test]
    fn test_should_evaluate_default_disjunction_operands() -> Result<(), Box<dyn std::error::Error>>
    {
        let context = Context::new();
        let value = context.compile_source(
            "operands.cue",
            "list: *[1] | [2]\ncondition: *true | false\nnum: *1 | 2\nobject: *{a: 1} | {a: \
             2}\nforLoop: [for e in list {\"count: \\(e)\"}]\nconditional: {if condition {a: 3}, \
             if num < 5 {b: 3}}\nselector: {a: object.a}\nindex: {a: list[0]}\nbinOp: {a: num + \
             4}\nunaryOp: {a: -num}\n",
        )?;
        assert_eq!(
            EvaluatedValue::List(vec![EvaluatedValue::String("count: 1".to_owned())]),
            value.lookup_path(&["forLoop"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["conditional", "a"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["conditional", "b"])?.evaluate()?,
        );
        for path in [&["selector", "a"][..], &["index", "a"][..]] {
            assert_eq!(
                EvaluatedValue::Number("1".to_owned()),
                value.lookup_path(path)?.evaluate()?,
                "unexpected value at {path:?}",
            );
        }
        assert_eq!(
            EvaluatedValue::Number("5".to_owned()),
            value.lookup_path(&["binOp", "a"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("-1".to_owned()),
            value.lookup_path(&["unaryOp", "a"])?.evaluate()?,
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
    fn test_should_resolve_local_list_self_index_cycles() -> Result<(), Box<dyn std::error::Error>>
    {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "a: 100\nlist: {t0: c: [c[0]], grouped: c: [(c)[0]], pair: c: [c[1], c[0]], p1: c: \
             [c[1], a], p2: c: [a, c[0]], open: c: [c[1], ...int], badIndex: c: [c[\"x\"]], \
             chain: c: [c[1], c[2], 42], addSelf: c: [c[0] + 1], foreign: {a: {b: a}.b, c: [a[1], \
             2]}, constrained: {a: 100, c: [c[1], a] & [100, 100]}}\n",
        )?;

        assert_eq!(
            EvaluatedValue::List(vec![EvaluatedValue::Top]),
            value.lookup_path(&["list", "t0", "c"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![EvaluatedValue::Top]),
            value.lookup_path(&["list", "grouped", "c"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![EvaluatedValue::Top, EvaluatedValue::Top]),
            value.lookup_path(&["list", "pair", "c"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("100".to_owned()),
                EvaluatedValue::Number("100".to_owned()),
            ]),
            value.lookup_path(&["list", "p1", "c"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("100".to_owned()),
                EvaluatedValue::Number("100".to_owned()),
            ]),
            value.lookup_path(&["list", "p2", "c"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::OpenList {
                items: vec![EvaluatedValue::Kind(ValueKind::Int)],
                tail: Box::new(EvaluatedValue::Kind(ValueKind::Int)),
            },
            value.lookup_path(&["list", "open", "c"])?.evaluate()?,
        );
        assert!(matches!(
            value.lookup_path(&["list", "badIndex", "c"])?.evaluate()?,
            EvaluatedValue::List(items)
                if matches!(
                    items.first(),
                    Some(EvaluatedValue::Bottom(bottom))
                        if bottom.code == "cue.eval.invalid_index"
                ),
        ));
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("42".to_owned()),
                EvaluatedValue::Number("42".to_owned()),
                EvaluatedValue::Number("42".to_owned()),
            ]),
            value.lookup_path(&["list", "chain", "c"])?.evaluate()?,
        );
        assert!(matches!(
            value.lookup_path(&["list", "addSelf", "c"])?.evaluate()?,
            EvaluatedValue::List(items)
                if matches!(
                    items.first(),
                    Some(EvaluatedValue::Bottom(bottom))
                        if bottom.code == "cue.eval.unsupported_add"
                ),
        ));
        assert!(matches!(
            value.lookup_path(&["list", "foreign", "c"])?.evaluate()?,
            EvaluatedValue::List(items)
                if matches!(
                    items.first(),
                    Some(EvaluatedValue::Bottom(bottom))
                        if bottom.code == "cue.eval.structural_cycle"
                ),
        ));
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("100".to_owned()),
                EvaluatedValue::Number("100".to_owned()),
            ]),
            value
                .lookup_path(&["list", "constrained", "c"])?
                .evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_resolve_structural_cycles_to_fixpoint() -> Result<(), Box<dyn std::error::Error>>
    {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "shadow: {x: {y: {z: x, x: 1}}}\nordinary: {y: x, x: y}\ninvalid: {a: c: a}\ntop: s1: \
             s1 & {a: 1}\nnodes: {\nself: s1: s1 & {a: 1}\ntwo: {s1: s2 & {a: 1}, s2: s1 & {b: \
             2}}\nthree: {s1: s2 & {a: 1}, s2: s3 & {b: 2}, s3: s1 & {c: 3}}\n}\n",
        )?;

        assert_evaluated_path(
            &value.lookup_path(&["shadow", "x", "y"])?,
            "z",
            &EvaluatedValue::Number("1".to_owned()),
        )?;
        assert_eq!(
            EvaluatedValue::Top,
            value.lookup_path(&["ordinary", "x"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Top,
            value.lookup_path(&["ordinary", "y"])?.evaluate()?,
        );
        let mut options = ValidateOptions::default();
        options.concrete = false;
        value.lookup_path(&["ordinary"])?.validate(options)?;
        assert!(matches!(
            value.lookup_path(&["invalid", "a", "c"])?.evaluate()?,
            EvaluatedValue::Bottom(bottom) if bottom.code == "cue.eval.structural_cycle",
        ));
        assert_evaluated_path(
            &value.lookup_path(&["top", "s1"])?,
            "a",
            &EvaluatedValue::Number("1".to_owned()),
        )?;
        assert_evaluated_path(
            &value.lookup_path(&["nodes", "self", "s1"])?,
            "a",
            &EvaluatedValue::Number("1".to_owned()),
        )?;
        assert_evaluated_path(
            &value.lookup_path(&["nodes", "two", "s1"])?,
            "b",
            &EvaluatedValue::Number("2".to_owned()),
        )?;
        assert_evaluated_path(
            &value.lookup_path(&["nodes", "two", "s2"])?,
            "a",
            &EvaluatedValue::Number("1".to_owned()),
        )?;
        assert_evaluated_path(
            &value.lookup_path(&["nodes", "three", "s1"])?,
            "c",
            &EvaluatedValue::Number("3".to_owned()),
        )?;
        assert_evaluated_path(
            &value.lookup_path(&["nodes", "three", "s2"])?,
            "a",
            &EvaluatedValue::Number("1".to_owned()),
        )?;
        assert_evaluated_path(
            &value.lookup_path(&["nodes", "three", "s3"])?,
            "b",
            &EvaluatedValue::Number("2".to_owned()),
        )?;
        Ok(())
    }

    #[test]
    fn test_should_resolve_top_level_vertex_cycles() -> Result<(), Box<dyn std::error::Error>> {
        let value = compile_top_level_cycle_fixture()?;

        assert_eq!(
            EvaluatedValue::Top,
            value.lookup_path(&["ordinary", "x"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Top,
            value.lookup_path(&["ordinary", "y"])?.evaluate()?,
        );
        assert!(matches!(
            value.lookup_path(&["structural", "a", "c"])?.evaluate()?,
            EvaluatedValue::Bottom(bottom) if bottom.code == "cue.eval.structural_cycle",
        ));
        assert_eq!(
            EvaluatedValue::List(vec![EvaluatedValue::Top]),
            value.lookup_path(&["rootListT0"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![EvaluatedValue::Top, EvaluatedValue::Top]),
            value.lookup_path(&["rootListPair"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("100".to_owned()),
                EvaluatedValue::Number("100".to_owned()),
            ]),
            value.lookup_path(&["rootListGrounded"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![EvaluatedValue::Number("1".to_owned())]),
            value.lookup_path(&["rootListConj"])?.evaluate()?,
        );
        assert!(matches!(
            value.lookup_path(&["notSelfList", "b"])?.evaluate()?,
            EvaluatedValue::List(items)
                if matches!(
                    items.first(),
                    Some(EvaluatedValue::Bottom(bottom))
                        if bottom.code == "cue.eval.structural_cycle"
                ),
        ));
        Ok(())
    }

    #[test]
    fn test_should_resolve_in_progress_selector_cycles() -> Result<(), Box<dyn std::error::Error>> {
        let value = compile_top_level_cycle_fixture()?;

        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["selectorOk", "b"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["selectorFixpoint", "b"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["selectorConj", "b"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value
                .lookup_path(&["selectorConjFixpoint", "b"])?
                .evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["selectorGenerated", "b"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["selectorDynamic", "b"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value
                .lookup_path(&["selectorGeneratedMerge", "b"])?
                .evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value
                .lookup_path(&["selectorDynamicMerge", "b"])?
                .evaluate()?,
        );
        assert!(matches!(
            value
                .lookup_path(&["selectorDynamicConflict", "b"])?
                .evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert!(matches!(
            value
                .lookup_path(&["selectorPatternConflict", "b"])?
                .evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert!(matches!(
            value
                .lookup_path(&["selectorGeneratedPatternConflict", "b"])?
                .evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
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
             1, 2]) & 1\nclosed: close(*{} | 0)\ndefaultBound: {min: *1 | int, range: >min, \
             range: 8}\ndefaultBoundSchema: {min: *1 | int, max: int & \
             >min}\ndefaultBoundGrounded: defaultBoundSchema\ndefaultBoundGrounded: {max: 8}\n",
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
        assert_eq!(
            EvaluatedValue::Number("8".to_owned()),
            value.lookup_path(&["defaultBound", "range"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("8".to_owned()),
            value
                .lookup_path(&["defaultBoundGrounded", "max"])?
                .evaluate()?,
        );

        let mut options = EncodeOptions::default();
        options.encoding = Encoding::Json;
        let json = encode_value(&value.lookup_path(&["chosen"])?, options)?;
        assert_eq!("5", json);
        let nested_json = encode_value(&value.lookup_path(&["nested"])?, options)?;
        assert!(nested_json.contains("\"chosen\": 5"));
        Ok(())
    }

    #[test]
    fn test_should_resolve_disjunction_cycle_fixpoints() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "cycle.cue",
            "xa1: (xa2 & 8) | (xa4 & 9)\nxa2: xa3 + 2\nxa3: 6 & xa1-2\nxa4: xa2 + 2\nda1: (da2 & \
             8) | *(da4 & 9)\nda2: da3 + 2\nda3: 6 & da1-2\nda4: da2 + 2\nxb1: (xb2 & 8) | (xb4 & \
             9)\nxb2: xb3 + 2\nxb3: (6 & (xb1 - 2)) | (xb4 & 9)\nxb4: xb2 + 2\ndb1: *(db2 & 8) | \
             (db4 & 9)\ndb2: db3 + 2\ndb3: *(6 & (db1 - 2)) | (db4 & 9)\ndb4: db2 + 2\n",
        )?;
        for (path, expected) in [
            ("xa1", "8"),
            ("xa2", "8"),
            ("xa3", "6"),
            ("xa4", "10"),
            ("da1", "8"),
            ("da2", "8"),
            ("da3", "6"),
            ("da4", "10"),
        ] {
            assert_eq!(
                EvaluatedValue::Number(expected.to_owned()),
                value.lookup_path(&[path])?.evaluate()?,
                "unexpected value for {path}",
            );
        }
        for path in ["xb1", "xb2", "xb3", "xb4", "db1", "db2", "db3", "db4"] {
            assert!(matches!(
                value.lookup_path(&[path])?.evaluate()?,
                EvaluatedValue::Bottom(bottom) if bottom.code == "cue.eval.cycle",
            ));
        }
        Ok(())
    }

    #[test]
    fn test_should_resolve_acyclic_defaults_while_solving_cycles()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "cycle-default-operands.cue",
            "flag: *true | false\nx: y | {if flag {a: 1}}\ny: x & {a: 1}\n",
        )?;
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["x", "a"])?.evaluate()?,
        );
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
    fn test_should_apply_closedness_to_definitions_and_patterns()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "closedness.cue",
            "#D: {env: a: \"A\", env: b: \"B\"}\nclosedDefinitionBad: #D & {env: c: \
             \"C\"}\nclosedPatternOk: close({[=~\"^a\"]: string}) & {apple: \
             \"ok\"}\nclosedPatternBad: close({[=~\"^a\"]: string}) & {banana: \
             \"bad\"}\ncloseShallow: close({a: b: int}) & {a: c: int}\n",
        )?;
        assert!(
            value
                .lookup_path(&["closedDefinitionBad"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        assert_eq!(
            EvaluatedValue::String("ok".to_owned()),
            value
                .lookup_path(&["closedPatternOk", "apple"])?
                .evaluate()?
        );
        assert!(
            value
                .lookup_path(&["closedPatternBad"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        assert_eq!(
            ValueKind::Int,
            value.lookup_path(&["closeShallow", "a", "c"])?.kind()?,
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
    fn test_should_evaluate_interpolation_dynamic_labels_and_comprehensions()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "phase9.cue",
            "key: \"name\"\ninterp: \"1+1=\\(1+1), ok=\\(true)\"\ndynamic: {(key): \"cue\", \
             \"\\(1)\": 1}\nsrc: {b: 1, c: 2}\ncomp: {for k, v in src {\"\\(k)\": v + 1}}\nlist: \
             [3, for x in [2, 3] if x > 2 {x}]\nempty: [for x in [] {x}]\n",
        )?;
        assert_eq!(
            EvaluatedValue::String("1+1=2, ok=true".to_owned()),
            value.lookup_path(&["interp"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::String("cue".to_owned()),
            value.lookup_path(&["dynamic", "name"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["dynamic", "1"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("2".to_owned()),
            value.lookup_path(&["comp", "b"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("3".to_owned()),
                EvaluatedValue::Number("3".to_owned()),
            ]),
            value.lookup_path(&["list"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(Vec::new()),
            value.lookup_path(&["empty"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    fn test_should_evaluate_list_sort_constants_and_detect_cycles()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "phase9-list.cue",
            "import \"list\"\nasc: list.Sort([2, 3, 1], list.Ascending)\ndesc: \
             list.SortStable([\"a\", \"c\", \"b\"], list.Descending)\ncustom: list.Sort([{a: 2}, \
             {a: 1}], {x: _, y: _, less: x.a < y.a})\ndup: list.Sort([{a: 1, i: 1}, {a: 1, i: \
             2}], {x: _, y: _, less: x.a < y.a})\nok: list.IsSorted(asc, \
             list.Ascending)\ncustomOk: list.IsSorted(custom, {x: _, y: _, less: x.a < \
             y.a})\nbad: list.IsSorted([2, 1], list.Ascending)\nbadSchema: list.Sort([\"b\", \
             \"a\"], {x: int, y: int, less: false})\ncycle: cycle\n",
        )?;
        assert_eq!(
            number_list(&["1", "2", "3"]),
            value.lookup_path(&["asc"])?.evaluate()?,
        );
        assert_eq!(
            string_list(&["c", "b", "a"]),
            value.lookup_path(&["desc"])?.evaluate()?,
        );
        let EvaluatedValue::List(custom) = value.lookup_path(&["custom"])?.evaluate()? else {
            return Err("expected custom sort list".into());
        };
        let Some(EvaluatedValue::Struct(first)) = custom.first() else {
            return Err("expected first custom sort item".into());
        };
        assert_eq!(
            Some(&EvaluatedValue::Number("1".to_owned())),
            first.get("a"),
        );
        let EvaluatedValue::List(dup) = value.lookup_path(&["dup"])?.evaluate()? else {
            return Err("expected duplicate-key sort list".into());
        };
        let Some(EvaluatedValue::Struct(first_dup)) = dup.first() else {
            return Err("expected first duplicate-key sort item".into());
        };
        let Some(EvaluatedValue::Struct(second_dup)) = dup.get(1) else {
            return Err("expected second duplicate-key sort item".into());
        };
        assert_eq!(
            Some(&EvaluatedValue::Number("1".to_owned())),
            first_dup.get("i"),
        );
        assert_eq!(
            Some(&EvaluatedValue::Number("2".to_owned())),
            second_dup.get("i"),
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["ok"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["customOk"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(false),
            value.lookup_path(&["bad"])?.evaluate()?,
        );
        assert!(
            value
                .lookup_path(&["badSchema"])?
                .validate(ValidateOptions::default())
                .is_err(),
        );
        assert!(
            value
                .lookup_path(&["cycle"])?
                .validate(ValidateOptions::default())
                .is_err(),
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
    fn test_should_evaluate_supported_standard_library_imports()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "test.cue",
            "import s \"strings\"\nimport l \"list\"\ntrimmed: s.TrimSpace(\" cue \")\nupper: \
             s.ToUpper(\"cue\")\njoined: s.Join([\"cue\", \"rust\"], \"-\")\nsplit: \
             s.Split(\"a.b\", \".\")\ncontains: l.Contains([\"cue\", \"rust\"], \
             \"rust\")\nhasPrefix: s.HasPrefix(\"cue-rust\", \"cue\")\nhasSuffix: \
             s.HasSuffix(\"cue-rust\", \"rust\")\ncount: s.Count(\"banana\", \"na\")\nindex: \
             s.Index(\"cue-rust\", \"rust\")\nstringRepeat: s.Repeat(\"ha\", 3)\nrepeated: \
             l.Repeat([\"x\"], 3)\nconcatenated: l.Concat([[1], [2, 3]])\n",
        )?;
        assert_eq!(
            EvaluatedValue::String("cue".to_owned()),
            value.lookup_path(&["trimmed"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::String("CUE".to_owned()),
            value.lookup_path(&["upper"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::String("cue-rust".to_owned()),
            value.lookup_path(&["joined"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::String("a".to_owned()),
                EvaluatedValue::String("b".to_owned()),
            ]),
            value.lookup_path(&["split"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["contains"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["hasPrefix"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["hasSuffix"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("2".to_owned()),
            value.lookup_path(&["count"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("4".to_owned()),
            value.lookup_path(&["index"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::String("hahaha".to_owned()),
            value.lookup_path(&["stringRepeat"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::String("x".to_owned()),
                EvaluatedValue::String("x".to_owned()),
                EvaluatedValue::String("x".to_owned()),
            ]),
            value.lookup_path(&["repeated"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("1".to_owned()),
                EvaluatedValue::Number("2".to_owned()),
                EvaluatedValue::Number("3".to_owned()),
            ]),
            value.lookup_path(&["concatenated"])?.evaluate()?,
        );
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "single broad stdlib regression test keeps related string builtin parity \
                  assertions together"
    )]
    fn test_should_evaluate_broad_strings_standard_library_surface()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "strings.cue",
            "import \"strings\"\ncompare: strings.Compare(\"a\", \"b\")\ncontainsAny: \
             strings.ContainsAny(\"cue\", \"xzue\")\nlastIndex: strings.LastIndex(\"banana\", \
             \"na\")\nindexAny: strings.IndexAny(\"cue\", \"xzue\")\nlastIndexAny: \
             strings.LastIndexAny(\"cuecue\", \"eu\")\nsplitN: strings.SplitN(\"a,b,c\", \",\", \
             2)\nsplitAfter: strings.SplitAfter(\"a,b\", \",\")\nsplitAfterN: \
             strings.SplitAfterN(\"a,b,c\", \",\", 2)\nfields: strings.Fields(\"  a  b\\t c \
             \")\ntrim: strings.Trim(\"abba\", \"a\")\ntrimLeft: strings.TrimLeft(\"abba\", \
             \"ab\")\ntrimRight: strings.TrimRight(\"abba\", \"ab\")\ntrimPrefix: \
             strings.TrimPrefix(\"cue-rust\", \"cue-\")\ntrimSuffix: \
             strings.TrimSuffix(\"cue-rust\", \"-rust\")\nreplace: strings.Replace(\"banana\", \
             \"na\", \"NA\", 1)\nrunes: strings.Runes(\"Café\")\nminRunes: \
             strings.MinRunes(\"Café\", 4)\nmaxRunes: strings.MaxRunes(\"Café\", 4)\nsliceRunes: \
             strings.SliceRunes(\"✓ Hello\", 0, 3)\nbyteAt: strings.ByteAt(\"abc\", \
             1)\nbyteSlice: strings.ByteSlice(\"Hello\", 2, 5)\nsplitAfterTrailing: \
             strings.SplitAfter(\"a,\", \",\")\nsplitAfterNTrailing: strings.SplitAfterN(\"a,\", \
             \",\", 2)\ntoTitle: strings.ToTitle(\"alpha beta\")\ntoCamel: \
             strings.ToCamel(\"Alpha Beta\")\nvalidatorMin: strings.MinRunes(3) & \
             \"hello\"\nvalidatorMax: strings.MaxRunes(3) & \"foo\"\nvalidatorBad: \
             strings.MaxRunes(3) & \"quux\"\nvalidatorCombined: strings.MinRunes(2) & \
             strings.MaxRunes(5) & \"cue\"\nvalidatorBare: strings.MinRunes(3)\n",
        )?;

        assert_evaluated_path(&value, "compare", &EvaluatedValue::Number("-1".to_owned()))?;
        assert_evaluated_path(&value, "containsAny", &EvaluatedValue::Bool(true))?;
        assert_evaluated_path(&value, "lastIndex", &EvaluatedValue::Number("4".to_owned()))?;
        assert_evaluated_path(&value, "indexAny", &EvaluatedValue::Number("1".to_owned()))?;
        assert_evaluated_path(
            &value,
            "lastIndexAny",
            &EvaluatedValue::Number("5".to_owned()),
        )?;
        assert_evaluated_path(&value, "splitN", &string_list(&["a", "b,c"]))?;
        assert_evaluated_path(&value, "splitAfter", &string_list(&["a,", "b"]))?;
        assert_evaluated_path(&value, "splitAfterN", &string_list(&["a,", "b,c"]))?;
        assert_evaluated_path(&value, "fields", &string_list(&["a", "b", "c"]))?;
        assert_evaluated_path(&value, "trim", &EvaluatedValue::String("bb".to_owned()))?;
        assert_evaluated_path(&value, "trimLeft", &EvaluatedValue::String(String::new()))?;
        assert_evaluated_path(&value, "trimRight", &EvaluatedValue::String(String::new()))?;
        assert_evaluated_path(
            &value,
            "trimPrefix",
            &EvaluatedValue::String("rust".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "trimSuffix",
            &EvaluatedValue::String("cue".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "replace",
            &EvaluatedValue::String("baNAna".to_owned()),
        )?;
        assert_evaluated_path(&value, "runes", &number_list(&["67", "97", "102", "233"]))?;
        assert_evaluated_path(&value, "minRunes", &EvaluatedValue::Bool(true))?;
        assert_evaluated_path(&value, "maxRunes", &EvaluatedValue::Bool(true))?;
        assert_evaluated_path(
            &value,
            "sliceRunes",
            &EvaluatedValue::String("✓ H".to_owned()),
        )?;
        assert_evaluated_path(&value, "byteAt", &EvaluatedValue::Number("98".to_owned()))?;
        assert_evaluated_path(&value, "byteSlice", &EvaluatedValue::Bytes(b"llo".to_vec()))?;
        assert_evaluated_path(&value, "splitAfterTrailing", &string_list(&["a,", ""]))?;
        assert_evaluated_path(&value, "splitAfterNTrailing", &string_list(&["a,", ""]))?;
        assert_evaluated_path(
            &value,
            "toTitle",
            &EvaluatedValue::String("Alpha Beta".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "toCamel",
            &EvaluatedValue::String("alpha beta".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "validatorMin",
            &EvaluatedValue::String("hello".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "validatorMax",
            &EvaluatedValue::String("foo".to_owned()),
        )?;
        assert!(matches!(
            value.lookup_path(&["validatorBad"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert_evaluated_path(
            &value,
            "validatorCombined",
            &EvaluatedValue::String("cue".to_owned()),
        )?;
        assert_eq!(
            ValueKind::String,
            value.lookup_path(&["validatorBare"])?.kind()?
        );
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "single exact math regression keeps constants, functions, and validator-style \
                  constraints together"
    )]
    fn test_should_evaluate_exact_math_standard_library_surface()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "math.cue",
            r#"import "math"
e: math.E
pi: math.Pi
phi: math.Phi
sqrt2: math.Sqrt2
ln10: math.Ln10
maxExp: math.MaxExp
maxBase: math.MaxBase
floorPi: math.Floor(math.Pi)
floorNeg: math.Floor(-2.2)
ceilNeg: math.Ceil(-2.2)
truncNeg: math.Trunc(-2.9)
round: math.Round(2.5)
roundNeg: math.Round(-2.5)
even: math.RoundToEven(2.5)
evenNeg: math.RoundToEven(-2.5)
abs: math.Abs(-2.2)
acos: math.Acos(0.5)
acosh: math.Acosh(1)
asin: math.Asin(0.5)
asinOverflow: math.Asin(2.0e400)
asinh: math.Asinh(0)
atan: math.Atan(1)
atan2: math.Atan2(1, 1)
atanh: math.Atanh(0.5)
floatUnderflow: math.Sin(1e-400)
copySign: math.Copysign(5, -2.2)
copySignZero: math.Copysign(0, -1)
cbrt: math.Cbrt(2)
cbrtNegative: math.Cbrt(-8)
cbrtNegativeZero: math.Cbrt(-0)
cos: math.Cos(0)
cosh: math.Cosh(0)
dimPositive: math.Dim(3, 2.5)
dimZero: math.Dim(5, 7.2)
expm1: math.Expm1(1)
hypot: math.Hypot(3, 4)
jacobi: math.Jacobi(1000, 201)
jacobiEven: math.Jacobi(1000, 2000)
jacobiNegative: math.Jacobi(-1, 3)
jacobiZero: math.Jacobi(0, 3)
jacobiCommonFactor: math.Jacobi(3, 9)
jacobiNegativeDenominator: math.Jacobi(1, -3)
jacobiBig: math.Jacobi(1, 170141183460469231731687303715884105729)
multipleBool: math.MultipleOf(5, 2.5)
multipleConstraint: 9 & math.MultipleOf(3)
multiConstraint: 12 & math.MultipleOf(2) & math.MultipleOf(3)
boundedMultiple: math.MultipleOf(2) & >3 & <=6
boundedMultipleGood: boundedMultiple & 4
boundedMultipleLow: boundedMultiple & 2
boundedMultipleHigh: boundedMultiple & 8
multipleBad: 10 & math.MultipleOf(3)
multiBad: 10 & math.MultipleOf(2) & math.MultipleOf(3)
badZero: math.MultipleOf(5, 0)
bareConstraint: math.MultipleOf(3)
log1p: math.Log1p(1)
logb: math.Logb(8)
logbMax: math.Logb(1.7976931348623157e308)
logbSubnormal: math.Logb(5e-324)
mod: math.Mod(5.5, 2)
sign: math.Signbit(-4)
signZero: math.Signbit(-0)
sin: math.Sin(0)
sinh: math.Sinh(0)
sqrt: math.Sqrt(9)
tan: math.Tan(0)
tanh: math.Tanh(0)
pow10: math.Pow10(4)
pow10Neg: math.Pow10(-2)
pow: math.Pow(8, 4)
powDecimal: math.Pow(2.5, 2)
powNegative: math.Pow(-2, 3)
powNegativeEven: math.Pow(-2, 4)
powNegativeExponent: math.Pow(2, -3)
powNegativeDecimalExponent: math.Pow(1.25, -2)
powNegativeZero: math.Pow(-0, 3)
"#,
        )?;

        assert_evaluated_path(
            &value,
            "e",
            &EvaluatedValue::Number(
                "2.71828182845904523536028747135266249775724709369995957496696763".to_owned(),
            ),
        )?;
        assert_evaluated_path(
            &value,
            "pi",
            &EvaluatedValue::Number(
                "3.14159265358979323846264338327950288419716939937510582097494459".to_owned(),
            ),
        )?;
        assert_evaluated_path(
            &value,
            "phi",
            &EvaluatedValue::Number(
                "1.61803398874989484820458683436563811772030917980576286213544861".to_owned(),
            ),
        )?;
        assert_evaluated_path(
            &value,
            "sqrt2",
            &EvaluatedValue::Number(
                "1.41421356237309504880168872420969807856967187537694807317667974".to_owned(),
            ),
        )?;
        assert_evaluated_path(
            &value,
            "ln10",
            &EvaluatedValue::Number(
                "2.3025850929940456840179914546843642076011014886287729760333278".to_owned(),
            ),
        )?;
        assert_evaluated_path(
            &value,
            "maxExp",
            &EvaluatedValue::Number("2147483647".to_owned()),
        )?;
        assert_evaluated_path(&value, "maxBase", &EvaluatedValue::Number("62".to_owned()))?;
        assert_evaluated_path(&value, "floorPi", &EvaluatedValue::Number("3".to_owned()))?;
        assert_evaluated_path(&value, "floorNeg", &EvaluatedValue::Number("-3".to_owned()))?;
        assert_evaluated_path(&value, "ceilNeg", &EvaluatedValue::Number("-2".to_owned()))?;
        assert_evaluated_path(&value, "truncNeg", &EvaluatedValue::Number("-2".to_owned()))?;
        assert_evaluated_path(&value, "round", &EvaluatedValue::Number("3".to_owned()))?;
        assert_evaluated_path(&value, "roundNeg", &EvaluatedValue::Number("-3".to_owned()))?;
        assert_evaluated_path(&value, "even", &EvaluatedValue::Number("2".to_owned()))?;
        assert_evaluated_path(&value, "evenNeg", &EvaluatedValue::Number("-2".to_owned()))?;
        assert_evaluated_path(&value, "abs", &EvaluatedValue::Number("2.2".to_owned()))?;
        assert_evaluated_path(
            &value,
            "acos",
            &EvaluatedValue::Number("1.0471975511965979".to_owned()),
        )?;
        assert_evaluated_path(&value, "acosh", &EvaluatedValue::Number("0".to_owned()))?;
        assert_evaluated_path(
            &value,
            "asin",
            &EvaluatedValue::Number("0.5235987755982989".to_owned()),
        )?;
        assert!(matches!(
            value.lookup_path(&["asinOverflow"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert_evaluated_path(&value, "asinh", &EvaluatedValue::Number("0".to_owned()))?;
        assert_evaluated_path(
            &value,
            "atan",
            &EvaluatedValue::Number("0.7853981633974483".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "atan2",
            &EvaluatedValue::Number("0.7853981633974483".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "atanh",
            &EvaluatedValue::Number("0.5493061443340548".to_owned()),
        )?;
        assert!(matches!(
            value.lookup_path(&["floatUnderflow"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert_evaluated_path(&value, "copySign", &EvaluatedValue::Number("-5".to_owned()))?;
        assert_evaluated_path(
            &value,
            "copySignZero",
            &EvaluatedValue::Number("-0".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "cbrt",
            &EvaluatedValue::Number("1.259921049894873164767210607278228".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "cbrtNegative",
            &EvaluatedValue::Number("-2".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "cbrtNegativeZero",
            &EvaluatedValue::Number("-0".to_owned()),
        )?;
        assert_evaluated_path(&value, "cos", &EvaluatedValue::Number("1".to_owned()))?;
        assert_evaluated_path(&value, "cosh", &EvaluatedValue::Number("1".to_owned()))?;
        assert_evaluated_path(
            &value,
            "dimPositive",
            &EvaluatedValue::Number("0.5".to_owned()),
        )?;
        assert_evaluated_path(&value, "dimZero", &EvaluatedValue::Number("0".to_owned()))?;
        assert_evaluated_path(
            &value,
            "expm1",
            &EvaluatedValue::Number("1.718281828459045".to_owned()),
        )?;
        assert_evaluated_path(&value, "hypot", &EvaluatedValue::Number("5".to_owned()))?;
        assert_evaluated_path(&value, "jacobi", &EvaluatedValue::Number("1".to_owned()))?;
        assert!(matches!(
            value.lookup_path(&["jacobiEven"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert_evaluated_path(
            &value,
            "jacobiNegative",
            &EvaluatedValue::Number("-1".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "jacobiZero",
            &EvaluatedValue::Number("0".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "jacobiCommonFactor",
            &EvaluatedValue::Number("0".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "jacobiNegativeDenominator",
            &EvaluatedValue::Number("1".to_owned()),
        )?;
        assert_evaluated_path(&value, "jacobiBig", &EvaluatedValue::Number("1".to_owned()))?;
        assert_evaluated_path(&value, "multipleBool", &EvaluatedValue::Bool(true))?;
        assert_evaluated_path(
            &value,
            "multipleConstraint",
            &EvaluatedValue::Number("9".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "multiConstraint",
            &EvaluatedValue::Number("12".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "boundedMultipleGood",
            &EvaluatedValue::Number("4".to_owned()),
        )?;
        assert!(matches!(
            value.lookup_path(&["boundedMultipleLow"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert!(matches!(
            value.lookup_path(&["boundedMultipleHigh"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert!(matches!(
            value.lookup_path(&["multipleBad"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert!(matches!(
            value.lookup_path(&["multiBad"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert!(matches!(
            value.lookup_path(&["badZero"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        assert_eq!(
            ValueKind::Number,
            value.lookup_path(&["bareConstraint"])?.kind()?,
        );
        assert_evaluated_path(
            &value,
            "log1p",
            &EvaluatedValue::Number("0.6931471805599453".to_owned()),
        )?;
        assert_evaluated_path(&value, "logb", &EvaluatedValue::Number("3".to_owned()))?;
        assert_evaluated_path(
            &value,
            "logbMax",
            &EvaluatedValue::Number("1023".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "logbSubnormal",
            &EvaluatedValue::Number("-1074".to_owned()),
        )?;
        assert_evaluated_path(&value, "mod", &EvaluatedValue::Number("1.5".to_owned()))?;
        assert_evaluated_path(&value, "sign", &EvaluatedValue::Bool(true))?;
        assert_evaluated_path(&value, "signZero", &EvaluatedValue::Bool(true))?;
        assert_evaluated_path(&value, "sin", &EvaluatedValue::Number("0".to_owned()))?;
        assert_evaluated_path(&value, "sinh", &EvaluatedValue::Number("0".to_owned()))?;
        assert_evaluated_path(&value, "sqrt", &EvaluatedValue::Number("3".to_owned()))?;
        assert_evaluated_path(&value, "tan", &EvaluatedValue::Number("0".to_owned()))?;
        assert_evaluated_path(&value, "tanh", &EvaluatedValue::Number("0".to_owned()))?;
        assert_evaluated_path(&value, "pow10", &EvaluatedValue::Number("10000".to_owned()))?;
        assert_evaluated_path(
            &value,
            "pow10Neg",
            &EvaluatedValue::Number("0.01".to_owned()),
        )?;
        assert_evaluated_path(&value, "pow", &EvaluatedValue::Number("4096".to_owned()))?;
        assert_evaluated_path(
            &value,
            "powDecimal",
            &EvaluatedValue::Number("6.25".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "powNegative",
            &EvaluatedValue::Number("-8".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "powNegativeEven",
            &EvaluatedValue::Number("16".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "powNegativeExponent",
            &EvaluatedValue::Number("0.125".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "powNegativeDecimalExponent",
            &EvaluatedValue::Number("0.64".to_owned()),
        )?;
        assert_evaluated_path(
            &value,
            "powNegativeZero",
            &EvaluatedValue::Number("-0".to_owned()),
        )?;

        let mut cue_options = EncodeOptions::default();
        cue_options.encoding = Encoding::Cue;
        cue_options.concrete = false;
        assert_eq!(
            ">3 & <=6 & math.MultipleOf(2)",
            encode_value(&value.lookup_path(&["boundedMultiple"])?, cue_options)?,
        );
        for encoding in [Encoding::Json, Encoding::Yaml, Encoding::Toml] {
            cue_options.encoding = encoding;
            assert!(encode_value(&value.lookup_path(&["boundedMultiple"])?, cue_options).is_err(),);
        }
        Ok(())
    }

    #[test]
    fn test_should_evaluate_broad_list_standard_library_surface()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "list.cue",
            "import \"list\"\ndrop: list.Drop([1, 2, 3, 4], 2)\ntake: list.Take([1, 2, 3, 4], \
             2)\nslice: list.Slice([1, 2, 3, 4], 1, 3)\nreverse: list.Reverse([1, 2, \
             3])\nflatten: list.FlattenN([1, [[2, 3], []], [4]], 2)\nuniqueGood: \
             list.UniqueItems([1, 2, 3])\nuniqueBad: list.UniqueItems([1, 2, 1])\nsorted: \
             list.SortStrings([\"b\", \"a\"])\nisSorted: list.IsSortedStrings([\"a\", \
             \"b\"])\nminItems: list.MinItems([1, 2], 2)\nmaxItems: list.MaxItems([1, 2], \
             3)\nsum: list.Sum([1, 2, 3])\nproduct: list.Product([2, 3, 4])\nmin: list.Min([3, 1, \
             2])\nmax: list.Max([3, 1, 2])\navg: list.Avg([4, 8, 12])\nrange: list.Range(0, 5, \
             2)\ndecimalSum: list.Sum([0.1, 0.2])\ndecimalRange: list.Range(0, 0.3, \
             0.1)\nhugeDecimal: list.Sum([1e20000000])\n",
        )?;

        assert_number_list(&value.lookup_path(&["drop"])?.evaluate()?, &["3", "4"]);
        assert_number_list(&value.lookup_path(&["take"])?.evaluate()?, &["1", "2"]);
        assert_number_list(&value.lookup_path(&["slice"])?.evaluate()?, &["2", "3"]);
        assert_number_list(
            &value.lookup_path(&["reverse"])?.evaluate()?,
            &["3", "2", "1"],
        );
        assert_number_list(
            &value.lookup_path(&["flatten"])?.evaluate()?,
            &["1", "2", "3", "4"],
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["uniqueGood"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(false),
            value.lookup_path(&["uniqueBad"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::String("a".to_owned()),
                EvaluatedValue::String("b".to_owned()),
            ]),
            value.lookup_path(&["sorted"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["isSorted"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["minItems"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["maxItems"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("6".to_owned()),
            value.lookup_path(&["sum"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("24".to_owned()),
            value.lookup_path(&["product"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("1".to_owned()),
            value.lookup_path(&["min"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["max"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("8".to_owned()),
            value.lookup_path(&["avg"])?.evaluate()?,
        );
        assert_number_list(
            &value.lookup_path(&["range"])?.evaluate()?,
            &["0", "2", "4"],
        );
        assert_eq!(
            EvaluatedValue::Number("0.3".to_owned()),
            value.lookup_path(&["decimalSum"])?.evaluate()?,
        );
        assert_number_list(
            &value.lookup_path(&["decimalRange"])?.evaluate()?,
            &["0", "0.1", "0.2"],
        );
        assert!(matches!(
            value.lookup_path(&["hugeDecimal"])?.evaluate()?,
            EvaluatedValue::Bottom(_),
        ));
        Ok(())
    }

    #[test]
    fn test_should_evaluate_open_list_ellipsis_constraints()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "open-list.cue",
            "ok: [1, 2, ...>=4 & <=5] & [1, 2, 4, 5]\nany: [1, ...] & [1, \"a\", true]\nbad: [1, \
             2, ...>=4 & <=5] & [1, 2, 4, 8]\ntail: [1, ...int][3]\nschema: [1, ...int] & [_, 2, \
             ...number]\n",
        )?;

        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("1".to_owned()),
                EvaluatedValue::Number("2".to_owned()),
                EvaluatedValue::Number("4".to_owned()),
                EvaluatedValue::Number("5".to_owned()),
            ]),
            value.lookup_path(&["ok"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::List(vec![
                EvaluatedValue::Number("1".to_owned()),
                EvaluatedValue::String("a".to_owned()),
                EvaluatedValue::Bool(true),
            ]),
            value.lookup_path(&["any"])?.evaluate()?,
        );
        assert!(
            value
                .lookup_path(&["bad"])?
                .validate(ValidateOptions::default())
                .is_err()
        );
        assert_eq!(ValueKind::Int, value.lookup_path(&["tail"])?.kind()?);
        assert_eq!(
            EvaluatedValue::OpenList {
                items: vec![
                    EvaluatedValue::Number("1".to_owned()),
                    EvaluatedValue::Number("2".to_owned()),
                ],
                tail: Box::new(EvaluatedValue::Kind(ValueKind::Int)),
            },
            value.lookup_path(&["schema"])?.evaluate()?,
        );

        let mut cue_options = EncodeOptions::default();
        cue_options.encoding = Encoding::Cue;
        cue_options.concrete = false;
        assert_eq!(
            "[1, 2, ...int]",
            encode_value(&value.lookup_path(&["schema"])?, cue_options)?,
        );
        for encoding in [Encoding::Json, Encoding::Yaml, Encoding::Toml] {
            cue_options.encoding = encoding;
            assert!(encode_value(&value.lookup_path(&["schema"])?, cue_options).is_err());
        }

        let invalid_schema = context.compile_source(
            "open-list-invalid.cue",
            "schema: [1, ...int] & [_, \"x\", ...int]\n",
        )?;
        let result = invalid_schema
            .lookup_path(&["schema"])?
            .validate(ValidateOptions::default());
        let Err(EvalError::Diagnostics(report)) = result else {
            return Err("expected open list validation diagnostics".into());
        };
        let first = report
            .diagnostics()
            .first()
            .ok_or("expected at least one diagnostic")?;
        assert_eq!("cue.eval.bottom", first.code());
        assert!(first.message().contains("$[1]"));
        assert!(
            first
                .message()
                .contains("conflicting values int and string")
        );
        Ok(())
    }

    #[test]
    fn test_should_resolve_field_alias_labels() -> Result<(), Box<dyn std::error::Error>> {
        let context = Context::new();
        let value = context.compile_source(
            "aliases.cue",
            "t0: {\n  a=_a: _\n  let _b = a\n  _out: _b\n}\nt1: {\n  _a: b\n  let b = c\n  c=d: \
             3\n}\n",
        )?;
        assert_eq!(ValueKind::Top, value.lookup_path(&["t0", "_out"])?.kind()?);
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["t1", "_a"])?.evaluate()?,
        );
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(&["t1", "d"])?.evaluate()?,
        );
        assert!(value.lookup_path(&["t1", "c"]).is_err());

        let mut cue_options = EncodeOptions::default();
        cue_options.encoding = Encoding::Cue;
        cue_options.concrete = false;
        let output = encode_value(&value.lookup_path(&["t1"])?, cue_options)?;
        assert!(output.contains("d: 3"));
        assert!(!output.contains("c:"));
        Ok(())
    }

    #[test]
    fn test_should_report_unsupported_import_as_compile_diagnostic() {
        let context = Context::new();
        let result =
            context.compile_source("test.cue", "import \"example.com/remote/pkg\"\nx: 1\n");
        assert!(matches!(result, Err(CueError::Diagnostics(_))));
    }
}
