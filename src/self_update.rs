//! Optional self-update of the `latex_handler` binary from GitHub releases.
//!
//! The check and download shell out to tools that are already part of the
//! project's runtime assumptions (`git` for release discovery, `curl` for
//! downloads, `tar` for extraction), so no additional crate dependencies are
//! required.

use semver::Version;
use std::{fs, process::Command};
use thiserror::Error;

/// Result of a self-update run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfUpdateResult {
    /// The version that was current before the run.
    pub current_version: String,
    /// The latest published release version, when one was found.
    pub latest_version: Option<String>,
    /// Whether a replacement binary was installed during this run.
    pub updated: bool,
}

/// Errors reported by the optional self-update.
#[derive(Debug, Error)]
pub enum SelfUpdateError {
    #[error("no published release was found for the latex_handler repository")]
    NoRelease,
    #[error("could not determine the latest release: {0}")]
    DetermineLatest(String),
    #[error("could not download the release archive: {0}")]
    Download(String),
    #[error("could not extract the release archive: {0}")]
    Extract(String),
    #[error("could not replace the running binary: {0}")]
    Replace(String),
}

/// Run a self-update, or only check for a newer release when `check_only` is
/// set.
///
/// The running binary is replaced atomically (write to a temporary file next
/// to the executable, then rename) and failures leave the previous binary in
/// place.
pub fn self_update(check_only: bool) -> Result<SelfUpdateResult, SelfUpdateError> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let latest_version = latest_release_version()?;
    let result = SelfUpdateResult {
        current_version: current_version.clone(),
        latest_version: latest_version.clone(),
        updated: false,
    };
    if check_only {
        return Ok(result);
    }
    let Some(latest_version) = latest_version else {
        return Err(SelfUpdateError::NoRelease);
    };
    let current = Version::parse(&current_version).unwrap_or_else(|_| Version::new(0, 0, 0));
    if let Some(latest) = Version::parse(&latest_version).ok() {
        if latest > current {
            install_release(&latest_version)?;
            return Ok(SelfUpdateResult {
                current_version,
                latest_version: Some(latest_version),
                updated: true,
            });
        }
    }
    Ok(SelfUpdateResult {
        current_version,
        latest_version: Some(latest_version),
        updated: false,
    })
}

/// The latest published stable release tag of the latex_handler repository.
pub fn latest_release_tag() -> Result<String, SelfUpdateError> {
    let output = Command::new("git")
        .args([
            "ls-remote",
            "--tags",
            "https://github.com/Ivan23BG/latex_handler.git",
        ])
        .output()
        .map_err(|source| SelfUpdateError::DetermineLatest(source.to_string()))?;
    if !output.status.success() {
        return Err(SelfUpdateError::DetermineLatest(
            "git ls-remote failed".to_string(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut best: Option<(String, Version)> = None;
    for line in stdout.lines() {
        let Some(tag) = line.split_whitespace().nth(1) else {
            continue;
        };
        let Some(tag) = tag.strip_prefix("refs/tags/") else {
            continue;
        };
        let Some(version) = version_from_tag(tag) else {
            continue;
        };
        let better = match &best {
            Some((_, best_version)) => version > *best_version,
            None => true,
        };
        if better {
            best = Some((tag.to_string(), version));
        }
    }
    best.map(|(tag, _)| tag).ok_or(SelfUpdateError::NoRelease)
}

/// The latest published stable release version, without a leading `v`.
pub fn latest_release_version() -> Result<Option<String>, SelfUpdateError> {
    let tag = latest_release_tag()?;
    Ok(Some(
        tag.strip_prefix('v')
            .filter(|version| !version.is_empty())
            .unwrap_or(&tag)
            .to_string(),
    ))
}

/// Map a release tag to a semantic version.
pub fn version_from_tag(tag: &str) -> Option<Version> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(version).ok()
}

/// Download the release archive for this platform and replace the running
/// binary with it.
fn install_release(version: &str) -> Result<(), SelfUpdateError> {
    let asset = release_asset().ok_or_else(|| {
        SelfUpdateError::Download(format!(
            "no release asset is published for this platform ({version})"
        ))
    })?;
    let tag = format!("v{version}");
    let archive_url =
        format!("https://github.com/Ivan23BG/latex_handler/releases/download/{tag}/{asset}");
    let archive_path = std::env::temp_dir().join(format!("latex_handler-update-{version}{asset}"));
    let download = Command::new("curl")
        .args(["-sSL", "--fail", &archive_url, "-o"])
        .arg(&archive_path)
        .output()
        .map_err(|source| SelfUpdateError::Download(source.to_string()))?;
    if !download.status.success() {
        return Err(SelfUpdateError::Download(format!(
            "curl exited with {}",
            download.status
        )));
    }
    let extract_dir = std::env::temp_dir().join(format!("latex_handler-update-{version}"));
    fs::create_dir_all(&extract_dir)
        .map_err(|source| SelfUpdateError::Extract(source.to_string()))?;
    let extract = Command::new("tar")
        .args(["-xzf", &archive_path.to_string_lossy(), "-C"])
        .arg(&extract_dir)
        .arg(if cfg!(windows) {
            "latex_handler.exe"
        } else {
            "latex_handler"
        })
        .output()
        .map_err(|source| SelfUpdateError::Extract(source.to_string()))?;
    if !extract.status.success() {
        return Err(SelfUpdateError::Extract(
            "tar extraction failed".to_string(),
        ));
    }
    let binary = extract_dir.join(if cfg!(windows) {
        "latex_handler.exe"
    } else {
        "latex_handler"
    });
    let bytes = fs::read(&binary).map_err(|source| SelfUpdateError::Extract(source.to_string()))?;
    let exe =
        std::env::current_exe().map_err(|source| SelfUpdateError::Replace(source.to_string()))?;
    let staged = exe.with_file_name("latex_handler.selfupdate.tmp");
    fs::write(&staged, &bytes).map_err(|source| SelfUpdateError::Replace(source.to_string()))?;
    make_executable(&staged)?;
    fs::rename(&staged, &exe).map_err(|source| SelfUpdateError::Replace(source.to_string()))?;
    let _ = fs::remove_dir_all(&extract_dir);
    let _ = fs::remove_file(&archive_path);
    Ok(())
}

/// Make a staged binary executable on Unix platforms.
#[cfg(unix)]
fn make_executable(path: &std::path::Path) -> Result<(), SelfUpdateError> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(|source| SelfUpdateError::Replace(source.to_string()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|source| SelfUpdateError::Replace(source.to_string()))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &std::path::Path) -> Result<(), SelfUpdateError> {
    Ok(())
}

/// The release asset matching the current platform, when self-update is
/// supported here.
fn release_asset() -> Option<&'static str> {
    if cfg!(windows) {
        Some("latex_handler-windows.zip")
    } else if cfg!(target_os = "linux") {
        Some("latex_handler-linux.tar.gz")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_tag_parses_release_tags() {
        assert_eq!(version_from_tag("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(version_from_tag("v0.2.0"), Some(Version::new(0, 2, 0)));
        assert!(version_from_tag("not-a-version").is_none());
    }
}
