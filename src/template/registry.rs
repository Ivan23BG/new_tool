use super::{TemplateManifest, TemplateManifestError};
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledTemplate {
    pub directory: PathBuf,
    pub manifest: TemplateManifest,
}

#[derive(Debug, Error)]
pub enum TemplateRegistryError {
    #[error("could not access template directory '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Manifest(#[from] TemplateManifestError),
    #[error("template '{0}' is not installed")]
    NotInstalled(String),
    #[error("multiple installed directories declare template name '{0}'")]
    DuplicateName(String),
}

/// Read-only view of templates installed in the user's template directory.
#[derive(Debug, Clone)]
pub struct TemplateRegistry {
    root: PathBuf,
}

impl TemplateRegistry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn list(&self) -> Result<Vec<InstalledTemplate>, TemplateRegistryError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(TemplateRegistryError::Io {
                    path: self.root.clone(),
                    source,
                });
            }
        };
        let mut templates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| TemplateRegistryError::Io {
                path: self.root.clone(),
                source,
            })?;
            let file_type = entry
                .file_type()
                .map_err(|source| TemplateRegistryError::Io {
                    path: entry.path(),
                    source,
                })?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let manifest_path = entry.path().join(super::TEMPLATE_MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue;
            }
            let manifest = TemplateManifest::load(entry.path())?;
            templates.push(InstalledTemplate {
                directory: entry.path(),
                manifest,
            });
        }
        templates.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        for pair in templates.windows(2) {
            if pair[0].manifest.name == pair[1].manifest.name {
                return Err(TemplateRegistryError::DuplicateName(
                    pair[0].manifest.name.clone(),
                ));
            }
        }
        Ok(templates)
    }

    pub fn get(&self, name: &str) -> Result<InstalledTemplate, TemplateRegistryError> {
        self.list()?
            .into_iter()
            .find(|item| item.manifest.name == name)
            .ok_or_else(|| TemplateRegistryError::NotInstalled(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn add(root: &Path, folder: &str, name: &str, version: &str) {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(super::super::TEMPLATE_MANIFEST_FILE),
            format!("name = '{name}'\nversion = '{version}'"),
        )
        .unwrap();
    }

    #[test]
    fn lists_only_valid_manifest_bearing_template_directories_in_order() {
        let dir = tempdir().unwrap();
        add(dir.path(), "zeta", "zeta", "1.0.0");
        add(dir.path(), "alpha", "alpha", "2.0.0");
        fs::create_dir(dir.path().join("not-a-template")).unwrap();
        let items = TemplateRegistry::new(dir.path()).list().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].manifest.name, "alpha");
        assert_eq!(items[1].manifest.name, "zeta");
    }

    #[test]
    fn missing_registry_is_an_empty_list() {
        let dir = tempdir().unwrap();
        assert!(
            TemplateRegistry::new(dir.path().join("missing"))
                .list()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn lookup_requires_an_installed_template() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            TemplateRegistry::new(dir.path()).get("missing"),
            Err(TemplateRegistryError::NotInstalled(_))
        ));
    }
}
