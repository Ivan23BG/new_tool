//! Built-in configuration values.

use super::{
    CompilerConfig, Config, DiscoveryConfig, GeneralConfig, LatexEngine, OutputConfig,
    StructureConfig, TemplateConfig, ThemeConfig, ThemeVariant,
};
use std::path::PathBuf;

/// Return the built-in defaults used when no configuration layer overrides a
/// setting. The returned `Config` is fully populated, unlike `Config::default()`
/// which represents an empty/partial configuration.
pub fn built_in_defaults() -> Config {
    Config {
        project: None,
        structure: Some(StructureConfig {
            source: Some(PathBuf::from("src")),
            build: Some(PathBuf::from("build")),
            pdf: Some(PathBuf::from("pdfs")),
            logs: Some(PathBuf::from("logs")),
            assets: Some(PathBuf::from("assets")),
        }),
        discovery: Some(DiscoveryConfig {
            enabled: Some(true),
            patterns: Some(vec!["main.tex".into(), "*_main.tex".into()]),
            exclude: Some(vec![
                "legacy".into(),
                "templates".into(),
                "tmp".into(),
                "temp".into(),
                "assets".into(),
            ]),
        }),
        compiler: Some(CompilerConfig {
            backend: Some("latexmk".into()),
            engine: Some(LatexEngine::PdfLatex),
            shell_escape: Some(true),
            synctex: Some(true),
            interaction: Some("nonstopmode".into()),
            halt_on_error: Some(true),
            parallel: Some(true),
            max_workers: Some(8),
        }),
        output: Some(OutputConfig {
            mirror_structure: Some(true),
            keep_intermediates: Some(true),
        }),
        general: Some(GeneralConfig {
            default_template: None,
            check_template_updates: Some(true),
        }),
        templates: Some(TemplateConfig {
            repository: None,
            auto_update_latest: Some(true),
            prerelease: Some(false),
        }),
        themes: Some(ThemeConfig {
            enabled: Some(vec![ThemeVariant::Light, ThemeVariant::Dark]),
            default: Some(ThemeVariant::Light),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::built_in_defaults;
    use crate::config::{LatexEngine, ThemeVariant};
    use std::path::PathBuf;

    #[test]
    fn built_in_defaults_define_standard_directories() {
        let config = built_in_defaults();
        let structure = config.structure.expect("structure defaults");

        assert_eq!(structure.source.unwrap(), PathBuf::from("src"));
        assert_eq!(structure.build.unwrap(), PathBuf::from("build"));
        assert_eq!(structure.pdf.unwrap(), PathBuf::from("pdfs"));
        assert_eq!(structure.logs.unwrap(), PathBuf::from("logs"));
        assert_eq!(structure.assets.unwrap(), PathBuf::from("assets"));
    }

    #[test]
    fn built_in_defaults_define_compiler_and_themes() {
        let config = built_in_defaults();

        assert_eq!(config.compiler.unwrap().engine, Some(LatexEngine::PdfLatex));
        let themes = config.themes.unwrap();
        assert_eq!(themes.default, Some(ThemeVariant::Light));
        assert_eq!(
            themes.enabled,
            Some(vec![ThemeVariant::Light, ThemeVariant::Dark])
        );
    }
}
