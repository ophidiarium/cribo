#![allow(clippy::disallowed_methods)] // insta macros use unwrap internally

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;
use cribo::util::get_python_executable;
use pretty_assertions::assert_eq;
use serde::Serialize;

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

/// Get filters for normalizing paths and Python version differences in snapshots
fn get_path_filters() -> Vec<(&'static str, &'static str)> {
    vec![
        // Python installation paths (minimal filtering needed with 5-line stderr limit)
        // macOS Homebrew Python paths
        (
            r"/opt/homebrew/Cellar/python@[\d.]+/[\d._]+/Frameworks/Python\.framework/Versions/[\d.]+/lib/python[\d.]+/",
            "<PYTHON_LIB>/",
        ),
        // Unix system Python paths
        (r"/usr/lib/python[\d.]+/", "<PYTHON_LIB>/"),
        // Windows Python paths
        (r"C:\\Python\d+\\lib\\", "<PYTHON_LIB>/"),
        // Windows hosted tool cache paths (GitHub Actions)
        (
            r"C:\\hostedtoolcache\\windows\\Python\\[\d.]+\\x64\\Lib\\",
            "<PYTHON_LIB>/",
        ),
        // Replace line numbers that may vary between Python versions
        (
            r"line \d+, in import_module",
            "line <LINE>, in import_module",
        ),
        // Note: Only keeping first 2 lines of stderr eliminates most cross-platform differences
        // Note: File paths eliminated by using stdin execution (shows as <stdin>)
    ]
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

/// Structured requirements data for YAML snapshots
#[derive(Debug, Serialize)]
struct RequirementsData {
    packages: Vec<String>,
    count: usize,
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
        let location = message.compute_start_location();
        let rule_name = message.name();
        let violation_info = format!(
            "Line {}: {} - {}",
            location.line.get(),
            rule_name,
            message.body()
        );

        match rule_name {
            "F401" => f401_violations.push(violation_info),
            "F404" => f404_violations.push(violation_info),
            _ => other_violations.push(violation_info),
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

/// Test bundling fixtures using Insta's glob feature
/// This discovers and tests all fixtures automatically
#[test]
fn test_bundling_fixtures() {
    insta::glob!("fixtures/bundling", "*/main.py", |path| {
        // Extract fixture name from the path
        let fixture_dir = path.parent().unwrap();
        let fixture_name = fixture_dir.file_name().unwrap().to_str().unwrap();

        // Print which fixture we're running (will only show when not filtered out)
        eprintln!("Running fixture: {}", fixture_name);

        // Check fixture type based on prefix
        let expects_bundling_failure = fixture_name.starts_with("xfail_");
        let expects_python_failure = fixture_name.starts_with("pyfail_");

        // First, run the original fixture to ensure it's valid Python code
        let python_cmd = get_python_executable();
        let original_output = Command::new(&python_cmd)
            .arg(path)
            .current_dir(fixture_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|child| child.wait_with_output())
            .expect("Failed to execute original fixture");

        // Handle Python execution based on fixture type
        match (
            original_output.status.success(),
            expects_python_failure,
            expects_bundling_failure,
        ) {
            // pyfail_: MUST fail Python direct execution
            (true, true, _) => {
                panic!(
                    "Fixture '{}' with pyfail_ prefix succeeded in direct Python execution, but it MUST fail",
                    fixture_name
                );
            }
            // pyfail_: Expected to fail, and it did - good!
            (false, true, _) => {
                // Continue to bundling
            }
            // xfail_: Python execution doesn't matter, will check bundling later
            (_, false, true) => {
                // Continue to bundling
            }
            // Normal fixture: MUST succeed in Python execution
            (false, false, false) => {
                let stderr = String::from_utf8_lossy(&original_output.stderr);
                let stdout = String::from_utf8_lossy(&original_output.stdout);

                panic!(
                    "Original fixture '{}' failed to execute:\n\
                    Exit code: {}\n\
                    Stdout:\n{}\n\
                    Stderr:\n{}\n\n\
                    Fix the fixture before testing bundling.",
                    fixture_name,
                    original_output.status.code().unwrap_or(-1),
                    stdout.trim(),
                    stderr.trim()
                );
            }
            // Normal fixture: Succeeded as expected
            (true, false, false) => {
                // Continue to bundling
            }
        }

        // Store original execution results for comparison
        let original_stdout = String::from_utf8_lossy(&original_output.stdout)
            .trim()
            .replace("\r\n", "\n")
            .to_string();
        let original_exit_code = original_output.status.code().unwrap_or(-1);

        // Create temporary directory for output
        let temp_dir = TempDir::new().unwrap();
        let bundle_path = temp_dir.path().join("bundled.py");

        // Configure bundler
        let config = Config::default();
        let mut bundler = BundleOrchestrator::new(config);

        // Bundle the fixture with requirements generation
        if let Err(e) = bundler.bundle(path, &bundle_path, true) {
            // xfail_: bundling failures are expected
            if expects_bundling_failure {
                // The fixture is expected to fail, so bundling failure is OK
                // We'll create a simple error output for the snapshot
                let error_msg = format!("Bundling failed as expected: {}", e);

                // Create error snapshot
                insta::with_settings!({
                    snapshot_suffix => fixture_name,
                    prepend_module_to_snapshot => false,
                }, {
                    insta::assert_snapshot!("bundling_error", error_msg);
                });

                return;
            } else {
                // Unexpected bundling failure
                panic!("Bundling failed unexpectedly for {}: {}", fixture_name, e);
            }
        }

        // For xfail_, bundling success is OK - we'll check execution later

        // Read the bundled code
        let bundled_code = fs::read_to_string(&bundle_path).unwrap();

        // Read and parse the requirements.txt if it was generated
        let requirements_path = temp_dir.path().join("requirements.txt");
        let requirements_data = if requirements_path.exists() {
            let content = fs::read_to_string(&requirements_path).unwrap_or_else(|_| String::new());
            let packages: Vec<String> = content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.trim().to_string())
                .collect();
            let count = packages.len();
            RequirementsData { packages, count }
        } else {
            RequirementsData {
                packages: vec![],
                count: 0,
            }
        };

        // Optionally validate Python syntax before execution
        let python_cmd = get_python_executable();
        let syntax_check = Command::new(&python_cmd)
            .args(["-m", "py_compile", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        if let Ok(mut child) = syntax_check {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(bundled_code.as_bytes());
            }
            if let Ok(output) = child.wait_with_output() {
                if !output.status.success() && std::env::var("RUST_TEST_VERBOSE").is_ok() {
                    eprintln!(
                        "Warning: Bundled code has syntax errors for fixture {}",
                        fixture_name
                    );
                    eprintln!("Stderr: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
        }

        // Run ruff linting for cross-validation
        let ruff_results = run_ruff_lint_on_bundle(&bundled_code);

        // Execute the bundled code via stdin for consistent snapshots
        let python_output = Command::new(&python_cmd)
            .arg("-")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(temp_dir.path())
            .spawn()
            .and_then(|mut child| {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(bundled_code.as_bytes());
                }
                child.wait_with_output()
            })
            .expect("Failed to execute Python");

        // Handle execution results based on fixture type
        let execution_success = python_output.status.success();

        // pyfail_: MUST succeed after bundling
        if expects_python_failure && !execution_success {
            let stderr = String::from_utf8_lossy(&python_output.stderr);
            let stdout = String::from_utf8_lossy(&python_output.stdout);

            panic!(
                "Fixture '{}' with pyfail_ prefix failed after bundling, but it MUST pass:\n\
                Exit code: {}\n\
                Stdout:\n{}\n\
                Stderr:\n{}",
                fixture_name,
                python_output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            );
        }

        // Normal fixtures without pyfail_ or xfail_: execution failure is unexpected
        if !expects_python_failure && !expects_bundling_failure && !execution_success {
            let stderr = String::from_utf8_lossy(&python_output.stderr);
            let stdout = String::from_utf8_lossy(&python_output.stdout);

            panic!(
                "Python execution failed unexpectedly for fixture '{}':\n\
                Exit code: {}\n\
                Stdout:\n{}\n\
                Stderr:\n{}",
                fixture_name,
                python_output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            );
        }

        // Compare bundled execution to original execution
        let bundled_stdout = String::from_utf8_lossy(&python_output.stdout)
            .trim()
            .replace("\r\n", "\n")
            .to_string();
        let bundled_exit_code = python_output.status.code().unwrap_or(-1);

        // For normal tests (not pyfail_), stdout should match exactly
        if !expects_python_failure && !expects_bundling_failure {
            assert_eq!(
                original_stdout, bundled_stdout,
                "\nBundled output differs from original for fixture '{}'",
                fixture_name
            );
        }

        // Exit codes should also match for normal tests
        if !expects_python_failure && !expects_bundling_failure {
            assert_eq!(
                original_exit_code, bundled_exit_code,
                "\nExit code differs for fixture '{}'",
                fixture_name
            );
        }

        // Check for pyfail tests that are now passing
        // Note: pyfail fixtures succeed after bundling due to circular dependency resolution
        // This is the expected behavior - the bundler has resolved issues that exist in the original code
        if python_output.status.success()
            && expects_python_failure
            && !original_output.status.success()
        {
            // This is expected - the bundler fixed issues in the original code
            eprintln!(
                "Note: Fixture '{}' fails when run directly but succeeds after bundling. \
                This demonstrates the bundler's ability to resolve circular dependencies.",
                fixture_name
            );
        }

        // Handle xfail_ validation
        if expects_bundling_failure {
            // xfail_ requires:
            // 1. Original fixture must run successfully
            if !original_output.status.success() {
                panic!(
                    "Fixture '{}' with xfail_ prefix: original fixture failed to run, but it MUST succeed",
                    fixture_name
                );
            }

            // 2. Bundled fixture must either:
            //    a. Fail during execution (different exit code)
            //    b. Produce different output than original
            let bundled_success = python_output.status.success();

            if bundled_success {
                // Both original and bundled succeeded - check if outputs match
                if bundled_stdout == original_stdout {
                    panic!(
                        "Fixture '{}' with xfail_ prefix: bundled code succeeded and produced same output as original.\n\
                        This test is now fully passing. Please remove the 'xfail_' prefix from the fixture directory name.",
                        fixture_name
                    );
                }
                // Outputs differ - this is expected for xfail
            }
            // If bundled failed, that's expected for xfail
        }

        // Create structured execution results
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
            stderr: {
                let full_stderr = String::from_utf8_lossy(&python_output.stderr)
                    .trim()
                    .replace("\r\n", "\n");
                // Keep only first 2 lines to avoid cross-platform traceback differences
                full_stderr.lines().take(2).collect::<Vec<_>>().join("\n")
            },
        };

        // Use Insta's with_settings for better snapshot organization
        insta::with_settings!({
            snapshot_suffix => fixture_name,
            omit_expression => true,
            prepend_module_to_snapshot => false,
            filters => get_path_filters(),
        }, {
            // Snapshot the bundled code
            insta::assert_snapshot!("bundled_code", bundled_code);

            // Snapshot execution results with filters applied
            insta::assert_debug_snapshot!("execution_results", execution_results);

            // Snapshot ruff linting results
            insta::assert_debug_snapshot!("ruff_lint_results", ruff_results);

            // Snapshot requirements data as YAML
            insta::assert_yaml_snapshot!("requirements", requirements_data);
        });
    });
}
