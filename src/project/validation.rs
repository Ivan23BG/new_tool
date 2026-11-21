use super::Project;
use std::path::Component;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectValidation {
    pub issues: Vec<ValidationIssue>,
}

impl ProjectValidation {
    pub fn is_valid(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|issue| issue.severity == ValidationSeverity::Error)
    }
}

pub fn validate_project(project: &Project) -> ProjectValidation {
    let mut report = ProjectValidation::default();
    let config = project.config();
    let source = project.root().join(&config.structure.source);
    if !source.is_dir() {
        report.issues.push(ValidationIssue {
            severity: ValidationSeverity::Warning,
            message: format!(
                "source directory '{}' does not exist; discovery will use the project root",
                config.structure.source.display()
            ),
        });
    }
    for (label, path) in [
        ("build", &config.structure.build),
        ("PDF", &config.structure.pdf),
        ("logs", &config.structure.logs),
        ("assets", &config.structure.assets),
    ] {
        if path.is_absolute() {
            report.issues.push(ValidationIssue {
                severity: ValidationSeverity::Error,
                message: format!(
                    "{label} directory '{}' must be relative to the project root",
                    path.display()
                ),
            });
        } else if path.components().any(|part| part == Component::ParentDir) {
            report.issues.push(ValidationIssue {
                severity: ValidationSeverity::Warning,
                message: format!(
                    "{label} directory '{}' escapes the project root",
                    path.display()
                ),
            });
        }
    }
    if config.discovery.patterns.is_empty() && config.discovery.enabled {
        report.issues.push(ValidationIssue {
            severity: ValidationSeverity::Warning,
            message: "automatic discovery is enabled but no filename patterns are configured"
                .into(),
        });
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::ConfigStore, project::Project};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn missing_source_is_warning_not_error() {
        let dir = tempdir().unwrap();
        let project =
            Project::load(dir.path(), &ConfigStore::new(dir.path().join("global"))).unwrap();
        let report = validate_project(&project);
        assert!(report.is_valid());
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.message.contains("source directory"))
        );
    }

    #[test]
    fn absolute_output_path_is_an_error() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("latex_handler.toml"),
            "[structure]\nbuild = '/tmp/build'\n",
        )
        .unwrap();
        let project =
            Project::load(dir.path(), &ConfigStore::new(dir.path().join("global"))).unwrap();
        assert!(!validate_project(&project).is_valid());
    }
}
