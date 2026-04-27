use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};
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
        format!(
            "Tendril is a stateless desktop inspection and control CLI for agents.\n\nWorkflow:\n  1. list targets:   tendril list --json\n  2. capture state:  tendril --window <id> capture --json\n  3. save to file:   tendril --window <id> capture -o /tmp/screen.png\n  4. run input:      tendril --window <id> run 'send(\"hello\")'\n  5. read clipboard: tendril clipboard get --json\n  6. reuse a target: eval \"$(tendril --window <id> alias --name desk)\"\n\nCommands:\n  list       Discover windows and displays\n  capture    Capture a screenshot from a window or display\n  run        Type text or execute an input sequence against a target\n  clipboard  Read or serve Linux/X11 text selections for deterministic browser↔OS transfer\n  alias      Emit a shell helper that pre-fills --window/--display\n  listen     Probe supported audio capture paths\n  mcp        Serve Tendril over MCP stdio\n\nUse --json for machine-readable success/error envelopes.\nUse -o/--output on capture to save the image directly to a file.\nUse --help on any subcommand for detailed flags.\n\n{WORKFLOW_HINT}\n"
        )
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
    /// Read or serve an OS clipboard/text selection.
    Clipboard(ClipboardCommand),
    /// Emit shell helpers for repeated targeting.
    Alias(AliasCommand),
    /// Expose the CLI surface over MCP stdio.
    ///
    /// Note: the global --window, --display, and --json flags are inherited
    /// from the top-level CLI but DO NOT apply to the MCP server. MCP tool
    /// calls carry their own `target` and option arguments in each request
    /// payload. Passing those flags to `tendril mcp ...` is rejected with a
    /// validation error to avoid silently ignored configuration.
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
            Self::Clipboard(_) => "clipboard",
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

    /// Write the decoded image to a file. Side-effecting: when combined with
    /// --json the JSON envelope is still printed to stdout.
    #[arg(short = 'o', long)]
    #[serde(skip)]
    pub output: Option<PathBuf>,

    /// Maximum time (milliseconds) to wait for the underlying capture backend
    /// before giving up. On Wayland this bounds both the xdg-desktop-portal
    /// screenshot D-Bus call and the `grim` fallback subprocess so a hung
    /// portal or compositor cannot freeze an agent session.
    #[arg(long = "timeout-ms")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct RunCommand {
    /// Disable the default host-local execution lock/queue for this run.
    #[arg(long = "no-lock")]
    #[serde(default)]
    pub no_lock: bool,

    /// Maximum time in milliseconds to wait for the host-local execution lock.
    #[arg(long = "lock-timeout-ms")]
    pub lock_timeout_ms: Option<u64>,

    /// Age in milliseconds after which an unrefreshed lock/ticket is considered stale.
    #[arg(long = "lock-stale-ms")]
    pub lock_stale_ms: Option<u64>,

    /// Override the host-local execution lock root path for advanced workflows.
    #[arg(long = "lock-path")]
    pub lock_path: Option<PathBuf>,

    /// Text or DSL input definition, e.g. `send("hi")`, `lclick(10,20)`, or `scroll(10,20,3)`.
    pub input_definition: Option<String>,

    /// Restore the window/app focus that was active before the run, when the
    /// active platform adapter can observe and restore focus. This is the
    /// default to minimize disruption on shared desktops.
    #[arg(long = "restore-focus", default_value_t = true, action = ArgAction::SetTrue)]
    #[serde(default = "default_restore_focus")]
    pub restore_focus: bool,

    /// Preserve legacy behavior for workflows that intentionally leave focus
    /// on the automation target after `tendril run`.
    #[arg(long = "no-restore-focus", action = ArgAction::SetTrue)]
    #[serde(default)]
    pub no_restore_focus: bool,
}

impl Default for RunCommand {
    fn default() -> Self {
        Self {
            no_lock: false,
            lock_timeout_ms: None,
            lock_stale_ms: None,
            lock_path: None,
            input_definition: None,
            restore_focus: true,
            no_restore_focus: false,
        }
    }
}

fn default_restore_focus() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct ClipboardCommand {
    #[command(subcommand)]
    pub command: ClipboardSubcommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ClipboardSubcommand {
    /// Read a text selection from the OS clipboard.
    Get(ClipboardGetCommand),
    /// Own and serve a text selection for other applications to paste.
    Set(ClipboardSetCommand),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct ClipboardGetCommand {
    /// X11 selection to read: `clipboard` (Ctrl+C/Ctrl+V) or `primary` (selection/middle-click).
    #[arg(long)]
    pub selection: Option<String>,

    /// Maximum time in milliseconds to wait for the owner to answer.
    #[arg(long = "timeout-ms")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct ClipboardSetCommand {
    /// X11 selection to serve: `clipboard` (Ctrl+C/Ctrl+V) or `primary` (selection/middle-click).
    #[arg(long)]
    pub selection: Option<String>,

    /// Text to expose through the selection while this process serves requests.
    #[arg(long)]
    pub text: String,

    /// Time in milliseconds to stay alive serving paste/read requests.
    #[arg(long = "serve-ms")]
    pub serve_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct ListenCommand {
    /// Audio source selector: `system`, `loopback`, `microphone`, or `device:<id>`.
    #[arg(long)]
    pub source: Option<String>,

    /// Requested capture duration in milliseconds.
    #[arg(long)]
    pub duration_ms: Option<u64>,

    /// Requested audio format: `wav`, `flac`, or `opus`.
    #[arg(long)]
    pub format: Option<String>,

    /// Write the captured audio to a file at this path. When omitted, listen
    /// allocates a temporary file on platforms that support real capture and
    /// returns its path in the JSON envelope.
    #[arg(short = 'o', long)]
    #[serde(skip)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct AliasCommand {
    #[arg(long)]
    pub shell: Option<String>,

    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Args)]
#[command(long_about = "Expose the Tendril CLI surface over MCP.\n\n\
Note: the global --window, --display, and --json flags are inherited from the\n\
top-level CLI but DO NOT apply to the MCP server. MCP tool calls carry their\n\
own `target` and option arguments in each request payload. Passing those\n\
flags to `tendril mcp ...` will be rejected with an error to avoid silently\n\
ignored configuration.")]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum McpSubcommand {
    /// Serve MCP tools over stdio.
    ///
    /// The server reads JSON-RPC requests from stdin and writes responses to
    /// stdout. Target scoping (window/display) and JSON envelope formatting
    /// are controlled per-call via the MCP tool arguments, not via top-level
    /// CLI flags. Passing --window, --display, or --json alongside this
    /// subcommand is rejected.
    Stdio,
}

#[cfg(test)]
mod tests {
    use super::TendrilCli;

    #[test]
    fn agent_help_includes_workflow_hint() {
        let help = TendrilCli::agent_help();

        assert!(help.contains("Workflow:"));
        assert!(help.contains("tendril list --json"));
        assert!(help.contains("capture --json"));
        assert!(help.contains("capture -o /tmp/screen.png"));
        assert!(help.contains(" run 'send(\"hello\")'"));
        assert!(help.contains("clipboard get --json"));
        assert!(help.contains("alias --name desk"));
    }
}
