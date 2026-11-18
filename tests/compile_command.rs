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

fn project_with_documents(root: &Path, good: usize, bad: usize) -> std::path::PathBuf {
    let project = root.join("project");
    fs::create_dir_all(project.join("src/modules")).unwrap();
    for n in 0..good {
        let dir = if n % 2 == 0 { "src" } else { "src/modules" };
        fs::write(
            project.join(dir).join(format!("doc{n}_main.tex")),
            document("Fine"),
        )
        .unwrap();
    }
    for n in 0..bad {
        fs::write(
            project.join(format!("src/bad{n}_main.tex")),
            document("\\undefinedcommand"),
        )
        .unwrap();
    }
    project
}

#[test]
fn parallel_batch_reports_every_document_and_keeps_blocks_intact() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = project_with_documents(dir.path(), 5, 1);

    let output = run(dir.path(), &["compile", project.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("up to 6 in parallel"), "{stdout}");
    assert!(stdout.contains("5 succeeded, 1 failed."), "{stdout}");
    for n in 0..5 {
        assert_eq!(
            stdout.matches(&format!("doc{n}_main.tex  SUCCESS")).count(),
            1,
            "{stdout}"
        );
    }
    // The failure block must not be interleaved with other documents' lines.
    let failed = stdout
        .split("bad0_main.tex  FAILED")
        .nth(1)
        .expect("failure line");
    let block: Vec<&str> = failed.lines().skip(1).take(3).collect();
    assert!(block[0].contains("Undefined control sequence"), "{block:?}");
    assert!(block[1].trim_start().starts_with("l."), "{block:?}");
    assert!(
        block[2].contains("full log: logs/bad0_main.log"),
        "{block:?}"
    );
    assert!(
        project
            .join("build/modules/doc1_main/doc1_main.pdf")
            .is_file()
    );
}

#[test]
fn parallelism_can_be_turned_off_in_project_configuration() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = project_with_documents(dir.path(), 3, 0);
    fs::write(
        project.join("latex_handler.toml"),
        "[compiler]\nparallel = false\n",
    )
    .unwrap();

    let output = run(dir.path(), &["compile", project.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(!stdout.contains("in parallel"), "{stdout}");
    assert!(stdout.contains("3 succeeded, 0 failed."), "{stdout}");
}

#[test]
fn colliding_flat_output_directories_are_refused() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src/one")).unwrap();
    fs::create_dir_all(project.join("src/two")).unwrap();
    fs::write(project.join("src/one/x_main.tex"), document("1")).unwrap();
    fs::write(project.join("src/two/x_main.tex"), document("2")).unwrap();
    fs::write(
        project.join("latex_handler.toml"),
        "[output]\nmirror_structure = false\n",
    )
    .unwrap();

    let output = run(dir.path(), &["compile", project.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("share the output directory"));
    assert!(
        !project.join("build").exists(),
        "nothing may be built after a collision"
    );
}

fn pdf_text(pdf: &Path) -> Option<String> {
    let output = Command::new("pdftotext").arg(pdf).arg("-").output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn themed_document() -> String {
    "\\documentclass{article}\\providecommand{\\lhtheme}{light}\\begin{document}Theme:\\lhtheme\\end{document}".to_string()
}

#[test]
fn successful_builds_are_published_to_pdfs_and_logs() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = project_with_documents(dir.path(), 2, 0);

    let output = run(dir.path(), &["compile", project.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(
        stdout.contains("SUCCESS") && stdout.contains("pdfs/doc0_main.pdf"),
        "{stdout}"
    );
    assert!(project.join("pdfs/doc0_main.pdf").is_file());
    assert!(project.join("pdfs/modules/doc1_main.pdf").is_file());
    assert!(project.join("pdfs/modules/doc1_main.synctex.gz").is_file());
    assert!(project.join("logs/doc0_main.log").is_file());
    assert!(project.join("logs/modules/doc1_main.log").is_file());
    assert!(
        !project.join("src/doc0_main.pdf").exists(),
        "src/ must stay clean"
    );
}

#[test]
fn a_failed_rebuild_keeps_the_last_good_pdf_and_refreshes_the_log() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    let source = project.join("src/main.tex");
    fs::write(&source, document("Good")).unwrap();
    assert!(
        run(dir.path(), &["compile", project.to_str().unwrap()])
            .status
            .success()
    );
    let good_pdf = fs::read(project.join("pdfs/main.pdf")).unwrap();

    fs::write(&source, document("\\undefinedcommand")).unwrap();
    let output = run(dir.path(), &["compile", project.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read(project.join("pdfs/main.pdf")).unwrap(), good_pdf);
    let log = fs::read_to_string(project.join("logs/main.log")).unwrap();
    assert!(
        log.contains("Undefined control sequence"),
        "log should describe the new failure"
    );
}

#[test]
fn theme_all_builds_light_and_dark_with_distinct_outputs() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/main.tex"), themed_document()).unwrap();

    let output = run(
        dir.path(),
        &["compile", project.to_str().unwrap(), "--theme", "all"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("1 documents x 2 themes"), "{stdout}");
    assert!(stdout.contains("main.tex [dark]"), "{stdout}");
    assert!(project.join("build/main/main.pdf").is_file());
    assert!(project.join("build/main-dark/main.pdf").is_file());
    if let (Some(light), Some(dark)) = (
        pdf_text(&project.join("pdfs/main.pdf")),
        pdf_text(&project.join("pdfs/main-dark.pdf")),
    ) {
        assert!(light.contains("Theme:light"), "{light}");
        assert!(dark.contains("Theme:dark"), "{dark}");
    }
    assert!(project.join("logs/main-dark.log").is_file());
}

#[test]
fn plain_compile_uses_the_configured_default_theme() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/main.tex"), themed_document()).unwrap();
    fs::write(
        project.join("latex_handler.toml"),
        "[themes]\ndefault = \"dark\"\n",
    )
    .unwrap();

    assert!(
        run(dir.path(), &["compile", project.to_str().unwrap()])
            .status
            .success()
    );
    if let Some(text) = pdf_text(&project.join("pdfs/main.pdf")) {
        assert!(text.contains("Theme:dark"), "{text}");
    }
}

#[test]
fn disabled_theme_is_rejected_with_the_enabled_list() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/main.tex"), document("x")).unwrap();
    fs::write(
        project.join("latex_handler.toml"),
        "[themes]\nenabled = [\"light\"]\n",
    )
    .unwrap();

    let output = run(
        dir.path(),
        &["compile", project.to_str().unwrap(), "--theme", "dark"],
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("theme 'dark' is not enabled") && stderr.contains("light"),
        "{stderr}"
    );
    assert!(!project.join("build").exists());
}
