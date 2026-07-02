use std::env;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mcp_cli::{JsonEnvelope, McpServer, StdioServerConfig, ToolRouter};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::info;

use crate::android::{AndroidAppSummary, AndroidDevice, AndroidDeviceSummary};
use crate::capture::{execute_capture, render_capture_human};
use crate::cli::{
    AliasCommand, CaptureCommand, ClipboardCommand, ClipboardGetCommand, ClipboardSetCommand,
    ClipboardSubcommand, Command, ElementListCommand, ListCommand, ListenCommand, McpSubcommand,
    PermissionsCommand, RunCommand, TendrilCli, UpdateCommand, VersionCommand, VersionSubcommand,
    WORKFLOW_HINT,
};
use crate::clipboard::{
    ClipboardGetInput, ClipboardSelection, ClipboardSetInput, DEFAULT_CLIPBOARD_SERVE_MS,
    DEFAULT_CLIPBOARD_TIMEOUT_MS, execute_clipboard_get, execute_clipboard_set,
    render_clipboard_get_human, render_clipboard_set_human,
};
use crate::config::TendrilConfig;
use crate::error::TendrilError;
use crate::execution_lock::{
    ExecutionLockRequest, acquire_execution_lock, default_execution_lock_path,
};
use crate::input::{execute_run, parse_input_definition, render_run_human};
use crate::listen::{
    ListenArtifact, ListenCaptureResult, ListenSkipReason, execute_listen_capture,
};
use crate::model::{
    AliasInput, AliasOutput, AudioFormat, AudioSourceKind, AudioSourceSelector,
    CameraCaptureOutput, CapabilitySet, CaptureInput, ElementListInput, ElementListOutput,
    ListInput, ListOutput, ListenInput, RunInput, RunInputPayload, ShellKind, TargetDescriptor,
    TargetKind, TargetSelector,
};
use crate::platform::{
    AdapterContext, AdapterInfo, AudioCapabilityReport, AudioProbeRequest,
    AudioSourceKind as PlatformAudioSourceKind, Capability, CaptureTargetKind, PermissionKind,
    PermissionRequestOutcome, PermissionState, PermissionStatus, PlatformAdapter, PlatformKind,
    TargetDiscoveryRequest, adapter_for_context, request_macos_permissions,
};
use crate::update::{execute_update, render_update_human, updater_config};
use crate::versioning::{execute_version_bump, render_version_bump_human};

#[derive(Clone)]
struct CommandContext {
    config: TendrilConfig,
    adapter_context: AdapterContext,
    adapter: Option<Arc<dyn PlatformAdapter>>,
}

impl CommandContext {
    fn adapter(&self) -> Arc<dyn PlatformAdapter> {
        self.adapter
            .clone()
            .unwrap_or_else(|| Arc::from(adapter_for_context(self.adapter_context.clone())))
    }

    /// Build an adapter for this call, optionally overriding the X11 display.
    ///
    /// When `x11_display` is a non-empty value, a fresh adapter is built from a
    /// clone of the base context with that display set, so a single long-lived
    /// MCP server can target a display that came up after it spawned
    /// (bd-6abe70). When it is `None`/empty, the base adapter is returned
    /// unchanged (including any test-injected adapter).
    fn adapter_with_x11_display(&self, x11_display: Option<&str>) -> Arc<dyn PlatformAdapter> {
        match x11_display.filter(|value| !value.is_empty()) {
            Some(display) => {
                let context = self
                    .adapter_context
                    .clone()
                    .with_x11_display(Some(display.to_owned()));
                Arc::from(adapter_for_context(context))
            }
            None => self.adapter(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TargetScope {
    pub window: Option<String>,
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    /// Explicit X11 display name (for example `:99`) to connect to for this
    /// call, overriding the Tendril MCP server's ambient `$DISPLAY`. Lets an
    /// MCP client on a headless node target a virtual display (Xvfb) brought up
    /// after the server started (bd-6abe70). Linux/X11 only; ignored on other
    /// backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x11_display: Option<String>,
}

/// MCP request wrapper for the `list` tool: the `list` options plus an optional
/// per-call X11 display override (bd-6abe70). `list` takes no target scope, so
/// it carries `x11_display` directly rather than through `TargetScope`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListRequest {
    #[serde(flatten)]
    pub options: ListCommand,
    /// Explicit X11 display name (for example `:99`) to connect to for this
    /// call, overriding the server's ambient `$DISPLAY`. Linux/X11 only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x11_display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CaptureRequest {
    #[serde(flatten)]
    pub target: TargetScope,
    #[serde(flatten)]
    pub options: CaptureCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunRequest {
    #[serde(flatten)]
    pub target: TargetScope,
    #[serde(flatten)]
    pub options: RunCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ElementListRequest {
    #[serde(flatten)]
    pub target: TargetScope,
    #[serde(flatten)]
    pub options: ElementListCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AliasRequest {
    #[serde(flatten)]
    pub target: TargetScope,
    #[serde(flatten)]
    pub options: AliasCommand,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClipboardGetRequest {
    /// Selection to read: `clipboard` or `primary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// Maximum time in milliseconds to wait for the owner to answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClipboardSetRequest {
    /// Selection to serve: `clipboard` or `primary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// Text to expose through the selection.
    pub text: String,
    /// Time in milliseconds to stay alive serving paste/read requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serve_ms: Option<u64>,
}

/// MCP request wrapper for the `listen` tool.
///
/// Mirrors the CLI surface (`tendril listen --source ... --duration-ms ...
/// --format ... -o <output>`) so MCP clients can capture audio without
/// shelling out. Unlike the CLI's `ListenCommand`, this struct serializes
/// `output` because MCP callers must be able to specify a write path
/// explicitly — there is no shared filesystem context the way there is for
/// a CLI invocation, so omitting it would silently fall back to a
/// platform-temp path the caller cannot predict.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListenRequest {
    /// Audio source selector: `system`, `loopback`, `microphone`, or `device:<id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Requested capture duration in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Requested audio format: `wav`, `flac`, or `opus`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Optional path to write the captured audio artifact to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<std::path::PathBuf>,
}

impl ListenRequest {
    fn to_listen_command(&self) -> ListenCommand {
        ListenCommand {
            source: self.source.clone(),
            duration_ms: self.duration_ms,
            format: self.format.clone(),
            output: self.output.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelpWorkflowStep {
    pub command: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelpCommandSummary {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelpExample {
    pub description: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelpOutput {
    pub help: String,
    pub workflow_hint: String,
    pub workflow_steps: Vec<HelpWorkflowStep>,
    pub commands: Vec<HelpCommandSummary>,
    pub examples: Vec<HelpExample>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListenResponse {
    pub request: ListenInput,
    pub adapter: AdapterInfo,
    pub capability: AudioCapabilityReport,
    pub execution: ListenExecutionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListenExecutionStatus {
    pub status: String,
    pub artifact_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ListenArtifact>,
    pub notes: Vec<String>,
}

#[derive(Debug)]
pub enum CommandOutput {
    Human(String),
    Json(Value),
    Empty,
}

impl CommandOutput {
    pub fn print(self) {
        match self {
            Self::Human(text) => print!("{text}"),
            Self::Json(value) => println!(
                "{}",
                serde_json::to_string_pretty(&value)
                    .expect("command output should always be serializable")
            ),
            Self::Empty => {}
        }
    }
}

pub fn dispatch(cli: &TendrilCli, config: &TendrilConfig) -> Result<CommandOutput, TendrilError> {
    match &cli.command {
        None => Ok(render_help(cli.json)),
        Some(Command::Mcp(command)) => {
            reject_inherited_target_flags_for_mcp(cli)?;
            dispatch_mcp(command, config)
        }
        Some(command) => dispatch_cli_command(cli, command, config),
    }
}

fn reject_inherited_target_flags_for_mcp(cli: &TendrilCli) -> Result<(), TendrilError> {
    let mut offending: Vec<&'static str> = Vec::new();
    if cli.window.is_some() {
        offending.push("--window");
    }
    if cli.display.is_some() {
        offending.push("--display");
    }
    if cli.json {
        offending.push("--json");
    }
    if offending.is_empty() {
        return Ok(());
    }
    Err(TendrilError::validation(format!(
        "the MCP server does not honor top-level CLI flag(s) {}; pass target scope and output formatting in each MCP tool request payload instead",
        offending.join(", ")
    )))
}

fn render_help(json_mode: bool) -> CommandOutput {
    let help = build_help_output();
    if json_mode {
        let envelope = JsonEnvelope::success_for("help", &help);
        CommandOutput::Json(serde_json::to_value(envelope).expect("json help should serialize"))
    } else {
        CommandOutput::Human(help.help)
    }
}

fn dispatch_mcp(
    command: &crate::cli::McpCommand,
    config: &TendrilConfig,
) -> Result<CommandOutput, TendrilError> {
    match &command.command {
        McpSubcommand::Stdio => {
            let server = build_mcp_server();
            let context = CommandContext {
                config: config.clone(),
                adapter_context: AdapterContext::detect(),
                adapter: None,
            };
            server
                .serve_stdio(&context)
                .map_err(|error| TendrilError::serialization(error.to_string()))?;
            Ok(CommandOutput::Empty)
        }
    }
}

fn dispatch_cli_command(
    cli: &TendrilCli,
    command: &Command,
    config: &TendrilConfig,
) -> Result<CommandOutput, TendrilError> {
    if let Some(selection) = android_selection_from_cli(cli) {
        return dispatch_android_cli_command(cli, command, config, &selection);
    }

    match command {
        Command::List(command) => dispatch_list_command(command, cli.json),
        Command::ListElements(command) => dispatch_list_elements_command(cli, command),
        Command::Capture(command) => dispatch_capture_command(cli, command, config),
        Command::Run(command) => dispatch_run_command(cli, command, config),
        Command::Listen(command) => {
            dispatch_listen_command(command, cli.json, &AdapterContext::detect())
        }
        Command::Clipboard(command) => dispatch_clipboard_command(command, cli.json),
        Command::Alias(command) => dispatch_alias_command(cli, command),
        Command::Update(command) => dispatch_update_command(command, cli.json),
        Command::Version(command) => dispatch_version_command(command, cli.json),
        Command::Permissions(command) => Ok(dispatch_permissions_command(command, cli.json)),
        Command::Mcp(_) => unreachable!("MCP commands are dispatched separately"),
    }
}

fn android_selection_from_cli(cli: &TendrilCli) -> Option<String> {
    cli.android
        .clone()
        .or_else(|| std::env::var("TENDRIL_ANDROID_SERIAL").ok())
        .filter(|value| !value.trim().is_empty())
}

fn dispatch_android_cli_command(
    cli: &TendrilCli,
    command: &Command,
    config: &TendrilConfig,
    selection: &str,
) -> Result<CommandOutput, TendrilError> {
    if cli.window.is_some() || cli.display.is_some() {
        return Err(TendrilError::validation(
            "--android selects the Android target; do not combine it with --window or --display",
        )
        .with_code("invalid_android_target_scope"));
    }
    let device = AndroidDevice::resolve(Some(selection))?;
    match command {
        Command::List(command) => {
            let active = device.active_app().ok().flatten();
            let recent = device.recent_apps().unwrap_or_default();
            let launchable = if command.all_apps {
                device.launchable_apps().unwrap_or_default()
            } else {
                Vec::new()
            };
            let target_apps = android_target_apps(active.as_ref(), &recent, &launchable);
            let output = AndroidListOutput {
                device: device.summary(),
                active_app: active,
                recent_apps: recent,
                launchable_apps: launchable,
                targets: device.list_output_with_apps(&target_apps),
            };
            Ok(render_command_output(
                "list",
                cli.json,
                output,
                render_android_list_human,
            ))
        }
        Command::ListElements(command) => {
            let output = device.list_elements_output(command.include_offscreen)?;
            Ok(render_command_output(
                "list-elements",
                cli.json,
                output,
                render_list_elements_human,
            ))
        }
        Command::Capture(command) => {
            let output =
                device.capture_output(command.compression.unwrap_or(config.capture.compression))?;
            if let Some(path) = &command.output {
                write_capture_to_file(&output.image_base64, path)?;
            }
            Ok(render_command_output(
                "capture",
                cli.json,
                output,
                render_capture_human,
            ))
        }
        Command::Run(command) => {
            let input_definition = command.input_definition.as_deref().ok_or_else(|| {
                TendrilError::validation("run requires text or a DSL input definition")
                    .with_code("invalid_run_input")
                    .with_field("input_definition")
            })?;
            let payload = parse_input_definition(input_definition)?;
            let output = device.execute_payload(&payload)?;
            Ok(render_command_output(
                "run",
                cli.json,
                output,
                render_run_human,
            ))
        }
        Command::Listen(_)
        | Command::Clipboard(_)
        | Command::Alias(_)
        | Command::Update(_)
        | Command::Version(_)
        | Command::Permissions(_)
        | Command::Mcp(_) => Err(TendrilError::unsupported_capability(
            "android_command_unsupported",
            format!(
                "--android does not support the `{}` command",
                command.name()
            ),
            Some(json!({ "command": command.name() })),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AndroidListOutput {
    device: AndroidDeviceSummary,
    active_app: Option<AndroidAppSummary>,
    recent_apps: Vec<AndroidAppSummary>,
    launchable_apps: Vec<AndroidAppSummary>,
    targets: ListOutput,
}

fn android_target_apps(
    active: Option<&AndroidAppSummary>,
    recent: &[AndroidAppSummary],
    launchable: &[AndroidAppSummary],
) -> Vec<AndroidAppSummary> {
    let mut apps = Vec::new();
    if let Some(active) = active {
        apps.push(active.clone());
    }
    apps.extend(recent.iter().cloned());
    apps.extend(launchable.iter().cloned());
    let mut seen = std::collections::HashSet::new();
    apps.into_iter()
        .filter(|app| seen.insert(app.package.clone()))
        .collect()
}

fn render_android_list_human(output: &AndroidListOutput) -> String {
    let mut rendered = String::new();
    let _ = writeln!(
        rendered,
        "android device: {} state={} model={} wm_size={} wm_density={} artifacts={}",
        output.device.serial,
        output.device.state,
        output.device.model.as_deref().unwrap_or("<unknown>"),
        output.device.wm_size.as_deref().unwrap_or("<unknown>"),
        output.device.wm_density.as_deref().unwrap_or("<unknown>"),
        output.device.artifact_dir.display(),
    );
    if let Some(focus) = &output.device.focused_window {
        let _ = writeln!(rendered, "focused: {focus}");
    }
    if let Some(app) = &output.active_app {
        let _ = writeln!(rendered, "active app: {}", app.package);
    }
    let _ = writeln!(
        rendered,
        "recent apps: {} launchable apps listed: {}",
        output.recent_apps.len(),
        output.launchable_apps.len()
    );
    rendered.push_str(&render_list_human(&output.targets));
    rendered
}

fn dispatch_list_command(
    command: &ListCommand,
    json_mode: bool,
) -> Result<CommandOutput, TendrilError> {
    let input = validate_list_command(command)?;
    let output = execute_list(&input, &AdapterContext::detect())?;
    info!(
        command = "list",
        target_count = output.targets.len(),
        "discovered desktop targets"
    );
    Ok(render_command_output(
        "list",
        json_mode,
        output,
        render_list_human,
    ))
}

fn dispatch_list_elements_command(
    cli: &TendrilCli,
    command: &ElementListCommand,
) -> Result<CommandOutput, TendrilError> {
    let input = build_element_list_input(&target_scope_from_cli(cli), command)?;
    let adapter = adapter_for_context(AdapterContext::detect());
    let output = execute_list_elements(&input, adapter.as_ref())?;
    Ok(render_command_output(
        "list-elements",
        cli.json,
        output,
        render_list_elements_human,
    ))
}

fn dispatch_capture_command(
    cli: &TendrilCli,
    command: &CaptureCommand,
    config: &TendrilConfig,
) -> Result<CommandOutput, TendrilError> {
    let target = target_scope_from_cli(cli);
    if let Some(device) = target.camera.clone() {
        ensure_camera_target_exclusive(&target)?;
        return dispatch_camera_capture(&device, command, config, cli.json);
    }
    let input = build_capture_input(&target, command, config)?;
    info!(
        command = "capture",
        target_kind = ?input.target.kind(),
        target_id = %input.target.id(),
        format = ?input.format,
        "validated capture request"
    );
    let adapter = adapter_for_context(AdapterContext::detect());
    let output = execute_capture(&input, adapter.as_ref())?;
    if let Some(path) = &command.output {
        write_capture_to_file(&output.image_base64, path)?;
    }
    Ok(render_command_output(
        "capture",
        cli.json,
        output,
        render_capture_human,
    ))
}

/// `--camera` selects a single capture device and is mutually exclusive with
/// the window/display selectors.
fn ensure_camera_target_exclusive(target: &TargetScope) -> Result<(), TendrilError> {
    if target.window.is_some() || target.display.is_some() {
        return Err(TendrilError::validation(
            "choose either `--camera` or a window/display target, but not both",
        )
        .with_code("invalid_target_selector")
        .with_field("target"));
    }
    Ok(())
}

/// Grab one frame from `device`, apply the shared capture post-processing
/// (resize/format/compression), and build the structured camera capture output
/// (returning the processed image bytes too, for an optional `--output` file
/// write).
fn build_camera_capture_output(
    device: &str,
    adapter_info: AdapterInfo,
    command: &CaptureCommand,
    config: &TendrilConfig,
) -> Result<(CameraCaptureOutput, Vec<u8>), TendrilError> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let raw = crate::camera::capture_camera_frame(device)?;
    let max_width = command.max_width.or(config.capture.max_width);
    let max_height = command.max_height.or(config.capture.max_height);
    let format = command
        .format
        .as_deref()
        .map(parse_image_format)
        .transpose()?
        .unwrap_or(config.capture.format);
    let compression = command.compression.unwrap_or(config.capture.compression);
    let processed =
        crate::capture::process_raw_image(&raw, max_width, max_height, format, compression)?;
    let output = CameraCaptureOutput {
        adapter: adapter_info,
        device: device.to_owned(),
        format,
        media_type: processed.media_type,
        width: processed.width,
        height: processed.height,
        image_base64: BASE64.encode(&processed.bytes),
        captured_at: crate::capture::current_timestamp(),
    };
    Ok((output, processed.bytes))
}

fn dispatch_camera_capture(
    device: &str,
    command: &CaptureCommand,
    config: &TendrilConfig,
    json_mode: bool,
) -> Result<CommandOutput, TendrilError> {
    let adapter = adapter_for_context(AdapterContext::detect());
    let (output, bytes) = build_camera_capture_output(device, adapter.info(), command, config)?;
    info!(
        command = "capture",
        target_kind = "camera",
        target_id = %device,
        "captured camera frame"
    );
    if let Some(path) = &command.output {
        std::fs::write(path, &bytes).map_err(|error| {
            TendrilError::execution_failure(
                "camera_capture_write_failed",
                format!(
                    "failed to write camera frame to {}: {error}",
                    path.display()
                ),
                None,
            )
        })?;
    }
    Ok(render_command_output(
        "capture",
        json_mode,
        output,
        render_camera_capture_human,
    ))
}

fn render_camera_capture_human(output: &CameraCaptureOutput) -> String {
    format!(
        "camera capture: {}\nplatform: {:?} / {:?}\nsize: {}x{}\nformat: {:?}\nmedia_type: {}\nimage_base64_bytes: {}\ncaptured_at: {}\n",
        output.device,
        output.adapter.platform,
        output.adapter.session,
        output.width,
        output.height,
        output.format,
        output.media_type,
        output.image_base64.len(),
        output.captured_at,
    )
}

fn dispatch_run_command(
    cli: &TendrilCli,
    command: &RunCommand,
    config: &TendrilConfig,
) -> Result<CommandOutput, TendrilError> {
    let input = build_run_input(&target_scope_from_cli(cli), command)?;
    info!(
        command = "run",
        target_kind = ?input.target.kind(),
        target_id = %input.target.id(),
        payload_kind = %payload_kind(&input.payload),
        "validated run request with redacted payload"
    );
    let lock_request = build_execution_lock_request(command, &input, config)?;
    let lock_permit = acquire_execution_lock(&lock_request)?;
    let lock_report = lock_permit.report().clone();
    let adapter = adapter_for_context(AdapterContext::detect());
    let mut output = execute_run(&input, adapter.as_ref())
        .map_err(|error| error.with_detail_entry("execution_lock", json!(lock_report)))?;
    output.execution_lock = Some(lock_permit.report().clone());
    Ok(render_command_output(
        "run",
        cli.json,
        output,
        render_run_human,
    ))
}

fn dispatch_alias_command(
    cli: &TendrilCli,
    command: &AliasCommand,
) -> Result<CommandOutput, TendrilError> {
    let input = build_alias_input(&target_scope_from_cli(cli), command)?;
    info!(
        command = "alias",
        target_kind = ?input.target.kind(),
        target_id = %input.target.id(),
        shell = ?input.shell,
        alias_name = %input.name,
        "validated alias request"
    );
    let output = execute_alias(&input);
    Ok(render_command_output(
        "alias",
        cli.json,
        output,
        render_alias_human,
    ))
}

fn dispatch_update_command(
    command: &UpdateCommand,
    json_mode: bool,
) -> Result<CommandOutput, TendrilError> {
    let output = execute_update(command)?;
    Ok(render_command_output(
        "update",
        json_mode,
        output,
        render_update_human,
    ))
}

fn dispatch_version_command(
    command: &VersionCommand,
    json_mode: bool,
) -> Result<CommandOutput, TendrilError> {
    match &command.command {
        VersionSubcommand::Bump(command) => {
            let output = execute_version_bump(command.level)?;
            Ok(render_command_output(
                "version bump",
                json_mode,
                output,
                render_version_bump_human,
            ))
        }
    }
}

fn build_mcp_server() -> McpServer<CommandContext> {
    McpServer::new(
        StdioServerConfig {
            server_name: "tendril".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        build_tool_router(),
    )
}

#[allow(clippy::too_many_lines)]
fn build_tool_router() -> ToolRouter<CommandContext> {
    let mut router = ToolRouter::new();
    router.add_typed_tool(
        "list",
        "Discover available desktop targets.",
        |context: &CommandContext, command: ListRequest| {
            let input = validate_list_command(&command.options)?;
            let adapter = context.adapter_with_x11_display(command.x11_display.as_deref());
            execute_list_with_adapter(&input, adapter.as_ref())
        },
    );
    router.add_typed_tool(
        "list_elements",
        "Discover lower-level UI elements for a display/window target or globally.",
        |context: &CommandContext, command: ElementListRequest| {
            let input = build_element_list_input(&command.target, &command.options)?;
            let adapter = context.adapter_with_x11_display(command.target.x11_display.as_deref());
            serde_json::to_value(execute_list_elements(&input, adapter.as_ref())?)
                .map_err(|error| TendrilError::serialization(error.to_string()))
        },
    );
    router.add_typed_tool(
        "capture",
        "Capture a screenshot from a display or window target.",
        |context: &CommandContext, command: CaptureRequest| {
            if let Some(device) = command.target.camera.as_deref() {
                ensure_camera_target_exclusive(&command.target)?;
                let adapter = context.adapter();
                let (output, _bytes) = build_camera_capture_output(
                    device,
                    adapter.info(),
                    &command.options,
                    &context.config,
                )?;
                return serde_json::to_value(output)
                    .map_err(|error| TendrilError::serialization(error.to_string()));
            }
            let input = build_capture_input(&command.target, &command.options, &context.config)?;
            let adapter = context.adapter_with_x11_display(command.target.x11_display.as_deref());
            capture_response_value(&input, adapter.as_ref())
        },
    );
    router.add_typed_tool(
        "run",
        "Execute input against a specific target.",
        |context: &CommandContext, command: RunRequest| {
            let input = build_run_input(&command.target, &command.options)?;
            let lock_request =
                build_execution_lock_request(&command.options, &input, &context.config)?;
            let lock_permit = acquire_execution_lock(&lock_request)?;
            let lock_report = lock_permit.report().clone();
            let adapter = context.adapter_with_x11_display(command.target.x11_display.as_deref());
            let mut output = execute_run(&input, adapter.as_ref())
                .map_err(|error| error.with_detail_entry("execution_lock", json!(lock_report)))?;
            output.execution_lock = Some(lock_permit.report().clone());
            serde_json::to_value(output)
                .map_err(|error| TendrilError::serialization(error.to_string()))
        },
    );
    router.add_typed_tool(
        "listen",
        "Capture audio from the system loopback or microphone (probe-only on unwired platforms).",
        |context: &CommandContext, command: ListenRequest| {
            let cli_command = command.to_listen_command();
            let input = build_listen_input(&cli_command)?;
            let response = build_listen_response(
                &input,
                cli_command.output.as_deref(),
                &context.adapter_context,
            )?;
            serde_json::to_value(response)
                .map_err(|error| TendrilError::serialization(error.to_string()))
        },
    );
    router.add_typed_tool(
        "clipboard_get",
        "Read text from the Linux/X11 clipboard or primary selection.",
        |_context: &CommandContext, command: ClipboardGetRequest| {
            let input = build_clipboard_get_input(&ClipboardGetCommand {
                selection: command.selection,
                timeout_ms: command.timeout_ms,
            })?;
            serde_json::to_value(execute_clipboard_get(&input)?)
                .map_err(|error| TendrilError::serialization(error.to_string()))
        },
    );
    router.add_typed_tool(
        "clipboard_set",
        "Own and serve text through the Linux/X11 clipboard or primary selection.",
        |_context: &CommandContext, command: ClipboardSetRequest| {
            let input = build_clipboard_set_input(&ClipboardSetCommand {
                selection: command.selection,
                text: command.text,
                serve_ms: command.serve_ms,
            })?;
            serde_json::to_value(execute_clipboard_set(&input)?)
                .map_err(|error| TendrilError::serialization(error.to_string()))
        },
    );
    router.add_typed_tool(
        "permissions",
        "Report Screen Recording, Accessibility, and Microphone permission status for the host platform.",
        |context: &CommandContext, _command: PermissionsCommand| {
            let adapter = context.adapter();
            let report = PermissionsReport {
                adapter: adapter.info(),
                permissions: adapter.permissions(),
            };
            serde_json::to_value(report)
                .map_err(|error| TendrilError::serialization(error.to_string()))
        },
    );
    updatable_cli::register_update_tool(&mut router, |_context: &CommandContext| updater_config());
    feedback_cli::register_feedback_tools(&mut router, |context: &CommandContext| {
        crate::feedback::feedback_config(context.config.feedback.as_ref())
    });
    router
}

fn validate_list_command(_command: &ListCommand) -> Result<ListInput, TendrilError> {
    let input = ListInput::default();
    input
        .validate()
        .map_err(|error| error.with_code("invalid_list_input"))?;
    Ok(input)
}

fn execute_list(
    input: &ListInput,
    adapter_context: &AdapterContext,
) -> Result<ListOutput, TendrilError> {
    let adapter = adapter_for_context(adapter_context.clone());
    execute_list_with_adapter(input, adapter.as_ref())
}

fn execute_list_with_adapter(
    input: &ListInput,
    adapter: &dyn PlatformAdapter,
) -> Result<ListOutput, TendrilError> {
    let inventory = adapter.discover_targets(&TargetDiscoveryRequest)?;

    let targets = inventory
        .targets
        .into_iter()
        .filter(|target| match target.kind {
            CaptureTargetKind::Window => input.include_windows,
            CaptureTargetKind::Display => input.include_displays,
        })
        .map(model_target_from_platform)
        .collect();

    let cameras = if input.include_cameras {
        adapter.cameras()
    } else {
        Vec::new()
    };

    Ok(ListOutput {
        adapter: adapter.info(),
        permissions: adapter.permissions(),
        targets,
        cameras,
    })
}

fn execute_list_elements(
    input: &ElementListInput,
    adapter: &dyn PlatformAdapter,
) -> Result<ElementListOutput, TendrilError> {
    adapter.list_elements(input)
}

fn model_target_from_platform(target: crate::platform::TargetDescriptor) -> TargetDescriptor {
    TargetDescriptor {
        id: target.id,
        kind: match target.kind {
            CaptureTargetKind::Window => TargetKind::Window,
            CaptureTargetKind::Display => TargetKind::Display,
        },
        name: target.name,
        title: target.title,
        bounds: target.bounds,
        scale_factor: target.scale_factor,
        capabilities: CapabilitySet {
            capture: target.capture_supported,
            input: target.input_supported,
            audio: false,
        },
        diagnostics: target.diagnostics,
        app_name: target.app_name,
        process_id: target.process_id,
    }
}

#[derive(Debug, Clone, Serialize)]
struct PermissionsReport {
    adapter: AdapterInfo,
    permissions: Vec<PermissionStatus>,
}

#[derive(Debug, Clone, Serialize)]
struct PermissionRequestReport {
    adapter: AdapterInfo,
    requested: Vec<PermissionRequestOutcome>,
    permissions: Vec<PermissionStatus>,
}

fn dispatch_permissions_command(command: &PermissionsCommand, json_mode: bool) -> CommandOutput {
    let adapter = adapter_for_context(AdapterContext::detect());
    let info = adapter.info();
    if command.request {
        let requested = if info.platform == PlatformKind::MacOs {
            request_macos_permissions()
        } else {
            unsupported_permission_request(info.platform)
        };
        let report = PermissionRequestReport {
            adapter: info,
            requested,
            permissions: adapter.permissions(),
        };
        return render_command_output(
            "permissions",
            json_mode,
            report,
            render_permission_request_human,
        );
    }
    let report = PermissionsReport {
        adapter: info,
        permissions: adapter.permissions(),
    };
    render_command_output("permissions", json_mode, report, render_permissions_human)
}

/// Fallback outcome for `--request` on platforms where a programmatic request
/// flow is not implemented. Reports that no prompt was fired rather than
/// silently doing nothing (bd-28c0f6).
fn unsupported_permission_request(platform: PlatformKind) -> Vec<PermissionRequestOutcome> {
    vec![PermissionRequestOutcome {
        permission: PermissionKind::ScreenCapture,
        actions: vec![format!(
            "--request is only implemented on macOS; no prompt was fired on {platform:?}"
        )],
        state_after: PermissionState::Unknown,
        note: Some(
            "On this platform, grant capture/input permissions through the OS settings surface reported by `tendril permissions`."
                .to_owned(),
        ),
    }]
}

fn permission_kind_label(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::ScreenCapture => "Screen Recording",
        PermissionKind::Accessibility => "Accessibility (input control)",
        PermissionKind::Microphone => "Microphone",
        PermissionKind::Camera => "Camera",
    }
}

fn permission_state_label(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Granted => "granted",
        PermissionState::NotRequired => "not required",
        PermissionState::Unknown => "unknown",
        PermissionState::Denied => "denied",
    }
}

fn render_permissions_human(report: &PermissionsReport) -> String {
    let mut rendered = String::new();
    let _ = writeln!(
        rendered,
        "platform: {:?} ({:?} session)",
        report.adapter.platform, report.adapter.session
    );
    if report.permissions.is_empty() {
        let _ = writeln!(rendered, "no platform permissions are required");
        return rendered;
    }
    for status in &report.permissions {
        let _ = writeln!(
            rendered,
            "- {}: {}",
            permission_kind_label(status.permission),
            permission_state_label(status.state)
        );
        if !status.summary.is_empty() {
            let _ = writeln!(rendered, "    {}", status.summary);
        }
        if let Some(action) = &status.suggested_action {
            let _ = writeln!(rendered, "    -> {action}");
        }
    }
    rendered
}

fn render_permission_request_human(report: &PermissionRequestReport) -> String {
    let mut rendered = String::new();
    let _ = writeln!(
        rendered,
        "platform: {:?} ({:?} session) - permission request flow",
        report.adapter.platform, report.adapter.session
    );
    for outcome in &report.requested {
        let _ = writeln!(
            rendered,
            "- {}: {} (after request)",
            permission_kind_label(outcome.permission),
            permission_state_label(outcome.state_after)
        );
        for action in &outcome.actions {
            let _ = writeln!(rendered, "    * {action}");
        }
        if let Some(note) = &outcome.note {
            let _ = writeln!(rendered, "    note: {note}");
        }
    }
    let _ = writeln!(rendered, "\ncurrent status:");
    if report.permissions.is_empty() {
        let _ = writeln!(rendered, "  no platform permissions are required");
    }
    for status in &report.permissions {
        let _ = writeln!(
            rendered,
            "  - {}: {}",
            permission_kind_label(status.permission),
            permission_state_label(status.state)
        );
    }
    rendered
}

fn render_command_output<T, Render>(
    command_name: &str,
    json_mode: bool,
    data: T,
    render_human: Render,
) -> CommandOutput
where
    T: Serialize,
    Render: FnOnce(&T) -> String,
{
    if json_mode {
        let envelope = JsonEnvelope::success_for(command_name, &data);
        CommandOutput::Json(serde_json::to_value(envelope).expect("command json should serialize"))
    } else {
        CommandOutput::Human(render_human(&data))
    }
}

#[allow(clippy::too_many_lines)]
fn build_help_output() -> HelpOutput {
    HelpOutput {
        help: TendrilCli::agent_help(),
        workflow_hint: WORKFLOW_HINT.to_owned(),
        workflow_steps: vec![
            HelpWorkflowStep {
                command: "tendril list --json".to_owned(),
                description: "Discover actionable window and display targets.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril --remote me@box list --json".to_owned(),
                description: "Discover windows/displays on a remote desktop over SSH with session auto-detection.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril --window <id> list-elements --json".to_owned(),
                description: "Discover clickable UI elements without choosing screenshot pixels.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril --window <id> capture --json".to_owned(),
                description: "Capture target state and keep resize metadata in JSON.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril --window <id> capture -o /tmp/screen.png".to_owned(),
                description: "Save the captured image directly to a file.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril --window <id> run 'send(\"hello\")'".to_owned(),
                description: "Execute text or input sequences against the chosen target.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril clipboard get --json".to_owned(),
                description: "Read text copied by a browser or app from the Linux/X11 clipboard selection.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril --wsl-tunnel list --json".to_owned(),
                description: "Discover Windows-host targets from WSL by proxying to the Windows tendril.exe.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril --android <serial> list --json".to_owned(),
                description: "Discover Android device or emulator targets through adb.".to_owned(),
            },
        ],
        commands: vec![
            HelpCommandSummary {
                name: "list".to_owned(),
                description: "Discover windows and displays.".to_owned(),
            },
            HelpCommandSummary {
                name: "list-elements".to_owned(),
                description: "Discover lower-level UI elements for target-aware control.".to_owned(),
            },
            HelpCommandSummary {
                name: "capture".to_owned(),
                description: "Capture a screenshot from a window or display target.".to_owned(),
            },
            HelpCommandSummary {
                name: "run".to_owned(),
                description: "Execute text or input sequences against a target.".to_owned(),
            },
            HelpCommandSummary {
                name: "clipboard".to_owned(),
                description: "Read or serve Linux/X11 text selections for deterministic browser↔OS clipboard transfer.".to_owned(),
            },
            HelpCommandSummary {
                name: "alias".to_owned(),
                description: "Emit a shell helper that pre-fills --window or --display.".to_owned(),
            },
            HelpCommandSummary {
                name: "listen".to_owned(),
                description: "Probe supported audio capture paths.".to_owned(),
            },
            HelpCommandSummary {
                name: "update".to_owned(),
                description: "Download and install a Tendril release binary.".to_owned(),
            },
            HelpCommandSummary {
                name: "version".to_owned(),
                description: "Inspect or bump the workspace release version.".to_owned(),
            },
            HelpCommandSummary {
                name: "mcp stdio".to_owned(),
                description: "Serve Tendril over MCP stdio.".to_owned(),
            },
        ],
        examples: vec![
            HelpExample {
                description: "Inspect targets".to_owned(),
                command: "tendril list --json".to_owned(),
            },
            HelpExample {
                description: "Inspect targets on a remote host over SSH".to_owned(),
                command: "tendril --remote me@box list --json".to_owned(),
            },
            HelpExample {
                description: "Inspect target elements".to_owned(),
                command: "tendril --window <id> list-elements --json".to_owned(),
            },
            HelpExample {
                description: "Click a discovered element".to_owned(),
                command: "tendril --window <id> run 'click(33)'".to_owned(),
            },
            HelpExample {
                description: "Capture a chosen target".to_owned(),
                command: "tendril --window <id> capture --json".to_owned(),
            },
            HelpExample {
                description: "Save a capture directly to a file".to_owned(),
                command: "tendril --display <id> capture -o /tmp/screen.png".to_owned(),
            },
            HelpExample {
                description: "Capture to file and get JSON metadata".to_owned(),
                command: "tendril --window <id> capture --json -o /tmp/screen.png".to_owned(),
            },
            HelpExample {
                description: "Read text copied from a browser through the X11 clipboard".to_owned(),
                command: "tendril clipboard get --json".to_owned(),
            },
            HelpExample {
                description: "Serve text for another X11 app to paste".to_owned(),
                command: "tendril clipboard set --text 'hello from OS' --serve-ms 8000".to_owned(),
            },
            HelpExample {
                description: "Create a reusable wrapper for repeated targeting".to_owned(),
                command: "eval \"$(tendril --window <id> alias --name desk)\"".to_owned(),
            },
        ],
        notes: vec![
            "Use --json for machine-readable success and error envelopes.".to_owned(),
            "Use --remote user@host to proxy the invocation over SSH; Linux remotes bootstrap DISPLAY/WAYLAND_DISPLAY/XDG_RUNTIME_DIR when an SSH login did not inherit the graphical session.".to_owned(),
            "Use -o/--output on capture to save the decoded image directly to a file; combine with --json to also get the JSON envelope.".to_owned(),
            "Alias helpers are plain shell wrappers around explicit tendril arguments; Tendril does not store session state.".to_owned(),
            "Element ids are snapshot-local and should be refreshed with list-elements when the UI changes.".to_owned(),
            "On Linux/X11, clipboard selections are owned by a live process; `clipboard set` intentionally stays alive for --serve-ms so browser/terminal paste requests can complete deterministically.".to_owned(),
        ],
    }
}

fn render_list_elements_human(output: &ElementListOutput) -> String {
    let mut rendered = format!(
        "platform: {:?} / {:?}\nelements: {}\n",
        output.adapter.platform,
        output.adapter.session,
        output.elements.len()
    );
    if !output.notes.is_empty() {
        let _ = writeln!(rendered, "notes: {}", output.notes.join(" "));
    }
    for element in &output.elements {
        let bounds = element.bounds.as_ref().map_or_else(
            || "bounds=<none>".to_owned(),
            |bounds| {
                format!(
                    "{}x{}+{}+{}",
                    bounds.width, bounds.height, bounds.x, bounds.y
                )
            },
        );
        let path = if element.path.is_empty() {
            element.name.clone()
        } else {
            element.path.join("/")
        };
        let _ = writeln!(
            rendered,
            "- {}: {}<{}> {} actions={}",
            element.id,
            element.role,
            path,
            bounds,
            if element.actions.is_empty() {
                "none".to_owned()
            } else {
                element.actions.join(",")
            }
        );
    }
    rendered
}

fn render_list_human(output: &ListOutput) -> String {
    let mut rendered = format!(
        "platform: {:?} / {:?}\npermissions: {}\ntargets:\n",
        output.adapter.platform,
        output.adapter.session,
        output.permissions.len()
    );

    for target in &output.targets {
        let capability_summary = format!(
            "capture={}, input={}",
            target.capabilities.capture, target.capabilities.input
        );
        let title_suffix = target
            .title
            .as_deref()
            .map(|title| format!(" title={title:?}"))
            .unwrap_or_default();
        let app_suffix = target
            .app_name
            .as_deref()
            .map(|app_name| format!(" app={app_name}"))
            .unwrap_or_default();
        let diagnostic_suffix = target
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.capability == crate::platform::Capability::DisplayCapture
                    || diagnostic.capability == crate::platform::Capability::WindowCapture
            })
            .map(|diagnostic| format!(" diagnostic={}: {}", diagnostic.code, diagnostic.message))
            .unwrap_or_default();
        let _ = writeln!(
            rendered,
            "- {:?} {} {} {}x{}+{}+{} scale={}/{} {}{}{}{}",
            target.kind,
            target.id,
            target.name,
            target.bounds.width,
            target.bounds.height,
            target.bounds.x,
            target.bounds.y,
            target.scale_factor.numerator,
            target.scale_factor.denominator,
            capability_summary,
            title_suffix,
            app_suffix,
            diagnostic_suffix,
        );
    }

    if !output.cameras.is_empty() {
        let _ = writeln!(rendered, "cameras:");
        for camera in &output.cameras {
            let model_suffix = camera
                .model_id
                .as_deref()
                .map(|model| format!(" model={model}"))
                .unwrap_or_default();
            let unique_suffix = camera
                .unique_id
                .as_deref()
                .map(|unique| format!(" unique_id={unique}"))
                .unwrap_or_default();
            let _ = writeln!(
                rendered,
                "- {} name={}{}{}",
                camera.id, camera.name, model_suffix, unique_suffix
            );
        }
    }

    rendered
}

fn build_element_list_input(
    target: &TargetScope,
    command: &ElementListCommand,
) -> Result<ElementListInput, TendrilError> {
    let input = ElementListInput {
        target: optional_target(target)?,
        include_offscreen: command.include_offscreen,
    };
    input.validate()?;
    Ok(input)
}

fn build_capture_input(
    target: &TargetScope,
    command: &CaptureCommand,
    config: &TendrilConfig,
) -> Result<CaptureInput, TendrilError> {
    let input = CaptureInput {
        target: required_target(target, "capture")?,
        max_width: command.max_width.or(config.capture.max_width),
        max_height: command.max_height.or(config.capture.max_height),
        format: command
            .format
            .as_deref()
            .map(parse_image_format)
            .transpose()?
            .unwrap_or(config.capture.format),
        compression: command.compression.unwrap_or(config.capture.compression),
        timeout_ms: command.timeout_ms.or(config.capture.timeout_ms),
    };
    input.validate()?;
    Ok(input)
}

fn write_capture_to_file(image_base64: &str, path: &Path) -> Result<(), TendrilError> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let bytes = BASE64.decode(image_base64).map_err(|error| {
        TendrilError::execution_failure(
            "capture_decode_failed",
            format!("failed to decode base64 image for --output: {error}"),
            None,
        )
    })?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                TendrilError::execution_failure(
                    "capture_write_failed",
                    format!(
                        "failed to create parent directory `{}`: {error}",
                        parent.display()
                    ),
                    None,
                )
            })?;
        }
    }
    std::fs::write(path, &bytes).map_err(|error| {
        TendrilError::execution_failure(
            "capture_write_failed",
            format!("failed to write capture to `{}`: {error}", path.display()),
            None,
        )
    })?;
    Ok(())
}

fn capture_response_value(
    input: &CaptureInput,
    adapter: &dyn PlatformAdapter,
) -> Result<Value, TendrilError> {
    serde_json::to_value(execute_capture(input, adapter)?)
        .map_err(|error| TendrilError::serialization(error.to_string()))
}

fn dispatch_clipboard_command(
    command: &ClipboardCommand,
    json_mode: bool,
) -> Result<CommandOutput, TendrilError> {
    match &command.command {
        ClipboardSubcommand::Get(command) => {
            let input = build_clipboard_get_input(command)?;
            let output = execute_clipboard_get(&input)?;
            Ok(render_command_output(
                "clipboard",
                json_mode,
                output,
                render_clipboard_get_human,
            ))
        }
        ClipboardSubcommand::Set(command) => {
            let input = build_clipboard_set_input(command)?;
            let output = execute_clipboard_set(&input)?;
            Ok(render_command_output(
                "clipboard",
                json_mode,
                output,
                render_clipboard_set_human,
            ))
        }
    }
}

fn build_clipboard_get_input(
    command: &ClipboardGetCommand,
) -> Result<ClipboardGetInput, TendrilError> {
    let input = ClipboardGetInput {
        selection: ClipboardSelection::parse(command.selection.as_deref())?,
        timeout_ms: command.timeout_ms.unwrap_or(DEFAULT_CLIPBOARD_TIMEOUT_MS),
    };
    input.validate()?;
    Ok(input)
}

fn build_clipboard_set_input(
    command: &ClipboardSetCommand,
) -> Result<ClipboardSetInput, TendrilError> {
    let input = ClipboardSetInput {
        selection: ClipboardSelection::parse(command.selection.as_deref())?,
        text: command.text.clone(),
        serve_ms: command.serve_ms.unwrap_or(DEFAULT_CLIPBOARD_SERVE_MS),
    };
    input.validate()?;
    Ok(input)
}

fn build_run_input(target: &TargetScope, command: &RunCommand) -> Result<RunInput, TendrilError> {
    let input_definition = command.input_definition.clone().ok_or_else(|| {
        TendrilError::validation("run requires an input definition")
            .with_code("invalid_run_input")
            .with_field("input_definition")
    })?;
    let input = RunInput {
        target: required_target(target, "run")?,
        payload: parse_input_definition(&input_definition)?,
        restore_focus: command.restore_focus && !command.no_restore_focus,
    };
    input.validate()?;
    Ok(input)
}

fn build_execution_lock_request(
    command: &RunCommand,
    input: &RunInput,
    config: &TendrilConfig,
) -> Result<ExecutionLockRequest, TendrilError> {
    let env_no_lock = parse_bool_env("TENDRIL_NO_LOCK")?;
    let disabled_by_cli = command.no_lock;
    let disabled_by_env = env_no_lock.unwrap_or(false);
    let enabled = config.execution_lock.enabled && !disabled_by_cli && !disabled_by_env;
    let reason = if disabled_by_cli {
        Some("--no-lock".to_owned())
    } else if disabled_by_env {
        Some("TENDRIL_NO_LOCK".to_owned())
    } else if !config.execution_lock.enabled {
        Some("config.execution_lock.enabled=false".to_owned())
    } else {
        None
    };

    Ok(ExecutionLockRequest {
        enabled,
        lock_path: command
            .lock_path
            .clone()
            .or_else(|| env::var_os("TENDRIL_LOCK_PATH").map(PathBuf::from))
            .or_else(|| config.execution_lock.path.clone())
            .unwrap_or_else(default_execution_lock_path),
        timeout_ms: command
            .lock_timeout_ms
            .or(parse_u64_env("TENDRIL_LOCK_TIMEOUT_MS")?)
            .unwrap_or(config.execution_lock.timeout_ms),
        stale_ms: command
            .lock_stale_ms
            .or(parse_u64_env("TENDRIL_LOCK_STALE_MS")?)
            .unwrap_or(config.execution_lock.stale_ms),
        command: "run".to_owned(),
        target_kind: Some(format!("{:?}", input.target.kind()).to_lowercase()),
        target_id: Some(input.target.id().to_owned()),
        reason,
    })
}

fn parse_u64_env(name: &'static str) -> Result<Option<u64>, TendrilError> {
    env::var(name)
        .ok()
        .map(|value| {
            value.parse::<u64>().map_err(|error| {
                TendrilError::validation(format!(
                    "environment variable {name} must be an unsigned integer: {error}"
                ))
                .with_code("invalid_run_input")
                .with_field(name)
            })
        })
        .transpose()
}

fn parse_bool_env(name: &'static str) -> Result<Option<bool>, TendrilError> {
    env::var(name)
        .ok()
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(TendrilError::validation(format!(
                "environment variable {name} must be a boolean (true/false or 1/0)"
            ))
            .with_code("invalid_run_input")
            .with_field(name)),
        })
        .transpose()
}

fn build_listen_input(command: &ListenCommand) -> Result<ListenInput, TendrilError> {
    let defaults = ListenInput::default();
    let input = ListenInput {
        source: match command.source.as_deref() {
            None => AudioSourceSelector {
                kind: AudioSourceKind::System,
                id: None,
            },
            Some(value)
                if value.eq_ignore_ascii_case("system")
                    || value.eq_ignore_ascii_case("loopback") =>
            {
                AudioSourceSelector {
                    kind: AudioSourceKind::System,
                    id: None,
                }
            }
            Some(value)
                if value.eq_ignore_ascii_case("microphone")
                    || value.eq_ignore_ascii_case("mic") =>
            {
                AudioSourceSelector {
                    kind: AudioSourceKind::Microphone,
                    id: None,
                }
            }
            Some(value) => {
                if let Some((prefix, id)) = value.split_once(':') {
                    if prefix.eq_ignore_ascii_case("device") {
                        AudioSourceSelector {
                            kind: AudioSourceKind::Device,
                            id: Some(id.to_owned()),
                        }
                    } else {
                        return Err(TendrilError::validation(format!(
                            "unsupported audio source `{value}`; use `system`, `loopback`, `microphone`, or `device:<id>`"
                        ))
                        .with_code("invalid_listen_input")
                        .with_field("source"));
                    }
                } else {
                    return Err(TendrilError::validation(format!(
                        "unsupported audio source `{value}`; use `system`, `loopback`, `microphone`, or `device:<id>`"
                    ))
                    .with_code("invalid_listen_input")
                    .with_field("source"));
                }
            }
        },
        duration_ms: command.duration_ms.unwrap_or(defaults.duration_ms),
        format: command
            .format
            .as_deref()
            .map(parse_audio_format)
            .transpose()?
            .unwrap_or(defaults.format),
    };
    input.validate()?;
    Ok(input)
}

fn dispatch_listen_command(
    command: &ListenCommand,
    json_mode: bool,
    adapter_context: &AdapterContext,
) -> Result<CommandOutput, TendrilError> {
    let input = build_listen_input(command)?;
    info!(
        command = "listen",
        source_kind = ?input.source.kind,
        source_id = input.source.id.as_deref().unwrap_or(""),
        duration_ms = input.duration_ms,
        format = ?input.format,
        "validated listen request"
    );
    let response = build_listen_response(&input, command.output.as_deref(), adapter_context)?;
    Ok(render_listen_output(&response, json_mode))
}

fn build_listen_response(
    input: &ListenInput,
    output: Option<&Path>,
    adapter_context: &AdapterContext,
) -> Result<ListenResponse, TendrilError> {
    let adapter = adapter_for_context(adapter_context.clone());
    let adapter_info = adapter.info();
    let capability = probe_listen_capability(input, adapter.as_ref())?;

    // Attempt a real recording. Recorder-level failures or unsupported
    // platforms degrade to the legacy probe-only response so callers always
    // get the diagnostic envelope they relied on in v0.0.1.
    let capture_result = execute_listen_capture(
        input,
        output,
        adapter_info.platform,
        adapter_info.audio_backend,
    )?;

    let execution = match capture_result {
        ListenCaptureResult::Captured {
            artifact,
            mut notes,
        } => {
            notes.push(
                "listen captured a real audio artifact; the JSON envelope reports its on-disk path."
                    .to_owned(),
            );
            ListenExecutionStatus {
                status: "captured".to_owned(),
                artifact_available: true,
                artifact: Some(artifact),
                notes,
            }
        }
        ListenCaptureResult::Skipped { reason, mut notes } => {
            let prefix = match reason {
                ListenSkipReason::UnsupportedPlatform => {
                    "listen capture is not yet wired for this platform; returning probe-only diagnostics."
                }
                ListenSkipReason::UnsupportedFormat => {
                    "listen capture currently emits only WAV; falling back to probe-only diagnostics."
                }
                ListenSkipReason::UnsupportedSource => {
                    "listen capture does not yet drive this source kind; returning probe-only diagnostics."
                }
                ListenSkipReason::RecorderUnavailable => {
                    "no usable audio recorder was found on PATH; returning probe-only diagnostics."
                }
            };
            notes.insert(0, prefix.to_owned());
            ListenExecutionStatus {
                status: "probe_only".to_owned(),
                artifact_available: false,
                artifact: None,
                notes,
            }
        }
        ListenCaptureResult::Failed {
            recorder,
            message,
            mut notes,
        } => {
            notes.insert(
                0,
                format!(
                    "listen recorder `{recorder}` failed: {message}; returning probe-only diagnostics."
                ),
            );
            ListenExecutionStatus {
                status: "probe_only".to_owned(),
                artifact_available: false,
                artifact: None,
                notes,
            }
        }
    };

    Ok(ListenResponse {
        request: input.clone(),
        adapter: adapter_info,
        capability,
        execution,
    })
}

fn probe_listen_capability(
    input: &ListenInput,
    adapter: &dyn crate::platform::PlatformAdapter,
) -> Result<AudioCapabilityReport, TendrilError> {
    let adapter_info = adapter.info();
    let request_value = serde_json::to_value(input)
        .expect("listen input should always serialize for structured diagnostics");
    let adapter_value = serde_json::to_value(&adapter_info)
        .expect("adapter info should always serialize for structured diagnostics");

    let platform_source = match input.source.kind {
        AudioSourceKind::System => PlatformAudioSourceKind::SystemLoopback,
        AudioSourceKind::Microphone => PlatformAudioSourceKind::Microphone,
        AudioSourceKind::Device => {
            return Err(TendrilError::unsupported_capability(
                "audio_device_selection_not_implemented",
                "explicit input-device selection is not implemented in the v0.0.1 listen surface",
                Some(json!({
                    "request": request_value,
                    "adapter": adapter_value,
                    "capability": Capability::AudioInputCapture,
                    "suggested_action": "Use `--source microphone` for the shippable v0.0.1 slice. Explicit per-device binding remains a documented gap for a future adapter implementation."
                })),
            ));
        }
    };

    adapter
        .probe_audio_capture(&AudioProbeRequest {
            source: platform_source,
            duration_hint_ms: Some(input.duration_ms.try_into().unwrap_or(u32::MAX)),
        })
        .map_err(|error| {
            TendrilError::from(error)
                .with_detail_entry("request", request_value.clone())
                .with_detail_entry("adapter", adapter_value.clone())
        })
}

fn render_listen_output(response: &ListenResponse, json_mode: bool) -> CommandOutput {
    if json_mode {
        let envelope = JsonEnvelope::success_for("listen", &response);
        CommandOutput::Json(
            serde_json::to_value(envelope).expect("listen response envelope should serialize"),
        )
    } else {
        let permission_lines = response
            .capability
            .permissions
            .iter()
            .map(|permission| {
                format!(
                    "- {:?}: {:?} ({})",
                    permission.permission, permission.state, permission.summary
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let notes = response.execution.notes.join(" ");
        let artifact_line = match &response.execution.artifact {
            Some(artifact) => format!(
                "artifact: {} ({} bytes, {} Hz, {} ch, recorder={})\n",
                artifact.path.display(),
                artifact.byte_size,
                artifact.sample_rate_hz,
                artifact.channels,
                artifact.recorder,
            ),
            None => String::new(),
        };
        CommandOutput::Human(format!(
            "listen source: {:?}\nformat: {:?}\nduration_ms: {}\nplatform: {:?}\nsession: {:?}\naudio_backend: {:?}\nsupported_sample_rates_hz: {:?}\nsupported_channel_counts: {:?}\npermissions:\n{}\nstatus: {}\n{}notes: {}\n",
            response.request.source,
            response.request.format,
            response.request.duration_ms,
            response.adapter.platform,
            response.adapter.session,
            response.capability.backend,
            response.capability.supported_sample_rates_hz,
            response.capability.supported_channel_counts,
            permission_lines,
            response.execution.status,
            artifact_line,
            notes,
        ))
    }
}

fn build_alias_input(
    target: &TargetScope,
    command: &AliasCommand,
) -> Result<AliasInput, TendrilError> {
    let input = AliasInput {
        target: required_target(target, "alias")?,
        shell: command
            .shell
            .as_deref()
            .map(parse_shell)
            .transpose()?
            .unwrap_or(ShellKind::Bash),
        name: command
            .name
            .clone()
            .unwrap_or_else(|| default_alias_name(target)),
    };
    input.validate()?;
    Ok(input)
}

fn execute_alias(input: &AliasInput) -> AliasOutput {
    let argv = alias_argv(&input.target);
    AliasOutput {
        shell: input.shell,
        name: input.name.clone(),
        command: render_shell_command(&argv, input.shell),
        argv,
        shell_code: render_alias_shell_code(input),
        target: input.target.clone(),
    }
}

fn render_alias_human(output: &AliasOutput) -> String {
    output.shell_code.clone()
}

fn target_scope_from_cli(cli: &TendrilCli) -> TargetScope {
    TargetScope {
        window: cli.window.clone(),
        display: cli.display.clone(),
        camera: cli.camera.clone(),
        x11_display: None,
    }
}

fn required_target(
    target: &TargetScope,
    command: &'static str,
) -> Result<TargetSelector, TendrilError> {
    selected_target(target)
        .map_err(|error| {
            error.with_code(match command {
                "capture" => "invalid_capture_input",
                "run" => "invalid_run_input",
                "alias" => "invalid_alias_input",
                _ => "invalid_input",
            })
        })?
        .ok_or_else(|| {
            TendrilError::validation(format!(
                "{command} requires exactly one of `--window <id>` or `--display <id>`"
            ))
            .with_code(match command {
                "capture" => "invalid_capture_input",
                "run" => "invalid_run_input",
                "alias" => "invalid_alias_input",
                _ => "invalid_input",
            })
            .with_field("target")
        })
}

fn optional_target(target: &TargetScope) -> Result<Option<TargetSelector>, TendrilError> {
    selected_target(target).map_err(|error| {
        error
            .with_code("invalid_list_elements_input")
            .with_field("target")
    })
}

fn selected_target(target: &TargetScope) -> Result<Option<TargetSelector>, TendrilError> {
    match (&target.window, &target.display) {
        (Some(_), Some(_)) => Err(TendrilError::validation(
            "choose either `--window` or `--display`, but not both",
        )
        .with_code("invalid_target_selector")
        .with_field("target")),
        (Some(id), None) => Ok(Some(TargetSelector::Window { id: id.clone() })),
        (None, Some(id)) => Ok(Some(TargetSelector::Display { id: id.clone() })),
        (None, None) => Ok(None),
    }
}

fn parse_image_format(value: &str) -> Result<crate::config::ImageFormat, TendrilError> {
    match value {
        "png" => Ok(crate::config::ImageFormat::Png),
        "jpeg" | "jpg" => Ok(crate::config::ImageFormat::Jpeg),
        other => Err(TendrilError::validation(format!(
            "unsupported image format `{other}`; expected `png` or `jpeg`"
        ))
        .with_code("invalid_capture_input")
        .with_field("format")),
    }
}

fn parse_audio_format(value: &str) -> Result<AudioFormat, TendrilError> {
    match value {
        "wav" => Ok(AudioFormat::Wav),
        "flac" => Ok(AudioFormat::Flac),
        "opus" => Ok(AudioFormat::Opus),
        other => Err(TendrilError::validation(format!(
            "unsupported audio format `{other}`; expected `wav`, `flac`, or `opus`"
        ))
        .with_code("invalid_listen_input")
        .with_field("format")),
    }
}

fn parse_shell(value: &str) -> Result<ShellKind, TendrilError> {
    match value.to_ascii_lowercase().as_str() {
        "bash" => Ok(ShellKind::Bash),
        "zsh" => Ok(ShellKind::Zsh),
        "fish" => Ok(ShellKind::Fish),
        "powershell" | "pwsh" => Ok(ShellKind::PowerShell),
        other => Err(TendrilError::validation(format!(
            "unsupported shell `{other}`; expected bash, zsh, fish, or powershell"
        ))
        .with_code("invalid_alias_input")
        .with_field("shell")),
    }
}

fn alias_argv(target: &TargetSelector) -> Vec<String> {
    let mut argv = vec!["tendril".to_owned()];
    match target {
        TargetSelector::Window { id } => {
            argv.push("--window".to_owned());
            argv.push(id.clone());
        }
        TargetSelector::Display { id } => {
            argv.push("--display".to_owned());
            argv.push(id.clone());
        }
    }
    argv
}

fn render_alias_shell_code(input: &AliasInput) -> String {
    let argv = alias_argv(&input.target);
    let invocation = render_shell_invocation(&argv, input.shell);
    match input.shell {
        ShellKind::Bash | ShellKind::Zsh => {
            format!("{}() {{\n  {} \"$@\"\n}}\n", input.name, invocation)
        }
        ShellKind::Fish => format!("function {}\n    {} $argv\nend\n", input.name, invocation),
        ShellKind::PowerShell => format!(
            "function {} {{\n    param(\n        [Parameter(ValueFromRemainingArguments = $true)]\n        [string[]]$Args\n    )\n    {} @Args\n}}\n",
            input.name, invocation,
        ),
    }
}

fn render_shell_invocation(argv: &[String], shell: ShellKind) -> String {
    match shell {
        ShellKind::PowerShell => format!("& {}", render_shell_command(argv, shell)),
        ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish => {
            format!("command {}", render_shell_command(argv, shell))
        }
    }
}

fn render_shell_command(argv: &[String], shell: ShellKind) -> String {
    let mut rendered = Vec::with_capacity(argv.len());
    if let Some(program) = argv.first() {
        rendered.push(program.clone());
    }
    rendered.extend(
        argv.iter()
            .skip(1)
            .map(|argument| quote_for_shell(argument, shell)),
    );
    rendered.join(" ")
}

fn quote_for_shell(argument: &str, shell: ShellKind) -> String {
    if !argument.is_empty() && argument.chars().all(is_shell_safe_character) {
        return argument.to_owned();
    }

    match shell {
        ShellKind::PowerShell => format!("'{}'", argument.replace('\'', "''")),
        ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish => {
            format!("'{}'", argument.replace('\'', "'\"'\"'"))
        }
    }
}

fn is_shell_safe_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/' | ':')
}

fn payload_kind(payload: &RunInputPayload) -> &'static str {
    match payload {
        RunInputPayload::Text { .. } => "text",
        RunInputPayload::Dsl { .. } => "dsl",
        RunInputPayload::Actions { .. } => "actions",
    }
}

fn default_alias_name(target: &TargetScope) -> String {
    if let Some(window) = &target.window {
        format!("tendril_window_{}", sanitize_identifier(window))
    } else if let Some(display) = &target.display {
        format!("tendril_display_{}", sanitize_identifier(display))
    } else {
        "tendril_target".to_owned()
    }
}

fn sanitize_identifier(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;

    use image::{DynamicImage, ImageFormat as RasterImageFormat, Rgba, RgbaImage};
    use mcp_cli::JsonEnvelope;
    use serde_json::{Value, json};

    use super::{
        AliasRequest, CaptureRequest, ClipboardGetRequest, ClipboardSetRequest, ElementListRequest,
        ListRequest, ListenRequest, RunRequest, TargetScope, build_alias_input,
        build_capture_input, build_clipboard_get_input, build_clipboard_set_input,
        build_listen_input, build_listen_response, build_mcp_server, build_run_input, dispatch,
        dispatch_listen_command, execute_alias, execute_list_elements, execute_list_with_adapter,
        render_command_output, render_list_elements_human, render_list_human,
    };
    use super::{
        CommandOutput, PermissionRequestReport, PermissionsReport, dispatch_permissions_command,
        render_permission_request_human, render_permissions_human, unsupported_permission_request,
    };
    use crate::capture::{execute_capture, render_capture_human};

    #[test]
    fn x11_display_override_deserializes_and_threads_into_context() {
        // bd-6abe70: TargetScope-based tool payloads (capture/run/list_elements/
        // alias) carry a per-call x11_display override.
        let capture: CaptureRequest =
            serde_json::from_value(json!({"window": "0x1", "x11_display": ":99"})).unwrap();
        assert_eq!(capture.target.x11_display.as_deref(), Some(":99"));
        let run: RunRequest = serde_json::from_value(
            json!({"input_definition": "lclick(1,2)", "x11_display": ":42"}),
        )
        .unwrap();
        assert_eq!(run.target.x11_display.as_deref(), Some(":42"));

        // The list tool has no target scope, so it carries x11_display directly.
        let list: ListRequest =
            serde_json::from_value(json!({"x11_display": ":7", "all_apps": true})).unwrap();
        assert_eq!(list.x11_display.as_deref(), Some(":7"));
        assert!(list.options.all_apps);

        // Absent override stays None -> unchanged ambient-$DISPLAY behaviour.
        let bare: CaptureRequest = serde_json::from_value(json!({"window": "0x1"})).unwrap();
        assert_eq!(bare.target.x11_display, None);

        // The AdapterContext builder threads the override and drops empty values.
        let ctx =
            crate::platform::AdapterContext::linux(crate::platform::DesktopSession::X11, None)
                .with_x11_display(Some(":99".to_owned()));
        assert_eq!(ctx.x11_display.as_deref(), Some(":99"));
        let empty =
            crate::platform::AdapterContext::linux(crate::platform::DesktopSession::X11, None)
                .with_x11_display(Some(String::new()));
        assert_eq!(empty.x11_display, None);
    }
    use crate::cli::{
        AliasCommand, CaptureCommand, ClipboardGetCommand, ClipboardSetCommand, Command,
        ListCommand, ListenCommand, McpCommand, McpSubcommand, PermissionsCommand, RunCommand,
        TendrilCli, WORKFLOW_HINT,
    };
    use crate::clipboard::{
        ClipboardSelection, DEFAULT_CLIPBOARD_SERVE_MS, DEFAULT_CLIPBOARD_TIMEOUT_MS,
    };
    use crate::config::{ImageFormat, TendrilConfig};
    use crate::error::TendrilError;
    use crate::input::{execute_run, render_run_human};
    use crate::model::{
        AudioFormat, AudioSourceKind, Bounds, CaptureInput, ElementListInput, RunInput, ShellKind,
        TargetSelector,
    };
    use crate::platform::{
        AdapterContext, AdapterInfo, AudioBackend, AudioCapabilityProbe, AudioCapabilityReport,
        AudioProbeRequest, CaptureAdapter, CaptureArtifact,
        CaptureRequest as PlatformCaptureRequest, CaptureTargetKind, DesktopSession,
        FeatureSupport, InputControlAdapter, InputOutcome, PermissionAdapter, PermissionKind,
        PermissionRequestOutcome, PermissionState, PermissionStatus, PlatformAdapter,
        PlatformAdapterError, PlatformKind, TargetDescriptor as PlatformTargetDescriptor,
        TargetDiscoveryAdapter, TargetDiscoveryRequest, TargetInventory,
    };
    use mcp_cli::ErrorCategory;

    #[test]
    fn permissions_human_render_lists_states_and_actions() {
        let report = PermissionsReport {
            adapter: AdapterInfo::from_context(&AdapterContext::windows11()),
            permissions: vec![
                PermissionStatus::granted(PermissionKind::ScreenCapture, "screen ok"),
                PermissionStatus::denied(
                    PermissionKind::Accessibility,
                    "accessibility missing",
                    "open settings",
                ),
                PermissionStatus::not_required(PermissionKind::Microphone, "mic not needed"),
            ],
        };
        let rendered = render_permissions_human(&report);
        assert!(rendered.contains("Screen Recording: granted"));
        assert!(rendered.contains("screen ok"));
        assert!(rendered.contains("Accessibility (input control): denied"));
        assert!(rendered.contains("-> open settings"));
        assert!(rendered.contains("Microphone: not required"));
    }

    #[test]
    fn permissions_human_render_handles_empty_permissions() {
        let report = PermissionsReport {
            adapter: AdapterInfo::from_context(&AdapterContext::windows11()),
            permissions: Vec::new(),
        };
        let rendered = render_permissions_human(&report);
        assert!(rendered.contains("no platform permissions are required"));
    }

    #[test]
    fn permissions_command_returns_json_envelope_on_host_adapter() {
        let output = dispatch_permissions_command(&PermissionsCommand::default(), true);
        assert!(matches!(output, CommandOutput::Json(_)));
    }

    #[test]
    fn permissions_command_renders_human_output_on_host_adapter() {
        let output = dispatch_permissions_command(&PermissionsCommand::default(), false);
        match output {
            CommandOutput::Human(text) => assert!(text.contains("platform:")),
            other => panic!("expected human output, got {other:?}"),
        }
    }

    #[test]
    fn permission_request_human_render_lists_outcomes_and_current_status() {
        let report = PermissionRequestReport {
            adapter: AdapterInfo::from_context(&AdapterContext::macos()),
            requested: vec![PermissionRequestOutcome {
                permission: PermissionKind::Accessibility,
                actions: vec![
                    "surfaced the Accessibility prompt via AXIsProcessTrustedWithOptions"
                        .to_owned(),
                    "opened System Settings pane: x-apple.systempreferences:...".to_owned(),
                ],
                state_after: PermissionState::Denied,
                note: Some("attribution caveat, see bd-5110d9".to_owned()),
            }],
            permissions: vec![PermissionStatus::denied(
                PermissionKind::Accessibility,
                "accessibility missing",
                "open settings",
            )],
        };
        let rendered = render_permission_request_human(&report);
        assert!(rendered.contains("permission request flow"));
        assert!(rendered.contains("Accessibility (input control): denied (after request)"));
        assert!(rendered.contains("surfaced the Accessibility prompt"));
        assert!(rendered.contains("note: attribution caveat, see bd-5110d9"));
        assert!(rendered.contains("current status:"));
    }

    #[test]
    fn unsupported_permission_request_reports_no_prompt_fired() {
        let outcomes = unsupported_permission_request(PlatformKind::Linux);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].state_after, PermissionState::Unknown);
        assert!(
            outcomes[0]
                .actions
                .iter()
                .any(|action| action.contains("only implemented on macOS"))
        );
    }

    #[derive(Debug)]
    struct FakeAdapter {
        inventory: TargetInventory,
        image_bytes: Vec<u8>,
    }

    impl Default for FakeAdapter {
        fn default() -> Self {
            Self {
                inventory: TargetInventory {
                    targets: vec![PlatformTargetDescriptor {
                        id: "window-1".to_owned(),
                        title: Some("Inbox".to_owned()),
                        kind: CaptureTargetKind::Window,
                        name: "Terminal".to_owned(),
                        bounds: Bounds {
                            x: 10,
                            y: 20,
                            width: 1280,
                            height: 720,
                        },
                        scale_factor: crate::model::ScaleFactor::identity(),
                        capture_supported: true,
                        input_supported: true,
                        app_name: Some("Mail".to_owned()),
                        process_id: Some(42),
                        diagnostics: Vec::new(),
                    }],
                },
                image_bytes: sample_png_bytes(),
            }
        }
    }

    impl TargetDiscoveryAdapter for FakeAdapter {
        fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
            Ok(FeatureSupport::available(
                crate::platform::Capability::TargetDiscovery,
            ))
        }

        fn discover_targets(
            &self,
            _request: &TargetDiscoveryRequest,
        ) -> Result<TargetInventory, PlatformAdapterError> {
            Ok(self.inventory.clone())
        }
    }

    impl CaptureAdapter for FakeAdapter {
        fn capture_support(
            &self,
            target: CaptureTargetKind,
        ) -> Result<FeatureSupport, PlatformAdapterError> {
            Ok(FeatureSupport::available(match target {
                CaptureTargetKind::Window => crate::platform::Capability::WindowCapture,
                CaptureTargetKind::Display => crate::platform::Capability::DisplayCapture,
            }))
        }

        fn capture(
            &self,
            request: &PlatformCaptureRequest,
        ) -> Result<CaptureArtifact, PlatformAdapterError> {
            Ok(CaptureArtifact {
                target_id: request.target_id.clone(),
                media_type: "image/png".to_owned(),
                image_bytes: self.image_bytes.clone(),
                captured_at: "2026-04-09T18:00:00Z".to_owned(),
            })
        }
    }

    impl InputControlAdapter for FakeAdapter {
        fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
            Ok(FeatureSupport::available(
                crate::platform::Capability::InputControl,
            ))
        }

        fn execute_input(
            &self,
            request: &crate::platform::InputRequest,
        ) -> Result<InputOutcome, crate::error::TendrilError> {
            Ok(InputOutcome {
                action_count: request.actions.len() + usize::from(request.text.is_some()),
                focus_required: true,
                focus_transferred: true,
                focused_target: Some(request.target_id.clone()),
                previous_focus: request.restore_focus.then(|| crate::model::FocusSnapshot {
                    id: "previous-window".to_owned(),
                    kind: "window".to_owned(),
                    name: Some("Previous".to_owned()),
                }),
                focus_restored: request.restore_focus,
                pointer_restored: false,
                restore_error: None,
                notes: vec!["fake adapter executed request".to_owned()],
            })
        }
    }

    impl PermissionAdapter for FakeAdapter {
        fn permissions(&self) -> Vec<crate::platform::PermissionStatus> {
            Vec::new()
        }
    }

    impl AudioCapabilityProbe for FakeAdapter {
        fn probe_audio_capture(
            &self,
            request: &AudioProbeRequest,
        ) -> Result<AudioCapabilityReport, PlatformAdapterError> {
            Ok(AudioCapabilityReport {
                source: request.source,
                backend: AudioBackend::Wasapi,
                supported_sample_rates_hz: vec![48_000],
                supported_channel_counts: vec![2],
                permissions: Vec::new(),
                notes: vec!["fake adapter probe".to_owned()],
            })
        }
    }

    impl PlatformAdapter for FakeAdapter {
        fn info(&self) -> AdapterInfo {
            AdapterInfo {
                platform: PlatformKind::Windows11,
                session: DesktopSession::WindowsDesktop,
                audio_backend: Some(AudioBackend::Wasapi),
                stateless: true,
            }
        }
    }

    fn sample_png_bytes() -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([0, 255, 0, 255])));
        let mut encoded = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut encoded), RasterImageFormat::Png)
            .expect("sample image should encode");
        encoded
    }

    fn fake_adapter() -> Arc<dyn PlatformAdapter> {
        Arc::new(FakeAdapter::default())
    }

    fn mcp_context(adapter: Arc<dyn PlatformAdapter>) -> super::CommandContext {
        super::CommandContext {
            config: TendrilConfig::default(),
            adapter_context: AdapterContext::windows11(),
            adapter: Some(adapter),
        }
    }

    fn mcp_structured_content(response: &Value) -> Value {
        response["result"]["structuredContent"].clone()
    }

    fn tool_call_response(context: &super::CommandContext, name: &str, arguments: &Value) -> Value {
        build_mcp_server()
            .handle_request_value(
                context,
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": name,
                        "arguments": arguments
                    }
                }),
            )
            .expect("request should parse")
            .expect("response should exist")
    }

    fn expect_json_output(output: super::CommandOutput) -> Value {
        match output {
            super::CommandOutput::Json(value) => value,
            super::CommandOutput::Human(_) | super::CommandOutput::Empty => {
                panic!("expected json output")
            }
        }
    }

    fn mcp_cli(window: Option<&str>, display: Option<&str>, json: bool) -> TendrilCli {
        TendrilCli {
            json,
            window: window.map(str::to_owned),
            display: display.map(str::to_owned),
            camera: None,
            remote: None,
            wsl_tunnel: false,
            android: None,
            command: Some(Command::Mcp(McpCommand {
                command: McpSubcommand::Stdio,
            })),
        }
    }

    fn assert_mcp_rejects(cli: &TendrilCli, expected_flag: &str) {
        let error = dispatch(cli, &TendrilConfig::default())
            .expect_err("MCP dispatch with inherited target flags should be rejected");
        assert!(matches!(error, TendrilError::Validation { .. }));
        assert_eq!(error.category(), ErrorCategory::Validation);
        let message = format!("{error}");
        assert!(
            message.contains(expected_flag),
            "expected error to mention {expected_flag}: {message}"
        );
        assert!(
            message.contains("MCP server does not honor"),
            "expected error to explain ignored flag: {message}"
        );
    }

    #[test]
    fn mcp_dispatch_rejects_inherited_window_flag() {
        assert_mcp_rejects(&mcp_cli(Some("hypr:0xabc"), None, false), "--window");
    }

    #[test]
    fn mcp_dispatch_rejects_inherited_display_flag() {
        assert_mcp_rejects(&mcp_cli(None, Some("DP-1"), false), "--display");
    }

    #[test]
    fn mcp_dispatch_rejects_inherited_json_flag() {
        assert_mcp_rejects(&mcp_cli(None, None, true), "--json");
    }

    #[test]
    fn mcp_dispatch_rejects_multiple_inherited_flags_in_one_message() {
        let cli = mcp_cli(Some("win"), Some("DP-1"), true);
        let error = dispatch(&cli, &TendrilConfig::default())
            .expect_err("MCP dispatch should reject combined inherited flags");
        let message = format!("{error}");
        for flag in ["--window", "--display", "--json"] {
            assert!(
                message.contains(flag),
                "expected error to mention {flag}: {message}"
            );
        }
    }

    #[test]
    fn json_help_dispatch_returns_machine_readable_envelope() {
        let cli = TendrilCli {
            json: true,
            window: None,
            display: None,
            camera: None,
            remote: None,
            wsl_tunnel: false,
            android: None,
            command: None,
        };

        let output = dispatch(&cli, &TendrilConfig::default()).expect("help should render");

        match output {
            super::CommandOutput::Json(value) => {
                assert_eq!(value["status"], "success");
                assert_eq!(value["meta"]["command"], "help");
                assert!(
                    value["data"]["help"]
                        .as_str()
                        .expect("help string")
                        .contains("Workflow:")
                );
                assert_eq!(value["data"]["workflow_hint"], WORKFLOW_HINT);
                assert_eq!(
                    value["data"]["workflow_steps"][0]["command"],
                    "tendril list --json"
                );
                assert_eq!(
                    value["data"]["workflow_steps"][1]["command"],
                    "tendril --remote me@box list --json"
                );
                assert_eq!(
                    value["data"]["workflow_steps"][2]["command"],
                    "tendril --window <id> list-elements --json"
                );
                assert_eq!(
                    value["data"]["workflow_steps"][3]["command"],
                    "tendril --window <id> capture --json"
                );
                assert_eq!(
                    value["data"]["workflow_steps"][4]["command"],
                    "tendril --window <id> capture -o /tmp/screen.png"
                );
                assert_eq!(
                    value["data"]["workflow_steps"][5]["command"],
                    "tendril --window <id> run 'send(\"hello\")'"
                );
                let command_names: Vec<&str> = value["data"]["commands"]
                    .as_array()
                    .expect("commands array")
                    .iter()
                    .map(|command| {
                        command["name"]
                            .as_str()
                            .expect("command summary should include a string name")
                    })
                    .collect();
                for expected in ["list", "update", "version", "mcp stdio"] {
                    assert!(
                        command_names.contains(&expected),
                        "expected JSON help command list to include {expected}; got {command_names:?}"
                    );
                }
                let workflow_commands: Vec<&str> = value["data"]["workflow_steps"]
                    .as_array()
                    .expect("workflow_steps array")
                    .iter()
                    .map(|step| {
                        step["command"]
                            .as_str()
                            .expect("workflow step should include a string command")
                    })
                    .collect();
                for expected in [
                    "tendril --wsl-tunnel list --json",
                    "tendril --android <serial> list --json",
                ] {
                    assert!(
                        workflow_commands.contains(&expected),
                        "expected JSON help workflow_steps to include {expected}; got {workflow_commands:?}"
                    );
                }
            }
            _ => panic!("expected json output"),
        }
    }

    #[test]
    fn capture_input_uses_config_defaults() {
        let target = TargetScope {
            window: Some("window-1".into()),
            display: None,
            camera: None,
            x11_display: None,
        };
        let input = build_capture_input(
            &target,
            &CaptureCommand {
                max_width: None,
                max_height: None,
                format: None,
                compression: None,
                output: None,
                timeout_ms: None,
            },
            &TendrilConfig::default(),
        )
        .expect("capture input should build");

        assert_eq!(input.format, ImageFormat::Png);
        assert_eq!(input.compression, 85);
    }

    #[test]
    fn capture_request_schema_includes_target_scope_and_capture_options() {
        let schema = serde_json::to_value(schemars::schema_for!(CaptureRequest))
            .expect("capture schema should serialize");

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("window").is_some());
        assert!(schema["properties"].get("display").is_some());
        assert!(schema["properties"].get("max_width").is_some());
        assert!(schema["properties"].get("compression").is_some());
    }

    #[test]
    fn run_request_preserves_shared_target_scope_model() {
        let request = RunRequest {
            target: TargetScope {
                window: Some("window-1".to_string()),
                display: None,
                camera: None,
                x11_display: None,
            },
            options: RunCommand {
                input_definition: Some("send(\"hello\")".to_string()),
                ..RunCommand::default()
            },
        };

        assert_eq!(request.target.window.as_deref(), Some("window-1"));
        assert_eq!(
            request.options.input_definition.as_deref(),
            Some("send(\"hello\")")
        );
        assert!(request.options.restore_focus);
        assert!(!request.options.no_restore_focus);
    }

    #[test]
    fn run_request_schema_includes_focus_restore_flags() {
        let schema = serde_json::to_value(schemars::schema_for!(RunRequest))
            .expect("run schema should serialize");

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("restore_focus").is_some());
        assert!(schema["properties"].get("no_restore_focus").is_some());
    }

    #[test]
    fn run_input_disables_focus_restore_when_requested() {
        let input = build_run_input(
            &TargetScope {
                window: Some("window-1".to_owned()),
                display: None,
                camera: None,
                x11_display: None,
            },
            &RunCommand {
                input_definition: Some("send(\"hello\")".to_owned()),
                no_restore_focus: true,
                ..RunCommand::default()
            },
        )
        .expect("run input should build");

        assert!(!input.restore_focus);
    }

    #[test]
    fn mcp_server_lists_initial_stdio_tools() {
        let server = build_mcp_server();
        let names: Vec<_> = server
            .tool_metadata()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "list",
                "list_elements",
                "capture",
                "run",
                "listen",
                "clipboard_get",
                "clipboard_set",
                "permissions",
                "self_update_status",
                "self_update_check",
                "self_update_run",
                "feedback_report",
                "feedback_status"
            ]
        );
    }

    #[test]
    fn mcp_tool_schemas_match_effective_cli_inputs() {
        let tools = build_mcp_server().tool_metadata();
        let list = tools
            .iter()
            .find(|tool| tool.name == "list")
            .expect("list tool should be registered");
        let list_elements = tools
            .iter()
            .find(|tool| tool.name == "list_elements")
            .expect("list_elements tool should be registered");
        let capture = tools
            .iter()
            .find(|tool| tool.name == "capture")
            .expect("capture tool should be registered");
        let run = tools
            .iter()
            .find(|tool| tool.name == "run")
            .expect("run tool should be registered");
        let listen = tools
            .iter()
            .find(|tool| tool.name == "listen")
            .expect("listen tool should be registered");
        let clipboard_get = tools
            .iter()
            .find(|tool| tool.name == "clipboard_get")
            .expect("clipboard_get tool should be registered");
        let clipboard_set = tools
            .iter()
            .find(|tool| tool.name == "clipboard_set")
            .expect("clipboard_set tool should be registered");
        let self_update_status = tools
            .iter()
            .find(|tool| tool.name == "self_update_status")
            .expect("self_update_status tool should be registered");
        let self_update_check = tools
            .iter()
            .find(|tool| tool.name == "self_update_check")
            .expect("self_update_check tool should be registered");
        let self_update_run = tools
            .iter()
            .find(|tool| tool.name == "self_update_run")
            .expect("self_update_run tool should be registered");

        assert_eq!(
            list.input_schema,
            serde_json::to_value(schemars::schema_for!(ListRequest))
                .expect("list schema should serialize")
        );
        assert_eq!(
            list_elements.input_schema,
            serde_json::to_value(schemars::schema_for!(ElementListRequest))
                .expect("list_elements schema should serialize")
        );
        assert_eq!(
            capture.input_schema,
            serde_json::to_value(schemars::schema_for!(CaptureRequest))
                .expect("capture schema should serialize")
        );
        assert_eq!(
            run.input_schema,
            serde_json::to_value(schemars::schema_for!(RunRequest))
                .expect("run schema should serialize")
        );
        assert_eq!(
            listen.input_schema,
            serde_json::to_value(schemars::schema_for!(ListenRequest))
                .expect("listen schema should serialize")
        );
        assert_eq!(
            clipboard_get.input_schema,
            serde_json::to_value(schemars::schema_for!(ClipboardGetRequest))
                .expect("clipboard_get schema should serialize")
        );
        assert_eq!(
            clipboard_set.input_schema,
            serde_json::to_value(schemars::schema_for!(ClipboardSetRequest))
                .expect("clipboard_set schema should serialize")
        );
        let empty_schema = serde_json::to_value(schemars::schema_for!(updatable_cli::EmptyArgs))
            .expect("self-update empty schema should serialize");
        assert_eq!(self_update_status.input_schema, empty_schema);
        assert_eq!(self_update_check.input_schema, empty_schema);
        assert_eq!(self_update_run.input_schema, empty_schema);
    }

    #[test]
    fn clipboard_input_parses_selection_and_defaults() {
        let get = build_clipboard_get_input(&ClipboardGetCommand {
            selection: Some("primary".to_owned()),
            timeout_ms: None,
        })
        .expect("clipboard get input should build");
        assert_eq!(get.selection, ClipboardSelection::Primary);
        assert_eq!(get.timeout_ms, DEFAULT_CLIPBOARD_TIMEOUT_MS);

        let set = build_clipboard_set_input(&ClipboardSetCommand {
            selection: None,
            text: "hello clipboard".to_owned(),
            serve_ms: None,
        })
        .expect("clipboard set input should build");
        assert_eq!(set.selection, ClipboardSelection::Clipboard);
        assert_eq!(set.serve_ms, DEFAULT_CLIPBOARD_SERVE_MS);
    }

    #[test]
    fn listen_input_parses_requested_source_duration_and_format() {
        let input = build_listen_input(&ListenCommand {
            source: Some("loopback".to_string()),
            duration_ms: Some(1_500),
            format: Some("flac".to_string()),
            output: None,
        })
        .expect("listen input should build");

        assert_eq!(input.source.kind, AudioSourceKind::System);
        assert_eq!(input.duration_ms, 1_500);
        assert_eq!(input.format, AudioFormat::Flac);
    }

    #[test]
    fn listen_request_schema_includes_source_duration_and_format() {
        let schema = serde_json::to_value(schemars::schema_for!(ListenCommand))
            .expect("listen schema should serialize");

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("source").is_some());
        assert!(schema["properties"].get("duration_ms").is_some());
        assert!(schema["properties"].get("format").is_some());
    }

    #[test]
    fn listen_probe_reports_probe_only_gap_on_unwired_platform() {
        let response = build_listen_response(
            &build_listen_input(&ListenCommand {
                source: Some("system".to_string()),
                duration_ms: Some(2_000),
                format: Some("wav".to_string()),
                output: None,
            })
            .expect("listen input should build"),
            None,
            &AdapterContext::windows11(),
        )
        .expect("windows loopback probe should succeed");

        assert_eq!(response.capability.backend, AudioBackend::Wasapi);
        // Windows is not yet wired for real capture, so it must continue to
        // report the probe-only gap so callers do not assume an artifact.
        assert_eq!(response.execution.status, "probe_only");
        assert!(!response.execution.artifact_available);
        assert!(response.execution.artifact.is_none());
        assert!(
            response
                .execution
                .notes
                .iter()
                .any(|note| note.contains("not yet wired")),
            "expected an unsupported-platform note, got {:?}",
            response.execution.notes
        );
    }

    #[test]
    fn capture_request_flattens_capture_command_fields() {
        let request = CaptureRequest {
            target: TargetScope {
                window: None,
                display: Some("1".to_string()),
                camera: None,
                x11_display: None,
            },
            options: CaptureCommand {
                max_width: Some(800),
                max_height: Some(600),
                format: Some("png".to_string()),
                compression: Some(90),
                output: None,
                timeout_ms: None,
            },
        };

        assert_eq!(request.target.display.as_deref(), Some("1"));
        assert_eq!(request.options.max_width, Some(800));
        assert_eq!(request.options.compression, Some(90));
    }

    #[test]
    fn alias_request_schema_includes_target_scope_and_alias_options() {
        let schema = serde_json::to_value(schemars::schema_for!(AliasRequest))
            .expect("alias schema should serialize");

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("window").is_some());
        assert!(schema["properties"].get("display").is_some());
        assert!(schema["properties"].get("shell").is_some());
        assert!(schema["properties"].get("name").is_some());
    }

    #[test]
    fn alias_output_is_shell_usable_and_transparent() {
        let input = build_alias_input(
            &TargetScope {
                window: Some("window-1".to_string()),
                display: None,
                camera: None,
                x11_display: None,
            },
            &AliasCommand {
                shell: Some("bash".to_string()),
                name: Some("desk".to_string()),
            },
        )
        .expect("alias input should build");

        let output = execute_alias(&input);

        assert_eq!(output.shell, ShellKind::Bash);
        assert_eq!(output.name, "desk");
        assert_eq!(output.argv, vec!["tendril", "--window", "window-1"]);
        assert_eq!(output.command, "tendril --window window-1");
        assert!(output.shell_code.contains("desk()"));
        assert!(
            output
                .shell_code
                .contains("command tendril --window window-1 \"$@\"")
        );
    }

    #[test]
    fn mcp_list_success_payload_matches_cli_json() {
        let adapter = fake_adapter();
        let context = mcp_context(adapter.clone());
        let response = tool_call_response(&context, "list", &json!({}));

        let cli_output = execute_list_with_adapter(
            &super::validate_list_command(&ListCommand::default())
                .expect("list input should validate"),
            adapter.as_ref(),
        )
        .expect("list output should build");
        let cli_json = expect_json_output(render_command_output(
            "list",
            true,
            cli_output,
            render_list_human,
        ));

        assert_eq!(response["result"]["isError"], false);
        assert_eq!(mcp_structured_content(&response), cli_json);
    }

    #[test]
    fn mcp_list_elements_success_payload_matches_cli_json() {
        let adapter = fake_adapter();
        let context = mcp_context(adapter.clone());
        let response = tool_call_response(
            &context,
            "list_elements",
            &json!({
                "window": "window-1"
            }),
        );

        let cli_output = execute_list_elements(
            &ElementListInput {
                target: Some(TargetSelector::Window {
                    id: "window-1".to_owned(),
                }),
                include_offscreen: false,
            },
            adapter.as_ref(),
        )
        .expect("list-elements output should build");
        let cli_json = expect_json_output(render_command_output(
            "list_elements",
            true,
            cli_output,
            render_list_elements_human,
        ));

        assert_eq!(response["result"]["isError"], false);
        assert_eq!(mcp_structured_content(&response), cli_json);
        assert_eq!(
            response["result"]["structuredContent"]["data"]["elements"][0]["id"],
            "1"
        );
    }

    #[test]
    fn mcp_capture_success_payload_matches_cli_json() {
        let adapter = fake_adapter();
        let context = mcp_context(adapter.clone());
        let response = tool_call_response(
            &context,
            "capture",
            &json!({
                "window": "window-1",
                "max_width": 1,
                "format": "png",
                "compression": 90
            }),
        );

        let cli_output = execute_capture(
            &CaptureInput {
                target: TargetSelector::Window {
                    id: "window-1".to_owned(),
                },
                max_width: Some(1),
                max_height: None,
                format: ImageFormat::Png,
                compression: 90,
                timeout_ms: None,
            },
            adapter.as_ref(),
        )
        .expect("capture output should build");
        let cli_json = expect_json_output(render_command_output(
            "capture",
            true,
            cli_output,
            render_capture_human,
        ));

        assert_eq!(response["result"]["isError"], false);
        assert_eq!(mcp_structured_content(&response), cli_json);
    }

    #[test]
    fn mcp_run_success_payload_matches_cli_json() {
        let adapter = fake_adapter();
        let context = mcp_context(adapter.clone());
        let tempdir = tempfile::tempdir().expect("temp lock path");
        let response = tool_call_response(
            &context,
            "run",
            &json!({
                "window": "window-1",
                "input_definition": "send(\"hello\")",
                "lock_path": tempdir.path().join("run-lock"),
            }),
        );

        let cli_output = execute_run(
            &RunInput {
                target: TargetSelector::Window {
                    id: "window-1".to_owned(),
                },
                payload: crate::model::RunInputPayload::Actions {
                    actions: vec![crate::model::InputAction::Send {
                        text: "hello".to_owned(),
                    }],
                },
                restore_focus: true,
            },
            adapter.as_ref(),
        )
        .expect("run output should build");
        let mut cli_json = expect_json_output(render_command_output(
            "run",
            true,
            cli_output,
            render_run_human,
        ));
        let structured = mcp_structured_content(&response);
        cli_json["data"]["execution_lock"] = structured["data"]["execution_lock"].clone();

        assert_eq!(response["result"]["isError"], false);
        assert_eq!(structured["data"]["execution_lock"]["enabled"], true);
        assert_eq!(structured["data"]["execution_lock"]["acquired"], true);
        assert_eq!(structured, cli_json);
        assert_eq!(
            response["result"]["structuredContent"]["data"]["previous_focus"]["id"],
            "previous-window"
        );
        assert_eq!(
            response["result"]["structuredContent"]["data"]["focus_restored"],
            true
        );
        assert_eq!(
            response["result"]["structuredContent"]["data"]["pointer_restored"],
            false
        );
    }

    #[test]
    fn mcp_run_no_lock_reports_disabled_execution_lock_metadata() {
        let response = tool_call_response(
            &mcp_context(fake_adapter()),
            "run",
            &json!({
                "window": "window-1",
                "input_definition": "send(\"hello\")",
                "no_lock": true
            }),
        );
        let structured = mcp_structured_content(&response);

        assert_eq!(response["result"]["isError"], false);
        assert_eq!(structured["data"]["execution_lock"]["enabled"], false);
        assert_eq!(structured["data"]["execution_lock"]["acquired"], false);
        assert_eq!(structured["data"]["execution_lock"]["reason"], "--no-lock");
    }

    #[test]
    fn mcp_capture_error_payload_matches_cli_json() {
        let cli_error = dispatch(
            &TendrilCli {
                json: true,
                window: Some("window-1".to_owned()),
                display: Some("1".to_owned()),
                camera: None,
                remote: None,
                wsl_tunnel: false,
                android: None,
                command: Some(Command::Capture(CaptureCommand::default())),
            },
            &TendrilConfig::default(),
        )
        .expect_err("capture should reject conflicting target flags");
        let cli_json = serde_json::to_value(JsonEnvelope::<Value>::error_for(
            "capture",
            cli_error.to_json_error(),
        ))
        .expect("error envelope should serialize");

        let response = tool_call_response(
            &mcp_context(fake_adapter()),
            "capture",
            &json!({
                "window": "window-1",
                "display": "1"
            }),
        );

        assert_eq!(response["result"]["isError"], true);
        assert_eq!(mcp_structured_content(&response), cli_json);
    }

    #[test]
    fn mcp_run_error_payload_matches_cli_json() {
        let cli_error = dispatch(
            &TendrilCli {
                json: true,
                window: Some("window-1".to_owned()),
                display: None,
                camera: None,
                remote: None,
                wsl_tunnel: false,
                android: None,
                command: Some(Command::Run(RunCommand {
                    input_definition: None,
                    ..RunCommand::default()
                })),
            },
            &TendrilConfig::default(),
        )
        .expect_err("run should require an input definition");
        let cli_json = serde_json::to_value(JsonEnvelope::<Value>::error_for(
            "run",
            cli_error.to_json_error(),
        ))
        .expect("error envelope should serialize");

        let response = tool_call_response(
            &mcp_context(fake_adapter()),
            "run",
            &json!({
                "window": "window-1"
            }),
        );

        assert_eq!(response["result"]["isError"], true);
        assert_eq!(mcp_structured_content(&response), cli_json);
    }

    /// Regression for bd-da01d3: when the adapter cannot satisfy input control
    /// (for example a Wayland session with neither `ydotool` nor `wtype` on
    /// PATH), `execute_run` must surface the actionable adapter-level
    /// missing-backend diagnostic instead of the generic per-target
    /// `input_not_supported_for_target` capability error. The actionable
    /// diagnostic names both helper tools and points at an install path.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn run_surfaces_wayland_missing_backend_error_before_generic_target_check() {
        #[derive(Debug)]
        struct WaylandMissingBackendsAdapter;

        impl TargetDiscoveryAdapter for WaylandMissingBackendsAdapter {
            fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
                Ok(FeatureSupport::available(
                    crate::platform::Capability::TargetDiscovery,
                ))
            }

            fn discover_targets(
                &self,
                _request: &TargetDiscoveryRequest,
            ) -> Result<TargetInventory, PlatformAdapterError> {
                // Discovery on Wayland marks targets as `input_supported = false`
                // when no helper tool is detected. Without the bd-da01d3 fix
                // this would route execute_run into the generic
                // `input_not_supported_for_target` branch and hide the
                // actionable remediation guidance.
                Ok(TargetInventory {
                    targets: vec![PlatformTargetDescriptor {
                        id: "display-1".to_owned(),
                        title: Some("HDMI-A-1".to_owned()),
                        kind: CaptureTargetKind::Display,
                        name: "HDMI-A-1".to_owned(),
                        bounds: Bounds {
                            x: 0,
                            y: 0,
                            width: 1920,
                            height: 1080,
                        },
                        scale_factor: crate::model::ScaleFactor::identity(),
                        capture_supported: true,
                        input_supported: false,
                        app_name: None,
                        process_id: None,
                        diagnostics: Vec::new(),
                    }],
                })
            }
        }

        impl CaptureAdapter for WaylandMissingBackendsAdapter {
            fn capture_support(
                &self,
                target: CaptureTargetKind,
            ) -> Result<FeatureSupport, PlatformAdapterError> {
                Ok(FeatureSupport::available(match target {
                    CaptureTargetKind::Window => crate::platform::Capability::WindowCapture,
                    CaptureTargetKind::Display => crate::platform::Capability::DisplayCapture,
                }))
            }

            fn capture(
                &self,
                _request: &PlatformCaptureRequest,
            ) -> Result<CaptureArtifact, PlatformAdapterError> {
                unreachable!("capture should not be invoked for this regression test")
            }
        }

        impl InputControlAdapter for WaylandMissingBackendsAdapter {
            fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
                Err(crate::wayland_input::missing_backend_error(
                    PlatformKind::Linux,
                ))
            }

            fn execute_input(
                &self,
                _request: &crate::platform::InputRequest,
            ) -> Result<InputOutcome, crate::error::TendrilError> {
                unreachable!(
                    "execute_input should never be reached when the adapter has no Wayland input backend"
                )
            }
        }

        impl PermissionAdapter for WaylandMissingBackendsAdapter {
            fn permissions(&self) -> Vec<crate::platform::PermissionStatus> {
                Vec::new()
            }
        }

        impl AudioCapabilityProbe for WaylandMissingBackendsAdapter {
            fn probe_audio_capture(
                &self,
                request: &AudioProbeRequest,
            ) -> Result<AudioCapabilityReport, PlatformAdapterError> {
                Ok(AudioCapabilityReport {
                    source: request.source,
                    backend: AudioBackend::PipeWire,
                    supported_sample_rates_hz: vec![48_000],
                    supported_channel_counts: vec![2],
                    permissions: Vec::new(),
                    notes: Vec::new(),
                })
            }
        }

        impl PlatformAdapter for WaylandMissingBackendsAdapter {
            fn info(&self) -> AdapterInfo {
                AdapterInfo {
                    platform: PlatformKind::Linux,
                    session: DesktopSession::Wayland,
                    audio_backend: Some(AudioBackend::PipeWire),
                    stateless: true,
                }
            }
        }

        let adapter = WaylandMissingBackendsAdapter;
        let error = execute_run(
            &RunInput {
                target: TargetSelector::Display {
                    id: "display-1".to_owned(),
                },
                payload: crate::model::RunInputPayload::Text {
                    text: "hello".to_owned(),
                },
                restore_focus: true,
            },
            &adapter,
        )
        .expect_err("run should fail when the Wayland adapter has no helper backend");

        assert_eq!(
            error.code(),
            "unsupported_capability",
            "missing-backend should surface the adapter-level capability error, not the generic per-target one"
        );
        assert_ne!(
            error.code(),
            "input_not_supported_for_target",
            "per-target capability check must not mask the actionable Wayland missing-backend diagnostic"
        );
        let message = error.to_string();
        assert!(
            message.contains("ydotool") && message.contains("wtype"),
            "diagnostic must name both Wayland helpers, got: {message}"
        );
        let details = error
            .details()
            .expect("unsupported_capability error should carry structured details");
        let suggestion = details
            .get("suggested_action")
            .and_then(serde_json::Value::as_str)
            .expect("missing-backend diagnostic should suggest an install path");
        assert!(
            suggestion.contains("ydotool") || suggestion.contains("wtype"),
            "suggested_action should mention the helper tools to install, got: {suggestion}"
        );
    }

    #[test]
    fn cli_listen_json_error_is_structured_for_unimplemented_device_selection() {
        let output = dispatch_listen_command(
            &ListenCommand {
                source: Some("device:mic-2".to_string()),
                duration_ms: Some(1_000),
                format: Some("wav".to_string()),
                output: None,
            },
            true,
            &AdapterContext::windows11(),
        )
        .expect_err("device-specific selection should remain a documented gap");

        assert_eq!(
            output.category(),
            mcp_cli::ErrorCategory::UnsupportedCapability
        );
        assert_eq!(output.code(), "audio_device_selection_not_implemented");
        assert_eq!(
            output.details().expect("structured details")["request"]["source"]["id"],
            "mic-2"
        );
    }

    #[test]
    fn capture_input_rejects_conflicting_target_flags() {
        let error = build_capture_input(
            &TargetScope {
                window: Some("window-1".to_owned()),
                display: Some("1".to_owned()),
                camera: None,
                x11_display: None,
            },
            &CaptureCommand::default(),
            &TendrilConfig::default(),
        )
        .expect_err("capture target selection should reject conflicting flags");

        assert_eq!(error.code(), "invalid_capture_input");
        assert_eq!(
            error.details().expect("structured details")["field"],
            "target"
        );
    }

    #[test]
    fn cli_capture_command_variant_still_builds() {
        let cli = TendrilCli {
            json: false,
            window: Some("window-1".into()),
            display: None,
            camera: None,
            remote: None,
            wsl_tunnel: false,
            android: None,
            command: Some(Command::Capture(CaptureCommand::default())),
        };
        assert!(matches!(cli.command, Some(Command::Capture(_))));
    }

    #[test]
    fn write_capture_to_file_creates_image_on_disk() {
        use base64::Engine as _;
        use base64::engine::general_purpose::STANDARD as BASE64;

        let dir = tempfile::tempdir().expect("temp dir should create");
        let path = dir.path().join("screenshot.png");
        let image_bytes = sample_png_bytes();
        let image_base64 = BASE64.encode(&image_bytes);

        super::write_capture_to_file(&image_base64, &path).expect("write should succeed");

        let written = std::fs::read(&path).expect("file should be readable");
        assert_eq!(written, image_bytes);
    }

    #[test]
    fn write_capture_to_file_creates_parent_directories() {
        use base64::Engine as _;
        use base64::engine::general_purpose::STANDARD as BASE64;

        let dir = tempfile::tempdir().expect("temp dir should create");
        let path = dir.path().join("sub").join("dir").join("capture.png");
        let image_bytes = sample_png_bytes();
        let image_base64 = BASE64.encode(&image_bytes);

        super::write_capture_to_file(&image_base64, &path).expect("write should succeed");

        let written = std::fs::read(&path).expect("file should be readable");
        assert_eq!(written, image_bytes);
    }

    #[test]
    fn capture_command_output_field_excluded_from_json_schema() {
        let schema = serde_json::to_value(schemars::schema_for!(CaptureRequest))
            .expect("capture schema should serialize");

        assert!(
            schema["properties"].get("output").is_none(),
            "output should not appear in the MCP schema"
        );
    }
}
