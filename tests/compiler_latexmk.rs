//! End-to-end tests against a real `latexmk`. Skipped when it is not installed.

use latex_handler::compiler::{
    CompileJob, CompileOutcome, CompileRequest, Compiler, compile_document, select_documents,
};
use latex_handler::config::{Config, ConfigStore, LatexEngine, ThemeVariant};
use latex_handler::project::create_project;
use std::{fs, process::Command};
use tempfile::tempdir;

fn latexmk_available() -> bool {
    Command::new("latexmk")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn compiler() -> Compiler {
    Compiler::new(Config::resolve_layers(&[]).compiler)
}

#[test]
fn compiles_a_document_with_every_engine() {
    if !latexmk_available() {
        eprintln!("skipping: latexmk not installed");
        return;
    }
    let dir = tempdir().unwrap();
    let source = dir.path().join("main.tex");
    fs::write(
        &source,
        "\\documentclass{article}\\begin{document}Hello\\end{document}",
    )
    .unwrap();

    for engine in [
        LatexEngine::PdfLatex,
        LatexEngine::XeLatex,
        LatexEngine::LuaLatex,
    ] {
        let mut request =
            CompileRequest::new(&source, dir.path().join(format!("build-{engine:?}")));
        request.engine = Some(engine);
        let report = compiler().compile(&request).unwrap();
        assert!(report.is_success(), "{engine:?} failed:\n{}", report.stdout);
        assert!(report.pdf.unwrap().is_file());
    }
}

#[test]
fn theme_reaches_the_document() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let source = dir.path().join("main.tex");
    fs::write(
        &source,
        "\\documentclass{article}\\providecommand{\\lhtheme}{light}\\begin{document}T:\\lhtheme\\end{document}",
    )
    .unwrap();
    let mut request = CompileRequest::new(&source, dir.path().join("build"));
    request.theme = Some(ThemeVariant::Dark);
    let report = compiler().compile(&request).unwrap();
    assert!(report.is_success());
    let text = Command::new("pdftotext")
        .arg(report.pdf.unwrap())
        .arg("-")
        .output();
    if let Ok(text) = text {
        assert!(String::from_utf8_lossy(&text.stdout).contains("T:dark"));
    }
}

#[test]
fn shared_and_local_assets_are_available_during_compilation() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let store = ConfigStore::new(dir.path().join("config"));
    let template = store.templates_dir().join("university");
    fs::create_dir_all(template.join("src")).unwrap();
    fs::create_dir_all(template.join("shared")).unwrap();
    fs::create_dir_all(template.join("assets")).unwrap();
    fs::write(
        template.join("template.toml"),
        "name = 'university'\nversion = '1.0.0'\nshared_assets = 'shared'\n",
    )
    .unwrap();
    fs::write(
        template.join("src/main.tex"),
        "\\documentclass{article}\\begin{document}\\input{shared-note}\\end{document}",
    )
    .unwrap();
    fs::write(
        template.join("src/local_main.tex"),
        "\\documentclass{article}\\begin{document}\\input{local-note}\\end{document}",
    )
    .unwrap();
    fs::write(template.join("shared/shared-note.tex"), "Shared asset").unwrap();
    fs::write(template.join("assets/local-note.tex"), "Local asset").unwrap();

    let project = create_project("thesis", dir.path(), Some("university"), &store).unwrap();
    assert!(
        !project.root().join("shared").exists(),
        "shared assets must stay in the installed template"
    );
    assert!(project.root().join("assets/local-note.tex").is_file());

    let documents = select_documents(&project, &[], project.root()).unwrap();
    let compiler = Compiler::new(project.config().compiler.clone());
    for document in documents {
        let job = CompileJob::new(document);
        let report = compile_document(&project, &compiler, &job, false).unwrap();
        assert!(
            report.is_success(),
            "{} failed:\n{}",
            job.document.relative_path.display(),
            report.stdout
        );
    }
}

#[test]
fn tex_errors_produce_a_failed_report_and_keep_the_log() {
    if !latexmk_available() {
        return;
    }
    let dir = tempdir().unwrap();
    let source = dir.path().join("bad.tex");
    fs::write(
        &source,
        "\\documentclass{article}\\begin{document}\\undefinedcommand\\end{document}",
    )
    .unwrap();
    let request = CompileRequest::new(&source, dir.path().join("build"));
    let report = compiler().compile(&request).unwrap();
    assert!(matches!(report.outcome, CompileOutcome::Failed { .. }));
    assert!(report.log.is_some());
}
