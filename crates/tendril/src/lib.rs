pub mod cli;
pub mod commands;
pub mod config;
pub mod discovery;
pub mod error;
pub mod logging;
pub mod model;
pub mod platform;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;
use mcp_cli::JsonEnvelope;
use serde_json::Value;

pub use cli::TendrilCli;
pub use config::{
    CaptureDefaults, ConfigPaths, ImageFormat, LogLevel, LoggingDefaults, TendrilConfig,
};
pub use error::TendrilError;
pub use logging::{LogDestination, LoggingMode, LoggingPolicy};
pub use model::*;
pub use platform::{AdapterContext, PlatformAdapterError, current_adapter};

/// Parse and execute Tendril commands from an argument iterator.
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = TendrilCli::parse_from(args);
    let config = match TendrilConfig::load() {
        Ok(config) => config,
        Err(error) => return emit_error(&cli, None, &error),
    };

    let logging_mode = match &cli.command {
        Some(crate::cli::Command::Mcp(crate::cli::McpCommand {
            command: crate::cli::McpSubcommand::Stdio,
        })) => LoggingMode::McpStdio,
        _ => LoggingMode::Cli,
    };

    if let Err(error) = logging::init_logging(config.logging.level, logging_mode) {
        return emit_error(&cli, None, &error);
    }

    match commands::dispatch(&cli, &config) {
        Ok(output) => {
            output.print();
            ExitCode::SUCCESS
        }
        Err(error) => emit_error(
            &cli,
            cli.command.as_ref().map(crate::cli::Command::name),
            &error,
        ),
    }
}

fn emit_error(cli: &TendrilCli, command: Option<&str>, error: &TendrilError) -> ExitCode {
    if cli.json {
        let envelope: JsonEnvelope<Value> = match command {
            Some(command) => JsonEnvelope::error_for(command, error.to_json_error()),
            None => JsonEnvelope::error(error.to_json_error()),
        };
        let rendered = serde_json::to_string_pretty(&envelope).unwrap_or_else(|serialization_error| {
            format!(
                "{{\n  \"status\": \"error\",\n  \"meta\": {{ \"schema_version\": 1 }},\n  \"error\": {{ \"category\": \"serialization_error\", \"code\": \"serialization_error\", \"message\": \"failed to serialize error envelope: {serialization_error}\" }}\n}}"
            )
        });
        println!("{rendered}");
    } else {
        eprintln!("{error}");
    }

    ExitCode::from(1)
}
