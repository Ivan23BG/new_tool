use super::args::{
    Command, ConfigCommand, ConfigScopeArgs, InitArgs, NewArgs, ProjectCommand, TemplateCommand,
};
use crate::config::{ConfigManager, ConfigScope, ConfigStore};
use crate::project::{Project, ValidationSeverity, create_project, initialize_project};
use crate::template::{
    TemplateRegistry, install_template, list_template_versions, remove_template, update_template,
};
use std::error::Error;

/// Dispatch parsed commands. Individual command implementations will be added
/// in later stages; keeping dispatch centralized avoids putting application
/// logic in the argument definitions.
pub fn execute(command: Command) -> Result<(), Box<dyn Error + Send + Sync>> {
    match command {
        Command::New(args) => execute_new(args),
        Command::Init(args) => execute_init(args),
        Command::Compile(_) => not_implemented("compile"),
        Command::Project { command } => execute_project(command),
        Command::Template { command } => execute_template(command),
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

fn execute_init(args: InitArgs) -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = ConfigStore::user_default()?;
    let project = initialize_project(&args.path, &store, args.template.as_deref())?;
    println!("Initialized project at {}", project.root().display());
    println!("Configuration: {}", project.config_path().display());
    Ok(())
}

fn execute_new(args: NewArgs) -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = ConfigStore::user_default()?;
    let parent = args.path.unwrap_or(std::env::current_dir()?);
    let project = create_project(&args.name, parent, args.template.as_deref(), &store)?;
    println!("Created project at {}", project.root().display());
    println!(
        "Template: {}",
        project
            .config()
            .project
            .template
            .as_deref()
            .unwrap_or("(none)")
    );
    Ok(())
}

fn execute_template(command: TemplateCommand) -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = ConfigStore::user_default()?;
    let registry = TemplateRegistry::new(store.templates_dir());
    match command {
        TemplateCommand::List => {
            let templates = registry.list()?;
            if templates.is_empty() {
                println!("No templates installed.");
            } else {
                println!("{:<24} {:<14} DESCRIPTION", "NAME", "VERSION");
                for template in templates {
                    println!(
                        "{:<24} {:<14} {}",
                        template.manifest.name,
                        template.manifest.version,
                        template.manifest.description.as_deref().unwrap_or("")
                    );
                }
            }
        }
        TemplateCommand::Status { name } => {
            let templates = match name {
                Some(name) => vec![registry.get(&name)?],
                None => registry.list()?,
            };
            if templates.is_empty() {
                println!("No templates installed.");
            }
            for template in templates {
                println!("Template: {}", template.manifest.name);
                println!("Version: {}", template.manifest.version);
                println!("Directory: {}", template.directory.display());
                println!(
                    "Repository: {}",
                    template
                        .manifest
                        .repository
                        .as_deref()
                        .unwrap_or("(not recorded)")
                );
                println!(
                    "Description: {}",
                    template.manifest.description.as_deref().unwrap_or("(none)")
                );
            }
        }
        TemplateCommand::Install { name, repository } => {
            let global = store.load_global()?;
            let configured_repository = global.templates.and_then(|templates| templates.repository);
            let repository = repository.or(configured_repository);
            let installed = install_template(&registry, &name, repository.as_deref())?;
            println!(
                "Installed template '{}' ({})",
                installed.manifest.name, installed.manifest.version
            );
            println!("Directory: {}", installed.directory.display());
            println!(
                "Repository: {}",
                installed
                    .manifest
                    .repository
                    .as_deref()
                    .unwrap_or("(not recorded)")
            );
        }
        TemplateCommand::Update { name, version } => {
            let allow_prerelease = store
                .load_global()?
                .templates
                .and_then(|templates| templates.prerelease)
                .unwrap_or(false);
            let names = match name {
                Some(name) => {
                    registry.get(&name)?;
                    vec![name]
                }
                None => registry
                    .list()?
                    .into_iter()
                    .map(|template| template.manifest.name)
                    .collect(),
            };
            if names.is_empty() {
                println!("No templates installed.");
                return Ok(());
            }
            let mut failures = Vec::new();
            for template_name in names {
                match update_template(
                    &registry,
                    &template_name,
                    version.as_deref(),
                    allow_prerelease,
                ) {
                    Ok(updated) => println!(
                        "Updated '{}' to version {}",
                        updated.manifest.name, updated.manifest.version
                    ),
                    Err(error) => {
                        eprintln!("Could not update '{}': {error}", template_name);
                        failures.push(template_name);
                    }
                }
            }
            if !failures.is_empty() {
                return Err(std::io::Error::other(format!(
                    "{} template(s) could not be updated: {}",
                    failures.len(),
                    failures.join(", ")
                ))
                .into());
            }
        }
        TemplateCommand::Remove { name, force } => {
            // Validate the name before prompting, so a missing template gives
            // the useful registry error even in non-interactive environments.
            registry.get(&name)?;
            if !force && !confirm_template_removal(&name)? {
                println!("Removal cancelled.");
                return Ok(());
            }
            let removed = remove_template(&registry, &name)?;
            println!(
                "Removed template '{}' from {}",
                removed.manifest.name,
                removed.directory.display()
            );
            println!("Existing projects created from this template were not modified.");
        }
        TemplateCommand::Versions { name } => {
            let versions = list_template_versions(&registry, &name)?;
            if versions.is_empty() {
                println!("No published Git tags found for '{name}'.");
            } else {
                println!("Published versions for '{name}':");
                for version in versions {
                    println!("  {version}");
                }
            }
        }
    }
    Ok(())
}

fn confirm_template_removal(name: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
    use std::io::{self, IsTerminal, Write};

    if !io::stdin().is_terminal() {
        return Err(io::Error::other(
            "template removal requires confirmation; rerun with --force in non-interactive use",
        )
        .into());
    }

    print!("Remove installed template '{name}'? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
