use super::{Config, LatexEngine, ThemeVariant};

#[test]
fn partial_configuration_round_trips_through_toml() {
    let source = r#"
[compiler]
engine = "lualatex"
max_workers = 4

[themes]
default = "dark"
"#;

    let config = Config::from_toml(source).expect("valid TOML");
    let compiler = config.compiler.as_ref().unwrap();
    assert_eq!(compiler.engine, Some(LatexEngine::LuaLatex));
    assert_eq!(compiler.max_workers, Some(4));
    assert_eq!(compiler.parallel, None);

    let serialized = config.to_toml().expect("serializable config");
    let reparsed = Config::from_toml(&serialized).expect("serialized TOML is valid");
    assert_eq!(config, reparsed);
}

#[test]
fn empty_toml_produces_an_empty_partial_configuration() {
    let config = Config::from_toml("").expect("empty TOML is valid");
    assert_eq!(config, Config::default());
}

#[test]
fn enum_values_use_documented_lowercase_names() {
    let config =
        Config::from_toml("[compiler]\nengine = \"xelatex\"\n[themes]\ndefault = \"light\"\n")
            .expect("enum values should deserialize");

    assert_eq!(config.compiler.unwrap().engine, Some(LatexEngine::XeLatex));
    assert_eq!(config.themes.unwrap().default, Some(ThemeVariant::Light));
}

#[test]
fn invalid_enum_values_are_rejected() {
    let result = Config::from_toml("[compiler]\nengine = \"tectonic\"\n");
    assert!(result.is_err());
}

#[test]
fn template_repository_is_a_partial_global_configuration_value() {
    let config =
        Config::from_toml("[templates]\nrepository = \"https://example.invalid/templates.git\"\n")
            .expect("template repository should deserialize");

    assert_eq!(
        config.templates.unwrap().repository.as_deref(),
        Some("https://example.invalid/templates.git")
    );
}
