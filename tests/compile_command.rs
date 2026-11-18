//! End-to-end tests of `latex_handler compile` against a real `latexmk`.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::tempdir;

fn latexmk_available() -> bool {
    Command::new("latexmk")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run the binary with an isolated configuration directory.
fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_latex_handler"))
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .output()
        .unwrap()
}

fn document(body: &str) -> String {
    format!("\\documentclass{{article}}\\begin{{document}}{body}\\end{{document}}")
}

#[test]
fn compiles_discovered_documents_into_a_mirrored_build_tree() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src/modules")).unwrap();
    fs::write(project.join("src/main.tex"), document("Main")).unwrap();
    fs::write(
        project.join("src/modules/ex_main.tex"),
        document("Exercises"),
    )
    .unwrap();
    fs::write(project.join("src/modules/helper.tex"), "not a document").unwrap();

    let output = run(dir.path(), &["compile", project.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("2 succeeded, 0 failed."), "{stdout}");
    assert!(project.join("build/main/main.pdf").is_file());
    assert!(project.join("build/modules/ex_main/ex_main.pdf").is_file());
    assert!(
        !project.join("src/main.pdf").exists(),
        "src/ must stay clean"
    );
    assert!(
        !project.join("src/main.aux").exists(),
        "src/ must stay clean"
    );
}

#[test]
fn file_option_and_engine_override_select_one_document() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/main.tex"), document("Main")).unwrap();
    fs::write(project.join("src/other_main.tex"), document("Other")).unwrap();

    let output = run(
        dir.path(),
        &[
            "--verbose",
            "compile",
            project.to_str().unwrap(),
            "--file",
            "other_main.tex",
            "--engine",
            "xelatex",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("1 succeeded"), "{stdout}");
    assert!(
        stdout.contains("-pdfxe"),
        "verbose output should show the command: {stdout}"
    );
    assert!(project.join("build/other_main/other_main.pdf").is_file());
    assert!(!project.join("build/main").exists());
}

#[test]
fn a_tex_file_path_compiles_just_that_document() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("latex_handler.toml"),
        "[project]\nname = 'demo'\n",
    )
    .unwrap();
    fs::write(project.join("src/main.tex"), document("Main")).unwrap();
    fs::write(project.join("src/other_main.tex"), document("Other")).unwrap();

    let file = project.join("src/main.tex");
    let output = run(dir.path(), &["compile", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(project.join("build/main/main.pdf").is_file());
    assert!(!project.join("build/other_main").exists());
}

#[test]
fn partial_failure_is_reported_and_exits_nonzero_with_the_log_kept() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/good_main.tex"), document("Fine")).unwrap();
    fs::write(
        project.join("src/bad_main.tex"),
        document("\\undefinedcommand"),
    )
    .unwrap();

    let output = run(dir.path(), &["compile", project.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("1 succeeded, 1 failed."), "{stdout}");
    assert!(stdout.contains("Undefined control sequence"), "{stdout}");
    assert!(project.join("build/good_main/good_main.pdf").is_file());
    assert!(project.join("build/bad_main/bad_main.log").is_file());
}

#[test]
fn incremental_rebuild_is_not_forced() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/main.tex"), document("Main")).unwrap();

    assert!(
        run(dir.path(), &["compile", project.to_str().unwrap()])
            .status
            .success()
    );
    let second = run(
        dir.path(),
        &["--verbose", "compile", project.to_str().unwrap()],
    );
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(second.status.success());
    assert!(
        !stdout.contains(" -g "),
        "-g must only be passed with --force: {stdout}"
    );

    let forced = run(
        dir.path(),
        &["--verbose", "compile", project.to_str().unwrap(), "--force"],
    );
    assert!(String::from_utf8_lossy(&forced.stdout).contains(" -g "));
}

#[test]
fn theme_flag_is_rejected_until_it_is_implemented() {
    let dir = tempdir().unwrap();
    let output = run(dir.path(), &["compile", "--theme", "dark"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--theme"));
}
