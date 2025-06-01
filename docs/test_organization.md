# Test File Organization

This document describes the reorganization of test files in the Serpen project.

## Test Structure

### Unit Tests

Located in `crates/serpen/src/` within each module file:

- `emit.rs` - Contains comprehensive unit tests for import rewriting and processing
- `resolver.rs` - Contains tests for module resolution
- `unused_imports_simple.rs` - Contains tests for unused import detection

### Integration Tests

Located in `crates/serpen/tests/`:

- `integration_tests.rs` - Main integration tests for bundling functionality, utilizing snapshot testing (`insta`) to verify outputs.
- `test_relative_imports.rs` - Tests for relative import resolution

#### Snapshot Testing in Integration Tests

- Snapshot files for integration tests are stored in `crates/serpen/tests/snapshots/`.
- These snapshots help ensure that the output of certain operations (like module resolution or code generation) remains consistent across changes.
- To review changes to snapshots, use the command:
  ```bash
  cargo insta review
  ```
- To accept new or updated snapshots, use:
  ```bash
  cargo insta accept
  ```
- It's crucial to ensure that data being snapshotted is deterministic (e.g., by sorting collections like HashSets or HashMaps before serialization) to avoid flaky tests.

### Test Fixtures

Located in `crates/serpen/tests/fixtures/`:

#### Alias Import Tests

- `alias_imports/` - Test files for aliased import functionality
  - `test_alias_imports.py` - Complex alias import scenarios
  - `test_simple_alias.py` - Simple alias import test
  - `test_module.py` - Shared module for alias tests

#### Import Filtering Tests

- `import_filtering/` - Test files for import filtering functionality
  - `test_import_filtering.py` - Import filtering scenarios
  - `test_rewrite_import.py` - Import rewriting tests

#### Output Files

- `outputs/` - Generated bundle output files for verification
  - `test_alias_output.py` - Output from alias import bundling
  - `test_simple_alias_output.py` - Output from simple alias bundling
  - `test_output.py` - General test output file

#### Other Test Files

- `test_working_example.py` - Working example test file
- Various project fixtures for integration testing

## Previous Organization Issues

Before this reorganization, test files were scattered in the root directory:

- `test_*.py` files cluttered the root workspace
- `test_*.rs` files were improperly structured as integration tests when they should have been unit tests
- Test files attempted to access private methods and APIs

## Improvements Made

1. **Cleaner Root Directory**: Removed all test files from the root directory
2. **Proper Test Structure**: Organized tests according to Rust best practices
3. **Logical Grouping**: Related test files are grouped together in subdirectories
4. **Removed Broken Tests**: Eliminated test files that attempted to use private APIs
5. **Maintained Functionality**: All existing functionality remains fully tested through proper unit and integration tests

## Running Tests

```bash
# Run all tests
cargo test

# Run specific test categories
cargo test --lib           # Unit tests only
cargo test --test '*'      # Integration tests only
```

All 19 tests (13 unit + 5 integration + 1 relative import) pass successfully.
