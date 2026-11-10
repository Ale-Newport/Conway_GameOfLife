//! `emergence-cli` — headless experiments with every Emergence Lab model:
//! list models and scenarios, describe them, run, compare replicates,
//! validate determinism, benchmark and convert pattern files.
//!
//! Exit status: 0 success, 1 error, 2 invalid command line (clap), 3 a
//! validation check failed. A closed standard output (`| head`) is not an
//! error: the rest of the output is dropped and the command completes.

mod cli;
mod commands;
mod context;
mod headless;
mod machine;
mod output;
mod text;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, ListCommand};

/// Exit status when a validation check fails.
const VALIDATION_FAILED: u8 = 3;

fn dispatch(cli: &Cli) -> anyhow::Result<ExitCode> {
    if let Command::Convert(args) = &cli.command {
        commands::convert::convert(args)?;
        return Ok(ExitCode::SUCCESS);
    }
    let registry = context::registry(cli)?;
    match &cli.command {
        Command::List(ListCommand::Models { json }) => commands::list::models(&registry, *json)?,
        Command::List(ListCommand::Scenarios { model, json }) => {
            commands::list::scenarios(&registry, model.as_deref(), *json)?;
        }
        Command::Info(args) => commands::info::info(&registry, &args.target, args.json)?,
        Command::Run(args) => commands::run::run(&registry, args)?,
        Command::Compare(args) => commands::compare::compare(&registry, args)?,
        Command::Validate(args) => {
            if !commands::validate::validate(&registry, args)? {
                return Ok(ExitCode::from(VALIDATION_FAILED));
            }
        }
        Command::Bench(args) => commands::bench::bench(&registry, args)?,
        Command::Convert(_) => unreachable!("handled above"),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(&cli) {
        Ok(_) if output::stdout_error().is_some() => {
            eprintln!("error: cannot write to standard output: {}", output::stdout_error().unwrap_or_default());
            ExitCode::FAILURE
        }
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
