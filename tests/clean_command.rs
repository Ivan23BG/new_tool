//! End-to-end tests of `latex_handler clean`.

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

fn project(home: &Path, config: &str) -> PathBuf {
    let project = home.join("thesis");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("assets")).unwrap();
    fs::create_dir_all(project.join("build/doc")).unwrap();
    fs::create_dir_all(project.join("logs/doc")).unwrap();
    fs::create_dir_all(project.join("pdfs/doc")).unwrap();
    fs::write(project.join("src/main.tex"), "source").unwrap();
    fs::write(project.join("assets/logo.png"), "asset").unwrap();
    fs::write(project.join("build/doc/main.aux"), "aux").unwrap();
    fs::write(project.join("logs/doc/main.log"), "log").unwrap();
    fs::write(project.join("pdfs/doc/main.pdf"), "pdf").unwrap();
    fs::write(project.join("latex_handler.toml"), config).unwrap();
    project
}

fn clean(home: &Path, project: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["clean", project.to_str().unwrap()];
    args.extend_from_slice(extra);
    run(home, &args)
}

#[test]
fn default_clean_removes_build_and_logs_but_keeps_pdfs() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = project(&home, "");

    let output = clean(&home, &project, &[]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("Removed build"), "{text}");
    assert!(text.contains("Removed logs"), "{text}");
    assert!(!project.join("build").exists());
    assert!(!project.join("logs").exists());
    assert!(project.join("pdfs/doc/main.pdf").is_file());
    assert!(project.join("src/main.tex").is_file());
    assert!(project.join("assets/logo.png").is_file());

    let output = clean(&home, &project, &[]);
    assert!(stdout(&output).contains("Nothing to clean."));
}

#[test]
fn flags_select_individual_output_directories() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = project(&home, "");

    assert!(clean(&home, &project, &["--build"]).status.success());
    assert!(!project.join("build").exists());
    assert!(project.join("logs/doc/main.log").is_file());
    assert!(project.join("pdfs/doc/main.pdf").is_file());

    assert!(clean(&home, &project, &["--logs"]).status.success());
    assert!(!project.join("logs").exists());
    assert!(project.join("pdfs/doc/main.pdf").is_file());

    assert!(clean(&home, &project, &["--all"]).status.success());
    assert!(!project.join("pdfs").exists());
    assert!(project.join("src/main.tex").is_file());
    assert!(project.join("assets/logo.png").is_file());
}

#[test]
fn clean_refuses_to_remove_the_source_directory() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = project(&home, "[structure]\nbuild = 'src'\n");

    let output = clean(&home, &project, &["--build"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("refusing to clean build directory"),
        "{}",
        stderr(&output)
    );
    assert!(project.join("src/main.tex").is_file());
}

#[test]
fn clean_dry_run_lists_would_remove_without_deleting() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = project(&home, "");

    let output = clean(&home, &project, &["--dry-run"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("Would remove build"), "{text}");
    assert!(text.contains("Would remove logs"), "{text}");
    assert!(text.contains("Dry run: no files were changed."), "{text}");
    // Dry run is read-only: nothing was deleted.
    assert!(project.join("build").exists());
    assert!(project.join("logs/doc/main.log").is_file());
    assert!(project.join("pdfs/doc/main.pdf").is_file());

    let output = clean(&home, &project, &["--dry-run", "--all"]);
    assert!(output.status.success());
    let text = stdout(&output);
    assert!(text.contains("Would remove pdfs"), "{text}");
    // Even with --all, the dry run must not delete anything.
    assert!(project.join("pdfs/doc/main.pdf").is_file());
}
