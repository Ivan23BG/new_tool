use crate::project::ProjectError;
use std::{io, path::PathBuf};
use thiserror::Error;

/// Failures that prevent a compilation from being attempted or observed.
///
/// A LaTeX run that *executes* but exits unsuccessfully (for example because
/// of a TeX error) is **not** a `CompileError`: it is reported as
/// [`CompileOutcome::Failed`](super::CompileOutcome::Failed) so callers can
/// collect per-document results.
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("unsupported compiler backend '{0}': only 'latexmk' is supported")]
    UnsupportedBackend(String),
    #[error(
        "invalid compiler.interaction '{0}': expected one of batchmode, nonstopmode, scrollmode, errorstopmode"
    )]
    InvalidInteraction(String),
    #[error("no LaTeX documents found to compile; pass --file or check discovery settings")]
    NoDocuments,
    #[error("'{document}' is outside the project's source directory '{source_dir}'")]
    DocumentOutsideSource {
        document: PathBuf,
        source_dir: PathBuf,
    },
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("LaTeX document not found: '{0}'")]
    DocumentNotFound(PathBuf),
    #[error(
        "could not find '{program}' on PATH; install it (for example via TeX Live) and try again"
    )]
    BackendNotFound { program: String },
    #[error("failed to run '{program}': {source}")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("compilation of '{document}' reported success but '{expected}' was not produced")]
    MissingOutput {
        document: PathBuf,
        expected: PathBuf,
    },
}
