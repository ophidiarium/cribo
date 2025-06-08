use std::process::Command;

#[test]
fn test_verbose_flag_help() {
    // Test that the help text shows the correct verbose flag description
    let output = Command::new("cargo")
        .args(["run", "--bin", "cribo", "--", "--help"])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("-v, --verbose..."));
    assert!(stdout.contains("Increase verbosity (can be repeated: -v, -vv, -vvv)"));
}

#[test]
fn test_verbose_flag_parsing() {
    // Test that multiple verbose flags are accepted
    let tests = vec![
        (vec!["-v"], "single verbose flag"),
        (vec!["-vv"], "double verbose flag"),
        (vec!["-vvv"], "triple verbose flag"),
        (vec!["--verbose"], "long verbose flag"),
        (
            vec!["--verbose", "--verbose"],
            "multiple long verbose flags",
        ),
    ];

    for (verbose_args, desc) in tests {
        let mut args = vec!["run", "--bin", "cribo", "--"];
        args.extend(verbose_args);
        args.extend(&["--entry", "nonexistent.py", "--output", "out.py"]);

        let output = Command::new("cargo")
            .args(args)
            .output()
            .expect("Failed to execute command");

        // The command should fail because the entry file doesn't exist,
        // but it should parse the verbose flags without error
        assert!(!output.status.success(), "Expected failure for {}", desc);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Should not have parsing errors
        assert!(
            !stderr.contains("error: invalid value"),
            "Failed to parse {}",
            desc
        );
    }
}
