use anyhow::Result;
use serpen::bundler::Bundler;
use serpen::config::Config;
use std::path::Path;
use tempfile::TempDir;

/// Test that demonstrates the alias transformation issue
///
/// Without proper transform_module_ast implementation, the bundled output will contain:
/// 1. Alias assignments (correct)
/// 2. Original import statements (incorrect - should be removed/transformed)
///
/// This test verifies that aliased import statements are properly filtered/transformed
/// during the bundling process.
#[test]
fn test_alias_transformation_removes_redundant_imports() -> Result<()> {
    let test_dir = Path::new("tests/fixtures/ast_rewriting/alias_transformation_test");
    let entry_file = test_dir.join("main.py");
    // Ensure test files exist
    assert!(
        entry_file.exists(),
        "Test entry file should exist: {}",
        entry_file.display()
    );

    // Create config and bundler instance
    let config = Config {
        src: vec![test_dir.to_path_buf()],
        ..Default::default()
    };
    let mut bundler = Bundler::new(config);

    // Create temporary output directory
    let temp_dir = TempDir::new()?;
    let output_path = temp_dir.path().join("bundle.py");

    // Bundle the project
    bundler.bundle(&entry_file, &output_path, false)?;

    // Read the bundled output
    let bundled_output = std::fs::read_to_string(&output_path)?;

    // The key assertions that will fail without proper transform_module_ast:

    // 1. Should NOT contain aliased imports in preserved imports section
    assert!(
        !bundled_output.contains("# Preserved imports\nimport json"),
        "Bundled output should NOT contain json in preserved imports when it's aliased"
    );
    assert!(
        !bundled_output.contains("# Preserved imports\nimport os"),
        "Bundled output should NOT contain os in preserved imports when it's aliased"
    );
    assert!(
        !bundled_output.contains("# Preserved imports\nimport sys"),
        "Bundled output should NOT contain sys in preserved imports when it's aliased"
    );

    // 2. Should have imports for alias assignments in separate section
    assert!(
        bundled_output.contains("# Imports for alias assignments\nimport json")
            || bundled_output.contains("# Imports for alias assignments")
                && bundled_output.contains("import json"),
        "Should have json import in alias assignments section"
    );
    assert!(
        bundled_output.contains("import os") && bundled_output.contains("operating_system = os"),
        "Should have both 'import os' (for alias) and alias assignment"
    );
    assert!(
        bundled_output.contains("import sys") && bundled_output.contains("system_info = sys"),
        "Should have both 'import sys' (for alias) and alias assignment"
    );

    // 2. Should NOT contain aliased from-import statements
    assert!(
        !bundled_output.contains("from utils.data_processor import process_data as process_a"),
        "Bundled output should NOT contain aliased from-import (should be transformed/removed)"
    );
    assert!(
        !bundled_output.contains("from utils.data_processor import format_output as format_a"),
        "Bundled output should NOT contain aliased from-import (should be transformed/removed)"
    );
    assert!(
        !bundled_output.contains("from utils.config_manager import load_config as config_a"),
        "Bundled output should NOT contain aliased from-import (should be transformed/removed)"
    );

    // 3. Mixed from-import should be partially transformed
    assert!(
        !bundled_output.contains("from utils.helpers import helper_func, debug_print as debug_a"),
        "Mixed from-import should be transformed to remove aliased items"
    );
    // The remaining non-aliased item should still be present in some form or handled by bundling

    // 4. Should contain alias assignments
    assert!(
        bundled_output.contains("j = json"),
        "Bundled output should contain alias assignment for json"
    );
    assert!(
        bundled_output.contains("operating_system = os"),
        "Bundled output should contain alias assignment for os"
    );
    assert!(
        bundled_output.contains("system_info = sys"),
        "Bundled output should contain alias assignment for sys"
    );
    assert!(
        bundled_output.contains("process_a = process_data"),
        "Bundled output should contain alias assignment for process_data"
    );
    assert!(
        bundled_output.contains("format_a = format_output"),
        "Bundled output should contain alias assignment for format_output"
    );
    assert!(
        bundled_output.contains("config_a = load_config"),
        "Bundled output should contain alias assignment for load_config"
    );
    assert!(
        bundled_output.contains("debug_a = debug_print"),
        "Bundled output should contain alias assignment for debug_print"
    );

    // 5. Should still contain non-aliased imports
    assert!(
        bundled_output.contains("import math"),
        "Non-aliased imports should remain unchanged"
    );
    assert!(
        bundled_output.contains("import hashlib"),
        "Non-aliased imports should remain unchanged"
    );

    // 6. Should contain the bundled module code
    assert!(
        bundled_output.contains("def process_data(data_list):"),
        "Should contain bundled module functions"
    );
    assert!(
        bundled_output.contains("def load_config(config_file):"),
        "Should contain bundled module functions"
    );
    assert!(
        bundled_output.contains("def helper_func(input_str):"),
        "Should contain bundled module functions"
    );

    Ok(())
}

/// Test that verifies alias assignments are generated correctly
/// This test should pass even without transform_module_ast implementation
#[test]
fn test_alias_assignments_generation() -> Result<()> {
    let test_dir = Path::new("tests/fixtures/ast_rewriting/alias_transformation_test");
    let entry_file = test_dir.join("main.py");

    // Create config and bundler instance
    let config = Config {
        src: vec![test_dir.to_path_buf()],
        ..Default::default()
    };
    let mut bundler = Bundler::new(config);

    // Create temporary output directory
    let temp_dir = TempDir::new()?;
    let output_path = temp_dir.path().join("bundle.py");

    // Bundle the project
    bundler.bundle(&entry_file, &output_path, false)?;

    // Read the bundled output
    let bundled_output = std::fs::read_to_string(&output_path)?;

    // This should work regardless of transform_module_ast implementation
    // because generate_alias_assignments should create these

    assert!(
        bundled_output.contains("j = json"),
        "Should generate alias assignment for json"
    );
    assert!(
        bundled_output.contains("operating_system = os"),
        "Should generate alias assignment for os"
    );
    assert!(
        bundled_output.contains("system_info = sys"),
        "Should generate alias assignment for sys"
    );

    Ok(())
}

/// Test that demonstrates the current broken behavior
/// This test documents what happens WITHOUT the fix
#[test]
#[ignore] // Remove this ignore when you want to see the current broken behavior
fn test_current_broken_behavior_with_redundant_imports() -> Result<()> {
    let test_dir = Path::new("tests/fixtures/ast_rewriting/alias_transformation_test");
    let entry_file = test_dir.join("main.py");

    // Create config and bundler instance
    let config = Config {
        src: vec![test_dir.to_path_buf()],
        ..Default::default()
    };
    let mut bundler = Bundler::new(config);

    // Create temporary output directory
    let temp_dir = TempDir::new()?;
    let output_path = temp_dir.path().join("bundle.py");

    // Bundle the project
    bundler.bundle(&entry_file, &output_path, false)?;

    // Read the bundled output
    let bundled_output = std::fs::read_to_string(&output_path)?;

    println!("=== CURRENT BROKEN OUTPUT (with redundant imports) ===");
    println!("{}", bundled_output);

    // This test demonstrates the problem: both alias assignments AND original imports exist
    assert!(
        bundled_output.contains("j = json") && bundled_output.contains("import json as j"),
        "BROKEN: Should have both alias assignment AND original import (redundant)"
    );

    Ok(())
}
