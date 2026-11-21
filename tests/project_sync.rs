//! End-to-end tests of `latex_handler project sync` against an isolated
//! configuration directory and hand-built installed templates.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::tempdir;

/// Run the binary with an isolated user configuration directory.
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

fn setup_template(home: &Path, version: &str) -> PathBuf {
    let template = home.join("config/latex_handler/templates/university");
    fs::create_dir_all(template.join("src")).unwrap();
    fs::write(
        template.join("template.toml"),
        format!("name = 'university'\nversion = '{version}'\n"),
    )
    .unwrap();
    fs::write(template.join("src/main.tex"), "template one").unwrap();
    fs::write(template.join("src/chapter.tex"), "chapter").unwrap();
    template
}

fn create_project(home: &Path, parent: &Path) -> PathBuf {
    let output = run(
        home,
        &[
            "new",
            "thesis",
            "--template",
            "university",
            "--path",
            parent.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    parent.join("thesis")
}

fn sync(home: &Path, project: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["project", "sync", project.to_str().unwrap()];
    args.extend_from_slice(extra);
    run(home, &args)
}

#[test]
fn dry_run_is_read_only_and_apply_updates_untouched_files() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let template = setup_template(&home, "1.0.0");
    let project = create_project(&home, dir.path());

    fs::write(template.join("src/main.tex"), "template two").unwrap();
    fs::write(
        template.join("template.toml"),
        "name = 'university'\nversion = '1.1.0'\n",
    )
    .unwrap();

    let output = sync(&home, &project, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("update    src/main.tex"), "{text}");
    assert!(text.contains("1.1.0"), "{text}");
    assert_eq!(
        fs::read_to_string(project.join("src/main.tex")).unwrap(),
        "template one",
        "a dry run must not modify the project"
    );

    let output = sync(&home, &project, &["--apply"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(project.join("src/main.tex")).unwrap(),
        "template two"
    );
    let snapshot = fs::read_to_string(project.join("latex_handler.snapshot.toml")).unwrap();
    assert!(
        snapshot.contains("template_version = \"1.1.0\""),
        "{snapshot}"
    );

    let output = sync(&home, &project, &[]);
    let text = stdout(&output);
    assert!(text.contains("Project is up to date."), "{text}");
}

#[test]
fn conflicts_require_an_explicit_policy_and_never_overwrite_by_default() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let template = setup_template(&home, "1.0.0");
    let project = create_project(&home, dir.path());

    fs::write(project.join("src/main.tex"), "local edit").unwrap();
    fs::write(template.join("src/main.tex"), "template edit").unwrap();
    fs::write(
        template.join("template.toml"),
        "name = 'university'\nversion = '1.1.0'\n",
    )
    .unwrap();

    let output = sync(&home, &project, &[]);
    let text = stdout(&output);
    assert!(text.contains("conflict  src/main.tex"), "{text}");

    let output = sync(&home, &project, &["--apply"]);
    assert!(
        !output.status.success(),
        "unresolved conflicts must fail: {}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("conflict(s) remain"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(project.join("src/main.tex")).unwrap(),
        "local edit"
    );

    let output = sync(&home, &project, &["--apply", "--prefer", "template"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(project.join("src/main.tex")).unwrap(),
        "template edit"
    );
}

#[test]
fn prefer_local_keeps_the_user_file_on_subsequent_syncs() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let template = setup_template(&home, "1.0.0");
    let project = create_project(&home, dir.path());

    fs::write(project.join("src/main.tex"), "local edit").unwrap();
    fs::write(template.join("src/main.tex"), "template edit").unwrap();
    fs::write(
        template.join("template.toml"),
        "name = 'university'\nversion = '1.1.0'\n",
    )
    .unwrap();

    let output = sync(&home, &project, &["--apply", "--prefer", "local"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(project.join("src/main.tex")).unwrap(),
        "local edit"
    );

    // The baseline must now be the template content, so the local change is
    // reported as an ordinary modification and is never auto-overwritten.
    let output = sync(&home, &project, &["--apply"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("keep      src/main.tex"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        fs::read_to_string(project.join("src/main.tex")).unwrap(),
        "local edit"
    );
}

#[test]
fn unrelated_user_files_survive_and_template_removals_are_applied() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let template = setup_template(&home, "1.0.0");
    let project = create_project(&home, dir.path());

    fs::write(project.join("notes.txt"), "user notes").unwrap();
    fs::remove_file(template.join("src/chapter.tex")).unwrap();
    fs::write(template.join("src/new.tex"), "new chapter").unwrap();
    fs::write(
        template.join("template.toml"),
        "name = 'university'\nversion = '1.1.0'\n",
    )
    .unwrap();

    let output = sync(&home, &project, &["--apply"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(
        fs::read_to_string(project.join("notes.txt")).unwrap(),
        "user notes"
    );
    assert_eq!(
        fs::read_to_string(project.join("src/new.tex")).unwrap(),
        "new chapter"
    );
    assert!(
        !project.join("src/chapter.tex").exists(),
        "an untouched resource removed by the template should be removed"
    );
}
