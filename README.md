# latex_handler

A Rust command-line tool for creating, configuring, and compiling LaTeX projects.

> **Development status:** the rewrite is in progress. The CLI foundation and the
> configuration structures, configuration-file loading and initialization,
> and layered configuration resolution are implemented. Configuration CLI
> operations are being added incrementally.

## Requirements

- Rust stable (edition 2024)
- A LaTeX distribution and `latexmk` for document compilation (once compilation is implemented)
- Git for template installation and updates (once template management is implemented)

## Build

```sh
cargo build
```

## CLI overview

```sh
latex_handler --help
latex_handler compile --help
latex_handler project --help
latex_handler template --help
latex_handler config --help
latex_handler clean --help
```

## Configuration

Configuration is represented as layered, partial TOML data. Unspecified values
remain unset so that project settings can inherit global settings and built-in
defaults. The built-in defaults are exposed by the library. `ConfigStore` can load the
global configuration and a project's `latex_handler.toml`, and can create
commented starter files without overwriting existing user configuration.

The global file is stored in the platform-specific user configuration
directory provided by the `directories` crate. Project configuration is stored
at the project root. Missing files load as empty configuration layers; malformed
TOML produces an error that identifies the offending file. Configuration
The effective configuration is resolved in this order: built-in defaults,
global configuration, project configuration, then command-line overrides.
Partial nested sections merge field by field, so setting one project compiler
option does not discard unrelated global compiler preferences. `load_resolved`
provides a fully populated configuration for application components.

## Design

The CLI is intentionally separated from the application logic. Project discovery,
configuration, compilation, and template management are implemented as
independent modules so they can be tested without invoking the command-line parser.

## Project initialization

`latex_handler init` initializes the current directory (or a supplied directory)
without replacing existing source files. It creates the configured `src/`,
`build/`, `pdfs/`, and `logs/` directories and adds `latex_handler.toml` only
when it is missing. Existing project metadata and configuration overrides are
preserved.

```sh
latex_handler init
latex_handler init ./existing-thesis --template university
```

`latex_handler new <name>` creates a new directory from an installed template.
Installed templates are currently expected under
`<user-config-directory>/templates/<template-name>`; template downloading and
version management are handled by the upcoming template-management stage.

```sh
latex_handler new thesis --template university
latex_handler new thesis --template university --path ~/Documents
```

Creation refuses to use an existing destination directory. Template copying
omits Git metadata and symbolic links.

### Template updates

Update one installed template to its latest stable Git tag, or update every
installed template:

```sh
latex_handler template update university
latex_handler template update
```

Select a specific published tag (with or without a leading `v`):

```sh
latex_handler template update university --version 1.0.0
```

Prerelease tags are excluded by default. Set `templates.prerelease = true` in
the global configuration to include them when selecting the latest release.
Updates are staged before replacing the installed template; local modifications
are protected and cause the update to stop instead of being discarded.
