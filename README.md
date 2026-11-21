# latex_handler

A Rust command-line tool for creating, configuring, and compiling LaTeX projects,
and for synchronizing projects with their source templates.

## Status

Implemented features:

- Project creation and initialization from installed templates.
- Template-based project snapshots with content fingerprints.
- Project synchronization with safe updates and explicit conflict policies.
- Asset management: project-local assets and template-shared assets exposed to
  compilation through `TEXINPUTS`.
- Document discovery and compilation with `latexmk`, themes, engines, and
  parallel compilation.
- Template installation, update, status, and version listing.
- Configuration management and project structure validation.
- `latex_handler clean` for generated build outputs.
- Self-update of the binary is **not** implemented; `latex_handler update` is
  an alias that updates installed templates to their latest stable versions.

## Requirements

- Rust stable (edition 2024)
- A LaTeX distribution and `latexmk` for document compilation
- Git for template installation and updates
## Install and build

```sh
cargo build
```

The resulting binary is `target/debug/latex_handler`.

## CLI overview

```sh
latex_handler --help
latex_handler new --help
latex_handler compile --help
latex_handler project --help
latex_handler template --help
latex_handler config --help
latex_handler clean --help
```

Available commands:

```text
new       Create a new project from an installed template
init      Initialize latex_handler in an existing directory
compile   Compile one or more LaTeX documents
project   Inspect and validate a project (info, validate, list, template, sync)
template  Manage installed project templates
config    Inspect and modify configuration
clean     Remove generated build artifacts
update    Update all installed templates
version   Display the installed version
```

## Quick start

```sh
latex_handler template install university --repository https://example.invalid/university.git
latex_handler new thesis --template university
cd thesis
latex_handler project validate
latex_handler compile
```

There is no built-in default template; templates are installed explicitly or by
configuration.

## Project creation

`latex_handler new <name>` creates a project directory from an installed
template. Installed templates live under `<config-dir>/templates/<name>`, where
the config directory is provided by the `directories` crate (for example
`~/.config/latex_handler/templates` on Linux).

`latex_handler init [PATH]` initializes the current directory, or an explicitly
supplied directory, without replacing existing source files.

New projects default to this layout relative to the project root:

```text
src/            Source documents
build/          Intermediate compilation files and build PDFs
pdfs/           Published PDF outputs
logs/           Published log files
assets/         Project-local assets (images, bibliography files, snippets)
```

`latex_handler new` refuses to create over an existing directory. Template
copying omits Git metadata and symbolic links. Projects record their template
and snapshot the template resources they copied into `latex_handler.snapshot.toml`.

## Compilation

`latex_handler compile [PATH]` compiles discovered documents.

Useful options:

```sh
latex_handler compile ./thesis --file src/main.tex --theme dark --engine xelatex
latex_handler compile ./thesis --file src/main.tex --engine lualatex --force
```

Behavior:

- Discovery follows the configured `structure.source` directory and uses the
  default patterns `main.tex` and `*_main.tex`; assets and some legacy/temp
  directories are excluded.
- `--engine` overrides the configured engine for the run. Supported engines
  are `pdflatex`, `xelatex`, and `lualatex`.
- `--theme all` compiles every enabled theme; other themes add `-<theme>` to the
  output filename.
- `--force` passes `-g` to latexmk to bypass incremental compilation decisions.
- Results are printed per document. Compilation is parallel by default, and a
  final summary reports success/failure counts.

## Assets

There are two asset sources:

- **Local assets:** files in the project's `structure.assets` directory
  (default `assets`), relative to the project root.
- **Shared assets:** directories declared in an installed template's manifest as
  `shared_assets` (for example `shared`). They stay in the template and are
  exposed to compilation instead of copied into new projects.

During compilation, latex_handler builds a recursive `TEXINPUTS` containing the
local asset directories first, then the shared asset directories, then any
user-provided `TEXINPUTS` value, and a TeX default path. Relative asset paths are
searched from the compiled document's directory.

## Project synchronization

`latex_handler project sync [PATH]` compares a project with its installed
source template.

```sh
# Show the plan; nothing is changed.
latex_handler project sync ./thesis

# Apply safe updates.
latex_handler project sync ./thesis --apply

# Explicitly resolve conflicts in favor of the local file.
latex_handler project sync ./thesis --apply --prefer local

# Explicitly resolve conflicts in favor of the template version.
latex_handler project sync ./thesis --apply --prefer template
```

The plan compares three states:

- the snapshot baseline recorded when the project was created,
- the local project file,
- the installed template file.

Safe changes are applied automatically:

- New files are added.
- Unchanged local files are updated to new template versions.
- Local files that match the template after a template rename are adopted.
- Files removed by the template and untouched locally are removed.
- Deletions made locally are preserved as deletions.
- Local modifications that are not touched by the template are kept.

Conflicts require an explicit policy and are never overwritten by default:

- `BothModified`: the local file and template both changed relative to the
  baseline.
- `AddedLocallyAndRemotely`: the user created a file and the template also adds
  it.
- `DeletedLocallyModifiedRemotely`: the user deleted a file and the template
  changed it.

`--apply` fails if any conflicts remain. `--prefer template` is destructive and
may discard local work; `--prefer local` keeps the local file and moves the
baseline to the template version, so the change appears as an ordinary local
modification on the next sync.

The recorded template version advances only when no conflicts remain. Resource
baselines are still persisted so safe updates are not repeated. Files not
managed by the template are never touched.

## Template version policies

```sh
latex_handler project template status
latex_handler project template pin 1.2.0
latex_handler project template track
```

A project pin describes the template release associated with the project.
Template files are copied when the project is created; changing the policy does
not rewrite the project files.

## Template management

```sh
latex_handler template list
latex_handler template install university --repository https://example.invalid/university.git
latex_handler template update university --version 1.2.0
latex_handler template status
latex_handler template versions university
latex_handler template remove university
```

`template install` can install a local Git repository or a remote repository URL.
`template update` updates installed templates to their latest stable release or
to an explicit version; prereleases are excluded by default unless configured.

## Configuration

Configuration is layered: built-in defaults, global user configuration, project
configuration, then command-line overrides.

```sh
latex_handler config show
latex_handler config get structure.assets
latex_handler config set structure.assets media
latex_handler config set structure.source src
latex_handler config set [compiler|project|...] ...
latex_handler config reset
latex_handler config edit
```

Some useful defaults include:

```text
[structure]
source = "src"
build = "build"
pdf = "pdfs"
logs = "logs"
assets = "assets"
```

## Cleaning generated outputs

```sh
latex_handler clean ./thesis
latex_handler clean ./thesis --build
latex_handler clean ./thesis --logs
latex_handler clean ./thesis --all
```

By default, `clean` removes build intermediates and published logs while
preserving PDFs. `--build` removes only build intermediates; `--logs` removes
only generated logs; `--all` removes build intermediates, logs, and PDFs.

Clean only removes directories configured under the build/log/PDF settings, and
refuses paths that would remove the project root, the source directory, or the
assets directory.

## Updating latex_handler

`latex_handler update` updates all installed templates to their latest stable
release; it does not update the `latex_handler` binary itself. To upgrade the
binary, reinstall it from the release or package manager you used.

## Development

```sh
cargo fmt --check
cargo test --locked
cargo build --locked
```

Run the full test suite before marking work complete.
