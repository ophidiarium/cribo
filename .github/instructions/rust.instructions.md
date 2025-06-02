---
applyTo: "**/*.rs"
---

# Project coding standards for Rust

Apply the [general coding guidelines](./general-coding.instructions.md) to all code.

## Rust Guidelines

- Use idiomatic, modern Rust (2024 edition or later).
- Use strong typing and Rust’s safety/concurrency principles throughout.
- Use standard Rust toolchain for development: `cargo fmt` for formatting, `cargo clippy` for linting, and `cargo test` for running tests.
- Ensure usage of proper error handling, using `Option` or `Result` types where appropriate. Utilize a custom error type for the project when reasonable.
- Ensure functions are documented with comments that abide by Rust's documentation standards
- Ensure that functions are tested in a way that is consistent with the rest of the codebase
- Write testable, extensible code; prefer pure functions where possible.
- Use `async`/`await` for asynchronous code
- Use `serde` and `postcard` for serialization/deserialization of data structures
- `alloc` is available for heap allocation, but use it sparingly
- Ensure that any feature gates that are added are added to the Cargo.toml and documented
- Ensure that any dependencies that are added are added to the Cargo.toml and documented

## Development Workflow

### Available Tools for Code Quality

**TOOL PREFERENCE HIERARCHY**: Always prefer VS Code tools when available, use terminal commands only as fallback when VS Code tools don't work as expected.

#### Testing

- **Primary Tool**: `run_tests` (VS Code tool)
  - **Usage**: Preferred method for running Rust tests with integrated output and error reporting
  - **Benefits**: Integrated with VS Code's test explorer, better error formatting, file-specific test running
  - **Options**: Can run specific test files or all tests
- **Fallback Tool**: `cargo test` (terminal)
  - **Usage**: Use only when `run_tests` tool fails or is unavailable
  - **Options**: `cargo test --no-run`, `cargo test <test_name>`, etc.

#### Error Checking and Diagnostics

- **Primary Tool**: `get_errors` (VS Code tool)
  - **Usage**: Preferred method for detecting compilation errors, warnings, and diagnostics
  - **Benefits**: Integrated with rust-analyzer, real-time error detection, precise error locations
  - **Usage Pattern**: Call after making changes to verify code correctness
- **Fallback Tools**: Terminal commands
  - **Build Checking**: `cargo check` for faster error detection without building binaries
  - **Full Build**: `cargo build` for complete compilation

#### Formatting

- **VS Code Integration**: rust-analyzer provides on-save formatting automatically
- **Manual Tool**: `cargo fmt` (rustfmt)
  - **Usage**: Use when manual formatting is needed or VS Code integration fails
  - **Configuration**: See `.rustfmt.toml` for project-specific formatting rules

#### Linting

- **Primary Tool**: `get_errors` (VS Code tool)
  - **Usage**: Detects clippy warnings and other lints through rust-analyzer integration
  - **Benefits**: Real-time clippy warnings, integrated error reporting, precise locations
  - **Coverage**: Includes most clippy lints automatically enabled in VS Code
- **Supplementary Tool**: `cargo clippy` (terminal)
  - **Usage**: Use for comprehensive clippy analysis or when `get_errors` doesn't show expected warnings
  - **Options**:
    - `cargo clippy --fix` to automatically apply fixable suggestions
    - `cargo clippy --no-deps` to lint only this crate without dependencies
  - **When to use**: For batch processing, CI/CD, or accessing advanced clippy options

### Recommended Development Cycle

**Follow this VS Code-first approach:**

1. **Write Code**: Implement functionality with proper typing and error handling
2. **Check Errors**: Use `get_errors` tool to verify no compilation issues and view clippy warnings (preferred over `cargo check`/`cargo clippy`)
3. **Test**: Use `run_tests` tool to verify functionality works correctly (preferred over `cargo test`)
4. **Format**: Automatic via rust-analyzer on-save, or `cargo fmt` if manual formatting needed
5. **Advanced Linting**: Use `cargo clippy` only if you need specific clippy options not available through `get_errors`

**VS Code Tool Benefits:**

- Integrated error reporting with precise file locations
- Better formatted test output with pass/fail indicators
- Seamless integration with the development environment
- Real-time feedback during development

**When to Use Terminal Commands:**

- When VS Code tools (`run_tests`, `get_errors`) fail or produce unexpected results
- For advanced cargo commands not available through VS Code tools
- For CI/CD scripts and automated workflows
- For advanced clippy options (like `--fix`, `--no-deps`) not accessible through `get_errors`

## Documentation Research Hierarchy

**MANDATORY**: When implementing ANY functionality or researching libraries and dependencies, you MUST follow this prioritized approach in order. Do NOT skip steps unless explicitly documented why a step cannot be completed.

### 1. Local Documentation First (`cargo doc`) - REQUIRED FIRST STEP

- **MANDATORY**: Always start with generating and examining locally available documentation
- **FAILURE TO COMPLY**: If you proceed without checking local docs first, this violates project standards
- **Benefits**: Most accurate for your exact dependency versions, includes private items, works offline
- **Use for**: All Rust crates in your dependency tree, local modules and functions
- **AI Agent Process** - MUST execute these commands:
  ```bash
  cargo doc --document-private-items  # Generate comprehensive documentation
  # Then examine generated files in target/doc/
  ```
- **Alternative commands** when needed:
  ```bash
  cargo doc --package <crate-name>    # Generate docs for specific crate
  cargo doc --no-deps                 # Generate docs only for workspace crates
  ```
- **Documentation Location**: Generated docs are in `target/doc/` directory as HTML files
- **Access Method**: Use `read_file` tool to examine the generated HTML documentation files
- **REQUIREMENT**: You MUST explicitly state what you found (or didn't find) in local docs before proceeding

### 2. Context7 for External Libraries - SECOND STEP ONLY

- **When permitted**: ONLY after local documentation proves insufficient or dependency is not locally available
- **MUST DOCUMENT**: Why local docs were insufficient before using Context7
- **Benefits**: Comprehensive documentation from official sources, handles multiple languages
- **Process**: Resolve library ID first with `f1e_resolve-library-id`, then get focused documentation with `f1e_get-library-docs`
- **Best for**: External APIs, libraries not in your dependency graph, cross-language references
- **REQUIREMENT**: You MUST document what specific gaps Context7 filled that local docs couldn't

### 3. GitHub MCP Server Tools - FINAL STEP ONLY

- **When permitted**: ONLY when documentation is unclear, need implementation examples, or troubleshooting edge cases AFTER exhausting steps 1 and 2
- **MUST DOCUMENT**: Why both local docs AND Context7 were insufficient before using GitHub search
- **Benefits**: Real-world usage patterns, issue resolution examples, source code understanding
- **Focus on**: High-quality repositories (especially astral-sh/ruff, astral-sh/uv for Python/Rust patterns), official examples, recent implementations
- **Available GitHub MCP Tools**:
  - `github_repo` - Search specific repositories for code snippets
  - `f1e_search_code` - Search code across GitHub repositories
  - `f1e_get_file_contents` - Retrieve file contents from GitHub repositories
  - `f1e_search_repositories` - Find relevant repositories
  - `f1e_search_issues` - Search issues for problem-solving patterns
- **Search strategy**: Look for actual usage patterns rather than just reading source code
- **REQUIREMENT**: You MUST document which specific implementation patterns you discovered

### MANDATORY Research Process - FOLLOW IN ORDER

1. **STEP 1 - Local Documentation (REQUIRED)**:
   - Execute: `cargo doc --document-private-items`
   - Examine: Generated files in `target/doc/` using `read_file`
   - Document: What you found or why it's insufficient
   - **DO NOT PROCEED** until this step is completed and documented

2. **STEP 2 - Context7 (If Step 1 Insufficient)**:
   - Document: Why local docs didn't provide what you need
   - Execute: `f1e_resolve-library-id` then `f1e_get-library-docs`
   - Document: What specific gaps Context7 filled

3. **STEP 3 - GitHub MCP (If Steps 1&2 Insufficient)**:
   - Document: Why both local docs and Context7 were insufficient
   - Use: GitHub repository search tools for implementation patterns and edge cases
   - Document: Which specific patterns or solutions you discovered

4. **MANDATORY Documentation**:
   - **ALWAYS** note which approach provided the key insights
   - **ALWAYS** document why you proceeded to each step
   - **VIOLATION**: Proceeding to later steps without documenting previous steps is non-compliant

### ENFORCEMENT

**For AI Agents**: You MUST explicitly follow this hierarchy and document each step. Failure to do so violates project coding standards and will require rework.

## Snapshot Testing with Insta

This project uses `insta` for snapshot testing. Snapshots are a way to test that the output of a function or a piece of code remains consistent over time.

- When a test using `insta` is run for the first time, or when the output changes, a new snapshot file (e.g., `.snap`) will be created or updated.
- To accept new or updated snapshots, ALWAYS use:
  ```bash
  cargo insta accept
  ```
- DO NOT USE `cargo insta review` as that is an interactive command requiring a human input
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
