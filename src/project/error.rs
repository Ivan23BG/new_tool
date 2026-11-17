use crate::config::ConfigError;
use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("could not access project path '{path}': {source}")]
    Path {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("'{0}' is not a project directory or TeX document")]
    NotProject(PathBuf),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("failed to inspect project files: {0}")]
    Walk(#[from] walkdir::Error),
}
