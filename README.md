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
- `latex_handler self-update` for binary updates from published releases.

## Requirements

- Rust stable (edition 2024) to build from source
- A LaTeX distribution and `latexmk` for document compilation
- Git for template installation and updates

## Install and build

### From a release

Prebuilt binaries are published on GitHub Releases for Linux and Windows.

Linux:

```sh
curl -sSL https://github.com/Ivan23BG/latex_handler/releases/latest/download/latex_handler-linux.tar.gz | tar -xz -C "$HOME/.local/bin" latex_handler
chmod +x "$HOME/.local/bin/latex_handler"
```

Windows (PowerShell):

```powershell
Invoke-WebRequest -Uri https://github.com/Ivan23BG/latex_handler/releases/latest/download/latex_handler-windows.zip -OutFile "$env:TEMP/latex_handler-windows.zip"
Expand-Archive -Path "$env:TEMP/latex_handler-windows.zip" -DestinationPath "$env:LOCALAPPDATA\latex_handler\bin" -Force
```

Alternatively, use the provided install scripts:

```sh
curl -sSL https://raw.githubusercontent.com/Ivan23BG/latex_handler/main/install.sh | bash
```

```powershell
irm https://raw.githubusercontent.com/Ivan23BG/latex_handler/main/install.ps1 | iex
```

### From source

```sh
git clone https://github.com/Ivan23BG/latex_handler.git
cd latex_handler
cargo build --locked
```

The resulting binary is `target/debug/latex_handler`.

For most users, the release binaries or install scripts are the simplest path;
building from source is useful for development.

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
project   Inspect, maintain, and validate a project (info, validate, status, list, pdfs, git, template, sync)
template  Manage installed project templates
config    Inspect and modify configuration
clean     Remove generated build artifacts
update    Update all installed templates
self-update  Update the latex_handler binary from the latest GitHub release
version   Display the installed version
help      Print this message or the help of the given subcommand(s)
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

## Template authoring

A template repository publishes a `template.toml` manifest at its root:

```toml
name = "university"
version = "1.0.0"
description = "University thesis template"
repository = "https://github.com/example/university.git"
authors = ["Your Name"]

# Optional: directory with reusable resources shared with projects.
shared_assets = "shared"

# Optional: templates this template depends on.
[dependencies]
base = { version = "^1.0", repository = "https://github.com/example/base.git" }
extra = { version = ">=1.0, <2.0", optional = true }
```

The manifest requires a valid `name` and a non-empty semantic-version `version`.
`shared_assets` names a directory that stays in the installed template and is
exposed to projects through the compilation-time asset search path instead of
being copied into new projects; it must exist in the installed template.

`dependencies` maps required templates to accepted version requirements
(for example `^1.2` or `>=1.0, <2.0`); optional dependencies may be skipped.

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

Use `--dry-run` to print which generated directories would be removed without
deleting anything. Safety checks still apply, so an unsafe target is reported
even in dry-run mode.

Clean only removes directories configured under the build/log/PDF settings, and
refuses paths that would remove the project root, the source directory, or the
assets directory.

## Generated PDF management

`latex_handler project pdfs [PATH]` lists the generated PDFs of a project and
their build status.

```sh
latex_handler project pdfs ./thesis
latex_handler project pdfs ./thesis --open src/main.tex
```

For every document/theme pair the listing reports the published PDF location,
and whether the PDF is `up-to-date`, `outdated`, or `missing`.

`--open <DOCUMENT>` resolves the document and opens its generated PDF with the
platform default PDF viewer (for example `xdg-open` on Linux, `open` on macOS,
or `explorer` on Windows). It reports an error when no PDF has been generated
yet.

## Project status and maintenance

`latex_handler project status [PATH]` summarizes the maintenance state of a
project.

```sh
latex_handler project status ./thesis
```

The report includes:

- the number of discovered documents,
- the generated PDFs and their build status (up-to-date, outdated, or missing),
- the size of the generated files under the configured build, log, and PDF
  directories,
- a clean preview listing what a default clean would remove.

## Git integration

Git integration is optional and lives under the `project git` command group.

```sh
latex_handler project git status ./thesis
latex_handler project git init ./thesis
```

`project git status` detects whether the project is inside a Git working tree
and reports the branch, whether the working tree is clean, and the changed
files (added, modified, deleted, renamed, untracked, or unmerged).

`project git init` initializes a Git repository in the project directory; it
refuses when the directory already belongs to a repository.

Git never runs automatically and is not required for creating, compiling, or
synchronizing projects. No Git command is executed for projects outside a
repository.

## Updating latex_handler

`latex_handler update` updates all installed templates to their latest stable
release; it does not update the `latex_handler` binary itself.

```sh
latex_handler self-update --check
latex_handler self-update
```

`latex_handler self-update` downloads the release archive for the current
platform from the GitHub releases of the `latex_handler` repository, replaces
the running binary, and reports whether an update was installed. The `--check`
flag only compares the installed version with the latest published release.

Binary self-update is best effort: it uses the `git`, `curl`, and `tar` commands
and never installs an archive it could not download and extract. Updates are
explicitly initiated; there is no background or automatic update behavior.

To upgrade the binary through a package manager or by reinstalling it from the
release, use those channels instead.

## Development

```sh
cargo fmt --check
cargo test --locked
cargo build --locked
```

Run the full test suite before marking work complete.
