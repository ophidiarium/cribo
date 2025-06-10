use clap::Parser;
use env_logger::Env;
use log::{debug, info};
use std::path::PathBuf;

use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Entry point Python script
    #[arg(short, long)]
    entry: PathBuf,

    /// Output bundled Python file
    #[arg(short, long, conflicts_with = "stdout")]
    output: Option<PathBuf>,

    /// Output bundled code to stdout instead of a file
    #[arg(long, conflicts_with = "output")]
    stdout: bool,

    /// Increase verbosity (can be repeated: -v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

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

    // Initialize logging based on verbosity level
    let log_level = match cli.verbose {
        0 => "warn",  // Default: warnings and errors only
        1 => "info",  // -v: informational messages
        2 => "debug", // -vv: debug messages
        _ => "trace", // -vvv or more: trace messages
    };
    env_logger::Builder::from_env(Env::default().default_filter_or(log_level)).init();

    debug!(
        "Verbosity level: {} (log level: {})",
        cli.verbose, log_level
    );
    info!("Starting Cribo Python bundler");

    debug!("Entry point: {:?}", cli.entry);
    if cli.stdout {
        debug!("Output mode: stdout");
    } else {
        debug!("Output: {:?}", cli.output);
    }

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

    // Validate arguments
    if !cli.stdout && cli.output.is_none() {
        return Err(anyhow::anyhow!(
            "Either --output or --stdout must be specified"
        ));
    }

    // Create bundler and run
    let mut bundler = BundleOrchestrator::new(config);

    if cli.stdout {
        // Output to stdout
        let bundled_code = bundler.bundle_to_string(&cli.entry, cli.emit_requirements)?;
        print!("{}", bundled_code);
        info!("Bundle output to stdout");
    } else {
        // Output to file
        let output_path = cli
            .output
            .as_ref()
            .expect("Output path should be present when not using stdout");
        bundler.bundle(&cli.entry, output_path, cli.emit_requirements)?;
        info!("Bundle created successfully at {:?}", output_path);
    }

    Ok(())
}
