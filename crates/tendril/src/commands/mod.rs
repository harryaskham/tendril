use mcp_cli::{JsonEnvelope, McpServer, StdioServerConfig, ToolRouter};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::info;

use crate::cli::{
    AliasCommand, CaptureCommand, Command, ListCommand, ListenCommand, McpSubcommand, RunCommand,
    TendrilCli, WORKFLOW_HINT,
};
use crate::config::TendrilConfig;
use crate::error::TendrilError;
use crate::model::{
    AliasInput, AudioSourceKind, AudioSourceSelector, CaptureInput, ListInput, ListenInput,
    RunInput, RunInputPayload, ShellKind, TargetSelector,
};

#[derive(Debug, Clone)]
struct CommandContext {
    config: TendrilConfig,
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
    if json_mode {
        let envelope = JsonEnvelope::success_for(
            "help",
            json!({
                "help": TendrilCli::agent_help(),
                "workflow": WORKFLOW_HINT,
            }),
        );
        CommandOutput::Json(serde_json::to_value(envelope).expect("json help should serialize"))
    } else {
        CommandOutput::Human(TendrilCli::agent_help())
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
            validate_list_command(command)?;
            info!(command = "list", "validated list command request");
            Err(TendrilError::not_implemented("list"))
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
            Err(TendrilError::not_implemented("capture"))
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
            Err(TendrilError::not_implemented("run"))
        }
        Command::Listen(command) => {
            let input = build_listen_input(command)?;
            info!(
                command = "listen",
                source_kind = ?input.source.kind,
                duration_ms = input.duration_ms,
                "validated listen request"
            );
            Err(TendrilError::not_implemented("listen"))
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
            Err(TendrilError::not_implemented("alias"))
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
        "tendril_list",
        "Discover available desktop targets.",
        |_: &CommandContext, command: ListCommand| {
            validate_list_command(&command)?;
            Err::<Value, TendrilError>(TendrilError::not_implemented("list"))
        },
    );
    router.add_typed_tool(
        "tendril_capture",
        "Capture a screenshot from a display or window target.",
        |context: &CommandContext, command: CaptureRequest| {
            let _input = build_capture_input(&command.target, &command.options, &context.config)?;
            Err::<Value, TendrilError>(TendrilError::not_implemented("capture"))
        },
    );
    router.add_typed_tool(
        "tendril_run",
        "Execute input against a specific target.",
        |_: &CommandContext, command: RunRequest| {
            let _input = build_run_input(&command.target, &command.options)?;
            Err::<Value, TendrilError>(TendrilError::not_implemented("run"))
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

fn build_run_input(target: &TargetScope, command: &RunCommand) -> Result<RunInput, TendrilError> {
    let input_definition = command.input_definition.clone().ok_or_else(|| {
        TendrilError::validation("run requires an input definition")
            .with_code("invalid_run_input")
            .with_field("input_definition")
    })?;
    let input = RunInput {
        target: required_target(target, "run")?,
        payload: if looks_like_dsl(&input_definition) {
            RunInputPayload::Dsl {
                sequence: input_definition,
            }
        } else {
            RunInputPayload::Text {
                text: input_definition,
            }
        },
    };
    input.validate()?;
    Ok(input)
}

fn build_listen_input(command: &ListenCommand) -> Result<ListenInput, TendrilError> {
    let input = ListenInput {
        source: match command.source.as_deref() {
            None | Some("system") => AudioSourceSelector {
                kind: AudioSourceKind::System,
                id: None,
            },
            Some("microphone") => AudioSourceSelector {
                kind: AudioSourceKind::Microphone,
                id: None,
            },
            Some(value) => {
                if let Some(id) = value.strip_prefix("device:") {
                    AudioSourceSelector {
                        kind: AudioSourceKind::Device,
                        id: Some(id.to_owned()),
                    }
                } else {
                    return Err(TendrilError::validation(format!(
                        "unsupported audio source `{value}`; use `system`, `microphone`, or `device:<id>`"
                    ))
                    .with_code("invalid_listen_input")
                    .with_field("source"));
                }
            }
        },
        ..ListenInput::default()
    };
    input.validate()?;
    Ok(input)
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
        name: default_alias_name(target),
    };
    input.validate()?;
    Ok(input)
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
    selected_target(target)?.ok_or_else(|| {
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

fn parse_shell(value: &str) -> Result<ShellKind, TendrilError> {
    match value {
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

fn payload_kind(payload: &RunInputPayload) -> &'static str {
    match payload {
        RunInputPayload::Text { .. } => "text",
        RunInputPayload::Dsl { .. } => "dsl",
        RunInputPayload::Actions { .. } => "actions",
    }
}

fn looks_like_dsl(input_definition: &str) -> bool {
    input_definition.contains('(') || input_definition.contains(',')
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
    use super::{
        CaptureRequest, RunRequest, TargetScope, build_capture_input, build_mcp_server, dispatch,
    };
    use crate::cli::{CaptureCommand, Command, RunCommand, TendrilCli};
    use crate::config::{ImageFormat, TendrilConfig};
    use serde_json::json;

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
                        .contains("tendril list")
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
    fn mcp_server_lists_core_tools() {
        let server = build_mcp_server();
        let names: Vec<_> = server
            .tool_metadata()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            names,
            vec!["tendril_list", "tendril_capture", "tendril_run"]
        );
    }

    #[test]
    fn capture_request_flattens_capture_command_fields() {
        let request = CaptureRequest {
            target: TargetScope {
                window: None,
                display: Some("display-1".to_string()),
            },
            options: CaptureCommand {
                max_width: Some(800),
                max_height: Some(600),
                format: Some("png".to_string()),
                compression: Some(90),
            },
        };

        assert_eq!(request.target.display.as_deref(), Some("display-1"));
        assert_eq!(request.options.max_width, Some(800));
        assert_eq!(request.options.compression, Some(90));
    }

    #[test]
    fn mcp_tool_call_returns_structured_error_envelope() {
        let response = build_mcp_server()
            .handle_request_value(
                &super::CommandContext {
                    config: TendrilConfig::default(),
                },
                json!({
                    "jsonrpc": "2.0",
                    "id": 7,
                    "method": "tools/call",
                    "params": {
                        "name": "tendril_capture",
                        "arguments": { "window": "window-1" }
                    }
                }),
            )
            .expect("request should parse")
            .expect("response should exist");

        assert_eq!(response["result"]["isError"], true);
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
