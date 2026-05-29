mod checker;
mod cli;
mod crawler;
mod extractor;
mod models;
mod reporter;

#[tokio::main]
async fn main() {
    if let Err(e) = cli::run().await {
        eprintln!("{}: {}", "Error".red(), e);
        std::process::exit(2);
    }
}

use colored::Colorize;
