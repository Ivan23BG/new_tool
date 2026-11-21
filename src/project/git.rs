//! Optional Git integration for projects.
//!
//! The Git helpers are intentionally thin wrappers over the `git` command and
//! never make Git a requirement: callers decide how to report a missing or
//! non-repository context.

use std::path::Path;
use thiserror::Error;

/// A file that differs between the working tree and the index/HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitChange {
    /// Repository-relative path of the changed file.
    pub path: std::path::PathBuf,
    /// What kind of change was detected.
    pub kind: ChangeKind,
}

/// The kind of Git change detected for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Unmerged,
    Untracked,
}

impl ChangeKind {
    /// Short human-readable label used by reports.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Unmerged => "unmerged",
            Self::Untracked => "untracked",
        }
    }
}

/// The Git state of a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    /// Whether the project lives inside a Git working tree.
    pub is_repository: bool,
    /// Current branch; `None` for detached HEAD or when unavailable.
    pub branch: Option<String>,
    /// Whether the working tree has no pending changes.
    pub clean: bool,
    /// Changes detected in the working tree.
    pub changes: Vec<GitChange>,
}

/// Errors reported by the optional Git integration.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("could not execute git; ensure Git is installed and available on PATH: {0}")]
    GitUnavailable(#[source] std::io::Error),
    #[error("git operation failed: {0}")]
    GitFailed(String),
}

/// Inspect the Git state of a project directory.
///
/// Returns a status with `is_repository: false` when the directory is not part
/// of a Git working tree; Git stays optional this way.
pub fn status(root: &Path) -> Result<GitStatus, GitError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .output()
        .map_err(GitError::GitUnavailable)?;
    if !output.status.success() {
        return Ok(GitStatus {
            is_repository: false,
            branch: None,
            clean: true,
            changes: Vec::new(),
        });
    }
    if String::from_utf8_lossy(&output.stdout).trim() != "true" {
        return Ok(GitStatus {
            is_repository: false,
            branch: None,
            clean: true,
            changes: Vec::new(),
        });
    }

    let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "HEAD")
        .or_else(|| {
            git(root, &["symbolic-ref", "--short", "HEAD"])
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });

    let status_output = git(root, &["status", "--porcelain"])?;
    let changes = parse_porcelain(&status_output);
    let clean = changes.is_empty();

    Ok(GitStatus {
        is_repository: true,
        branch,
        clean,
        changes,
    })
}

/// Initialize a Git repository inside `root`.
///
/// Fails when the directory already belongs to a Git working tree.
pub fn init(root: &Path) -> Result<(), GitError> {
    if status(root)?.is_repository {
        return Err(GitError::GitFailed(
            "the directory is already inside a git repository".into(),
        ));
    }
    let output = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .map_err(GitError::GitUnavailable)?;
    if !output.status.success() {
        return Err(GitError::GitFailed(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(GitError::GitUnavailable)?;
    if !output.status.success() {
        return Err(GitError::GitFailed(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse `git status --porcelain` output into classified changes.
///
/// Porcelain lines look like `XY PATH` or, for renames, `XY OLD -> NEW`, where
/// `X`/`Y` describe the index/worktree states (a leading space means "unchanged
/// in that column").
fn parse_porcelain(output: &str) -> Vec<GitChange> {
    let mut changes = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let Some(status_field) = line.get(..2) else {
            continue;
        };
        let path_part = line[2..].trim_start();
        if path_part.is_empty() {
            continue;
        }
        let new_path = path_part
            .split(" -> ")
            .last()
            .unwrap_or(path_part)
            .to_string();
        let path = std::path::PathBuf::from(&new_path);
        let x = status_field.chars().next().unwrap_or(' ');
        let y = status_field.chars().nth(1).unwrap_or(' ');
        let kind = if x == '?' && y == '?' {
            ChangeKind::Untracked
        } else if x == 'R' || y == 'R' || x == 'C' || y == 'C' {
            ChangeKind::Renamed
        } else if x == 'D' || y == 'D' {
            ChangeKind::Deleted
        } else if x == 'A' || y == 'A' {
            ChangeKind::Added
        } else if x == 'U' || y == 'U' {
            ChangeKind::Unmerged
        } else if x == 'M' || y == 'M' {
            ChangeKind::Modified
        } else {
            ChangeKind::Modified
        };
        changes.push(GitChange { path, kind });
    }
    changes
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

    fn repository(root: &Path) -> std::path::PathBuf {
        let repo = root.join("project");
        fs::create_dir_all(&repo).unwrap();
        git(&["init", "-q", "-b", "main"], &repo);
        git(
            &[
                "-c",
                "user.name=tests",
                "-c",
                "user.email=tests@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
            &repo,
        );
        repo
    }

    #[test]
    fn status_reports_a_clean_repository() {
        let root = tempdir().unwrap();
        let repo = repository(root.path());
        let status = status(&repo).unwrap();

        assert!(status.is_repository);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(status.clean);
        assert!(status.changes.is_empty());
    }

    #[test]
    fn status_reports_untracked_and_modified_files() {
        let root = tempdir().unwrap();
        let repo = repository(root.path());
        fs::write(repo.join("src_main.tex"), "one\n").unwrap();
        fs::write(repo.join("src_other.tex"), "two\n").unwrap();
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
                "add sources",
            ],
            &repo,
        );
        fs::write(repo.join("src_main.tex"), "changed\n").unwrap();
        fs::write(repo.join("src_untracked.tex"), "new\n").unwrap();

        let status = status(&repo).unwrap();
        assert!(!status.clean);
        assert_eq!(status.changes.len(), 2);
        assert!(status.changes.iter().any(|change| {
            change.kind == ChangeKind::Modified && change.path == Path::new("src_main.tex")
        }));
        assert!(status.changes.iter().any(|change| {
            change.kind == ChangeKind::Untracked && change.path == Path::new("src_untracked.tex")
        }));
    }

    #[test]
    fn init_refuses_to_reinitialize_an_existing_repository() {
        let root = tempdir().unwrap();
        let repo = repository(root.path());
        assert!(status(&repo).unwrap().is_repository);
        assert!(init(&repo).is_err());
    }
}
