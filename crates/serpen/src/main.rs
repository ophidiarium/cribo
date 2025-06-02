use clap::Parser;
use env_logger::Env;
use log::{debug, info};
use std::path::PathBuf;

use serpen::bundler::Bundler;
use serpen::config::Config;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Entry point Python script
    #[arg(short, long)]
    entry: PathBuf,

    /// Output bundled Python file
    #[arg(short, long)]
    output: PathBuf,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Configuration file path
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Emit requirements.txt with third-party dependencies
    #[arg(long)]
    emit_requirements: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(log_level)).init();

    info!("Starting Serpen Python bundler");

    debug!("Entry point: {:?}", cli.entry);
    debug!("Output: {:?}", cli.output);

    // Load configuration
    let config = Config::load(cli.config.as_deref())?;
    debug!("Configuration: {:?}", config);

    // Create bundler and run
    let mut bundler = Bundler::new(config);
    bundler.bundle(&cli.entry, &cli.output, cli.emit_requirements)?;

    info!("Bundle created successfully at {:?}", cli.output);

    Ok(())
}
