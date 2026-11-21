//! Synchronization of a project with its source template.
//!
//! The snapshot records the template baseline. A sync compares three states
//! for every managed path:
//!
//! * the snapshot fingerprint (the template version the project was created
//!   from),
//! * the current project file (the user's working copy), and
//! * the installed template file (the target version).
//!
//! Safe updates are applied automatically. A file modified on both sides is a
//! conflict and is never overwritten unless the caller explicitly asks for the
//! template version. Files that are not managed by the template are never
//! touched.

use super::{
    PROJECT_SNAPSHOT_FILE, Project, ProjectSnapshot, ResourceRecord, SnapshotError,
    fingerprint_bytes, key_path, path_key,
};
use crate::template::{InstalledTemplate, TemplateRegistry, TemplateRegistryError};
use semver::Version;
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;
use walkdir::WalkDir;

/// How to handle conflicts during [`apply_sync`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    /// Leave conflicts untouched and report them.
    #[default]
    Skip,
    /// Keep the user's file and record the template version as the new
    /// baseline, so the local change becomes an ordinary local modification.
    PreferLocal,
    /// Replace the user's file with the template version. This is explicit
    /// and may discard local work.
    PreferTemplate,
}

/// Why a locally modified resource is being kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    /// The template did not change this file; the user did.
    LocalModified,
    /// The template removed the file, but the user modified it. It is kept and
    /// becomes an unmanaged user file.
    TemplateRemovedLocallyModified,
}

/// Why a path is a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    BothModified,
    DeletedLocallyModifiedRemotely,
    AddedLocallyAndRemotely,
}

impl ConflictKind {
    pub fn description(self) -> &'static str {
        match self {
            Self::BothModified => "modified locally and in the template",
            Self::DeletedLocallyModifiedRemotely => "deleted locally but modified in the template",
            Self::AddedLocallyAndRemotely => "created locally and added by the template",
        }
    }
}

/// One planned synchronization action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    /// Copy a resource that is new in the template.
    Add { path: String, template_path: String },
    /// Replace a resource that is unchanged locally but newer in the template.
    Update { path: String, template_path: String },
    /// The local file already matches the target; only refresh the baseline.
    Adopt { path: String, template_path: String },
    /// Remove a resource that the template removed and the user did not touch.
    Remove { path: String },
    /// Keep a local modification.
    KeepLocal { path: String, reason: KeepReason },
    /// The user deleted a resource and the template did not change it.
    KeepDeleted { path: String },
    /// Both sides changed; requires an explicit policy.
    Conflict {
        path: String,
        kind: ConflictKind,
        template_path: Option<String>,
    },
    /// Nothing to do.
    Unchanged { path: String },
}

impl SyncAction {
    pub fn path(&self) -> &str {
        match self {
            Self::Add { path, .. }
            | Self::Update { path, .. }
            | Self::Adopt { path, .. }
            | Self::Remove { path }
            | Self::KeepLocal { path, .. }
            | Self::KeepDeleted { path }
            | Self::Conflict { path, .. }
            | Self::Unchanged { path } => path,
        }
    }

    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict { .. })
    }

    pub fn is_change(&self) -> bool {
        matches!(
            self,
            Self::Add { .. } | Self::Update { .. } | Self::Adopt { .. } | Self::Remove { .. }
        )
    }
}

/// Summary counts for a synchronization plan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncCounts {
    pub added: usize,
    pub updated: usize,
    pub adopted: usize,
    pub removed: usize,
    pub kept: usize,
    pub conflicts: usize,
    pub unchanged: usize,
}

impl SyncCounts {
    pub fn changes(&self) -> usize {
        self.added + self.updated + self.adopted + self.removed
    }
}

/// A computed synchronization plan. Inspect it before applying.
#[derive(Debug, Clone)]
pub struct SyncPlan {
    pub template: String,
    pub from_version: String,
    pub to_version: String,
    pub actions: Vec<SyncAction>,
    project_root: PathBuf,
    target_template_root: PathBuf,
    target_shared_assets: Option<String>,
    snapshot: ProjectSnapshot,
}

impl SyncPlan {
    pub fn counts(&self) -> SyncCounts {
        let mut counts = SyncCounts::default();
        for action in &self.actions {
            match action {
                SyncAction::Add { .. } => counts.added += 1,
                SyncAction::Update { .. } => counts.updated += 1,
                SyncAction::Adopt { .. } => counts.adopted += 1,
                SyncAction::Remove { .. } => counts.removed += 1,
                SyncAction::KeepLocal { .. } | SyncAction::KeepDeleted { .. } => counts.kept += 1,
                SyncAction::Conflict { .. } => counts.conflicts += 1,
                SyncAction::Unchanged { .. } => counts.unchanged += 1,
            }
        }
        counts
    }

    pub fn has_changes(&self) -> bool {
        self.actions.iter().any(SyncAction::is_change)
    }

    pub fn has_conflicts(&self) -> bool {
        self.actions.iter().any(SyncAction::is_conflict)
    }
}

/// Result of applying a plan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncOutcome {
    pub applied: usize,
    pub resolved: usize,
    pub conflicts_remaining: usize,
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Registry(#[from] TemplateRegistryError),
    #[error(
        "this project has no template snapshot; synchronization requires a project created with `latex_handler new`"
    )]
    MissingSnapshot,
    #[error("this project is not associated with a template")]
    MissingTemplate,
    #[error("project template '{project}' does not match the snapshot template '{snapshot}'")]
    TemplateMismatch { project: String, snapshot: String },
    #[error(
        "project is pinned to template version {pinned}, but the installed template is {installed}; install the pinned version first"
    )]
    PinnedVersionMismatch { pinned: String, installed: String },
    #[error("could not inspect template resources: {0}")]
    Walk(#[from] walkdir::Error),
    #[error("could not read '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not apply synchronization to '{path}': {source}")]
    Apply {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("'{path}' changed after the synchronization plan was created; rerun the plan")]
    ChangedDuringSync { path: String },
}

/// Compare a project with its installed source template.
pub fn plan_sync(project: &Project, registry: &TemplateRegistry) -> Result<SyncPlan, SyncError> {
    let snapshot =
        ProjectSnapshot::load_optional(project.root())?.ok_or(SyncError::MissingSnapshot)?;
    let project_template = project
        .config()
        .project
        .template
        .as_deref()
        .ok_or(SyncError::MissingTemplate)?;
    if project_template != snapshot.template {
        return Err(SyncError::TemplateMismatch {
            project: project_template.to_owned(),
            snapshot: snapshot.template,
        });
    }

    let installed = registry.get(&snapshot.template)?;
    let policy = project
        .config()
        .project
        .template_version
        .as_deref()
        .unwrap_or("latest");
    if policy != "latest" && !same_version(policy, &installed.manifest.version) {
        return Err(SyncError::PinnedVersionMismatch {
            pinned: policy.to_owned(),
            installed: installed.manifest.version.clone(),
        });
    }

    let target_shared_assets = installed
        .manifest
        .shared_assets
        .as_deref()
        .map(path_key)
        .transpose()?;
    let mut remote_files = template_files(&installed)?;
    let mut actions = Vec::new();

    for (key, record) in &snapshot.resources {
        let remote_path = remote_files.remove(&record.template_path);
        let remote_fingerprint = remote_path.as_deref().map(fingerprint_path).transpose()?;
        let local_path = project.root().join(key_path(key)?);
        let local_fingerprint = fingerprint_if_exists(&local_path)?;
        let template_path = record.template_path.clone();
        let action = classify_managed(
            key,
            record,
            local_fingerprint.as_deref(),
            remote_fingerprint.as_deref(),
            template_path,
        );
        actions.push(action);
    }

    for (template_path, path) in remote_files {
        let key = template_path.clone();
        let remote_fingerprint = fingerprint_path(&path)?;
        let local_path = project.root().join(key_path(&key)?);
        let local_fingerprint = fingerprint_if_exists(&local_path)?;
        let action = match local_fingerprint {
            None => SyncAction::Add {
                path: key,
                template_path,
            },
            Some(local) if local == remote_fingerprint => SyncAction::Adopt {
                path: key,
                template_path,
            },
            Some(_) => SyncAction::Conflict {
                path: key,
                kind: ConflictKind::AddedLocallyAndRemotely,
                template_path: Some(template_path),
            },
        };
        actions.push(action);
    }

    actions.sort_by(|left, right| left.path().cmp(right.path()));
    Ok(SyncPlan {
        template: snapshot.template.clone(),
        from_version: snapshot.template_version.clone(),
        to_version: installed.manifest.version.clone(),
        actions,
        project_root: project.root().to_path_buf(),
        target_template_root: installed.directory,
        target_shared_assets,
        snapshot,
    })
}

fn classify_managed(
    key: &str,
    record: &ResourceRecord,
    local: Option<&str>,
    remote: Option<&str>,
    template_path: String,
) -> SyncAction {
    match (local, remote) {
        (Some(local), Some(remote)) if local == record.fingerprint => {
            if remote == record.fingerprint {
                SyncAction::Unchanged {
                    path: key.to_owned(),
                }
            } else {
                SyncAction::Update {
                    path: key.to_owned(),
                    template_path,
                }
            }
        }
        (Some(local), Some(remote)) => {
            if local == remote {
                SyncAction::Adopt {
                    path: key.to_owned(),
                    template_path,
                }
            } else if remote == record.fingerprint {
                SyncAction::KeepLocal {
                    path: key.to_owned(),
                    reason: KeepReason::LocalModified,
                }
            } else {
                SyncAction::Conflict {
                    path: key.to_owned(),
                    kind: ConflictKind::BothModified,
                    template_path: Some(template_path),
                }
            }
        }
        (Some(local), None) => {
            if local == record.fingerprint {
                SyncAction::Remove {
                    path: key.to_owned(),
                }
            } else {
                SyncAction::KeepLocal {
                    path: key.to_owned(),
                    reason: KeepReason::TemplateRemovedLocallyModified,
                }
            }
        }
        (None, Some(remote)) => {
            if remote == record.fingerprint {
                SyncAction::KeepDeleted {
                    path: key.to_owned(),
                }
            } else {
                SyncAction::Conflict {
                    path: key.to_owned(),
                    kind: ConflictKind::DeletedLocallyModifiedRemotely,
                    template_path: Some(template_path),
                }
            }
        }
        (None, None) => SyncAction::Remove {
            path: key.to_owned(),
        },
    }
}

/// Apply a plan. Safe changes are always applied; conflicts follow `policy`.
///
/// The plan is validated against the current project before anything is
/// written, so a plan created before a later edit is rejected instead of
/// partially applied.
pub fn apply_sync(plan: &SyncPlan, policy: ConflictPolicy) -> Result<SyncOutcome, SyncError> {
    validate_plan_is_current(plan)?;
    let mut snapshot = plan.snapshot.clone();
    let mut outcome = SyncOutcome::default();

    for action in &plan.actions {
        match action {
            SyncAction::Add {
                path,
                template_path,
            }
            | SyncAction::Update {
                path,
                template_path,
            } => {
                copy_resource(
                    &plan.target_template_root,
                    template_path,
                    &plan.project_root,
                    path,
                )?;
                record_baseline(
                    &mut snapshot,
                    &plan.target_template_root,
                    path,
                    template_path,
                )?;
                outcome.applied += 1;
            }
            SyncAction::Adopt {
                path,
                template_path,
            } => {
                record_baseline(
                    &mut snapshot,
                    &plan.target_template_root,
                    path,
                    template_path,
                )?;
                outcome.applied += 1;
            }
            SyncAction::Remove { path } => {
                remove_file_if_exists(&plan.project_root.join(key_path(path)?))?;
                snapshot.resources.remove(path);
                outcome.applied += 1;
            }
            SyncAction::KeepLocal { path, reason } => {
                if *reason == KeepReason::TemplateRemovedLocallyModified {
                    snapshot.resources.remove(path);
                }
            }
            SyncAction::KeepDeleted { .. } => {
                // Keep the baseline record: otherwise the next plan would see
                // the still-installed template file as newly added and restore
                // a file the user deliberately deleted.
            }
            SyncAction::Unchanged { .. } => {}
            SyncAction::Conflict {
                path,
                template_path,
                ..
            } => match policy {
                ConflictPolicy::Skip => outcome.conflicts_remaining += 1,
                ConflictPolicy::PreferLocal => {
                    // Keep the local file, but move the baseline to the
                    // template version. The local difference then shows up as
                    // an ordinary local modification on the next sync.
                    if let Some(template_path) = template_path {
                        record_baseline(
                            &mut snapshot,
                            &plan.target_template_root,
                            path,
                            template_path,
                        )?;
                    } else {
                        snapshot.resources.remove(path);
                    }
                    outcome.resolved += 1;
                }
                ConflictPolicy::PreferTemplate => {
                    if let Some(template_path) = template_path {
                        copy_resource(
                            &plan.target_template_root,
                            template_path,
                            &plan.project_root,
                            path,
                        )?;
                        record_baseline(
                            &mut snapshot,
                            &plan.target_template_root,
                            path,
                            template_path,
                        )?;
                    } else {
                        remove_file_if_exists(&plan.project_root.join(key_path(path)?))?;
                        snapshot.resources.remove(path);
                    }
                    outcome.resolved += 1;
                }
            },
        }
    }

    // Advancing the recorded template version is only truthful once no
    // conflicts remain. Resource baselines are still persisted so an
    // interrupted or partially resolved sync does not redo safe updates.
    let fully_synced = outcome.conflicts_remaining == 0;
    let changed = outcome.applied > 0 || outcome.resolved > 0 || fully_synced;
    if changed {
        if fully_synced {
            snapshot.template_version = plan.to_version.clone();
            snapshot.shared_assets = plan.target_shared_assets.clone();
        }
        snapshot.write(&plan.project_root)?;
    }
    Ok(outcome)
}

/// Verify that every write in the plan still matches the filesystem state it
/// was computed from. This keeps a stale plan from overwriting later edits.
fn validate_plan_is_current(plan: &SyncPlan) -> Result<(), SyncError> {
    for action in &plan.actions {
        match action {
            SyncAction::Add { path, .. } => {
                let destination = plan.project_root.join(key_path(path)?);
                if fingerprint_if_exists(&destination)?.is_some() {
                    return Err(SyncError::ChangedDuringSync { path: path.clone() });
                }
            }
            SyncAction::Update { path, .. } => {
                let destination = plan.project_root.join(key_path(path)?);
                let current = fingerprint_if_exists(&destination)?;
                let expected = plan
                    .snapshot
                    .resources
                    .get(path)
                    .map(|record| record.fingerprint.as_str());
                if current.as_deref() != expected {
                    return Err(SyncError::ChangedDuringSync { path: path.clone() });
                }
            }
            SyncAction::Remove { path } => {
                let destination = plan.project_root.join(key_path(path)?);
                if let Some(current) = fingerprint_if_exists(&destination)? {
                    let expected = plan
                        .snapshot
                        .resources
                        .get(path)
                        .map(|record| record.fingerprint.as_str());
                    if Some(current.as_str()) != expected {
                        return Err(SyncError::ChangedDuringSync { path: path.clone() });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn template_files(installed: &InstalledTemplate) -> Result<BTreeMap<String, PathBuf>, SyncError> {
    let shared_assets = installed.manifest.shared_assets.as_deref();
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(&installed.directory)
        .min_depth(1)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry?;
        if entry.file_type().is_symlink() || !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&installed.directory)
            .expect("walked path is under template root");
        if relative
            .components()
            .any(|component| component == std::path::Component::Normal(".git".as_ref()))
        {
            continue;
        }
        if relative == Path::new(PROJECT_SNAPSHOT_FILE) {
            continue;
        }
        if shared_assets.is_some_and(|shared| relative.starts_with(shared)) {
            continue;
        }
        files.insert(path_key(relative)?, entry.path().to_path_buf());
    }
    Ok(files)
}

fn fingerprint_path(path: &Path) -> Result<String, SyncError> {
    let bytes = fs::read(path).map_err(|source| SyncError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(fingerprint_bytes(&bytes))
}

fn fingerprint_if_exists(path: &Path) -> Result<Option<String>, SyncError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(fingerprint_bytes(&bytes))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(SyncError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn record_baseline(
    snapshot: &mut ProjectSnapshot,
    template_root: &Path,
    path: &str,
    template_path: &str,
) -> Result<(), SyncError> {
    // The baseline is the template content, never the local file: otherwise
    // `PreferLocal` would make the user's change look unmodified and the next
    // sync would overwrite it.
    let source = template_root.join(key_path(template_path)?);
    let bytes = fs::read(&source).map_err(|error| SyncError::Io {
        path: source,
        source: error,
    })?;
    snapshot.resources.insert(
        path.to_owned(),
        ResourceRecord {
            template_path: template_path.to_owned(),
            fingerprint: fingerprint_bytes(&bytes),
        },
    );
    Ok(())
}

fn copy_resource(
    template_root: &Path,
    template_path: &str,
    project_root: &Path,
    path: &str,
) -> Result<(), SyncError> {
    let source = template_root.join(key_path(template_path)?);
    let destination = project_root.join(key_path(path)?);
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| SyncError::Apply {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temporary = destination.as_os_str().to_owned();
    temporary.push(format!(".sync-{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    fs::copy(&source, &temporary).map_err(|source| SyncError::Apply {
        path: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, &destination).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        SyncError::Apply {
            path: destination.clone(),
            source,
        }
    })
}

fn remove_file_if_exists(path: &Path) -> Result<(), SyncError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SyncError::Apply {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn same_version(left: &str, right: &str) -> bool {
    let normalize = |value: &str| value.trim().trim_start_matches('v').to_owned();
    let (left, right) = (normalize(left), normalize(right));
    if left == right {
        return true;
    }
    match (parse_version(&left), parse_version(&right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// Parse a version, accepting abbreviated forms such as `1.2`.
fn parse_version(value: &str) -> Option<Version> {
    if let Ok(version) = Version::parse(value) {
        return Some(version);
    }
    let (core, suffix) = match value.find(['-', '+']) {
        Some(index) => value.split_at(index),
        None => (value, ""),
    };
    let missing = 3_usize.saturating_sub(core.split('.').count());
    if missing == 0 || core.split('.').any(|part| part.is_empty()) {
        return None;
    }
    let padded = format!("{core}{}{suffix}", ".0".repeat(missing));
    Version::parse(&padded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigStore;
    use crate::project::{Project, ResourceState, create_project, set_template_version};
    use std::{fs, path::Path};
    use tempfile::{TempDir, tempdir};

    struct Fixture {
        _dir: TempDir,
        store: ConfigStore,
        project: Project,
        template: PathBuf,
    }

    fn manifest(version: &str) -> String {
        format!("name = 'university'\nversion = '{version}'\nshared_assets = 'shared'\n")
    }

    fn fixture() -> Fixture {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        let template = store.templates_dir().join("university");
        fs::create_dir_all(template.join("src")).unwrap();
        fs::create_dir_all(template.join("shared")).unwrap();
        fs::write(template.join("template.toml"), manifest("1.0.0")).unwrap();
        fs::write(template.join("src/main.tex"), "version one").unwrap();
        fs::write(template.join("src/keep.tex"), "keep").unwrap();
        fs::write(template.join("src/removed.tex"), "remove me").unwrap();
        fs::write(template.join("shared/asset.tex"), "shared asset").unwrap();
        let project = create_project("thesis", dir.path(), Some("university"), &store).unwrap();
        Fixture {
            _dir: dir,
            store,
            project,
            template,
        }
    }

    fn registry(fixture: &Fixture) -> TemplateRegistry {
        TemplateRegistry::new(fixture.store.templates_dir())
    }

    fn action<'a>(plan: &'a SyncPlan, path: &str) -> &'a SyncAction {
        plan.actions
            .iter()
            .find(|action| action.path() == path)
            .unwrap_or_else(|| panic!("no action for '{path}' in {:#?}", plan.actions))
    }

    fn update_template(fixture: &Fixture, version: &str) {
        fs::write(fixture.template.join("template.toml"), manifest(version)).unwrap();
    }

    #[test]
    fn unchanged_template_produces_an_empty_change_set() {
        let fixture = fixture();
        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        let counts = plan.counts();
        assert_eq!(counts.changes(), 0);
        assert_eq!(counts.conflicts, 0);
        assert!(
            plan.actions
                .iter()
                .all(|action| matches!(action, SyncAction::Unchanged { .. }))
        );
        assert!(
            plan.actions
                .iter()
                .all(|action| !action.path().starts_with("shared/"))
        );

        let outcome = apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert_eq!(outcome, SyncOutcome::default());
    }

    #[test]
    fn template_changes_update_only_untouched_files() {
        let fixture = fixture();
        fs::write(fixture.project.root().join("src/keep.tex"), "user edit").unwrap();
        fs::write(fixture.template.join("src/main.tex"), "version two").unwrap();
        update_template(&fixture, "1.1.0");

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        assert!(matches!(
            action(&plan, "src/main.tex"),
            SyncAction::Update { .. }
        ));
        assert!(matches!(
            action(&plan, "src/keep.tex"),
            SyncAction::KeepLocal {
                reason: KeepReason::LocalModified,
                ..
            }
        ));
        assert!(matches!(
            action(&plan, "template.toml"),
            SyncAction::Update { .. }
        ));

        let outcome = apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert_eq!(outcome.conflicts_remaining, 0);
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/main.tex")).unwrap(),
            "version two"
        );
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/keep.tex")).unwrap(),
            "user edit"
        );
        let snapshot = ProjectSnapshot::load(fixture.project.root()).unwrap();
        assert_eq!(snapshot.template_version, "1.1.0");
        assert_eq!(
            snapshot
                .resource_state(fixture.project.root(), "src/main.tex")
                .unwrap(),
            Some(ResourceState::Unchanged)
        );
        assert_eq!(
            snapshot
                .resource_state(fixture.project.root(), "src/keep.tex")
                .unwrap(),
            Some(ResourceState::Modified)
        );
    }

    #[test]
    fn new_and_removed_template_resources_are_added_and_removed() {
        let fixture = fixture();
        fs::write(fixture.template.join("src/new.tex"), "new").unwrap();
        fs::remove_file(fixture.template.join("src/removed.tex")).unwrap();
        update_template(&fixture, "1.1.0");

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        assert!(matches!(
            action(&plan, "src/new.tex"),
            SyncAction::Add { .. }
        ));
        assert!(matches!(
            action(&plan, "src/removed.tex"),
            SyncAction::Remove { .. }
        ));

        apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/new.tex")).unwrap(),
            "new"
        );
        assert!(!fixture.project.root().join("src/removed.tex").exists());
        let snapshot = ProjectSnapshot::load(fixture.project.root()).unwrap();
        assert!(snapshot.resources.contains_key("src/new.tex"));
        assert!(!snapshot.resources.contains_key("src/removed.tex"));
    }

    #[test]
    fn both_sides_modified_is_a_conflict_until_explicitly_resolved() {
        let fixture = fixture();
        fs::write(fixture.project.root().join("src/main.tex"), "local edit").unwrap();
        fs::write(fixture.template.join("src/main.tex"), "template edit").unwrap();
        update_template(&fixture, "1.1.0");

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        assert!(matches!(
            action(&plan, "src/main.tex"),
            SyncAction::Conflict {
                kind: ConflictKind::BothModified,
                ..
            }
        ));
        let outcome = apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert_eq!(outcome.conflicts_remaining, 1);
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/main.tex")).unwrap(),
            "local edit",
            "the default policy must never overwrite user changes"
        );

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        let outcome = apply_sync(&plan, ConflictPolicy::PreferLocal).unwrap();
        assert_eq!(outcome.resolved, 1);
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/main.tex")).unwrap(),
            "local edit"
        );
        let snapshot = ProjectSnapshot::load(fixture.project.root()).unwrap();
        assert_eq!(snapshot.template_version, "1.1.0");
        assert_eq!(
            snapshot
                .resource_state(fixture.project.root(), "src/main.tex")
                .unwrap(),
            Some(ResourceState::Modified)
        );
    }

    #[test]
    fn prefer_template_explicitly_replaces_local_changes() {
        let fixture = fixture();
        fs::write(fixture.project.root().join("src/main.tex"), "local edit").unwrap();
        fs::write(fixture.template.join("src/main.tex"), "template edit").unwrap();
        update_template(&fixture, "1.1.0");

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        let outcome = apply_sync(&plan, ConflictPolicy::PreferTemplate).unwrap();
        assert_eq!(outcome.resolved, 1);
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/main.tex")).unwrap(),
            "template edit"
        );
        assert_eq!(
            ProjectSnapshot::load(fixture.project.root())
                .unwrap()
                .template_version,
            "1.1.0"
        );
    }

    #[test]
    fn user_created_files_are_preserved_and_collisions_are_conflicts() {
        let fixture = fixture();
        fs::write(fixture.project.root().join("notes.txt"), "user notes").unwrap();
        fs::write(fixture.template.join("src/new.tex"), "template new").unwrap();
        fs::write(fixture.project.root().join("src/new.tex"), "local new").unwrap();
        update_template(&fixture, "1.1.0");

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        assert!(matches!(
            action(&plan, "src/new.tex"),
            SyncAction::Conflict {
                kind: ConflictKind::AddedLocallyAndRemotely,
                ..
            }
        ));
        apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("notes.txt")).unwrap(),
            "user notes"
        );
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/new.tex")).unwrap(),
            "local new"
        );
    }

    #[test]
    fn local_deletions_are_respected_and_remote_changes_can_restore_them() {
        let first = fixture();
        fs::remove_file(first.project.root().join("src/keep.tex")).unwrap();
        let plan = plan_sync(&first.project, &registry(&first)).unwrap();
        assert!(matches!(
            action(&plan, "src/keep.tex"),
            SyncAction::KeepDeleted { .. }
        ));
        apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert!(!first.project.root().join("src/keep.tex").exists());
        let snapshot = ProjectSnapshot::load(first.project.root()).unwrap();
        assert!(
            snapshot.resources.contains_key("src/keep.tex"),
            "the baseline record must survive so the deletion is not undone"
        );
        let plan = plan_sync(&first.project, &registry(&first)).unwrap();
        assert!(matches!(
            action(&plan, "src/keep.tex"),
            SyncAction::KeepDeleted { .. }
        ));
        assert!(!first.project.root().join("src/keep.tex").exists());

        let second = fixture();
        fs::remove_file(second.project.root().join("src/main.tex")).unwrap();
        fs::write(second.template.join("src/main.tex"), "template edit").unwrap();
        update_template(&second, "1.1.0");
        let plan = plan_sync(&second.project, &registry(&second)).unwrap();
        assert!(matches!(
            action(&plan, "src/main.tex"),
            SyncAction::Conflict {
                kind: ConflictKind::DeletedLocallyModifiedRemotely,
                ..
            }
        ));
        apply_sync(&plan, ConflictPolicy::PreferTemplate).unwrap();
        assert_eq!(
            fs::read_to_string(second.project.root().join("src/main.tex")).unwrap(),
            "template edit"
        );
    }

    #[test]
    fn modified_resource_removed_by_template_is_kept_as_a_user_file() {
        let fixture = fixture();
        fs::write(fixture.project.root().join("src/removed.tex"), "user edit").unwrap();
        fs::remove_file(fixture.template.join("src/removed.tex")).unwrap();
        update_template(&fixture, "1.1.0");

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        assert!(matches!(
            action(&plan, "src/removed.tex"),
            SyncAction::KeepLocal {
                reason: KeepReason::TemplateRemovedLocallyModified,
                ..
            }
        ));
        apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/removed.tex")).unwrap(),
            "user edit"
        );
        assert!(
            !ProjectSnapshot::load(fixture.project.root())
                .unwrap()
                .resources
                .contains_key("src/removed.tex")
        );
    }

    #[test]
    fn pinned_projects_require_the_installed_version_to_match() {
        let fixture = fixture();
        set_template_version(fixture.project.root(), "1.0.0", &fixture.store).unwrap();
        update_template(&fixture, "1.1.0");
        let project = Project::load(fixture.project.root(), &fixture.store).unwrap();

        assert!(matches!(
            plan_sync(&project, &registry(&fixture)),
            Err(SyncError::PinnedVersionMismatch { .. })
        ));
    }

    #[test]
    fn sync_requires_a_snapshot_and_matching_template_metadata() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        fs::create_dir_all(dir.path().join("project/src")).unwrap();
        fs::write(
            dir.path().join("project/latex_handler.toml"),
            "[project]\ntemplate = 'university'\n",
        )
        .unwrap();
        let project = Project::load(dir.path().join("project"), &store).unwrap();
        assert!(matches!(
            plan_sync(&project, &TemplateRegistry::new(store.templates_dir())),
            Err(SyncError::MissingSnapshot)
        ));

        let fixture = fixture();
        fs::write(
            fixture.project.config_path(),
            "[project]\ntemplate = 'different'\ntemplate_version = 'latest'\n",
        )
        .unwrap();
        let project = Project::load(fixture.project.root(), &fixture.store).unwrap();
        assert!(matches!(
            plan_sync(&project, &registry(&fixture)),
            Err(SyncError::TemplateMismatch { .. })
        ));
    }

    #[test]
    fn syncing_twice_is_idempotent() {
        let fixture = fixture();
        fs::write(fixture.template.join("src/main.tex"), "version two").unwrap();
        update_template(&fixture, "1.1.0");

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        assert_eq!(plan.counts().changes(), 0);
        assert_eq!(plan.counts().conflicts, 0);
        let outcome = apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert_eq!(outcome, SyncOutcome::default());
    }

    #[test]
    fn partial_sync_persists_safe_updates_without_claiming_the_new_version() {
        let fixture = fixture();
        fs::write(fixture.project.root().join("src/main.tex"), "local edit").unwrap();
        fs::write(fixture.template.join("src/main.tex"), "template edit").unwrap();
        fs::write(fixture.template.join("src/keep.tex"), "keep two").unwrap();
        update_template(&fixture, "1.1.0");

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        let outcome = apply_sync(&plan, ConflictPolicy::Skip).unwrap();
        assert_eq!(outcome.conflicts_remaining, 1);
        assert!(outcome.applied > 0);
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/keep.tex")).unwrap(),
            "keep two"
        );
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/main.tex")).unwrap(),
            "local edit"
        );

        let snapshot = ProjectSnapshot::load(fixture.project.root()).unwrap();
        assert_eq!(
            snapshot.template_version, "1.0.0",
            "the snapshot must not claim the new version while a conflict remains"
        );

        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();
        assert!(matches!(
            action(&plan, "src/keep.tex"),
            SyncAction::Unchanged { .. }
        ));
        assert!(matches!(
            action(&plan, "src/main.tex"),
            SyncAction::Conflict { .. }
        ));
        apply_sync(&plan, ConflictPolicy::PreferLocal).unwrap();
        assert_eq!(
            ProjectSnapshot::load(fixture.project.root())
                .unwrap()
                .template_version,
            "1.1.0"
        );
    }

    #[test]
    fn apply_refuses_to_overwrite_a_file_changed_after_planning() {
        let fixture = fixture();
        fs::write(fixture.template.join("src/main.tex"), "template edit").unwrap();
        update_template(&fixture, "1.1.0");
        let plan = plan_sync(&fixture.project, &registry(&fixture)).unwrap();

        fs::write(
            fixture.project.root().join("src/main.tex"),
            "edited after the plan",
        )
        .unwrap();
        assert!(matches!(
            apply_sync(&plan, ConflictPolicy::Skip),
            Err(SyncError::ChangedDuringSync { .. })
        ));
        assert_eq!(
            fs::read_to_string(fixture.project.root().join("src/main.tex")).unwrap(),
            "edited after the plan"
        );
    }

    #[test]
    fn same_version_accepts_a_leading_v_and_full_semver() {
        assert!(same_version("v1.2.0", "1.2.0"));
        assert!(same_version("1.2", "1.2.0"));
        assert!(!same_version("1.2.0", "1.2.1"));
    }

    #[test]
    fn template_files_exclude_git_shared_assets_and_snapshots() {
        let fixture = fixture();
        fs::create_dir_all(fixture.template.join(".git")).unwrap();
        fs::write(fixture.template.join(".git/config"), "metadata").unwrap();
        fs::write(
            fixture.template.join(PROJECT_SNAPSHOT_FILE),
            "not a real snapshot",
        )
        .unwrap();
        let files = template_files(&registry(&fixture).get("university").unwrap()).unwrap();
        assert!(!files.keys().any(|key| key.starts_with(".git/")));
        assert!(!files.keys().any(|key| key.starts_with("shared/")));
        assert!(!files.contains_key(PROJECT_SNAPSHOT_FILE));
        assert!(files.contains_key("src/main.tex"));
    }

    #[test]
    fn template_file_listing_is_relative_to_the_template() {
        let fixture = fixture();
        let files = template_files(&registry(&fixture).get("university").unwrap()).unwrap();
        assert_eq!(files["src/main.tex"], fixture.template.join("src/main.tex"));
        assert!(Path::new(&files.keys().next().unwrap()).is_relative());
    }
}
