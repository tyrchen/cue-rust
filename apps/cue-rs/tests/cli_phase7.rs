//! Phase 7 CLI integration coverage.

use std::{
    error::Error,
    path::PathBuf,
    process::Output,
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::{fs, process::Command};

async fn fixture_dir() -> Result<PathBuf, Box<dyn Error>> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!("cue-rust-cli-{nanos}"));
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
    fs::write(&cue, "x: 1\ny: \"ok\"\n").await?;
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
