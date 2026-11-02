use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "binage", version, about = "A tiny portable-binary manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize binage and add its bin directory to PATH once.
    Init {
        /// Installation root (defaults to the platform-specific user directory).
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Install a single binary, or a directory with one or more entry files.
    Add {
        /// Source file or directory.
        source: PathBuf,
        /// Executable files relative to SOURCE (required when SOURCE is a directory).
        #[arg(short = 'e', long = "entry", num_args = 1.., value_name = "FILE")]
        entries: Vec<PathBuf>,
    },
    /// Uninstall an installed entry by its command name.
    Remove {
        entry: String,
    },
    /// List installed command entries.
    Ls,
}

