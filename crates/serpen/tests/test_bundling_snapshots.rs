#![allow(clippy::disallowed_methods)] // insta macros use unwrap internally

use anyhow::Result;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use serpen::bundler::Bundler;
use serpen::config::Config;
use serpen::util::get_python_executable;

// Ruff linting integration for cross-validation
use ruff_linter::linter::{ParseSource, lint_only};
use ruff_linter::registry::Rule;
use ruff_linter::settings::{LinterSettings, flags};
use ruff_linter::source_kind::SourceKind;
use ruff_python_ast::PySourceType;

/// Structured execution results for better snapshot formatting
#[derive(Debug)]
#[allow(dead_code)] // Fields are used via Debug trait for snapshots
struct ExecutionResults {
    status: ExecutionStatus,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
#[allow(dead_code)] // Fields are used via Debug trait for snapshots
enum ExecutionStatus {
    Success,
    Failed(i32),
}

/// Ruff linting results for cross-validation
#[derive(Debug)]
#[allow(dead_code)] // Fields are used via Debug trait for snapshots
struct RuffLintResults {
    f401_violations: Vec<String>, // Unused imports
    f404_violations: Vec<String>, // Late future imports
    other_violations: Vec<String>,
    total_violations: usize,
}

/// Run ruff linting on bundled code to cross-validate import handling
fn run_ruff_lint_on_bundle(bundled_code: &str) -> RuffLintResults {
    // Create settings for multiple import-related rules with both F401 and F404 enabled
    let settings = LinterSettings {
        rules: [Rule::UnusedImport, Rule::LateFutureImport]
            .into_iter()
            .collect(),
        ..LinterSettings::default()
    };

    let path = Path::new("<bundled>.py");
    let source_kind = SourceKind::Python(bundled_code.to_string());

    let result = lint_only(
        path,
        None,
        &settings,
        flags::Noqa::Enabled,
        &source_kind,
        PySourceType::Python,
        ParseSource::None,
    );

    let mut f401_violations = Vec::new();
    let mut f404_violations = Vec::new();
    let mut other_violations = Vec::new();

    for message in &result.messages {
        if let Some(rule) = message.to_rule() {
            let location = message.compute_start_location();
            let violation_info = format!(
                "Line {}: {} - {}",
                location.line.get(),
                rule.noqa_code(),
                message.body()
            );

            match rule {
                Rule::UnusedImport => f401_violations.push(violation_info),
                Rule::LateFutureImport => f404_violations.push(violation_info),
                _ => other_violations.push(violation_info),
            }
        }
    }

    let total_violations = f401_violations.len() + f404_violations.len() + other_violations.len();

    RuffLintResults {
        f401_violations,
        f404_violations,
        other_violations,
        total_violations,
    }
}

/// Generic test that processes all fixture directories in tests/fixtures/bundling
/// Each directory should contain a main.py entry point and will be bundled and executed
#[test]
fn test_all_bundling_fixtures() -> Result<()> {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bundling");

    // Skip if bundling directory doesn't exist
    if !fixtures_dir.exists() {
        return Ok(());
    }

    // Read all subdirectories in the bundling fixtures directory
    let entries = fs::read_dir(&fixtures_dir)?;
    let mut fixture_names = Vec::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                fixture_names.push(name.to_string());
            }
        }
    }

    // Sort for deterministic test order
    fixture_names.sort();

    // Process each fixture directory
    for fixture_name in fixture_names {
        test_single_bundling_fixture(&fixtures_dir, &fixture_name)?;
    }

    Ok(())
}

/// Test a single bundling fixture directory
fn test_single_bundling_fixture(fixtures_dir: &Path, fixture_name: &str) -> Result<()> {
    let fixture_path = fixtures_dir.join(fixture_name);
    let main_py_path = fixture_path.join("main.py");

    // Skip if main.py doesn't exist
    if !main_py_path.exists() {
        // Skip silently or use conditional verbose logging
        if std::env::var("RUST_TEST_VERBOSE").is_ok() {
            eprintln!("Skipping {}: no main.py found", fixture_name);
        }
        return Ok(());
    }

    // Create temporary directory for output
    let temp_dir = TempDir::new()?;
    let bundle_path = temp_dir.path().join("bundled.py");

    // Configure bundler
    let config = Config::default();
    let mut bundler = Bundler::new(config);

    // Bundle the fixture
    bundler.bundle(&main_py_path, &bundle_path, false)?;

    // Optionally validate Python syntax before execution
    let python_cmd = get_python_executable();
    let syntax_check = Command::new(&python_cmd)
        .args(["-m", "py_compile"])
        .arg(&bundle_path)
        .output();
    if let Ok(output) = syntax_check {
        if !output.status.success() && std::env::var("RUST_TEST_VERBOSE").is_ok() {
            eprintln!(
                "Warning: Bundled code has syntax errors for fixture {}",
                fixture_name
            );
            eprintln!("Stderr: {}", String::from_utf8_lossy(&output.stderr));
        }
    }

    // Read the bundled code and normalize line endings for cross-platform compatibility
    let bundled_code = fs::read_to_string(&bundle_path)?;

    // Run ruff linting for cross-validation of unused imports elimination
    let ruff_results = run_ruff_lint_on_bundle(&bundled_code);

    // Execute the bundled code with Python and capture output
    // Use Python executable with virtual environment support
    let python_cmd = get_python_executable();

    let python_output = Command::new(&python_cmd)
        .arg(&bundle_path)
        .current_dir(temp_dir.path()) // Set working directory for consistent execution
        .output()
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to execute Python: {} (command: {} {:?})",
                e,
                python_cmd,
                bundle_path
            )
        })?;

    // Note: For timeout support, we'd need a more complex solution or external crate
    // The standard library doesn't provide built-in timeout for process execution

    // Create separate snapshots using insta's named snapshot feature
    insta::with_settings!({
        snapshot_suffix => fixture_name,
        omit_expression => true
    }, {
        // Snapshot the bundled code
        insta::assert_snapshot!("bundled_code", bundled_code);

        // Create structured execution results snapshot
        let execution_status = if python_output.status.success() {
            ExecutionStatus::Success
        } else {
            ExecutionStatus::Failed(python_output.status.code().unwrap_or(-1))
        };

        let execution_results = ExecutionResults {
            status: execution_status,
            stdout: String::from_utf8_lossy(&python_output.stdout)
                .trim()
                .replace("\r\n", "\n")
                .to_string(),
            stderr: String::from_utf8_lossy(&python_output.stderr)
                .trim()
                .replace("\r\n", "\n")
                .to_string(),
        };

        insta::assert_debug_snapshot!("execution_results", execution_results);

        // Snapshot ruff linting results for cross-validation
        insta::assert_debug_snapshot!("ruff_lint_results", ruff_results);
    });

    Ok(())
}
