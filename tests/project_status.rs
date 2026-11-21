//! End-to-end tests of `latex_handler project status`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::tempdir;

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_latex_handler"))
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn project(home: &Path) -> PathBuf {
    let project = home.join("thesis");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("build/doc")).unwrap();
    fs::create_dir_all(project.join("logs/doc")).unwrap();
    fs::create_dir_all(project.join("pdfs/doc")).unwrap();
    fs::write(project.join("src/main.tex"), "\\documentclass{article}").unwrap();
    fs::write(project.join("build/doc/main.aux"), "aux").unwrap();
    fs::write(project.join("logs/doc/main.log"), "log").unwrap();
    fs::write(project.join("pdfs/doc/main.pdf"), "pdf").unwrap();
    project
}

fn status_output(home: &Path, project: &Path) -> Output {
    run(home, &["project", "status", project.to_str().unwrap()])
}

#[test]
fn project_status_reports_build_and_clean_information() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = project(&home);

    let output = status_output(&home, &project);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("Documents: 1"), "{text}");
    assert!(text.contains("Generated PDFs: 2"), "{text}");
    assert!(text.contains("Build status:"), "{text}");
    assert!(text.contains("Generated files:"), "{text}");
    assert!(text.contains("Clean preview:"), "{text}");
    assert!(text.contains("Would remove build"), "{text}");
    assert!(text.contains("Would remove logs"), "{text}");
}

#[test]
fn project_status_works_without_generated_files() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = home.join("empty");
    fs::create_dir_all(project.join("src")).unwrap();

    let output = status_output(&home, &project);
    assert!(output.status.success());
    let text = stdout(&output);
    assert!(text.contains("Documents: 0"), "{text}");
}
