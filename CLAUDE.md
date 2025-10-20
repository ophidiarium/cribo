# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 🛠️ PROJECT TECHNICAL DETAILS

### Project Overview

Cribo is a Rust-based source bundler for Python projects. It merges a multi-module Python codebase into a single `.py` file by inlining all first-party modules. Cribo is distributed as a command-line interface on both PyPI and npm.

#### Key features:

- Inline first-party modules while preserving original behavior
- Tree-shaking to include only necessary modules
- Detect and remove unused imports
- Generate a `requirements.txt` listing all third-party dependencies
- Customizable import classification
- Support for PYTHONPATH and virtual environments

#### ❤️ Project Requirements / Objectives

- The bundled output must be functionally equivalent to the original code.
- The resulting bundle should remain clear and easy to understand, particularly for LLM agents. Specifically:
  1. Preserve the original code structure as much as possible; avoid renaming, restructuring, or wrapping unless required to maintain functionality.
  2. Perform all resolvable computations and wiring at bundle time to minimize runtime evaluation.
- Runtime performance of the bundle should match or exceed the original code: avoid unnecessary wrappers and favor direct inlined references.

#### 👎 NOT an objective

- Maintaining full Python module semantics (e.g., `__name__`, `__all__`) is only necessary when it affects functionality; otherwise, static analysis and rewriting suffice.
- Guaranteeing theoretical compatibility with every potential side effect is not required; address clear side effects without introducing complexity for unlikely scenarios.

### Architecture Overview

The project is organized as a Rust workspace with the main crate in `crates/cribo`. The architecture follows a clear separation of concerns with dedicated modules for analysis, code generation, and AST traversal.

#### 🔍 Core Components & Navigation Guide

**THE REAL CRITICAL PATH: How Modules Get Bundled**

1. **Application Orchestration** → `crates/cribo/src/orchestrator.rs::BundleOrchestrator`
   - `bundle()` / `bundle_to_string()` → Entry points
   - `bundle_core()` → Module discovery, parsing, dependency resolution
   - `emit_static_bundle()` → Delegates to PhaseOrchestrator

2. **🔥 Phase-Based Bundling** → `crates/cribo/src/code_generator/phases/orchestrator.rs::PhaseOrchestrator`
   Coordinates 9 bundling phases:
   - `PhaseOrchestrator::bundle(&mut bundler, &params)` → Main entry
   - `InitializationPhase` → Setup, future imports
   - `PreparationPhase` → Trim imports, index ASTs
   - `ClassificationPhase` → Inlinable vs wrapper decision
   - `ProcessingPhase` → Module emission in dependency order
   - `EntryModulePhase` → Special entry module handling
   - `PostProcessingPhase` → Namespace attachments, proxies

3. **Module Classification** → `crates/cribo/src/analyzers/module_classifier.rs::classify_modules()`

   ```rust
   // THE decision that determines bundle structure:
   if has_side_effects || has_invalid_identifier || needs_wrapping_for_circular:
       → wrapper_modules.push()  // Becomes init function with circular import guards
   else:
       → inlinable_modules.push() // Directly inserted into bundle
   ```

4. **Side Effect Detection** → `crates/cribo/src/visitors/side_effect_detector.rs`
   - `visit_expr()` → `Expr::Call(_)`, `Expr::Lambda(_)` = SIDE EFFECT
   - `visit_stmt()` → `Stmt::ClassDef` with `metaclass`, `Stmt::Expr` (non-literals) = SIDE EFFECT

5. **Module Processing** → `crates/cribo/src/code_generator/phases/processing.rs::ProcessingPhase`
   - Processes modules in topological order
   - Two-phase emission for circular dependencies
   - Routes to inlinable or wrapper handlers

#### 💀 The ACTUAL Code Generation Functions

**Inlining Path** (`crates/cribo/src/code_generator/bundler.rs` + `crates/cribo/src/code_generator/inliner.rs`)

- `process_inlinable_module()` → Handles inlined modules
- `Inliner::inline_module()` → Actually inlines the module content
- Transforms imports via `RecursiveImportTransformer`
- Direct variable assignments, no function wrapper

**Wrapper Path** (`crates/cribo/src/code_generator/bundler.rs`)

- `process_wrapper_module()` → Handles wrapper modules
- `create_wrapper_module()` → Creates the init function
- `module_wrapper::create_wrapper_module()` → Generates init function with `__initializing__` and `__initialized__` guards
- Returns module namespace object (types.SimpleNamespace)

**Import Rewriting** (`crates/cribo/src/code_generator/bundler.rs`)

- `transform_bundled_import()` → Routes import to correct handler
- `transform_wrapper_wildcard_import()` → Special case for `from wrapper import *`
- `transform_wrapper_symbol_imports()` → Handles `from wrapper import symbol`

#### 🔥 Critical Decision Points

**1. The Wrapper vs Inline Decision** (`crates/cribo/src/analyzers/module_classifier.rs::classify_modules()`)

- THE most important decision - determines entire module structure
- Check `module_has_side_effects()` call
- Check `has_invalid_identifier` using `is_identifier()`
- Check `needs_wrapping_for_circular` from circular modules set

**2. Circular Dependency Classification** (`crates/cribo/src/analyzers/dependency_analyzer.rs::classify_cycle_type()`)

- `CircularDependencyType::FunctionLevel`: Can be resolved by moving imports
- `CircularDependencyType::ClassLevel`: Inheritance cycles, needs careful handling
- `CircularDependencyType::ModuleConstants`: UNRESOLVABLE - will fail
- `CircularDependencyType::ImportTime`: Needs wrapper functions
- Check `is_parent_child_package_cycle()` for package patterns

**3. Global Variable Lifting** (`crates/cribo/src/analyzers/global_analyzer.rs::analyze()`)

- Problem: `global x` in wrapper function needs real module-level `x`
- Solution: Lift to bundle top-level with unique names
- `collect_from_target()` collects module-level vars
- `visit_stmt()` for `Stmt::Global` tracks declarations
- Result in `ModuleGlobalInfo` struct

**4. Tree Shaking Mark & Sweep** (`crates/cribo/src/tree_shaking.rs::analyze()`)

- Entry: `TreeShaker::analyze()` starts from entry module
- `mark_used_symbols()` does recursive marking
- For `import module`: `mark_direct_import()` keeps ALL exports
- For `from module import x`: Only marks specific symbol
- `resolve_relative_with_context()` handles relative imports
- Gotcha: Circular deps with tree-shaking can break

#### 📊 Key Data Structures

**Module Registry** (`crates/cribo/src/orchestrator.rs` struct `ModuleRegistry`)

```rust
ModuleRegistry {
    modules: FxIndexMap<ModuleId, ModuleInfo>,
    name_to_id: FxIndexMap<String, ModuleId>,
    path_to_id: FxIndexMap<PathBuf, ModuleId>,
}
```

**Bundler State** (`crates/cribo/src/code_generator/bundler.rs` struct `Bundler`)

```rust
Bundler {
    module_synthetic_names: FxIndexMap<ModuleId, String>,
    module_init_functions: FxIndexMap<ModuleId, String>,
    wrapper_modules: FxIndexSet<ModuleId>,
    inlined_modules: FxIndexSet<ModuleId>,
}
```

**Dependency Graph** (`crates/cribo/src/cribo_graph.rs` struct `CriboGraph`)

```rust
CriboGraph {
    modules: FxIndexMap<ModuleId, ModuleDepGraph>,
    module_names: FxIndexMap<String, ModuleId>,
}
```

**Phase Modules** (`crates/cribo/src/code_generator/phases/`)

- `orchestrator.rs` → PhaseOrchestrator coordinates all phases
- `initialization.rs` → InitializationPhase: setup, future imports
- `classification.rs` → ClassificationPhase: inlinable vs wrapper
- `processing.rs` → ProcessingPhase: main module emission loop
- `entry_module.rs` → EntryModulePhase: special entry handling
- `post_processing.rs` → PostProcessingPhase: namespaces, proxies

#### 🐛 Common Issues & Where to Debug

**"Module unexpectedly wrapped"**
→ Set breakpoint in `crates/cribo/src/analyzers/module_classifier.rs::classify_modules()` at the if statement
→ Check `crates/cribo/src/visitors/side_effect_detector.rs::visit_expr()` for `Expr::Call` or `Expr::Lambda`
→ Look for "has side effects - using wrapper" in debug logs

**"Circular import error"**
→ `crates/cribo/src/code_generator/phases/processing.rs::process_circular_group()`
→ `crates/cribo/src/analyzers/dependency_analyzer.rs::analyze_circular_dependencies()`
→ Look for `CircularDependencyType::ModuleConstants` (unresolvable)

**"Import not transformed correctly"**
→ `crates/cribo/src/code_generator/bundler.rs::transform_bundled_import()` - THE router for all import transforms
→ Check `RecursiveImportTransformer::transform_import()`
→ Trace which handler was selected in `import_transformer/handlers/`

**"Symbol undefined after bundling"**
→ `crates/cribo/src/code_generator/phases/processing.rs::execute()` - check module processing
→ `crates/cribo/src/tree_shaking.rs::mark_used_symbols()` - tree-shaking analysis
→ Verify topological sort order

**"Global variable not working in wrapper"**
→ `crates/cribo/src/code_generator/bundler.rs::process_wrapper_module()`
→ `crates/cribo/src/analyzers/global_analyzer.rs::analyze()`
→ `crates/cribo/src/code_generator/module_transformer.rs::transform_module_with_globals()`

**"Module not bundled at all"**
→ `crates/cribo/src/orchestrator.rs::bundle_core()` - module discovery
→ `crates/cribo/src/code_generator/phases/orchestrator.rs::PhaseOrchestrator::bundle()` - orchestration
→ Check topological sort

#### 🔧 Debugging Commands

```bash
# See module classification decisions
RUST_LOG=debug cargo run -- --entry main.py --stdout 2>&1 | grep "side effects\|wrapper\|inline"

# Trace import transformation
RUST_LOG=trace cargo run -- --entry main.py --stdout 2>&1 | grep "transform.*import"

# Check circular dependency detection
RUST_LOG=debug cargo run -- --entry main.py --stdout 2>&1 | grep "circular\|cycle"

# Tree-shaking decisions
RUST_LOG=debug cargo run -- --entry main.py --stdout 2>&1 | grep "Tree-shaking\|Keeping\|Removing"
```

#### 🚀 Performance Hotspots

1. **Module Cache** (`crates/cribo/src/orchestrator.rs::process_module()`) - Caches parsed ASTs in `module_cache`
2. **Import Deduplication** (`crates/cribo/src/code_generator/import_deduplicator.rs::deduplicate_imports()`) - O(n²) comparisons
3. **Tree Shaking** (`crates/cribo/src/tree_shaking.rs::analyze()`) - Full graph traversal
4. **Side Effect Detection** (`crates/cribo/src/visitors/side_effect_detector.rs::module_has_side_effects()`) - Two-pass visitor

### CLI Usage

```bash
cribo --entry src/main.py --output bundle.py [options]

# Output to stdout instead of file (useful for debugging)
cribo --entry src/main.py --stdout [options]

# Common options
--emit-requirements    # Generate requirements.txt with third-party dependencies
--no-tree-shake        # Disable tree-shaking optimization (tree-shaking is enabled by default)
-v, --verbose...       # Increase verbosity (can be repeated: -v, -vv, -vvv)
                       # No flag: warnings/errors only
                       # -v: informational messages  
                       # -vv: debug messages
                       # -vvv: trace messages
--stdout               # Output bundled code to stdout instead of a file
```

#### Tree-Shaking (Enabled by Default)

Tree-shaking removes unused code from the bundle to reduce size:

- Analyzes which symbols are actually used starting from the entry point
- Preserves all symbols from directly imported modules (`import module` or `from pkg import module`)
- Respects `__all__` declarations and side effects
- Handles import aliases correctly
- Use `--no-tree-shake` to disable if you need to preserve all code

**Known Limitation**: Complex circular dependencies with generated init functions may cause issues with tree-shaking. Use `--no-tree-shake` if you encounter undefined symbol errors.

#### Stdout Mode for Debugging

The `--stdout` flag is particularly useful for debugging and development workflows:

```bash
# Quick inspection of bundled output without creating files
cribo --entry main.py --stdout

# Pipe to tools for analysis
cribo --entry main.py --stdout | python -m py_compile -

# Combine with verbose logging (logs go to stderr, code to stdout)
cribo --entry main.py --stdout -vv
```

**Key Benefits:**

- No temporary files created
- All log output properly separated to stderr
- Perfect for piping to other tools
- Ideal for containerized environments
- Excellent for quick debugging workflows

## 🚨 CRITICAL: ALL TESTS ON MAIN BRANCH ARE ALWAYS WORKING! 🚨

**ALL TESTS THAT EXIST ON MAIN BRANCH ARE ALWAYS WORKING! YOU MUST NEVER DOUBLE CHECK THAT! THERE ARE NO "PRE-EXISTING ISSUES"! DON'T SPEND TIME ON SEARCHING FOR AN ESCAPE HATCH - DO INVESTIGATE TO FIND THE ROOT CAUSE OF A PROBLEM**

### MANDATORY GIT FLOW TODO TEMPLATE

**CRITICAL**: Use this exact template for ANY git operation

#### Phase 0: Pre-Work Baseline (MANDATORY)

- [ ] **GitHub Tools Check**: Verify `gh` CLI authenticated
- [ ] **Dependencies**: Run `cargo nextest run --workspace` for clean starting state

#### Phase 1: Feature Branch Creation & Implementation

- [ ] Create feature branch: `git checkout -b fix/descriptive-name origin/main`
- [ ] Implement changes
- [ ] **Test validation**: `cargo test --workspace` (must pass)
- [ ] **Clippy validation**: `cargo clippy --workspace --all-targets` (must be clean)
- [ ] Commit with conventional message
- [ ] Push with upstream: `git push -u origin <branch-name>`

#### Phase 2: PR Creation

- [ ] **Use gh CLI**: `gh pr create`
- [ ] Include comprehensive description (Summary, Changes, Test Results)
- [ ] Add coverage impact note if significant
- [ ] Add performance impact note if benchmarks show changes

## Rule: Use Worktree, Not Checkout

When you need to check how something works on `main` branch while working on a feature branch, **use `git worktree` instead of `git checkout`**.

### Why

- Preserves your current uncommitted changes
- Keeps development context intact
- Enables side-by-side comparison

### Standard Process

```bash
# Create worktree for main
git worktree add ../main-ref main

# Check the code
cd ../main-ref
# ... examine files ...
cd -

# Clean up when done
git worktree remove ../main-ref
```

### Quick Commands

```bash
git worktree add <path> <branch>    # Create
git worktree list                   # List all
git worktree remove <path>          # Remove
```

### Exception

Only use `git checkout` when you have no uncommitted changes and are permanently switching branches.

### CODE COVERAGE & PERFORMANCE DISCIPLINE

#### Coverage Verification Commands

```bash
# During development (frequent checks)
cargo coverage-text

# Detailed coverage analysis
cargo coverage

# For CI-style validation
cargo coverage-lcov
```

#### Performance Baseline Management

```bash
# Before starting work - save baseline
cargo bench-save
# or
./scripts/bench.sh --save-baseline main

# After implementing changes - compare
cargo bench-compare
# or  
./scripts/bench.sh --baseline main

# View detailed HTML report
./scripts/bench.sh --open
```

### Context Preservation Rules

**MANDATORY PRACTICES**:

- Always check `TodoRead` before starting new work
- Update todos immediately when scope changes
- When resuming work, first verify current state with `git status`
- Mark todos completed IMMEDIATELY when finished, not in batches

### Build Commands

#### Python Package

```bash
# Build for development (creates a local installable package)
uvx maturin develop

# Build release package
uvx maturin build --release
```

#### npm Package

```bash
# Generate npm packages
node scripts/generate-npm-packages.js

# Build npm binaries
./scripts/build-npm-binaries.sh
```

### Testing Commands

```bash
# Run all tests
cargo nextest run --workspace
```

#### Running Specific Bundling Fixtures with Insta Glob

The bundling snapshot tests use Insta's glob feature for automatic fixture discovery. You can run specific fixtures using glob filters:

```bash
# Run a specific fixture using environment variable
INSTA_GLOB_FILTER="**/stickytape_single_file/main.py" cargo nextest run --test test_bundling_snapshots --cargo-quiet --cargo-quiet

# Run all fixtures matching a pattern
INSTA_GLOB_FILTER="**/future_imports_*/main.py" cargo nextest run --test test_bundling_snapshots --cargo-quiet --cargo-quiet

# Run fixture with debug output to see which fixture is running
INSTA_GLOB_FILTER="**/stickytape_single_file/main.py" cargo nextest run --no-capture --test test_bundling_snapshots --cargo-quiet --cargo-quiet

# List available fixtures (useful for finding fixture names)
find crates/cribo/tests/fixtures -name "main.py" -type f | sed 's|.*/fixtures/||' | sed 's|/main.py||' | sort
```

**Common fixture patterns:**

- `stickytape_*` - Compatibility tests from stickytape project
- `future_imports_*` - Tests for **future** import handling
- `ast_rewriting_*` - Tests for AST transformation features
- `pyail_*` - Expected failure fixtures (must fail by direct python execution)
- `xfail_*` - Expected failure fixtures (prefix with xfail_ - bundled code MUST fail)

**Tips:**

- The glob filter matches against the full path relative to the glob base directory
- Use `**` to match any number of directories
- The fixture name is the directory name containing `main.py`
- Fixtures are automatically discovered - just add a new directory with `main.py`

### Development Guidelines

- **Temporary Directory Usage**: When the agent requires a temporary directory for input or output files, it MUST use the `target/tmp` directory.

#### Technical Decision-Making Requirements

**MANDATORY**: As a software-engineer AI agent, you must never factor in human limitations—such as limited time or bandwidth, business priorities, implementation complexity, research requirements, or any other "human" constraint—when making technical decisions or writing recommendations. Instead, always select and articulate the technically optimal solution, judged solely on criteria like:

- **Performance** (e.g., latency, throughput)
- **Physical resource constraints** (e.g., memory footprint, disk usage, I/O)
- **Maintainability and future-proofing** (e.g., scalability, extensibility, ease of refactoring)

Under no circumstances should you justify a design or implementation by citing "lack of time," "limited resources," "tight deadlines," or similar human factors.

## Logging Guidelines (IMPORTANT to not break --stdout switch)

- Always use structured logging instead of `println!` for debug output: `use log::{debug, info, warn, error};`
- Use appropriate log levels:
  - `debug!()` for detailed diagnostic information useful during development
  - `info!()` for general information about program execution
  - `warn!()` for potentially problematic situations
  - `error!()` for error conditions that should be addressed
- If debug logging was essential to find a bug in the codebase, that logging should be kept in the codebase at the appropriate log level to aid future debugging
- Avoid temporary `println!` statements - replace them with proper logging before committing code
- Use structured logging with context where helpful: `debug!("Processing file: {}", file_path)`

#### Deterministic Output Requirements (CRITICAL)

**MANDATORY**: Considering the potential use of this tool in deployment scenarios, it is **essential** to aim for deterministic, reproducible bundle output. This enables users to:

- Avoid unnecessary redeployments when source code hasn't meaningfully changed
- Simplify change diff inspection and validation
- Maintain predictable deployment pipelines
- Enable reliable content-based caching and optimization

**This principle explains**:

- Why we have disallowed types and methods in `.clippy.toml` (e.g., `HashSet` → `IndexSet` for deterministic iteration order)
- Why we must apply sorting or deterministic rules for any output where order doesn't matter

**Implementation Rules**:

- **Sort imports**: `from foo import d, a, b` → `from foo import a, b, d`
- **Sort collections**: When outputting multiple items, always apply consistent ordering
- **Stable iteration**: Use `IndexMap`/`IndexSet` instead of `HashMap`/`HashSet` for deterministic order
- **Consistent formatting**: Apply the same formatting rules regardless of input order
- **Reproducible timestamps**: Avoid embedding timestamps or random values in output

**Testing Determinism**:

- Run bundler multiple times on same input - output must be identical
- Test with different module discovery orders - final bundle must be same
- Verify sorting applies to all user-visible output elements

#### Generic Snapshot Testing Framework (REUSE FOR NEW FEATURES)

**MANDATORY**: Before implementing custom test logic for bundling features, **ALWAYS** evaluate if the existing generic snapshot testing framework can be used or extended. This framework provides comprehensive testing with minimal implementation effort.

**Framework Location**: `crates/cribo/tests/test_bundling_snapshots.rs`

**How It Works**:

- **Automatic Discovery**: Scans `crates/cribo/tests/fixtures/` for test directories
- **Convention-Based**: Each directory with `main.py` becomes a test case automatically
- **Dual Snapshots**: Generates both bundled code and execution result snapshots
- **Deterministic**: All output is sorted and reproducible across runs

**Generated Snapshots**:

- **`bundled_code@my_new_feature.snap`**: Clean Python code showing bundling structure
- **`execution_results@my_new_feature.snap`**: Structured execution results with status/output

**ALWAYS** prefer this framework when creating a new functionality or fixing a newly discovered regression.

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

#### Documentation Research Hierarchy

When implementing or researching functionality, follow this order:

1. **FIRST**: Generate and examine local documentation

   ```bash
   cargo doc --document-private-items
   ```

2. **SECOND**: Use Context7 for external libraries (only if local docs insufficient)

3. **FINAL**: Use GitHub MCP tools for implementation patterns (only when steps 1&2 insufficient)

#### Reference Patterns from Established Repositories

When implementing functionality, consult these high-quality repositories:

- **[astral-sh/ruff](https://github.com/astral-sh/ruff)** - For Python AST handling, rule implementation, configuration patterns
- **[astral-sh/uv](https://github.com/astral-sh/uv)** - For package resolution, dependency management, Python ecosystem integration
- **[web-infra-dev/rspack](https://github.com/web-infra-dev/rspack)** - For module graph construction, dependency resolution

Check `references/` directory for local clones of above and some other examples

#### MANDATORY: Final Validation Before Claiming Success

**🚨 CRITICAL REQUIREMENT 🚨**: Before claiming that any implementation is complete or successful, you MUST run the complete validation suite:

```bash
# 1. Run all tests in the workspace
cargo test --workspace

# 2. Run clippy on all targets
cargo clippy --workspace --all-targets

# 3. Fix any clippy errors or warnings
# NEVER use #[allow] annotations as a "fix" - do actual refactoring
```

**NO EXCEPTIONS**: Do not declare success, claim completion, or say "implementation is working" without running both commands above and ensuring they pass without errors. This applies to:

- Feature implementations
- Bug fixes
- Refactoring
- Any code changes

If tests fail or clippy reports issues, the implementation is NOT complete until these are resolved.

## 📑 MANDATORY: Handling GitHub PR Review Comment(s)

1. When given an URL - parse the incoming URL

```text
https://github.com/ophidiarium/cribo/pull/<PR_NUMBER>#discussion_r<COMMENT_ID>
```

- `<PR_NUMBER>` → digits between `/pull/` and `#`.
- `<COMMENT_ID>` → digits after `discussion_r`.

2. Fetch the comment body

```bash
gh api repos/ophidiarium/cribo/pulls/comments/<COMMENT_ID> \
  --jq '
"id:         \(.id)
pr_number:   \(.pull_request_url | split("/") | last)
author:      \(.user.login)
created_at:  \(.created_at)
file:        \( .path )
line:        \( .start_line )
--- BEGIN_BODY ---
\(.body)
--- END_BODY ---"'
```

The text between BEGIN_BODY/END_BODY is what you must act on.

3. Apply every suggestion immediately

- Treat even “nitpick” remarks as mandatory.
- Do not leave TODOs, placeholders, or defer fixes.

4. Commit & push

5.Reply directly to that comment

- DO NOT create a new review
- DO NOT add a top level issue comment
- REPLY DIRECTLY AND SPECIFICALLY to the original comment:

```bash
gh api repos/ophidiarium/cribo/pulls/<PR_NUMBER>/comments/<COMMENT_ID>/replies \
  -X POST -f body='✅ Addressed in <hash>. Thanks!'
```

*Replace `<hash>` with the short commit SHA.*

**Follow these five steps exactly to process a GitHub review comment.**

## 🧠 WORKFLOW MEMORY AIDS

### Recovery Procedures

**If lost in git flow**:

1. Run `git status` to understand current state
2. Check `TodoRead` to see where you left off
3. Verify which phase you're in based on branch and remote state
4. Continue from appropriate checklist item

**If review comments missed**:

1. Check PR comments immediately
2. Create todo item for each comment
3. Address systematically before any other work

## Memories

- Don't add timing complexity estimation to any documents - you don't know the team velocity
- When running on macOS, you should try `gsed` instead of `sed` for GNU sed compatibility on macOS
- MANDATORY: When addressing a clippy issue, never treat `#[allow]` annotations as a solution—perform actual refactoring to resolve the issue
- Remember you have full ruff repository cloned locally at references/type-strip/ruff so you may search in files easier
- lefhook (git hooks) config is at lefthook.yml
- use `bun` to manage Node.js dependencies and `bunx` to run npm packages
- use ast-grep if needed
- NEVER drop stashes!
- There are NEVER pre-existing test failures. Every feature development starts from the `main` branch, which is always in a clean state with all tests passing. If any test fails during or after a change, immediately investigate the root cause—do not assume the failure was present before your work. Never waste time considering the possibility of a pre-existing broken test.

## 🚨 REMINDER: ALL TESTS ON MAIN BRANCH ARE ALWAYS WORKING! 🚨

**ALL TESTS THAT EXIST ON MAIN BRANCH ARE ALWAYS WORKING! YOU MUST NEVER DOUBLE CHECK THAT! THERE ARE NO "PRE-EXISTING ISSUES"! DON'T SPEND TIME ON SEARCHING FOR AN ESCAPE HATCH - DO INVESTIGATE TO FIND THE ROOT CAUSE OF A PROBLEM**
