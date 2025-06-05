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

    /// Target Python version (e.g., py38, py39, py310, py311, py312, py313)
    #[arg(long, alias = "python-version")]
    target_version: Option<String>,
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
    let mut config = Config::load(cli.config.as_deref())?;

    // Override target-version from CLI if provided
    if let Some(target_version) = cli.target_version {
        config.set_target_version(target_version)?;
    }

    debug!("Configuration: {:?}", config);

    // Display target version for troubleshooting
    info!(
        "Target Python version: {} (resolved to Python 3.{})",
        config.target_version,
        config.python_version().unwrap_or(10)
    );

    // Create bundler and run
    let mut bundler = Bundler::new(config);
    bundler.bundle(&cli.entry, &cli.output, cli.emit_requirements)?;

    info!("Bundle created successfully at {:?}", cli.output);

    Ok(())
}
