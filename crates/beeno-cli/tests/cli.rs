use assert_cmd::Command;
use predicates::str::contains;
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
