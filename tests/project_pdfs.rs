//! End-to-end tests of `latex_handler project pdfs`.

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
    fs::create_dir_all(project.join("pdfs")).unwrap();
    fs::write(project.join("src/main.tex"), "\\documentclass{article}").unwrap();
    project
}

fn list_output(home: &Path, project: &Path) -> Output {
    run(home, &["project", "pdfs", project.to_str().unwrap()])
}

#[test]
fn project_pdfs_lists_generated_pdfs_with_status() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = project(&home);

    // The source is written before the PDF, so the default-theme PDF is up to date.
    fs::write(project.join("pdfs/main.pdf"), b"pdf").unwrap();

    let output = list_output(&home, &project);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("Generated PDFs: 2"), "{text}");
    assert!(text.contains("pdfs/main.pdf"), "{text}");
    assert!(text.contains("pdfs/main-dark.pdf"), "{text}");
    assert!(text.contains("up-to-date"), "{text}");
}

#[test]
fn project_pdfs_open_requires_an_existing_pdf() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = project(&home);

    let output = run(
        &home,
        &[
            "project",
            "pdfs",
            project.to_str().unwrap(),
            "--open",
            "src/main.tex",
        ],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no generated PDF"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn project_pdfs_without_documents_reports_no_pdfs() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = home.join("empty");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("pdfs")).unwrap();

    let output = list_output(&home, &project);
    assert!(output.status.success());
    assert!(
        stdout(&output).contains("No generated PDFs."),
        "{}",
        stdout(&output)
    );
}
