//! Safe project initialization and creation from installed templates.

use super::{PROJECT_SNAPSHOT_FILE, Project, ProjectError, ProjectSnapshot};
use crate::config::ConfigStore;
use crate::template::{TEMPLATE_MANIFEST_FILE, TemplateManifest};
use std::fs;
use std::path::{Component, Path, PathBuf};
use toml_edit::{DocumentMut, value};
use walkdir::WalkDir;

/// Initialize an existing directory without replacing any existing files.
pub fn initialize_project(
    path: impl AsRef<Path>,
    store: &ConfigStore,
    template: Option<&str>,
) -> Result<Project, ProjectError> {
    let root = path.as_ref();
    if let Some(template_name) = template {
        validate_template_name(template_name)?;
    }
    if !root.is_dir() {
        return Err(ProjectError::NotProject(root.to_path_buf()));
    }

    let config_path = store.ensure_project(root)?;
    let mut document = read_document(&config_path)?;
    let project = document
        .as_table_mut()
        .entry("project")
        .or_insert(toml_edit::table())
        .as_table_mut()
        .ok_or_else(|| ProjectError::InvalidProjectConfig("[project] must be a table".into()))?;

    if !project.contains_key("name") {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project");
        project.insert("name", value(name));
    }
    if let Some(template_name) = template {
        if !project.contains_key("template") {
            project.insert("template", value(template_name));
        }
        if !project.contains_key("template_version") {
            project.insert("template_version", value("latest"));
        }
    }
    fs::write(&config_path, document.to_string()).map_err(|source| ProjectError::Path {
        path: config_path.clone(),
        source,
    })?;

    let config = store.load_resolved(root, &crate::config::Config::default())?;
    for relative in [
        &config.structure.source,
        &config.structure.build,
        &config.structure.pdf,
        &config.structure.logs,
        &config.structure.assets,
    ] {
        let directory = root.join(relative);
        fs::create_dir_all(&directory).map_err(|source| ProjectError::Path {
            path: directory,
            source,
        })?;
    }

    Project::load(root, store)
}

/// Create a new project by copying an installed template into a new directory.
/// Installed templates live under `<config_dir>/templates/<name>`.
pub fn create_project(
    name: &str,
    parent: impl AsRef<Path>,
    template: Option<&str>,
    store: &ConfigStore,
) -> Result<Project, ProjectError> {
    validate_project_name(name)?;
    let parent = parent.as_ref();
    if !parent.is_dir() {
        return Err(ProjectError::Path {
            path: parent.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "parent directory does not exist",
            ),
        });
    }

    let destination = parent.join(name);
    if destination.exists() {
        return Err(ProjectError::AlreadyExists(destination));
    }

    let selected_template = match template {
        Some(name) => name.to_owned(),
        None => store
            .load_global()?
            .general
            .and_then(|general| general.default_template)
            .ok_or(ProjectError::NoTemplateSelected)?,
    };
    validate_template_name(&selected_template)?;
    let template_root = store.templates_dir().join(&selected_template);
    if !template_root.is_dir() {
        return Err(ProjectError::TemplateNotInstalled {
            name: selected_template,
            path: template_root,
        });
    }

    fs::create_dir(&destination).map_err(|source| {
        if source.kind() == std::io::ErrorKind::AlreadyExists {
            ProjectError::AlreadyExists(destination.clone())
        } else {
            ProjectError::Path {
                path: destination.clone(),
                source,
            }
        }
    })?;

    let result = (|| {
        let manifest = installed_template_manifest(&template_root)?;
        let shared_assets = manifest
            .as_ref()
            .and_then(|manifest| manifest.shared_assets.as_deref());
        let copied = copy_template(&template_root, &destination, shared_assets)?;
        let project = initialize_project(&destination, store, Some(&selected_template))?;
        if let Some(manifest) = &manifest {
            let mut snapshot =
                ProjectSnapshot::new(&manifest.name, &manifest.version, shared_assets)?;
            for relative in &copied {
                snapshot.record_file(project.root(), relative, relative)?;
            }
            snapshot.write(project.root())?;
        }
        Ok(project)
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&destination);
    }
    result
}

/// Set a project's template version policy. The value `latest` tracks the
/// latest stable release; any other non-empty value pins an exact release.
pub fn set_template_version(
    path: impl AsRef<Path>,
    version: &str,
    store: &ConfigStore,
) -> Result<Project, ProjectError> {
    let version = version.trim();
    if version.is_empty() || version.chars().any(char::is_whitespace) {
        return Err(ProjectError::InvalidProjectConfig(
            "template version must be a non-empty version or 'latest'".into(),
        ));
    }

    let project = Project::discover(path, store)?;
    if project.config().project.template.is_none() {
        return Err(ProjectError::InvalidProjectConfig(
            "this project is not associated with a template; initialize it with --template first"
                .into(),
        ));
    }

    let config_path = store.ensure_project(project.root())?;
    let mut document = read_document(&config_path)?;
    let project_table = document
        .as_table_mut()
        .entry("project")
        .or_insert(toml_edit::table())
        .as_table_mut()
        .ok_or_else(|| ProjectError::InvalidProjectConfig("[project] must be a table".into()))?;
    project_table.insert("template_version", value(version));
    fs::write(&config_path, document.to_string()).map_err(|source| ProjectError::Path {
        path: config_path,
        source,
    })?;

    Project::load(project.root(), store)
}

fn installed_template_manifest(
    template_root: &Path,
) -> Result<Option<TemplateManifest>, ProjectError> {
    if !template_root.join(TEMPLATE_MANIFEST_FILE).is_file() {
        return Ok(None);
    }
    Ok(Some(TemplateManifest::load(template_root)?))
}

fn copy_template(
    source: &Path,
    destination: &Path,
    excluded: Option<&Path>,
) -> Result<Vec<PathBuf>, ProjectError> {
    let mut copied = Vec::new();
    for entry in WalkDir::new(source).min_depth(1).sort_by_file_name() {
        let entry = entry?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .expect("walked path is under source");
        if relative
            .components()
            .any(|component| component == Component::Normal(std::ffi::OsStr::new(".git")))
        {
            continue;
        }
        // Never propagate another project's snapshot into a new project.
        if relative == Path::new(PROJECT_SNAPSHOT_FILE) {
            continue;
        }
        // Shared assets stay in the installed template and are exposed to
        // projects through the asset search path instead of being copied.
        if excluded.is_some_and(|excluded| relative.starts_with(excluded)) {
            continue;
        }
        // Do not copy symlinks: templates must not write outside their own tree.
        if entry.file_type().is_symlink() {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|source| ProjectError::Path {
                path: target,
                source,
            })?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|source| ProjectError::Path {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            fs::copy(entry.path(), &target).map_err(|source| ProjectError::Path {
                path: target,
                source,
            })?;
            copied.push(relative.to_path_buf());
        }
    }
    Ok(copied)
}

fn read_document(path: &Path) -> Result<DocumentMut, ProjectError> {
    let source = fs::read_to_string(path).map_err(|source| ProjectError::Path {
        path: path.to_path_buf(),
        source,
    })?;
    source
        .parse::<DocumentMut>()
        .map_err(|error| ProjectError::InvalidProjectConfig(format!("{}: {error}", path.display())))
}

fn validate_project_name(name: &str) -> Result<(), ProjectError> {
    let path = Path::new(name);
    let mut components = path.components();
    if name.trim().is_empty()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(ProjectError::InvalidProjectName(name.to_owned()));
    }
    Ok(())
}

fn validate_template_name(name: &str) -> Result<(), ProjectError> {
    validate_project_name(name).map_err(|_| ProjectError::InvalidTemplateName(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn init_creates_standard_directories_and_metadata() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.tex"), "existing content").unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        let project = initialize_project(dir.path(), &store, Some("university")).unwrap();
        assert!(dir.path().join("src").is_dir());
        assert!(dir.path().join("build").is_dir());
        assert!(dir.path().join("pdfs").is_dir());
        assert!(dir.path().join("logs").is_dir());
        assert!(dir.path().join("assets").is_dir());
        assert_eq!(
            fs::read_to_string(dir.path().join("main.tex")).unwrap(),
            "existing content"
        );
        assert_eq!(
            project.config().project.template.as_deref(),
            Some("university")
        );
        assert_eq!(
            project.config().project.template_version.as_deref(),
            Some("latest")
        );
    }

    #[test]
    fn init_is_idempotent_and_preserves_existing_metadata() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("latex_handler.toml");
        fs::write(
            &config,
            "[project]\nname = 'My name'\ntemplate = 'custom'\n\n[compiler]\nengine = 'lualatex'\n",
        )
        .unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        initialize_project(dir.path(), &store, Some("university")).unwrap();
        initialize_project(dir.path(), &store, Some("university")).unwrap();
        let contents = fs::read_to_string(config).unwrap();
        assert!(contents.contains("name = 'My name'"));
        assert!(contents.contains("template = 'custom'"));
        assert!(contents.contains("engine = 'lualatex'"));
    }

    #[test]
    fn new_copies_template_without_git_metadata() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        let template = store.config_dir().join("templates/university");
        fs::create_dir_all(template.join("src")).unwrap();
        fs::create_dir_all(template.join(".git")).unwrap();
        fs::write(template.join("src/main.tex"), "template document").unwrap();
        fs::write(template.join(".git/config"), "git metadata").unwrap();
        let project = create_project("thesis", dir.path(), Some("university"), &store).unwrap();
        assert_eq!(
            fs::read_to_string(project.root().join("src/main.tex")).unwrap(),
            "template document"
        );
        assert!(!project.root().join(".git").exists());
        assert_eq!(
            project.config().project.template.as_deref(),
            Some("university")
        );
    }

    #[test]
    fn new_skips_shared_assets_and_creates_a_local_assets_directory() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        let template = store.config_dir().join("templates/university");
        fs::create_dir_all(template.join("src")).unwrap();
        fs::create_dir_all(template.join("shared")).unwrap();
        fs::create_dir_all(template.join("assets")).unwrap();
        fs::write(
            template.join("template.toml"),
            "name = 'university'\nversion = '1.0.0'\nshared_assets = 'shared'\n",
        )
        .unwrap();
        fs::write(template.join("src/main.tex"), "template document").unwrap();
        fs::write(template.join("shared/branding.tex"), "shared asset").unwrap();
        fs::write(template.join("assets/local.tex"), "local asset").unwrap();

        let project = create_project("thesis", dir.path(), Some("university"), &store).unwrap();

        assert!(!project.root().join("shared").exists());
        assert_eq!(
            fs::read_to_string(project.root().join("assets/local.tex")).unwrap(),
            "local asset"
        );
        assert_eq!(
            project.shared_assets_dir(),
            Some(template.join("shared").as_path())
        );
        assert_eq!(
            project.asset_directories(),
            vec![project.assets_dir(), template.join("shared")]
        );
    }

    #[test]
    fn new_records_a_snapshot_of_template_resources() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        let template = store.config_dir().join("templates/university");
        fs::create_dir_all(template.join("src")).unwrap();
        fs::create_dir_all(template.join("shared")).unwrap();
        fs::write(
            template.join("template.toml"),
            "name = 'university'\nversion = '1.4.0'\nshared_assets = 'shared'\n",
        )
        .unwrap();
        fs::write(template.join("src/main.tex"), "template document").unwrap();
        fs::write(template.join("src/chapter.tex"), "chapter").unwrap();
        fs::write(template.join("shared/branding.tex"), "shared asset").unwrap();
        fs::write(
            template.join(PROJECT_SNAPSHOT_FILE),
            "format_version = 1\ntemplate = 'old'\ntemplate_version = '0.1.0'\n",
        )
        .unwrap();

        let project = create_project("thesis", dir.path(), Some("university"), &store).unwrap();
        let snapshot = project.snapshot().unwrap().expect("snapshot should exist");

        assert_eq!(snapshot.template, "university");
        assert_eq!(snapshot.template_version, "1.4.0");
        assert_eq!(snapshot.shared_assets.as_deref(), Some("shared"));
        assert!(snapshot.resources.contains_key("src/main.tex"));
        assert!(snapshot.resources.contains_key("src/chapter.tex"));
        assert!(snapshot.resources.contains_key("template.toml"));
        assert!(!snapshot.resources.contains_key("shared/branding.tex"));
        assert!(!snapshot.resources.contains_key(PROJECT_SNAPSHOT_FILE));
        assert!(!project.root().join("shared").exists());
        assert_eq!(
            snapshot
                .resource_state(project.root(), "src/main.tex")
                .unwrap(),
            Some(crate::project::ResourceState::Unchanged)
        );

        fs::write(project.root().join("src/main.tex"), "user edit").unwrap();
        assert_eq!(
            snapshot
                .resource_state(project.root(), "src/main.tex")
                .unwrap(),
            Some(crate::project::ResourceState::Modified)
        );
    }

    #[test]
    fn configured_project_assets_directory_is_created_by_init() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        fs::write(
            dir.path().join("latex_handler.toml"),
            "[structure]\nassets = 'media'\n",
        )
        .unwrap();

        let project = initialize_project(dir.path(), &store, None).unwrap();

        assert!(dir.path().join("media").is_dir());
        assert_eq!(project.assets_dir(), dir.path().join("media"));
    }

    #[test]
    fn new_uses_global_default_template_when_not_explicit() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        fs::create_dir_all(store.templates_dir().join("university")).unwrap();
        fs::write(
            store.global_config_path(),
            "[general]\ndefault_template = 'university'\n",
        )
        .unwrap();
        let project = create_project("thesis", dir.path(), None, &store).unwrap();
        assert_eq!(
            project.config().project.template.as_deref(),
            Some("university")
        );
    }

    #[test]
    fn failed_creation_does_not_leave_a_partial_directory() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        assert!(create_project("thesis", dir.path(), Some("missing"), &store).is_err());
        assert!(!dir.path().join("thesis").exists());
    }

    #[test]
    fn new_refuses_to_overwrite_existing_directory() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        fs::create_dir(dir.path().join("thesis")).unwrap();
        let error = create_project("thesis", dir.path(), Some("missing"), &store).unwrap_err();
        assert!(matches!(error, ProjectError::AlreadyExists(_)));
    }

    #[test]
    fn project_template_can_be_pinned_and_returned_to_latest_tracking() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        let project_dir = dir.path().join("thesis");
        fs::create_dir_all(project_dir.join("src")).unwrap();
        fs::write(
            project_dir.join("latex_handler.toml"),
            "[project]\nname = 'Thesis'\ntemplate = 'university'\ntemplate_version = 'latest'\n\n[compiler]\nengine = 'lualatex'\n",
        )
        .unwrap();

        let pinned = set_template_version(&project_dir, "1.2.0", &store).unwrap();
        assert_eq!(
            pinned.config().project.template_version.as_deref(),
            Some("1.2.0")
        );
        let contents = fs::read_to_string(project_dir.join("latex_handler.toml")).unwrap();
        assert!(contents.contains("engine = 'lualatex'"));

        let tracking = set_template_version(&project_dir, "latest", &store).unwrap();
        assert_eq!(
            tracking.config().project.template_version.as_deref(),
            Some("latest")
        );
    }

    #[test]
    fn template_version_policy_requires_an_associated_template() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        fs::create_dir_all(dir.path().join("project/src")).unwrap();
        assert!(set_template_version(dir.path().join("project"), "1.0.0", &store).is_err());
    }

    #[test]
    fn rejects_empty_or_whitespace_template_versions() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config"));
        let project_dir = dir.path().join("project");
        fs::create_dir_all(project_dir.join("src")).unwrap();
        fs::write(
            project_dir.join("latex_handler.toml"),
            "[project]\ntemplate = 'university'\n",
        )
        .unwrap();
        assert!(set_template_version(&project_dir, "  ", &store).is_err());
        assert!(set_template_version(&project_dir, "1.0.0 rc1", &store).is_err());
    }

    #[test]
    fn rejects_path_traversal_in_names() {
        assert!(validate_project_name("../outside").is_err());
        assert!(validate_template_name("../outside").is_err());
    }
}
