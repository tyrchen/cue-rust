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
    assert_upstream_booleans(&context, &root).await?;
    assert_upstream_null(&context, &root).await?;
    assert_upstream_len(&context, &root).await?;
    assert_upstream_strings_and_bytes(&context, &root).await?;
    assert_upstream_escaping(&context, &root).await?;
    assert_upstream_definitions_and_export_profiles(&context, &root).await?;
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
