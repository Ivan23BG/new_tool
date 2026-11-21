//! End-to-end tests of `latex_handler project git`.

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

fn git(args: &[&str], cwd: &Path) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn repository(home: &Path) -> PathBuf {
    let project = home.join("thesis");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/main.tex"), "one\n").unwrap();
    let init = git(&["init", "--quiet", "-b", "main"], &project);
    assert!(init.status.success());
    let add = git(&["add", "."], &project);
    assert!(add.status.success());
    let commit = git(
        &[
            "-c",
            "user.name=tests",
            "-c",
            "user.email=tests@example.invalid",
            "commit",
            "-q",
            "-m",
            "init",
        ],
        &project,
    );
    assert!(commit.status.success());
    project
}

#[test]
fn project_git_status_reports_changes_in_a_repository() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = repository(&home);

    fs::write(project.join("src/main.tex"), "changed\n").unwrap();
    fs::write(project.join("src_untracked.tex"), "new\n").unwrap();

    let output = run(
        &home,
        &["project", "git", "status", project.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("Git repository: true"), "{text}");
    assert!(text.contains("Branch: main"), "{text}");
    assert!(text.contains("Clean: false"), "{text}");
    assert!(text.contains("modified"), "{text}");
    assert!(text.contains("src/main.tex"), "{text}");
    assert!(text.contains("untracked"), "{text}");
}

#[test]
fn project_git_status_outside_a_repository_reports_no_repository() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let project = home.join("plain");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/main.tex"), "source\n").unwrap();

    let output = run(
        &home,
        &["project", "git", "status", project.to_str().unwrap()],
    );
    assert!(output.status.success());
    assert!(
        stdout(&output).contains("Git repository: false"),
        "{}",
        stdout(&output)
    );
}
