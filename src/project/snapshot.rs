//! Snapshots of the template resources copied into a project.
//!
//! A snapshot records which template and version a project came from, and a
//! content fingerprint for every file copied from that template. Fingerprints
//! describe the *template baseline*, not the current project file, so a later
//! synchronization can distinguish user modifications from template updates.
//!
//! Fingerprints are stable, non-cryptographic FNV-1a hashes combined with the
//! file length. They are intended for change detection on a developer's
//! machine, not for security-sensitive integrity checks.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

pub const PROJECT_SNAPSHOT_FILE: &str = "latex_handler.snapshot.toml";
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;

/// A file copied from the source template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRecord {
    /// Path relative to the template root.
    pub template_path: String,
    /// Fingerprint of the template version recorded in this snapshot.
    pub fingerprint: String,
}

/// Whether a managed resource still matches its snapshot baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceState {
    Unchanged,
    Modified,
    Missing,
}

/// Metadata written to `latex_handler.snapshot.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub format_version: u32,
    pub template: String,
    pub template_version: String,
    /// Shared-assets directory declared by the template, relative to the
    /// installed template root. Shared assets are not copied into projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_assets: Option<String>,
    /// Managed resources keyed by project-relative path.
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceRecord>,
}

impl ProjectSnapshot {
    pub fn new(
        template: impl Into<String>,
        template_version: impl Into<String>,
        shared_assets: Option<&Path>,
    ) -> Result<Self, SnapshotError> {
        Ok(Self {
            format_version: SNAPSHOT_FORMAT_VERSION,
            template: template.into(),
            template_version: template_version.into(),
            shared_assets: shared_assets.map(path_key).transpose()?,
            resources: BTreeMap::new(),
        })
    }

    /// Path of a project's snapshot file.
    pub fn path(project_root: impl AsRef<Path>) -> PathBuf {
        project_root.as_ref().join(PROJECT_SNAPSHOT_FILE)
    }

    /// Load a snapshot, treating a missing file as `None`.
    pub fn load_optional(project_root: impl AsRef<Path>) -> Result<Option<Self>, SnapshotError> {
        let path = Self::path(project_root);
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(SnapshotError::Io { path, source });
            }
        };
        let snapshot: Self = toml::from_str(&source).map_err(|source| SnapshotError::Parse {
            path: path.clone(),
            source,
        })?;
        if snapshot.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(SnapshotError::UnsupportedVersion {
                path,
                found: snapshot.format_version,
                supported: SNAPSHOT_FORMAT_VERSION,
            });
        }
        snapshot.validate(&path)?;
        Ok(Some(snapshot))
    }

    /// Load a snapshot, reporting a missing file as an error.
    pub fn load(project_root: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        let path = Self::path(project_root.as_ref());
        Self::load_optional(project_root)?.ok_or(SnapshotError::NotFound(path))
    }

    /// Write the snapshot atomically through a temporary file.
    pub fn write(&self, project_root: impl AsRef<Path>) -> Result<PathBuf, SnapshotError> {
        let path = Self::path(project_root);
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| SnapshotError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        let contents = toml::to_string_pretty(self).map_err(SnapshotError::Serialize)?;
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        fs::write(&temporary, contents).map_err(|source| SnapshotError::Io {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, &path).map_err(|source| {
            let _ = fs::remove_file(&temporary);
            SnapshotError::Io {
                path: path.clone(),
                source,
            }
        })?;
        Ok(path)
    }

    /// Record a copied file, fingerprinting its current project content.
    pub fn record_file(
        &mut self,
        project_root: impl AsRef<Path>,
        relative_path: &Path,
        template_path: &Path,
    ) -> Result<(), SnapshotError> {
        let key = path_key(relative_path)?;
        let template_key = path_key(template_path)?;
        let path = project_root.as_ref().join(key_path(&key)?);
        let bytes = fs::read(&path).map_err(|source| SnapshotError::Io {
            path: path.clone(),
            source,
        })?;
        self.resources.insert(
            key,
            ResourceRecord {
                template_path: template_key,
                fingerprint: fingerprint_bytes(&bytes),
            },
        );
        Ok(())
    }

    /// State of a managed resource, or `None` when the path is not managed.
    pub fn resource_state(
        &self,
        project_root: impl AsRef<Path>,
        key: &str,
    ) -> Result<Option<ResourceState>, SnapshotError> {
        let Some(record) = self.resources.get(key) else {
            return Ok(None);
        };
        let path = project_root.as_ref().join(key_path(key)?);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(if fingerprint_bytes(&bytes) == record.fingerprint {
                ResourceState::Unchanged
            } else {
                ResourceState::Modified
            })),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(Some(ResourceState::Missing))
            }
            Err(source) => Err(SnapshotError::Io { path, source }),
        }
    }

    fn validate(&self, path: &Path) -> Result<(), SnapshotError> {
        if self.template.trim().is_empty() {
            return Err(SnapshotError::Invalid {
                path: path.to_path_buf(),
                message: "template name must not be empty".into(),
            });
        }
        if self.template_version.trim().is_empty() {
            return Err(SnapshotError::Invalid {
                path: path.to_path_buf(),
                message: "template version must not be empty".into(),
            });
        }
        if let Some(shared_assets) = &self.shared_assets {
            key_path(shared_assets)?;
        }
        for (key, record) in &self.resources {
            key_path(key)?;
            key_path(&record.template_path)?;
            if record.fingerprint.trim().is_empty() {
                return Err(SnapshotError::Invalid {
                    path: path.to_path_buf(),
                    message: format!("resource '{key}' has an empty fingerprint"),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("project snapshot not found at '{0}'")]
    NotFound(PathBuf),
    #[error("could not access project snapshot '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid project snapshot '{path}': {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid project snapshot '{path}': {message}")]
    Invalid { path: PathBuf, message: String },
    #[error(
        "project snapshot '{path}' uses format version {found}, but this version supports {supported}"
    )]
    UnsupportedVersion {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    #[error("unsafe path '{0}' in project snapshot")]
    UnsafePath(String),
    #[error("could not serialize project snapshot: {0}")]
    Serialize(toml::ser::Error),
}

/// Stable, non-cryptographic content fingerprint (`fnv1a64:hash:length`).
pub fn fingerprint_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}:{}", bytes.len())
}

/// Convert a relative filesystem path to the portable key used in snapshots.
pub fn path_key(path: &Path) -> Result<String, SnapshotError> {
    if path.is_absolute() {
        return Err(SnapshotError::UnsafePath(path.display().to_string()));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| SnapshotError::UnsafePath(path.display().to_string()))?;
                if part.contains('\\') || part.is_empty() {
                    return Err(SnapshotError::UnsafePath(path.display().to_string()));
                }
                parts.push(part.to_owned());
            }
            _ => return Err(SnapshotError::UnsafePath(path.display().to_string())),
        }
    }
    if parts.is_empty() {
        return Err(SnapshotError::UnsafePath(path.display().to_string()));
    }
    Ok(parts.join("/"))
}

/// Convert a portable snapshot key to a safe relative path.
pub fn key_path(key: &str) -> Result<PathBuf, SnapshotError> {
    if key.is_empty() || key.starts_with('/') || key.contains('\\') {
        return Err(SnapshotError::UnsafePath(key.to_owned()));
    }
    let mut path = PathBuf::new();
    for part in key.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return Err(SnapshotError::UnsafePath(key.to_owned()));
        }
        path.push(part);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn snapshot() -> ProjectSnapshot {
        ProjectSnapshot::new("university", "1.2.0", Some(Path::new("shared"))).unwrap()
    }

    #[test]
    fn fingerprints_are_stable_and_detect_content_changes() {
        let first = fingerprint_bytes(b"hello");
        assert_eq!(first, fingerprint_bytes(b"hello"));
        assert_ne!(first, fingerprint_bytes(b"hello!"));
        assert!(first.starts_with("fnv1a64:"));
        assert!(first.ends_with(":5"));
    }

    #[test]
    fn snapshot_round_trips_through_toml() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.tex"), "content").unwrap();
        let mut snapshot = snapshot();
        snapshot
            .record_file(
                dir.path(),
                Path::new("src/main.tex"),
                Path::new("src/main.tex"),
            )
            .unwrap();
        let path = snapshot.write(dir.path()).unwrap();
        assert_eq!(path.file_name().unwrap(), PROJECT_SNAPSHOT_FILE);
        assert_eq!(
            ProjectSnapshot::load(dir.path()).unwrap(),
            snapshot,
            "snapshot should round-trip"
        );
    }

    #[test]
    fn reports_unchanged_modified_and_missing_resources() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.tex"), "original").unwrap();
        let mut snapshot = snapshot();
        snapshot
            .record_file(
                dir.path(),
                Path::new("src/main.tex"),
                Path::new("src/main.tex"),
            )
            .unwrap();

        assert_eq!(
            snapshot.resource_state(dir.path(), "src/main.tex").unwrap(),
            Some(ResourceState::Unchanged)
        );
        fs::write(dir.path().join("src/main.tex"), "changed").unwrap();
        assert_eq!(
            snapshot.resource_state(dir.path(), "src/main.tex").unwrap(),
            Some(ResourceState::Modified)
        );
        fs::remove_file(dir.path().join("src/main.tex")).unwrap();
        assert_eq!(
            snapshot.resource_state(dir.path(), "src/main.tex").unwrap(),
            Some(ResourceState::Missing)
        );
        assert_eq!(
            snapshot
                .resource_state(dir.path(), "src/other.tex")
                .unwrap(),
            None
        );
    }

    #[test]
    fn rejects_unsafe_and_unsupported_snapshots() {
        let dir = tempdir().unwrap();
        let path = ProjectSnapshot::path(dir.path());
        fs::write(
            &path,
            "format_version = 1\ntemplate = 'x'\ntemplate_version = '1.0.0'\n[resources.'../escape']\ntemplate_path = 'a'\nfingerprint = 'x'\n",
        )
        .unwrap();
        assert!(matches!(
            ProjectSnapshot::load(dir.path()),
            Err(SnapshotError::UnsafePath(_))
        ));

        fs::write(
            &path,
            "format_version = 99\ntemplate = 'x'\ntemplate_version = '1.0.0'\n",
        )
        .unwrap();
        assert!(matches!(
            ProjectSnapshot::load(dir.path()),
            Err(SnapshotError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn missing_snapshot_is_reported_or_optional() {
        let dir = tempdir().unwrap();
        assert!(
            ProjectSnapshot::load_optional(dir.path())
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            ProjectSnapshot::load(dir.path()),
            Err(SnapshotError::NotFound(_))
        ));
    }

    #[test]
    fn path_keys_are_portable_and_safe() {
        assert_eq!(
            path_key(Path::new("src/chapters/one.tex")).unwrap(),
            "src/chapters/one.tex"
        );
        assert!(path_key(Path::new("../escape")).is_err());
        assert!(path_key(Path::new("/absolute")).is_err());
        assert!(key_path("src/../../escape").is_err());
        assert!(key_path("C:/windows").is_err());
        assert_eq!(
            key_path("src/chapters/one.tex").unwrap(),
            Path::new("src/chapters/one.tex")
        );
    }
}
