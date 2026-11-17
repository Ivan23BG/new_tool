//! Project root detection, configuration loading, document discovery, and validation.

mod creation;
mod discovery;
mod error;
mod validation;

use crate::config::{ConfigStore, ResolvedConfig};
use std::path::{Path, PathBuf};

pub use creation::{create_project, initialize_project, set_template_version};
pub use discovery::{DiscoveredDocument, discover_documents};
pub use error::ProjectError;
pub use validation::{ProjectValidation, ValidationIssue, ValidationSeverity, validate_project};

/// A loaded LaTeX project and its effective configuration.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    config_path: PathBuf,
    config: ResolvedConfig,
}

impl Project {
    /// Load a project from a directory or a `.tex` file inside it.
    pub fn load(path: impl AsRef<Path>, store: &ConfigStore) -> Result<Self, ProjectError> {
        let supplied = path.as_ref();
        let path = supplied
            .canonicalize()
            .map_err(|source| ProjectError::Path {
                path: supplied.to_path_buf(),
                source,
            })?;
        let root = if path.is_file() {
            if path.extension().is_some_and(|ext| ext == "tex") {
                path.parent().unwrap_or(Path::new(".")).to_path_buf()
            } else {
                return Err(ProjectError::NotProject(path));
            }
        } else if path.is_dir() {
            path
        } else {
            return Err(ProjectError::NotProject(path));
        };

        let config_path = ConfigStore::project_config_path(&root);
        let config = store.load_resolved(&root, &crate::config::Config::default())?;
        Ok(Self {
            root,
            config_path,
            config,
        })
    }

    /// Find the nearest project root by walking from `start` towards `/`.
    /// A project marker (`latex_handler.toml`) takes precedence over a `src/` directory.
    pub fn discover(start: impl AsRef<Path>, store: &ConfigStore) -> Result<Self, ProjectError> {
        let start = start
            .as_ref()
            .canonicalize()
            .map_err(|source| ProjectError::Path {
                path: start.as_ref().to_path_buf(),
                source,
            })?;
        let directory = if start.is_file() {
            start.parent().unwrap_or(Path::new("/"))
        } else {
            &start
        };
        let mut fallback = None;
        for candidate in directory.ancestors() {
            if ConfigStore::project_config_path(candidate).is_file() {
                return Self::load(candidate, store);
            }
            if candidate.join("src").is_dir() && fallback.is_none() {
                fallback = Some(candidate.to_path_buf());
            }
        }
        Self::load(fallback.unwrap_or_else(|| directory.to_path_buf()), store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
    pub fn config(&self) -> &ResolvedConfig {
        &self.config
    }

    /// Resolve the configured source directory, falling back to the root when absent.
    pub fn source_dir(&self) -> PathBuf {
        let configured = self.root.join(&self.config.structure.source);
        if configured.is_dir() {
            configured
        } else {
            self.root.clone()
        }
    }

    pub fn documents(&self) -> Result<Vec<DiscoveredDocument>, ProjectError> {
        discover_documents(self)
    }

    pub fn validate(&self) -> ProjectValidation {
        validate_project(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn loads_project_and_resolves_defaults() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        let project =
            Project::load(dir.path(), &ConfigStore::new(dir.path().join("global"))).unwrap();
        assert_eq!(project.config().structure.source, PathBuf::from("src"));
        assert_eq!(project.source_dir(), dir.path().join("src"));
    }

    #[test]
    fn discovers_documents_and_respects_excluded_directories() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("src");
        fs::create_dir_all(source.join("chapters")).unwrap();
        fs::create_dir_all(source.join("assets")).unwrap();
        fs::write(source.join("main.tex"), "\\documentclass{article}").unwrap();
        fs::write(source.join("chapters/one_main.tex"), "").unwrap();
        fs::write(source.join("assets/ignored_main.tex"), "").unwrap();
        let project =
            Project::load(dir.path(), &ConfigStore::new(dir.path().join("global"))).unwrap();
        let documents = project.documents().unwrap();
        assert_eq!(documents.len(), 2);
        assert_eq!(
            documents[0].relative_path,
            PathBuf::from("chapters/one_main.tex")
        );
        assert_eq!(documents[1].relative_path, PathBuf::from("main.tex"));
    }

    #[test]
    fn discovers_nearest_configured_ancestor() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("src").join("chapter");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            dir.path().join("latex_handler.toml"),
            "[project]\nname = 'demo'\n",
        )
        .unwrap();
        let project =
            Project::discover(&nested, &ConfigStore::new(dir.path().join("global"))).unwrap();
        assert_eq!(project.root(), dir.path());
    }
}
