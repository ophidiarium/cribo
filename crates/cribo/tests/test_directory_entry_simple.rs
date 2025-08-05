#![allow(clippy::disallowed_methods)] // tempfile uses unwrap internally

use std::{env, fs, process::Command};

use tempfile::TempDir;

/// Run cribo with given arguments and return (stdout, stderr, `exit_code`)
fn run_cribo(args: &[&str]) -> (String, String, i32) {
    let cribo_exe = env!("CARGO_BIN_EXE_cribo");

    let output = Command::new(cribo_exe)
        .args(args)
        .env("RUST_LOG", "off")
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    (stdout, stderr, exit_code)
}

#[test]
fn test_directory_entry_main_py_priority() {
    let temp_dir = TempDir::new().unwrap();
    let package_dir = temp_dir.path().join("testpkg");
    fs::create_dir(&package_dir).unwrap();

    // Create __main__.py
    fs::write(
        package_dir.join("__main__.py"),
        r#"#!/usr/bin/env python3
print("Running from __main__.py")
"#,
    )
    .unwrap();

    // Create __init__.py (should be ignored when __main__.py exists)
    fs::write(
        package_dir.join("__init__.py"),
        r#"print("This is __init__.py - should not run")"#,
    )
    .unwrap();

    let output_path = temp_dir.path().join("bundled.py");

    let (_, stderr, exit_code) = run_cribo(&[
        "--entry",
        package_dir.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]);

    assert_eq!(exit_code, 0, "Bundling failed: {stderr}");
    assert!(output_path.exists());

    // Verify the bundled content
    let bundled_content = fs::read_to_string(&output_path).unwrap();
    assert!(bundled_content.contains("Running from __main__.py"));
    assert!(!bundled_content.contains("This is __init__.py"));

    // Execute and verify output
    let python_output = Command::new("python3")
        .arg(&output_path)
        .output()
        .expect("Failed to execute Python");

    let python_stdout = String::from_utf8_lossy(&python_output.stdout);
    assert!(python_output.status.success());
    assert_eq!(python_stdout.trim(), "Running from __main__.py");
}

#[test]
fn test_directory_entry_init_py_fallback() {
    let temp_dir = TempDir::new().unwrap();
    let package_dir = temp_dir.path().join("testpkg");
    fs::create_dir(&package_dir).unwrap();

    // Create only __init__.py (no __main__.py)
    fs::write(
        package_dir.join("__init__.py"),
        r#"print("Running from __init__.py")"#,
    )
    .unwrap();

    let output_path = temp_dir.path().join("bundled.py");

    let (_, stderr, exit_code) = run_cribo(&[
        "--entry",
        package_dir.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]);

    assert_eq!(exit_code, 0, "Bundling failed: {stderr}");
    assert!(output_path.exists());

    // Verify the bundled content
    let bundled_content = fs::read_to_string(&output_path).unwrap();
    assert!(bundled_content.contains("Running from __init__.py"));

    // Execute and verify output
    let python_output = Command::new("python3")
        .arg(&output_path)
        .output()
        .expect("Failed to execute Python");

    let python_stdout = String::from_utf8_lossy(&python_output.stdout);
    assert!(python_output.status.success());
    assert_eq!(python_stdout.trim(), "Running from __init__.py");
}

#[test]
fn test_directory_entry_empty_dir_error() {
    let temp_dir = TempDir::new().unwrap();
    let empty_dir = temp_dir.path().join("empty");
    fs::create_dir(&empty_dir).unwrap();

    let output_path = temp_dir.path().join("bundled.py");

    let (_, stderr, exit_code) = run_cribo(&[
        "--entry",
        empty_dir.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]);

    assert_ne!(exit_code, 0);
    assert!(stderr.contains("does not contain __main__.py or __init__.py"));
    assert!(!output_path.exists());
}
