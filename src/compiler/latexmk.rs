//! Construction of `latexmk` command lines.

use super::{CommandSpec, CompileError, CompileRequest};
use crate::config::{LatexEngine, ResolvedCompilerConfig, ThemeVariant};
use std::{ffi::OsString, path::Path};

/// Name of the TeX macro through which the selected theme reaches a document.
///
/// Templates should declare a fallback with
/// `\providecommand{\lhtheme}{light}` and branch on it.
pub const THEME_MACRO: &str = "lhtheme";

const INTERACTION_MODES: [&str; 4] = ["batchmode", "nonstopmode", "scrollmode", "errorstopmode"];

pub(super) fn engine_flag(engine: LatexEngine) -> &'static str {
    match engine {
        LatexEngine::PdfLatex => "-pdf",
        LatexEngine::XeLatex => "-pdfxe",
        LatexEngine::LuaLatex => "-pdflua",
    }
}

/// The engine's conventional lowercase name, as used in configuration files.
pub fn engine_name(engine: LatexEngine) -> &'static str {
    match engine {
        LatexEngine::PdfLatex => "pdflatex",
        LatexEngine::XeLatex => "xelatex",
        LatexEngine::LuaLatex => "lualatex",
    }
}

pub fn theme_name(theme: ThemeVariant) -> &'static str {
    match theme {
        ThemeVariant::Light => "light",
        ThemeVariant::Dark => "dark",
    }
}

/// Build the `latexmk` invocation for one document.
///
/// The working directory is the document's own directory so relative
/// `\input`/`\includegraphics` paths behave as authors expect. `-g` is added
/// only when `force` is requested, preserving latexmk's incremental builds.
pub(super) fn build_command(
    settings: &ResolvedCompilerConfig,
    request: &CompileRequest,
) -> Result<CommandSpec, CompileError> {
    if settings.backend != "latexmk" {
        return Err(CompileError::UnsupportedBackend(settings.backend.clone()));
    }
    if !INTERACTION_MODES.contains(&settings.interaction.as_str()) {
        return Err(CompileError::InvalidInteraction(
            settings.interaction.clone(),
        ));
    }

    let engine = request.engine.unwrap_or(settings.engine);
    let document = request.document.as_path();
    let working_dir = document
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let file_name = document
        .file_name()
        .ok_or_else(|| CompileError::DocumentNotFound(request.document.clone()))?;

    let mut args: Vec<OsString> = vec![
        engine_flag(engine).into(),
        format!("-interaction={}", settings.interaction).into(),
        "-file-line-error".into(),
    ];
    if settings.halt_on_error {
        args.push("-halt-on-error".into());
    }
    args.push(
        if settings.shell_escape {
            "-shell-escape"
        } else {
            "-no-shell-escape"
        }
        .into(),
    );
    args.push(format!("-synctex={}", u8::from(settings.synctex)).into());

    let mut outdir = OsString::from("-outdir=");
    outdir.push(&request.output_dir);
    args.push(outdir);

    if let Some(theme) = request.theme {
        args.push(format!("-pretex=\\def\\{THEME_MACRO}{{{}}}", theme_name(theme)).into());
        args.push("-usepretex".into());
    }
    if request.force {
        args.push("-g".into());
    }
    args.push(file_name.to_os_string());

    Ok(CommandSpec {
        program: "latexmk".into(),
        args,
        working_dir,
        env: Vec::new(),
    })
}
