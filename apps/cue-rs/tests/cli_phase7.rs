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

async fn run(args: &[&str]) -> Result<Output, Box<dyn Error>> {
    let binary = std::env::var("CARGO_BIN_EXE_cue-rs")?;
    Ok(Command::new(binary).args(args).output().await?)
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
