use super::{InstalledTemplate, TemplateManifest, TemplateRegistry};
use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};
use thiserror::Error;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum TemplateUpdateError {
    #[error(transparent)]
    Registry(#[from] super::TemplateRegistryError),
    #[error("template '{0}' has no recorded Git repository")]
    RepositoryNotRecorded(String),
    #[error("template '{0}' is not a Git working tree")]
    NotGitRepository(String),
    #[error("template '{0}' has local modifications; commit or remove them before updating")]
    LocalModifications(String),
    #[error("could not execute git; ensure Git is installed and available on PATH: {0}")]
    GitUnavailable(#[source] std::io::Error),
    #[error("Git operation failed for '{repository}': {details}")]
    GitFailed { repository: String, details: String },
    #[error("no published Git tags found for '{0}'")]
    NoVersions(String),
    #[error("no stable Git tags found for '{0}' (enable templates.prerelease to use prereleases)")]
    NoStableVersions(String),
    #[error("version '{version}' is not published for template '{name}'")]
    VersionNotFound { name: String, version: String },
    #[error("could not read template manifest in update: {0}")]
    Manifest(#[from] super::TemplateManifestError),
    #[error("updated repository declares template name '{actual}', but '{expected}' was expected")]
    NameMismatch { expected: String, actual: String },
    #[error("temporary update path already exists: '{0}'")]
    TemporaryPathExists(PathBuf),
    #[error("could not prepare update directory '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not replace installed template '{name}': {source}")]
    Replace {
        name: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "template '{name}' was updated, but its backup could not be removed at '{path}': {source}"
    )]
    BackupCleanup {
        name: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Update an installed template to a published tag, or to the newest stable
/// tag when `version` is `None`. If the repository has no tags, an unpinned
/// update follows its default branch.
pub fn update_template(
    registry: &TemplateRegistry,
    name: &str,
    version: Option<&str>,
    allow_prerelease: bool,
) -> Result<InstalledTemplate, TemplateUpdateError> {
    let installed = registry.get(name)?;
    let repository = installed
        .manifest
        .repository
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| TemplateUpdateError::RepositoryNotRecorded(name.to_owned()))?;

    ensure_safe_to_replace(&installed)?;

    let tags = remote_tags(repository)?;
    let selected_tag = match version {
        Some(requested) => Some(resolve_requested_tag(name, requested, &tags)?),
        None if tags.is_empty() => None,
        None => Some(latest_tag(name, &tags, allow_prerelease)?),
    };

    let id = TEMP_ID.fetch_add(1, AtomicOrdering::Relaxed);
    let staging = registry
        .root()
        .join(format!(".{name}.updating-{}-{id}", std::process::id()));
    let backup = registry
        .root()
        .join(format!(".{name}.backup-{}-{id}", std::process::id()));
    if staging.exists() {
        return Err(TemplateUpdateError::TemporaryPathExists(staging));
    }
    if backup.exists() {
        return Err(TemplateUpdateError::TemporaryPathExists(backup));
    }
    fs::create_dir_all(registry.root()).map_err(|source| TemplateUpdateError::Io {
        path: registry.root().to_path_buf(),
        source,
    })?;

    let clone_output =
        clone_selected(repository, selected_tag.as_deref(), &staging).map_err(|error| {
            let _ = fs::remove_dir_all(&staging);
            error
        })?;
    let _ = clone_output;

    let prepared = (|| {
        let mut manifest = TemplateManifest::load(&staging)?;
        if manifest.name != name {
            return Err(TemplateUpdateError::NameMismatch {
                expected: name.to_owned(),
                actual: manifest.name,
            });
        }
        manifest.repository = Some(repository.to_owned());
        manifest.write(&staging)?;
        Ok::<TemplateManifest, TemplateUpdateError>(manifest)
    })();

    let manifest = match prepared {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };

    fs::rename(&installed.directory, &backup).map_err(|source| {
        let _ = fs::remove_dir_all(&staging);
        TemplateUpdateError::Replace {
            name: name.to_owned(),
            source,
        }
    })?;

    if let Err(source) = fs::rename(&staging, &installed.directory) {
        let _ = fs::rename(&backup, &installed.directory);
        let _ = fs::remove_dir_all(&staging);
        return Err(TemplateUpdateError::Replace {
            name: name.to_owned(),
            source,
        });
    }

    if let Err(source) = fs::remove_dir_all(&backup) {
        return Err(TemplateUpdateError::BackupCleanup {
            name: name.to_owned(),
            path: backup,
            source,
        });
    }

    Ok(InstalledTemplate {
        directory: installed.directory,
        manifest,
    })
}

fn clone_selected(
    repository: &str,
    tag: Option<&str>,
    destination: &Path,
) -> Result<Output, TemplateUpdateError> {
    let mut command = Command::new("git");
    command.args(["clone", "--depth", "1"]);
    if let Some(tag) = tag {
        command.args(["--branch", tag]);
    }
    let output = command
        .args(["--", repository])
        .arg(destination)
        .output()
        .map_err(TemplateUpdateError::GitUnavailable)?;
    if !output.status.success() {
        let details = stderr_or_status(&output);
        let _ = fs::remove_dir_all(destination);
        return Err(TemplateUpdateError::GitFailed {
            repository: repository.to_owned(),
            details,
        });
    }
    Ok(output)
}

fn remote_tags(repository: &str) -> Result<Vec<String>, TemplateUpdateError> {
    let output = Command::new("git")
        .args(["ls-remote", "--tags", "--refs", "--", repository])
        .output()
        .map_err(TemplateUpdateError::GitUnavailable)?;
    if !output.status.success() {
        return Err(TemplateUpdateError::GitFailed {
            repository: repository.to_owned(),
            details: stderr_or_status(&output),
        });
    }
    let mut tags = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(_, reference)| reference))
        .filter_map(|reference| reference.strip_prefix("refs/tags/"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    tags.sort_by(|a, b| compare_version_tags(a, b));
    tags.dedup();
    Ok(tags)
}

fn resolve_requested_tag<'a>(
    name: &str,
    requested: &str,
    tags: &'a [String],
) -> Result<&'a str, TemplateUpdateError> {
    let requested = requested.trim();
    let prefixed = if requested.starts_with('v') {
        None
    } else {
        Some(format!("v{requested}"))
    };
    tags.iter()
        .find(|tag| tag.as_str() == requested)
        .or_else(|| {
            prefixed
                .as_ref()
                .and_then(|candidate| tags.iter().find(|tag| *tag == candidate))
        })
        .map(String::as_str)
        .ok_or_else(|| TemplateUpdateError::VersionNotFound {
            name: name.to_owned(),
            version: requested.to_owned(),
        })
}

fn latest_tag<'a>(
    name: &str,
    tags: &'a [String],
    allow_prerelease: bool,
) -> Result<&'a str, TemplateUpdateError> {
    let selected = tags
        .iter()
        .rev()
        .find(|tag| allow_prerelease || !is_prerelease(tag));
    selected.map(String::as_str).ok_or_else(|| {
        if tags.is_empty() {
            TemplateUpdateError::NoVersions(name.to_owned())
        } else {
            TemplateUpdateError::NoStableVersions(name.to_owned())
        }
    })
}

fn is_prerelease(tag: &str) -> bool {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    version.split_once('-').is_some()
}

/// Compare common semantic-version tags numerically (so v1.10 sorts after
/// v1.9), with a lexical fallback for non-semver tag names.
pub(crate) fn compare_version_tags(left: &str, right: &str) -> Ordering {
    let left_version = left.strip_prefix('v').unwrap_or(left);
    let right_version = right.strip_prefix('v').unwrap_or(right);
    let left_core = left_version.split('-').next().unwrap_or(left_version);
    let right_core = right_version.split('-').next().unwrap_or(right_version);
    let left_parts = left_core.split('.').collect::<Vec<_>>();
    let right_parts = right_core.split('.').collect::<Vec<_>>();
    for index in 0..left_parts.len().max(right_parts.len()) {
        let a = left_parts
            .get(index)
            .and_then(|part| part.parse::<u64>().ok());
        let b = right_parts
            .get(index)
            .and_then(|part| part.parse::<u64>().ok());
        match (a, b) {
            (Some(a), Some(b)) if a != b => return a.cmp(&b),
            (Some(_), Some(_)) => {}
            _ => return left.cmp(right),
        }
    }
    match (left_version.contains('-'), right_version.contains('-')) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        _ => left.cmp(right),
    }
}

fn ensure_safe_to_replace(installed: &InstalledTemplate) -> Result<(), TemplateUpdateError> {
    let status = Command::new("git")
        .args(["-C"])
        .arg(&installed.directory)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .map_err(TemplateUpdateError::GitUnavailable)?;
    if !status.status.success() {
        return Err(TemplateUpdateError::NotGitRepository(
            installed.manifest.name.clone(),
        ));
    }
    let changes = String::from_utf8_lossy(&status.stdout);
    let lines = changes.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Ok(());
    }

    // Stage 4.2 records the clone source in template.toml. That one expected
    // metadata-only modification is safe; all other local changes are kept
    // protected from replacement.
    if lines.len() == 1 && lines[0].get(3..) == Some("template.toml") {
        let head = Command::new("git")
            .args(["-C"])
            .arg(&installed.directory)
            .args(["show", "HEAD:template.toml"])
            .output()
            .map_err(TemplateUpdateError::GitUnavailable)?;
        if head.status.success() {
            let mut original: TemplateManifest =
                toml::from_str(&String::from_utf8_lossy(&head.stdout)).map_err(|_| {
                    TemplateUpdateError::LocalModifications(installed.manifest.name.clone())
                })?;
            original.repository = installed.manifest.repository.clone();
            if original == installed.manifest {
                return Ok(());
            }
        }
    }
    Err(TemplateUpdateError::LocalModifications(
        installed.manifest.name.clone(),
    ))
}

fn stderr_or_status(output: &Output) -> String {
    let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if details.is_empty() {
        output.status.to_string()
    } else {
        details
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    use tempfile::tempdir;

    fn git(args: &[&str], cwd: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .unwrap();
        assert!(status.success(), "git command failed: {args:?}");
    }

    fn repository(root: &Path) -> PathBuf {
        let repo = root.join("source");
        fs::create_dir_all(&repo).unwrap();
        git(&["init", "-q", "-b", "main"], &repo);
        fs::write(
            repo.join("template.toml"),
            "name = 'university'\nversion = '1.0.0'\n",
        )
        .unwrap();
        fs::write(repo.join("content.txt"), "version one\n").unwrap();
        git(&["add", "."], &repo);
        git(
            &[
                "-c",
                "user.name=tests",
                "-c",
                "user.email=tests@example.invalid",
                "commit",
                "-q",
                "-m",
                "v1",
            ],
            &repo,
        );
        git(&["tag", "v1.0.0"], &repo);
        fs::write(
            repo.join("template.toml"),
            "name = 'university'\nversion = '1.1.0'\n",
        )
        .unwrap();
        fs::write(repo.join("content.txt"), "version one point one\n").unwrap();
        git(&["add", "."], &repo);
        git(
            &[
                "-c",
                "user.name=tests",
                "-c",
                "user.email=tests@example.invalid",
                "commit",
                "-q",
                "-m",
                "v1.1",
            ],
            &repo,
        );
        git(&["tag", "v1.1.0"], &repo);
        fs::write(
            repo.join("template.toml"),
            "name = 'university'\nversion = '2.0.0-rc.1'\n",
        )
        .unwrap();
        git(&["add", "template.toml"], &repo);
        git(
            &[
                "-c",
                "user.name=tests",
                "-c",
                "user.email=tests@example.invalid",
                "commit",
                "-q",
                "-m",
                "prerelease",
            ],
            &repo,
        );
        git(&["tag", "v2.0.0-rc.1"], &repo);
        repo
    }

    fn install_at_tag(root: &Path, repo: &Path, tag: &str) -> TemplateRegistry {
        let registry = TemplateRegistry::new(root.join("installed"));
        let destination = registry.root().join("university");
        fs::create_dir_all(registry.root()).unwrap();
        let output = Command::new("git")
            .args(["clone", "--depth", "1", "--branch", tag, "--"])
            .arg(repo)
            .arg(&destination)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut manifest = TemplateManifest::load(&destination).unwrap();
        manifest.repository = Some(repo.to_string_lossy().into_owned());
        manifest.write(&destination).unwrap();
        registry
    }

    #[test]
    fn updates_to_requested_tag_and_latest_stable_tag() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempdir().unwrap();
        let repo = repository(temp.path());
        let registry = install_at_tag(temp.path(), &repo, "v1.0.0");

        let first = update_template(&registry, "university", Some("1.1.0"), false).unwrap();
        assert_eq!(first.manifest.version, "1.1.0");
        assert_eq!(
            fs::read_to_string(first.directory.join("content.txt")).unwrap(),
            "version one point one\n"
        );

        let latest = update_template(&registry, "university", None, false).unwrap();
        assert_eq!(latest.manifest.version, "1.1.0");
    }

    #[test]
    fn prerelease_can_be_selected_when_enabled() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempdir().unwrap();
        let repo = repository(temp.path());
        let registry = install_at_tag(temp.path(), &repo, "v1.0.0");
        let latest = update_template(&registry, "university", None, true).unwrap();
        assert_eq!(latest.manifest.version, "2.0.0-rc.1");
    }

    #[test]
    fn refuses_to_discard_user_modifications() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempdir().unwrap();
        let repo = repository(temp.path());
        let registry = install_at_tag(temp.path(), &repo, "v1.0.0");
        fs::write(registry.root().join("university/custom.txt"), "keep me").unwrap();
        assert!(matches!(
            update_template(&registry, "university", None, false),
            Err(TemplateUpdateError::LocalModifications(_))
        ));
        assert_eq!(
            fs::read_to_string(registry.root().join("university/custom.txt")).unwrap(),
            "keep me"
        );
    }

    #[test]
    fn rejects_unknown_version_without_touching_installation() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempdir().unwrap();
        let repo = repository(temp.path());
        let registry = install_at_tag(temp.path(), &repo, "v1.0.0");
        assert!(matches!(
            update_template(&registry, "university", Some("9.9.9"), false),
            Err(TemplateUpdateError::VersionNotFound { .. })
        ));
        assert_eq!(
            TemplateManifest::load(registry.root().join("university"))
                .unwrap()
                .version,
            "1.0.0"
        );
    }
}
