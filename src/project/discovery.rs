use super::{Project, ProjectError};
use std::path::PathBuf;
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDocument {
    /// Path relative to the project's configured source directory.
    pub relative_path: PathBuf,
    /// Absolute path to the TeX entry point.
    pub path: PathBuf,
}

pub fn discover_documents(project: &Project) -> Result<Vec<DiscoveredDocument>, ProjectError> {
    let rules = &project.config().discovery;
    if !rules.enabled {
        return Ok(Vec::new());
    }
    let source = project.source_dir();
    let mut documents = Vec::new();
    let walker = WalkDir::new(&source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_excluded(entry, &source, &rules.exclude));

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(&source).unwrap_or(entry.path());
        let name = entry.file_name().to_string_lossy();
        if rules
            .patterns
            .iter()
            .any(|pattern| matches_pattern(&name, pattern))
        {
            documents.push(DiscoveredDocument {
                relative_path: relative.to_path_buf(),
                path: entry.path().to_path_buf(),
            });
        }
    }
    documents.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(documents)
}

fn is_excluded(entry: &DirEntry, source: &std::path::Path, exclusions: &[String]) -> bool {
    // Exclusions are project-relative directory/file names. Inspecting the
    // absolute path would accidentally exclude projects under `/tmp`, for
    // example, because `tmp` is a default exclusion.
    let path = entry.path().strip_prefix(source).unwrap_or(entry.path());
    path.components().any(|component| {
        let component = component.as_os_str().to_string_lossy();
        exclusions
            .iter()
            .any(|excluded| component == excluded.as_str())
    })
}

/// Match the deliberately small glob syntax used by discovery: `*` matches
/// any sequence of characters within a single path component.
fn matches_pattern(value: &str, pattern: &str) -> bool {
    let (mut vi, mut pi, mut star, mut retry) = (0, 0, None, 0);
    let v = value.as_bytes();
    let p = pattern.as_bytes();
    while vi < v.len() {
        if pi < p.len() && p[pi] == v[vi] {
            vi += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            pi += 1;
            retry = vi;
        } else if let Some(star_at) = star {
            retry += 1;
            vi = retry;
            pi = star_at + 1;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wildcard_matches_only_filename_patterns() {
        assert!(matches_pattern("chapter_main.tex", "*_main.tex"));
        assert!(matches_pattern("main.tex", "main.tex"));
        assert!(!matches_pattern("chapter.tex", "*_main.tex"));
        assert!(!matches_pattern("chapter_main.tex.bak", "*_main.tex"));
    }
}
