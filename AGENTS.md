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

The project is organized as a Rust workspace with the main crate in `crates/cribo`. The architecture follows a clear separation of concerns with dedicated modules for analysis, code generation, and AST traversal.

#### 🔍 Core Components & Navigation Guide

**THE REAL CRITICAL PATH: How Modules Get Bundled**

1. **CLI Entry Point** → `main.rs`
   - [`main()` in `main.rs`](crates/cribo/src/main.rs#L85) is the entry point.
   - It creates a [`BundleOrchestrator`](crates/cribo/src/orchestrator.rs#L176) and calls `bundle()` or `bundle_to_string()`.

2. **Orchestration Layer** → `orchestrator.rs`
   - [`bundle_to_string()` or `bundle()`](crates/cribo/src/orchestrator.rs#L636) → Entry points for bundling.
   - [`bundle_core()`](crates/cribo/src/orchestrator.rs#L356) → Module discovery and dependency graph building.
   - [`emit_static_bundle()`](crates/cribo/src/orchestrator.rs#L1850) → **Calls the REAL orchestrator**.

3. **🔥 THE ACTUAL BUNDLER** → `code_generator/bundler.rs`
   This is THE struct that orchestrates everything:
   - [`bundle_modules()` in `bundler.rs`](crates/cribo/src/code_generator/bundler.rs#L1263) is the main function that orchestrates the bundling of modules.

   ```rust
   pub fn bundle_modules(&mut self, params: &BundleParams<'a>) -> ModModule {
       // 1. Initialize bundler settings
       self.initialize_bundler(params);

       // 2. Prepare modules (trim imports, index ASTs)
       let modules = self.prepare_modules(params);

       // 3. Classify modules (THE critical decision)
       let classifier = ModuleClassifier::new(...);
       let classification = classifier.classify_modules(&modules);

       // 4. Process modules in dependency order
       // This is where inlining vs wrapping happens!
   }
   ```

4. **Module Classification** → `analyzers/module_classifier.rs`
   - [`classify_modules()` in `module_classifier.rs`](crates/cribo/src/analyzers/module_classifier.rs#L126) is where the decision to inline or wrap a module is made.

   ```rust
   // THE decision that determines bundle structure:
   if has_side_effects || has_invalid_identifier || needs_wrapping_for_circular:
       → wrapper_modules.push()  // Becomes init function with circular import guards
   else:
       → inlinable_modules.push() // Directly inserted into bundle
   ```

5. **Side Effect Detection** → `visitors/side_effect_detector.rs`
   - Key triggers that force wrapping are detected by visiting the AST. For example, a function call `Expr::Call(_)` is considered a side effect.

6. **Module Processing Loop** (inside `bundle_modules()`)
   - Processes modules in topological order from the dependency graph.
   - For circular dependencies: Two-phase emission (declarations then init).
   - For each module: Either inline content OR create a wrapper function.

#### 💀 The ACTUAL Code Generation Functions

**Inlining Path** (`bundler.rs` + `inliner.rs`)

- `process_inlinable_module()` → Handles inlined modules.
- `Inliner::inline_module()` → Actually inlines the module content.
- Transforms imports via `RecursiveImportTransformer`.
- Direct variable assignments, no function wrapper.

**Wrapper Path** (`bundler.rs`)

- `process_wrapper_module()` → Handles wrapper modules.
- `create_wrapper_module()` → Creates the init function.
- `module_wrapper::create_wrapper_module()` → Generates init function with `__initializing__` and `__initialized__` guards.
- Returns module namespace object (types.SimpleNamespace).

**Import Rewriting** (`bundler.rs`)

- `transform_bundled_import()` → Routes import to correct handler.
- `transform_wrapper_wildcard_import()` → Special case for `from wrapper import *`.
- `transform_wrapper_symbol_imports()` → Handles `from wrapper import symbol`.

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
