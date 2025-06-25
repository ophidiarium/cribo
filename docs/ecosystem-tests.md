# Ecosystem Testing for Cribo

## Overview

This document outlines the design and implementation of ecosystem testing for Cribo, which validates the bundler against real-world Python packages. The system tests Cribo's ability to bundle popular pure Python packages and verifies the bundled output maintains functional equivalence through smoke tests.

## Goals

1. **Validation**: Ensure Cribo can successfully bundle real-world Python packages
2. **Regression Detection**: Catch breaking changes early by testing against established packages
3. **Performance Tracking**: Monitor bundling performance across different package complexities
4. **CI Integration**: Automated testing with PR performance comparisons

## Package Selection

Initial packages chosen for diversity in complexity and usage patterns:

- **requests** - HTTP library with complex module structure
- **rich** - Terminal formatting with dynamic imports
- **idna** - Internationalized domain names with data files
- **pyyaml** - YAML parser with optional C extensions
- **httpx** - Modern HTTP client with async support

## Architecture

### Directory Structure

```
ecosystem/
├── packages/              # Git submodules for test packages
│   ├── requests/
│   ├── rich/
│   ├── idna/
│   ├── pyyaml/
│   └── httpx/
├── scenarios/            # Test scenarios for each package
│   ├── test_requests.py
│   ├── test_rich.py
│   ├── test_idna.py
│   ├── test_pyyaml.py
│   └── test_httpx.py
├── benchmarks/          # Benchmark configurations
│   └── ecosystem_bench.rs
└── README.md           # Ecosystem testing guide
```

### Components

#### 1. Package Management

- Packages added as git submodules pinned to specific versions
- Dependencies installed in project virtual environment
- Version tracking in `.gitmodules` for reproducibility

#### 2. Test Scenarios

Each package has a dedicated test scenario that:

- Bundles the package using Cribo
- Executes package-specific functionality tests
- Compares output between original and bundled versions

Example scenarios:

- **requests**: Make HTTP GET/POST to httpbin.org, verify response
- **rich**: Generate formatted console output, capture and compare
- **idna**: Encode/decode international domain names
- **pyyaml**: Parse and dump YAML documents
- **httpx**: Async HTTP requests with response validation

#### 3. Benchmark Integration

Extends existing benchmark infrastructure:

```rust
// crates/cribo/benches/ecosystem_bench.rs
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_ecosystem_bundling(c: &mut Criterion) {
    let packages = ["requests", "rich", "idna", "pyyaml", "httpx"];

    for package in packages {
        c.bench_function(&format!("bundle_{}", package), |b| {
            b.iter(|| bundle_ecosystem_package(package))
        });
    }
}
```

#### 4. CI Integration

GitHub Actions workflow:

- Runs on every PR
- Executes bundling benchmarks
- Posts performance comparison as PR comment
- Fails if any smoke test fails

## Implementation Plan

### Phase 1: Infrastructure Setup

1. Create `ecosystem/` directory structure
2. Add git submodules for target packages
3. Update project dependencies to include ecosystem packages
4. Create base test framework

### Phase 2: Test Scenarios

Implement test scenarios for each package:

```python
# ecosystem/scenarios/test_requests.py
import subprocess
import json
import sys
from pathlib import Path

def test_requests_bundled():
    # Bundle requests
    result = subprocess.run([
        "cribo",
        "--entry", "ecosystem/packages/requests/src/requests/__init__.py",
        "--output", "target/tmp/requests_bundled.py"
    ], capture_output=True)
    
    assert result.returncode == 0
    
    # Run smoke test
    test_script = """
import sys
sys.path.insert(0, 'target/tmp')
import requests_bundled as requests

# Basic GET request
resp = requests.get('https://httpbin.org/get')
assert resp.status_code == 200
assert 'headers' in resp.json()

# POST with data
resp = requests.post('https://httpbin.org/post', json={'key': 'value'})
assert resp.json()['json'] == {'key': 'value'}

print("✓ All requests tests passed")
"""
    
    result = subprocess.run([sys.executable, "-c", test_script], capture_output=True)
    assert result.returncode == 0
```

### Phase 3: Benchmark Integration

1. Create `ecosystem_bench.rs` in existing benchmark structure
2. Implement bundling time measurements
3. Add memory usage tracking
4. Generate criterion reports

### Phase 4: CI Pipeline

```yaml
# .github/workflows/ecosystem-tests.yml
name: Ecosystem Tests

on:
  pull_request:
  push:
    branches: [main]

jobs:
  ecosystem-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive

      - name: Setup Python
        uses: actions/setup-python@v4
        with:
          python-version: '3.12'

      - name: Install dependencies
        run: |
          python -m pip install -e ecosystem/packages/requests
          python -m pip install -e ecosystem/packages/rich
          # ... other packages

      - name: Run ecosystem tests
        run: cargo test --test ecosystem_tests

      - name: Run benchmarks
        run: cargo bench --bench ecosystem_bench -- --save-baseline pr-${{ github.event.pull_request.number }}

      - name: Compare benchmarks
        if: github.event_name == 'pull_request'
        run: ./scripts/ecosystem-bench-compare.sh

      - name: Post results
        if: github.event_name == 'pull_request'
        uses: actions/github-script@v6
        with:
          script: |
            const results = require('./target/ecosystem-results.json');
            github.rest.issues.createComment({
              issue_number: context.issue.number,
              owner: context.repo.owner,
              repo: context.repo.repo,
              body: results.markdown
            });
```

## Local Development

### Running Tests

```bash
# Run all ecosystem tests
cargo test --test ecosystem_tests

# Run specific package test
cargo test --test ecosystem_tests test_requests

# Run with verbose output
RUST_LOG=debug cargo test --test ecosystem_tests
```

### Running Benchmarks

```bash
# Run ecosystem benchmarks
cargo bench --bench ecosystem_bench

# Save baseline for comparison
cargo bench --bench ecosystem_bench -- --save-baseline main

# Compare against baseline
cargo bench --bench ecosystem_bench -- --baseline main
```

### Adding New Packages

1. Add submodule: `git submodule add https://github.com/org/package ecosystem/packages/package`
2. Create test scenario in `ecosystem/scenarios/test_package.py`
3. Add to benchmark suite in `ecosystem_bench.rs`
4. Update CI workflow dependencies

## Performance Metrics

Track for each package:

- **Bundling time**: Wall clock time to complete bundling
- **Memory usage**: Peak RSS during bundling
- **Output size**: Bundled file size vs original
- **Module count**: Number of modules processed

## Success Criteria

1. All packages bundle without errors
2. Smoke tests pass for bundled output
3. Performance remains within 10% of baseline
4. No memory usage regression >20%
5. Bundle size reasonable (<2x original)

## Future Enhancements

1. **Extended Package Set**: Add more complex packages (Django, Flask, NumPy-stubs)
2. **Hyperfine Integration**: Use hyperfine for more robust benchmarking
3. **Coverage Analysis**: Track which Cribo features each package exercises
4. **Error Recovery**: Test handling of unsupported patterns
5. **Optimization Validation**: Verify tree-shaking effectiveness per package
6. **Tree-shaking Metrics**: Measure percentage of code eliminated when tree-shaking is enabled

## Maintenance

- Review package versions quarterly
- Update smoke tests as packages evolve
- Monitor for new popular pure Python packages
- Track bundling success rate over time
- Document any package-specific workarounds needed
