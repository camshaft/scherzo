use anyhow::Result;
use clap::{Parser, Subcommand};

mod cli;
mod config;
mod server;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Compile(args) => args.run(),
        Command::Start(args) => args.run(),
    }
}

#[derive(Parser)]
#[command(name = "scherzo", about = "G-code tooling for Scherzo")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile a G-code job into WIT, core wasm, and a component.
    Compile(cli::compile::CompileArgs),
    /// Start the Scherzo runtime with the specified configuration.
    Start(cli::start::StartArgs),
}
