//! Integration coverage borrowed from the vendored upstream CUE corpus.

use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use cue_rust::{
    Context, DecodeOptions, EncodeOptions, Encoding, EvaluatedValue, ValidateOptions, ValueKind,
    decode_bytes, encode_value,
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
    assert_upstream_object_unification(&context, &root).await?;
    assert_upstream_list_index_and_slice(&context, &root).await?;
    assert_upstream_arithmetic(&context, &root).await?;
    assert_upstream_integer_arithmetic(&context, &root).await?;
    assert_upstream_len(&context, &root).await?;
    assert_upstream_strings_and_bytes(&context, &root).await?;
    assert_upstream_escaping(&context, &root).await?;
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

async fn assert_upstream_list_index_and_slice(context: &Context, root: &Path) -> TestResult {
    let lists =
        TxtarArchive::read(&root.join("vendors/cue/cue/testdata/basicrewrite/010_lists.txtar"))
            .await?;
    let list_source = first_lines(lists.file("in.cue")?, 2);
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
