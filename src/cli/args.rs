use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Manage LaTeX projects, templates, and document compilation.
#[derive(Debug, Parser)]
#[command(
    name = "latex_handler",
    version,
    about,
    long_about = "A project-oriented tool for creating, configuring, and compiling LaTeX documents."
)]
pub struct Cli {
    /// Print additional diagnostic information.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new project from an installed template.
    New(NewArgs),

    /// Initialize latex_handler in an existing directory.
    Init(InitArgs),

    /// Compile one or more LaTeX documents.
    Compile(CompileArgs),

    /// Inspect and validate a project.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },

    /// Manage installed project templates.
    Template {
        #[command(subcommand)]
        command: TemplateCommand,
    },

    /// Inspect and modify configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    /// Remove generated build artifacts.
    Clean(CleanArgs),

    /// Update latex_handler itself.
    Update,

    /// Display the installed version.
    Version,
}

#[derive(Debug, Args)]
pub struct NewArgs {
    /// Name of the new project (also used as its directory name by default).
    pub name: String,

    /// Template to use. Defaults to the globally configured template.
    #[arg(short, long)]
    pub template: Option<String>,

    /// Parent directory in which to create the project.
    #[arg(short, long)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Existing project directory. Defaults to the current directory.
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Template to associate with the project.
    #[arg(short, long)]
    pub template: Option<String>,
}

#[derive(Debug, Args)]
pub struct CompileArgs {
    /// Project directory or a specific .tex file. Defaults to the current directory.
    pub path: Option<PathBuf>,

    /// Compile a specific document. May be supplied more than once.
    #[arg(short, long, action = clap::ArgAction::Append)]
    pub file: Vec<PathBuf>,

    /// Theme to compile. `all` compiles every configured theme.
    #[arg(short, long, value_enum)]
    pub theme: Option<Theme>,

    /// LaTeX engine to use (latexmk remains the compilation backend).
    #[arg(short, long, value_enum)]
    pub engine: Option<LatexEngine>,

    /// Force recompilation rather than relying on latexmk's dependency checks.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Theme {
    Light,
    Dark,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LatexEngine {
    Pdflatex,
    Xelatex,
    Lualatex,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Display project configuration and detected structure.
    Info {
        /// Project directory. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Check whether a project follows the expected structure.
    Validate {
        /// Project directory. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// List documents that automatic discovery would compile.
    List {
        /// Project directory. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Inspect or change the template version policy for a project.
    Template {
        #[command(subcommand)]
        command: ProjectTemplateCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProjectTemplateCommand {
    /// Show the project's template and version policy.
    Status {
        /// Project directory. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Pin the project to an exact template version.
    Pin {
        /// Version or Git tag to pin (for example 1.2.0).
        version: String,
        /// Project directory. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Follow the latest stable template release instead of a fixed version.
    Track {
        /// Project directory. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// List templates installed on this machine.
    List,

    /// Install a template from its configured repository.
    Install {
        name: String,
        /// Git repository URL. Overrides the global templates.repository setting.
        #[arg(long)]
        repository: Option<String>,
    },

    /// Update one template, or all installed templates to the latest release.
    Update {
        name: Option<String>,
        /// Install a specific released version instead of the latest stable one.
        #[arg(long)]
        version: Option<String>,
    },

    /// Remove an installed template.
    Remove {
        name: String,
        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        force: bool,
    },

    /// Show installed and selected template versions.
    Status { name: Option<String> },

    /// List published versions of a template.
    Versions { name: String },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Display the effective or selected configuration.
    Show(ConfigScopeArgs),

    /// Read one configuration value using a dotted key.
    Get {
        key: String,
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },

    /// Set a configuration value using a dotted key.
    Set {
        key: String,
        value: String,
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },

    /// Restore built-in defaults, or reset one setting.
    Reset {
        key: Option<String>,
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },

    /// Open the selected configuration in the configured editor.
    Edit(ConfigScopeArgs),
}

#[derive(Debug, Clone, Copy, Default, Args)]
pub struct ConfigScopeArgs {
    /// Select the global configuration.
    #[arg(long, conflicts_with = "project")]
    pub global: bool,

    /// Select the project configuration.
    #[arg(long, conflicts_with = "global")]
    pub project: bool,
}

#[derive(Debug, Args)]
pub struct CleanArgs {
    /// Project directory. Defaults to the current directory.
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Remove only build intermediates.
    #[arg(long, conflicts_with_all = ["logs", "all"])]
    pub build: bool,

    /// Remove only generated logs.
    #[arg(long, conflicts_with_all = ["build", "all"])]
    pub logs: bool,

    /// Remove all generated outputs (build, logs, and PDFs).
    #[arg(long, conflicts_with_all = ["build", "logs"])]
    pub all: bool,
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn parses_compile_with_file_theme_and_engine() {
        let cli = Cli::try_parse_from([
            "latex_handler",
            "compile",
            "./course",
            "--file",
            "rapport/main.tex",
            "--theme",
            "dark",
            "--engine",
            "lualatex",
            "--force",
        ])
        .expect("compile arguments should parse");

        assert!(matches!(cli.command, super::Command::Compile(_)));
    }

    #[test]
    fn parses_project_template_pin_and_track_commands() {
        let pin = Cli::try_parse_from([
            "latex_handler",
            "project",
            "template",
            "pin",
            "1.2.0",
            "./thesis",
        ])
        .expect("project template pin arguments should parse");
        assert!(matches!(pin.command, super::Command::Project {
            command: super::ProjectCommand::Template {
                command: super::ProjectTemplateCommand::Pin { version, path }
            }
        } if version == "1.2.0" && path == PathBuf::from("./thesis")));

        let track = Cli::try_parse_from(["latex_handler", "project", "template", "track"])
            .expect("project template track arguments should parse");
        assert!(matches!(
            track.command,
            super::Command::Project {
                command: super::ProjectCommand::Template {
                    command: super::ProjectTemplateCommand::Track { .. }
                }
            }
        ));
    }

    #[test]
    fn parses_template_install_with_repository_override() {
        let cli = Cli::try_parse_from([
            "latex_handler",
            "template",
            "install",
            "university",
            "--repository",
            "https://example.invalid/university.git",
        ])
        .expect("template install arguments should parse");

        assert!(matches!(cli.command, super::Command::Template {
            command: super::TemplateCommand::Install { name, repository: Some(repository) }
        } if name == "university" && repository == "https://example.invalid/university.git"));
    }

    #[test]
    fn parses_nested_project_command() {
        let cli = Cli::try_parse_from(["latex_handler", "project", "list", "."])
            .expect("project list arguments should parse");

        assert!(matches!(cli.command, super::Command::Project { .. }));
    }

    #[test]
    fn parses_template_update_with_version() {
        let cli = Cli::try_parse_from([
            "latex_handler",
            "template",
            "update",
            "university",
            "--version",
            "1.2.0",
        ])
        .expect("template update arguments should parse");

        assert!(matches!(cli.command, super::Command::Template { .. }));
    }

    #[test]
    fn parses_config_set_with_explicit_project_scope() {
        let cli = Cli::try_parse_from([
            "latex_handler",
            "config",
            "set",
            "compiler.engine",
            "lualatex",
            "--project",
        ])
        .expect("config set arguments should parse");
        assert!(matches!(cli.command, super::Command::Config { .. }));
    }

    #[test]
    fn clean_modes_are_mutually_exclusive() {
        let result = Cli::try_parse_from(["latex_handler", "clean", ".", "--build", "--logs"]);

        assert!(result.is_err());
    }
}
