use super::args::{
    Command, CompileArgs, ConfigCommand, ConfigScopeArgs, InitArgs, NewArgs, ProjectCommand,
    ProjectTemplateCommand, TemplateCommand,
};
use crate::compiler::{
    CompileOutcome, Compiler, DocumentResult, compile_documents, engine_name, error_excerpt,
    select_documents, worker_count,
};
use crate::config::{CompilerConfig, Config, ConfigManager, ConfigScope, ConfigStore};
use crate::project::{
    Project, ValidationSeverity, create_project, initialize_project, set_template_version,
};
use crate::template::{
    TemplateRegistry, install_template, list_template_versions, remove_template, update_template,
};
use std::error::Error;

/// Dispatch parsed commands. Individual command implementations will be added
/// in later stages; keeping dispatch centralized avoids putting application
/// logic in the argument definitions.
pub fn execute(command: Command, verbose: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
    match command {
        Command::New(args) => execute_new(args),
        Command::Init(args) => execute_init(args),
        Command::Compile(args) => execute_compile(args, verbose),
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

fn execute_compile(args: CompileArgs, verbose: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
    if args.theme.is_some() {
        return Err(std::io::Error::other("--theme is not supported yet").into());
    }
    let store = ConfigStore::user_default()?;
    let overrides = Config {
        compiler: args.engine.map(|engine| CompilerConfig {
            engine: Some(engine.into()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let start = args.path.unwrap_or_else(|| ".".into());
    let project = Project::discover_with_overrides(&start, &store, &overrides)?;

    // A .tex file given as the project path selects that document.
    let mut files = args.file;
    if start.is_file() && start.extension().is_some_and(|ext| ext == "tex") {
        files.push(start.canonicalize()?);
    }
    let documents = select_documents(&project, &files, &std::env::current_dir()?)?;

    let compiler = Compiler::new(project.config().compiler.clone());
    let total = documents.len();
    let workers = worker_count(compiler.settings(), total);
    println!(
        "Compiling {total} document{} with {} ({}){}\n",
        if total == 1 { "" } else { "s" },
        compiler.settings().backend,
        engine_name(compiler.settings().engine),
        if workers > 1 {
            format!(", up to {workers} in parallel")
        } else {
            String::new()
        }
    );

    // Results are printed as each document finishes. Each block is written
    // with a single call so output from concurrent workers never interleaves.
    let print_result = |index: usize, item: &DocumentResult| {
        let label = format!(
            "[{}/{total}] {}",
            index + 1,
            item.document.relative_path.display()
        );
        let relative = |path: &std::path::Path| {
            path.strip_prefix(project.root())
                .unwrap_or(path)
                .display()
                .to_string()
        };
        let mut block = String::new();
        match &item.result {
            Ok(report) => {
                let seconds = report.duration.as_secs_f32();
                match report.outcome {
                    CompileOutcome::Succeeded => {
                        let pdf = report.pdf.as_deref().map(&relative).unwrap_or_default();
                        block.push_str(&format!("{label}  SUCCESS ({seconds:.1}s)  {pdf}\n"));
                    }
                    CompileOutcome::Failed { exit_code } => {
                        let code = exit_code.map_or("signal".to_string(), |c| c.to_string());
                        block.push_str(&format!("{label}  FAILED (exit {code}, {seconds:.1}s)\n"));
                        for line in error_excerpt(&report.stdout) {
                            block.push_str(&format!("    {line}\n"));
                        }
                        if let Some(log) = &report.log {
                            block.push_str(&format!("    full log: {}\n", relative(log)));
                        }
                    }
                }
                if verbose {
                    block.push_str(&format!("    $ {}\n", report.command.display()));
                }
            }
            Err(error) => block.push_str(&format!("{label}  ERROR\n    {error}\n")),
        }
        print!("{block}");
    };

    let batch = compile_documents(&project, &compiler, &documents, args.force, print_result)?;
    let (succeeded, failed) = (batch.succeeded(), batch.failed());
    println!("\n{succeeded} succeeded, {failed} failed.");
    if failed > 0 {
        return Err(
            std::io::Error::other(format!("{failed} document(s) failed to compile")).into(),
        );
    }
    Ok(())
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
            println!(
                "Template version policy: {}",
                project
                    .config()
                    .project
                    .template_version
                    .as_deref()
                    .unwrap_or("(unset)")
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
        ProjectCommand::Template { command } => execute_project_template(command, &store)?,
    }
    Ok(())
}

fn execute_project_template(
    command: ProjectTemplateCommand,
    store: &ConfigStore,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    match command {
        ProjectTemplateCommand::Status { path } => {
            let project = Project::discover(path, store)?;
            let template = project
                .config()
                .project
                .template
                .as_deref()
                .ok_or_else(|| {
                    std::io::Error::other("this project is not associated with a template")
                })?;
            let policy = project
                .config()
                .project
                .template_version
                .as_deref()
                .unwrap_or("latest");
            println!("Project: {}", project.root().display());
            println!("Template: {template}");
            if policy == "latest" {
                println!("Version policy: tracking latest stable release");
            } else {
                println!("Version policy: pinned to {policy}");
            }
            let registry = TemplateRegistry::new(store.templates_dir());
            match registry.get(template) {
                Ok(installed) => {
                    println!("Installed version: {}", installed.manifest.version);
                    if policy != "latest"
                        && normalize_version(policy)
                            != normalize_version(&installed.manifest.version)
                    {
                        println!(
                            "Note: the globally installed template differs from this project's pinned version."
                        );
                    }
                }
                Err(crate::template::TemplateRegistryError::NotInstalled(_)) => {
                    println!("Installed version: (template not installed)");
                }
                Err(error) => return Err(error.into()),
            }
            println!("Project files are a snapshot; changing this policy does not rewrite them.");
        }
        ProjectTemplateCommand::Pin { version, path } => {
            if version.trim() == "latest" {
                return Err(
                    std::io::Error::other("use `project template track` to follow latest").into(),
                );
            }
            let project = set_template_version(path, &version, store)?;
            println!(
                "Pinned '{}' to template version {} (project files were not modified)",
                project
                    .config()
                    .project
                    .template
                    .as_deref()
                    .unwrap_or("(unknown)"),
                version.trim()
            );
        }
        ProjectTemplateCommand::Track { path } => {
            let project = set_template_version(path, "latest", store)?;
            println!(
                "Project '{}' now tracks the latest stable release of '{}' (project files were not modified)",
                project
                    .config()
                    .project
                    .name
                    .as_deref()
                    .unwrap_or("(unnamed)"),
                project
                    .config()
                    .project
                    .template
                    .as_deref()
                    .unwrap_or("(unknown)")
            );
        }
    }
    Ok(())
}

fn normalize_version(version: &str) -> &str {
    version.strip_prefix('v').unwrap_or(version)
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
