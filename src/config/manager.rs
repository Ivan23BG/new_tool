//! User-facing configuration operations, including comment-preserving edits.

use super::{Config, ConfigError, ConfigStore};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use toml_edit::{DocumentMut, Item, Table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    Global,
    Project,
}

#[derive(Debug, Clone)]
pub struct ConfigManager {
    store: ConfigStore,
    project_root: PathBuf,
}

impl ConfigManager {
    pub fn new(store: ConfigStore, project_root: impl Into<PathBuf>) -> Self {
        Self {
            store,
            project_root: project_root.into(),
        }
    }

    pub fn default_scope(&self) -> ConfigScope {
        if ConfigStore::project_config_path(&self.project_root).is_file() {
            ConfigScope::Project
        } else {
            ConfigScope::Global
        }
    }

    pub fn path(&self, scope: ConfigScope) -> PathBuf {
        match scope {
            ConfigScope::Global => self.store.global_config_path(),
            ConfigScope::Project => ConfigStore::project_config_path(&self.project_root),
        }
    }

    pub fn show(&self, scope: Option<ConfigScope>) -> Result<String, ConfigError> {
        match scope {
            Some(ConfigScope::Global) => {
                let config = self.store.load_global()?;
                toml::to_string_pretty(&config).map_err(|e| ConfigError::UnknownKey(e.to_string()))
            }
            Some(ConfigScope::Project) => {
                let config = self.store.load_project(&self.project_root)?;
                toml::to_string_pretty(&config).map_err(|e| ConfigError::UnknownKey(e.to_string()))
            }
            None => {
                let resolved = self
                    .store
                    .load_resolved(&self.project_root, &Config::default())?;
                toml::to_string_pretty(&resolved)
                    .map_err(|e| ConfigError::UnknownKey(e.to_string()))
            }
        }
    }

    pub fn get(&self, key: &str, scope: Option<ConfigScope>) -> Result<String, ConfigError> {
        let value = match scope {
            Some(ConfigScope::Global) => config_value(&self.store.load_global()?, key)?,
            Some(ConfigScope::Project) => {
                config_value(&self.store.load_project(&self.project_root)?, key)?
            }
            None => {
                let resolved = self
                    .store
                    .load_resolved(&self.project_root, &Config::default())?;
                let value = toml::Value::try_from(resolved)
                    .map_err(|e| ConfigError::UnknownKey(e.to_string()))?;
                lookup_value(&value, key)?
            }
        };
        Ok(value.to_string())
    }

    pub fn set(&self, key: &str, raw_value: &str, scope: ConfigScope) -> Result<(), ConfigError> {
        validate_key(key)?;
        let path = self.path(scope);
        self.ensure_file(scope)?;
        let source = fs::read_to_string(&path).map_err(|source| ConfigError::Io {
            path: path.clone(),
            source,
        })?;
        let mut document = source.parse::<DocumentMut>().map_err(|e| {
            ConfigError::UnknownKey(format!("invalid TOML in {}: {e}", path.display()))
        })?;
        let item = parse_item(raw_value)
            .map_err(|e| ConfigError::UnknownKey(format!("invalid value for '{key}': {e}")))?;
        set_item(&mut document, &key.split('.').collect::<Vec<_>>(), item)?;
        fs::write(&path, document.to_string()).map_err(|source| ConfigError::Io { path, source })
    }

    pub fn reset(&self, key: Option<&str>, scope: ConfigScope) -> Result<(), ConfigError> {
        let path = self.path(scope);
        self.ensure_file(scope)?;
        if key.is_none() {
            let template = match scope {
                ConfigScope::Global => super::store::global_template(),
                ConfigScope::Project => super::store::project_template(),
            };
            fs::write(&path, template).map_err(|source| ConfigError::Io { path, source })?;
            return Ok(());
        }
        let key = key.unwrap();
        validate_key(key)?;
        let source = fs::read_to_string(&path).map_err(|source| ConfigError::Io {
            path: path.clone(),
            source,
        })?;
        let mut document = source
            .parse::<DocumentMut>()
            .map_err(|e| ConfigError::UnknownKey(e.to_string()))?;
        remove_item(&mut document, &key.split('.').collect::<Vec<_>>())?;
        fs::write(&path, document.to_string()).map_err(|source| ConfigError::Io { path, source })
    }

    pub fn edit(&self, scope: ConfigScope) -> Result<(), ConfigError> {
        self.ensure_file(scope)?;
        let editor = env::var("VISUAL")
            .or_else(|_| env::var("EDITOR"))
            .map_err(|_| ConfigError::EditorUnavailable)?;
        let mut parts = editor.split_whitespace();
        let executable = parts.next().ok_or(ConfigError::EditorUnavailable)?;
        let status = Command::new(executable)
            .args(parts)
            .arg(self.path(scope))
            .status()
            .map_err(ConfigError::EditorLaunch)?;
        if status.success() {
            Ok(())
        } else {
            Err(ConfigError::EditorFailed)
        }
    }

    fn ensure_file(&self, scope: ConfigScope) -> Result<(), ConfigError> {
        match scope {
            ConfigScope::Global => self.store.ensure_global().map(|_| ()),
            ConfigScope::Project => self.store.ensure_project(&self.project_root).map(|_| ()),
        }
    }
}

fn config_value(config: &Config, key: &str) -> Result<toml::Value, ConfigError> {
    let value =
        toml::Value::try_from(config).map_err(|e| ConfigError::UnknownKey(e.to_string()))?;
    lookup_value(&value, key)
}

fn lookup_value(value: &toml::Value, key: &str) -> Result<toml::Value, ConfigError> {
    let mut current = value;
    for part in key.split('.') {
        current = current
            .get(part)
            .ok_or_else(|| ConfigError::UnknownKey(key.to_owned()))?;
    }
    Ok(current.clone())
}

fn validate_key(key: &str) -> Result<(), ConfigError> {
    const KEYS: &[&str] = &[
        "project.name",
        "project.template",
        "project.template_version",
        "structure.source",
        "structure.build",
        "structure.pdf",
        "structure.logs",
        "structure.assets",
        "discovery.enabled",
        "discovery.patterns",
        "discovery.exclude",
        "compiler.backend",
        "compiler.engine",
        "compiler.shell_escape",
        "compiler.synctex",
        "compiler.interaction",
        "compiler.halt_on_error",
        "compiler.parallel",
        "compiler.max_workers",
        "output.mirror_structure",
        "output.keep_intermediates",
        "general.default_template",
        "general.check_template_updates",
        "templates.repository",
        "templates.auto_update_latest",
        "templates.prerelease",
        "themes.enabled",
        "themes.default",
    ];
    if KEYS.contains(&key) {
        Ok(())
    } else {
        Err(ConfigError::UnknownKey(key.to_owned()))
    }
}

fn parse_item(raw: &str) -> Result<Item, toml_edit::TomlError> {
    let wrapped = format!("value = {raw}\n");
    match wrapped.parse::<DocumentMut>() {
        Ok(doc) => Ok(doc["value"].clone()),
        Err(_) => Ok(Item::Value(toml_edit::Value::from(raw))),
    }
}

fn set_item(doc: &mut DocumentMut, path: &[&str], item: Item) -> Result<(), ConfigError> {
    if path.is_empty() {
        return Err(ConfigError::UnknownKey(String::new()));
    }
    let mut table = doc.as_table_mut();
    for part in &path[..path.len() - 1] {
        if !table.contains_key(part) {
            table.insert(part, Item::Table(Table::new()));
        }
        table = table
            .get_mut(part)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| ConfigError::UnknownKey(path.join(".")))?;
    }
    table.insert(path[path.len() - 1], item);
    Ok(())
}

fn remove_item(doc: &mut DocumentMut, path: &[&str]) -> Result<(), ConfigError> {
    let mut table = doc.as_table_mut();
    for part in &path[..path.len() - 1] {
        table = table
            .get_mut(part)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| ConfigError::UnknownKey(path.join(".")))?;
    }
    if table.remove(path[path.len() - 1]).is_none() {
        return Err(ConfigError::UnknownKey(path.join(".")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn set_preserves_comments_and_updates_only_requested_value() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let store = ConfigStore::new(dir.path().join("global"));
        let path = ConfigStore::project_config_path(&project);
        fs::write(
            &path,
            "# keep this comment\n[compiler]\n# keep this too\nmax_workers = 4\n",
        )
        .unwrap();
        let manager = ConfigManager::new(store, &project);
        manager
            .set("compiler.engine", "lualatex", ConfigScope::Project)
            .unwrap();
        let result = fs::read_to_string(path).unwrap();
        assert!(result.contains("# keep this comment"));
        assert!(result.contains("# keep this too"));
        assert!(result.contains("engine = \"lualatex\""));
        assert!(result.contains("max_workers = 4"));
    }

    #[test]
    fn reset_removes_override_and_preserves_other_values() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let manager = ConfigManager::new(ConfigStore::new(dir.path().join("global")), &project);
        manager
            .set("compiler.max_workers", "3", ConfigScope::Project)
            .unwrap();
        manager
            .set("compiler.engine", "\"lualatex\"", ConfigScope::Project)
            .unwrap();
        manager
            .reset(Some("compiler.max_workers"), ConfigScope::Project)
            .unwrap();
        let config =
            Config::from_toml(&fs::read_to_string(manager.path(ConfigScope::Project)).unwrap())
                .unwrap();
        assert_eq!(config.compiler.unwrap().max_workers, None);
    }

    #[test]
    fn typed_values_are_parsed_and_unknown_keys_rejected() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let manager = ConfigManager::new(ConfigStore::new(dir.path().join("global")), &project);
        manager
            .set("compiler.max_workers", "6", ConfigScope::Project)
            .unwrap();
        assert_eq!(
            manager
                .get("compiler.max_workers", Some(ConfigScope::Project))
                .unwrap(),
            "6"
        );
        assert!(
            manager
                .set("compiler.not_a_setting", "true", ConfigScope::Project)
                .is_err()
        );
    }
}
