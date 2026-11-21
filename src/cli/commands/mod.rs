use super::args::{
    CleanArgs, Command, CompileArgs, ConfigCommand, ConfigScopeArgs, GitCommand, InitArgs, NewArgs,
    ProjectCommand, ProjectTemplateCommand, SelfUpdateArgs, SyncArgs, TemplateCommand, Theme,
};
use crate::compiler::{
    CompileOutcome, Compiler, DocumentResult, PdfStatus, ThemeSelection, compile_documents,
    engine_name, error_excerpt, generated_pdfs, pdf_path_for, plan_jobs, select_documents,
    worker_count,
};
use crate::config::ThemeVariant;
use crate::config::{CompilerConfig, Config, ConfigManager, ConfigScope, ConfigStore};
use crate::project::{
    CleanSelection, ConflictPolicy, KeepReason, Project, SyncAction, ValidationSeverity,
    apply_sync, clean_project, create_project, directory_bytes, init, initialize_project,
    plan_clean, plan_sync, set_template_version, status,
};
use crate::template::{
    TemplateRegistry, install_template_with_dependencies, list_template_versions, remove_template,
    update_template,
};
use std::{error::Error, path::PathBuf};

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
        Command::Clean(args) => execute_clean(args),
        Command::Update => execute_template(TemplateCommand::Update {
            name: None,
            version: None,
        }),
        Command::SelfUpdate(args) => execute_self_update(args),
        Command::Version => {
            println!("latex_handler {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn execute_compile(args: CompileArgs, verbose: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
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

    let selection = match args.theme {
        None => ThemeSelection::Default,
        Some(Theme::Light) => ThemeSelection::Only(ThemeVariant::Light),
        Some(Theme::Dark) => ThemeSelection::Only(ThemeVariant::Dark),
        Some(Theme::All) => ThemeSelection::All,
    };
    let jobs = plan_jobs(&project, &documents, selection)?;

    let compiler = Compiler::new(project.config().compiler.clone());
    let total = jobs.len();
    let workers = worker_count(compiler.settings(), total);
    let subject = if total == documents.len() {
        format!("{total} document{}", if total == 1 { "" } else { "s" })
    } else {
        format!(
            "{} documents x {} themes",
            documents.len(),
            total / documents.len()
        )
    };
    println!(
        "Compiling {subject} with {} ({}){}\n",
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
        let label = format!("[{}/{total}] {}", index + 1, item.job.label(&project));
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
                        let pdf = report
                            .final_pdf
                            .as_deref()
                            .map(&relative)
                            .unwrap_or_default();
                        block.push_str(&format!("{label}  SUCCESS ({seconds:.1}s)  {pdf}\n"));
                    }
                    CompileOutcome::Failed { exit_code } => {
                        let code = exit_code.map_or("signal".to_string(), |c| c.to_string());
                        block.push_str(&format!("{label}  FAILED (exit {code}, {seconds:.1}s)\n"));
                        for line in error_excerpt(&report.stdout) {
                            block.push_str(&format!("    {line}\n"));
                        }
                        if let Some(log) = report.final_log.as_ref().or(report.log.as_ref()) {
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

    let batch = compile_documents(&project, &compiler, &jobs, args.force, print_result)?;
    let (succeeded, failed) = (batch.succeeded(), batch.failed());
    println!("\n{succeeded} succeeded, {failed} failed.");
    if failed > 0 {
        return Err(
            std::io::Error::other(format!("{failed} document(s) failed to compile")).into(),
        );
    }
    Ok(())
}

fn execute_clean(args: CleanArgs) -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = ConfigStore::user_default()?;
    let project = Project::discover(&args.path, &store)?;
    let selection = clean_selection(&args);

    if args.dry_run {
        return execute_clean_dry_run(&project, &selection);
    }

    let removed = clean_project(&project, selection)?;
    if removed.is_empty() {
        println!("Nothing to clean.");
        return Ok(());
    }
    for path in removed {
        let relative = path.strip_prefix(project.root()).unwrap_or(&path);
        println!("Removed {}", relative.display());
    }
    Ok(())
}

fn execute_clean_dry_run(
    project: &Project,
    selection: &CleanSelection,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let planned = plan_clean(project, *selection)?;
    if planned.is_empty() {
        println!("Nothing to clean.");
        return Ok(());
    }
    for path in planned {
        let relative = path.strip_prefix(project.root()).unwrap_or(&path);
        println!("Would remove {}", relative.display());
    }
    println!("Dry run: no files were changed.");
    Ok(())
}

fn clean_selection(args: &CleanArgs) -> CleanSelection {
    if args.all {
        CleanSelection::all()
    } else if args.build {
        CleanSelection::build()
    } else if args.logs {
        CleanSelection::logs()
    } else {
        CleanSelection::default()
    }
}

fn execute_self_update(args: SelfUpdateArgs) -> Result<(), Box<dyn Error + Send + Sync>> {
    let result = crate::self_update::self_update(args.check)?;
    println!("Current version: {}", result.current_version);
    println!(
        "Latest version: {}",
        result.latest_version.as_deref().unwrap_or("(none)")
    );
    if args.check {
        println!("Check complete.");
        return Ok(());
    }
    if result.updated {
        println!(
            "Updated latex_handler to {}.",
            result.latest_version.unwrap_or_default()
        );
    } else {
        println!("latex_handler is already up to date.");
    }
    Ok(())
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
            println!(
                "Assets directory: {}",
                project.config().structure.assets.display()
            );
            println!(
                "Shared assets: {}",
                project
                    .shared_assets_dir()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "(none)".into())
            );
            match project.snapshot() {
                Ok(Some(snapshot)) => println!(
                    "Template snapshot: {} {} ({} resources)",
                    snapshot.template,
                    snapshot.template_version,
                    snapshot.resources.len()
                ),
                Ok(None) => println!("Template snapshot: (none)"),
                Err(error) => println!("Template snapshot: unreadable ({error})"),
            }
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
        ProjectCommand::Status { path } => execute_project_status(path)?,
        ProjectCommand::Pdfs { path, open } => execute_project_pdfs(path, open)?,
        ProjectCommand::Template { command } => execute_project_template(command, &store)?,
        ProjectCommand::Sync(args) => execute_project_sync(args, &store)?,
        ProjectCommand::Git { command } => execute_project_git(command)?,
    }
    Ok(())
}

fn execute_project_sync(
    args: SyncArgs,
    store: &ConfigStore,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let project = Project::discover(&args.path, store)?;
    let registry = TemplateRegistry::new(store.templates_dir());
    let plan = plan_sync(&project, &registry)?;
    let counts = plan.counts();

    println!("Project: {}", project.root().display());
    println!(
        "Template: {} ({} -> {})",
        plan.template, plan.from_version, plan.to_version
    );
    for action in &plan.actions {
        match action {
            SyncAction::Add { path, .. } => println!("  add       {path}"),
            SyncAction::Update { path, .. } => println!("  update    {path}"),
            SyncAction::Adopt { path, .. } => {
                println!("  adopt     {path} (already matches the template)")
            }
            SyncAction::Remove { path } => println!("  remove    {path}"),
            SyncAction::KeepLocal { path, reason } => match reason {
                KeepReason::LocalModified => println!("  keep      {path} (modified locally)"),
                KeepReason::TemplateRemovedLocallyModified => {
                    println!("  keep      {path} (modified locally; removed from the template)")
                }
            },
            SyncAction::KeepDeleted { path } => {
                println!("  keep      {path} (deleted locally)")
            }
            SyncAction::Conflict { path, kind, .. } => {
                println!("  conflict  {path} ({})", kind.description())
            }
            SyncAction::Unchanged { .. } => {}
        }
    }
    if counts.changes() == 0 && counts.conflicts == 0 {
        println!("Project is up to date.");
    }
    println!(
        "Summary: {} change(s), {} conflict(s), {} unchanged.",
        counts.changes(),
        counts.conflicts,
        counts.unchanged
    );

    if !args.apply {
        if counts.conflicts > 0 {
            println!(
                "Rerun with --apply to apply safe changes; resolve conflicts with --prefer local|template."
            );
        } else if counts.changes() > 0 {
            println!("Dry run: rerun with --apply to apply these changes.");
        }
        return Ok(());
    }

    let policy = args.prefer.map(Into::into).unwrap_or_default();
    let outcome = apply_sync(&plan, policy)?;
    println!(
        "Applied {} change(s); resolved {} conflict(s).",
        outcome.applied, outcome.resolved
    );
    if outcome.conflicts_remaining > 0 {
        return Err(std::io::Error::other(format!(
            "{} conflict(s) remain; rerun with --apply --prefer local or --prefer template",
            outcome.conflicts_remaining
        ))
        .into());
    }
    if policy == ConflictPolicy::PreferTemplate && outcome.resolved > 0 {
        println!("Template versions replaced local files for resolved conflicts.");
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
            match project.snapshot() {
                Ok(Some(snapshot)) => println!(
                    "Snapshot: {} {} ({} resources)",
                    snapshot.template,
                    snapshot.template_version,
                    snapshot.resources.len()
                ),
                Ok(None) => println!("Snapshot: (none)"),
                Err(error) => println!("Snapshot: unreadable ({error})"),
            }
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

fn execute_project_pdfs(
    path: PathBuf,
    open: Option<PathBuf>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = ConfigStore::user_default()?;
    let project = Project::discover(&path, &store)?;
    let documents = project.documents()?;

    if let Some(document) = open {
        let current = std::env::current_dir()?;
        let selected = select_documents(&project, std::slice::from_ref(&document), &current)?;
        let jobs = plan_jobs(&project, &selected, ThemeSelection::Default)?;
        let job = jobs.into_iter().next().ok_or_else(|| {
            std::io::Error::other(format!("no compile job for '{}'", document.display()))
        })?;
        let pdf = pdf_path_for(&project, &job);
        if !pdf.is_file() {
            return Err(std::io::Error::other(format!(
                "no generated PDF for '{}' yet",
                job.document.relative_path.display()
            ))
            .into());
        }
        open_pdf(&pdf)?;
        println!(
            "Opened {}",
            pdf.strip_prefix(project.root()).unwrap_or(&pdf).display()
        );
        return Ok(());
    }

    let pdfs = generated_pdfs(&project, &documents);
    if pdfs.is_empty() {
        println!("No generated PDFs.");
        return Ok(());
    }
    println!("Generated PDFs: {}", pdfs.len());
    for pdf in &pdfs {
        let relative = pdf.path.strip_prefix(project.root()).unwrap_or(&pdf.path);
        println!(
            "  {}  {}  {}",
            relative.display(),
            pdf.label(&project),
            pdf.status.description()
        );
    }
    let up_to_date = pdfs
        .iter()
        .filter(|pdf| pdf.status == PdfStatus::UpToDate)
        .count();
    let outdated = pdfs
        .iter()
        .filter(|pdf| pdf.status == PdfStatus::Outdated)
        .count();
    let missing = pdfs
        .iter()
        .filter(|pdf| pdf.status == PdfStatus::Missing)
        .count();
    println!(
        "\n{} generated PDF(s): {} up-to-date, {} outdated, {} missing.",
        pdfs.len(),
        up_to_date,
        outdated,
        missing
    );
    Ok(())
}

/// Open a PDF with the platform default viewer.
fn open_pdf(path: &std::path::Path) -> Result<(), Box<dyn Error + Send + Sync>> {
    let program = pdf_opener();
    let status = std::process::Command::new(program)
        .arg(path)
        .status()
        .map_err(|source| std::io::Error::other(format!("could not run '{program}': {source}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(
            std::io::Error::other(format!("'{program}' failed to open '{}'", path.display()))
                .into(),
        )
    }
}

/// The platform-default PDF viewer command, when one is available.
fn pdf_opener() -> &'static str {
    if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

fn execute_project_status(path: PathBuf) -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = ConfigStore::user_default()?;
    let project = Project::discover(&path, &store)?;
    let documents = project.documents()?;
    let pdfs = generated_pdfs(&project, &documents);

    let up_to_date = pdfs
        .iter()
        .filter(|pdf| pdf.status == PdfStatus::UpToDate)
        .count();
    let outdated = pdfs
        .iter()
        .filter(|pdf| pdf.status == PdfStatus::Outdated)
        .count();
    let missing = pdfs
        .iter()
        .filter(|pdf| pdf.status == PdfStatus::Missing)
        .count();

    println!("Project: {}", project.root().display());
    println!("Documents: {}", documents.len());
    println!("Generated PDFs: {}", pdfs.len());
    println!(
        "Build status: {up_to_date} up-to-date, {outdated} outdated, {missing} missing PDF(s)."
    );

    println!("\nGenerated files:");
    for (label, directory) in [
        ("build", project.config().structure.build.clone()),
        ("logs", project.config().structure.logs.clone()),
        ("PDF", project.config().structure.pdf.clone()),
    ] {
        if let Some(bytes) = directory_bytes(&project.root().join(&directory)) {
            println!("  {label}: {bytes} byte(s)");
        }
    }

    println!("\nClean preview:");
    let planned = plan_clean(&project, CleanSelection::default())?;
    if planned.is_empty() {
        println!("  Nothing to clean.");
    }
    for path in planned {
        let relative = path.strip_prefix(project.root()).unwrap_or(&path);
        println!("  Would remove {}", relative.display());
    }
    Ok(())
}

fn execute_project_git(command: GitCommand) -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = ConfigStore::user_default()?;
    match command {
        GitCommand::Status { path } => {
            let project = Project::discover(&path, &store)?;
            let state = status(project.root())?;
            println!("Project: {}", project.root().display());
            println!("Git repository: {}", state.is_repository);
            if state.is_repository {
                println!(
                    "Branch: {}",
                    state.branch.as_deref().unwrap_or("(detached)")
                );
                println!("Clean: {}", state.clean);
                for change in &state.changes {
                    println!("  {:<10} {}", change.kind.label(), change.path.display());
                }
            } else {
                println!("No Git repository found; Git integration is optional.");
            }
        }
        GitCommand::Init { path } => {
            let project = Project::discover(&path, &store)?;
            let root = project.root().to_path_buf();
            if status(&root)?.is_repository {
                return Err(std::io::Error::other(
                    "the directory is already inside a git repository",
                )
                .into());
            }
            init(&root)?;
            println!("Initialized git repository at {}", root.display());
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
            let installed =
                install_template_with_dependencies(&registry, &name, repository.as_deref())?;
            println!(
                "Installed template '{}' ({})",
                installed.manifest.name, installed.manifest.version
            );
            println!("Directory: {}", installed.directory.display());
            println!("Dependencies:");
            if installed.manifest.dependencies.is_empty() {
                println!("  (none)");
            } else {
                for (dependency, requirement) in &installed.manifest.dependencies {
                    let state = match registry.get(dependency) {
                        Ok(item) => format!("installed ({})", item.manifest.version),
                        Err(_) if requirement.optional => "optional, not installed".to_owned(),
                        Err(_) => "missing".to_owned(),
                    };
                    println!("  {dependency} {} — {state}", requirement.version);
                }
            }
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
