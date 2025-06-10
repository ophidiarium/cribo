#!/usr/bin/env python3
"""Test stdlib detection"""

import subprocess
import sys

# Test if typing is recognized as stdlib
result = subprocess.run(
    [
        sys.executable,
        "-c",
        """
import ruff_python_stdlib
print(ruff_python_stdlib.sys.is_known_standard_library(10, 'typing'))
""",
    ],
    capture_output=True,
    text=True,
)

if result.returncode == 0:
    print(f"Typing is stdlib: {result.stdout.strip()}")
else:
    print(f"Error: {result.stderr}")

# Also test in Rust
rust_test = """
fn main() {
    println!("typing is stdlib: {}", ruff_python_stdlib::sys::is_known_standard_library(10, "typing"));
    println!("collections is stdlib: {}", ruff_python_stdlib::sys::is_known_standard_library(10, "collections"));
    println!("Optional (not a module) is stdlib: {}", ruff_python_stdlib::sys::is_known_standard_library(10, "Optional"));
}
"""

with open("/tmp/test_stdlib.rs", "w") as f:
    f.write(rust_test)
