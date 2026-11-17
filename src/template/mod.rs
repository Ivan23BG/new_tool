//! Installed template metadata and local registry inspection.

mod manifest;
mod registry;

pub use manifest::{TEMPLATE_MANIFEST_FILE, TemplateManifest, TemplateManifestError};
pub use registry::{InstalledTemplate, TemplateRegistry, TemplateRegistryError};
