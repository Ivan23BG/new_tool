//! Template metadata, installation, and registry management.

mod installer;
mod manifest;
mod registry;

pub use installer::{TemplateInstallError, install_template};
pub use manifest::{TEMPLATE_MANIFEST_FILE, TemplateManifest, TemplateManifestError};
pub use registry::{InstalledTemplate, TemplateRegistry, TemplateRegistryError};
