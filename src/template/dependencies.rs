use super::{
    InstalledTemplate, TemplateInstallError, TemplateRegistry, TemplateRegistryError,
    install_template,
};
use semver::{Version, VersionReq};
use std::{fs, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TemplateDependencyError {
    #[error(transparent)]
    Registry(#[from] TemplateRegistryError),
    #[error(transparent)]
    Install(#[from] TemplateInstallError),
    #[error("template '{template}' has an invalid version '{version}': {source}")]
    InvalidTemplateVersion {
        template: String,
        version: String,
        #[source]
        source: semver::Error,
    },
    #[error(
        "template '{template}' has an invalid version requirement '{requirement}' for dependency '{dependency}': {source}"
    )]
    InvalidRequirement {
        template: String,
        dependency: String,
        requirement: String,
        #[source]
        source: semver::Error,
    },
    #[error(
        "template '{template}' requires '{dependency}' ({requirement}), but it is not installed"
    )]
    MissingDependency {
        template: String,
        dependency: String,
        requirement: String,
    },
    #[error(
        "template '{template}' requires '{dependency}' ({requirement}), but it is not installed and no repository was specified"
    )]
    MissingRepository {
        template: String,
        dependency: String,
        requirement: String,
    },
    #[error(
        "template '{template}' requires '{dependency}' ({requirement}), but installed version {installed} does not satisfy it"
    )]
    VersionMismatch {
        template: String,
        dependency: String,
        requirement: String,
        installed: String,
    },
    #[error("template dependency cycle detected: {0}")]
    Cycle(String),
    #[error("could not roll back newly installed template at '{path}': {source}")]
    Rollback {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Install a template and all of its required dependencies.
///
/// Dependencies are installed recursively from their declared repositories.
/// Optional dependencies are skipped when absent. Existing installations are
/// never replaced. If resolution fails, every template installed by this call
/// is removed, leaving pre-existing installations untouched.
pub fn install_template_with_dependencies(
    registry: &TemplateRegistry,
    name: &str,
    repository: Option<&str>,
) -> Result<InstalledTemplate, TemplateDependencyError> {
    let root = install_template(registry, name, repository)?;
    let mut newly_installed = vec![root.directory.clone()];
    let mut visiting = Vec::new();

    let result = resolve_dependencies(registry, &root, &mut visiting, &mut newly_installed, true);
    if let Err(error) = result {
        for path in newly_installed.into_iter().rev() {
            if path.exists() {
                fs::remove_dir_all(&path)
                    .map_err(|source| TemplateDependencyError::Rollback { path, source })?;
            }
        }
        return Err(error);
    }

    Ok(root)
}

/// Validate the dependency closure of an already-installed template without
/// changing the registry. This is useful for status and project validation.
pub fn validate_template_dependencies(
    registry: &TemplateRegistry,
    name: &str,
) -> Result<(), TemplateDependencyError> {
    let template = registry.get(name)?;
    let mut visiting = Vec::new();
    let mut ignored = Vec::new();
    resolve_dependencies(registry, &template, &mut visiting, &mut ignored, false)
}

fn resolve_dependencies(
    registry: &TemplateRegistry,
    template: &InstalledTemplate,
    visiting: &mut Vec<String>,
    newly_installed: &mut Vec<PathBuf>,
    install_missing: bool,
) -> Result<(), TemplateDependencyError> {
    let name = &template.manifest.name;
    if let Some(position) = visiting.iter().position(|item| item == name) {
        let mut cycle = visiting[position..].to_vec();
        cycle.push(name.clone());
        return Err(TemplateDependencyError::Cycle(cycle.join(" -> ")));
    }
    visiting.push(name.clone());

    let result = (|| {
        for (dependency_name, dependency) in &template.manifest.dependencies {
            let requirement = VersionReq::parse(&dependency.version).map_err(|source| {
                TemplateDependencyError::InvalidRequirement {
                    template: name.clone(),
                    dependency: dependency_name.clone(),
                    requirement: dependency.version.clone(),
                    source,
                }
            })?;

            let installed = match registry.get(dependency_name) {
                Ok(installed) => installed,
                Err(TemplateRegistryError::NotInstalled(_)) if dependency.optional => continue,
                Err(TemplateRegistryError::NotInstalled(_)) if !install_missing => {
                    return Err(TemplateDependencyError::MissingDependency {
                        template: name.clone(),
                        dependency: dependency_name.clone(),
                        requirement: dependency.version.clone(),
                    });
                }
                Err(TemplateRegistryError::NotInstalled(_)) => {
                    let repository = dependency
                        .repository
                        .as_deref()
                        .filter(|repository| !repository.trim().is_empty())
                        .ok_or_else(|| TemplateDependencyError::MissingRepository {
                            template: name.clone(),
                            dependency: dependency_name.clone(),
                            requirement: dependency.version.clone(),
                        })?;
                    let installed = install_template(registry, dependency_name, Some(repository))?;
                    newly_installed.push(installed.directory.clone());
                    installed
                }
                Err(error) => return Err(error.into()),
            };

            let version = Version::parse(&installed.manifest.version).map_err(|source| {
                TemplateDependencyError::InvalidTemplateVersion {
                    template: dependency_name.clone(),
                    version: installed.manifest.version.clone(),
                    source,
                }
            })?;
            if !requirement.matches(&version) {
                return Err(TemplateDependencyError::VersionMismatch {
                    template: name.clone(),
                    dependency: dependency_name.clone(),
                    requirement: dependency.version.clone(),
                    installed: installed.manifest.version.clone(),
                });
            }

            resolve_dependencies(
                registry,
                &installed,
                visiting,
                newly_installed,
                install_missing,
            )?;
        }
        Ok(())
    })();

    visiting.pop();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn add_template(root: &Path, name: &str, version: &str, dependencies: &str) {
        let directory = root.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("template.toml"),
            format!("name = '{name}'\nversion = '{version}'\n{dependencies}"),
        )
        .unwrap();
    }

    #[test]
    fn validates_installed_dependency_version_ranges() {
        let temp = tempdir().unwrap();
        add_template(
            temp.path(),
            "university",
            "1.0.0",
            "[dependencies.common]\nversion = '^2.1'\n",
        );
        add_template(temp.path(), "common", "2.4.1", "");
        let registry = TemplateRegistry::new(temp.path());
        validate_template_dependencies(&registry, "university").unwrap();
    }

    #[test]
    fn rejects_incompatible_installed_dependency() {
        let temp = tempdir().unwrap();
        add_template(
            temp.path(),
            "university",
            "1.0.0",
            "[dependencies.common]\nversion = '^2.0'\n",
        );
        add_template(temp.path(), "common", "3.0.0", "");
        let registry = TemplateRegistry::new(temp.path());
        assert!(matches!(
            validate_template_dependencies(&registry, "university"),
            Err(TemplateDependencyError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn skips_missing_optional_dependency() {
        let temp = tempdir().unwrap();
        add_template(
            temp.path(),
            "university",
            "1.0.0",
            "[dependencies.diagrams]\nversion = '^1.0'\noptional = true\n",
        );
        let registry = TemplateRegistry::new(temp.path());
        validate_template_dependencies(&registry, "university").unwrap();
    }

    #[test]
    fn reports_missing_required_dependency_without_repository() {
        let temp = tempdir().unwrap();
        add_template(
            temp.path(),
            "university",
            "1.0.0",
            "[dependencies.common]\nversion = '^2.0'\n",
        );
        let registry = TemplateRegistry::new(temp.path());
        assert!(matches!(
            validate_template_dependencies(&registry, "university"),
            Err(TemplateDependencyError::MissingDependency { .. })
        ));
    }

    fn git(args: &[&str], cwd: &Path) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .unwrap();
        assert!(status.success(), "git command failed: {args:?}");
    }

    fn local_repository(root: &Path, name: &str, version: &str, manifest_tail: &str) -> PathBuf {
        let repository = root.join(format!("{name}-repository"));
        fs::create_dir_all(&repository).unwrap();
        git(&["init", "-q"], &repository);
        fs::write(
            repository.join("template.toml"),
            format!("name = '{name}'\nversion = '{version}'\n{manifest_tail}"),
        )
        .unwrap();
        fs::write(repository.join("main.tex"), "\\documentclass{article}\n").unwrap();
        git(&["add", "."], &repository);
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
            &repository,
        );
        repository
    }

    #[test]
    fn installs_required_dependencies_recursively_from_declared_repositories() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let temp = tempdir().unwrap();
        let foundation = local_repository(temp.path(), "foundation", "1.4.0", "");
        let foundation_path = foundation.to_string_lossy();
        let common_tail = format!(
            "[dependencies.foundation]\nversion = '^1.0'\nrepository = \"{}\"\n",
            foundation_path.replace('\\', "\\\\").replace('\"', "\\\"")
        );
        let common = local_repository(temp.path(), "common", "2.3.0", &common_tail);
        let common_path = common.to_string_lossy();
        let university_tail = format!(
            "[dependencies.common]\nversion = '^2.0'\nrepository = \"{}\"\n",
            common_path.replace('\\', "\\\\").replace('\"', "\\\"")
        );
        let university = local_repository(temp.path(), "university", "1.0.0", &university_tail);
        let registry = TemplateRegistry::new(temp.path().join("installed"));

        let installed = install_template_with_dependencies(
            &registry,
            "university",
            Some(university.to_str().unwrap()),
        )
        .unwrap();

        assert_eq!(installed.manifest.name, "university");
        assert_eq!(registry.get("common").unwrap().manifest.version, "2.3.0");
        assert_eq!(
            registry.get("foundation").unwrap().manifest.version,
            "1.4.0"
        );
    }

    #[test]
    fn rolls_back_root_install_when_a_required_dependency_cannot_be_resolved() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let temp = tempdir().unwrap();
        let university = local_repository(
            temp.path(),
            "university",
            "1.0.0",
            "[dependencies.common]\nversion = '^2.0'\n",
        );
        let registry = TemplateRegistry::new(temp.path().join("installed"));

        assert!(matches!(
            install_template_with_dependencies(
                &registry,
                "university",
                Some(university.to_str().unwrap()),
            ),
            Err(TemplateDependencyError::MissingRepository { .. })
        ));
        assert!(matches!(
            registry.get("university"),
            Err(TemplateRegistryError::NotInstalled(_))
        ));
    }

    #[test]
    fn detects_dependency_cycles() {
        let temp = tempdir().unwrap();
        add_template(
            temp.path(),
            "alpha",
            "1.0.0",
            "[dependencies.beta]\nversion = '^1.0'\n",
        );
        add_template(
            temp.path(),
            "beta",
            "1.0.0",
            "[dependencies.alpha]\nversion = '^1.0'\n",
        );
        let registry = TemplateRegistry::new(temp.path());
        assert!(matches!(
            validate_template_dependencies(&registry, "alpha"),
            Err(TemplateDependencyError::Cycle(_))
        ));
    }
}
