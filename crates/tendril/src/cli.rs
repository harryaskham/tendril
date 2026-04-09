use clap::{Args, CommandFactory, Parser, Subcommand};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Recommended high-level workflow for agents and operators.
pub const WORKFLOW_HINT: &str =
    "Recommended workflow: tendril list -> tendril capture -> tendril run";

/// Top-level Tendril CLI.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "tendril",
    version,
    about = "Stateless desktop inspection and control for agents.",
    long_about = None,
    disable_help_subcommand = true,
    propagate_version = true
)]
pub struct TendrilCli {
    /// Emit machine-readable JSON envelopes.
    #[arg(long, global = true)]
    pub json: bool,

    /// Scope target-aware commands to a window.
    #[arg(long, global = true)]
    pub window: Option<String>,

    /// Scope target-aware commands to a display.
    #[arg(long, global = true)]
    pub display: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

impl TendrilCli {
    #[must_use]
    pub fn agent_help() -> String {
        let mut command = Self::command();
        let mut output = Vec::new();
        command
            .write_long_help(&mut output)
            .expect("help text should render into memory");

        let mut help = String::from_utf8(output).expect("clap help must be valid utf-8");
        help.push_str("\n\n");
        help.push_str(WORKFLOW_HINT);
        help.push('\n');
        help
    }
}

/// Top-level commands scaffolded for future feature work.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Discover windows, displays, and other future targets.
    List(ListCommand),
    /// Capture a screenshot from a window or display target.
    Capture(CaptureCommand),
    /// Execute input against a target.
    Run(RunCommand),
    /// Capture audio from a supported source.
    Listen(ListenCommand),
    /// Emit shell helpers for repeated targeting.
    Alias(AliasCommand),
    /// Expose the CLI surface over MCP stdio.
    Mcp(McpCommand),
}

impl Command {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::List(_) => "list",
            Self::Capture(_) => "capture",
            Self::Run(_) => "run",
            Self::Listen(_) => "listen",
            Self::Alias(_) => "alias",
            Self::Mcp(_) => "mcp",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct ListCommand {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct CaptureCommand {
    #[arg(long)]
    pub max_width: Option<u32>,

    #[arg(long)]
    pub max_height: Option<u32>,

    #[arg(long)]
    pub format: Option<String>,

    #[arg(long)]
    pub compression: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct RunCommand {
    /// Placeholder input definition argument for future DSL and string support.
    pub input_definition: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct ListenCommand {
    #[arg(long)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct AliasCommand {
    #[arg(long)]
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum McpSubcommand {
    /// Serve MCP tools over stdio.
    Stdio,
}

#[cfg(test)]
mod tests {
    use super::TendrilCli;

    #[test]
    fn agent_help_includes_workflow_hint() {
        let help = TendrilCli::agent_help();

        assert!(help.contains("Recommended workflow:"));
        assert!(help.contains("tendril capture"));
        assert!(help.contains("tendril run"));
    }
}
