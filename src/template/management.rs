use super::{InstalledTemplate, TemplateRegistry};
use std::{path::PathBuf, process::Command};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TemplateManagementError {
    #[error(transparent)]
    Registry(#[from] super::TemplateRegistryError),
    #[error("template '{0}' has no recorded Git repository")]
    RepositoryNotRecorded(String),
    #[error("could not execute git; ensure Git is installed and available on PATH: {0}")]
    GitUnavailable(#[source] std::io::Error),
    #[error("could not query versions for '{repository}': {details}")]
    GitFailed { repository: String, details: String },
    #[error("could not remove template directory '{path}': {source}")]
    Remove {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Remove an installed template directory. Projects created from the template
/// are independent and are deliberately not inspected or modified.
pub fn remove_template(
    registry: &TemplateRegistry,
    name: &str,
) -> Result<InstalledTemplate, TemplateManagementError> {
    let installed = registry.get(name)?;
    std::fs::remove_dir_all(&installed.directory).map_err(|source| {
        TemplateManagementError::Remove {
            path: installed.directory.clone(),
            source,
        }
    })?;
    Ok(installed)
}

/// Return the Git tags published by the installed template's remote repository.
pub fn list_template_versions(
    registry: &TemplateRegistry,
    name: &str,
) -> Result<Vec<String>, TemplateManagementError> {
    let installed = registry.get(name)?;
    let repository = installed
        .manifest
        .repository
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| TemplateManagementError::RepositoryNotRecorded(name.to_owned()))?;

    let output = Command::new("git")
        .args(["ls-remote", "--tags", "--refs", "--", repository])
        .output()
        .map_err(TemplateManagementError::GitUnavailable)?;

    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(TemplateManagementError::GitFailed {
            repository: repository.to_owned(),
            details: if details.is_empty() {
                output.status.to_string()
            } else {
                details
            },
        });
    }

    let mut versions = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(_, reference)| reference))
        .filter_map(|reference| reference.strip_prefix("refs/tags/"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| super::updater::compare_version_tags(left, right));
    versions.dedup();
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{TEMPLATE_MANIFEST_FILE, TemplateManifest};
    use std::{fs, path::Path, process::Command};
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
        let repo = root.join("repository");
        fs::create_dir_all(&repo).unwrap();
        git(&["init", "-q"], &repo);
        fs::write(
            repo.join(TEMPLATE_MANIFEST_FILE),
            "name = 'university'\nversion = '1.0.0'\n",
        )
        .unwrap();
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
                "initial",
            ],
            &repo,
        );
        git(&["tag", "v1.0.0"], &repo);
        git(&["tag", "v1.1.0"], &repo);
        repo
    }

    fn installed_registry(root: &Path, repository: &Path) -> TemplateRegistry {
        let registry = TemplateRegistry::new(root.join("installed"));
        let directory = registry.root().join("university");
        fs::create_dir_all(&directory).unwrap();
        TemplateManifest {
            name: "university".into(),
            version: "1.0.0".into(),
            description: None,
            repository: Some(repository.to_string_lossy().into_owned()),
            authors: Vec::new(),
        }
        .write(&directory)
        .unwrap();
        registry
    }

    #[test]
    fn lists_remote_git_tags_in_sorted_order() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempdir().unwrap();
        let repo = repository(temp.path());
        let registry = installed_registry(temp.path(), &repo);
        assert_eq!(
            list_template_versions(&registry, "university").unwrap(),
            vec!["v1.0.0", "v1.1.0"]
        );
    }

    #[test]
    fn reports_missing_repository_metadata() {
        let temp = tempdir().unwrap();
        let registry = TemplateRegistry::new(temp.path().join("installed"));
        let directory = registry.root().join("university");
        fs::create_dir_all(&directory).unwrap();
        TemplateManifest {
            name: "university".into(),
            version: "1.0.0".into(),
            description: None,
            repository: None,
            authors: Vec::new(),
        }
        .write(&directory)
        .unwrap();
        assert!(matches!(
            list_template_versions(&registry, "university"),
            Err(TemplateManagementError::RepositoryNotRecorded(_))
        ));
    }

    #[test]
    fn removes_only_the_installed_template_directory() {
        let temp = tempdir().unwrap();
        let repo = repository(temp.path());
        let registry = installed_registry(temp.path(), &repo);
        let project = temp.path().join("existing-project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("main.tex"), "\\documentclass{article}").unwrap();

        let removed = remove_template(&registry, "university").unwrap();
        assert_eq!(removed.manifest.name, "university");
        assert!(!registry.root().join("university").exists());
        assert!(project.join("main.tex").is_file());
    }
}
