use std::path::PathBuf;

use anyhow::anyhow;
use clap::{Parser, Subcommand};
use env_logger::Env;
use log::{debug, info};

// Module declarations - keeping only what's needed for the binary
mod analyzers;
mod ast_builder;
mod ast_indexer;
mod code_generator;
mod combine;
mod config;
mod dependency_graph;
mod deps;
mod dirs;
mod graph_builder;
mod import_alias_tracker;
mod import_rewriter;
mod module_facts;
mod orchestrator;
mod python;
mod requirement_resolver;
mod resolver;
mod side_effects;
mod source_map;
mod symbol_conflict_resolver;
mod transformation_context;
mod tree_shaking;
mod types;
mod util;
mod visitors;

use config::{Config, SourceMapMode};
use orchestrator::BundleOrchestrator;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Entry point Python script (required unless a subcommand is used)
    #[arg(short, long)]
    entry: Option<PathBuf>,

    /// Output bundled Python file
    #[arg(short, long, conflicts_with = "stdout")]
    output: Option<PathBuf>,

    /// Output bundled code to stdout instead of a file
    #[arg(long, conflicts_with = "output")]
    stdout: bool,

    /// Increase verbosity (can be repeated: -v, -vv, -vvv)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Configuration file path
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Emit requirements.txt with third-party dependencies
    #[arg(long)]
    emit_requirements: bool,

    /// Target Python version (e.g., py38, py39, py310, py311, py312, py313)
    #[arg(long, global = true, alias = "python-version")]
    target_version: Option<String>,

    /// Python interpreter whose installed distributions should be inspected
    #[arg(long, global = true)]
    python: Option<PathBuf>,

    /// Disable tree-shaking optimization (tree-shaking is enabled by default)
    #[arg(long = "no-tree-shake", global = true, default_value_t = true, action = clap::ArgAction::SetFalse)]
    tree_shake: bool,

    /// Bundle third-party (site-packages) dependencies into the output.
    /// Packages with native extensions (.so/.pyd) automatically stay external
    /// and are emitted into requirements.txt
    #[arg(long)]
    bundle_third_party: bool,

    /// Generate a Source Map v3 for the bundle. A bare `--sourcemap` selects
    /// `linked` (`inline` with --stdout); or choose explicitly with
    /// `--sourcemap=linked|inline|external`
    // Option<Option<T>> is clap's idiom for a flag with an optional value:
    // the outer Option is flag presence, the inner one the explicit value.
    #[expect(clippy::option_option)]
    #[arg(long, value_enum, num_args = 0..=1, require_equals = true)]
    sourcemap: Option<Option<SourceMapMode>>,

    /// Force embedding original sources in the map as `sourcesContent`
    /// (default: omitted for inline, included for linked/external)
    #[arg(long, require_equals = true, value_name = "BOOL")]
    sources_content: Option<bool>,
}

#[derive(Subcommand)]
enum Command {
    /// Detect third-party dependencies of a Python file or directory and emit
    /// them as requirements (to stdout by default)
    Deps(DepsArgs),
}

/// Output format for the `deps` subcommand
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum DepsFormat {
    /// requirements.txt lines (one PEP 508 requirement per line)
    Requirements,
    /// Structured JSON with per-import detail
    Json,
}

#[derive(clap::Args)]
struct DepsArgs {
    /// Entry point: a Python file, a package directory, or a directory of sources
    #[arg(short, long)]
    entry: PathBuf,

    /// Write the report to a file instead of stdout
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output format
    #[arg(long, value_enum, default_value_t = DepsFormat::Requirements)]
    format: DepsFormat,

    /// Exclude imports used only within `if TYPE_CHECKING:` blocks
    #[arg(long)]
    exclude_type_checking: bool,

    /// Exclude imports that appear only inside conditional control flow
    /// (if/elif/else, try/except, loops); `TYPE_CHECKING` blocks are governed
    /// by --exclude-type-checking instead
    #[arg(long)]
    exclude_conditional: bool,
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

    // Load configuration
    let mut config = Config::load(cli.config.as_deref())?;

    // Override target-version from CLI if provided
    if let Some(target_version) = cli.target_version.clone() {
        config.set_target_version(target_version)?;
    }

    if let Some(python) = cli.python.clone() {
        config.requirements.python = Some(python);
    }

    // Override tree-shake from CLI
    config.tree_shake = cli.tree_shake;

    match cli.command {
        Some(Command::Deps(ref deps_args)) => run_deps(config, deps_args),
        None => run_bundle(config, &cli),
    }
}

/// Run the `deps` subcommand: detect third-party dependencies and emit the report.
fn run_deps(config: Config, args: &DepsArgs) -> anyhow::Result<()> {
    info!("Starting Cribo dependency detection");
    debug!("Entry point: {}", args.entry.display());
    debug!("Configuration: {config:?}");

    let options = deps::DepsOptions {
        exclude_type_checking: args.exclude_type_checking,
        exclude_conditional: args.exclude_conditional,
    };

    let mut orchestrator = BundleOrchestrator::new(config);
    let report = orchestrator.analyze_deps(&args.entry, options)?;

    let content = match args.format {
        DepsFormat::Requirements => report.to_requirements_txt(),
        DepsFormat::Json => report.to_json()?,
    };

    if let Some(output_path) = &args.output {
        std::fs::write(output_path, &content)
            .map_err(|e| anyhow!("Failed to write {}: {e}", output_path.display()))?;
        info!("Dependency report written to: {}", output_path.display());
    } else {
        use std::io::Write;
        std::io::stdout()
            .write_all(content.as_bytes())
            .map_err(|e| anyhow!("Failed to write dependency report to stdout: {e}"))?;
    }

    if report.requirements.is_empty() {
        info!("No third-party dependencies found");
    }

    Ok(())
}

/// Run the default bundling flow.
fn run_bundle(mut config: Config, cli: &Cli) -> anyhow::Result<()> {
    info!("Starting Cribo Python bundler");

    let Some(entry) = cli.entry.as_ref() else {
        return Err(anyhow!("--entry is required"));
    };

    debug!("Entry point: {}", entry.display());
    if cli.stdout {
        debug!("Output mode: stdout");
    } else {
        debug!("Output: {:?}", cli.output);
    }

    // Enable third-party bundling from CLI (opt-in; config file/env can also enable it)
    if cli.bundle_third_party {
        config.bundle_third_party = Some(true);
    }

    // Resolve the source map mode: CLI takes precedence over the config file.
    // A bare `--sourcemap` selects the esbuild-style default: linked for file
    // output, inline for stdout (a linked map has nowhere to live next to stdout).
    if let Some(cli_mode) = cli.sourcemap {
        config.sourcemap = Some(cli_mode.unwrap_or(if cli.stdout {
            SourceMapMode::Inline
        } else {
            SourceMapMode::Linked
        }));
    }
    if cli.stdout
        && matches!(
            config.sourcemap,
            Some(SourceMapMode::Linked | SourceMapMode::External)
        )
    {
        return Err(anyhow!(
            "linked and external source maps require an output file; use --sourcemap=inline with \
             --stdout"
        ));
    }
    if let Some(sources_content) = cli.sources_content {
        config.sources_content = Some(sources_content);
    }

    debug!("Configuration: {config:?}");

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

    let mut bundler = BundleOrchestrator::new(config);

    if cli.stdout {
        // Output to stdout - use write_all for explicit I/O control and error handling
        let bundled_code = bundler.bundle_to_string(entry, cli.emit_requirements)?;
        use std::io::Write;
        std::io::stdout()
            .write_all(bundled_code.as_bytes())
            .map_err(|e| anyhow!("Failed to write bundle to stdout: {e}"))?;
        info!("Bundle output to stdout");
    } else {
        // Output to file
        let output_path = cli
            .output
            .as_ref()
            .expect("Output path should be present when not using stdout");
        bundler.bundle(entry, output_path, cli.emit_requirements)?;
        info!("Bundle created successfully at {}", output_path.display());
    }

    Ok(())
}
