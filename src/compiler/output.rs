//! Publishing build results to the project's `pdfs/` and `logs/` directories.

use super::{
    CompileError, CompileJob, CompileReport, log_path_for, output_dir_for, output_stem,
    pdf_path_for,
};
use crate::project::Project;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Copy the results of a finished build to their final locations.
///
/// * The log is always published, including after a failed build.
/// * The PDF (and its SyncTeX file, so editors can find it next to the PDF)
///   is published only after a successful build; a failed build never
///   replaces the last good PDF.
/// * With `output.keep_intermediates = false`, the build directory is
///   removed after a successful build.
pub(super) fn publish_outputs(
    project: &Project,
    job: &CompileJob,
    report: &mut CompileReport,
) -> Result<(), CompileError> {
    let build_dir = output_dir_for(project, job);

    if let Some(log) = &report.log {
        let destination = log_path_for(project, job);
        copy_atomic(log, &destination)?;
        report.final_log = Some(destination);
    }
    if !report.is_success() {
        return Ok(());
    }

    if let Some(pdf) = &report.pdf {
        let destination = pdf_path_for(project, job);
        copy_atomic(pdf, &destination)?;

        let built_stem = job
            .document
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let synctex = build_dir.join(format!("{built_stem}.synctex.gz"));
        if synctex.is_file() {
            let name = format!("{}.synctex.gz", output_stem(project, job));
            copy_atomic(&synctex, &destination.with_file_name(name))?;
        }
        report.final_pdf = Some(destination);
    }

    if !project.config().output.keep_intermediates {
        fs::remove_dir_all(&build_dir).map_err(|source| publish_error(&build_dir, source))?;
        report.pdf = None;
        report.log = None;
    }
    Ok(())
}

/// Copy via a temporary name so a viewer never sees a half-written file.
fn copy_atomic(source: &Path, destination: &Path) -> Result<(), CompileError> {
    let parent = destination.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| publish_error(parent, error))?;
    let mut temporary = destination.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::copy(source, &temporary).map_err(|error| publish_error(&temporary, error))?;
    fs::rename(&temporary, destination).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        publish_error(destination, error)
    })
}

fn publish_error(path: &Path, source: io::Error) -> CompileError {
    CompileError::Publish {
        path: path.to_path_buf(),
        source,
    }
}
