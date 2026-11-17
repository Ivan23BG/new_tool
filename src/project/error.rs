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
    #[error("project directory already exists: '{0}'")]
    AlreadyExists(PathBuf),
    #[error("invalid project name '{0}': use a single directory name")]
    InvalidProjectName(String),
    #[error("invalid template name '{0}': use a single directory name")]
    InvalidTemplateName(String),
    #[error("no template selected; pass --template or configure general.default_template")]
    NoTemplateSelected,
    #[error("template '{name}' is not installed at '{path}'")]
    TemplateNotInstalled { name: String, path: PathBuf },
    #[error("invalid project configuration: {0}")]
    InvalidProjectConfig(String),
}
