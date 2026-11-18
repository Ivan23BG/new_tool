use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
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
    /// Other templates required by this template.
    ///
    /// Keys are template names and values describe the accepted version range.
    /// A dependency marked optional is not required for a valid installation.
    #[serde(default)]
    pub dependencies: BTreeMap<String, TemplateDependency>,
}

/// A template dependency declared in `template.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDependency {
    /// A semantic-version requirement, for example `^1.2` or `>=1.0, <2.0`.
    pub version: String,
    /// Whether the dependency may be omitted by the project.
    #[serde(default)]
    pub optional: bool,
    /// Git repository used when this dependency is not installed yet.
    #[serde(default)]
    pub repository: Option<String>,
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
    #[error("template manifest '{path}' declares an invalid dependency name '{name}'")]
    InvalidDependencyName { path: PathBuf, name: String },
    #[error("template manifest '{path}' cannot depend on itself ('{name}')")]
    SelfDependency { path: PathBuf, name: String },
    #[error(
        "template manifest '{path}' declares an empty version requirement for dependency '{name}'"
    )]
    EmptyDependencyVersion { path: PathBuf, name: String },
    #[error(
        "template manifest '{path}' declares an invalid semantic version requirement for dependency '{name}': {requirement}"
    )]
    InvalidDependencyVersion {
        path: PathBuf,
        name: String,
        requirement: String,
    },
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
        for (name, dependency) in &manifest.dependencies {
            let valid_dependency_name = !name.trim().is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid_dependency_name {
                return Err(TemplateManifestError::InvalidDependencyName {
                    path,
                    name: name.clone(),
                });
            }
            if name == &manifest.name {
                return Err(TemplateManifestError::SelfDependency {
                    path,
                    name: name.clone(),
                });
            }
            if dependency.version.trim().is_empty() {
                return Err(TemplateManifestError::EmptyDependencyVersion {
                    path,
                    name: name.clone(),
                });
            }
            if semver::VersionReq::parse(&dependency.version).is_err() {
                return Err(TemplateManifestError::InvalidDependencyVersion {
                    path,
                    name: name.clone(),
                    requirement: dependency.version.clone(),
                });
            }
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
    fn loads_required_and_optional_dependencies() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(TEMPLATE_MANIFEST_FILE),
            r#"name = "university"
version = "1.2.0"

[dependencies.common]
version = "^2.1"

[dependencies.diagrams]
version = ">=1.0, <2.0"
optional = true
repository = "https://example.invalid/diagrams.git"
"#,
        )
        .unwrap();

        let manifest = TemplateManifest::load(dir.path()).unwrap();
        assert_eq!(manifest.dependencies.len(), 2);
        assert_eq!(manifest.dependencies["common"].version, "^2.1");
        assert!(!manifest.dependencies["common"].optional);
        assert_eq!(manifest.dependencies["diagrams"].version, ">=1.0, <2.0");
        assert!(manifest.dependencies["diagrams"].optional);
        assert_eq!(
            manifest.dependencies["diagrams"].repository.as_deref(),
            Some("https://example.invalid/diagrams.git")
        );

        manifest.write(dir.path()).unwrap();
        assert_eq!(TemplateManifest::load(dir.path()).unwrap(), manifest);
    }

    #[test]
    fn old_manifests_default_to_no_dependencies() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(TEMPLATE_MANIFEST_FILE),
            "name = 'university'\nversion = '1.2.0'\n",
        )
        .unwrap();
        let manifest = TemplateManifest::load(dir.path()).unwrap();
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn rejects_invalid_dependency_metadata() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(TEMPLATE_MANIFEST_FILE);

        fs::write(
            &path,
            "name = 'university'\nversion = '1.0.0'\n[dependencies.'../escape']\nversion = '1.0'",
        )
        .unwrap();
        assert!(matches!(
            TemplateManifest::load(dir.path()),
            Err(TemplateManifestError::InvalidDependencyName { .. })
        ));

        fs::write(
            &path,
            "name = 'university'\nversion = '1.0.0'\n[dependencies.university]\nversion = '1.0'",
        )
        .unwrap();
        assert!(matches!(
            TemplateManifest::load(dir.path()),
            Err(TemplateManifestError::SelfDependency { .. })
        ));

        fs::write(
            &path,
            "name = 'university'\nversion = '1.0.0'\n[dependencies.common]\nversion = '  '",
        )
        .unwrap();
        assert!(matches!(
            TemplateManifest::load(dir.path()),
            Err(TemplateManifestError::EmptyDependencyVersion { .. })
        ));

        fs::write(
            &path,
            "name = 'university'\nversion = '1.0.0'\n[dependencies.common]\nversion = 'not a version'",
        )
        .unwrap();
        assert!(matches!(
            TemplateManifest::load(dir.path()),
            Err(TemplateManifestError::InvalidDependencyVersion { .. })
        ));
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
