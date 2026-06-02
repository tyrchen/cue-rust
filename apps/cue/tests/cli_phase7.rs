//! Phase 7 CLI integration coverage.

use std::{
    error::Error,
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
};

use tokio::{fs, process::Command};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

async fn fixture_dir() -> Result<PathBuf, Box<dyn Error>> {
    let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cue-rust-cli-{}-{id}", std::process::id()));
    fs::create_dir_all(&path).await?;
    Ok(path)
}

fn binary_path() -> Result<String, std::env::VarError> {
    std::env::var("CARGO_BIN_EXE_cue").or_else(|_| std::env::var("CARGO_BIN_EXE_cue-rs"))
}

async fn run(args: &[&str]) -> Result<Output, Box<dyn Error>> {
    let binary = binary_path()?;
    Ok(Command::new(binary).args(args).output().await?)
}

async fn run_in_dir(args: &[&str], current_dir: &PathBuf) -> Result<Output, Box<dyn Error>> {
    let binary = binary_path()?;
    Ok(Command::new(binary)
        .current_dir(current_dir)
        .args(args)
        .output()
        .await?)
}

async fn run_with_stdin(args: &[&str], stdin: &str) -> Result<Output, Box<dyn Error>> {
    let binary = binary_path()?;
    let mut child = Command::new(binary)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut child_stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;

        child_stdin.write_all(stdin.as_bytes()).await?;
    }
    Ok(child.wait_with_output().await?)
}

#[tokio::test]
async fn test_should_eval_cue_file() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("basic.cue");
    fs::write(&cue, "x: 1\ny: \"ok\"\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["eval", &cue_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("{x: 1, y: \"ok\"}"));
    Ok(())
}

#[tokio::test]
async fn test_should_eval_selected_expression() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("basic.cue");
    fs::write(&cue, "x: { y: { z: 3 } }\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["eval", "--expr", "x.y.z", &cue_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!("3\n", stdout);
    Ok(())
}

#[tokio::test]
async fn test_should_export_json() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("basic.cue");
    fs::write(&cue, "x: 1\ny: \"ok\"\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["export", "--out", "json", &cue_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"x\": 1"));
    assert!(stdout.contains("\"y\": \"ok\""));
    Ok(())
}

#[tokio::test]
async fn test_should_export_selected_expression_json() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("basic.cue");
    fs::write(&cue, "x: { y: 2 }\nz: 3\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["export", "--out", "json", "--expr", "x", &cue_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"y\": 2"));
    assert!(!stdout.contains("\"z\": 3"));
    Ok(())
}

#[tokio::test]
async fn test_should_hide_definitions_hidden_and_optional_fields_by_default()
-> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("profile.cue");
    fs::write(
        &cue,
        "#Port: int & >=1\n_hidden: \"scratch\"\noptional?: string\nport: #Port & 8080\n",
    )
    .await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["export", "--out", "json", &cue_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"port\": 8080"));
    assert!(!stdout.contains("#Port"));
    assert!(!stdout.contains("_hidden"));
    assert!(!stdout.contains("optional"));
    Ok(())
}

#[tokio::test]
async fn test_should_show_export_profile_fields_when_requested() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("profile.cue");
    fs::write(
        &cue,
        "#Port: 8080\n_hidden: \"scratch\"\noptional?: \"maybe\"\nport: #Port\n",
    )
    .await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&[
        "eval",
        "--show-definitions",
        "--show-hidden",
        "--show-optional",
        &cue_arg,
    ])
    .await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("#Port: 8080"));
    assert!(stdout.contains("_hidden: \"scratch\""));
    assert!(stdout.contains("optional?: \"maybe\""));
    Ok(())
}

#[tokio::test]
async fn test_should_reject_optional_constraints_in_concrete_export() -> Result<(), Box<dyn Error>>
{
    let dir = fixture_dir().await?;
    let cue = dir.join("profile.cue");
    fs::write(&cue, "optional?: \"maybe\"\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["export", "--show-optional", "--out", "json", &cue_arg]).await?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("optional field constraint is not concrete data"));
    Ok(())
}

#[tokio::test]
async fn test_should_fail_on_missing_selected_expression() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("basic.cue");
    fs::write(&cue, "x: 1\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["eval", "--expr", "missing", &cue_arg]).await?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("failed to select expression"));
    Ok(())
}

#[tokio::test]
async fn test_should_export_directory() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    fs::write(dir.join("a.cue"), "package p\nx: 1\n").await?;
    fs::write(dir.join("b.cue"), "package p\ny: \"ok\"\n").await?;
    let dir_arg = dir.to_string_lossy().into_owned();
    let output = run(&["export", "--out", "json", &dir_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"x\": 1"));
    assert!(stdout.contains("\"y\": \"ok\""));
    Ok(())
}

#[tokio::test]
async fn test_should_vet_json_data() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("schema.cue");
    let data = dir.join("data.json");
    fs::write(&cue, "x: number\ny: string\n").await?;
    fs::write(&data, "{\"x\":1,\"y\":\"ok\"}\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let data_arg = data.to_string_lossy().into_owned();
    let output = run(&["vet", &cue_arg, "--data", &data_arg]).await?;
    assert!(output.status.success());
    Ok(())
}

#[tokio::test]
async fn test_should_fail_vet_on_conflicting_json_data() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("schema.cue");
    let data = dir.join("data.json");
    fs::write(&cue, "x: 1\n").await?;
    fs::write(&data, "{\"x\":2}\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let data_arg = data.to_string_lossy().into_owned();
    let output = run(&["vet", &cue_arg, "--data", &data_arg]).await?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("conflicting values"));
    Ok(())
}

#[tokio::test]
async fn test_should_eval_cue_from_stdin() -> Result<(), Box<dyn Error>> {
    let output = run_with_stdin(&["eval", "-"], "x: 1\ny: x + 2\n").await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("y: 3"));
    Ok(())
}

#[tokio::test]
async fn test_should_inject_top_level_tag_value() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("tagged.cue");
    fs::write(&cue, "name: string @tag(env)\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["export", "--out", "json", "--inject", "env=prod", &cue_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"name\": \"prod\""));
    Ok(())
}

#[tokio::test]
async fn test_should_inject_nested_tag_value() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("tagged.cue");
    fs::write(&cue, "cfg: { name: string @tag(env) }\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["export", "--out", "json", "--inject", "env=prod", &cue_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"name\": \"prod\""));
    Ok(())
}

#[tokio::test]
async fn test_should_inject_multiline_nested_tag_value() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("tagged.cue");
    fs::write(
        &cue,
        "cfg: {\n  name: string @tag(env)\n}\n// ignored: string @tag(env)\n",
    )
    .await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["export", "--out", "json", "--inject", "env=prod", &cue_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"name\": \"prod\""));
    assert!(!stdout.contains("ignored"));
    Ok(())
}

#[tokio::test]
async fn test_should_ignore_comment_markers_and_braces_inside_tagged_line_strings()
-> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("tagged.cue");
    fs::write(
        &cue,
        "cfg: {\n  marker: \"}\"\n  url: string | *\"https://example.com\" @tag(env)\n}\n/* \
         blocked: string @tag(env) */\n",
    )
    .await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&[
        "export",
        "--out",
        "json",
        "--inject",
        "env=https://prod.example",
        &cue_arg,
    ])
    .await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"url\": \"https://prod.example\""));
    assert!(!stdout.contains("blocked"));
    Ok(())
}

#[tokio::test]
async fn test_should_select_requested_package_only() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    fs::write(dir.join("a.cue"), "package p\nx: 1\n").await?;
    fs::write(dir.join("b.cue"), "package q\ny: 2\n").await?;
    let dir_arg = dir.to_string_lossy().into_owned();
    let output = run(&["--package", "p", "export", "--out", "json", &dir_arg]).await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"x\": 1"));
    assert!(!stdout.contains("\"y\": 2"));
    Ok(())
}

#[tokio::test]
async fn test_should_inject_tag_with_named_package_selection() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    fs::write(dir.join("a.cue"), "package p\nname: string @tag(env)\n").await?;
    fs::write(dir.join("b.cue"), "package q\ny: string @tag(env)\n").await?;
    let dir_arg = dir.to_string_lossy().into_owned();
    let output = run(&[
        "--package",
        "p",
        "--inject",
        "env=prod",
        "export",
        "--out",
        "json",
        &dir_arg,
    ])
    .await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"name\": \"prod\""));
    assert!(!stdout.contains("\"y\": \"prod\""));
    Ok(())
}

#[tokio::test]
async fn test_should_apply_source_limit_to_parse() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("large.cue");
    fs::write(&cue, "x: 12345\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&["--source-limit", "4", "parse", &cue_arg]).await?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("maximum size"));
    Ok(())
}

#[tokio::test]
async fn test_should_apply_source_limit_to_stdin() -> Result<(), Box<dyn Error>> {
    let output = run_with_stdin(&["--source-limit", "4", "parse", "-"], "x: 12345\n").await?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("stdin source exceeds maximum size"));
    Ok(())
}

#[tokio::test]
async fn test_should_vet_json_data_from_stdin() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("schema.cue");
    fs::write(&cue, "x: number\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run_with_stdin(
        &["vet", &cue_arg, "--data", "-", "--data-format", "json"],
        "{\"x\":1}\n",
    )
    .await?;
    assert!(output.status.success());
    Ok(())
}

#[tokio::test]
async fn test_should_export_qualified_json_from_stdin() -> Result<(), Box<dyn Error>> {
    let output = run_with_stdin(
        &["export", "--out", "json", "json:-"],
        "{\"x\":1,\"nested\":{\"ok\":true}}\n",
    )
    .await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"nested\""));
    Ok(())
}

#[tokio::test]
async fn test_should_vet_positional_qualified_data_file() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("schema.cue");
    let data = dir.join("data.json");
    fs::write(&cue, "x: number\n").await?;
    fs::write(&data, "{\"x\":1}\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let data_arg = format!("json:{}", data.to_string_lossy());
    let output = run(&["vet", &cue_arg, &data_arg]).await?;
    assert!(output.status.success());
    Ok(())
}

#[tokio::test]
async fn test_should_vet_positional_unqualified_data_file() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let cue = dir.join("schema.cue");
    let data = dir.join("data.yaml");
    fs::write(&cue, "x: number\n").await?;
    fs::write(&data, "x: 1\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let data_arg = data.to_string_lossy().into_owned();
    let output = run(&["vet", &cue_arg, &data_arg]).await?;
    assert!(output.status.success());
    Ok(())
}

#[tokio::test]
async fn test_should_reject_data_file_escaping_module_root() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let root = dir.join("root");
    let outside = dir.join("outside.json");
    fs::create_dir_all(&root).await?;
    let cue = root.join("schema.cue");
    fs::write(&cue, "x: number\n").await?;
    fs::write(&outside, "{\"x\":1}\n").await?;
    let root_arg = root.to_string_lossy().into_owned();
    let cue_arg = cue.to_string_lossy().into_owned();
    let escaping = root.join("..").join("outside.json");
    let escaping_arg = escaping.to_string_lossy().into_owned();
    let output = run(&[
        "--module-root",
        &root_arg,
        "vet",
        &cue_arg,
        "--data",
        &escaping_arg,
    ])
    .await?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("path traversal") || stderr.contains("escapes module root"));
    Ok(())
}

#[tokio::test]
async fn test_should_honor_module_root_flag_for_absolute_inputs() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let module_dir = dir.join("cue.mod");
    let nested = dir.join("nested");
    fs::create_dir_all(&module_dir).await?;
    fs::create_dir_all(&nested).await?;
    fs::write(
        module_dir.join("module.cue"),
        "module: \"example.com/test\"\n",
    )
    .await?;
    let cue = nested.join("value.cue");
    fs::write(&cue, "x: 1\n").await?;
    let root_arg = dir.to_string_lossy().into_owned();
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run(&[
        "--module-root",
        &root_arg,
        "export",
        "--out",
        "json",
        &cue_arg,
    ])
    .await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"x\": 1"));
    Ok(())
}

#[tokio::test]
async fn test_should_resolve_relative_module_root_from_process_cwd() -> Result<(), Box<dyn Error>> {
    let dir = fixture_dir().await?;
    let root = dir.join("root");
    let module_dir = root.join("cue.mod");
    let nested = root.join("nested");
    fs::create_dir_all(&module_dir).await?;
    fs::create_dir_all(&nested).await?;
    fs::write(
        module_dir.join("module.cue"),
        "module: \"example.com/test\"\n",
    )
    .await?;
    let cue = nested.join("value.cue");
    fs::write(&cue, "x: 1\n").await?;
    let cue_arg = cue.to_string_lossy().into_owned();
    let output = run_in_dir(
        &["--module-root", "root", "export", "--out", "json", &cue_arg],
        &dir,
    )
    .await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("\"x\": 1"));
    Ok(())
}
