//! CLI integration coverage borrowed from vendored upstream CUE scripts.

use std::{
    collections::BTreeMap,
    error::Error,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
};

use tokio::{fs, process::Command};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

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
async fn test_should_export_upstream_export_cue_fixture() -> TestResult {
    let archive = TxtarArchive::read(
        &workspace_root().join("vendors/cue/cmd/cue/cmd/testdata/script/export_cue.txtar"),
    )
    .await?;
    let dir = fixture_dir().await?;
    let cue = dir.join("nopkg.cue");
    fs::write(&cue, archive.file("nopkg.cue")?).await?;

    let output = run(&["export", "--out", "json", path_arg(&cue)?.as_str()]).await?;
    assert_success(&output)?;
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"x\": 5"));
    assert!(stdout.contains("\"y\": 4"));
    Ok(())
}

#[tokio::test]
async fn test_should_vet_upstream_json_fixture_against_borrowed_schema() -> TestResult {
    let archive = TxtarArchive::read(
        &workspace_root().join("vendors/cue/cmd/cue/cmd/testdata/script/export_cue.txtar"),
    )
    .await?;
    let dir = fixture_dir().await?;
    let schema = dir.join("schema.cue");
    let data = dir.join("x.json");
    fs::write(&schema, "x: number\n").await?;
    fs::write(&data, archive.file("x.json")?).await?;

    let output = run(&[
        "vet",
        path_arg(&schema)?.as_str(),
        "--data",
        path_arg(&data)?.as_str(),
    ])
    .await?;
    assert_success(&output)
}

#[tokio::test]
async fn test_should_reject_upstream_json_syntax_errors() -> TestResult {
    let archive = TxtarArchive::read(
        &workspace_root().join("vendors/cue/cmd/cue/cmd/testdata/script/json_syntax_error.txtar"),
    )
    .await?;
    let dir = fixture_dir().await?;
    let schema = dir.join("schema.cue");
    fs::write(&schema, "foo: bool\nbar: number\nbaz: bool\n").await?;

    for source_name in ["x1.json", "x2.json", "x3.json", "x4.json", "x5.jsonl"] {
        let data = dir.join(source_name);
        fs::write(&data, archive.file(source_name)?).await?;
        let output = run(&[
            "vet",
            path_arg(&schema)?.as_str(),
            "--data",
            path_arg(&data)?.as_str(),
        ])
        .await?;
        assert!(
            !output.status.success(),
            "expected upstream invalid JSON fixture `{source_name}` to fail",
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_should_export_upstream_expression_fixture() -> TestResult {
    let archive = TxtarArchive::read(
        &workspace_root().join("vendors/cue/cmd/cue/cmd/testdata/script/export_expr.txtar"),
    )
    .await?;
    let dir = fixture_dir().await?;
    let simple = dir.join("data.cue");
    fs::write(&simple, archive.file("simple/data.cue")?).await?;

    let output = run(&[
        "export",
        "--out",
        "yaml",
        "--expr",
        "a+c",
        "--expr",
        "d.e.f",
        path_arg(&simple)?.as_str(),
    ])
    .await?;
    assert_success(&output)?;
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains('4'));
    assert!(stdout.contains("jam"));
    Ok(())
}

#[tokio::test]
async fn test_should_eval_upstream_expression_selection_fixture() -> TestResult {
    let archive = TxtarArchive::read(
        &workspace_root().join("vendors/cue/cmd/cue/cmd/testdata/script/eval_expr.txtar"),
    )
    .await?;
    let dir = fixture_dir().await?;
    let partial = dir.join("partial.cue");
    fs::write(&partial, archive.file("partial.cue")?).await?;

    let output = run(&["eval", "--expr", "b.a.b", path_arg(&partial)?.as_str()]).await?;
    assert_success(&output)?;
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains('4'));
    Ok(())
}

#[tokio::test]
async fn test_should_export_positional_upstream_toml_data_fixture() -> TestResult {
    let archive = TxtarArchive::read(
        &workspace_root().join("vendors/cue/cmd/cue/cmd/testdata/script/encoding_toml.txtar"),
    )
    .await?;
    let dir = fixture_dir().await?;
    let data = dir.join("export.toml");
    fs::write(&data, archive.file("export.toml")?).await?;
    let data_arg = format!("toml:{}", path_arg(&data)?);

    let output = run(&["export", "--out", "json", &data_arg]).await?;
    assert_success(&output)?;
    let stdout = String::from_utf8(output.stdout)?;
    assert!(!stdout.starts_with("{}"));
    assert!(stdout.contains("\"message\": \"Hello World!\""));
    assert!(stdout.contains("\"b\": \"two levels\""));
    Ok(())
}

async fn fixture_dir() -> Result<PathBuf, Box<dyn Error>> {
    let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cue-rust-vendor-{}-{id}", std::process::id()));
    fs::create_dir_all(&path).await?;
    Ok(path)
}

async fn run(args: &[&str]) -> Result<Output, Box<dyn Error>> {
    let binary = std::env::var("CARGO_BIN_EXE_cue-rs")?;
    Ok(Command::new(binary).args(args).output().await?)
}

fn assert_success(output: &Output) -> TestResult {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "command failed: stdout=`{}`, stderr=`{}`",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
        .into())
    }
}

fn path_arg(path: &Path) -> Result<String, Box<dyn Error>> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()).into())
}

fn txtar_header(line: &str) -> Option<&str> {
    line.strip_prefix("-- ")?.strip_suffix(" --")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}
