use std::sync::OnceLock;

use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use crate::config::LogLevel;
use crate::error::TendrilError;

static LOGGING_INIT: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoggingMode {
    Cli,
    McpStdio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogDestination {
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggingPolicy {
    pub destination: LogDestination,
    pub ansi: bool,
    pub quiet_stdout: bool,
}

impl LoggingMode {
    #[must_use]
    pub fn policy(self) -> LoggingPolicy {
        match self {
            Self::Cli => LoggingPolicy {
                destination: LogDestination::Stderr,
                ansi: false,
                quiet_stdout: false,
            },
            Self::McpStdio => LoggingPolicy {
                destination: LogDestination::Stderr,
                ansi: false,
                quiet_stdout: true,
            },
        }
    }
}

pub fn init_logging(level: LogLevel, mode: LoggingMode) -> Result<(), TendrilError> {
    if LOGGING_INIT.get().is_some() {
        return Ok(());
    }

    let policy = mode.policy();
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level.as_tracing_level())
        .with_ansi(policy.ansi)
        .with_writer(std::io::stderr)
        .without_time()
        .finish();

    tracing::subscriber::set_global_default(subscriber).map_err(|error| {
        TendrilError::serialization(format!("failed to initialize logging subscriber: {error}"))
    })?;

    let _ = LOGGING_INIT.set(());
    Ok(())
}

impl LogLevel {
    #[must_use]
    pub fn as_tracing_level(self) -> Level {
        match self {
            Self::Error => Level::ERROR,
            Self::Warn => Level::WARN,
            Self::Info => Level::INFO,
            Self::Debug => Level::DEBUG,
            Self::Trace => Level::TRACE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LogDestination, LoggingMode};

    #[test]
    fn mcp_stdio_logging_policy_is_protocol_safe() {
        let policy = LoggingMode::McpStdio.policy();

        assert_eq!(policy.destination, LogDestination::Stderr);
        assert!(!policy.ansi);
        assert!(policy.quiet_stdout);
    }

    #[test]
    fn cli_logging_policy_still_avoids_stdout_log_pollution() {
        let policy = LoggingMode::Cli.policy();

        assert_eq!(policy.destination, LogDestination::Stderr);
        assert!(!policy.ansi);
        assert!(!policy.quiet_stdout);
    }
}
