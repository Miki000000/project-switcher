mod cache;
mod catalog;
mod cli;
mod clone;
mod commands;
mod config;
mod discovery;
mod giturl;
mod model;
mod providers;
mod shell;
mod tui;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};
use config::Config;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("proj: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        None => tui::run(Config::load()?, tui::View::Browse),
        Some(Command::Add(args)) => match &args.repository {
            None => tui::run(Config::load()?, tui::View::Add),
            Some(repo) => commands::add(&Config::load()?, &args, repo),
        },
        Some(Command::List(args)) => commands::list(&Config::load()?, &args),
        Some(Command::Refresh) => commands::refresh(&Config::load()?),
        Some(Command::Status { dirty }) => commands::status(&Config::load()?, dirty),
        Some(Command::Config { action }) => commands::config(action),
        Some(Command::Init { shell, cmd }) => {
            print!("{}", shell::init_script(shell, &cmd));
            Ok(())
        }
    }
}
