//! Integration coverage borrowed from the vendored upstream CUE corpus.

use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use cue_rust::{
    Context, DecodeOptions, EncodeOptions, Encoding, EvalError, EvaluatedValue, ValidateOptions,
    ValueKind, decode_bytes, encode_value,
};
use tokio::fs;

const MIN_CLI_SCRIPT_CASES: usize = 390;
const MIN_CORE_TXTAR_CASES: usize = 470;
const MIN_E2E_SCRIPT_CASES: usize = 3;

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Debug)]
struct TxtarArchive {
    files: BTreeMap<String, String>,
}

impl TxtarArchive {
    async fn read(path: &Path) -> Result<Self, Box<dyn Error>> {
        let content = fs::read_to_string(path).await?;
        let mut files = BTreeMap::new();
        let mut current_name: Option<String> = None;
        let mut current_content = String::new();

        for line in content.lines() {
            if let Some(name) = txtar_header(line) {
                if let Some(previous_name) = current_name.replace(name.to_owned()) {
                    files.insert(previous_name, current_content);
                    current_content = String::new();
                }
            } else if current_name.is_some() {
                current_content.push_str(line);
                current_content.push('\n');
            }
        }

        if let Some(previous_name) = current_name {
            files.insert(previous_name, current_content);
        }

        Ok(Self { files })
    }

    fn file(&self, name: &str) -> Result<&str, Box<dyn Error>> {
        self.files
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("txtar file `{name}` is missing").into())
    }
}

#[tokio::test]
async fn test_should_inventory_all_upstream_vendor_integration_corpus() -> TestResult {
    let root = workspace_root();
    let cli_scripts =
        collect_txtar_files(&root.join("vendors/cue/cmd/cue/cmd/testdata/script")).await?;
    let core_cases = collect_txtar_files(&root.join("vendors/cue/cue/testdata")).await?;
    let e2e_scripts =
        collect_txtar_files(&root.join("vendors/cue/internal/_e2e/testdata/script")).await?;

    assert!(
        cli_scripts.len() >= MIN_CLI_SCRIPT_CASES,
        "expected at least {MIN_CLI_SCRIPT_CASES} upstream CLI script cases, got {}",
        cli_scripts.len(),
    );
    assert!(
        core_cases.len() >= MIN_CORE_TXTAR_CASES,
        "expected at least {MIN_CORE_TXTAR_CASES} upstream core txtar cases, got {}",
        core_cases.len(),
    );
    assert!(
        e2e_scripts.len() >= MIN_E2E_SCRIPT_CASES,
        "expected at least {MIN_E2E_SCRIPT_CASES} upstream e2e script cases, got {}",
        e2e_scripts.len(),
    );
    assert!(
        cli_scripts
            .iter()
            .any(|path| path.ends_with("export_cue.txtar"))
    );
    assert!(
        cli_scripts
            .iter()
            .any(|path| path.ends_with("encoding_toml.txtar"))
    );
    assert!(
        core_cases
            .iter()
            .any(|path| path.ends_with("basicrewrite/013_obj_unify.txtar"))
    );
    Ok(())
}

#[tokio::test]
async fn test_should_run_supported_upstream_cli_script_fixtures() -> TestResult {
    let root = workspace_root();
    let context = Context::new();

    let export_cue =
        TxtarArchive::read(&root.join("vendors/cue/cmd/cue/cmd/testdata/script/export_cue.txtar"))
            .await?;
    for source_name in ["nopkg.cue", "pkg.cue"] {
        let value = context.compile_source(source_name, export_cue.file(source_name)?)?;
        assert_eq!(ValueKind::Number, value.lookup_path(&["x"])?.kind()?);
        assert_eq!(ValueKind::Number, value.lookup_path(&["y"])?.kind()?);
    }

    let encoding_toml = TxtarArchive::read(
        &root.join("vendors/cue/cmd/cue/cmd/testdata/script/encoding_toml.txtar"),
    )
    .await?;
    let decoded_toml = decode_bytes(
        Encoding::Toml,
        encoding_toml.file("export.toml")?.as_bytes(),
        DecodeOptions::default(),
    )?;
    assert_eq!(
        EvaluatedValue::String("two levels".to_owned()),
        decoded_toml
            .lookup_path(&["nested", "a2", "b"])?
            .evaluate()?,
    );

    let json_errors = TxtarArchive::read(
        &root.join("vendors/cue/cmd/cue/cmd/testdata/script/json_syntax_error.txtar"),
    )
    .await?;
    for source_name in ["x1.json", "x2.json", "x3.json", "x4.json", "x5.jsonl"] {
        let result = decode_bytes(
            Encoding::Json,
            json_errors.file(source_name)?.as_bytes(),
            DecodeOptions::default(),
        );
        assert!(
            result.is_err(),
            "expected upstream invalid JSON fixture `{source_name}` to fail",
        );
    }

    let eval_resolve = TxtarArchive::read(
        &root.join("vendors/cue/cmd/cue/cmd/testdata/script/eval_resolve.txtar"),
    )
    .await?;
    let package_value = context.compile_source("package.cue", eval_resolve.file("package.cue")?)?;
    assert_eq!(
        ValueKind::List,
        package_value.lookup_path(&["nodes"])?.kind()?
    );
    Ok(())
}

#[tokio::test]
async fn test_should_run_supported_upstream_core_eval_fixtures() -> TestResult {
    let root = workspace_root();
    let context = Context::new();
    assert_upstream_references(&context, &root).await?;
    assert_upstream_regex(&context, &root).await?;
    assert_upstream_disjunctions_and_defaults(&context, &root).await?;
    assert_upstream_aggregate_builtins(&context, &root).await?;
    assert_upstream_stdlib_surface_builtins(&context, &root).await?;
    assert_upstream_object_unification(&context, &root).await?;
    assert_upstream_list_index_and_slice(&context, &root).await?;
    assert_upstream_selecting(&context, &root).await?;
    assert_upstream_basic_types(&context, &root).await?;
    assert_upstream_types(&context, &root).await?;
    assert_upstream_numeric_comparisons(&context, &root).await?;
    assert_upstream_list_comparisons(&context, &root).await?;
    assert_upstream_struct_comparisons(&context, &root).await?;
    assert_upstream_arithmetic(&context, &root).await?;
    assert_upstream_integer_arithmetic(&context, &root).await?;
    assert_upstream_incomplete_operand_errors(&context, &root).await?;
    assert_upstream_booleans(&context, &root).await?;
    assert_upstream_null(&context, &root).await?;
    assert_upstream_len(&context, &root).await?;
    assert_upstream_strings_and_bytes(&context, &root).await?;
    assert_upstream_escaping(&context, &root).await?;
    assert_upstream_definitions_and_export_profiles(&context, &root).await?;
    assert_upstream_alias_labels(&context, &root).await?;
    assert_upstream_phase9_parity_tranche(&context, &root).await?;
    Ok(())
}

async fn assert_upstream_definitions_and_export_profiles(
    context: &Context,
    root: &Path,
) -> TestResult {
    let definitions =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/definitions/files.txtar")).await?;
    let value = context.compile_source("definitions/files/in.cue", definitions.file("in.cue")?)?;
    assert_eq!(
        EvaluatedValue::String("dark".to_owned()),
        value.lookup_path(&["dark", "color"])?.evaluate()?,
    );
    let output = encode_value(&value, EncodeOptions::default())?;
    assert!(output.contains("\"dark\""));
    assert!(output.contains("\"light\""));
    assert!(!output.contains("#theme"));
    assert!(!output.contains("#Config"));

    let comparison =
        TxtarArchive::read(&root.join(
            "vendors/cue/internal/core/compile/testdata/sync/basicrewrite/016_comparison.txtar",
        ))
        .await?;
    let source = comparison.file("structs.cue")?;
    assert!(source.contains("b?: 2"));
    assert!(source.contains("_hidden: 1"));
    assert!(source.contains("#def: 1"));

    let reduced = context.compile_source(
        "basicrewrite/016_comparison/reduced.cue",
        "visible: 1\n_hidden: 1\n#def: 1\noptional?: string\n",
    )?;
    let reduced_json = encode_value(&reduced, EncodeOptions::default())?;
    assert!(reduced_json.contains("\"visible\": 1"));
    assert!(!reduced_json.contains("_hidden"));
    assert!(!reduced_json.contains("#def"));
    assert!(!reduced_json.contains("optional"));

    let optional_reference = context.compile_source(
        "basicrewrite/016_comparison/optional_reference.cue",
        "x?: 1\ny: x\n",
    )?;
    assert!(encode_value(&optional_reference, EncodeOptions::default()).is_err());
    Ok(())
}

async fn assert_upstream_alias_labels(context: &Context, root: &Path) -> TestResult {
    let aliases = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/aliases/aliases.txtar"),
    )
    .await?;
    let upstream_source = aliases.file("in.cue")?;
    assert!(upstream_source.contains("a=_a: _"));
    assert!(upstream_source.contains("c=d: 3"));

    let value = context.compile_source(
        "basicrewrite/aliases/in.cue",
        "t0: {\n  a=_a: _\n  let _b = a\n  _out: _b\n}\nt1: {\n  _a: b\n  let b = c\n  c=d: 3\n}\n",
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
    Ok(())
}

async fn assert_upstream_object_unification(context: &Context, root: &Path) -> TestResult {
    let object_unify =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/013_obj_unify.txtar"))
            .await?;
    let value = context.compile_source(
        "basicrewrite/013_obj_unify/in.cue",
        object_unify.file("in.cue")?,
    )?;

    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        value.lookup_path(&["o1", "a"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        value.lookup_path(&["o1", "b"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        value.lookup_path(&["o4", "b"])?.evaluate()?,
    );
    assert!(
        value
            .lookup_path(&["e"])?
            .validate(ValidateOptions::default())
            .is_err()
    );
    Ok(())
}

async fn assert_upstream_references(context: &Context, root: &Path) -> TestResult {
    let references =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/009_reference.txtar"))
            .await?;
    let value = context.compile_source(
        "basicrewrite/009_reference/in.cue",
        references.file("in.cue")?,
    )?;
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        value.lookup_path(&["a"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("3".to_owned()),
        value.lookup_path(&["d", "e"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        value.lookup_path(&["e", "f", "v"])?.evaluate()?,
    );
    Ok(())
}

async fn assert_upstream_regex(context: &Context, root: &Path) -> TestResult {
    let regexp =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/001_regexp.txtar"))
            .await?;
    let regex_source = selected_lines(
        regexp.file("in.cue")?,
        &["c1:", "c2:", "c3:", "c4:", "b1:", "b2:", "b3:", "b4:"],
    );
    let value = context.compile_source("basicrewrite/001_regexp/in.cue", regex_source)?;
    for (path, expected) in [("c1", true), ("c2", true), ("c3", false), ("c4", true)] {
        assert_eq!(
            EvaluatedValue::Bool(expected),
            value.lookup_path(&[path])?.evaluate()?,
        );
    }
    assert_eq!(
        EvaluatedValue::String("a".to_owned()),
        value.lookup_path(&["b1"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("foo".to_owned()),
        value.lookup_path(&["b2"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("foo".to_owned()),
        value.lookup_path(&["b4"])?.evaluate()?,
    );
    assert!(
        value
            .lookup_path(&["b3"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    Ok(())
}

async fn assert_upstream_disjunctions_and_defaults(context: &Context, root: &Path) -> TestResult {
    let disjunctions = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/014_disjunctions.txtar"),
    )
    .await?;
    let value = context.compile_source(
        "basicrewrite/014_disjunctions/in.cue",
        disjunctions.file("in.cue")?,
    )?;
    for (path, expected) in [("o2", "1"), ("o3", "2"), ("i1", "\"c\"")] {
        let evaluated = value.lookup_path(&[path])?.evaluate()?.resolve_defaults();
        match expected {
            "\"c\"" => assert_eq!(EvaluatedValue::String("c".to_owned()), evaluated),
            number => assert_eq!(EvaluatedValue::Number(number.to_owned()), evaluated),
        }
    }
    assert_eq!(ValueKind::Number, value.lookup_path(&["o7"])?.kind()?);
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        value.lookup_path(&["m1"])?.evaluate()?.resolve_defaults(),
    );

    let default_builtins =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/builtins/default.txtar")).await?;
    let default_value = context.compile_source(
        "builtins/default/in.cue",
        selected_lines(
            default_builtins.file("in.cue")?,
            &[
                "Len:", "Close:", "And:", "Or:", "Div:", "Mod:", "Quo:", "Rem:",
            ],
        ),
    )?;
    for (path, expected) in [
        ("Len", "3"),
        ("And", "1"),
        ("Or", "1"),
        ("Div", "2"),
        ("Mod", "1"),
        ("Quo", "2"),
        ("Rem", "1"),
    ] {
        assert_eq!(
            EvaluatedValue::Number(expected.to_owned()),
            default_value.lookup_path(&[path])?.evaluate()?,
        );
    }
    let EvaluatedValue::ClosedStruct(fields) = default_value.lookup_path(&["Close"])?.evaluate()?
    else {
        return Err("expected close default to resolve to an empty struct".into());
    };
    assert!(fields.is_empty());

    assert_upstream_default_operand_parity(context, root).await?;
    Ok(())
}

async fn assert_upstream_default_operand_parity(context: &Context, root: &Path) -> TestResult {
    let fixture =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/disjunctions/operands.txtar"))
            .await?;
    let source = fixture.file("in.cue")?;
    for snippet in [
        "forLoop:",
        "if condition",
        "if num < 5",
        "a: object.a",
        "a: list[0]",
        "a: num + 4",
        "a: -num",
    ] {
        assert!(
            source.contains(snippet),
            "upstream operand fixture no longer contains `{snippet}`",
        );
    }
    let value = context.compile_source(
        "disjunctions/operands/reduced.cue",
        "list: *[1] | [2]\ncondition: *true | false\nnum: *1 | 2\nobject: *{a: 1} | {a: \
         2}\nforLoop: [for e in list {\"count: \\(e)\"}]\nconditional: {if condition {a: 3}, if \
         num < 5 {b: 3}}\nselector: {a: object.a}\nindex: {a: list[0]}\nbinOp: {a: num + \
         4}\nunaryOp: {a: -num}\n",
    )?;
    assert_eq!(
        EvaluatedValue::List(vec![EvaluatedValue::String("count: 1".to_owned())]),
        value.lookup_path(&["forLoop"])?.evaluate()?,
    );
    for path in [&["conditional", "a"][..], &["conditional", "b"][..]] {
        assert_eq!(
            EvaluatedValue::Number("3".to_owned()),
            value.lookup_path(path)?.evaluate()?,
            "unexpected default operand value at {path:?}",
        );
    }
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        value.lookup_path(&["selector", "a"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        value.lookup_path(&["index", "a"])?.evaluate()?,
    );
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

async fn assert_upstream_aggregate_builtins(context: &Context, root: &Path) -> TestResult {
    let and_fixture =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/builtins/and.txtar")).await?;
    let and_value = context.compile_source(
        "builtins/and/in.cue",
        selected_lines(and_fixture.file("in.cue")?, &["merge:"]),
    )?;
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        and_value.lookup_path(&["merge"])?.evaluate()?,
    );

    let or_fixture =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/builtins/or.txtar")).await?;
    let or_value = context.compile_source(
        "builtins/or/in.cue",
        selected_lines(
            or_fixture.file("in.cue")?,
            &["unwrap:", "unique1:", "unique2:"],
        ),
    )?;
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        or_value.lookup_path(&["unwrap"])?.evaluate()?,
    );
    assert_eq!(
        ValueKind::Number,
        or_value.lookup_path(&["unique1"])?.kind()?
    );
    assert_eq!(
        ValueKind::Struct,
        or_value.lookup_path(&["unique2"])?.kind()?
    );

    let strings_arity =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/builtins/issue_3567.txtar"))
            .await?;
    assert!(strings_arity.file("in.cue")?.contains("strings.Join"));
    let invalid_join = context.compile_source(
        "builtins/issue_3567/reduced.cue",
        "import \"strings\"\na: strings.Join([\"1\"])\n",
    )?;
    assert!(
        invalid_join
            .lookup_path(&["a"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );

    let cmd_print =
        TxtarArchive::read(&root.join("vendors/cue/cmd/cue/cmd/testdata/script/cmd_print.txtar"))
            .await?;
    assert!(cmd_print.file("task_tool.cue")?.contains("strings.Join"));
    let strings_value = context.compile_source(
        "cmd/cmd_print/reduced.cue",
        "import \"strings\"\nresult: strings.Join(strings.Split(\"abc\", \"\"), \".\")\nupper: \
         strings.ToUpper(\"cue\")\ntrimmed: strings.TrimSpace(\" cue \")\n",
    )?;
    assert_eq!(
        EvaluatedValue::String("a.b.c".to_owned()),
        strings_value.lookup_path(&["result"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("CUE".to_owned()),
        strings_value.lookup_path(&["upper"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("cue".to_owned()),
        strings_value.lookup_path(&["trimmed"])?.evaluate()?,
    );

    let export_force = TxtarArchive::read(
        &root.join("vendors/cue/cmd/cue/cmd/testdata/script/export_force.txtar"),
    )
    .await?;
    assert!(export_force.file("slow.cue")?.contains("list.Repeat"));
    let list_value = context.compile_source(
        "cmd/export_force/reduced.cue",
        "import \"list\"\nrepeated: list.Repeat([\"x\"], 3)\nconcatenated: list.Concat([[\"a\"], \
         [\"b\", \"c\"]])\ncontains: list.Contains([\"a\", \"b\"], \"b\")\n",
    )?;
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::String("x".to_owned()),
            EvaluatedValue::String("x".to_owned()),
            EvaluatedValue::String("x".to_owned()),
        ]),
        list_value.lookup_path(&["repeated"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::String("a".to_owned()),
            EvaluatedValue::String("b".to_owned()),
            EvaluatedValue::String("c".to_owned()),
        ]),
        list_value.lookup_path(&["concatenated"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Bool(true),
        list_value.lookup_path(&["contains"])?.evaluate()?,
    );
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "vendor stdlib fixture assertions intentionally stay adjacent to the borrowed \
              upstream archive checks"
)]
async fn assert_upstream_stdlib_surface_builtins(context: &Context, root: &Path) -> TestResult {
    let strings_gen =
        TxtarArchive::read(&root.join("vendors/cue/pkg/strings/testdata/gen.txtar")).await?;
    let strings_source = strings_gen.file("in.cue")?;
    assert!(strings_source.contains("strings.ByteAt"));
    assert!(strings_source.contains("strings.ByteSlice"));
    assert!(strings_source.contains("strings.SliceRunes"));
    assert!(strings_source.contains("strings.MaxRunes(3) & \"foo\""));

    let strings_value = context.compile_source(
        "pkg/strings/gen/reduced.cue",
        "import \"strings\"\nbyteAt: strings.ByteAt(\"a\", 0)\nsliceRunes: strings.SliceRunes(\"✓ \
         Hello\", 0, 3)\nbyteSlice: strings.ByteSlice(\"Hello\", 2, 5)\nrunes: \
         strings.Runes(\"Café\")\ntrimPrefix: strings.TrimPrefix(\"cue-rust\", \
         \"cue-\")\nreplace: strings.Replace(\"banana\", \"na\", \"NA\", 1)\nvalidator: \
         strings.MaxRunes(3) & \"foo\"\nvalidatorRegex: strings.MinRunes(2) & =~\"^fo\" & \
         \"foo\"\nvalidatorMultiRegex: =~\"^f\" & =~\"o$\" & strings.MinRunes(3) & \
         \"foo\"\nvalidatorBad: strings.MinRunes(10) & \"hello\"\nvalidatorRegexBad: \
         strings.MaxRunes(3) & =~\"^ba\" & \"foo\"\n",
    )?;
    assert_eq!(
        EvaluatedValue::Number("97".to_owned()),
        strings_value.lookup_path(&["byteAt"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("✓ H".to_owned()),
        strings_value.lookup_path(&["sliceRunes"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Bytes(b"llo".to_vec()),
        strings_value.lookup_path(&["byteSlice"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("67".to_owned()),
            EvaluatedValue::Number("97".to_owned()),
            EvaluatedValue::Number("102".to_owned()),
            EvaluatedValue::Number("233".to_owned()),
        ]),
        strings_value.lookup_path(&["runes"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("rust".to_owned()),
        strings_value.lookup_path(&["trimPrefix"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("baNAna".to_owned()),
        strings_value.lookup_path(&["replace"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("foo".to_owned()),
        strings_value.lookup_path(&["validator"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("foo".to_owned()),
        strings_value.lookup_path(&["validatorRegex"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("foo".to_owned()),
        strings_value
            .lookup_path(&["validatorMultiRegex"])?
            .evaluate()?,
    );
    assert!(
        strings_value
            .lookup_path(&["validatorBad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    assert!(
        strings_value
            .lookup_path(&["validatorRegexBad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );

    let list_gen =
        TxtarArchive::read(&root.join("vendors/cue/pkg/list/testdata/gen.txtar")).await?;
    let list_source = list_gen.file("in.cue")?;
    assert!(list_source.contains("list.FlattenN"));
    assert!(list_source.contains("list.SortStrings"));
    assert!(list_source.contains("list.Sort([2, 3, 1, 4]"));

    let list_value = context.compile_source(
        "pkg/list/gen/reduced.cue",
        "import \"list\"\nflatten: list.FlattenN([1, [[2, 3], []], [4]], 2)\nrange: list.Range(0, \
         5, 2)\nsorted: list.SortStrings([\"b\", \"a\"])\nsortNumbers: list.Sort([2, 3, 1, 4], \
         list.Ascending)\ndir: list.Ascending\nsortAlias: list.Sort([2, 1], dir)\ncmp: {x: _, y: \
         _, less: x.a < y.a}\nsortCustom: list.Sort([{a: 2}, {a: 1}], {x: _, y: _, less: x.a < \
         y.a})\nsortNamed: list.Sort([{a: 2}, {a: 1}], cmp)\nsortStableDup: list.Sort([{a: 1, i: \
         1}, {a: 1, i: 2}], cmp)\nsortDesc: list.SortStable([\"a\", \"c\", \"b\"], \
         list.Descending)\nisSorted: list.IsSorted([1, 2, 3], list.Ascending)\nisSortedAlias: \
         list.IsSorted([1, 2, 3], dir)\ninvalidSort: list.Sort([2, 1], {x: _, y: _, less: \
         \"bad\"})\nsum: list.Sum([1, 2, 3, 4])\navg: list.Avg([4, 8, 12])\n",
    )?;
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("1".to_owned()),
            EvaluatedValue::Number("2".to_owned()),
            EvaluatedValue::Number("3".to_owned()),
            EvaluatedValue::Number("4".to_owned()),
        ]),
        list_value.lookup_path(&["flatten"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("0".to_owned()),
            EvaluatedValue::Number("2".to_owned()),
            EvaluatedValue::Number("4".to_owned()),
        ]),
        list_value.lookup_path(&["range"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::String("a".to_owned()),
            EvaluatedValue::String("b".to_owned()),
        ]),
        list_value.lookup_path(&["sorted"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("1".to_owned()),
            EvaluatedValue::Number("2".to_owned()),
            EvaluatedValue::Number("3".to_owned()),
            EvaluatedValue::Number("4".to_owned()),
        ]),
        list_value.lookup_path(&["sortNumbers"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("1".to_owned()),
            EvaluatedValue::Number("2".to_owned()),
        ]),
        list_value.lookup_path(&["sortAlias"])?.evaluate()?,
    );
    let EvaluatedValue::List(sort_custom) = list_value.lookup_path(&["sortCustom"])?.evaluate()?
    else {
        return Err("expected custom sort list".into());
    };
    let Some(EvaluatedValue::Struct(first)) = sort_custom.first() else {
        return Err("expected first custom sort item".into());
    };
    assert_eq!(
        Some(&EvaluatedValue::Number("1".to_owned())),
        first.get("a"),
    );
    let EvaluatedValue::List(sort_named) = list_value.lookup_path(&["sortNamed"])?.evaluate()?
    else {
        return Err("expected named custom sort list".into());
    };
    let Some(EvaluatedValue::Struct(first_named)) = sort_named.first() else {
        return Err("expected first named custom sort item".into());
    };
    assert_eq!(
        Some(&EvaluatedValue::Number("1".to_owned())),
        first_named.get("a"),
    );
    let EvaluatedValue::List(sort_stable_dup) =
        list_value.lookup_path(&["sortStableDup"])?.evaluate()?
    else {
        return Err("expected stable duplicate-key sort list".into());
    };
    let Some(EvaluatedValue::Struct(first_dup)) = sort_stable_dup.first() else {
        return Err("expected first stable duplicate-key sort item".into());
    };
    let Some(EvaluatedValue::Struct(second_dup)) = sort_stable_dup.get(1) else {
        return Err("expected second stable duplicate-key sort item".into());
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
        EvaluatedValue::List(vec![
            EvaluatedValue::String("c".to_owned()),
            EvaluatedValue::String("b".to_owned()),
            EvaluatedValue::String("a".to_owned()),
        ]),
        list_value.lookup_path(&["sortDesc"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Bool(true),
        list_value.lookup_path(&["isSorted"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Bool(true),
        list_value.lookup_path(&["isSortedAlias"])?.evaluate()?,
    );
    assert!(
        list_value
            .lookup_path(&["invalidSort"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    assert_eq!(
        EvaluatedValue::Number("10".to_owned()),
        list_value.lookup_path(&["sum"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("8".to_owned()),
        list_value.lookup_path(&["avg"])?.evaluate()?,
    );

    let math_consts =
        TxtarArchive::read(&root.join("vendors/cue/pkg/math/testdata/consts.txtar")).await?;
    let math_round =
        TxtarArchive::read(&root.join("vendors/cue/pkg/math/testdata/round.txtar")).await?;
    let math_mult =
        TxtarArchive::read(&root.join("vendors/cue/pkg/math/testdata/mult.txtar")).await?;
    assert!(math_consts.file("in.cue")?.contains("math.Pi"));
    assert!(math_round.file("in.cue")?.contains("math.RoundToEven"));
    assert!(math_round.file("in.cue")?.contains("math.Floor(math.Pi)"));
    let math_gen =
        TxtarArchive::read(&root.join("vendors/cue/pkg/math/testdata/gen.txtar")).await?;
    assert!(math_gen.file("in.cue")?.contains("math.Jacobi(1000, 201)"));
    assert!(math_gen.file("in.cue")?.contains("math.Jacobi(1000, 2000)"));
    assert!(math_gen.file("in.cue")?.contains("math.Asin(2.0e400)"));
    assert!(math_gen.file("in.cue")?.contains("math.Pow(8, 4)"));
    assert!(math_gen.file("in.cue")?.contains("math.Cbrt(2)"));
    assert!(math_gen.file("in.cue")?.contains("math.Copysign(5, -2.2)"));
    assert!(math_gen.file("in.cue")?.contains("math.Dim(3, 2.5)"));
    assert!(
        math_mult
            .file("in.cue")?
            .contains("math.MultipleOf(99*99, 99)")
    );

    let math_value = context.compile_source(
        "pkg/math/reduced.cue",
        "import \"math\"\npi: math.Pi\nmaxPrec: math.MaxPrec\nfloorPi: \
         math.Floor(math.Pi)\nfloor: math.Floor(-2.2)\nceil: math.Ceil(-2.2)\ntrunc: \
         math.Trunc(-2.9)\nround: math.Round(-2.5)\neven: math.RoundToEven(2.5)\nabs: \
         math.Abs(-2.2)\nacos: math.Acos(0.5)\nacosh: math.Acosh(1)\nasin: \
         math.Asin(0.5)\nasinOverflow: math.Asin(2.0e400)\nasinh: math.Asinh(0)\natan: \
         math.Atan(1)\natan2: math.Atan2(1, 1)\natanh: math.Atanh(0.5)\nfloatUnderflow: \
         math.Sin(1e-400)\nfloatUnderflowBoundary: math.Sin(1e-324)\nfloatOverflowBoundary: \
         math.Sin(1.7976931348623158e308)\ncopySign: math.Copysign(5, -2.2)\ncbrt: \
         math.Cbrt(2)\ncbrtNeg: math.Cbrt(-8)\ncbrtNegZero: math.Cbrt(-0)\ncos: \
         math.Cos(0)\ncosh: math.Cosh(0)\ndimPositive: math.Dim(3, 2.5)\ndimZero: math.Dim(5, \
         7.2)\nerf: math.Erf(0)\nerfc: math.Erfc(0)\ngamma: math.Gamma(5)\nilogb: \
         math.Ilogb(8)\nj0: math.J0(0)\nj1: math.J1(0)\njacobi: math.Jacobi(1000, \
         201)\njacobiNeg: math.Jacobi(-1, 3)\njacobiZero: math.Jacobi(0, 3)\njacobiCommon: \
         math.Jacobi(3, 9)\njacobiNegDenom: math.Jacobi(1, -3)\njacobiBig: math.Jacobi(1, \
         170141183460469231731687303715884105729)\njacobiBad: math.Jacobi(1000, 2000)\njn: \
         math.Jn(0, 0)\njnNext: math.Jn(1, 0)\nldexp: math.Ldexp(0.5, 3)\nmultipleBool: \
         math.MultipleOf(5, 2.5)\nmultipleConstraint: 9 & math.MultipleOf(3)\nmultipleBoth: 12 & \
         math.MultipleOf(2) & math.MultipleOf(3)\nmultipleBad: 10 & math.MultipleOf(3)\nzero: \
         math.MultipleOf(5, 0)\nexpm1: math.Expm1(1)\nhypot: math.Hypot(3, 4)\nlog1p: \
         math.Log1p(1)\nlogb: math.Logb(8)\nlogbMax: \
         math.Logb(1.7976931348623157e308)\nlogbSubnormal: math.Logb(5e-324)\nmod: math.Mod(5.5, \
         2)\nsign: math.Signbit(-4)\nsin: math.Sin(0)\nsinh: math.Sinh(0)\nsqrt: \
         math.Sqrt(9)\ntan: math.Tan(0)\ntanh: math.Tanh(0)\nremainder: math.Remainder(5.5, \
         2)\ny0: math.Y0(1)\ny1: math.Y1(1)\nyn: math.Yn(2, 1)\npow: math.Pow(8, 4)\npowDecimal: \
         math.Pow(2.5, 2)\npowNeg: math.Pow(-2, 3)\npowNegEven: math.Pow(-2, 4)\npowNegExp: \
         math.Pow(2, -3)\npowNegDecimalExp: math.Pow(1.25, -2)\npowNegZero: math.Pow(-0, \
         3)\npow10: math.Pow10(4)\npow10Neg: math.Pow10(-2)\n",
    )?;
    assert_eq!(
        EvaluatedValue::Number(
            "3.14159265358979323846264338327950288419716939937510582097494459".to_owned(),
        ),
        math_value.lookup_path(&["pi"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("4294967295".to_owned()),
        math_value.lookup_path(&["maxPrec"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("3".to_owned()),
        math_value.lookup_path(&["floorPi"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-3".to_owned()),
        math_value.lookup_path(&["floor"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-2".to_owned()),
        math_value.lookup_path(&["ceil"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-2".to_owned()),
        math_value.lookup_path(&["trunc"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-3".to_owned()),
        math_value.lookup_path(&["round"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        math_value.lookup_path(&["even"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("2.2".to_owned()),
        math_value.lookup_path(&["abs"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1.0471975511965979".to_owned()),
        math_value.lookup_path(&["acos"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["acosh"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.5235987755982989".to_owned()),
        math_value.lookup_path(&["asin"])?.evaluate()?,
    );
    assert!(matches!(
        math_value.lookup_path(&["asinOverflow"])?.evaluate()?,
        EvaluatedValue::Bottom(_),
    ));
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["asinh"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.7853981633974483".to_owned()),
        math_value.lookup_path(&["atan"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.7853981633974483".to_owned()),
        math_value.lookup_path(&["atan2"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.5493061443340548".to_owned()),
        math_value.lookup_path(&["atanh"])?.evaluate()?,
    );
    assert!(matches!(
        math_value.lookup_path(&["floatUnderflow"])?.evaluate()?,
        EvaluatedValue::Bottom(_),
    ));
    assert!(matches!(
        math_value
            .lookup_path(&["floatUnderflowBoundary"])?
            .evaluate()?,
        EvaluatedValue::Bottom(_),
    ));
    assert!(matches!(
        math_value
            .lookup_path(&["floatOverflowBoundary"])?
            .evaluate()?,
        EvaluatedValue::Bottom(_),
    ));
    assert_eq!(
        EvaluatedValue::Number("-5".to_owned()),
        math_value.lookup_path(&["copySign"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1.259921049894873164767210607278228".to_owned()),
        math_value.lookup_path(&["cbrt"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-2".to_owned()),
        math_value.lookup_path(&["cbrtNeg"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-0".to_owned()),
        math_value.lookup_path(&["cbrtNegZero"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        math_value.lookup_path(&["cos"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        math_value.lookup_path(&["cosh"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.5".to_owned()),
        math_value.lookup_path(&["dimPositive"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["dimZero"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["erf"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        math_value.lookup_path(&["erfc"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("24".to_owned()),
        math_value.lookup_path(&["gamma"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("3".to_owned()),
        math_value.lookup_path(&["ilogb"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        math_value.lookup_path(&["j0"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["j1"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        math_value.lookup_path(&["jacobi"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-1".to_owned()),
        math_value.lookup_path(&["jacobiNeg"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["jacobiZero"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["jacobiCommon"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        math_value.lookup_path(&["jacobiNegDenom"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        math_value.lookup_path(&["jacobiBig"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        math_value.lookup_path(&["jn"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["jnNext"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("4".to_owned()),
        math_value.lookup_path(&["ldexp"])?.evaluate()?,
    );
    assert!(
        math_value
            .lookup_path(&["jacobiBad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    assert_eq!(
        EvaluatedValue::Bool(true),
        math_value.lookup_path(&["multipleBool"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("9".to_owned()),
        math_value
            .lookup_path(&["multipleConstraint"])?
            .evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("12".to_owned()),
        math_value.lookup_path(&["multipleBoth"])?.evaluate()?,
    );
    assert!(
        math_value
            .lookup_path(&["multipleBad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    assert!(matches!(
        math_value.lookup_path(&["zero"])?.evaluate()?,
        EvaluatedValue::Bottom(_),
    ));
    assert_eq!(
        EvaluatedValue::Number("1.718281828459045".to_owned()),
        math_value.lookup_path(&["expm1"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("5".to_owned()),
        math_value.lookup_path(&["hypot"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.6931471805599453".to_owned()),
        math_value.lookup_path(&["log1p"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("3".to_owned()),
        math_value.lookup_path(&["logb"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1023".to_owned()),
        math_value.lookup_path(&["logbMax"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-1074".to_owned()),
        math_value.lookup_path(&["logbSubnormal"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1.5".to_owned()),
        math_value.lookup_path(&["mod"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Bool(true),
        math_value.lookup_path(&["sign"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["sin"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["sinh"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("3".to_owned()),
        math_value.lookup_path(&["sqrt"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["tan"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        math_value.lookup_path(&["tanh"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-0.5".to_owned()),
        math_value.lookup_path(&["remainder"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.08825696421567697".to_owned()),
        math_value.lookup_path(&["y0"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-0.7812128213002887".to_owned()),
        math_value.lookup_path(&["y1"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-1.6506826068162543".to_owned()),
        math_value.lookup_path(&["yn"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("4096".to_owned()),
        math_value.lookup_path(&["pow"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("6.25".to_owned()),
        math_value.lookup_path(&["powDecimal"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-8".to_owned()),
        math_value.lookup_path(&["powNeg"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("16".to_owned()),
        math_value.lookup_path(&["powNegEven"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.125".to_owned()),
        math_value.lookup_path(&["powNegExp"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.64".to_owned()),
        math_value.lookup_path(&["powNegDecimalExp"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("-0".to_owned()),
        math_value.lookup_path(&["powNegZero"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("10000".to_owned()),
        math_value.lookup_path(&["pow10"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0.01".to_owned()),
        math_value.lookup_path(&["pow10Neg"])?.evaluate()?,
    );
    Ok(())
}

async fn assert_upstream_phase9_parity_tranche(context: &Context, root: &Path) -> TestResult {
    assert_upstream_interpolation_parity(context, root).await?;
    assert_upstream_comprehension_parity(context, root).await?;
    assert_upstream_dynamic_field_parity(context, root).await?;
    assert_upstream_cycle_fixpoint_parity(context, root).await?;
    assert_pattern_field_parity(context)?;
    Ok(())
}

async fn assert_upstream_interpolation_parity(context: &Context, root: &Path) -> TestResult {
    let interpolation =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/interpolation/scalars.txtar"))
            .await?;
    assert!(
        interpolation
            .file("in.cue")?
            .contains("\"1+1=2:  \\(true)\"")
    );
    let interpolated = context.compile_source(
        "interpolation/scalars/reduced.cue",
        "n: \"\\(1) \\(2.00)\"\nb: \"1+1=2:  \\(true)\"\n",
    )?;
    assert_eq!(
        EvaluatedValue::String("1 2.00".to_owned()),
        interpolated.lookup_path(&["n"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("1+1=2:  true".to_owned()),
        interpolated.lookup_path(&["b"])?.evaluate()?,
    );
    Ok(())
}

async fn assert_upstream_comprehension_parity(context: &Context, root: &Path) -> TestResult {
    let comprehensions =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/comprehensions/for.txtar")).await?;
    assert!(comprehensions.file("in.cue")?.contains("for k, v in a"));
    let comprehension_value = context.compile_source(
        "comprehensions/for/reduced.cue",
        "a: {b: 1, c: 2}\nb: {for k, v in a {\"\\(k)\": v + 1}}\nempty: {for k, v in {} \
         {\"\\(k)\": v}}\nlistStructs: [for x in [1, 2] {{a: x}}]\nbad: [for x in 1 {x}]\n",
    )?;
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        comprehension_value.lookup_path(&["b", "b"])?.evaluate()?,
    );
    assert_eq!(
        ValueKind::Struct,
        comprehension_value.lookup_path(&["empty"])?.kind()?,
    );
    let EvaluatedValue::List(list_structs) = comprehension_value
        .lookup_path(&["listStructs"])?
        .evaluate()?
    else {
        return Err("expected list-valued struct comprehension".into());
    };
    assert_eq!(2, list_structs.len());
    assert!(
        comprehension_value
            .lookup_path(&["bad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    Ok(())
}

async fn assert_upstream_dynamic_field_parity(context: &Context, root: &Path) -> TestResult {
    let dynamic =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/eval/dynamic_field.txtar")).await?;
    assert!(dynamic.file("in.cue")?.contains("(x):"));
    let dynamic_value = context.compile_source(
        "eval/dynamic_field/reduced.cue",
        "k: \"field\"\nout: {(k): 1, \"\\(2)\": 2}\nbad: {(1): 1}\n",
    )?;
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        dynamic_value.lookup_path(&["out", "field"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        dynamic_value.lookup_path(&["out", "2"])?.evaluate()?,
    );
    assert!(
        dynamic_value
            .lookup_path(&["bad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    Ok(())
}

fn assert_pattern_field_parity(context: &Context) -> TestResult {
    let pattern_value = context.compile_source(
        "eval/pattern_field/reduced.cue",
        "topBad: {[string]: int, a: \"x\"}\nok: {[string]: int, a: 1}\nbad: {[string]: int, a: \
         \"x\"}\ncrossBad: {[string]: int} & {a: \"x\"}\nregex: {[=~\"^a\"]: string, apple: \
         \"ok\", banana: 1}\n",
    )?;
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        pattern_value.lookup_path(&["ok", "a"])?.evaluate()?,
    );
    assert!(
        pattern_value
            .lookup_path(&["topBad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    assert!(
        pattern_value
            .lookup_path(&["bad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    assert!(
        pattern_value
            .lookup_path(&["crossBad"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    assert!(
        pattern_value
            .lookup_path(&["regex"])?
            .validate(ValidateOptions::default())
            .is_ok(),
    );
    Ok(())
}

async fn assert_upstream_cycle_fixpoint_parity(context: &Context, root: &Path) -> TestResult {
    let disjunctions = TxtarArchive::read(&root.join(
        "vendors/cue/cue/testdata/cycle/051_resolved_self-reference_cycles_with_disjunction.txtar",
    ))
    .await?;
    assert!(
        disjunctions
            .file("in.cue")?
            .contains("xa1: (xa2 & 8) | (xa4 & 9)")
    );
    let defaults = TxtarArchive::read(&root.join(
        "vendors/cue/cue/testdata/cycle/\
         052_resolved_self-reference_cycles_with_disjunction_with_defaults.txtar",
    ))
    .await?;
    assert!(
        defaults
            .file("in.cue")?
            .contains("xa1: (xa2 & 8) | *(xa4 & 9)")
    );
    let default_bounds =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/cycle/with_defaults.txtar"))
            .await?;
    assert!(default_bounds.file("in.cue")?.contains("range: >min"));

    let value = context.compile_source(
        "cycle/fixpoint/reduced.cue",
        "xa1: (xa2 & 8) | (xa4 & 9)\nxa2: xa3 + 2\nxa3: 6 & xa1-2\nxa4: xa2 + 2\nda1: (da2 & 8) | \
         *(da4 & 9)\nda2: da3 + 2\nda3: 6 & da1-2\nda4: da2 + 2\nrange1: {min: *1 | int, range: \
         >min, range: 8}\nrange2: {min: *1 | int, max: int & >min}\nrg: range2\nrg: {max: \
         8}\nxb1: (xb2 & 8) | (xb4 & 9)\nxb2: xb3 + 2\nxb3: (6 & (xb1 - 2)) | (xb4 & 9)\nxb4: xb2 \
         + 2\ndb1: *(db2 & 8) | (db4 & 9)\ndb2: db3 + 2\ndb3: *(6 & (db1 - 2)) | (db4 & 9)\ndb4: \
         db2 + 2\n",
    )?;
    for (path, expected) in [
        (&["xa1"][..], "8"),
        (&["xa2"][..], "8"),
        (&["xa3"][..], "6"),
        (&["xa4"][..], "10"),
        (&["da1"][..], "8"),
        (&["da2"][..], "8"),
        (&["da3"][..], "6"),
        (&["da4"][..], "10"),
        (&["range1", "range"][..], "8"),
        (&["rg", "max"][..], "8"),
    ] {
        assert_eq!(
            EvaluatedValue::Number(expected.to_owned()),
            value.lookup_path(path)?.evaluate()?,
            "unexpected borrowed cycle value at {path:?}",
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

async fn assert_upstream_incomplete_operand_errors(context: &Context, root: &Path) -> TestResult {
    let incomplete =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/eval/incompleteperm.txtar"))
            .await?;
    let source = incomplete.file("in.cue")?;
    assert!(source.contains("issue680: (>10 * 2) & 0"));
    assert!(source.contains("issue405: >=100 <= 200"));
    let value = context.compile_source(
        "eval/incompleteperm/reduced.cue",
        "nested: (int + 1) + (int + 1)\ndisjunct: (int + 1) | (int + 1)\nissue680: (>10 * 2) & \
         0\nissue405: >=100 <= 200\n",
    )?;
    for path in ["nested", "disjunct", "issue680", "issue405"] {
        assert!(
            value
                .lookup_path(&[path])?
                .validate(ValidateOptions::default())
                .is_err(),
            "expected borrowed incomplete operand case `{path}` to fail",
        );
    }
    Ok(())
}

async fn assert_upstream_list_index_and_slice(context: &Context, root: &Path) -> TestResult {
    let lists =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/010_lists.txtar"))
            .await?;
    let upstream_source = lists.file("in.cue")?;
    assert!(upstream_source.contains("...>=4 & <=5"));
    let list_source = first_lines(upstream_source, 2);
    let list_value = context.compile_source("basicrewrite/010_lists/in.cue", list_source)?;
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        list_value.lookup_path(&["index"])?.evaluate()?,
    );
    let slice_value = context.compile_source(
        "basicrewrite/010_lists/slice.cue",
        "slice: [1, 2, 3][1:3]\nopenEnd: [1, 2, 3][1:]\nopenStart: [1, 2, 3][:2]\n",
    )?;
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("2".to_owned()),
            EvaluatedValue::Number("3".to_owned()),
        ]),
        slice_value.lookup_path(&["slice"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("2".to_owned()),
            EvaluatedValue::Number("3".to_owned()),
        ]),
        slice_value.lookup_path(&["openEnd"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("1".to_owned()),
            EvaluatedValue::Number("2".to_owned()),
        ]),
        slice_value.lookup_path(&["openStart"])?.evaluate()?,
    );

    let open_value = context.compile_source(
        "basicrewrite/010_lists/open.cue",
        "ok: [1, 2, ...>=4 & <=5] & [1, 2, 4, 5]\ne4: [1, 2, ...>=4 & <=5] & [1, 2, 4, 8]\ne5: \
         [1, 2, 4, 8] & [1, 2, ...>=4 & <=5]\n",
    )?;
    assert_eq!(
        EvaluatedValue::List(vec![
            EvaluatedValue::Number("1".to_owned()),
            EvaluatedValue::Number("2".to_owned()),
            EvaluatedValue::Number("4".to_owned()),
            EvaluatedValue::Number("5".to_owned()),
        ]),
        open_value.lookup_path(&["ok"])?.evaluate()?,
    );
    assert_validation_diagnostic_contains(
        &open_value,
        "e4",
        "invalid value 8 for numeric constraint >=4 & <=5",
    )?;
    assert_validation_diagnostic_contains(
        &open_value,
        "e5",
        "invalid value 8 for numeric constraint >=4 & <=5",
    )?;
    Ok(())
}

fn assert_validation_diagnostic_contains(
    value: &cue_rust::Value,
    path: &str,
    expected: &str,
) -> TestResult {
    let result = value
        .lookup_path(&[path])?
        .validate(ValidateOptions::default());
    let Err(EvalError::Diagnostics(report)) = result else {
        return Err(format!("expected validation diagnostics for `{path}`").into());
    };
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message().contains(expected)),
        "expected `{path}` diagnostic to contain `{expected}`, got {:?}",
        report.diagnostics(),
    );
    Ok(())
}

async fn assert_upstream_selecting(context: &Context, root: &Path) -> TestResult {
    let selecting =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/012_selecting.txtar"))
            .await?;
    let value = context.compile_source(
        "basicrewrite/012_selecting/in.cue",
        selecting.file("in.cue")?,
    )?;
    assert_eq!(
        EvaluatedValue::Number("2".to_owned()),
        value.lookup_path(&["index"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("3".to_owned()),
        value.lookup_path(&["mulidx"])?.evaluate()?,
    );
    for path in ["e", "f", "g", "h"] {
        assert!(
            value
                .lookup_path(&[path])?
                .validate(ValidateOptions::default())
                .is_err(),
        );
    }
    Ok(())
}

async fn assert_upstream_basic_types(context: &Context, root: &Path) -> TestResult {
    let basic_types = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/006_basic_type.txtar"),
    )
    .await?;
    let value = context.compile_source(
        "basicrewrite/006_basic_type/in.cue",
        basic_types.file("in.cue")?,
    )?;
    for (path, expected) in [
        ("a", EvaluatedValue::Number("1".to_owned())),
        ("b", EvaluatedValue::Number("1".to_owned())),
        ("c", EvaluatedValue::Number("1.0".to_owned())),
        ("e", EvaluatedValue::String("4".to_owned())),
        ("f", EvaluatedValue::Bool(true)),
    ] {
        assert_eq!(expected, value.lookup_path(&[path])?.evaluate()?);
    }
    assert!(
        value
            .lookup_path(&["d"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    Ok(())
}

async fn assert_upstream_types(context: &Context, root: &Path) -> TestResult {
    let types =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/015_types.txtar"))
            .await?;
    let value = context.compile_source("basicrewrite/015_types/in.cue", types.file("in.cue")?)?;
    assert_eq!(ValueKind::Int, value.lookup_path(&["i"])?.kind()?);
    assert_eq!(
        EvaluatedValue::Number("3".to_owned()),
        value.lookup_path(&["j"])?.evaluate()?,
    );
    assert_eq!(ValueKind::String, value.lookup_path(&["s"])?.kind()?);
    assert_eq!(
        EvaluatedValue::String("s".to_owned()),
        value.lookup_path(&["t"])?.evaluate()?,
    );
    for path in ["e", "e2", "b", "p", "m"] {
        assert!(
            value
                .lookup_path(&[path])?
                .validate(ValidateOptions::default())
                .is_err(),
        );
    }
    Ok(())
}

async fn assert_upstream_numeric_comparisons(context: &Context, root: &Path) -> TestResult {
    let comparison = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/016_comparison.txtar"),
    )
    .await?;
    let value = context.compile_source(
        "basicrewrite/016_comparison/in.cue",
        comparison.file("in.cue")?,
    )?;
    for path in ["tLss", "tLeq", "tEql", "tNeq", "tGeq", "tGtr", "tExpr"] {
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["numbers", path])?.evaluate()?,
        );
    }
    Ok(())
}

async fn assert_upstream_list_comparisons(context: &Context, root: &Path) -> TestResult {
    let comparison = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/016_comparison.txtar"),
    )
    .await?;
    let list_source = format!(
        "lists: {{\n{}\n}}\n",
        selected_lines(
            comparison.file("lists.cue")?,
            &[
                "t1:", "t2:", "t3:", "t4:", "f1:", "f2:", "f3:", "f4:", "f5:", "tNeq1:", "fNeq2:",
            ],
        )
    );
    let value = context.compile_source("basicrewrite/016_comparison/lists.cue", list_source)?;
    for path in ["t1", "t2", "t3", "t4", "tNeq1"] {
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["lists", path])?.evaluate()?,
        );
    }
    for path in ["f1", "f2", "f3", "f4", "f5", "fNeq2"] {
        assert_eq!(
            EvaluatedValue::Bool(false),
            value.lookup_path(&["lists", path])?.evaluate()?,
        );
    }
    Ok(())
}

async fn assert_upstream_struct_comparisons(context: &Context, root: &Path) -> TestResult {
    let comparison = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/016_comparison.txtar"),
    )
    .await?;
    let struct_source = format!(
        "structs: {{ eq: {{\n{}\n}} }}\n",
        selected_lines_in_block(
            comparison.file("structs.cue")?,
            "structs: eq:",
            "}",
            &[
                "t1:", "t2:", "t3:", "t4:", "t5:", "f1:", "f2:", "f3:", "f4:", "f5:", "f6:",
                "tNe1:", "fNe1:",
            ],
        )
    );
    let value = context.compile_source("basicrewrite/016_comparison/structs.cue", struct_source)?;
    for path in ["t1", "t2", "t3", "t4", "t5", "tNe1"] {
        assert_eq!(
            EvaluatedValue::Bool(true),
            value.lookup_path(&["structs", "eq", path])?.evaluate()?,
        );
    }
    for path in ["f1", "f2", "f3", "f4", "f5", "f6", "fNe1"] {
        assert_eq!(
            EvaluatedValue::Bool(false),
            value.lookup_path(&["structs", "eq", path])?.evaluate()?,
        );
    }
    Ok(())
}

async fn assert_upstream_arithmetic(context: &Context, root: &Path) -> TestResult {
    let arithmetic = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/002_arithmetic.txtar"),
    )
    .await?;
    let arithmetic_source = first_lines(arithmetic.file("in.cue")?, 7);
    let arithmetic_value =
        context.compile_source("basicrewrite/002_arithmetic/in.cue", arithmetic_source)?;
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        arithmetic_value.lookup_path(&["sum"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("4".to_owned()),
        arithmetic_value.lookup_path(&["div1"])?.evaluate()?,
    );
    Ok(())
}

async fn assert_upstream_integer_arithmetic(context: &Context, root: &Path) -> TestResult {
    let integer_arithmetic = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/003_integer-specific_arithmetic.txtar"),
    )
    .await?;
    let value = context.compile_source(
        "basicrewrite/003_integer-specific_arithmetic/in.cue",
        integer_arithmetic.file("in.cue")?,
    )?;
    for (path, expected) in [
        ("q1", "2"),
        ("q2", "-2"),
        ("q3", "-2"),
        ("q4", "2"),
        ("r1", "1"),
        ("r2", "1"),
        ("r3", "-1"),
        ("r4", "-1"),
        ("d1", "2"),
        ("d2", "-2"),
        ("d3", "-3"),
        ("d4", "3"),
        ("m1", "1"),
        ("m2", "1"),
        ("m3", "1"),
        ("m4", "1"),
    ] {
        assert_eq!(
            EvaluatedValue::Number(expected.to_owned()),
            value.lookup_path(&[path])?.evaluate()?,
        );
    }
    for path in ["qe1", "qe2", "re1", "re2", "de1", "de2", "me1", "me2"] {
        assert!(
            value
                .lookup_path(&[path])?
                .validate(ValidateOptions::default())
                .is_err(),
        );
    }
    Ok(())
}

async fn assert_upstream_booleans(context: &Context, root: &Path) -> TestResult {
    let booleans =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/004_booleans.txtar"))
            .await?;
    let boolean_value =
        context.compile_source("basicrewrite/004_booleans/in.cue", booleans.file("in.cue")?)?;
    assert_eq!(
        EvaluatedValue::Bool(true),
        boolean_value.lookup_path(&["t"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Bool(false),
        boolean_value.lookup_path(&["f"])?.evaluate()?,
    );
    assert!(
        boolean_value
            .lookup_path(&["e"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );

    let boolean_arithmetic = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/005_boolean_arithmetic.txtar"),
    )
    .await?;
    let value = context.compile_source(
        "basicrewrite/005_boolean_arithmetic/in.cue",
        boolean_arithmetic.file("in.cue")?,
    )?;
    for (path, expected) in [
        ("a", true),
        ("b", true),
        ("c", false),
        ("d", true),
        ("e", true),
    ] {
        assert_eq!(
            EvaluatedValue::Bool(expected),
            value.lookup_path(&[path])?.evaluate()?,
        );
    }
    assert!(
        value
            .lookup_path(&["f"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    Ok(())
}

async fn assert_upstream_null(context: &Context, root: &Path) -> TestResult {
    let null_fixture =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/017_null.txtar"))
            .await?;
    let value =
        context.compile_source("basicrewrite/017_null/in.cue", null_fixture.file("in.cue")?)?;
    for (path, expected) in [
        ("eql", EvaluatedValue::Bool(true)),
        ("neq", EvaluatedValue::Bool(false)),
        ("unf", EvaluatedValue::Null),
        ("eq1", EvaluatedValue::Bool(false)),
        ("eq2", EvaluatedValue::Bool(false)),
        ("ne1", EvaluatedValue::Bool(true)),
    ] {
        assert_eq!(expected, value.lookup_path(&[path])?.evaluate()?);
    }
    assert!(
        value
            .lookup_path(&["call"])?
            .validate(ValidateOptions::default())
            .is_err(),
    );
    Ok(())
}

async fn assert_upstream_len(context: &Context, root: &Path) -> TestResult {
    let len_fixture = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/fulleval/027_len_of_incomplete_types.txtar"),
    )
    .await?;
    let len_source = selected_lines(len_fixture.file("in.cue")?, &["v2:", "v3:", "v4:"]);
    let len_value =
        context.compile_source("fulleval/027_len_of_incomplete_types/in.cue", len_source)?;
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        len_value.lookup_path(&["v2"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("0".to_owned()),
        len_value.lookup_path(&["v3"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Number("1".to_owned()),
        len_value.lookup_path(&["v4"])?.evaluate()?,
    );
    Ok(())
}

async fn assert_upstream_strings_and_bytes(context: &Context, root: &Path) -> TestResult {
    let strings_bytes = TxtarArchive::read(
        &root.join("vendors/cue/cue/testdata/basicrewrite/007_strings_and_bytes.txtar"),
    )
    .await?;
    let strings_bytes_source = first_lines(strings_bytes.file("in.cue")?, 7);
    let strings_bytes_value = context.compile_source(
        "basicrewrite/007_strings_and_bytes/in.cue",
        strings_bytes_source,
    )?;
    assert_eq!(
        EvaluatedValue::String("abcabcabc".to_owned()),
        strings_bytes_value.lookup_path(&["s1"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Bytes(b"foobar".to_vec()),
        strings_bytes_value.lookup_path(&["b0"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::Bytes(b"abcabc".to_vec()),
        strings_bytes_value.lookup_path(&["b2"])?.evaluate()?,
    );
    Ok(())
}

async fn assert_upstream_escaping(context: &Context, root: &Path) -> TestResult {
    let escaping =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/008_escaping.txtar"))
            .await?;
    let escaping_source = first_lines(escaping.file("in.cue")?, 2);
    let escaping_value =
        context.compile_source("basicrewrite/008_escaping/in.cue", escaping_source)?;
    assert_eq!(
        EvaluatedValue::String("foo\nbar".to_owned()),
        escaping_value.lookup_path(&["a"])?.evaluate()?,
    );
    assert_eq!(
        EvaluatedValue::String("foo\nbar".to_owned()),
        escaping_value.lookup_path(&["b"])?.evaluate()?,
    );
    Ok(())
}

#[tokio::test]
async fn test_should_export_borrowed_vendor_fixture_values() -> TestResult {
    let root = workspace_root();
    let context = Context::new();
    let export_cue =
        TxtarArchive::read(&root.join("vendors/cue/cmd/cue/cmd/testdata/script/export_cue.txtar"))
            .await?;
    let value = context.compile_source("nopkg.cue", export_cue.file("nopkg.cue")?)?;

    let mut options = EncodeOptions::default();
    options.encoding = Encoding::Json;
    let json = encode_value(&value, options)?;

    assert!(json.contains("\"x\": 5"));
    assert!(json.contains("\"y\": 4"));
    Ok(())
}

fn txtar_header(line: &str) -> Option<&str> {
    line.strip_prefix("-- ")?.strip_suffix(" --")
}

fn first_lines(source: &str, limit: usize) -> String {
    source.lines().take(limit).collect::<Vec<_>>().join("\n")
}

fn selected_lines(source: &str, prefixes: &[&str]) -> String {
    source
        .lines()
        .filter(|line| {
            prefixes
                .iter()
                .any(|prefix| line.trim_start().starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn selected_lines_in_block(source: &str, start: &str, end: &str, prefixes: &[&str]) -> String {
    let mut in_block = false;
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            if !in_block {
                in_block = trimmed.starts_with(start);
                return false;
            }
            if trimmed == end {
                in_block = false;
                return false;
            }
            prefixes.iter().any(|prefix| trimmed.starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn collect_txtar_files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(path) = stack.pop() {
        let mut entries = fs::read_dir(&path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
            } else if entry_path.extension() == Some(OsStr::new("txtar")) {
                files.push(entry_path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}
