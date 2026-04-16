use std::fmt::Write as _;
use std::sync::Arc;

use mcp_cli::{JsonEnvelope, McpServer, StdioServerConfig, ToolRouter};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::info;

use crate::capture::{execute_capture, render_capture_human};
use crate::cli::{
    AliasCommand, CaptureCommand, Command, ListCommand, ListenCommand, McpSubcommand, RunCommand,
    TendrilCli, WORKFLOW_HINT,
};
use crate::config::TendrilConfig;
use crate::error::TendrilError;
use crate::input::{execute_run, parse_input_definition, render_run_human};
use crate::model::{
    AliasInput, AliasOutput, AudioFormat, AudioSourceKind, AudioSourceSelector, CapabilitySet,
    CaptureInput, ListInput, ListOutput, ListenInput, RunInput, RunInputPayload, ShellKind,
    TargetDescriptor, TargetKind, TargetSelector,
};
use crate::platform::{
    AdapterContext, AdapterInfo, AudioCapabilityReport, AudioProbeRequest,
    AudioSourceKind as PlatformAudioSourceKind, Capability, CaptureTargetKind, PlatformAdapter,
    TargetDiscoveryRequest, adapter_for_context,
};

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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TargetScope {
    pub window: Option<String>,
    pub display: Option<String>,
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
pub struct AliasRequest {
    #[serde(flatten)]
    pub target: TargetScope,
    #[serde(flatten)]
    pub options: AliasCommand,
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
        Some(Command::Mcp(command)) => dispatch_mcp(command, config),
        Some(command) => dispatch_cli_command(cli, command, config),
    }
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
    match command {
        Command::List(command) => {
            let input = validate_list_command(command)?;
            let output = execute_list(&input, &AdapterContext::detect())?;
            info!(
                command = "list",
                target_count = output.targets.len(),
                "discovered desktop targets"
            );
            Ok(render_command_output(
                "list",
                cli.json,
                output,
                render_list_human,
            ))
        }
        Command::Capture(command) => {
            let input = build_capture_input(&target_scope_from_cli(cli), command, config)?;
            info!(
                command = "capture",
                target_kind = ?input.target.kind(),
                target_id = %input.target.id(),
                format = ?input.format,
                "validated capture request"
            );
            let adapter = adapter_for_context(AdapterContext::detect());
            let output = execute_capture(&input, adapter.as_ref())?;
            Ok(render_command_output(
                "capture",
                cli.json,
                output,
                render_capture_human,
            ))
        }
        Command::Run(command) => {
            let input = build_run_input(&target_scope_from_cli(cli), command)?;
            info!(
                command = "run",
                target_kind = ?input.target.kind(),
                target_id = %input.target.id(),
                payload_kind = %payload_kind(&input.payload),
                "validated run request with redacted payload"
            );
            let adapter = adapter_for_context(AdapterContext::detect());
            let output = execute_run(&input, adapter.as_ref())?;
            Ok(render_command_output(
                "run",
                cli.json,
                output,
                render_run_human,
            ))
        }
        Command::Listen(command) => {
            dispatch_listen_command(command, cli.json, &AdapterContext::detect())
        }
        Command::Alias(command) => {
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
        Command::Mcp(_) => unreachable!("MCP commands are dispatched separately"),
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

fn build_tool_router() -> ToolRouter<CommandContext> {
    let mut router = ToolRouter::new();
    router.add_typed_tool(
        "list",
        "Discover available desktop targets.",
        |context: &CommandContext, command: ListCommand| {
            let input = validate_list_command(&command)?;
            let adapter = context.adapter();
            execute_list_with_adapter(&input, adapter.as_ref())
        },
    );
    router.add_typed_tool(
        "capture",
        "Capture a screenshot from a display or window target.",
        |context: &CommandContext, command: CaptureRequest| {
            let input = build_capture_input(&command.target, &command.options, &context.config)?;
            let adapter = context.adapter();
            capture_response_value(&input, adapter.as_ref())
        },
    );
    router.add_typed_tool(
        "run",
        "Execute input against a specific target.",
        |context: &CommandContext, command: RunRequest| {
            let input = build_run_input(&command.target, &command.options)?;
            let adapter = context.adapter();
            serde_json::to_value(execute_run(&input, adapter.as_ref())?)
                .map_err(|error| TendrilError::serialization(error.to_string()))
        },
    );
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

    Ok(ListOutput {
        adapter: adapter.info(),
        permissions: adapter.permissions(),
        targets,
    })
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
        app_name: target.app_name,
        process_id: target.process_id,
    }
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
                command: "tendril --window <id> capture --json".to_owned(),
                description: "Capture target state and keep resize metadata in JSON.".to_owned(),
            },
            HelpWorkflowStep {
                command: "tendril --window <id> run 'send(\"hello\")'".to_owned(),
                description: "Execute text or input sequences against the chosen target.".to_owned(),
            },
        ],
        commands: vec![
            HelpCommandSummary {
                name: "list".to_owned(),
                description: "Discover windows and displays.".to_owned(),
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
                name: "alias".to_owned(),
                description: "Emit a shell helper that pre-fills --window or --display.".to_owned(),
            },
            HelpCommandSummary {
                name: "listen".to_owned(),
                description: "Probe supported audio capture paths.".to_owned(),
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
                description: "Capture a chosen target".to_owned(),
                command: "tendril --window <id> capture --json".to_owned(),
            },
            HelpExample {
                description: "Create a reusable wrapper for repeated targeting".to_owned(),
                command: "eval \"$(tendril --window <id> alias --name desk)\"".to_owned(),
            },
        ],
        notes: vec![
            "Use --json for machine-readable success and error envelopes.".to_owned(),
            "Alias helpers are plain shell wrappers around explicit tendril arguments; Tendril does not store session state.".to_owned(),
        ],
    }
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
        let _ = writeln!(
            rendered,
            "- {:?} {} {} {}x{}+{}+{} scale={}/{} {}{}{}",
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
        );
    }

    rendered
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
    };
    input.validate()?;
    Ok(input)
}

fn capture_response_value(
    input: &CaptureInput,
    adapter: &dyn PlatformAdapter,
) -> Result<Value, TendrilError> {
    serde_json::to_value(execute_capture(input, adapter)?)
        .map_err(|error| TendrilError::serialization(error.to_string()))
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
    };
    input.validate()?;
    Ok(input)
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
    let response = build_listen_response(&input, adapter_context)?;
    Ok(render_listen_output(&response, json_mode))
}

fn build_listen_response(
    input: &ListenInput,
    adapter_context: &AdapterContext,
) -> Result<ListenResponse, TendrilError> {
    let adapter = adapter_for_context(adapter_context.clone());
    let adapter_info = adapter.info();
    let capability = probe_listen_capability(input, adapter.as_ref())?;

    Ok(ListenResponse {
        request: input.clone(),
        adapter: adapter_info,
        capability,
        execution: ListenExecutionStatus {
            status: "probe_only".to_owned(),
            artifact_available: false,
            notes: vec![
                "The v0.0.1 listen surface reports command-scoped capability and permission diagnostics but does not yet emit an audio artifact.".to_owned(),
                "Use the returned backend, permissions, and channel metadata to decide whether a follow-up capture path is viable on this platform/session.".to_owned(),
            ],
        },
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
        CommandOutput::Human(format!(
            "listen source: {:?}\nformat: {:?}\nduration_ms: {}\nplatform: {:?}\nsession: {:?}\naudio_backend: {:?}\nsupported_sample_rates_hz: {:?}\nsupported_channel_counts: {:?}\npermissions:\n{}\nstatus: {}\nnotes: {}\n",
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
        AliasRequest, CaptureRequest, RunRequest, TargetScope, build_alias_input,
        build_capture_input, build_listen_input, build_listen_response, build_mcp_server, dispatch,
        dispatch_listen_command, execute_alias, execute_list_with_adapter, render_command_output,
        render_list_human,
    };
    use crate::capture::{execute_capture, render_capture_human};
    use crate::cli::{
        AliasCommand, CaptureCommand, Command, ListCommand, ListenCommand, RunCommand, TendrilCli,
        WORKFLOW_HINT,
    };
    use crate::config::{ImageFormat, TendrilConfig};
    use crate::input::{execute_run, render_run_human};
    use crate::model::{
        AudioFormat, AudioSourceKind, Bounds, CaptureInput, RunInput, ShellKind, TargetSelector,
    };
    use crate::platform::{
        AdapterContext, AdapterInfo, AudioBackend, AudioCapabilityProbe, AudioCapabilityReport,
        AudioProbeRequest, CaptureAdapter, CaptureArtifact,
        CaptureRequest as PlatformCaptureRequest, CaptureTargetKind, DesktopSession,
        FeatureSupport, InputControlAdapter, InputOutcome, PermissionAdapter, PlatformAdapter,
        PlatformAdapterError, PlatformKind, TargetDescriptor as PlatformTargetDescriptor,
        TargetDiscoveryAdapter, TargetDiscoveryRequest, TargetInventory,
    };

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

    #[test]
    fn json_help_dispatch_returns_machine_readable_envelope() {
        let cli = TendrilCli {
            json: true,
            window: None,
            display: None,
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
                    "tendril --window <id> capture --json"
                );
                assert_eq!(
                    value["data"]["workflow_steps"][2]["command"],
                    "tendril --window <id> run 'send(\"hello\")'"
                );
            }
            _ => panic!("expected json output"),
        }
    }

    #[test]
    fn capture_input_uses_config_defaults() {
        let target = TargetScope {
            window: Some("window-1".into()),
            display: None,
        };
        let input = build_capture_input(
            &target,
            &CaptureCommand {
                max_width: None,
                max_height: None,
                format: None,
                compression: None,
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
            },
            options: RunCommand {
                input_definition: Some("send(\"hello\")".to_string()),
            },
        };

        assert_eq!(request.target.window.as_deref(), Some("window-1"));
        assert_eq!(
            request.options.input_definition.as_deref(),
            Some("send(\"hello\")")
        );
    }

    #[test]
    fn mcp_server_lists_initial_stdio_tools() {
        let server = build_mcp_server();
        let names: Vec<_> = server
            .tool_metadata()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(names, vec!["list", "capture", "run"]);
    }

    #[test]
    fn mcp_tool_schemas_match_effective_cli_inputs() {
        let tools = build_mcp_server().tool_metadata();
        let list = tools
            .iter()
            .find(|tool| tool.name == "list")
            .expect("list tool should be registered");
        let capture = tools
            .iter()
            .find(|tool| tool.name == "capture")
            .expect("capture tool should be registered");
        let run = tools
            .iter()
            .find(|tool| tool.name == "run")
            .expect("run tool should be registered");

        assert_eq!(
            list.input_schema,
            serde_json::to_value(schemars::schema_for!(ListCommand))
                .expect("list schema should serialize")
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
    }

    #[test]
    fn listen_input_parses_requested_source_duration_and_format() {
        let input = build_listen_input(&ListenCommand {
            source: Some("loopback".to_string()),
            duration_ms: Some(1_500),
            format: Some("flac".to_string()),
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
    fn listen_probe_reports_probe_only_gap_in_success_payload() {
        let response = build_listen_response(
            &build_listen_input(&ListenCommand {
                source: Some("system".to_string()),
                duration_ms: Some(2_000),
                format: Some("wav".to_string()),
            })
            .expect("listen input should build"),
            &AdapterContext::windows11(),
        )
        .expect("windows loopback probe should succeed");

        assert_eq!(response.capability.backend, AudioBackend::Wasapi);
        assert_eq!(response.execution.status, "probe_only");
        assert!(!response.execution.artifact_available);
        assert!(response.execution.notes[0].contains("does not yet emit an audio artifact"));
    }

    #[test]
    fn capture_request_flattens_capture_command_fields() {
        let request = CaptureRequest {
            target: TargetScope {
                window: None,
                display: Some("1".to_string()),
            },
            options: CaptureCommand {
                max_width: Some(800),
                max_height: Some(600),
                format: Some("png".to_string()),
                compression: Some(90),
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
        let response = tool_call_response(
            &context,
            "run",
            &json!({
                "window": "window-1",
                "input_definition": "send(\"hello\")"
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
            },
            adapter.as_ref(),
        )
        .expect("run output should build");
        let cli_json = expect_json_output(render_command_output(
            "run",
            true,
            cli_output,
            render_run_human,
        ));

        assert_eq!(response["result"]["isError"], false);
        assert_eq!(mcp_structured_content(&response), cli_json);
    }

    #[test]
    fn mcp_capture_error_payload_matches_cli_json() {
        let cli_error = dispatch(
            &TendrilCli {
                json: true,
                window: Some("window-1".to_owned()),
                display: Some("1".to_owned()),
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
                command: Some(Command::Run(RunCommand {
                    input_definition: None,
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

    #[test]
    fn cli_listen_json_error_is_structured_for_unimplemented_device_selection() {
        let output = dispatch_listen_command(
            &ListenCommand {
                source: Some("device:mic-2".to_string()),
                duration_ms: Some(1_000),
                format: Some("wav".to_string()),
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
            command: Some(Command::Capture(CaptureCommand::default())),
        };
        assert!(matches!(cli.command, Some(Command::Capture(_))));
    }
}
