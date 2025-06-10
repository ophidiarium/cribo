use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;

fn get_fixture_path(fixture_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bundling")
        .join(format!("future_imports_{}", fixture_name))
}

#[test]
fn test_future_imports_bundling_and_execution() -> Result<()> {
    let fixture_path = get_fixture_path("basic");
    let temp_dir = tempfile::TempDir::new()?;
    let output_path = temp_dir.path().join("bundled.py");

    // Bundle the project
    let config = Config::default();
    let mut bundler = BundleOrchestrator::new(config);

    let entry_path = fixture_path.join("main.py");
    bundler.bundle(&entry_path, &output_path, false)?;

    // Read the bundled output
    let bundled_content = fs::read_to_string(&output_path)?;

    // FIRST: Most importantly: verify the bundled file executes without syntax errors
    let output = Command::new("python3")
        .arg(&output_path)
        .current_dir(temp_dir.path())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "Bundled Python file failed to execute!\nstderr: {}\nstdout: {}\n\nBundled content:\n{}",
            stderr, stdout, bundled_content
        );
    }

    // SECOND: Verify the output contains expected result
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Processing result:"),
        "Expected output not found in: {}",
        stdout
    );

    // THIRD: Check that future imports are properly hoisted to the top
    let lines: Vec<&str> = bundled_content.lines().collect();

    // Find the first non-comment, non-shebang, non-docstring line that contains code
    let mut first_code_line_idx = None;
    let mut in_docstring = false;
    let mut docstring_delimiter = "";

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Handle docstring detection
        if !in_docstring && (trimmed.starts_with("'''") || trimmed.starts_with("\"\"\"")) {
            in_docstring = true;
            docstring_delimiter = if trimmed.starts_with("'''") {
                "'''"
            } else {
                "\"\"\""
            };
            // Check if docstring closes on same line
            if trimmed.len() > 3 && trimmed[3..].contains(docstring_delimiter) {
                in_docstring = false;
            }
            continue;
        }

        if in_docstring {
            if trimmed.contains(docstring_delimiter) {
                in_docstring = false;
            }
            continue;
        }

        // Skip empty lines, comments, and shebang
        if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("#!/") {
            first_code_line_idx = Some(i);
            break;
        }
    }

    // The first code line should be a future import
    if let Some(idx) = first_code_line_idx {
        assert!(
            lines[idx].trim().starts_with("from __future__ import"),
            "First code line should be a future import, but found: {} at line {}",
            lines[idx],
            idx
        );
    }

    // Verify that there are no future imports later in the file
    // (after the initial hoisted ones)
    let mut found_non_future_code = false;
    let mut future_imports_section_ended = false;

    for line in &lines {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("#!/") {
            continue;
        }

        if trimmed.starts_with("from __future__ import") {
            if future_imports_section_ended && found_non_future_code {
                panic!("Found future import after non-future code: {}", line);
            }
        } else {
            // This is non-future code
            if !future_imports_section_ended {
                future_imports_section_ended = true;
            }
            found_non_future_code = true;
        }
    }

    Ok(())
}

#[test]
fn test_multiple_future_imports_deduplication() -> Result<()> {
    let fixture_path = get_fixture_path("multiple");
    let temp_dir = tempfile::TempDir::new()?;
    let output_path = temp_dir.path().join("bundled.py");

    // Bundle the project
    let config = Config::default();
    let mut bundler = BundleOrchestrator::new(config);

    let entry_path = fixture_path.join("main.py");
    bundler.bundle(&entry_path, &output_path, false)?;

    // Read and verify the bundled output
    let bundled_content = fs::read_to_string(&output_path)?;

    // Verify execution
    let output = Command::new("python3")
        .arg(&output_path)
        .current_dir(temp_dir.path())
        .output()?;

    assert!(
        output.status.success(),
        "Bundled file execution failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should have hoisted and deduplicated future imports
    // Each unique feature should appear only once, preferably in a single import statement
    let future_import_lines: Vec<&str> = bundled_content
        .lines()
        .filter(|line| line.trim().starts_with("from __future__ import"))
        .collect();

    // Should have exactly one future import line
    assert_eq!(
        future_import_lines.len(),
        1,
        "Should have exactly one future import line"
    );

    let future_import_line = future_import_lines[0];

    // Verify all three features are present
    assert!(
        future_import_line.contains("annotations"),
        "Should contain annotations import"
    );
    assert!(
        future_import_line.contains("print_function"),
        "Should contain print_function import"
    );
    assert!(
        future_import_line.contains("division"),
        "Should contain division import"
    );

    // Verify no duplicate future imports exist elsewhere
    let total_future_mentions = bundled_content.matches("from __future__").count();
    assert_eq!(
        total_future_mentions, 1,
        "Should have no duplicate future imports"
    );

    Ok(())
}

#[test]
fn test_future_imports_deterministic_output() -> Result<()> {
    let fixture_path = get_fixture_path("multiple");
    let temp_dir = tempfile::TempDir::new()?;

    // Bundle the same project multiple times
    let mut outputs = Vec::new();
    for i in 0..3 {
        let output_path = temp_dir.path().join(format!("bundled_{}.py", i));

        let config = Config::default();
        let mut bundler = BundleOrchestrator::new(config);
        let entry_path = fixture_path.join("main.py");
        bundler.bundle(&entry_path, &output_path, false)?;

        let content = fs::read_to_string(&output_path)?;
        outputs.push(content);
    }

    // All outputs should be identical (deterministic)
    assert_eq!(
        outputs[0], outputs[1],
        "First and second bundle outputs should be identical"
    );
    assert_eq!(
        outputs[1], outputs[2],
        "Second and third bundle outputs should be identical"
    );

    // Verify that future imports are sorted alphabetically
    let lines: Vec<&str> = outputs[0].lines().collect();
    let future_import_line = lines
        .iter()
        .find(|line| line.trim().starts_with("from __future__ import"))
        .expect("Should find future import line");

    // Extract features from the import line
    let import_part = future_import_line
        .split("from __future__ import ")
        .nth(1)
        .expect("Should have import part");

    let features: Vec<&str> = import_part.split(", ").map(|f| f.trim()).collect();

    // Verify features are sorted alphabetically
    let mut sorted_features = features.clone();
    sorted_features.sort();
    assert_eq!(
        features, sorted_features,
        "Future import features should be sorted alphabetically"
    );

    // Verify expected features are present in correct order
    assert_eq!(features, vec!["annotations", "division", "print_function"]);

    Ok(())
}
