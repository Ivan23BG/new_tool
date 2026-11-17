use super::args::Command;
use std::error::Error;

/// Dispatch parsed commands. Individual command implementations will be added
/// in later stages; keeping dispatch centralized avoids putting application
/// logic in the argument definitions.
pub fn execute(command: Command) -> Result<(), Box<dyn Error + Send + Sync>> {
    match command {
        Command::New(_) => not_implemented("new"),
        Command::Init(_) => not_implemented("init"),
        Command::Compile(_) => not_implemented("compile"),
        Command::Project { .. } => not_implemented("project"),
        Command::Template { .. } => not_implemented("template"),
        Command::Config { .. } => not_implemented("config"),
        Command::Clean(_) => not_implemented("clean"),
        Command::Update => not_implemented("update"),
        Command::Version => {
            println!("latex_handler {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn not_implemented(command: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!("`{command}` is not implemented yet"),
    )
    .into())
}
