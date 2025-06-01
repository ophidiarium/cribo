---
applyTo: "**/*.rs"
---

# Project coding standards for Rust

Apply the [general coding guidelines](./general-coding.instructions.md) to all code.

## Rust Guidelines

- Use idiomatic, modern Rust (2024 edition or later).
- Use strong typing and Rust’s safety/concurrency principles throughout.
- Use `rust-lang.rust-analyzer` for formatting, linting and running tests.
- Ensure usage of proper error handling, using `Option` or `Result` types where appropriate. Utilize a custom error type for the project when reasonable.
- Ensure functions are documented with comments that abide by Rust's documentation standards
- Ensure that functions are tested in a way that is consistent with the rest of the codebase
- Write testable, extensible code; prefer pure functions where possible.
- Use `async`/`await` for asynchronous code
- Use `serde` and `postcard` for serialization/deserialization of data structures
- `alloc` is available for heap allocation, but use it sparingly
- Ensure that any feature gates that are added are added to the Cargo.toml and documented
- Ensure that any dependencies that are added are added to the Cargo.toml and documented

## Snapshot Testing with Insta

This project uses `insta` for snapshot testing. Snapshots are a way to test that the output of a function or a piece of code remains consistent over time.

- When a test using `insta` is run for the first time, or when the output changes, a new snapshot file (e.g., `.snap`) will be created or updated.
- To review changes to snapshots, use:
  ```bash
  cargo insta review
  ```
- To accept new or updated snapshots, use:
  ```bash
  cargo insta accept
  ```
- Ensure that snapshot files are committed to the repository along with the code changes.
- When snapshot tests fail due to non-deterministic output (e.g., from iterating over `HashSet`s or `HashMap`s), modify the code or the test to produce a stable, sorted output before snapshotting.

## Logging Guidelines

- Always use structured logging instead of `println!` for debug output: `use log::{debug, info, warn, error};`
- Use appropriate log levels:
  - `debug!()` for detailed diagnostic information useful during development
  - `info!()` for general information about program execution
  - `warn!()` for potentially problematic situations
  - `error!()` for error conditions that should be addressed
- If debug logging was essential to find a bug in the codebase, that logging should be kept in the codebase at the appropriate log level to aid future debugging
- Avoid temporary `println!` statements - replace them with proper logging before committing code
- Use structured logging with context where helpful: `debug!("Processing file: {}", file_path)`

## Test Coverage Requirements

### Coverage Monitoring for Features

**MANDATORY**: Before implementing any significant feature (new modules, major functions, or substantial logic changes), always:

1. **Baseline Coverage Check**:
   ```bash
   cargo coverage-text  # Get current coverage baseline
   ```
   Document the current coverage percentages for affected files.

2. **Implementation with Tests**:
   - Write tests alongside implementation (TDD approach preferred)
   - Ensure new code paths are covered by tests
   - Add both unit tests and integration tests as appropriate

3. **Post-Implementation Coverage Verification**:
   ```bash
   cargo coverage-text  # Check coverage after implementation
   ```
   **REQUIREMENT**: Coverage must not drop by more than 2% for any file or overall project.

4. **Coverage Quality Standards**:
   - **New files**: Aim for >90% line coverage
   - **Modified files**: Maintain existing coverage level (±2%)
   - **Critical paths**: Ensure 100% coverage for error handling and edge cases
   - **Branch coverage**: Use `cargo +nightly coverage-branch-text` to verify conditional logic is tested

### Coverage Commands Reference

```bash
# Standard coverage reports
cargo coverage-text           # Istanbul-style text report
cargo coverage               # HTML report with browser
cargo coverage-lcov          # LCOV format

# Branch coverage (more comprehensive)
cargo +nightly coverage-branch-text  # Text with branch coverage
cargo +nightly coverage-branch       # HTML with branch coverage

# Coverage cleanup
cargo coverage-clean         # Clean coverage data
```

### Coverage Failure Response

If coverage drops significantly (>2%):

1. **Identify uncovered code**: Use `cargo coverage` HTML report to see missed lines
2. **Add missing tests**: Focus on the red/uncovered lines in the HTML report
3. **Re-verify coverage**: Run coverage again to confirm improvement
4. **Document exceptions**: If coverage cannot be maintained, document why in code comments

### Integration with Development Workflow

- **Before starting feature work**: `cargo coverage-text > baseline_coverage.txt`
- **During development**: Write tests as you implement each function/method
- **Before committing**: Verify coverage meets requirements
- **In PRs**: GitHub Actions will automatically generate branch coverage reports
