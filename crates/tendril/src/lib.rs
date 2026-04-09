pub mod cli;
pub mod commands;
pub mod config;
pub mod error;
pub mod platform;

use std::ffi::OsString;

use clap::Parser;

pub use cli::TendrilCli;
pub use config::{CaptureDefaults, ConfigPaths, ImageFormat, TendrilConfig};
pub use error::TendrilError;
pub use platform::{AdapterContext, PlatformAdapterError, current_adapter};

/// Parse and execute Tendril commands from an argument iterator.
pub fn run<I, T>(args: I) -> Result<(), TendrilError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = TendrilCli::parse_from(args);
    commands::dispatch(&cli)
}
