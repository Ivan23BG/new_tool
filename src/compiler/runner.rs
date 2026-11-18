//! Process-execution abstraction so compiler logic can be tested without TeX.

use std::{
    ffi::OsString,
    io,
    path::PathBuf,
    process::{Command, Stdio},
};

/// A fully specified external command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<OsString>,
    pub working_dir: PathBuf,
    pub env: Vec<(OsString, OsString)>,
}

impl CommandSpec {
    /// Render the command for diagnostics. This is for humans only and is not
    /// guaranteed to be safe to paste into a shell.
    pub fn display(&self) -> String {
        let mut text = self.program.clone();
        for arg in &self.args {
            text.push(' ');
            let arg = arg.to_string_lossy();
            if arg.contains(char::is_whitespace) {
                text.push_str(&format!("'{arg}'"));
            } else {
                text.push_str(&arg);
            }
        }
        text
    }
}

/// Captured result of a finished process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// `None` when the process was terminated by a signal.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs external commands. Implemented for real processes by [`SystemRunner`]
/// and by fakes in tests.
pub trait CommandRunner: Send + Sync {
    fn run(&self, spec: &CommandSpec) -> io::Result<CommandOutput>;
}

/// Runs commands as operating-system processes.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, spec: &CommandSpec) -> io::Result<CommandOutput> {
        let output = Command::new(&spec.program)
            .args(&spec.args)
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&spec.working_dir)
            // latexmk must never wait for interactive input.
            .stdin(Stdio::null())
            .output()?;
        Ok(CommandOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
