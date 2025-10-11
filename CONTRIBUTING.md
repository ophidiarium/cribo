# Contributing to Cribo

We welcome contributions to Cribo! Please follow these guidelines to ensure a smooth development process.

## Development Setup

```bash
# Clone the repository
git clone https://github.com/ophidiarium/cribo.git
cd cribo

# Install Rust toolchain and components
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov

# Build Rust CLI
cargo build --release

# Build Python package
pip install maturin
maturin develop

# Run tests
cargo test
```

## Code Coverage

The project uses `cargo-llvm-cov` for code coverage analysis:

```bash
# Generate text coverage report (Istanbul-style)
cargo coverage-text

# Generate HTML coverage report and open in browser
cargo coverage

# Generate LCOV format for CI
cargo coverage-lcov

# Clean coverage data
cargo coverage-clean
```

**Branch Coverage (Experimental)**:

```bash
# Requires nightly Rust for branch coverage
cargo +nightly coverage-branch
```

Coverage reports are automatically generated in CI and uploaded to Codecov. See [`docs/coverage.md`](docs/coverage.md) for detailed coverage documentation.

**Note**: If you see zeros in the "Branch Coverage" column in HTML reports, this is expected with stable Rust. Branch coverage requires nightly Rust and is experimental.

## Performance Benchmarking

Cribo uses [Bencher.dev](https://bencher.dev) for comprehensive performance tracking with statistical analysis and regression detection:

```bash
# Run all benchmarks
cargo bench

# Save a performance baseline
./scripts/bench.sh --save-baseline main

# Compare against baseline
./scripts/bench.sh --baseline main

# View detailed HTML report
./scripts/bench.sh --open
```

**Key benchmarks:**

- **End-to-end bundling**: Full project bundling performance (Criterion.rs)
- **AST parsing**: Python code parsing speed (Criterion.rs)
- **Module resolution**: Import resolution efficiency (Criterion.rs)
- **CLI performance**: Command-line interface speed (Hyperfine)

**CI Integration:**

- Automated PR comments with performance comparisons and visual charts
- Historical performance tracking with trend analysis
- Statistical significance testing to prevent false positives
- Dashboard available at [bencher.dev/perf/cribo](https://bencher.dev/perf/cribo)

See [docs/benchmarking.md](docs/benchmarking.md) for detailed benchmarking guide.

## Contributing Guidelines

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Submit a pull request
