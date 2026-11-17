use super::args::{Command, ConfigCommand, ConfigScopeArgs, ProjectCommand};
use crate::config::{ConfigManager, ConfigScope, ConfigStore};
use crate::project::{Project, ValidationSeverity};
use std::error::Error;

/// Dispatch parsed commands. Individual command implementations will be added
/// in later stages; keeping dispatch centralized avoids putting application
/// logic in the argument definitions.
pub fn execute(command: Command) -> Result<(), Box<dyn Error + Send + Sync>> {
    match command {
        Command::New(_) => not_implemented("new"),
        Command::Init(_) => not_implemented("init"),
        Command::Compile(_) => not_implemented("compile"),
        Command::Project { command } => execute_project(command),
        Command::Template { .. } => not_implemented("template"),
        Command::Config { command } => execute_config(command),
        Command::Clean(_) => not_implemented("clean"),
        Command::Update => not_implemented("update"),
        Command::Version => {
            println!("latex_handler {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn not_implemented(command: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!("`{command}` is not implemented yet"),
    )
    .into())
}

fn execute_config(command: ConfigCommand) -> Result<(), Box<dyn Error + Send + Sync>> {
    let project_root = std::env::current_dir()?;
    let manager = ConfigManager::new(ConfigStore::user_default()?, project_root);
    match command {
        ConfigCommand::Show(args) => {
            let scope = selected_scope(args);
            print!("{}", manager.show(scope)?);
        }
        ConfigCommand::Get { key, scope } => {
            println!("{}", manager.get(&key, selected_scope(scope))?);
        }
        ConfigCommand::Set {
            key,
            value,
            scope: args,
        } => {
            let scope = selected_scope(args).unwrap_or_else(|| manager.default_scope());
            manager.set(&key, &value, scope)?;
            println!("Updated {} in {} configuration.", key, scope_name(scope));
        }
        ConfigCommand::Reset { key, scope: args } => {
            let scope = selected_scope(args).unwrap_or_else(|| manager.default_scope());
            manager.reset(key.as_deref(), scope)?;
            println!(
                "Reset {} in {} configuration.",
                key.as_deref().unwrap_or("all settings"),
                scope_name(scope)
            );
        }
        ConfigCommand::Edit(args) => {
            let scope = selected_scope(args).unwrap_or_else(|| manager.default_scope());
            manager.edit(scope)?;
        }
    }
    Ok(())
}

fn selected_scope(args: ConfigScopeArgs) -> Option<ConfigScope> {
    if args.global {
        Some(ConfigScope::Global)
    } else if args.project {
        Some(ConfigScope::Project)
    } else {
        None
    }
}

fn scope_name(scope: ConfigScope) -> &'static str {
    match scope {
        ConfigScope::Global => "global",
        ConfigScope::Project => "project",
    }
}

fn execute_project(command: ProjectCommand) -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = ConfigStore::user_default()?;
    match command {
        ProjectCommand::Info { path } => {
            let project = Project::discover(path, &store)?;
            println!("Project root: {}", project.root().display());
            println!("Configuration: {}", project.config_path().display());
            println!("Source directory: {}", project.source_dir().display());
            println!(
                "Build directory: {}",
                project.config().structure.build.display()
            );
            println!(
                "PDF directory: {}",
                project.config().structure.pdf.display()
            );
            println!(
                "Logs directory: {}",
                project.config().structure.logs.display()
            );
            println!("Engine: {:?}", project.config().compiler.engine);
            println!(
                "Template: {}",
                project
                    .config()
                    .project
                    .template
                    .as_deref()
                    .unwrap_or("(none)")
            );
            let documents = project.documents()?;
            println!("Discovered documents: {}", documents.len());
            for document in documents {
                println!("  {}", document.relative_path.display());
            }
        }
        ProjectCommand::Validate { path } => {
            let project = Project::discover(path, &store)?;
            let report = project.validate();
            if report.issues.is_empty() {
                println!("Project structure is valid.");
            }
            for issue in &report.issues {
                let label = match issue.severity {
                    ValidationSeverity::Warning => "warning",
                    ValidationSeverity::Error => "error",
                };
                println!("{label}: {}", issue.message);
            }
            if !report.is_valid() {
                return Err(std::io::Error::other("project validation failed").into());
            }
        }
        ProjectCommand::List { path } => {
            let project = Project::discover(path, &store)?;
            let documents = project.documents()?;
            if documents.is_empty() {
                println!("No documents found.");
            }
            for document in documents {
                println!("{}", document.relative_path.display());
            }
        }
    }
    Ok(())
}
