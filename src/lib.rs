//! Core library for `latex_handler`.
//!
//! The command-line interface is intentionally kept thin. Project discovery,
//! configuration, template management, and compilation will live in separate
//! modules and remain usable independently of the CLI.

pub mod cli;
pub mod config;
pub mod project;
