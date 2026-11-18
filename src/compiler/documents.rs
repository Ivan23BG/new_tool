//! Project-level document selection, job planning, and compilation.

use super::{
    CommandRunner, CompileError, CompileReport, CompileRequest, Compiler, output::publish_outputs,
    theme_name,
};
use crate::config::ThemeVariant;
use crate::project::{DiscoveredDocument, Project};
use std::path::{Path, PathBuf};

/// One build: a document compiled with one theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileJob {
    pub document: DiscoveredDocument,
    /// Theme exposed to the document as `\lhtheme`. `None` when themes are
    /// disabled (`themes.enabled` is empty).
    pub theme: Option<ThemeVariant>,
}

impl CompileJob {
    /// A job without a theme.
    pub fn new(document: DiscoveredDocument) -> Self {
        Self {
            document,
            theme: None,
        }
    }

    /// Human-readable name: the document path, tagged with its theme when
    /// that is not the configured default.
    pub fn label(&self, project: &Project) -> String {
        let name = self.document.relative_path.display().to_string();
        match self.theme {
            Some(theme) if theme != project.config().themes.default => {
                format!("{name} [{}]", theme_name(theme))
            }
            _ => name,
        }
    }
}

/// Which themes a compilation should produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeSelection {
    /// Only the configured default theme.
    #[default]
    Default,
    /// One specific enabled theme.
    Only(ThemeVariant),
    /// Every enabled theme.
    All,
}

/// Expand documents into jobs according to the theme selection.
///
/// The default theme keeps the plain output names (`pdfs/main.pdf`); other
/// themes are suffixed (`pdfs/main-dark.pdf`). When `themes.enabled` is empty
/// themes are off: documents get no `\lhtheme` definition and `--theme` is an
/// error.
pub fn plan_jobs(
    project: &Project,
    documents: &[DiscoveredDocument],
    selection: ThemeSelection,
) -> Result<Vec<CompileJob>, CompileError> {
    let themes = &project.config().themes;
    let mut enabled: Vec<ThemeVariant> = Vec::new();
    for theme in &themes.enabled {
        if !enabled.contains(theme) {
            enabled.push(*theme);
        }
    }

    let chosen: Vec<Option<ThemeVariant>> = if enabled.is_empty() {
        match selection {
            ThemeSelection::Default => vec![None],
            _ => return Err(CompileError::ThemesDisabled),
        }
    } else {
        match selection {
            ThemeSelection::Default => vec![Some(themes.default)],
            ThemeSelection::All => enabled.iter().copied().map(Some).collect(),
            ThemeSelection::Only(theme) => {
                if !enabled.contains(&theme) {
                    return Err(CompileError::ThemeNotEnabled {
                        theme: theme_name(theme).to_string(),
                        enabled: enabled
                            .iter()
                            .map(|t| theme_name(*t))
                            .collect::<Vec<_>>()
                            .join(", "),
                    });
                }
                vec![Some(theme)]
            }
        }
    };

    Ok(documents
        .iter()
        .flat_map(|document| {
            chosen.iter().map(move |theme| CompileJob {
                document: document.clone(),
                theme: *theme,
            })
        })
        .collect())
}

/// File-name stem of a job's published outputs: the document's stem, with
/// `-<theme>` appended for themes other than the configured default.
pub fn output_stem(project: &Project, job: &CompileJob) -> String {
    let stem = job
        .document
        .relative_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    match job.theme {
        Some(theme) if theme != project.config().themes.default => {
            format!("{stem}-{}", theme_name(theme))
        }
        _ => stem.into_owned(),
    }
}

/// `<base>/<document's directory under the source dir>` when mirroring is on,
/// otherwise just `<base>`.
fn mirrored_dir(project: &Project, base: &Path, job: &CompileJob) -> PathBuf {
    let mut dir = project.root().join(base);
    let parent = job
        .document
        .relative_path
        .parent()
        .filter(|_| project.config().output.mirror_structure);
    if let Some(parent) = parent {
        dir.push(parent);
    }
    dir
}

/// Directory receiving the intermediate files for `job`:
/// `build/<mirrored path>/<stem>/`.
pub fn output_dir_for(project: &Project, job: &CompileJob) -> PathBuf {
    mirrored_dir(project, &project.config().structure.build, job).join(output_stem(project, job))
}

/// Final PDF location: `pdfs/<mirrored path>/<stem>.pdf`.
pub fn pdf_path_for(project: &Project, job: &CompileJob) -> PathBuf {
    mirrored_dir(project, &project.config().structure.pdf, job)
        .join(format!("{}.pdf", output_stem(project, job)))
}

/// Final log location: `logs/<mirrored path>/<stem>.log`.
pub fn log_path_for(project: &Project, job: &CompileJob) -> PathBuf {
    mirrored_dir(project, &project.config().structure.logs, job)
        .join(format!("{}.log", output_stem(project, job)))
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

/// Compile one job and publish its results.
///
/// latexmk works in `build/…`; afterwards the log is copied to `logs/…`
/// (always) and, if the build succeeded, the PDF to `pdfs/…`.
pub fn compile_document<R: CommandRunner>(
    project: &Project,
    compiler: &Compiler<R>,
    job: &CompileJob,
    force: bool,
) -> Result<CompileReport, CompileError> {
    let mut request = CompileRequest::new(&job.document.path, output_dir_for(project, job));
    request.force = force;
    request.theme = job.theme;
    let mut report = compiler.compile(&request)?;
    publish_outputs(project, job, &mut report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{CommandOutput, CommandSpec, CompileOutcome};
    use crate::config::ConfigStore;
    use std::{fs, io};
    use tempfile::{TempDir, tempdir};

    /// Simulates latexmk: writes `<stem>.pdf`, `.log` and `.synctex.gz`; fails
    /// for stems starting with "bad" (log only).
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
            fs::write(outdir.join(format!("{stem}.log")), b"log")?;
            if stem.starts_with("bad") {
                return Ok(CommandOutput {
                    exit_code: Some(12),
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            let theme = spec
                .args
                .iter()
                .any(|a| a.to_str().is_some_and(|a| a.contains("{dark}")));
            fs::write(
                outdir.join(format!("{stem}.pdf")),
                if theme { b"dark" as &[u8] } else { b"std" },
            )?;
            fs::write(outdir.join(format!("{stem}.synctex.gz")), b"sync")?;
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
        fs::write(root.join("src/bad_main.tex"), "").unwrap();
        fs::write(root.join("src/modules/ex_main.tex"), "").unwrap();
        fs::write(root.join("src/modules/notes.tex"), "").unwrap();
        fs::write(root.join("latex_handler.toml"), config).unwrap();
        let project = Project::load(root, &ConfigStore::new(root.join("global"))).unwrap();
        (dir, project)
    }

    fn doc(project: &Project, name: &str) -> DiscoveredDocument {
        project
            .documents()
            .unwrap()
            .into_iter()
            .find(|d| d.relative_path.ends_with(name))
            .unwrap()
    }

    fn rel(project: &Project, path: PathBuf) -> String {
        path.strip_prefix(project.root())
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn names(documents: &[DiscoveredDocument]) -> Vec<String> {
        documents
            .iter()
            .map(|d| d.relative_path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn locations_mirror_the_source_layout() {
        let (_dir, project) = sample_project("");
        let job = CompileJob::new(doc(&project, "modules/ex_main.tex"));
        assert_eq!(
            rel(&project, output_dir_for(&project, &job)),
            "build/modules/ex_main"
        );
        assert_eq!(
            rel(&project, pdf_path_for(&project, &job)),
            "pdfs/modules/ex_main.pdf"
        );
        assert_eq!(
            rel(&project, log_path_for(&project, &job)),
            "logs/modules/ex_main.log"
        );
    }

    #[test]
    fn flat_locations_when_mirroring_is_disabled() {
        let (_dir, project) = sample_project("[output]\nmirror_structure = false\n");
        let job = CompileJob::new(doc(&project, "modules/ex_main.tex"));
        assert_eq!(
            rel(&project, output_dir_for(&project, &job)),
            "build/ex_main"
        );
        assert_eq!(
            rel(&project, pdf_path_for(&project, &job)),
            "pdfs/ex_main.pdf"
        );
    }

    #[test]
    fn default_theme_keeps_plain_names_and_others_are_suffixed() {
        let (_dir, project) = sample_project("");
        let document = doc(&project, "main.tex");
        let light = CompileJob {
            document: document.clone(),
            theme: Some(ThemeVariant::Light),
        };
        let dark = CompileJob {
            document,
            theme: Some(ThemeVariant::Dark),
        };
        assert_eq!(
            rel(&project, pdf_path_for(&project, &light)),
            "pdfs/main.pdf"
        );
        assert_eq!(
            rel(&project, pdf_path_for(&project, &dark)),
            "pdfs/main-dark.pdf"
        );
        assert_eq!(
            rel(&project, output_dir_for(&project, &dark)),
            "build/main-dark"
        );
        assert_eq!(dark.label(&project), "main.tex [dark]");
        assert_eq!(light.label(&project), "main.tex");
    }

    #[test]
    fn plan_jobs_expands_documents_by_selected_themes() {
        let (_dir, project) = sample_project("");
        let documents = vec![doc(&project, "main.tex"), doc(&project, "ex_main.tex")];
        let themes = |selection| -> Vec<(String, Option<ThemeVariant>)> {
            plan_jobs(&project, &documents, selection)
                .unwrap()
                .into_iter()
                .map(|j| {
                    (
                        j.document.relative_path.to_string_lossy().into_owned(),
                        j.theme,
                    )
                })
                .collect()
        };
        assert_eq!(themes(ThemeSelection::Default).len(), 2);
        assert!(
            themes(ThemeSelection::Default)
                .iter()
                .all(|(_, t)| *t == Some(ThemeVariant::Light))
        );
        assert_eq!(themes(ThemeSelection::Only(ThemeVariant::Dark)).len(), 2);
        let all = themes(ThemeSelection::All);
        assert_eq!(all.len(), 4);
        assert_eq!(all[0], ("main.tex".to_string(), Some(ThemeVariant::Light)));
        assert_eq!(all[1], ("main.tex".to_string(), Some(ThemeVariant::Dark)));
    }

    #[test]
    fn themes_respect_the_enabled_list() {
        let (_dir, project) = sample_project("[themes]\nenabled = [\"light\"]\n");
        let documents = vec![doc(&project, "main.tex")];
        assert!(matches!(
            plan_jobs(
                &project,
                &documents,
                ThemeSelection::Only(ThemeVariant::Dark)
            ),
            Err(CompileError::ThemeNotEnabled { .. })
        ));
        assert_eq!(
            plan_jobs(&project, &documents, ThemeSelection::All)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn empty_enabled_list_turns_themes_off() {
        let (_dir, project) = sample_project("[themes]\nenabled = []\n");
        let documents = vec![doc(&project, "main.tex")];
        let jobs = plan_jobs(&project, &documents, ThemeSelection::Default).unwrap();
        assert_eq!(jobs[0].theme, None);
        assert!(matches!(
            plan_jobs(&project, &documents, ThemeSelection::All),
            Err(CompileError::ThemesDisabled)
        ));
    }

    #[test]
    fn no_files_selects_every_discovered_document() {
        let (_dir, project) = sample_project("");
        let selected = select_documents(&project, &[], project.root()).unwrap();
        assert_eq!(
            names(&selected),
            vec!["bad_main.tex", "main.tex", "modules/ex_main.tex"]
        );
    }

    #[test]
    fn explicit_files_resolve_from_cwd_root_or_source_and_deduplicate() {
        let (_dir, project) = sample_project("");
        let elsewhere = tempdir().unwrap();
        let files = [
            PathBuf::from("src/modules/notes.tex"),
            PathBuf::from("main.tex"),
            project.root().join("src/main.tex"),
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

    fn compiler(project: &Project) -> Compiler<FakeRunner> {
        Compiler::with_runner(project.config().compiler.clone(), FakeRunner)
    }

    #[test]
    fn success_publishes_pdf_synctex_and_log() {
        let (_dir, project) = sample_project("");
        let job = CompileJob::new(doc(&project, "modules/ex_main.tex"));
        let report = compile_document(&project, &compiler(&project), &job, false).unwrap();

        assert_eq!(report.outcome, CompileOutcome::Succeeded);
        let root = project.root();
        assert_eq!(
            report.final_pdf.as_deref(),
            Some(root.join("pdfs/modules/ex_main.pdf").as_path())
        );
        assert!(root.join("pdfs/modules/ex_main.synctex.gz").is_file());
        assert!(root.join("logs/modules/ex_main.log").is_file());
        assert!(
            root.join("build/modules/ex_main/ex_main.pdf").is_file(),
            "intermediates kept by default"
        );
        assert!(!root.join("pdfs/modules/ex_main.pdf.tmp").exists());
    }

    #[test]
    fn failure_publishes_the_log_but_keeps_the_last_good_pdf() {
        let (_dir, project) = sample_project("");
        let job = CompileJob::new(doc(&project, "bad_main.tex"));
        let root = project.root();
        fs::create_dir_all(root.join("pdfs")).unwrap();
        fs::write(root.join("pdfs/bad_main.pdf"), b"previous good build").unwrap();

        let report = compile_document(&project, &compiler(&project), &job, false).unwrap();

        assert_eq!(
            report.outcome,
            CompileOutcome::Failed {
                exit_code: Some(12)
            }
        );
        assert!(report.final_pdf.is_none());
        assert_eq!(
            report.final_log.as_deref(),
            Some(root.join("logs/bad_main.log").as_path())
        );
        assert!(root.join("logs/bad_main.log").is_file());
        assert_eq!(
            fs::read(root.join("pdfs/bad_main.pdf")).unwrap(),
            b"previous good build"
        );
    }

    #[test]
    fn themed_builds_publish_under_suffixed_names_with_the_theme_applied() {
        let (_dir, project) = sample_project("");
        let document = doc(&project, "main.tex");
        for theme in [ThemeVariant::Light, ThemeVariant::Dark] {
            let job = CompileJob {
                document: document.clone(),
                theme: Some(theme),
            };
            compile_document(&project, &compiler(&project), &job, false).unwrap();
        }
        let root = project.root();
        assert_eq!(fs::read(root.join("pdfs/main.pdf")).unwrap(), b"std");
        assert_eq!(fs::read(root.join("pdfs/main-dark.pdf")).unwrap(), b"dark");
        assert!(root.join("pdfs/main-dark.synctex.gz").is_file());
        assert!(root.join("logs/main-dark.log").is_file());
    }

    #[test]
    fn intermediates_are_removed_after_success_when_not_kept() {
        let (_dir, project) = sample_project("[output]\nkeep_intermediates = false\n");
        let root = project.root().to_path_buf();
        let good = CompileJob::new(doc(&project, "main.tex"));
        let report = compile_document(&project, &compiler(&project), &good, false).unwrap();
        assert!(!root.join("build/main").exists());
        assert!(root.join("pdfs/main.pdf").is_file());
        assert!(root.join("logs/main.log").is_file());
        assert!(report.pdf.is_none() && report.final_pdf.is_some());

        let bad = CompileJob::new(doc(&project, "bad_main.tex"));
        compile_document(&project, &compiler(&project), &bad, false).unwrap();
        assert!(
            root.join("build/bad_main").exists(),
            "failed builds keep intermediates for debugging"
        );
    }
}
