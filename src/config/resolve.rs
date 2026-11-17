//! Layer merging and fully resolved configuration values.

use super::{Config, LatexEngine, ThemeVariant, built_in_defaults};
use serde::Serialize;
use std::path::PathBuf;

/// Configuration with every operational setting populated.
///
/// Unlike [`Config`], this type contains no optional operational values. It is
/// intended for application components such as discovery and compilation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedConfig {
    pub project: ResolvedProjectConfig,
    pub structure: ResolvedStructureConfig,
    pub discovery: ResolvedDiscoveryConfig,
    pub compiler: ResolvedCompilerConfig,
    pub output: ResolvedOutputConfig,
    pub general: ResolvedGeneralConfig,
    pub templates: ResolvedTemplateConfig,
    pub themes: ResolvedThemeConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ResolvedProjectConfig {
    pub name: Option<String>,
    pub template: Option<String>,
    pub template_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedStructureConfig {
    pub source: PathBuf,
    pub build: PathBuf,
    pub pdf: PathBuf,
    pub logs: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedDiscoveryConfig {
    pub enabled: bool,
    pub patterns: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedCompilerConfig {
    pub backend: String,
    pub engine: LatexEngine,
    pub shell_escape: bool,
    pub synctex: bool,
    pub interaction: String,
    pub halt_on_error: bool,
    pub parallel: bool,
    pub max_workers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedOutputConfig {
    pub mirror_structure: bool,
    pub keep_intermediates: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedGeneralConfig {
    pub default_template: Option<String>,
    pub check_template_updates: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedTemplateConfig {
    pub auto_update_latest: bool,
    pub prerelease: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedThemeConfig {
    pub enabled: Vec<ThemeVariant>,
    pub default: ThemeVariant,
}

impl ResolvedConfig {
    pub(super) fn from_config(config: Config) -> Self {
        // `built_in_defaults()` fully populates every operational section.
        // These fallbacks make this conversion robust if that invariant changes.
        let defaults = built_in_defaults();
        let structure = config
            .structure
            .unwrap_or_else(|| defaults.structure.unwrap_or_default());
        let discovery = config
            .discovery
            .unwrap_or_else(|| defaults.discovery.unwrap_or_default());
        let compiler = config
            .compiler
            .unwrap_or_else(|| defaults.compiler.unwrap_or_default());
        let output = config
            .output
            .unwrap_or_else(|| defaults.output.unwrap_or_default());
        let general = config
            .general
            .unwrap_or_else(|| defaults.general.unwrap_or_default());
        let templates = config
            .templates
            .unwrap_or_else(|| defaults.templates.unwrap_or_default());
        let themes = config
            .themes
            .unwrap_or_else(|| defaults.themes.unwrap_or_default());
        let project = config.project.unwrap_or_default();

        Self {
            project: ResolvedProjectConfig {
                name: project.name,
                template: project.template,
                template_version: project.template_version,
            },
            structure: ResolvedStructureConfig {
                source: structure.source.unwrap_or_else(|| PathBuf::from("src")),
                build: structure.build.unwrap_or_else(|| PathBuf::from("build")),
                pdf: structure.pdf.unwrap_or_else(|| PathBuf::from("pdfs")),
                logs: structure.logs.unwrap_or_else(|| PathBuf::from("logs")),
            },
            discovery: ResolvedDiscoveryConfig {
                enabled: discovery.enabled.unwrap_or(true),
                patterns: discovery
                    .patterns
                    .unwrap_or_else(|| vec!["main.tex".into(), "*_main.tex".into()]),
                exclude: discovery.exclude.unwrap_or_else(|| {
                    vec![
                        "legacy".into(),
                        "templates".into(),
                        "tmp".into(),
                        "temp".into(),
                        "assets".into(),
                    ]
                }),
            },
            compiler: ResolvedCompilerConfig {
                backend: compiler.backend.unwrap_or_else(|| "latexmk".into()),
                engine: compiler.engine.unwrap_or(LatexEngine::PdfLatex),
                shell_escape: compiler.shell_escape.unwrap_or(true),
                synctex: compiler.synctex.unwrap_or(true),
                interaction: compiler.interaction.unwrap_or_else(|| "nonstopmode".into()),
                halt_on_error: compiler.halt_on_error.unwrap_or(true),
                parallel: compiler.parallel.unwrap_or(true),
                max_workers: compiler.max_workers.unwrap_or(8),
            },
            output: ResolvedOutputConfig {
                mirror_structure: output.mirror_structure.unwrap_or(true),
                keep_intermediates: output.keep_intermediates.unwrap_or(true),
            },
            general: ResolvedGeneralConfig {
                default_template: general.default_template,
                check_template_updates: general.check_template_updates.unwrap_or(true),
            },
            templates: ResolvedTemplateConfig {
                auto_update_latest: templates.auto_update_latest.unwrap_or(true),
                prerelease: templates.prerelease.unwrap_or(false),
            },
            themes: ResolvedThemeConfig {
                enabled: themes
                    .enabled
                    .unwrap_or_else(|| vec![ThemeVariant::Light, ThemeVariant::Dark]),
                default: themes.default.unwrap_or(ThemeVariant::Light),
            },
        }
    }
}

/// Merge global, project, and command-line layers over built-in defaults.
///
/// Precedence: built-in defaults < global < project < CLI.
pub fn resolve_config(global: &Config, project: &Config, cli: &Config) -> ResolvedConfig {
    Config::resolve_layers(&[global, project, cli])
}

#[cfg(test)]
mod tests {
    use super::resolve_config;
    use crate::config::{
        CompilerConfig, Config, DiscoveryConfig, LatexEngine, StructureConfig, ThemeConfig,
        ThemeVariant,
    };
    use std::path::PathBuf;

    #[test]
    fn empty_layers_resolve_to_built_in_defaults() {
        let resolved = resolve_config(&Config::default(), &Config::default(), &Config::default());
        assert_eq!(resolved.structure.source, PathBuf::from("src"));
        assert_eq!(resolved.compiler.engine, LatexEngine::PdfLatex);
        assert_eq!(resolved.compiler.max_workers, 8);
        assert_eq!(resolved.themes.default, ThemeVariant::Light);
    }

    #[test]
    fn project_values_override_global_values_field_by_field() {
        let global = Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::XeLatex),
                max_workers: Some(4),
                ..Default::default()
            }),
            ..Default::default()
        };
        let project = Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::LuaLatex),
                ..Default::default()
            }),
            ..Default::default()
        };

        let resolved = resolve_config(&global, &project, &Config::default());
        assert_eq!(resolved.compiler.engine, LatexEngine::LuaLatex);
        assert_eq!(resolved.compiler.max_workers, 4);
    }

    #[test]
    fn cli_values_have_highest_precedence() {
        let global = Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::XeLatex),
                ..Default::default()
            }),
            ..Default::default()
        };
        let project = Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::LuaLatex),
                ..Default::default()
            }),
            ..Default::default()
        };
        let cli = Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::PdfLatex),
                ..Default::default()
            }),
            ..Default::default()
        };

        let resolved = resolve_config(&global, &project, &cli);
        assert_eq!(resolved.compiler.engine, LatexEngine::PdfLatex);
    }

    #[test]
    fn partial_nested_overrides_preserve_unrelated_values() {
        let global = Config {
            structure: Some(StructureConfig {
                source: Some(PathBuf::from("documents")),
                build: Some(PathBuf::from(".build")),
                ..Default::default()
            }),
            discovery: Some(DiscoveryConfig {
                enabled: Some(false),
                ..Default::default()
            }),
            themes: Some(ThemeConfig {
                enabled: Some(vec![ThemeVariant::Dark]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let project = Config {
            structure: Some(StructureConfig {
                pdf: Some(PathBuf::from("output")),
                ..Default::default()
            }),
            ..Default::default()
        };

        let resolved = resolve_config(&global, &project, &Config::default());
        assert_eq!(resolved.structure.source, PathBuf::from("documents"));
        assert_eq!(resolved.structure.build, PathBuf::from(".build"));
        assert_eq!(resolved.structure.pdf, PathBuf::from("output"));
        assert!(!resolved.discovery.enabled);
        assert_eq!(resolved.themes.enabled, vec![ThemeVariant::Dark]);
        assert_eq!(resolved.themes.default, ThemeVariant::Light);
    }

    #[test]
    fn removing_a_project_override_restores_global_inheritance() {
        let global = Config {
            compiler: Some(CompilerConfig {
                max_workers: Some(6),
                ..Default::default()
            }),
            ..Default::default()
        };
        let project_override = Config {
            compiler: Some(CompilerConfig {
                max_workers: Some(2),
                ..Default::default()
            }),
            ..Default::default()
        };
        let no_project_override = Config::default();

        assert_eq!(
            resolve_config(&global, &project_override, &Config::default())
                .compiler
                .max_workers,
            2
        );
        assert_eq!(
            resolve_config(&global, &no_project_override, &Config::default())
                .compiler
                .max_workers,
            6
        );
    }

    #[test]
    fn overlay_does_not_replace_unset_fields() {
        let mut base = Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::XeLatex),
                max_workers: Some(5),
                ..Default::default()
            }),
            ..Default::default()
        };
        base.overlay(&Config {
            compiler: Some(CompilerConfig {
                engine: Some(LatexEngine::LuaLatex),
                ..Default::default()
            }),
            ..Default::default()
        });

        let compiler = base.compiler.unwrap();
        assert_eq!(compiler.engine, Some(LatexEngine::LuaLatex));
        assert_eq!(compiler.max_workers, Some(5));
    }
}
