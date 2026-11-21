//! Project maintenance reporting: generated file sizes and summaries.

use std::path::Path;

/// Total size of the files below `path`, in bytes.
///
/// Returns `None` when the path is missing or is not a directory, so callers
/// can skip generated directories that were never created.
pub fn directory_bytes(path: &Path) -> Option<u64> {
    if !path.is_dir() {
        return None;
    }
    let mut total = 0_u64;
    for entry in walkdir::WalkDir::new(path).into_iter() {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(metadata) = entry.metadata() {
            total = total.saturating_add(metadata.len());
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn directory_bytes_sums_files_and_skips_missing_paths() {
        let dir = tempdir().unwrap();
        assert_eq!(directory_bytes(dir.path()), Some(0));

        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.txt"), b"12345").unwrap();
        fs::write(dir.path().join("sub/b.txt"), b"1234").unwrap();
        assert_eq!(directory_bytes(dir.path()), Some(9));

        assert_eq!(directory_bytes(&dir.path().join("missing")), None);
    }
}
