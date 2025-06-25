# Ecosystem Testing for Cribo

This directory contains real-world Python packages used to validate Cribo's bundling capabilities.

## Overview

Ecosystem tests ensure Cribo can successfully bundle popular Python packages while maintaining their functionality. Each package includes smoke tests that verify the bundled output behaves identically to the original.

## Structure

```
ecosystem/
├── packages/              # Git submodules of test packages
│   └── requests/          # HTTP library
├── scenarios/             # Test scenarios for each package
│   ├── __init__.py        # Package marker
│   ├── utils.py           # Shared utility functions
│   └── test_requests.py   # Smoke tests for requests
└── benchmarks/            # Performance benchmarks
```

### Shared Utilities

The `ecosystem/scenarios/utils.py` module provides common functions used across all test scenarios:

- `ensure_test_directories()` - Creates necessary test directories (e.g., target/tmp)
- `run_cribo()` - Executes cribo bundler with configurable options
- `load_bundled_module()` - Context manager for safely loading bundled modules
- `format_bundle_size()` - Formats file sizes in human-readable format
- `run_bundled_test()` - Runs test scripts with bundled modules (for string-based tests)

## Running Tests

### Prerequisites

First, build cribo in release mode:

```bash
cargo build --release --bin cribo
```

### All Ecosystem Tests

```bash
# Run the Rust integration test
cargo test --test test_ecosystem -- --ignored --nocapture

# Or run Python test directly as a module
python -m ecosystem.scenarios.test_requests
```

Note: The test scripts automatically find the cribo executable in `target/release/`. If not found, they fall back to using `cribo` from PATH.

### Benchmarks

```bash
# Run ecosystem bundling benchmarks
cargo bench --bench ecosystem

# Save baseline for comparison
cargo bench --bench ecosystem -- --save-baseline main

# Compare against baseline
cargo bench --bench ecosystem -- --baseline main
```

## Adding New Packages: Step-by-Step Guide

Follow these steps to add a new package to the ecosystem test suite:

### 1. Add Package as Git Submodule

First, add the package's repository as a git submodule. Use a specific tag/release for reproducibility:

```bash
# Add the submodule
git submodule add https://github.com/psf/requests.git ecosystem/packages/requests

# Navigate to the submodule
cd ecosystem/packages/requests

# Checkout a specific version tag
git checkout v2.32.4

# Return to project root
cd ../../..

# Commit the submodule addition
git add .gitmodules ecosystem/packages/requests
git commit -m "Add requests v2.32.4 to ecosystem tests"
```

### 2. Update Project Dependencies

Edit `pyproject.toml` to include the new package in the ecosystem dependency group:

```toml
# dependencies for ecosystem testing
ecosystem = [
    "requests>=2.32.0",
    "rich>=13.7.0", # Add your new package here
]
```

Then sync dependencies:

```bash
uv sync
```

### 3. Create Test Scenario

Create a new test file `ecosystem/scenarios/test_<package>.py`. Use this template:

```python
#!/usr/bin/env python3
"""Test scenario for bundled <package> library."""

import sys
from pathlib import Path
from types import ModuleType
from typing import TYPE_CHECKING

from utils import run_cribo, format_bundle_size, load_bundled_module, ensure_test_directories

# Type hint for better IDE support (if package is in dependencies)
if TYPE_CHECKING:
    import <package> as <package>_type


def run_smoke_tests(<package>: "ModuleType | <package>_type"):
    """Run smoke tests using the bundled <package> module.
    
    Args:
        <package>: The dynamically imported bundled module
    """
    print("🧪 Running smoke tests...")
    
    # Add your package-specific tests here
    # Example for rich:
    # from rich.console import Console
    # console = <package>.console.Console()
    # console.print("[bold red]Hello[/bold red] World!")
    
    # Example for httpx:
    # client = <package>.Client()
    # response = client.get("https://httpbin.org/get")
    # assert response.status_code == 200
    
    print("\n✅ All smoke tests passed!")


def test_<package>_bundled():
    """Test the bundled <package> library."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()
    
    # Adjust path based on package structure
    package_init = Path("ecosystem/packages/<package>/src/<package>/__init__.py")
    bundled_output = tmp_dir / "<package>_bundled.py"
    
    print("🔧 Bundling <package> library...")
    result = run_cribo(
        str(package_init), 
        str(bundled_output),
        emit_requirements=True,
        tree_shake=True  # Set to False if tree-shaking causes issues
    )
    
    if result.returncode != 0:
        sys.exit(1)
    
    bundle_size = bundled_output.stat().st_size
    print(f"✅ Successfully bundled to {bundled_output}")
    print(f"   Bundle size: {format_bundle_size(bundle_size)}")
    
    # Run smoke tests by importing the bundled module
    print("\n🧪 Running smoke tests with bundled library...")
    
    try:
        with load_bundled_module(bundled_output, "<package>_bundled") as module:
            run_smoke_tests(module)
    except Exception as e:
        print(f"❌ Failed to load or test bundled module: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)
    
    return True


def main():
    """Main entry point."""
    print("🚀 Ecosystem test: <package>")
    print("=" * 50)
    
    try:
        test_<package>_bundled()
        print("\n✅ Ecosystem test completed successfully!")
        return 0
    except Exception as e:
        print(f"\n❌ Ecosystem test failed: {e}")
        import traceback
        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
```

### 4. Update Rust Integration Test

Edit `crates/cribo/tests/test_ecosystem.rs` to add a new test function:

```rust
#[test]
#[ignore = "ecosystem test - run with --ignored"]
fn test_ecosystem_<package>() {
    // Get the workspace root
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get parent of manifest dir")
        .parent()
        .expect("Failed to get workspace root");

    let test_script = workspace_root.join("ecosystem/scenarios/test_<package>.py");

    // Run the Python test script
    let output = Command::new("python3")
        .arg(&test_script)
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
```

### 5. Update Benchmark Configuration

Edit `crates/cribo/benches/ecosystem.rs` to include the new package:

```rust
// In bundle_ecosystem_package function, add entry point mapping:
let entry_point = match package_name {
    "requests" => workspace_root.join("ecosystem/packages/requests/src/requests/__init__.py"),
    "rich" => workspace_root.join("ecosystem/packages/rich/src/rich/__init__.py"),  // Add this
    _ => panic!("Unknown package: {}", package_name),
};

// Update the config.src path accordingly:
config.src.push(match package_name {
    "requests" => workspace_root.join("ecosystem/packages/requests/src"),
    "rich" => workspace_root.join("ecosystem/packages/rich/src"),  // Add this
    _ => panic!("Unknown package: {}", package_name),
});

// In benchmark_ecosystem_bundling function, add to packages array:
let packages = ["requests", "rich"];  // Add new package here
```

### 6. Verify Setup

Run these commands to verify everything works:

```bash
# Build cribo in release mode
cargo build --release --bin cribo

# Install dependencies
uv sync

# Run the Python test directly as a module
python -m ecosystem.scenarios.test_<package>

# Run the Rust integration test
cargo test --test test_ecosystem test_ecosystem_<package> -- --ignored --nocapture

# Run benchmarks
cargo bench --bench ecosystem
```

### 7. Update Documentation

Update this README's "Current Packages" section with:

- Package name and version
- Brief description of what the package does
- List of smoke tests performed
- Current status (passing/failing and any known issues)

### 8. Commit Changes

```bash
git add -A
git commit -m "Add <package> to ecosystem tests

- Add <package> v<version> as git submodule
- Create smoke tests for core functionality
- Add benchmark integration
- Update documentation"
```

## Test Scenarios

Each test scenario:

1. Bundles the package using Cribo
2. Runs functional tests using the bundled version
3. Verifies output matches expected behavior

### Current Packages

- **requests** (v2.32.4): Popular HTTP library
  - Tests GET/POST requests
  - Verifies headers and parameters
  - Tests timeout handling
  - Validates status codes
  - Verifies requirements.txt generation (detects urllib3, idna, charset_normalizer, plus optional deps)
  - **Status**: Requirements generation ✅, Code execution ❌ (fails due to relative import bug: `from . import sessions`)

## CI Integration

Ecosystem tests run automatically on:

- Every pull request affecting relevant paths
- Pushes to main branch
- Manual workflow dispatch

Results are posted as PR comments including:

- Test status
- Benchmark comparisons
- Bundle size metrics
