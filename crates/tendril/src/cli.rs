use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
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

    /// Execute this Tendril invocation on a remote host over SSH.
    ///
    /// The remote host must have `tendril` on PATH (or set
    /// `TENDRIL_REMOTE_BIN` on the remote side). For Linux desktops, Tendril
    /// bootstraps X11/Wayland session variables before launching the remote
    /// command so SSH logins do not need `DISPLAY/WAYLAND_DISPLAY` pre-wired.
    #[arg(long, global = true, value_name = "USER@HOST")]
    pub remote: Option<String>,

    /// Proxy this invocation from WSL/Linux to a Windows-host Tendril binary.
    ///
    /// The Windows side must have `tendril.exe` on PATH, or set
    /// `TENDRIL_WSL_WINDOWS_BIN` to the Windows executable path visible from
    /// the Linux/WSL environment. This flag composes with --remote by being
    /// forwarded to the remote Tendril process.
    #[arg(long, global = true)]
    pub wsl_tunnel: bool,

    /// Drive an Android device or emulator through adb instead of the desktop backend.
    ///
    /// Pass an adb serial such as `sgu24:5555`, `emulator-5554`, or `auto` to
    /// select the single connected device. When omitted, Tendril also honors
    /// `TENDRIL_ANDROID_SERIAL`.
    #[arg(long, global = true, value_name = "SERIAL")]
    pub android: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

impl TendrilCli {
    #[must_use]
    pub fn agent_help() -> String {
        format!(
            "Tendril is a stateless desktop inspection and control CLI for agents.\n\nWorkflow:\n  1. list targets:    tendril list --json\n  2. remote targets:  tendril --remote me@box list --json\n  3. WSL host:        tendril --wsl-tunnel list --json\n  4. Android device:  tendril --android sgu24:5555 list --json\n  5. list elements:   tendril --window <id> list-elements --json\n  6. capture state:   tendril --window <id> capture --json\n  7. save to file:    tendril --window <id> capture -o /tmp/screen.png\n  8. run input:       tendril --window <id> run 'send(\"hello\")'\n  9. click element:   tendril --window <id> run 'click(33)'\n  10. read clipboard: tendril clipboard get --json\n  11. reuse a target: eval \"$(tendril --window <id> alias --name desk)\"\n\nCommands:\n  list           Discover windows, displays, and Android devices\n  list-elements  Discover UI elements for a window/display or Android device\n  capture        Capture a screenshot from a window/display/Android target\n  run            Type text or execute an input sequence against a target\n  clipboard      Read or serve Linux/X11 text selections for deterministic browser↔OS transfer\n  alias          Emit a shell helper that pre-fills --window/--display\n  listen         Probe supported audio capture paths\n  update         Download and install a Tendril release binary\n  version        Inspect or bump the workspace release version\n  mcp            Serve Tendril over MCP stdio\n\nUse --json for machine-readable success/error envelopes.\nUse --remote user@host to proxy any invocation over ssh; Linux remotes auto-discover X11/Wayland session variables when SSH did not inherit them.\nUse -o/--output on capture to save the image directly to a file.\nUse --help on any subcommand for detailed flags.\n\n{WORKFLOW_HINT}\n"
        )
    }
}

/// Top-level commands scaffolded for future feature work.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Discover windows, displays, and other future targets.
    List(ListCommand),
    /// List lower-level UI elements for a window/display or globally.
    #[command(name = "list-elements")]
    ListElements(ElementListCommand),
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
    /// Download and install a Tendril release binary.
    Update(UpdateCommand),
    /// Inspect or bump the workspace release version.
    Version(VersionCommand),
    /// Report input and screen-capture permission status for this platform.
    Permissions(PermissionsCommand),
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
            Self::ListElements(_) => "list-elements",
            Self::Capture(_) => "capture",
            Self::Run(_) => "run",
            Self::Listen(_) => "listen",
            Self::Clipboard(_) => "clipboard",
            Self::Alias(_) => "alias",
            Self::Update(_) => "update",
            Self::Version(_) => "version",
            Self::Permissions(_) => "permissions",
            Self::Mcp(_) => "mcp",
        }
    }
}

/// Report input and screen-capture permission status for the active platform adapter.
///
/// This is a read-only probe: it reports whether Screen Recording, Accessibility
/// (input control), and Microphone access are granted, unknown, denied, or not
/// required, along with remediation guidance. It performs no capture or input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct PermissionsCommand {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct ListCommand {
    /// Android only: include all launchable apps as switchable window targets.
    #[arg(long = "all-apps")]
    #[serde(default)]
    pub all_apps: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct ElementListCommand {
    /// Include elements outside the target bounds when the platform backend reports them.
    #[arg(long)]
    #[serde(default)]
    pub include_offscreen: bool,
}

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

    /// Text or DSL input definition, e.g. `send("hi")`, `lclick(10,20)`, `hover(10,20)`, `dblclick(10,20)`, `scroll(10,20,3)`, or `click(33)` for an element from list-elements.
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Args, Serialize, Deserialize, JsonSchema)]
pub struct UpdateCommand {
    /// Install a specific release version. Defaults to the latest GitHub release.
    #[arg(long = "release-version")]
    pub release_version: Option<String>,

    /// Override the GitHub repository in owner/name form.
    #[arg(long)]
    pub repository: Option<String>,

    /// Directory where the tendril binary should be installed. Defaults to ~/.local/bin.
    #[arg(long = "install-dir")]
    pub install_dir: Option<PathBuf>,

    /// Print the download/install plan without writing files.
    #[arg(long = "dry-run")]
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct VersionCommand {
    #[command(subcommand)]
    pub command: VersionSubcommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum VersionSubcommand {
    /// Bump the workspace semver version and create a git commit.
    Bump(VersionBumpCommand),
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct VersionBumpCommand {
    /// Semver component to increment.
    pub level: VersionBumpLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum VersionBumpLevel {
    Patch,
    Minor,
    Major,
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
    use clap::Parser;

    use super::{Command, TendrilCli, VersionBumpLevel, VersionSubcommand};

    #[test]
    fn agent_help_includes_workflow_hint() {
        let help = TendrilCli::agent_help();

        assert!(help.contains("Workflow:"));
        assert!(help.contains("tendril list --json"));
        assert!(help.contains("tendril --remote me@box list --json"));
        assert!(help.contains("capture --json"));
        assert!(help.contains("capture -o /tmp/screen.png"));
        assert!(help.contains(" run 'send(\"hello\")'"));
        assert!(help.contains("clipboard get --json"));
        assert!(help.contains("alias --name desk"));
        assert!(help.contains("update"));
        assert!(help.contains("version"));
    }

    #[test]
    fn parses_version_bump_command() {
        let cli = TendrilCli::parse_from(["tendril", "version", "bump", "minor"]);

        let Some(Command::Version(command)) = cli.command else {
            panic!("version command should parse");
        };
        let VersionSubcommand::Bump(command) = command.command;
        assert_eq!(command.level, VersionBumpLevel::Minor);
    }

    #[test]
    fn parses_wsl_tunnel_flag_as_global_proxy_mode() {
        let cli = TendrilCli::parse_from(["tendril", "--wsl-tunnel", "--json", "list"]);

        assert!(cli.wsl_tunnel);
        assert!(cli.json);
        assert!(matches!(cli.command, Some(Command::List(_))));
    }

    #[test]
    fn command_name_maps_each_subcommand_to_its_stable_label() {
        fn name_of(args: &[&str]) -> &'static str {
            TendrilCli::parse_from(args)
                .command
                .expect("a subcommand should parse")
                .name()
        }

        assert_eq!(name_of(&["tendril", "list"]), "list");
        assert_eq!(name_of(&["tendril", "list-elements"]), "list-elements");
        assert_eq!(name_of(&["tendril", "capture"]), "capture");
        assert_eq!(name_of(&["tendril", "run"]), "run");
        assert_eq!(name_of(&["tendril", "listen"]), "listen");
        assert_eq!(name_of(&["tendril", "clipboard", "get"]), "clipboard");
        assert_eq!(name_of(&["tendril", "alias", "--name", "desk"]), "alias");
        assert_eq!(name_of(&["tendril", "update"]), "update");
        assert_eq!(name_of(&["tendril", "version", "bump", "minor"]), "version");
        assert_eq!(name_of(&["tendril", "mcp", "stdio"]), "mcp");
    }
}
