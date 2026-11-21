//! Configuration data structures, built-in defaults, and file storage.
//!
//! `Config` represents a *partial* configuration: absent values are left as
//! `None` so they can inherit values from another configuration layer.

mod defaults;
mod manager;
mod resolve;
mod store;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use defaults::built_in_defaults;
pub use manager::{ConfigManager, ConfigScope};
pub use resolve::{
    ResolvedCompilerConfig, ResolvedConfig, ResolvedDiscoveryConfig, ResolvedGeneralConfig,
    ResolvedOutputConfig, ResolvedProjectConfig, ResolvedStructureConfig, ResolvedTemplateConfig,
    ResolvedThemeConfig, resolve_config,
};
pub use store::{ConfigError, ConfigStore, GLOBAL_CONFIG_FILE, PROJECT_CONFIG_FILE};

/// A partially specified latex_handler configuration.
///
/// Every section and setting is optional. This is intentional: a project
/// configuration should only need to contain values it wants to override.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Project identity and template dependency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectConfig>,
    /// Project directory layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<StructureConfig>,
    /// Automatic TeX document discovery rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery: Option<DiscoveryConfig>,
    /// Compilation backend and engine settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler: Option<CompilerConfig>,
    /// Generated output behaviour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputConfig>,
    /// User-wide defaults and preferences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub general: Option<GeneralConfig>,
    /// Template update preferences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub templates: Option<TemplateConfig>,
    /// Available document themes and the default theme.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub themes: Option<ThemeConfig>,
}

impl Config {
    /// Serialize this configuration as readable TOML.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Parse a configuration from TOML.
    pub fn from_toml(source: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(source)
    }

    /// Overlay explicitly configured values from `overrides` onto this layer.
    /// Unspecified values in `overrides` leave the existing values untouched.
    pub fn overlay(&mut self, overrides: &Config) {
        macro_rules! section {
            ($field:ident, $ty:ty, { $($setting:ident),* $(,)? }) => {
                if let Some(source) = &overrides.$field {
                    let target = self.$field.get_or_insert_with(<$ty>::default);
                    $(if source.$setting.is_some() {
                        target.$setting = source.$setting.clone();
                    })*
                }
            };
        }

        section!(project, ProjectConfig, { name, template, template_version });
        section!(structure, StructureConfig, { source, build, pdf, logs, assets });
        section!(discovery, DiscoveryConfig, { enabled, patterns, exclude });
        section!(compiler, CompilerConfig, {
            backend, engine, shell_escape, synctex, interaction, halt_on_error,
            parallel, max_workers
        });
        section!(output, OutputConfig, { mirror_structure, keep_intermediates });
        section!(general, GeneralConfig, { default_template, check_template_updates });
        section!(templates, TemplateConfig, { repository, auto_update_latest, prerelease });
        section!(themes, ThemeConfig, { enabled, default });
    }

    /// Resolve this configuration with later layers taking precedence.
    /// The arguments are ordered from lowest to highest precedence.
    pub fn resolve_layers(layers: &[&Config]) -> ResolvedConfig {
        let mut merged = built_in_defaults();
        for layer in layers {
            merged.overlay(layer);
        }
        ResolvedConfig::from_config(merged)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StructureConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<PathBuf>,
    /// Project-local directory for assets (images, bibliography, shared snippets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patterns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompilerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<LatexEngine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_escape: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synctex: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub halt_on_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_workers: Option<usize>,
}

/// A LaTeX engine, distinct from the compilation backend (such as latexmk).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LatexEngine {
    PdfLatex,
    XeLatex,
    LuaLatex,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror_structure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_intermediates: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_template_updates: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_update_latest: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prerelease: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Vec<ThemeVariant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<ThemeVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeVariant {
    Light,
    Dark,
}

#[cfg(test)]
mod tests;
