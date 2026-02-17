use assert_cmd::Command;
use insta::assert_snapshot;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::{contains, is_empty};
use std::fs;
use tempfile::tempdir;

#[test]
fn eval_prints_result() {
    Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .args(["eval", "1+2+3"])
        .assert()
        .success()
        .stdout(contains("6"));
}

#[test]
fn run_js_file_works_without_llm() {
    let dir = tempdir().expect("tempdir should work");
    let path = dir.path().join("hello.js");
    fs::write(&path, "40 + 2").expect("write should work");

    Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .args(["run", path.to_str().expect("path utf8")])
        .assert()
        .success()
        .stdout(contains("42"));
}

#[test]
fn llm_path_without_api_key_fails_cleanly() {
    let dir = tempdir().expect("tempdir should work");
    let path = dir.path().join("needs-llm.pseudo");
    fs::write(&path, "write hello").expect("write should work");

    Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .env_remove("OPENAI_API_KEY")
        .args([
            "run",
            path.to_str().expect("path utf8"),
            "--provider",
            "openai",
            "--force-llm",
        ])
        .assert()
        .failure()
        .stderr(contains("OPENAI_API_KEY is required"));
}

#[test]
fn js_run_has_no_progress_output_by_default() {
    let dir = tempdir().expect("tempdir should work");
    let path = dir.path().join("hello.js");
    fs::write(&path, "2 + 3").expect("write should work");

    Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .args(["run", path.to_str().expect("path utf8")])
        .assert()
        .success()
        .stdout(contains("5"))
        .stderr(is_empty());
}

#[test]
fn config_file_applies_defaults() {
    let dir = tempdir().expect("tempdir should work");
    let source = dir.path().join("hello.pseudo");
    let config = dir.path().join("beeno.json");

    fs::write(&source, "write hello").expect("write should work");
    fs::write(&config, r#"{"provider":"openai","force_llm":true}"#).expect("write should work");

    Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .env_remove("OPENAI_API_KEY")
        .args(["run", source.to_str().expect("path utf8")])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(contains("OPENAI_API_KEY is required"));
}

#[test]
fn cli_overrides_config_provider() {
    let dir = tempdir().expect("tempdir should work");
    let source = dir.path().join("hello.pseudo");
    let config = dir.path().join("beeno.json");

    fs::write(&source, "write hello").expect("write should work");
    fs::write(&config, r#"{"provider":"openai","force_llm":true}"#).expect("write should work");

    Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .env_remove("OPENAI_API_KEY")
        .args([
            "run",
            source.to_str().expect("path utf8"),
            "--provider",
            "ollama",
            "--ollama-url",
            "http://127.0.0.1:1",
        ])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(contains("failed calling Ollama"));
}

#[test]
fn no_progress_suppresses_status_lines() {
    let dir = tempdir().expect("tempdir should work");
    let source = dir.path().join("hello.pseudo");

    fs::write(&source, "write hello").expect("write should work");

    Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .env_remove("OPENAI_API_KEY")
        .args([
            "run",
            source.to_str().expect("path utf8"),
            "--provider",
            "openai",
            "--force-llm",
            "--no-progress",
        ])
        .assert()
        .failure()
        .stderr(contains("[beeno]").not());
}

#[test]
fn snapshot_config_parse_error_stderr() {
    let dir = tempdir().expect("tempdir should work");
    let source = dir.path().join("hello.pseudo");
    let config = dir.path().join("beeno.json");

    fs::write(&source, "write hello").expect("write should work");
    fs::write(&config, "{\n  \"provider\":\n").expect("write should work");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .args(["run", source.to_str().expect("path utf8")])
        .current_dir(dir.path())
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr)
        .replace(config.to_str().expect("path utf8"), "<TMP>/beeno.json");
    assert_snapshot!("config_parse_error_stderr", stderr);
}

#[test]
fn snapshot_llm_failure_stderr() {
    let dir = tempdir().expect("tempdir should work");
    let source = dir.path().join("hello.pseudo");
    fs::write(&source, "write hello").expect("write should work");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("beeno"))
        .env_remove("OPENAI_API_KEY")
        .args([
            "run",
            source.to_str().expect("path utf8"),
            "--provider",
            "openai",
            "--force-llm",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr)
        .replace(source.to_str().expect("path utf8"), "<TMP>/hello.pseudo");
    assert_snapshot!("llm_failure_stderr", stderr);
}
