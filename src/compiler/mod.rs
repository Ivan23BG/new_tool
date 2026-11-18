//! LaTeX compilation through `latexmk`.
//!
//! This module is independent of the CLI: it takes resolved configuration and
//! a [`CompileRequest`], and returns a structured [`CompileReport`].
//!
//! ```no_run
//! use latex_handler::compiler::{CompileRequest, Compiler};
//! use latex_handler::config::Config;
//!
//! let settings = Config::resolve_layers(&[]).compiler;
//! let request = CompileRequest::new("src/main.tex", "build/main");
//! let report = Compiler::new(settings).compile(&request)?;
//! println!("{}", report.is_success());
//! # Ok::<(), latex_handler::compiler::CompileError>(())
//! ```

mod batch;
mod diagnostics;
mod documents;
mod error;
mod latexmk;
mod runner;

use crate::config::{LatexEngine, ResolvedCompilerConfig, ThemeVariant};
use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub use batch::{BatchReport, DocumentResult, compile_documents, worker_count};
pub use diagnostics::error_excerpt;
pub use documents::{compile_document, output_dir_for, select_documents};
pub use error::CompileError;
pub use latexmk::{THEME_MACRO, engine_name};
pub use runner::{CommandOutput, CommandRunner, CommandSpec, SystemRunner};

/// One document to compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileRequest {
    /// The TeX entry point.
    pub document: PathBuf,
    /// Directory receiving latexmk's intermediate files and the PDF.
    pub output_dir: PathBuf,
    /// Overrides the configured engine for this run.
    pub engine: Option<LatexEngine>,
    /// Force a full rebuild (`latexmk -g`) instead of incremental compilation.
    pub force: bool,
    /// Expose a theme to the document as `\lhtheme`.
    pub theme: Option<ThemeVariant>,
}

impl CompileRequest {
    pub fn new(document: impl Into<PathBuf>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            document: document.into(),
            output_dir: output_dir.into(),
            engine: None,
            force: false,
            theme: None,
        }
    }
}

/// Whether the LaTeX run succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileOutcome {
    Succeeded,
    /// latexmk ran but exited unsuccessfully. `exit_code` is `None` if the
    /// process was killed by a signal.
    Failed {
        exit_code: Option<i32>,
    },
}

/// Structured result of compiling one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileReport {
    pub document: PathBuf,
    pub engine: LatexEngine,
    pub outcome: CompileOutcome,
    pub duration: Duration,
    /// The PDF in `output_dir`, when one exists (it may be stale after a failure).
    pub pdf: Option<PathBuf>,
    /// The TeX `.log` in `output_dir`, when one exists.
    pub log: Option<PathBuf>,
    pub command: CommandSpec,
    pub stdout: String,
    pub stderr: String,
}

impl CompileReport {
    pub fn is_success(&self) -> bool {
        self.outcome == CompileOutcome::Succeeded
    }
}

/// Compiles documents with latexmk using resolved compiler settings.
#[derive(Debug, Clone)]
pub struct Compiler<R: CommandRunner = SystemRunner> {
    settings: ResolvedCompilerConfig,
    runner: R,
}

impl Compiler<SystemRunner> {
    pub fn new(settings: ResolvedCompilerConfig) -> Self {
        Self {
            settings,
            runner: SystemRunner,
        }
    }
}

impl<R: CommandRunner> Compiler<R> {
    pub fn with_runner(settings: ResolvedCompilerConfig, runner: R) -> Self {
        Self { settings, runner }
    }

    pub fn settings(&self) -> &ResolvedCompilerConfig {
        &self.settings
    }

    /// The command that [`compile`](Self::compile) would run, without running it.
    pub fn command_for(&self, request: &CompileRequest) -> Result<CommandSpec, CompileError> {
        latexmk::build_command(&self.settings, request)
    }

    /// Compile one document.
    ///
    /// Returns `Err` only when compilation could not be attempted or observed.
    /// A TeX failure is an `Ok` report with [`CompileOutcome::Failed`].
    pub fn compile(&self, request: &CompileRequest) -> Result<CompileReport, CompileError> {
        if !request.document.is_file() {
            return Err(CompileError::DocumentNotFound(request.document.clone()));
        }
        let command = self.command_for(request)?;
        let engine = request.engine.unwrap_or(self.settings.engine);

        let started = Instant::now();
        let output = self
            .runner
            .run(&command)
            .map_err(|source| match source.kind() {
                io::ErrorKind::NotFound => CompileError::BackendNotFound {
                    program: command.program.clone(),
                },
                _ => CompileError::Spawn {
                    program: command.program.clone(),
                    source,
                },
            })?;
        let duration = started.elapsed();

        let stem = request.document.file_stem().unwrap_or_default();
        let pdf = existing(request.output_dir.join(stem).with_extension("pdf"));
        let log = existing(request.output_dir.join(stem).with_extension("log"));

        let outcome = if output.exit_code == Some(0) {
            if pdf.is_none() {
                return Err(CompileError::MissingOutput {
                    document: request.document.clone(),
                    expected: request.output_dir.join(stem).with_extension("pdf"),
                });
            }
            CompileOutcome::Succeeded
        } else {
            CompileOutcome::Failed {
                exit_code: output.exit_code,
            }
        };

        Ok(CompileReport {
            document: request.document.clone(),
            engine,
            outcome,
            duration,
            pdf,
            log,
            command,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

fn existing(path: PathBuf) -> Option<PathBuf> {
    Path::is_file(&path).then_some(path)
}

#[cfg(test)]
mod tests;
