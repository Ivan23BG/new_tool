use super::*;
use crate::config::Config;
use std::{ffi::OsString, fs, sync::Mutex};
use tempfile::tempdir;

/// Records commands and optionally simulates latexmk producing a PDF.
struct FakeRunner {
    exit_code: Option<i32>,
    write_pdf: bool,
    calls: Mutex<Vec<CommandSpec>>,
}

impl FakeRunner {
    fn new(exit_code: Option<i32>, write_pdf: bool) -> Self {
        Self {
            exit_code,
            write_pdf,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl CommandRunner for &FakeRunner {
    fn run(&self, spec: &CommandSpec) -> io::Result<CommandOutput> {
        self.calls.lock().unwrap().push(spec.clone());
        if self.write_pdf {
            let outdir = spec
                .args
                .iter()
                .find_map(|a| a.to_str()?.strip_prefix("-outdir=").map(PathBuf::from))
                .unwrap();
            fs::create_dir_all(&outdir)?;
            fs::write(outdir.join("main.pdf"), b"%PDF")?;
            fs::write(outdir.join("main.log"), b"log")?;
        }
        Ok(CommandOutput {
            exit_code: self.exit_code,
            stdout: "out".into(),
            stderr: String::new(),
        })
    }
}

struct MissingRunner;
impl CommandRunner for MissingRunner {
    fn run(&self, _: &CommandSpec) -> io::Result<CommandOutput> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }
}

fn settings() -> ResolvedCompilerConfig {
    Config::resolve_layers(&[]).compiler
}

fn args(spec: &CommandSpec) -> Vec<String> {
    spec.args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn project_with_document() -> (tempfile::TempDir, CompileRequest) {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.tex"), "\\documentclass{article}").unwrap();
    let request = CompileRequest::new(
        dir.path().join("src/main.tex"),
        dir.path().join("build/main"),
    );
    (dir, request)
}

#[test]
fn default_command_is_incremental_and_runs_in_document_directory() {
    let (dir, request) = project_with_document();
    let spec = Compiler::new(settings()).command_for(&request).unwrap();
    let args = args(&spec);

    assert_eq!(spec.program, "latexmk");
    assert_eq!(spec.working_dir, dir.path().join("src"));
    assert_eq!(args.first().unwrap(), "-pdf");
    assert_eq!(args.last().unwrap(), "main.tex");
    assert!(args.contains(&"-interaction=nonstopmode".to_string()));
    assert!(args.contains(&"-halt-on-error".to_string()));
    assert!(args.contains(&"-shell-escape".to_string()));
    assert!(args.contains(&"-synctex=1".to_string()));
    assert!(
        !args.contains(&"-g".to_string()),
        "-g defeats incremental builds"
    );
}

#[test]
fn engine_flags_match_latexmk() {
    let (_dir, mut request) = project_with_document();
    let compiler = Compiler::new(settings());
    for (engine, flag) in [
        (LatexEngine::PdfLatex, "-pdf"),
        (LatexEngine::XeLatex, "-pdfxe"),
        (LatexEngine::LuaLatex, "-pdflua"),
    ] {
        request.engine = Some(engine);
        assert_eq!(args(&compiler.command_for(&request).unwrap())[0], flag);
    }
}

#[test]
fn request_engine_overrides_configured_engine() {
    let (_dir, mut request) = project_with_document();
    let mut configured = settings();
    configured.engine = LatexEngine::XeLatex;
    let compiler = Compiler::new(configured);
    assert_eq!(args(&compiler.command_for(&request).unwrap())[0], "-pdfxe");
    request.engine = Some(LatexEngine::LuaLatex);
    assert_eq!(args(&compiler.command_for(&request).unwrap())[0], "-pdflua");
}

#[test]
fn force_adds_g_and_options_toggle_flags() {
    let (_dir, mut request) = project_with_document();
    request.force = true;
    let mut configured = settings();
    configured.shell_escape = false;
    configured.synctex = false;
    configured.halt_on_error = false;
    let args = args(&Compiler::new(configured).command_for(&request).unwrap());

    assert!(args.contains(&"-g".to_string()));
    assert!(args.contains(&"-no-shell-escape".to_string()));
    assert!(args.contains(&"-synctex=0".to_string()));
    assert!(!args.contains(&"-halt-on-error".to_string()));
}

#[test]
fn theme_is_injected_as_a_pretex_definition() {
    let (_dir, mut request) = project_with_document();
    request.theme = Some(ThemeVariant::Dark);
    let args = args(&Compiler::new(settings()).command_for(&request).unwrap());
    assert!(args.contains(&"-pretex=\\def\\lhtheme{dark}".to_string()));
    assert!(args.contains(&"-usepretex".to_string()));
}

#[test]
fn invalid_backend_and_interaction_are_rejected() {
    let (_dir, request) = project_with_document();
    let mut configured = settings();
    configured.backend = "tectonic".into();
    assert!(matches!(
        Compiler::new(configured).command_for(&request),
        Err(CompileError::UnsupportedBackend(name)) if name == "tectonic"
    ));
    let mut configured = settings();
    configured.interaction = "chatty".into();
    assert!(matches!(
        Compiler::new(configured).command_for(&request),
        Err(CompileError::InvalidInteraction(_))
    ));
}

#[test]
fn missing_document_is_an_error() {
    let request = CompileRequest::new("/nonexistent/main.tex", "/tmp/out");
    assert!(matches!(
        Compiler::new(settings()).compile(&request),
        Err(CompileError::DocumentNotFound(_))
    ));
}

#[test]
fn successful_run_reports_pdf_and_log() {
    let (_dir, request) = project_with_document();
    let runner = FakeRunner::new(Some(0), true);
    let report = Compiler::with_runner(settings(), &runner)
        .compile(&request)
        .unwrap();

    assert!(report.is_success());
    assert_eq!(
        report.pdf.as_deref(),
        Some(request.output_dir.join("main.pdf").as_path())
    );
    assert!(report.log.is_some());
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
}

#[test]
fn tex_failure_is_a_failed_report_not_an_error() {
    let (_dir, request) = project_with_document();
    let runner = FakeRunner::new(Some(12), true);
    let report = Compiler::with_runner(settings(), &runner)
        .compile(&request)
        .unwrap();

    assert_eq!(
        report.outcome,
        CompileOutcome::Failed {
            exit_code: Some(12)
        }
    );
    assert!(
        report.log.is_some(),
        "logs must survive failed compilations"
    );
}

#[test]
fn success_without_a_pdf_is_an_error() {
    let (_dir, request) = project_with_document();
    let runner = FakeRunner::new(Some(0), false);
    assert!(matches!(
        Compiler::with_runner(settings(), &runner).compile(&request),
        Err(CompileError::MissingOutput { .. })
    ));
}

#[test]
fn missing_latexmk_is_reported_clearly() {
    let (_dir, request) = project_with_document();
    let error = Compiler::with_runner(settings(), MissingRunner)
        .compile(&request)
        .unwrap_err();
    assert!(matches!(error, CompileError::BackendNotFound { ref program } if program == "latexmk"));
    assert!(error.to_string().contains("PATH"));
}

#[test]
fn command_display_quotes_arguments_with_spaces() {
    let spec = CommandSpec {
        program: "latexmk".into(),
        args: vec![OsString::from("-pdf"), OsString::from("my doc.tex")],
        working_dir: PathBuf::from("."),
        env: Vec::new(),
    };
    assert_eq!(spec.display(), "latexmk -pdf 'my doc.tex'");
}
