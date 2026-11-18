//! Compilation of several documents, optionally in parallel.

use super::{
    CommandRunner, CompileError, CompileJob, CompileReport, Compiler, compile_document,
    output_dir_for,
};
use crate::config::ResolvedCompilerConfig;
use crate::project::Project;
use rayon::prelude::*;
use std::collections::HashMap;

/// The outcome of one job (a document with one theme) of a batch.
#[derive(Debug)]
pub struct DocumentResult {
    pub job: CompileJob,
    /// `Err` means compilation could not be attempted; a TeX failure is an
    /// `Ok` report whose outcome is `Failed`.
    pub result: Result<CompileReport, CompileError>,
}

impl DocumentResult {
    pub fn is_success(&self) -> bool {
        matches!(&self.result, Ok(report) if report.is_success())
    }
}

/// Results for every document of a batch, in the order the documents were given.
#[derive(Debug)]
pub struct BatchReport {
    pub results: Vec<DocumentResult>,
}

impl BatchReport {
    pub fn succeeded(&self) -> usize {
        self.results.iter().filter(|r| r.is_success()).count()
    }

    pub fn failed(&self) -> usize {
        self.results.len() - self.succeeded()
    }
}

/// Number of documents compiled at the same time.
///
/// One when parallelism is disabled; otherwise `max_workers`, but never more
/// than the number of documents and never less than one.
pub fn worker_count(settings: &ResolvedCompilerConfig, documents: usize) -> usize {
    if !settings.parallel {
        return 1;
    }
    settings.max_workers.clamp(1, documents.max(1))
}

/// Compile `jobs`, running up to [`worker_count`] at once.
///
/// `on_complete` is called from the worker thread as soon as each document
/// finishes, with the job's index in `jobs`; calls for different
/// documents may therefore arrive in any order and may overlap. The returned
/// report is always in input order. A failing document never stops the others.
///
/// Fails before compiling anything if two jobs would share an output
/// directory, since concurrent latexmk runs would corrupt each other.
pub fn compile_documents<R, F>(
    project: &Project,
    compiler: &Compiler<R>,
    jobs: &[CompileJob],
    force: bool,
    on_complete: F,
) -> Result<BatchReport, CompileError>
where
    R: CommandRunner,
    F: Fn(usize, &DocumentResult) + Sync,
{
    check_output_collisions(project, jobs)?;

    let workers = worker_count(compiler.settings(), jobs.len());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .map_err(|error| CompileError::WorkerPool(error.to_string()))?;

    let results = pool.install(|| {
        jobs.par_iter()
            .enumerate()
            .map(|(index, job)| {
                let result = DocumentResult {
                    job: job.clone(),
                    result: compile_document(project, compiler, job, force),
                };
                on_complete(index, &result);
                result
            })
            .collect()
    });
    Ok(BatchReport { results })
}

fn check_output_collisions(project: &Project, jobs: &[CompileJob]) -> Result<(), CompileError> {
    let mut seen: HashMap<_, &CompileJob> = HashMap::new();
    for job in jobs {
        let directory = output_dir_for(project, job);
        if let Some(first) = seen.insert(directory.clone(), job) {
            return Err(CompileError::OutputCollision {
                first: first.document.relative_path.clone(),
                second: job.document.relative_path.clone(),
                directory,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{CommandOutput, CommandSpec};
    use crate::config::ConfigStore;
    use std::{
        fs, io,
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };
    use tempfile::{TempDir, tempdir};

    /// Simulates latexmk: sleeps, tracks peak concurrency, fails for stems
    /// starting with "bad".
    #[derive(Default)]
    struct SlowRunner {
        running: AtomicUsize,
        peak: AtomicUsize,
    }

    impl CommandRunner for &SlowRunner {
        fn run(&self, spec: &CommandSpec) -> io::Result<CommandOutput> {
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(60));
            self.running.fetch_sub(1, Ordering::SeqCst);

            let stem = spec
                .args
                .last()
                .unwrap()
                .to_str()
                .unwrap()
                .trim_end_matches(".tex")
                .to_string();
            let outdir = spec
                .args
                .iter()
                .find_map(|a| a.to_str()?.strip_prefix("-outdir=").map(PathBuf::from))
                .unwrap();
            fs::create_dir_all(&outdir)?;
            fs::write(outdir.join(format!("{stem}.log")), b"log")?;
            if stem.starts_with("bad") {
                return Ok(CommandOutput {
                    exit_code: Some(12),
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            fs::write(outdir.join(format!("{stem}.pdf")), b"%PDF")?;
            Ok(CommandOutput {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn project_with(files: &[&str], config: &str) -> (TempDir, Project, Vec<CompileJob>) {
        let dir = tempdir().unwrap();
        let root = dir.path();
        for file in files {
            let path = root.join("src").join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        fs::write(root.join("latex_handler.toml"), config).unwrap();
        let project = Project::load(root, &ConfigStore::new(root.join("global"))).unwrap();
        let documents = project
            .documents()
            .unwrap()
            .into_iter()
            .map(CompileJob::new)
            .collect();
        (dir, project, documents)
    }

    const SIX: [&str; 6] = [
        "a_main.tex",
        "b_main.tex",
        "c_main.tex",
        "d_main.tex",
        "e_main.tex",
        "f_main.tex",
    ];

    #[test]
    fn worker_count_respects_configuration_and_document_count() {
        let mut settings = crate::config::Config::resolve_layers(&[]).compiler;
        assert_eq!(worker_count(&settings, 20), 8);
        assert_eq!(worker_count(&settings, 3), 3);
        assert_eq!(worker_count(&settings, 0), 1);
        settings.max_workers = 0;
        assert_eq!(worker_count(&settings, 5), 1);
        settings.max_workers = 4;
        settings.parallel = false;
        assert_eq!(worker_count(&settings, 5), 1);
    }

    #[test]
    fn documents_compile_concurrently_up_to_the_worker_limit() {
        let (_dir, project, documents) = project_with(&SIX, "[compiler]\nmax_workers = 3\n");
        let runner = SlowRunner::default();
        let compiler = Compiler::with_runner(project.config().compiler.clone(), &runner);

        let report = compile_documents(&project, &compiler, &documents, false, |_, _| {}).unwrap();

        assert_eq!(report.succeeded(), 6);
        let peak = runner.peak.load(Ordering::SeqCst);
        assert!(peak > 1, "expected parallel execution, peak was {peak}");
        assert!(peak <= 3, "worker limit exceeded, peak was {peak}");
    }

    #[test]
    fn parallelism_can_be_disabled() {
        let (_dir, project, documents) = project_with(&SIX, "[compiler]\nparallel = false\n");
        let runner = SlowRunner::default();
        let compiler = Compiler::with_runner(project.config().compiler.clone(), &runner);

        compile_documents(&project, &compiler, &documents, false, |_, _| {}).unwrap();

        assert_eq!(runner.peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn partial_failures_are_collected_and_results_keep_input_order() {
        let files = ["a_main.tex", "bad_main.tex", "c_main.tex", "d_main.tex"];
        let (_dir, project, documents) = project_with(&files, "");
        let runner = SlowRunner::default();
        let compiler = Compiler::with_runner(project.config().compiler.clone(), &runner);
        let completed = Mutex::new(Vec::new());

        let report = compile_documents(&project, &compiler, &documents, false, |index, result| {
            completed.lock().unwrap().push((index, result.is_success()));
        })
        .unwrap();

        assert_eq!((report.succeeded(), report.failed()), (3, 1));
        let order: Vec<_> = report
            .results
            .iter()
            .map(|r| r.job.document.relative_path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            order,
            ["a_main.tex", "bad_main.tex", "c_main.tex", "d_main.tex"]
        );
        assert!(!report.results[1].is_success());

        let mut completed = completed.into_inner().unwrap();
        completed.sort();
        assert_eq!(completed, [(0, true), (1, false), (2, true), (3, true)]);
        assert!(
            project.root().join("build/bad_main/bad_main.log").is_file(),
            "logs must survive a failed compilation"
        );
    }

    #[test]
    fn shared_output_directories_are_rejected_before_compiling() {
        let files = ["one/x_main.tex", "two/x_main.tex"];
        let (_dir, project, documents) =
            project_with(&files, "[output]\nmirror_structure = false\n");
        let runner = SlowRunner::default();
        let compiler = Compiler::with_runner(project.config().compiler.clone(), &runner);

        let error =
            compile_documents(&project, &compiler, &documents, false, |_, _| {}).unwrap_err();

        assert!(matches!(error, CompileError::OutputCollision { .. }));
        assert_eq!(
            runner.peak.load(Ordering::SeqCst),
            0,
            "nothing may run after a collision"
        );
    }

    #[test]
    fn mirrored_layout_allows_documents_with_the_same_name() {
        let files = ["one/x_main.tex", "two/x_main.tex"];
        let (_dir, project, documents) = project_with(&files, "");
        let runner = SlowRunner::default();
        let compiler = Compiler::with_runner(project.config().compiler.clone(), &runner);

        let report = compile_documents(&project, &compiler, &documents, false, |_, _| {}).unwrap();
        assert_eq!(report.succeeded(), 2);
        assert!(project.root().join("build/one/x_main/x_main.pdf").is_file());
        assert!(project.root().join("build/two/x_main/x_main.pdf").is_file());
    }
}
