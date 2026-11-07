mod cli;
mod commands;
mod config;
mod manifest;
mod platform;
mod util;

use anyhow::Result;
use clap::Parser;

fn main() {
    if let Err(error) = run() {
        eprintln!("binage: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::Init { path } => commands::init::run(path),
        cli::Command::Destruct => commands::destruct::run(),
        cli::Command::Add { source, entries } => commands::add::run(source, entries),
        cli::Command::Remove { entry } => commands::remove::run(&entry),
        cli::Command::Ls => commands::ls::run(),
    }
}
