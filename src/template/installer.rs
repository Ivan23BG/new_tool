use super::{TemplateManifest, TemplateRegistry};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TemplateInstallError {
    #[error("invalid template name '{0}'")]
    InvalidName(String),
    #[error(
        "no repository specified for template '{0}'; pass --repository or set templates.repository in the global configuration"
    )]
    RepositoryNotConfigured(String),
    #[error("template '{0}' is already installed")]
    AlreadyInstalled(String),
    #[error("temporary installation path already exists: '{0}'")]
    TemporaryPathExists(PathBuf),
    #[error("could not access template directory '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not execute git; ensure Git is installed and available on PATH: {0}")]
    GitUnavailable(#[source] std::io::Error),
    #[error("git clone failed for '{repository}': {details}")]
    GitFailed { repository: String, details: String },
    #[error("cloned repository does not contain a valid template manifest: {0}")]
    Manifest(#[from] super::TemplateManifestError),
    #[error("repository declares template name '{actual}', but '{expected}' was requested")]
    NameMismatch { expected: String, actual: String },
    #[error("could not finalize template installation at '{path}': {source}")]
    Finalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Clone and validate a template before making it visible in the registry.
/// Installation is finalized with a directory rename, so incomplete clones
/// are never exposed as installed templates.
pub fn install_template(
    registry: &TemplateRegistry,
    name: &str,
    repository: Option<&str>,
) -> Result<super::InstalledTemplate, TemplateInstallError> {
    if !valid_name(name) {
        return Err(TemplateInstallError::InvalidName(name.to_owned()));
    }
    let repository = repository
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| TemplateInstallError::RepositoryNotConfigured(name.to_owned()))?;

    let destination = registry.root().join(name);
    if destination.exists() {
        return Err(TemplateInstallError::AlreadyInstalled(name.to_owned()));
    }

    fs::create_dir_all(registry.root()).map_err(|source| TemplateInstallError::Io {
        path: registry.root().to_path_buf(),
        source,
    })?;

    let staging = registry
        .root()
        .join(format!(".{name}.installing-{}", std::process::id()));
    if staging.exists() {
        return Err(TemplateInstallError::TemporaryPathExists(staging));
    }

    let output = match Command::new("git")
        .args(["clone", "--depth", "1", "--", repository])
        .arg(&staging)
        .output()
    {
        Ok(output) => output,
        Err(source) => return Err(TemplateInstallError::GitUnavailable(source)),
    };
    if !output.status.success() {
        let _ = fs::remove_dir_all(&staging);
        let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(TemplateInstallError::GitFailed {
            repository: repository.to_owned(),
            details: if details.is_empty() {
                output.status.to_string()
            } else {
                details
            },
        });
    }

    let result = validate_and_finalize(&staging, &destination, name, repository);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn validate_and_finalize(
    staging: &Path,
    destination: &Path,
    expected_name: &str,
    repository: &str,
) -> Result<super::InstalledTemplate, TemplateInstallError> {
    let mut manifest = TemplateManifest::load(staging)?;
    if manifest.name != expected_name {
        return Err(TemplateInstallError::NameMismatch {
            expected: expected_name.to_owned(),
            actual: manifest.name,
        });
    }

    // Record the actual clone source so status/update commands can use it.
    manifest.repository = Some(repository.to_owned());
    manifest.write(staging)?;

    fs::rename(staging, destination).map_err(|source| TemplateInstallError::Finalize {
        path: destination.to_path_buf(),
        source,
    })?;

    Ok(super::InstalledTemplate {
        directory: destination.to_path_buf(),
        manifest,
    })
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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

    fn local_template_repo(root: &Path, name: &str) -> PathBuf {
        let repo = root.join("source-repository");
        fs::create_dir_all(&repo).unwrap();
        git(&["init", "-q"], &repo);
        fs::write(
            repo.join("template.toml"),
            format!("name = '{name}'\nversion = '1.0.0'\n"),
        )
        .unwrap();
        fs::write(
            repo.join("main.tex"),
            "\\documentclass{article}\\begin{document}Hello\\end{document}\n",
        )
        .unwrap();
        git(&["add", "."], &repo);
        git(
            &[
                "-c",
                "user.name=latex_handler tests",
                "-c",
                "user.email=tests@example.invalid",
                "commit",
                "-q",
                "-m",
                "initial",
            ],
            &repo,
        );
        repo
    }

    #[test]
    fn installs_local_git_repository_and_records_source() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempdir().unwrap();
        let repo = local_template_repo(temp.path(), "university");
        let registry = TemplateRegistry::new(temp.path().join("installed"));
        let installed =
            install_template(&registry, "university", Some(repo.to_str().unwrap())).unwrap();
        assert_eq!(installed.manifest.name, "university");
        assert_eq!(installed.manifest.repository.as_deref(), repo.to_str());
        assert!(installed.directory.join("main.tex").is_file());
        assert_eq!(registry.get("university").unwrap(), installed);
    }

    #[test]
    fn rejects_manifest_name_mismatch_and_cleans_staging_directory() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempdir().unwrap();
        let repo = local_template_repo(temp.path(), "different");
        let root = temp.path().join("installed");
        let registry = TemplateRegistry::new(&root);
        assert!(matches!(
            install_template(&registry, "requested", Some(repo.to_str().unwrap())),
            Err(TemplateInstallError::NameMismatch { .. })
        ));
        assert!(!root.join("requested").exists());
        assert!(
            !root
                .join(".requested.installing-".to_owned() + &std::process::id().to_string())
                .exists()
        );
    }

    #[test]
    fn refuses_to_overwrite_existing_installation_and_invalid_names() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("installed");
        fs::create_dir_all(root.join("university")).unwrap();
        let registry = TemplateRegistry::new(&root);
        assert!(matches!(
            install_template(&registry, "university", Some("unused")),
            Err(TemplateInstallError::AlreadyInstalled(_))
        ));
        assert!(matches!(
            install_template(&registry, "../escape", Some("unused")),
            Err(TemplateInstallError::InvalidName(_))
        ));
    }
}
