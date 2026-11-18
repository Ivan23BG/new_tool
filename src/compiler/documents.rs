//! Project-level document selection and compilation.

use super::{CommandRunner, CompileError, CompileReport, CompileRequest, Compiler};
use crate::project::{DiscoveredDocument, Project};
use std::path::{Path, PathBuf};

/// Directory receiving the intermediate files and PDF for `document`.
///
/// With `output.mirror_structure` (the default) this mirrors the document's
/// position under the source directory: `src/modules/ex_main.tex` builds in
/// `build/modules/ex_main/`. Otherwise every document builds in
/// `build/<stem>/`.
pub fn output_dir_for(project: &Project, document: &DiscoveredDocument) -> PathBuf {
    let config = project.config();
    let stem = document.relative_path.file_stem().unwrap_or_default();
    let mut dir = project.root().join(&config.structure.build);
    let parent = document
        .relative_path
        .parent()
        .filter(|_| config.output.mirror_structure);
    if let Some(parent) = parent {
        dir.push(parent);
    }
    dir.push(stem);
    dir
}

/// Decide which documents to compile.
///
/// With no `files`, every discovered document is selected. Explicit paths may
/// be absolute, or relative to `base_dir` (normally the current directory),
/// the project root, or the source directory, tried in that order. Selected
/// documents must live under the source directory so their output can mirror
/// their location. Duplicates are removed, preserving first occurrence.
pub fn select_documents(
    project: &Project,
    files: &[PathBuf],
    base_dir: &Path,
) -> Result<Vec<DiscoveredDocument>, CompileError> {
    if files.is_empty() {
        let documents = project.documents()?;
        return if documents.is_empty() {
            Err(CompileError::NoDocuments)
        } else {
            Ok(documents)
        };
    }

    let source = project.source_dir();
    let source = source.canonicalize().unwrap_or(source);
    let mut selected: Vec<DiscoveredDocument> = Vec::new();
    for file in files {
        let candidates: Vec<PathBuf> = if file.is_absolute() {
            vec![file.clone()]
        } else {
            vec![
                base_dir.join(file),
                project.root().join(file),
                source.join(file),
            ]
        };
        let found = candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| CompileError::DocumentNotFound(file.clone()))?;
        let found = found.canonicalize().unwrap_or(found);
        let relative =
            found
                .strip_prefix(&source)
                .map_err(|_| CompileError::DocumentOutsideSource {
                    document: found.clone(),
                    source_dir: source.clone(),
                })?;
        if selected.iter().all(|existing| existing.path != found) {
            selected.push(DiscoveredDocument {
                relative_path: relative.to_path_buf(),
                path: found,
            });
        }
    }
    Ok(selected)
}

/// Compile one document of `project` with `compiler`.
pub fn compile_document<R: CommandRunner>(
    project: &Project,
    compiler: &Compiler<R>,
    document: &DiscoveredDocument,
    force: bool,
) -> Result<CompileReport, CompileError> {
    let mut request = CompileRequest::new(&document.path, output_dir_for(project, document));
    request.force = force;
    compiler.compile(&request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{CommandOutput, CommandSpec, CompileOutcome};
    use crate::config::ConfigStore;
    use std::{fs, io};
    use tempfile::{TempDir, tempdir};

    struct FakeRunner;
    impl CommandRunner for FakeRunner {
        fn run(&self, spec: &CommandSpec) -> io::Result<CommandOutput> {
            let outdir = spec
                .args
                .iter()
                .find_map(|a| a.to_str()?.strip_prefix("-outdir=").map(PathBuf::from))
                .unwrap();
            let stem = spec
                .args
                .last()
                .unwrap()
                .to_str()
                .unwrap()
                .trim_end_matches(".tex")
                .to_string();
            fs::create_dir_all(&outdir)?;
            fs::write(outdir.join(format!("{stem}.pdf")), b"%PDF")?;
            Ok(CommandOutput {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn sample_project(config: &str) -> (TempDir, Project) {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/modules")).unwrap();
        fs::write(root.join("src/main.tex"), "").unwrap();
        fs::write(root.join("src/modules/ex_main.tex"), "").unwrap();
        fs::write(root.join("src/modules/notes.tex"), "").unwrap();
        fs::write(root.join("latex_handler.toml"), config).unwrap();
        let project = Project::load(root, &ConfigStore::new(root.join("global"))).unwrap();
        (dir, project)
    }

    fn names(documents: &[DiscoveredDocument]) -> Vec<String> {
        documents
            .iter()
            .map(|d| d.relative_path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn output_directories_mirror_the_source_layout() {
        let (_dir, project) = sample_project("");
        let documents = project.documents().unwrap();
        let outputs: Vec<PathBuf> = documents
            .iter()
            .map(|d| {
                output_dir_for(&project, d)
                    .strip_prefix(project.root())
                    .unwrap()
                    .to_path_buf()
            })
            .collect();
        assert_eq!(
            outputs,
            vec![
                PathBuf::from("build/main"),
                PathBuf::from("build/modules/ex_main")
            ]
        );
    }

    #[test]
    fn flat_output_when_mirroring_is_disabled() {
        let (_dir, project) = sample_project("[output]\nmirror_structure = false\n");
        let documents = project.documents().unwrap();
        let output = output_dir_for(&project, &documents[1]);
        assert_eq!(output, project.root().join("build/ex_main"));
    }

    #[test]
    fn no_files_selects_every_discovered_document() {
        let (_dir, project) = sample_project("");
        let selected = select_documents(&project, &[], project.root()).unwrap();
        assert_eq!(names(&selected), vec!["main.tex", "modules/ex_main.tex"]);
    }

    #[test]
    fn explicit_files_resolve_from_cwd_root_or_source_and_deduplicate() {
        let (_dir, project) = sample_project("");
        let elsewhere = tempdir().unwrap();
        let files = [
            PathBuf::from("src/modules/notes.tex"), // relative to project root
            PathBuf::from("main.tex"),              // relative to source dir
            project.root().join("src/main.tex"),    // absolute duplicate
        ];
        let selected = select_documents(&project, &files, elsewhere.path()).unwrap();
        assert_eq!(names(&selected), vec!["modules/notes.tex", "main.tex"]);
    }

    #[test]
    fn missing_and_outside_documents_are_rejected() {
        let (_dir, project) = sample_project("");
        assert!(matches!(
            select_documents(&project, &[PathBuf::from("nope.tex")], project.root()),
            Err(CompileError::DocumentNotFound(_))
        ));
        let outside = tempdir().unwrap();
        let stray = outside.path().join("stray.tex");
        fs::write(&stray, "").unwrap();
        assert!(matches!(
            select_documents(&project, &[stray], project.root()),
            Err(CompileError::DocumentOutsideSource { .. })
        ));
    }

    #[test]
    fn empty_discovery_is_an_error() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        let project =
            Project::load(dir.path(), &ConfigStore::new(dir.path().join("global"))).unwrap();
        assert!(matches!(
            select_documents(&project, &[], dir.path()),
            Err(CompileError::NoDocuments)
        ));
    }

    #[test]
    fn compile_document_builds_in_the_mirrored_directory() {
        let (_dir, project) = sample_project("");
        let compiler = Compiler::with_runner(project.config().compiler.clone(), FakeRunner);
        let documents = project.documents().unwrap();
        let report = compile_document(&project, &compiler, &documents[1], false).unwrap();

        assert_eq!(report.outcome, CompileOutcome::Succeeded);
        assert_eq!(
            report.pdf.unwrap(),
            project.root().join("build/modules/ex_main/ex_main.pdf")
        );
    }
}
