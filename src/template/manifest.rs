use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const TEMPLATE_MANIFEST_FILE: &str = "template.toml";

/// Metadata distributed with a template repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
}

#[derive(Debug, Error)]
pub enum TemplateManifestError {
    #[error("could not read template manifest '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid template manifest '{path}': {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("template manifest '{path}' has an invalid name '{name}'")]
    InvalidName { path: PathBuf, name: String },
    #[error("template manifest '{0}' has an empty version")]
    EmptyVersion(PathBuf),
}

impl TemplateManifest {
    pub fn load(template_dir: impl AsRef<Path>) -> Result<Self, TemplateManifestError> {
        let path = template_dir.as_ref().join(TEMPLATE_MANIFEST_FILE);
        let source = fs::read_to_string(&path).map_err(|source| TemplateManifestError::Io {
            path: path.clone(),
            source,
        })?;
        let manifest: Self =
            toml::from_str(&source).map_err(|source| TemplateManifestError::Parse {
                path: path.clone(),
                source,
            })?;
        let valid_name = !manifest.name.trim().is_empty()
            && manifest
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid_name {
            return Err(TemplateManifestError::InvalidName {
                path,
                name: manifest.name,
            });
        }
        if manifest.version.trim().is_empty() {
            return Err(TemplateManifestError::EmptyVersion(path));
        }
        Ok(manifest)
    }

    pub fn write(&self, template_dir: impl AsRef<Path>) -> Result<(), TemplateManifestError> {
        let path = template_dir.as_ref().join(TEMPLATE_MANIFEST_FILE);
        let contents =
            toml::to_string_pretty(self).expect("serializing TemplateManifest cannot fail");
        fs::write(&path, contents).map_err(|source| TemplateManifestError::Io { path, source })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn loads_manifest_and_optional_metadata() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(TEMPLATE_MANIFEST_FILE),
            "name = 'university'\nversion = '1.2.0'\ndescription = 'University thesis'\n",
        )
        .unwrap();
        let manifest = TemplateManifest::load(dir.path()).unwrap();
        assert_eq!(manifest.name, "university");
        assert_eq!(manifest.version, "1.2.0");
        assert_eq!(manifest.repository, None);
    }

    #[test]
    fn rejects_invalid_names_and_empty_versions() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(TEMPLATE_MANIFEST_FILE);
        fs::write(&path, "name = '../escape'\nversion = '1.0.0'").unwrap();
        assert!(matches!(
            TemplateManifest::load(dir.path()),
            Err(TemplateManifestError::InvalidName { .. })
        ));
        fs::write(&path, "name = 'valid'\nversion = ' '").unwrap();
        assert!(matches!(
            TemplateManifest::load(dir.path()),
            Err(TemplateManifestError::EmptyVersion(_))
        ));
    }
}
