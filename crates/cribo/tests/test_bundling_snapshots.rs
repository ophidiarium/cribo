#![expect(clippy::disallowed_methods)] // insta macros use unwrap internally

mod common;

use std::{
    env, fs,
    io::Write,
    path::Path,
    process::{Child, Command},
    sync::atomic::{AtomicUsize, Ordering},
};

use pretty_assertions::assert_eq;
// Ruff linting integration for cross-validation
use ruff_linter::linter::{ParseSource, lint_only};
use ruff_linter::{
    registry::Rule,
    settings::{LinterSettings, flags},
    source_kind::SourceKind,
};
use ruff_python_ast::PySourceType;
use serde::Serialize;
use tempfile::TempDir;

static FIXTURE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Run cribo with given arguments and return (stdout, stderr, `exit_code`).
/// When a fixture provides a fake virtual environment, it is exposed via `VIRTUAL_ENV`.
fn run_cribo(args: &[&str], virtualenv: Option<&Path>) -> (String, String, i32) {
    let cribo_exe = env!("CARGO_BIN_EXE_cribo");

    let mut command = Command::new(cribo_exe);
    command
        .args(args)
        .env("RUST_LOG", "off")
        .env("CARGO_TERM_COLOR", "never")
        .env("NO_COLOR", "1");
    if let Some(venv) = virtualenv {
        command.env("VIRTUAL_ENV", venv);
    }
    let output = command.output().expect("Failed to execute cribo");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    (stdout, stderr, exit_code)
}

/// Collect the site-packages directories of a fixture's fake virtual environment.
fn fake_venv_site_packages(venv: &Path) -> Vec<std::path::PathBuf> {
    let mut site_packages = Vec::new();
    let lib_dir = venv.join("lib");
    if let Ok(entries) = fs::read_dir(&lib_dir) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("site-packages");
            if candidate.is_dir() {
                site_packages.push(candidate);
            }
        }
    }
    // read_dir order is unspecified; sort for deterministic PYTHONPATH ordering
    site_packages.sort();
    let windows_layout = venv.join("Lib").join("site-packages");
    if windows_layout.is_dir() {
        site_packages.push(windows_layout);
    }
    site_packages
}

#[derive(Default)]
struct TestSummary;

impl Drop for TestSummary {
    fn drop(&mut self) {
        let count = FIXTURE_COUNT.load(Ordering::Relaxed);
        if count > 0 && !std::thread::panicking() {
            // Use logging to ensure it's printed even if tests pass and stdout is captured
            log::info!("Total fixtures checked: {count}");
        }
    }
}

static SUMMARY: std::sync::LazyLock<TestSummary> = std::sync::LazyLock::new(TestSummary::default);

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
        // The digest authenticates exact map bytes, which legitimately vary
        // when source paths differ across platforms or coverage builds.
        (
            r#"(_CriboSourceMapRuntime\._bootstrap\("[^"]+", globals\(\)\.get\("__file__", "<stdin>"\), ")[0-9a-f]{64}("\))"#,
            "$1<SOURCE_MAP_DIGEST>$2",
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
    f401: Vec<String>, // Unused imports
    f404: Vec<String>, // Late future imports
    other: Vec<String>,
    total: usize,
}

/// Structured requirements data for YAML snapshots
#[derive(Debug, Serialize)]
struct RequirementsData {
    packages: Vec<String>,
    count: usize,
}

/// Render a Source Map v3 JSON as a deterministic, path-free text dump.
///
/// One line per mapping: `bundle:<line> `<generated statement>` -> <source
/// basename>:<line>` with 1-based line numbers. Basenames keep the snapshot
/// free of machine-specific absolute paths.
fn render_source_map_dump(map_json: &str, bundled_code: &str) -> String {
    use std::fmt::Write;

    let map = oxc_sourcemap::SourceMap::from_json_string(map_json)
        .expect("emitted source map must be valid Source Map v3 JSON");
    let bundle_lines: Vec<&str> = bundled_code.lines().collect();
    let mut dump = String::new();
    for token in map.get_tokens() {
        let source = token
            .get_source_id()
            .and_then(|id| map.get_source(id))
            .unwrap_or("<unknown>");
        let source_name = Path::new(source).file_name().map_or_else(
            || source.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let generated_text = bundle_lines
            .get(token.get_dst_line() as usize)
            .map(|line| line.trim())
            .unwrap_or_default();
        let _ = writeln!(
            dump,
            "bundle:{:<4} `{}` -> {}:{}",
            token.get_dst_line() + 1,
            generated_text,
            source_name,
            token.get_src_line() + 1,
        );
    }
    dump
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
    let source_kind = SourceKind::Python {
        code: bundled_code.to_owned(),
        is_stub: false,
    };

    let result = lint_only(
        path,
        None,
        &settings,
        flags::Noqa::Enabled,
        &source_kind,
        PySourceType::Python,
        ParseSource::None,
    );

    let mut f401 = Vec::new();
    let mut f404 = Vec::new();
    let mut other = Vec::new();

    for message in &result.diagnostics {
        let location = message.ruff_start_location();
        let rule_name = message.name();
        let violation_info = location.map_or_else(
            || format!("{} - {}", rule_name, message.headline_message()),
            |loc| {
                format!(
                    "Line {}: {} - {}",
                    loc.line.get(),
                    rule_name,
                    message.headline_message()
                )
            },
        );

        // Check if it's a lint rule by looking at the diagnostic id
        if message.id().is_lint_named("F401") {
            f401.push(violation_info);
        } else if message.id().is_lint_named("F404") {
            f404.push(violation_info);
        } else {
            other.push(violation_info);
        }
    }

    let total = f401.len() + f404.len() + other.len();

    RuffLintResults {
        f401,
        f404,
        other,
        total,
    }
}

/// Test bundling fixtures using Insta's glob feature
/// This discovers and tests all fixtures automatically
#[test]
fn test_bundling_fixtures() {
    // Reset fixture counter before running fixtures
    FIXTURE_COUNT.store(0, Ordering::Relaxed);
    insta::glob!("fixtures/", "*/main.py", |path| {
        // Initialize summary reporter on the first test run and increment count
        std::sync::LazyLock::force(&SUMMARY);
        FIXTURE_COUNT.fetch_add(1, Ordering::Relaxed);

        // Extract fixture name from the path
        let fixture_dir = path.parent().unwrap();
        let fixture_name = fixture_dir.file_name().unwrap().to_str().unwrap();

        // Print which fixture we're running (will only show when not filtered out)
        eprintln!("Running fixture: {fixture_name}");

        // Check fixture type based on prefix
        let expects_bundling_failure = fixture_name.starts_with("xfail_");
        let expects_python_failure = fixture_name.starts_with("pyfail_");
        // sourcemap_ fixtures opt into --sourcemap=linked and get an extra
        // normalized mapping snapshot (see render_source_map_dump)
        let enables_sourcemap = fixture_name.starts_with("sourcemap_");

        // Get Python executable once for the entire test
        let python_cmd = common::get_python_executable();

        // Fixture-provided fake virtual environment (fake-venv/) opts the fixture into
        // third-party bundling: cribo locates it via VIRTUAL_ENV, while the original
        // execution reaches its site-packages through PYTHONPATH
        let fake_venv_dir = fixture_dir.join("fake-venv");
        let fake_venv = fake_venv_dir.is_dir().then_some(fake_venv_dir);
        let original_pythonpath = fake_venv.as_ref().map_or_else(
            || fixture_dir.as_os_str().to_owned(),
            |venv| {
                let mut paths = vec![fixture_dir.to_path_buf()];
                paths.extend(fake_venv_site_packages(venv));
                env::join_paths(paths).expect("fixture paths must be joinable")
            },
        );

        // First, run the original fixture to ensure it's valid Python code
        let original_output = Command::new(&python_cmd)
            .arg(path)
            .current_dir(fixture_dir)
            .env("PYTHONPATH", &original_pythonpath)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONLEGACYWINDOWSSTDIO", "utf-8")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(Child::wait_with_output)
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
                    "Fixture '{fixture_name}' with pyfail_ prefix succeeded in direct Python \
                     execution, but it MUST fail"
                );
            }
            // pyfail_: Expected to fail, and it did - good!
            // xfail_: Python succeeded, will check bundling failure later
            // Normal fixture: Succeeded as expected
            (false, true, _) | (true, false, true | false) => {
                // Continue to bundling
            }
            // xfail_: Python execution MUST succeed (only bundling should fail)
            (false, false, true) => {
                let stderr = String::from_utf8_lossy(&original_output.stderr);
                let stdout = String::from_utf8_lossy(&original_output.stdout);

                panic!(
                    "Fixture '{}' with xfail_ prefix failed Python execution, but it should only \
                     fail bundling.\nIf the original Python code has errors, use pyfail_ prefix \
                     instead.\nExit code: {}\nStdout:\n{}\nStderr:\n{}\n",
                    fixture_name,
                    original_output.status.code().unwrap_or(-1),
                    stdout.trim(),
                    stderr.trim()
                );
            }
            // Normal fixture: MUST succeed in Python execution
            (false, false, false) => {
                let stderr = String::from_utf8_lossy(&original_output.stderr);
                let stdout = String::from_utf8_lossy(&original_output.stdout);

                panic!(
                    "Original fixture '{}' failed to execute:\nExit code: \
                     {}\nStdout:\n{}\nStderr:\n{}\n\nFix the fixture before testing bundling.",
                    fixture_name,
                    original_output.status.code().unwrap_or(-1),
                    stdout.trim(),
                    stderr.trim()
                );
            }
        }

        // Store original execution results for comparison
        let original_stdout = String::from_utf8_lossy(&original_output.stdout)
            .trim()
            .replace("\r\n", "\n");
        let original_exit_code = original_output.status.code().unwrap_or(-1);

        // Create temporary directory for output
        let temp_dir = TempDir::new().unwrap();
        let bundle_path = temp_dir.path().join("bundled.py");

        // Bundle the fixture with requirements generation using the cribo binary
        let mut cribo_args = vec![
            "--entry",
            path.to_str().unwrap(),
            "--output",
            bundle_path.to_str().unwrap(),
            "--emit-requirements",
        ];
        if fake_venv.is_some() {
            cribo_args.push("--bundle-third-party");
        }
        if enables_sourcemap {
            cribo_args.push("--sourcemap=linked");
        }
        // A fixture-level cribo.toml supplies configuration (e.g. module-map entries)
        let fixture_config = fixture_dir.join("cribo.toml");
        let fixture_config_str = fixture_config.to_str().map(ToOwned::to_owned);
        if fixture_config.is_file()
            && let Some(config_path) = fixture_config_str.as_deref()
        {
            cribo_args.push("--config");
            cribo_args.push(config_path);
        }
        let (_bundle_stdout, bundle_stderr, bundle_exit_code) =
            run_cribo(&cribo_args, fake_venv.as_deref());

        // Check if bundling failed
        if bundle_exit_code != 0 {
            // xfail_: bundling failures are expected
            // pyfail_: bundling failures are allowed (but not required)
            if expects_bundling_failure || expects_python_failure {
                // The fixture is expected to fail bundling (xfail_) or allowed to fail bundling
                // (pyfail_) We'll create a simple error output for the snapshot
                let error_msg = format!("Bundling failed as expected: {}", bundle_stderr.trim());

                // Create error snapshot
                insta::with_settings!({
                    snapshot_suffix => fixture_name,
                    prepend_module_to_snapshot => false,
                }, {
                    insta::assert_snapshot!("bundling_error", error_msg);
                });

                return;
            }
            // Unexpected bundling failure
            panic!(
                "Bundling failed unexpectedly for {fixture_name}:\nExit code: \
                 {bundle_exit_code}\nStderr: {}",
                bundle_stderr.trim()
            );
        }

        // For xfail_, bundling success is OK - we'll check execution later

        // Read the bundled code
        let bundled_code = fs::read_to_string(&bundle_path).unwrap();

        // Sourcemap fixtures must produce a sibling map; snapshot it in a
        // normalized, path-free form so mapping regressions are visible.
        let source_map_dump = enables_sourcemap.then(|| {
            let map_path = temp_dir.path().join("bundled.py.map");
            let map_json = fs::read_to_string(&map_path)
                .expect("sourcemap fixture must produce bundled.py.map");
            render_source_map_dump(&map_json, &bundled_code)
        });

        // Read and parse the requirements.txt if it was generated
        let requirements_path = temp_dir.path().join("requirements.txt");
        let requirements_data = if requirements_path.exists() {
            let content = fs::read_to_string(&requirements_path).unwrap_or_else(|_| String::new());
            let packages: Vec<String> = content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.trim().to_owned())
                .collect();
            let count = packages.len();
            RequirementsData { packages, count }
        } else {
            RequirementsData {
                packages: vec![],
                count: 0,
            }
        };

        // Validate Python syntax of the bundled output before execution.
        // Use compile() with the source piped via stdin: unlike `py_compile
        // <file>` this writes no `__pycache__` bytecode into temp_dir (which is
        // later the working directory for bundle execution and must stay
        // pristine), and unlike `py_compile -` it actually reads source from
        // stdin. compile() performs full syntax and semantic compilation
        // checks, matching what execution would do.
        let syntax_check = Command::new(&python_cmd)
            .args([
                "-c",
                "import sys; compile(sys.stdin.buffer.read(), '<bundled>', 'exec')",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(bundled_code.as_bytes())?;
                }
                child.wait_with_output()
            })
            .expect("Failed to run Python syntax validation on bundled output");

        // The bundler must never emit syntactically invalid Python, regardless of
        // fixture type (xfail_/pyfail_ cover bundling/runtime failures, not
        // malformed generated code), so this is a hard failure for all fixtures.
        assert!(
            syntax_check.status.success(),
            "Bundled code has invalid Python syntax for fixture '{}':\n{}",
            fixture_name,
            String::from_utf8_lossy(&syntax_check.stderr).trim()
        );

        // Run ruff linting for cross-validation
        let ruff_results = run_ruff_lint_on_bundle(&bundled_code);

        // Check for F401 violations (unused imports) - fail if any are found
        // This ensures we catch regressions where imports aren't properly removed
        assert!(
            ruff_results.f401.is_empty(),
            "F401 violations (unused imports) detected in bundled code for fixture \
             '{}':\n{}\n\nThis indicates a regression in import handling. The bundler should \
             remove all unused imports.",
            fixture_name,
            ruff_results.f401.join("\n")
        );

        // Execute the bundled code via stdin for consistent snapshots
        let python_output = Command::new(&python_cmd)
            .arg("-")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(temp_dir.path())
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONLEGACYWINDOWSSTDIO", "utf-8")
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

        // pyfail_: MAY fail after bundling (allowed but not required)
        // We don't panic here - pyfail_ tests are allowed to fail after bundling

        // Normal fixtures without pyfail_ or xfail_: execution failure is unexpected
        if !expects_python_failure && !expects_bundling_failure && !execution_success {
            let stderr = String::from_utf8_lossy(&python_output.stderr);
            let stdout = String::from_utf8_lossy(&python_output.stdout);

            panic!(
                "Python execution failed unexpectedly for fixture '{}':\nExit code: \
                 {}\nStdout:\n{}\nStderr:\n{}",
                fixture_name,
                python_output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            );
        }

        // Compare bundled execution to original execution
        let bundled_stdout = String::from_utf8_lossy(&python_output.stdout)
            .trim()
            .replace("\r\n", "\n");
        let python_exit_code = python_output.status.code().unwrap_or(-1);

        // For normal tests (not pyfail_ or xfail_), stdout should match exactly
        if !expects_python_failure && !expects_bundling_failure {
            assert_eq!(
                original_stdout, bundled_stdout,
                "\nBundled output differs from original for fixture '{}'",
                fixture_name
            );
        }

        // Exit codes should also match for normal tests (not pyfail_ or xfail_)
        if !expects_python_failure && !expects_bundling_failure {
            assert_eq!(
                original_exit_code, python_exit_code,
                "\nExit code differs for fixture '{}'",
                fixture_name
            );
        }

        // Check for pyfail tests that are now passing
        // Note: pyfail fixtures MAY succeed after bundling if the bundler resolves
        // issues like circular dependencies that exist in the original code
        if python_output.status.success()
            && expects_python_failure
            && !original_output.status.success()
        {
            // This is allowed - the bundler may have fixed issues in the original code
            eprintln!(
                "Note: Fixture '{fixture_name}' fails when run directly but succeeds after \
                 bundling. This demonstrates the bundler's ability to resolve issues like \
                 circular dependencies."
            );
        }

        // Handle xfail_ validation
        if expects_bundling_failure {
            // xfail_ requires:
            // 1. Original fixture must run successfully
            assert!(
                original_output.status.success(),
                "Fixture '{fixture_name}' with xfail_ prefix: original fixture failed to run, but \
                 it MUST succeed"
            );

            // 2. Bundled fixture must either: a. Fail during execution (different exit code) b.
            //    Produce different output than original
            let bundled_success = python_output.status.success();

            if bundled_success {
                // Both original and bundled succeeded - check if outputs match
                if bundled_stdout == original_stdout {
                    // Before suggesting to remove xfail prefix, check for duplicate lines
                    // If there are duplicates, the test should remain xfail
                    if let Err(duplicate_msg) =
                        check_for_duplicate_lines_with_result(&bundled_code, fixture_name)
                    {
                        // Has duplicates - don't suggest renaming, the test is still xfail
                        eprintln!(
                            "Note: Fixture '{fixture_name}' produces matching output but has \
                             duplicate lines. It remains an xfail test.\n{duplicate_msg}"
                        );
                    } else {
                        // No duplicates and matching output - suggest renaming
                        panic!(
                            "Fixture '{fixture_name}' with xfail_ prefix: bundled code succeeded \
                             and produced same output as original.\nThis test is now fully \
                             passing. Please remove the 'xfail_' prefix from the fixture \
                             directory name."
                        );
                    }
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
                .replace("\r\n", "\n"),
            stderr: {
                let full_stderr = String::from_utf8_lossy(&python_output.stderr)
                    .trim()
                    .replace("\r\n", "\n");
                // Keep only first 2 lines to avoid cross-platform traceback differences
                full_stderr.lines().take(2).collect::<Vec<_>>().join("\n")
            },
        };

        // Check for duplicate lines in the bundled code
        // Skip duplicate checks for xfail and pyfail tests as they may have known issues
        // Also skip for cross_package_mixed_import which has a known duplicate module assignment
        // issue
        if !expects_bundling_failure
            && !expects_python_failure
            && fixture_name != "cross_package_mixed_import"
        {
            check_for_duplicate_lines(&bundled_code, fixture_name);
        }

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

            // Snapshot the normalized source map for sourcemap_ fixtures
            if let Some(dump) = &source_map_dump {
                insta::assert_snapshot!("source_map", dump);
            }

            // Snapshot requirements data as YAML
            insta::assert_yaml_snapshot!("requirements", requirements_data);
        });
    });
    // Fail the test if no fixtures were executed
    let count = FIXTURE_COUNT.load(Ordering::Relaxed);
    // Report applied glob filter and instruct on running a specific fixture
    let filter = env::var("INSTA_GLOB_FILTER").unwrap_or_else(|_| "<none>".to_owned());
    assert!(
        count > 0,
        "\x1b[1;31m 🛑 No fixtures tested from `fixtures/` directory.\x1b[0m\n 🧩 Applied glob \
         filter: \x1b[1;95m{filter}\x1b[0m\n\n 📝 To run a specific fixture, \
         use:\nINSTA_GLOB_FILTER=\"**/stickytape_single_file/main.py\" cargo nextest run \
         --no-capture --test test_bundling_snapshots --cargo-quiet --cargo-quiet\n\n",
    );
}

/// Check for duplicate lines in the bundled code and return Result
/// Returns Ok(()) if no problematic duplicates, Err(message) if duplicates found
fn check_for_duplicate_lines_with_result(
    bundled_code: &str,
    fixture_name: &str,
) -> Result<(), String> {
    use std::fmt::Write;

    use indexmap::IndexMap;

    let mut line_counts: IndexMap<String, Vec<usize>> = IndexMap::new();

    for (line_num, line) in bundled_code.lines().enumerate() {
        // Trim only from the right to preserve indentation
        let trimmed_line = line.trim_end();

        // Skip empty lines and comments
        if trimmed_line.is_empty() || trimmed_line.trim_start().starts_with('#') {
            continue;
        }

        // Skip common Python constructs that are expected to appear multiple times
        let trimmed_no_indent = trimmed_line.trim_start();
        if trimmed_no_indent == "pass"
            || trimmed_no_indent == "return"
            || trimmed_no_indent == "continue"
            || trimmed_no_indent == "break"
            || trimmed_no_indent.starts_with("def __")  // Any dunder method
            || (trimmed_no_indent.starts_with("self.") && !trimmed_line.contains("sys.modules"))   // Common in class methods
            || (trimmed_no_indent.starts_with("_cribo_self.") && !trimmed_line.contains("sys.modules"))   // Init function module attribute stamps
            || (trimmed_no_indent.starts_with("return ") && !trimmed_line.contains("sys.modules"))
        // Return statements
        {
            continue;
        }

        // Track line numbers where this line appears
        line_counts
            .entry(trimmed_line.to_owned())
            .or_default()
            .push(line_num + 1); // Use 1-based line numbers
    }

    // Find duplicates - but only report problematic ones
    let mut duplicates: Vec<(String, Vec<usize>)> = line_counts
        .into_iter()
        .filter(|(line, occurrences)| {
            // Only report duplicates that are likely to be actual issues
            occurrences.len() > 1
                && (
                    // Definitely report duplicate sys.modules assignments, except the
                    // per-init registration/cleanup idioms every self-registering
                    // wrapper module emits
                    (line.contains("sys.modules[")
                        && line.trim_start() != "_sys.modules[_cribo_self.__name__] = _cribo_self"
                        && line.trim_start() != "del _sys.modules[_cribo_self.__name__]") ||
                // Report duplicate imports
                line.trim_start().starts_with("import ") ||
                line.trim_start().starts_with("from ") ||
                // Report duplicate global assignments (but not in class methods)
                (!line.starts_with("    ") && line.contains(" = ") && !line.contains("self."))
                )
                // Allow duplicate init function calls since they're cached with @functools.cache
                && !line.contains("_cribo_init")
        })
        .collect();

    if !duplicates.is_empty() {
        // Sort by first occurrence line number for consistent output
        duplicates.sort_by_key(|(_, occurrences)| occurrences[0]);

        let mut error_msg =
            format!("Fixture '{fixture_name}' has duplicate lines in bundled output:\n\n");

        for (line, occurrences) in duplicates {
            let _ = writeln!(
                error_msg,
                "Line '{}' appears {} times at lines: {}",
                line,
                occurrences.len(),
                occurrences
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        return Err(error_msg);
    }
    Ok(())
}

/// Check for duplicate lines in the bundled code
/// Trims lines from the right but preserves left indentation
fn check_for_duplicate_lines(bundled_code: &str, fixture_name: &str) {
    if let Err(msg) = check_for_duplicate_lines_with_result(bundled_code, fixture_name) {
        panic!("{}", msg);
    }
}
