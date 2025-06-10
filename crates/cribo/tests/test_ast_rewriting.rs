#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;

/// Test fixture for AST rewriting scenarios
#[derive(Debug)]
struct AstRewritingFixture {
    name: String,
    fixture_path: PathBuf,
    entry_point: PathBuf,
    expected_output: String,
}

impl AstRewritingFixture {
    fn new(name: &str) -> Self {
        // Use relative path from the crate directory
        let fixture_path = PathBuf::from("tests/fixtures/ast_rewriting").join(name);
        let entry_point = fixture_path.join("main.py");
        let expected_output_path = fixture_path.join("expected_output.txt");

        let expected_output = if expected_output_path.exists() {
            fs::read_to_string(&expected_output_path)
                .unwrap_or_else(|_| panic!("Failed to read expected output for {}", name))
        } else {
            String::new()
        };

        Self {
            name: name.to_string(),
            fixture_path,
            entry_point,
            expected_output,
        }
    }

    /// Bundle the fixture using Cribo
    fn bundle(&self) -> Result<String, Box<dyn std::error::Error>> {
        // Skip if fixture doesn't exist
        if !self.entry_point.exists() {
            return Err(format!("Entry point does not exist: {:?}", self.entry_point).into());
        }

        let config = Config {
            src: vec![self.fixture_path.clone()],
            ..Default::default()
        };

        let mut bundler = BundleOrchestrator::new(config);

        // Create temporary output directory
        let temp_dir = TempDir::new()?;
        let output_path = temp_dir.path().join("bundle.py");

        // Bundle the code
        bundler.bundle(&self.entry_point, &output_path, false)?;

        // Read the bundled output
        let bundled_content = fs::read_to_string(&output_path)?;
        Ok(bundled_content)
    }

    /// Execute the bundled code with Python and capture output
    fn execute_bundled_code(
        &self,
        bundled_code: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        // Create temporary file for the bundled code
        let temp_dir = TempDir::new()?;
        let bundle_path = temp_dir.path().join("bundle.py");
        fs::write(&bundle_path, bundled_code)?;

        // Execute with Python
        let output = Command::new("python3").arg(&bundle_path).output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Python execution failed: {}", stderr).into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.to_string())
    }

    /// Execute the original modular code for comparison
    fn execute_original_code(&self) -> Result<String, Box<dyn std::error::Error>> {
        // Execute the original main.py from the fixture directory
        let output = Command::new("python3")
            .arg("main.py")
            .current_dir(&self.fixture_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Original code execution failed: {}", stderr).into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.to_string())
    }

    /// Run the complete test: bundle, execute, and compare
    fn run_test(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Running AST rewriting test: {}", self.name);

        // Step 1: Bundle the code
        let bundled_code = self
            .bundle()
            .map_err(|e| format!("Bundling failed for {}: {}", self.name, e))?;

        println!("✓ Bundling successful for {}", self.name);

        // Step 2: Execute bundled code
        let bundled_output = self
            .execute_bundled_code(&bundled_code)
            .map_err(|e| format!("Bundled execution failed for {}: {}", self.name, e))?;

        println!("✓ Bundled code execution successful for {}", self.name);

        // Step 3: Execute original code for comparison
        let original_output = self
            .execute_original_code()
            .map_err(|e| format!("Original execution failed for {}: {}", self.name, e))?;

        println!("✓ Original code execution successful for {}", self.name);

        // Step 4: Compare outputs
        let bundled_trimmed = bundled_output.trim();
        let original_trimmed = original_output.trim();

        if bundled_trimmed != original_trimmed {
            return Err(format!(
                "Output mismatch for {}:\nBundled:\n{}\nOriginal:\n{}",
                self.name, bundled_trimmed, original_trimmed
            )
            .into());
        }

        // Step 5: Check against expected output if provided
        if !self.expected_output.is_empty() {
            let expected_trimmed = self.expected_output.trim();
            if bundled_trimmed != expected_trimmed {
                return Err(format!(
                    "Expected output mismatch for {}:\nActual:\n{}\nExpected:\n{}",
                    self.name, bundled_trimmed, expected_trimmed
                )
                .into());
            }
        }

        println!("✓ All tests passed for {}", self.name);
        Ok(())
    }
}

/// Helper function to run a single test fixture and return the result
fn run_single_fixture(fixture_name: &str) -> (String, bool, Option<String>) {
    let fixture = AstRewritingFixture::new(fixture_name);
    match fixture.run_test() {
        Ok(()) => {
            println!("✅ {} passed", fixture_name);
            (fixture_name.to_string(), true, None)
        }
        Err(e) => {
            eprintln!("❌ {} failed: {}", fixture_name, e);
            (fixture_name.to_string(), false, Some(e.to_string()))
        }
    }
}

#[test]
fn test_ast_rewriting_all_fixtures() {
    // Initialize logger for debugging
    let _ = env_logger::try_init();

    let fixtures_dir = PathBuf::from("tests/fixtures/ast_rewriting");

    if !fixtures_dir.exists() {
        println!("Fixtures directory does not exist, skipping tests");
        return;
    }

    let mut test_results = Vec::new();

    // Discover all fixture directories
    if let Ok(entries) = fs::read_dir(&fixtures_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }

            let fixture_name = entry.file_name().to_string_lossy().to_string();
            let main_py = entry.path().join("main.py");
            if !main_py.exists() {
                continue;
            }

            println!("Found fixture: {}", fixture_name);
            let test_result = run_single_fixture(&fixture_name);
            test_results.push(test_result);
        }
    }

    // Print summary
    let passed = test_results
        .iter()
        .filter(|(_, success, _)| *success)
        .count();
    let total = test_results.len();

    println!("\n=== AST Rewriting Test Summary ===");
    println!("Passed: {}/{}", passed, total);

    for (name, success, error) in &test_results {
        if *success {
            println!("✅ {}", name);
        } else {
            println!(
                "❌ {}: {}",
                name,
                error.as_ref().unwrap_or(&"Unknown error".to_string())
            );
        }
    }

    if passed != total {
        panic!(
            "{} out of {} AST rewriting tests failed",
            total - passed,
            total
        );
    }
}
