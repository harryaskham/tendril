use crate::cli::{Command, TendrilCli};
use crate::error::TendrilError;

pub fn dispatch(cli: &TendrilCli) -> Result<(), TendrilError> {
    match &cli.command {
        None => {
            print!("{}", TendrilCli::agent_help());
            Ok(())
        }
        Some(command) => dispatch_command(command),
    }
}

fn dispatch_command(command: &Command) -> Result<(), TendrilError> {
    Err(TendrilError::not_implemented(command.name()))
}
