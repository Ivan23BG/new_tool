//! Template metadata, installation, and registry management.

mod dependencies;
mod installer;
mod management;
mod manifest;
mod registry;
mod updater;

pub use dependencies::{
    TemplateDependencyError, install_template_with_dependencies, validate_template_dependencies,
};
pub use installer::{TemplateInstallError, install_template};
pub use management::{TemplateManagementError, list_template_versions, remove_template};
pub use manifest::{
    TEMPLATE_MANIFEST_FILE, TemplateDependency, TemplateManifest, TemplateManifestError,
};
pub use registry::{InstalledTemplate, TemplateRegistry, TemplateRegistryError};
pub use updater::{TemplateUpdateError, update_template};
