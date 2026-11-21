//! Project root detection, configuration loading, document discovery, and validation.

mod clean;
mod creation;
mod discovery;
mod error;
mod git;
mod snapshot;
mod status;
mod sync;
mod validation;

use crate::config::{ConfigStore, ResolvedConfig};
use crate::template::{TEMPLATE_MANIFEST_FILE, TemplateManifest};
use std::path::{Path, PathBuf};

pub use clean::{CleanError, CleanSelection, clean_project, plan_clean};
pub use creation::{create_project, initialize_project, set_template_version};
pub use discovery::{DiscoveredDocument, discover_documents};
pub use error::ProjectError;
pub use git::{ChangeKind, GitChange, GitError, GitStatus, init, status};
pub use snapshot::{
    PROJECT_SNAPSHOT_FILE, ProjectSnapshot, ResourceRecord, ResourceState, SNAPSHOT_FORMAT_VERSION,
    SnapshotError, fingerprint_bytes, key_path, path_key,
};
pub use status::directory_bytes;
pub use sync::{
    ConflictKind, ConflictPolicy, KeepReason, SyncAction, SyncCounts, SyncError, SyncOutcome,
    SyncPlan, apply_sync, plan_sync,
};
pub use validation::{ProjectValidation, ValidationIssue, ValidationSeverity, validate_project};

/// A loaded LaTeX project and its effective configuration.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    config_path: PathBuf,
    config: ResolvedConfig,
    /// Shared-assets directory of the installed source template, when present.
    shared_assets: Option<PathBuf>,
}

impl Project {
    /// Load a project from a directory or a `.tex` file inside it.
    pub fn load(path: impl AsRef<Path>, store: &ConfigStore) -> Result<Self, ProjectError> {
        Self::load_with_overrides(path, store, &crate::config::Config::default())
    }

    /// Like [`Project::load`], with command-line overrides applied as the
    /// highest-precedence configuration layer.
    pub fn load_with_overrides(
        path: impl AsRef<Path>,
        store: &ConfigStore,
        overrides: &crate::config::Config,
    ) -> Result<Self, ProjectError> {
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
        let config = store.load_resolved(&root, overrides)?;
        let shared_assets = resolve_shared_assets(store, config.project.template.as_deref())?;
        Ok(Self {
            root,
            config_path,
            config,
            shared_assets,
        })
    }

    /// Find the nearest project root by walking from `start` towards `/`.
    /// A project marker (`latex_handler.toml`) takes precedence over a `src/` directory.
    pub fn discover(start: impl AsRef<Path>, store: &ConfigStore) -> Result<Self, ProjectError> {
        Self::discover_with_overrides(start, store, &crate::config::Config::default())
    }

    /// Like [`Project::discover`], with command-line overrides applied as the
    /// highest-precedence configuration layer.
    pub fn discover_with_overrides(
        start: impl AsRef<Path>,
        store: &ConfigStore,
        overrides: &crate::config::Config,
    ) -> Result<Self, ProjectError> {
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
                return Self::load_with_overrides(candidate, store, overrides);
            }
            if candidate.join("src").is_dir() && fallback.is_none() {
                fallback = Some(candidate.to_path_buf());
            }
        }
        Self::load_with_overrides(
            fallback.unwrap_or_else(|| directory.to_path_buf()),
            store,
            overrides,
        )
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

    /// Project-local assets directory. It need not exist yet.
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join(&self.config.structure.assets)
    }

    /// Shared-assets directory declared by the installed source template.
    pub fn shared_assets_dir(&self) -> Option<&Path> {
        self.shared_assets.as_deref()
    }

    /// Directories that should be searched for project assets, local first.
    /// Non-existent directories are omitted.
    pub fn asset_directories(&self) -> Vec<PathBuf> {
        let mut directories = Vec::new();
        let local = self.assets_dir();
        if local.is_dir() {
            directories.push(local);
        }
        if let Some(shared) = &self.shared_assets {
            if shared.is_dir() {
                directories.push(shared.clone());
            }
        }
        directories
    }

    pub fn documents(&self) -> Result<Vec<DiscoveredDocument>, ProjectError> {
        discover_documents(self)
    }

    /// Load this project's template snapshot, if it has one.
    pub fn snapshot(&self) -> Result<Option<ProjectSnapshot>, SnapshotError> {
        ProjectSnapshot::load_optional(&self.root)
    }

    pub fn validate(&self) -> ProjectValidation {
        validate_project(self)
    }
}

fn resolve_shared_assets(
    store: &ConfigStore,
    template: Option<&str>,
) -> Result<Option<PathBuf>, ProjectError> {
    let Some(template) = template else {
        return Ok(None);
    };
    let directory = store.templates_dir().join(template);
    if !directory.join(TEMPLATE_MANIFEST_FILE).is_file() {
        return Ok(None);
    }
    let manifest = TemplateManifest::load(&directory)?;
    Ok(manifest.shared_assets_dir(&directory))
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
    fn cli_overrides_take_precedence_over_project_configuration() {
        use crate::config::{CompilerConfig, Config, LatexEngine};
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("latex_handler.toml"),
            "[compiler]\nengine = \"xelatex\"\n",
        )
        .unwrap();
        let store = ConfigStore::new(dir.path().join("global"));

        let plain = Project::load(dir.path(), &store).unwrap();
        assert_eq!(plain.config().compiler.engine, LatexEngine::XeLatex);

        let overrides = Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::LuaLatex),
                ..Default::default()
            }),
            ..Default::default()
        };
        let overridden = Project::discover_with_overrides(dir.path(), &store, &overrides).unwrap();
        assert_eq!(overridden.config().compiler.engine, LatexEngine::LuaLatex);
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
