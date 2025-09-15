# AGENTS.md

This file provides guidance to [OpenAI codex](https://github.com/openai/codex) when working with code in this repository.

## 🛠️ PROJECT TECHNICAL DETAILS

### Project Overview

Cribo is a Python source bundler written in Rust that produces a single .py file from a multi-module Python project by inlining first-party source files. It's available as a CLI tool.

Key features:

- Tree-shaking to include only needed modules
- Unused import detection and trimming
- Requirements.txt generation
- Configurable import classification

### Build Commands

#### Rust Binary

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run the tool directly
cargo run -- --entry path/to/main.py --output bundle.py

# Output to stdout for debugging (no temporary files)
cargo run -- --entry path/to/main.py --stdout

# Run with verbose output for debugging
cargo run -- --entry path/to/main.py --output bundle.py -vv

# Run with trace-level output for detailed debugging
cargo run -- --entry path/to/main.py --output bundle.py -vvv

# Combine stdout output with verbose logging for development
cargo run -- --entry path/to/main.py --stdout -vv
```

### CLI Usage

```bash
cribo --entry src/main.py --output bundle.py [options]

# Output to stdout instead of file (ideal for debugging)
cribo --entry src/main.py --stdout [options]

# Common options
--emit-requirements    # Generate requirements.txt with third-party dependencies
-v, --verbose...       # Increase verbosity (can be repeated: -v, -vv, -vvv)
                       # No flag: warnings/errors only
                       # -v: informational messages  
                       # -vv: debug messages
                       # -vvv: trace messages
--config               # Specify custom config file path
--target-version       # Target Python version (e.g., py38, py39, py310, py311, py312, py313)
--stdout               # Output bundled code to stdout instead of a file
```

The verbose flag is particularly useful for debugging bundling issues. Each level provides progressively more detail about the bundling process, import resolution, and dependency graph construction.

The `--stdout` flag is especially valuable for debugging workflows as it avoids creating temporary files and allows direct inspection of the bundled output. All log messages are properly separated to stderr, making it perfect for piping to other tools or quick inspection.

### Testing Commands

```bash
# Run all tests
cargo nextest run --workspace

# Run with code coverage
cargo llvm-cov nextest --workspace --json
```

#### Snapshot Testing with Insta

Accept new or updated snapshots using:

```bash
cargo insta accept
```

### Architecture Overview

The project is organized as a Rust workspace with the main crate in `crates/cribo`.

#### Key Components

1. **Bundle Orchestration** (`orchestrator.rs`)
   - Coordinates the entire bundling workflow
   - Manages module discovery and dependency resolution
   - Handles circular dependency detection using Tarjan's algorithm
   - Calls the code generator for final output

2. **Code Generation** (`code_generator.rs`)
   - Implements the sys.modules-based bundling approach
   - Generates deterministic module names using content hashing
   - Performs AST transformations and import rewriting
   - Integrates unused import trimming
   - Produces the final bundled Python output

3. **Module Resolution & Import Classification** (`resolver.rs`)
   - Classifies imports as standard library, first-party, or third-party
   - Resolves actual file paths for bundling
   - Handles PYTHONPATH and VIRTUAL_ENV support

4. **Dependency Graph** (`dependency_graph.rs`)
   - Builds a directed graph of module dependencies
   - Uses topological sorting to determine bundling order
   - Implements Tarjan's SCC algorithm for circular dependency detection

5. **Unused Import Detection** (`unused_imports.rs`)
   - Detects and removes unused imports
   - Handles various import formats (simple, from, aliased)
   - Operates directly on AST to avoid double parsing

#### Generic Snapshot Testing Framework (REUSE FOR NEW FEATURES)

**MANDATORY**: Before implementing custom test logic for bundling features, **ALWAYS** evaluate if the existing generic snapshot testing framework can be used or extended. This framework provides comprehensive testing with minimal implementation effort.

**Framework Location**: `crates/cribo/tests/test_bundling_snapshots.rs`

**How It Works**:

- **Automatic Discovery**: Scans `crates/cribo/tests/fixtures/` for test directories
- **Convention-Based**: Each directory with `main.py` becomes a test case automatically
- **Dual Snapshots**: Generates both bundled code and execution result snapshots
- **Deterministic**: All output is sorted and reproducible across runs

**Usage Pattern**:

```bash
# Create fixture directory
mkdir crates/cribo/tests/fixtures/my_new_feature
# Add test files (main.py + any supporting modules)
echo "print('Hello Feature')" > crates/cribo/tests/fixtures/my_new_feature/main.py

# Run a specific fixture using environment variable
INSTA_GLOB_FILTER="**/my_new_feature/main.py" cargo nextest run --workspace --test test_bundling_snapshots --cargo-quiet

# Run all fixtures matching a pattern
INSTA_GLOB_FILTER="**/future_imports_*/main.py" cargo nextest run --workspace --test test_bundling_snapshots --cargo-quiet

# Run fixture with debug output to see which fixture is running
INSTA_GLOB_FILTER="**/my_new_feature/main.py" cargo nextest run --workspace --no-capture --test test_bundling_snapshots --cargo-quiet

# List available fixtures (useful for finding fixture names)
find crates/cribo/tests/fixtures -name "main.py" -type f | sed 's|.*/fixtures/||' | sed 's|/main.py||' | sort

# Accept snapshots
cargo insta accept
```

**Generated Snapshots**:

- **`bundled_code@my_new_feature.snap`**: Clean Python code showing bundling structure
- **`execution_results@my_new_feature.snap`**: Structured execution results with status/output

**When to Use This Framework**:

- ✅ **New bundling features** (import handling, transformations, etc.)
- ✅ **Regression testing** for existing functionality
- ✅ **Integration testing** requiring end-to-end bundling + execution
- ✅ **Cross-platform validation** (consistent Python output)

**When NOT to Use**:

- ❌ **Unit tests** for individual functions (use direct unit tests)
- ❌ **Parser-only testing** (use AST unit tests)
- ❌ **Error condition testing** (use targeted error tests)

**Framework Benefits**:

- 🎯 **Zero Code Required**: Add fixture directory → get comprehensive tests
- 📸 **Dual Verification**: Both bundling correctness AND runtime behavior
- 🔄 **Automatic Maintenance**: New fixtures auto-discovered, no test code updates
- 🐛 **Excellent Debugging**: Separate snapshots pinpoint bundling vs execution issues
- 📊 **Great Diffs**: insta provides excellent change visualization
- 🚀 **Scales Infinitely**: Supports unlimited test cases with no code growth

**Snapshot Technology**:

- **Bundled Code**: Uses `insta::assert_snapshot!` for clean Python code
- **Execution Results**: Uses `insta::assert_debug_snapshot!` with structured `ExecutionResults` type
- **Named Snapshots**: Uses `insta::with_settings!` for organized, fixture-specific snapshots

**Example Fixture Structure**:

```text
crates/cribo/tests/fixtures/
├── future_imports_basic/          # Complex nested packages + future imports
│   ├── main.py
│   └── mypackage/
│       ├── __init__.py
│       ├── core.py
│       └── submodule/...
├── future_imports_multiple/       # Multiple future features + deduplication  
│   ├── main.py
│   ├── module_a.py
│   └── module_b.py
└── simple_math/                   # Basic bundling without special features
    ├── main.py
    ├── calculator.py
    └── utils.py
```

**MANDATORY Practice**: When implementing ANY new bundling feature:

1. **First**: Create fixture directory showcasing the feature
2. **Second**: Run snapshot tests to establish baseline
3. **Third**: Implement feature with snapshot-driven development
4. **Fourth**: Verify snapshots show correct bundling + execution

This approach provides **comprehensive validation with minimal effort** and creates **excellent regression protection** for all bundling functionality.

#### General Coding Standards

- Follow Rust idiomatic practices and use the Rust 2024 edition or later
- Use strong typing and leverage Rust's safety principles
- Write testable, extensible code; prefer pure functions where possible
- Ensure all functions are properly documented with Rust doc comments
- Take the opportunity to refactor code to improve readability and maintainability

#### Prohibited Coding Practice: Hardcoding Test Values in Production

- **Never** insert hardcoded literals in production code solely to satisfy a test.
- All production logic must implement genuine functionality; tests should validate real behavior, not bypass it.
- If you need to simulate or stub behavior for testing, use dedicated test files or mocking frameworks—do **not** alter production code.
- Any attempt to hardcode a test value in production code is strictly forbidden and should be treated as a critical violation.
- Violations of this policy must be reported and the offending code reverted immediately.

#### Agent Directive: Enforce `.clippy.toml` Disallowed Lists

- **Before generating, editing, or refactoring any Rust code**, automatically locate and parse the project's `.clippy.toml` file.
- Extract the arrays under `disallowed-types` and `disallowed-methods`. Treat each listed `path` or `method` as an absolute prohibition.
- **Never** emit or import a type identified in `disallowed-types`. For example, if `std::collections::HashSet` appears in the list, do not generate any code that uses it—use the approved alternative (e.g., `indexmap::IndexSet`) instead.
- **Never** invoke or generate code calling a method listed under `disallowed-methods`. If a method is disallowed, replace it immediately with the approved pattern or API.
- If any disallowed type or method remains in the generated code, **treat it as a critical error**: halt code generation for that snippet, annotate the violation with the specific reason from `.clippy.toml`, and refuse to proceed until the violation is removed.
- Continuously re-validate against `.clippy.toml` whenever generating new code or applying automated fixes—do not assume a one-time check is sufficient.
- Log each check and violation in clear comments or warnings within the pull request or code review context so that maintainers immediately see why a disallowed construct was rejected.

#### Immediate Code Removal Over Deprecation

**MANDATORY**: Since cribo only exposes a binary CLI interface (not a library API), unused methods and functions MUST be removed immediately rather than annotated with deprecation markers.

- **No deprecation annotations**: Do not use `#[deprecated]`, `#[allow(dead_code)]`, or similar annotations to preserve unused code
- **Binary-only interface**: This project does not maintain API compatibility for external consumers - all code must serve the current CLI functionality
- **Dead code elimination**: Aggressively remove any unused functions, methods, structs, or modules during refactoring
- **Immediate cleanup**: When refactoring or implementing features, remove unused code paths immediately rather than marking them for future removal

### MANDATORY: Handling GitHub PR Review Comments

Follow this exact workflow whenever you receive a GitHub PR review comment link like:

```text
https://github.com/ophidiarium/cribo/pull/<PR_NUMBER>#discussion_r<COMMENT_ID>
```

1. Parse identifiers

- `<PR_NUMBER>`: digits after `/pull/`
- `<COMMENT_ID>`: digits after `discussion_r`

2. Fetch the comment body

```bash
gh api repos/ophidiarium/cribo/pulls/comments/<COMMENT_ID> \
  --jq '
"id:         \(.id)
pr_number:   \(.pull_request_url | split("/") | last)
author:      \(.user.login)
created_at:  \(.created_at)
file:        \(.path)
line:        \(.start_line)
--- BEGIN_BODY ---
\(.body)
--- END_BODY ---"'
```

3. Apply every suggestion immediately

- Treat even nitpicks as mandatory; do not defer
- Implement requested changes directly and completely

4. Commit and push

```bash
git add -A
git commit -m "chore: address PR review comment <COMMENT_ID>"
git push
```

5. Reply inline to the original comment

```bash
gh api repos/ophidiarium/cribo/pulls/<PR_NUMBER>/comments/<COMMENT_ID>/replies \
  -X POST -f body='✅ Addressed in <short-hash>. Thanks!'
```

Pre-checks and validations

- Verify GitHub CLI auth: `gh auth status`
- Ensure tests and lint are clean before replying:
  - `cargo nextest run --workspace`
  - `cargo clippy --workspace --all-targets`
