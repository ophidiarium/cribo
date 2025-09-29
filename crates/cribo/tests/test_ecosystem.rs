mod common;

use std::{path::Path, process::Command};

#[test]
fn test_ecosystem_all() {
    // Get the workspace root
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get parent of manifest dir")
        .parent()
        .expect("Failed to get workspace root");

    // Get the Python executable with proper venv support
    let python_cmd = common::get_python_executable();

    // Get the path to the compiled cribo binary using compile-time env macro
    // This is set by cargo when building integration tests for a binary crate
    let cribo_bin = env!("CARGO_BIN_EXE_cribo");

    // Run pytest as a module using python -m pytest
    // This ensures pytest is found in the same environment as Python
    let output = Command::new(&python_cmd)
        .args([
            "-m",
            "pytest",
            "-q",                  // Quiet output
            "--tb=short",          // Short traceback format
            "ecosystem/scenarios", // Test directory
        ])
        .env("CARGO_BIN_EXE_cribo", cribo_bin)
        .current_dir(workspace_root)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "Failed to execute pytest: {e}. Python: {}",
                python_cmd.display()
            )
        });

    // Print output for debugging
    if !output.status.success() {
        eprintln!("=== PYTEST OUTPUT ===");
        eprintln!("Python executable: {}", python_cmd.display());
        eprintln!("STDOUT:\n{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("STDERR:\n{}", String::from_utf8_lossy(&output.stderr));
        eprintln!("=== END PYTEST OUTPUT ===");
    }

    assert!(
        output.status.success(),
        "Ecosystem tests failed. Python: {}. Exit code: {:?}",
        python_cmd.display(),
        output.status.code()
    );
}
