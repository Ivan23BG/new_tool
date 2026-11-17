//! Loading and initial creation of global and project configuration files.
//!
//! Existing files are never rewritten by `ensure_*`: this is deliberate so
//! comments and user formatting are preserved. Configuration editing and
//! merging are handled by higher-level components.

use super::{Config, ResolvedConfig, resolve_config};
use directories::ProjectDirs;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const PROJECT_CONFIG_FILE: &str = "latex_handler.toml";
pub const GLOBAL_CONFIG_FILE: &str = "config.toml";

/// Errors encountered while locating, creating, or parsing configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the platform configuration directory")]
    ConfigDirectoryUnavailable,

    #[error("failed to access configuration file '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("invalid TOML in configuration file '{path}': {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("unknown configuration key '{0}'")]
    UnknownKey(String),

    #[error("no editor found: set VISUAL or EDITOR")]
    EditorUnavailable,

    #[error("failed to launch editor: {0}")]
    EditorLaunch(#[source] io::Error),

    #[error("editor exited unsuccessfully")]
    EditorFailed,
}

/// Resolves configuration paths and loads or initializes configuration files.
///
/// `config_dir` is injectable to keep tests independent of the host machine's
/// real user configuration directory.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    config_dir: PathBuf,
}

impl ConfigStore {
    /// Create a store using the operating system's standard user config path.
    pub fn user_default() -> Result<Self, ConfigError> {
        let dirs = ProjectDirs::from("com", "Ivan23BG", "latex_handler")
            .ok_or(ConfigError::ConfigDirectoryUnavailable)?;
        Ok(Self::new(dirs.config_dir()))
    }

    /// Create a store rooted at an explicit configuration directory.
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn global_config_path(&self) -> PathBuf {
        self.config_dir.join(GLOBAL_CONFIG_FILE)
    }

    pub fn project_config_path(project_root: impl AsRef<Path>) -> PathBuf {
        project_root.as_ref().join(PROJECT_CONFIG_FILE)
    }

    /// Load the global configuration. A missing file means an empty layer.
    pub fn load_global(&self) -> Result<Config, ConfigError> {
        self.load_file(&self.global_config_path())
    }

    /// Load a project's configuration. A missing file means an empty layer.
    pub fn load_project(&self, project_root: impl AsRef<Path>) -> Result<Config, ConfigError> {
        self.load_file(&Self::project_config_path(project_root))
    }

    /// Load global and project configuration and resolve them with optional CLI overrides.
    /// Precedence is built-in defaults < global < project < CLI.
    pub fn load_resolved(
        &self,
        project_root: impl AsRef<Path>,
        cli_overrides: &Config,
    ) -> Result<ResolvedConfig, ConfigError> {
        let global = self.load_global()?;
        let project = self.load_project(project_root)?;
        Ok(resolve_config(&global, &project, cli_overrides))
    }

    /// Create the global configuration with commented defaults if absent.
    /// Existing files are left byte-for-byte untouched.
    pub fn ensure_global(&self) -> Result<PathBuf, ConfigError> {
        let path = self.global_config_path();
        self.create_if_missing(&path, GLOBAL_CONFIG_TEMPLATE)?;
        Ok(path)
    }

    /// Create a project's configuration with commented defaults if absent.
    /// Existing files are left byte-for-byte untouched.
    pub fn ensure_project(&self, project_root: impl AsRef<Path>) -> Result<PathBuf, ConfigError> {
        let path = Self::project_config_path(project_root);
        self.create_if_missing(&path, PROJECT_CONFIG_TEMPLATE)?;
        Ok(path)
    }

    fn load_file(&self, path: &Path) -> Result<Config, ConfigError> {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Config::default());
            }
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        Config::from_toml(&source).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    fn create_if_missing(&self, path: &Path, contents: &str) -> Result<(), ConfigError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;

        let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        if let Err(source) = file.write_all(contents.as_bytes()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            });
        }

        Ok(())
    }
}

/// Commented starter configuration for user-wide preferences.
///
/// Values are commented out so the effective value continues to come from
/// built-in defaults unless the user explicitly opts into an override.
const GLOBAL_CONFIG_TEMPLATE: &str = r#"# latex_handler global configuration
# These settings apply to all projects unless overridden in a project's
# latex_handler.toml file. Uncomment a setting to override its built-in value.

[general]
# Template used by `latex_handler new` when --template is omitted.
# Default: unset (a template must otherwise be selected explicitly).
# default_template = "university"

# Check for newer template releases when a project tracks `latest`.
# Default: true
# check_template_updates = true

[compiler]
# Default LaTeX engine: pdflatex, xelatex, or lualatex.
# Default: "pdflatex"
# engine = "pdflatex"

# Run independent documents concurrently.
# Default: true
# parallel = true

# Maximum number of simultaneous compilations.
# Default: 8
# max_workers = 8

[templates]
# Automatically activate new stable releases for projects tracking `latest`.
# Default: true
# auto_update_latest = true

# Permit prerelease template versions during resolution.
# Default: false
# prerelease = false
"#;

/// Commented starter configuration for a project.
///
/// The comments document built-in values without writing them as overrides.
const PROJECT_CONFIG_TEMPLATE: &str = r#"# latex_handler project configuration
# Uncomment settings to override global preferences or built-in defaults.
# Remove an override to inherit its value again.

[project]
# Human-readable project name.
# Default: directory name (when no explicit name is configured).
# name = "My Project"

# Installed template this project uses.
# Default: unset
# template = "university"

# Use "latest" to follow stable releases, or pin an exact version.
# Default: "latest" when a template is configured.
# template_version = "latest"

[structure]
# Paths are relative to the project root unless absolute.
# Default: "src"
# source = "src"
# Default: "build"
# build = "build"
# Default: "pdfs"
# pdf = "pdfs"
# Default: "logs"
# logs = "logs"

[discovery]
# Automatically search for main TeX documents.
# Default: true
# enabled = true

# File-name patterns identifying main documents.
# Default: ["main.tex", "*_main.tex"]
# patterns = ["main.tex", "*_main.tex"]

# Directory names or paths excluded from discovery.
# Default: ["legacy", "templates", "tmp", "temp", "assets"]
# exclude = ["legacy", "templates", "tmp", "temp", "assets"]

[compiler]
# Compilation backend. Currently supported: latexmk.
# Default: "latexmk"
# backend = "latexmk"

# LaTeX engine: pdflatex, xelatex, or lualatex.
# Default: "pdflatex"
# engine = "pdflatex"

# Permit \write18 and other shell commands from TeX.
# Default: true
# shell_escape = true

# Generate SyncTeX data for editor synchronization.
# Default: true
# synctex = true

# latexmk interaction mode.
# Default: "nonstopmode"
# interaction = "nonstopmode"

# Stop after the first TeX error.
# Default: true
# halt_on_error = true

# Compile independent documents concurrently.
# Default: true
# parallel = true

# Maximum number of simultaneous compilations.
# Default: 8
# max_workers = 8

[output]
# Reproduce the source directory hierarchy in build/, pdfs/, and logs/.
# Default: true
# mirror_structure = true

# Keep intermediate files after successful compilation.
# Default: true
# keep_intermediates = true

[themes]
# Themes available to the project.
# Default: ["light", "dark"]
# enabled = ["light", "dark"]

# Theme used when --theme is omitted.
# Default: "light"
# default = "light"
"#;

#[cfg(test)]
mod tests {
    use super::{ConfigError, ConfigStore, GLOBAL_CONFIG_FILE, PROJECT_CONFIG_FILE};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn missing_configuration_files_load_as_empty_layers() {
        let temp = tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("global"));

        assert!(store.load_global().unwrap().project.is_none());
        assert!(store.load_project(temp.path()).unwrap().project.is_none());
    }

    #[test]
    fn ensure_global_creates_commented_config_and_parent_directories() {
        let temp = tempdir().unwrap();
        let config_dir = temp.path().join("nested").join("config");
        let store = ConfigStore::new(&config_dir);

        let path = store.ensure_global().unwrap();
        let contents = fs::read_to_string(&path).unwrap();

        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            GLOBAL_CONFIG_FILE
        );
        assert!(contents.contains("# Default: \"pdflatex\""));
        assert!(contents.contains("# engine = \"pdflatex\""));
        assert!(store.load_global().is_ok());
    }

    #[test]
    fn ensure_project_creates_a_parseable_commented_file() {
        let temp = tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("global"));

        let path = store.ensure_project(temp.path()).unwrap();
        let contents = fs::read_to_string(&path).unwrap();

        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            PROJECT_CONFIG_FILE
        );
        assert!(contents.contains("# template_version = \"latest\""));
        assert!(store.load_project(temp.path()).is_ok());
    }

    #[test]
    fn ensure_does_not_overwrite_existing_user_configuration() {
        let temp = tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("global"));
        let project_path = temp.path().join(PROJECT_CONFIG_FILE);
        let original = "[compiler]\nengine = \"lualatex\"\n# my comment\n";
        fs::write(&project_path, original).unwrap();

        store.ensure_project(temp.path()).unwrap();

        assert_eq!(fs::read_to_string(project_path).unwrap(), original);
        assert_eq!(
            store
                .load_project(temp.path())
                .unwrap()
                .compiler
                .unwrap()
                .engine,
            Some(crate::config::LatexEngine::LuaLatex)
        );
    }

    #[test]
    fn load_resolved_applies_global_project_and_cli_precedence() {
        use crate::config::{CompilerConfig, Config, LatexEngine};

        let temp = tempdir().unwrap();
        let global_dir = temp.path().join("global");
        fs::create_dir_all(&global_dir).unwrap();
        let store = ConfigStore::new(&global_dir);
        fs::write(
            store.global_config_path(),
            "[compiler]\nengine = \"xelatex\"\nmax_workers = 4\n",
        )
        .unwrap();
        fs::write(
            ConfigStore::project_config_path(temp.path()),
            "[compiler]\nengine = \"lualatex\"\n",
        )
        .unwrap();
        let cli = Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::PdfLatex),
                ..Default::default()
            }),
            ..Default::default()
        };

        let resolved = store.load_resolved(temp.path(), &cli).unwrap();
        assert_eq!(resolved.compiler.engine, LatexEngine::PdfLatex);
        assert_eq!(resolved.compiler.max_workers, 4);
    }

    #[test]
    fn malformed_toml_reports_the_file_path() {
        let temp = tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("global"));
        let path = temp.path().join(PROJECT_CONFIG_FILE);
        fs::write(&path, "[compiler\nengine = \"pdflatex\"").unwrap();

        let error = store.load_project(temp.path()).unwrap_err();
        assert!(matches!(&error, ConfigError::Parse { .. }));
        assert!(error.to_string().contains(PROJECT_CONFIG_FILE));
    }
}

pub(super) fn global_template() -> &'static str {
    GLOBAL_CONFIG_TEMPLATE
}
pub(super) fn project_template() -> &'static str {
    PROJECT_CONFIG_TEMPLATE
}
