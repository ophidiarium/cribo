#![allow(clippy::disallowed_methods)] // insta macros use unwrap internally

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;
use cribo::util::get_python_executable;

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

        // Check if this is an expected failure fixture
        let expects_failure = fixture_name.starts_with("xfail_");

        // Create temporary directory for output
        let temp_dir = TempDir::new().unwrap();
        let bundle_path = temp_dir.path().join("bundled.py");

        // Configure bundler
        let config = Config::default();
        let mut bundler = BundleOrchestrator::new(config);

        // Bundle the fixture
        bundler.bundle(path, &bundle_path, false).unwrap();

        // Read the bundled code
        let bundled_code = fs::read_to_string(&bundle_path).unwrap();

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

        // Check for unexpected Python execution failures
        if !python_output.status.success() && !expects_failure {
            let stderr = String::from_utf8_lossy(&python_output.stderr);
            let stdout = String::from_utf8_lossy(&python_output.stdout);

            panic!(
                "Python execution failed unexpectedly for fixture '{}':\n\
                Exit code: {}\n\
                Stdout:\n{}\n\
                Stderr:\n{}\n\n\
                If this failure is expected, rename the fixture directory with 'xfail_' prefix.",
                fixture_name,
                python_output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            );
        }

        // Check for xfail tests that are now passing
        if python_output.status.success() && expects_failure {
            let stdout = String::from_utf8_lossy(&python_output.stdout);
            let stderr = String::from_utf8_lossy(&python_output.stderr);

            panic!(
                "Expected fixture '{}' to fail (marked with xfail_), but it succeeded!\n\
                Exit code: 0\n\
                Stdout:\n{}\n\
                Stderr:\n{}\n\n\
                This test is now passing. Please remove the 'xfail_' prefix from the fixture directory name.",
                fixture_name,
                stdout.trim(),
                stderr.trim()
            );
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
        });
    });
}
