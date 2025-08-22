use std::{path::Path, process::Command};

#[test]
#[ignore = "ecosystem test - run with --ignored"]
fn test_ecosystem_requests() {
    // Get the workspace root
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get parent of manifest dir")
        .parent()
        .expect("Failed to get workspace root");

    // Run the Python test script as a module
    let output = Command::new("python3")
        .args(["-m", "ecosystem.scenarios.test_requests"])
        .current_dir(workspace_root)
        .output()
        .expect("Failed to execute ecosystem test");

    // Print output for debugging
    if !output.status.success() {
        eprintln!("STDOUT:\n{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("STDERR:\n{}", String::from_utf8_lossy(&output.stderr));
    }

    assert!(
        output.status.success(),
        "Ecosystem test failed with exit code: {:?}",
        output.status.code()
    );
}
