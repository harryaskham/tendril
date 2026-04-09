use std::io;

use mcp_cli::{McpServer, StdioServerConfig, ToolRouter, write_json_result_ref};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::{CaptureCommand, Command, ListCommand, McpSubcommand, RunCommand, TendrilCli};
use crate::error::TendrilError;

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

pub fn dispatch(cli: &TendrilCli) -> Result<(), TendrilError> {
    match &cli.command {
        None => {
            print!("{}", TendrilCli::agent_help());
            Ok(())
        }
        Some(Command::Mcp(command)) => dispatch_mcp(command),
        Some(command) => dispatch_cli_command(cli, command),
    }
}

fn dispatch_mcp(command: &crate::cli::McpCommand) -> Result<(), TendrilError> {
    match command.command {
        McpSubcommand::Stdio => build_mcp_server()
            .serve_stdio(&())
            .map_err(|error| TendrilError::mcp(&error)),
    }
}

fn dispatch_cli_command(cli: &TendrilCli, command: &Command) -> Result<(), TendrilError> {
    let result = execute_command(cli, command);

    if cli.json {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        write_json_result_ref(&mut stdout, &result).map_err(|error| TendrilError::mcp(&error))?;
    }

    result.map(|_| ())
}

fn execute_command(cli: &TendrilCli, command: &Command) -> Result<Value, TendrilError> {
    match command {
        Command::List(list) => handle_list(list),
        Command::Capture(options) => handle_capture(&CaptureRequest {
            target: target_scope_from_cli(cli),
            options: options.clone(),
        }),
        Command::Run(options) => handle_run(&RunRequest {
            target: target_scope_from_cli(cli),
            options: options.clone(),
        }),
        Command::Listen(_) => Err(TendrilError::not_implemented("listen")),
        Command::Alias(_) => Err(TendrilError::not_implemented("alias")),
        Command::Mcp(_) => unreachable!("MCP commands are dispatched separately"),
    }
}

fn target_scope_from_cli(cli: &TendrilCli) -> TargetScope {
    TargetScope {
        window: cli.window.clone(),
        display: cli.display.clone(),
    }
}

fn handle_list(_command: &ListCommand) -> Result<Value, TendrilError> {
    Err(TendrilError::not_implemented("list"))
}

fn handle_capture(_command: &CaptureRequest) -> Result<Value, TendrilError> {
    Err(TendrilError::not_implemented("capture"))
}

fn handle_run(_command: &RunRequest) -> Result<Value, TendrilError> {
    Err(TendrilError::not_implemented("run"))
}

fn build_mcp_server() -> McpServer<()> {
    McpServer::new(
        StdioServerConfig {
            server_name: "tendril".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        build_tool_router(),
    )
}

fn build_tool_router() -> ToolRouter<()> {
    let mut router = ToolRouter::new();
    router.add_typed_tool(
        "tendril_list",
        "Discover available desktop targets.",
        |(), command: ListCommand| handle_list(&command),
    );
    router.add_typed_tool(
        "tendril_capture",
        "Capture a screenshot from a display or window target.",
        |(), command: CaptureRequest| handle_capture(&command),
    );
    router.add_typed_tool(
        "tendril_run",
        "Execute input against a specific target.",
        |(), command: RunRequest| handle_run(&command),
    );
    router
}

#[cfg(test)]
mod tests {
    use super::{CaptureRequest, RunRequest, TargetScope};
    use crate::cli::{CaptureCommand, RunCommand};

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
}
