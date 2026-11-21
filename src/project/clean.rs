//! Safe removal of generated project outputs.
//!
//! Cleaning only ever removes directories configured under `structure.build`,
//! `structure.logs`, and (with `--all`) `structure.pdf`. Paths that are
//! absolute, escape the project root, point at the project root itself, or
//! contain the source/assets directories are rejected instead of deleted.

use super::Project;
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

/// Which generated directories a clean should remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanSelection {
    build: bool,
    logs: bool,
    pdfs: bool,
}

impl CleanSelection {
    /// Remove only build intermediates.
    pub const fn build() -> Self {
        Self {
            build: true,
            logs: false,
            pdfs: false,
        }
    }

    /// Remove only published logs.
    pub const fn logs() -> Self {
        Self {
            build: false,
            logs: true,
            pdfs: false,
        }
    }

    /// Remove build intermediates, logs, and published PDFs.
    pub const fn all() -> Self {
        Self {
            build: true,
            logs: true,
            pdfs: true,
        }
    }
}

impl Default for CleanSelection {
    /// The default clean removes intermediates and logs but preserves PDFs.
    fn default() -> Self {
        Self {
            build: true,
            logs: true,
            pdfs: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum CleanError {
    #[error("refusing to clean {label} directory '{path}': {reason}")]
    UnsafeTarget {
        label: &'static str,
        path: PathBuf,
        reason: &'static str,
    },
    #[error("refusing to clean {label} path '{path}': it is not a directory")]
    NotDirectory { label: &'static str, path: PathBuf },
    #[error("could not remove '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Remove the selected generated directories.
///
/// Missing directories are skipped, so cleaning an already clean project
/// succeeds. The returned paths are the directories that were removed.
pub fn clean_project(
    project: &Project,
    selection: CleanSelection,
) -> Result<Vec<PathBuf>, CleanError> {
    let mut removed = Vec::new();
    for (label, target) in clean_targets(project, selection)? {
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CleanError::UnsafeTarget {
                    label,
                    path: target,
                    reason: "symbolic links are not followed",
                });
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(CleanError::NotDirectory {
                    label,
                    path: target,
                });
            }
            Ok(_) => {
                fs::remove_dir_all(&target).map_err(|source| CleanError::Io {
                    path: target.clone(),
                    source,
                })?;
                removed.push(target);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CleanError::Io {
                    path: target,
                    source,
                });
            }
        }
    }
    Ok(removed)
}

/// Plan which generated directories a clean would remove, without touching
/// the filesystem.
///
/// Safety checks still run, so unsafe or invalid targets are reported even in
/// dry-run mode. Missing directories are skipped; only directories that exist
/// are planned for removal.
pub fn plan_clean(
    project: &Project,
    selection: CleanSelection,
) -> Result<Vec<PathBuf>, CleanError> {
    let mut planned = Vec::new();
    for (label, target) in clean_targets(project, selection)? {
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CleanError::UnsafeTarget {
                    label,
                    path: target,
                    reason: "symbolic links are not followed",
                });
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(CleanError::NotDirectory {
                    label,
                    path: target,
                });
            }
            Ok(_) => planned.push(target),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CleanError::Io {
                    path: target,
                    source,
                });
            }
        }
    }
    Ok(planned)
}

/// Resolve the generated directories selected by `selection` and enforce the
/// safety checks that [`clean_project`] relies on.
fn clean_targets(
    project: &Project,
    selection: CleanSelection,
) -> Result<Vec<(&'static str, PathBuf)>, CleanError> {
    let structure = &project.config().structure;
    let mut targets = Vec::new();
    if selection.build {
        targets.push(("build", structure.build.clone()));
    }
    if selection.logs {
        targets.push(("logs", structure.logs.clone()));
    }
    if selection.pdfs {
        targets.push(("PDF", structure.pdf.clone()));
    }
    let mut resolved = Vec::new();
    for (label, relative) in targets {
        resolved.push((label, safe_target(project, label, &relative)?));
    }
    Ok(resolved)
}

/// Resolve a configured directory and reject targets that could delete user
/// content or escape the project.
fn safe_target(
    project: &Project,
    label: &'static str,
    relative: &Path,
) -> Result<PathBuf, CleanError> {
    let mut components = relative.components();
    let valid = matches!(components.next(), Some(Component::Normal(_)))
        && components.all(|component| matches!(component, Component::Normal(_)));
    if !valid {
        return Err(CleanError::UnsafeTarget {
            label,
            path: relative.to_path_buf(),
            reason: "the path must be a non-empty relative path without '.' or '..'",
        });
    }

    let root = project.root();
    let target = root.join(relative);
    if target == root {
        return Err(CleanError::UnsafeTarget {
            label,
            path: target,
            reason: "it would remove the project root",
        });
    }
    for (protected_label, protected) in [
        ("source", project.source_dir()),
        ("assets", project.assets_dir()),
    ] {
        if target == protected || protected.starts_with(&target) {
            return Err(CleanError::UnsafeTarget {
                label,
                path: target,
                reason: if protected_label == "source" {
                    "it would remove the source directory or one of its parents"
                } else {
                    "it would remove the assets directory or one of its parents"
                },
            });
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigStore;
    use std::fs;
    use tempfile::tempdir;

    fn project(config: &str) -> (tempfile::TempDir, Project) {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join("assets")).unwrap();
        fs::create_dir_all(dir.path().join("build/doc")).unwrap();
        fs::create_dir_all(dir.path().join("logs/doc")).unwrap();
        fs::create_dir_all(dir.path().join("pdfs/doc")).unwrap();
        fs::write(dir.path().join("build/doc/main.aux"), "aux").unwrap();
        fs::write(dir.path().join("logs/doc/main.log"), "log").unwrap();
        fs::write(dir.path().join("pdfs/doc/main.pdf"), "pdf").unwrap();
        fs::write(dir.path().join("src/main.tex"), "source").unwrap();
        fs::write(dir.path().join("assets/logo.png"), "asset").unwrap();
        fs::write(dir.path().join("latex_handler.toml"), config).unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        let project = Project::load(dir.path(), &store).unwrap();
        (dir, project)
    }

    #[test]
    fn default_clean_removes_build_and_logs_but_keeps_pdfs() {
        let (dir, project) = project("");
        let removed = clean_project(&project, CleanSelection::default()).unwrap();

        assert_eq!(removed.len(), 2);
        assert!(!dir.path().join("build").exists());
        assert!(!dir.path().join("logs").exists());
        assert!(dir.path().join("pdfs/doc/main.pdf").is_file());
        assert!(dir.path().join("src/main.tex").is_file());
        assert!(dir.path().join("assets/logo.png").is_file());
    }

    #[test]
    fn each_flag_removes_only_its_own_outputs() {
        let (dir, project) = project("");

        clean_project(&project, CleanSelection::build()).unwrap();
        assert!(!dir.path().join("build").exists());
        assert!(dir.path().join("logs/doc/main.log").is_file());
        assert!(dir.path().join("pdfs/doc/main.pdf").is_file());

        clean_project(&project, CleanSelection::logs()).unwrap();
        assert!(!dir.path().join("logs").exists());
        assert!(dir.path().join("pdfs/doc/main.pdf").is_file());

        clean_project(&project, CleanSelection::all()).unwrap();
        assert!(!dir.path().join("pdfs").exists());
        assert!(dir.path().join("src/main.tex").is_file());
        assert!(dir.path().join("assets/logo.png").is_file());
    }

    #[test]
    fn missing_directories_are_skipped() {
        let (dir, project) = project("");
        clean_project(&project, CleanSelection::all()).unwrap();
        let removed = clean_project(&project, CleanSelection::all()).unwrap();
        assert!(removed.is_empty());
        assert!(dir.path().join("src/main.tex").is_file());
    }

    #[test]
    fn plan_clean_lists_existing_outputs_without_removing_them() {
        let (_dir, project) = project("");
        let planned = plan_clean(&project, CleanSelection::default()).unwrap();

        assert_eq!(planned.len(), 2);
        assert!(
            planned
                .iter()
                .any(|path| path.ends_with("build") && path.is_dir())
        );
        assert!(
            planned
                .iter()
                .any(|path| path.ends_with("logs") && path.is_dir())
        );
        // PDFs are preserved by the default selection.
        assert!(!planned.iter().any(|path| path.ends_with("pdfs")));
        // The plan is read-only: nothing was deleted.
        assert!(project.root().join("build/doc/main.aux").is_file());
        assert!(project.root().join("logs/doc/main.log").is_file());
        assert!(project.root().join("pdfs/doc/main.pdf").is_file());
    }

    #[test]
    fn plan_clean_all_includes_the_pdf_directory() {
        let (_dir, project) = project("");
        let planned = plan_clean(&project, CleanSelection::all()).unwrap();

        assert_eq!(planned.len(), 3);
        assert!(planned.iter().any(|path| path.ends_with("pdfs")));
    }

    #[test]
    fn plan_clean_skips_missing_directories() {
        let (_dir, project) = project("");
        let planned = plan_clean(&project, CleanSelection::all()).unwrap();

        assert_eq!(planned.len(), 3);
        clean_project(&project, CleanSelection::all()).unwrap();
        assert!(
            plan_clean(&project, CleanSelection::all())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn plan_clean_reports_unsafe_targets_without_removing_them() {
        let (_dir, project) = project("[structure]\nbuild = 'src'\n");

        assert!(matches!(
            plan_clean(&project, CleanSelection::build()),
            Err(CleanError::UnsafeTarget { .. })
        ));
        assert!(project.root().join("src/main.tex").is_file());
    }

    #[test]
    fn configured_subdirectories_are_respected() {
        let (dir, project) =
            project("[structure]\nbuild = 'out/build'\nlogs = 'out/logs'\npdf = 'out/pdfs'\n");
        fs::create_dir_all(dir.path().join("out/build/doc")).unwrap();
        fs::create_dir_all(dir.path().join("out/logs/doc")).unwrap();
        fs::create_dir_all(dir.path().join("out/pdfs/doc")).unwrap();

        let removed = clean_project(&project, CleanSelection::all()).unwrap();
        assert_eq!(removed.len(), 3);
        assert!(!dir.path().join("out/build").exists());
        assert!(!dir.path().join("out/logs").exists());
        assert!(!dir.path().join("out/pdfs").exists());
        assert!(dir.path().join("src/main.tex").is_file());
    }

    #[test]
    fn refuses_absolute_escaping_and_project_root_targets() {
        for (config, expected) in [
            ("[structure]\nbuild = '/tmp/outside'\n", "relative"),
            ("[structure]\nbuild = '../outside'\n", "relative"),
            ("[structure]\nbuild = '.'\n", "relative"),
            ("[structure]\nbuild = ''\n", "relative"),
        ] {
            let (_dir, project) = project(config);
            let error = clean_project(&project, CleanSelection::build()).unwrap_err();
            assert!(
                matches!(error, CleanError::UnsafeTarget { .. }),
                "{config}: {error}"
            );
            assert!(error.to_string().contains(expected), "{config}: {error}");
        }
    }

    #[test]
    fn refuses_targets_that_would_remove_source_or_assets() {
        for config in [
            "[structure]\nbuild = 'src'\n",
            "[structure]\nbuild = 'assets'\n",
            "[structure]\nbuild = '.'\n",
        ] {
            let (_dir, project) = project(config);
            assert!(matches!(
                clean_project(&project, CleanSelection::build()),
                Err(CleanError::UnsafeTarget { .. })
            ));
        }
    }

    #[test]
    fn refuses_a_file_or_symlink_at_the_output_path() {
        let (dir, project) = project("");
        fs::remove_dir_all(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build"), "not a directory").unwrap();
        assert!(matches!(
            clean_project(&project, CleanSelection::build()),
            Err(CleanError::NotDirectory { .. })
        ));
        assert!(dir.path().join("build").is_file());
    }
}
