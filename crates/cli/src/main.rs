use clap::{Parser, Subcommand};

/// Control plane for coding agents.
#[derive(Parser)]
#[command(
    name = "workengine",
    version,
    about = "Control plane for coding agents"
)]
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the Workengine version
    Version,
}

fn main() {
    match Cli::parse().command {
        Command::Version => {
            println!("workengine {}", env!("CARGO_PKG_VERSION"));
        }
    }
}
