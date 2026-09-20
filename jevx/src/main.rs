mod cli;

use clap::Parser;

#[tokio::main]
async fn main() {
    std::process::exit(cli::run_with_cli(cli::Cli::parse()).await);
}
