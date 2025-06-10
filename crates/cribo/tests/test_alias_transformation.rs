#![allow(clippy::disallowed_methods)]

use anyhow::Result;
use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;
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
    let mut bundler = BundleOrchestrator::new(config);

    // Create temporary output directory
    let temp_dir = TempDir::new()?;
    let output_path = temp_dir.path().join("bundle.py");

    // Bundle the project
    bundler.bundle(&entry_file, &output_path, false)?;

    // Read the bundled output
    let bundled_output = std::fs::read_to_string(&output_path)?;

    // The key assertions for hybrid bundler behavior:

    // 1. Stdlib imports with aliases are preserved as-is
    assert!(
        bundled_output.contains("import json as j"),
        "Bundled output should contain aliased json import"
    );
    assert!(
        bundled_output.contains("import os as operating_system"),
        "Bundled output should contain aliased os import"
    );
    assert!(
        bundled_output.contains("import sys as system_info"),
        "Bundled output should contain aliased sys import"
    );

    // 2. Local module imports are transformed to simple assignments since modules are inlined
    assert!(
        bundled_output.contains("process_a = process_data"),
        "Should have process_a assignment to inlined function"
    );
    assert!(
        bundled_output.contains("format_a = format_output"),
        "Should have format_a assignment to inlined function"
    );
    assert!(
        bundled_output.contains("config_a = load_config"),
        "Should have config_a assignment to inlined function"
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

    // 4. Mixed from-import should have simple assignments since modules are inlined
    assert!(
        bundled_output.contains("debug_a = debug_print"),
        "Should have debug_a assignment to inlined function"
    );

    // 5. Non-aliased stdlib imports remain unchanged
    assert!(
        bundled_output.contains("import math"),
        "Non-aliased math import should remain"
    );
    assert!(
        bundled_output.contains("import hashlib"),
        "Non-aliased hashlib import should remain"
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
    let mut bundler = BundleOrchestrator::new(config);

    // Create temporary output directory
    let temp_dir = TempDir::new()?;
    let output_path = temp_dir.path().join("bundle.py");

    // Bundle the project
    bundler.bundle(&entry_file, &output_path, false)?;

    // Read the bundled output
    let bundled_output = std::fs::read_to_string(&output_path)?;

    // With hybrid bundler, stdlib imports are preserved with aliases
    // Local imports use sys.modules

    assert!(
        bundled_output.contains("import json as j"),
        "Should have aliased json import"
    );
    assert!(
        bundled_output.contains("import os as operating_system"),
        "Should have aliased os import"
    );
    assert!(
        bundled_output.contains("import sys as system_info"),
        "Should have aliased sys import"
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
    let mut bundler = BundleOrchestrator::new(config);

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
