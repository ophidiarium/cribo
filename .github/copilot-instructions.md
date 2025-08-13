# Copilot Coding Assistant Instructions for Cribo

This document provides essential guidance for AI coding assistants working with the Cribo codebase.

## 🏗️ Project Overview

**Cribo** is a Rust-based Python source bundler that merges multi-module Python projects into a single `.py` file. It's distributed via both PyPI (`pip install cribo`) and npm (`npm install -g cribo`).

### Core Objectives

- Bundle Python projects while preserving functional equivalence
- Produce clean, readable output suitable for LLM ingestion
- Resolve at bundle-time rather than runtime for performance
- Handle circular dependencies via Tarjan's SCC and lazy imports

## 🎯 Architecture: The Big Picture

### Module Flow: Entry → Analysis → Transformation → Output

```
Entry Point → Orchestrator → Analyzers → Code Generator → Bundled Output
                    ↓             ↓             ↓
              ModuleRegistry  CriboGraph   Bundler/Transformers
```

### Key Components & Their Roles

1. **`orchestrator.rs`** (Control Center)
   - Coordinates entire bundling workflow
   - Manages module discovery and registry
   - Integrates all analysis phases
   - Handles circular dependency detection

2. **`analyzers/`** (Intelligence Layer)
   - `symbol_analyzer.rs`: Tracks symbol definitions and usage
   - `dependency_analyzer.rs`: Topological sorting, circular deps via Tarjan
   - `import_analyzer.rs`: Import relationship mapping
   - `namespace_analyzer.rs`: Detects namespace requirements

3. **`code_generator/`** (Transformation Engine)
   - `bundler.rs`: Main orchestration
   - `inliner.rs`: Module inlining logic for functions, classes, and assignments
   - `module_transformer.rs`: Module-level AST transformations
   - `expression_handlers.rs`: Creates/analyzes/transforms expressions
   - `namespace_manager.rs`: Manages namespace objects
   - `circular_deps.rs`: Handles circular dependency patterns

4. **`cribo_graph.rs`** (Dependency Tracking)
   - Pure graph structure for fine-grained dependencies
   - Item-level tracking (functions, classes, variables)
   - Cross-module reference analysis
   - Inspired by Turbopack's architecture

5. **`resolver.rs`** (Import Classification)
   - Classifies: stdlib vs first-party vs third-party
   - Resolves file paths for bundling
   - Handles PYTHONPATH and virtual environments

6. **`tree_shaking.rs`** (Dead Code Elimination)
   - Mark-and-sweep from entry points
   - Preserves directly imported module exports
   - Respects `__all__` declarations
   - Enabled by default (disable with `--no-tree-shake`)

## 🔧 Critical Developer Workflows

### Building & Running

```bash
# Development build
cargo build

# Run with entry point
cargo run -- --entry src/main.py --output bundle.py

# With tree-shaking disabled
cargo run -- --entry src/main.py --output bundle.py --no-tree-shake

# Output to stdout for debugging
cargo run -- --entry src/main.py --stdout

# Verbose logging (-v info, -vv debug, -vvv trace)
cargo run -- --entry src/main.py --output bundle.py -vv
```

### Testing Commands

```bash
# Run all tests (primary testing tool)
cargo nextest run --workspace

# Run and remove redundant insta snapshots
cargo insta test --all-features --unreferenced auto
```

### 📸 Snapshot Testing Framework

Cribo uses an automatic snapshot testing system that validates both bundled output AND execution results.

#### Test Organization

- Fixtures: `crates/cribo/tests/fixtures/<test_name>/main.py`
- Snapshots: `crates/cribo/tests/snapshots/`
  - `bundled_code@<test_name>.snap` - The bundled Python code
  - `execution_results@<test_name>.snap` - Runtime output/status
  - `ruff_lint_results@<test_name>.snap` - Linting validation
  - `requirements@<test_name>.snap` - Third-party dependencies

#### Running Specific Fixtures

```bash
# Run a single fixture
INSTA_GLOB_FILTER="**/simple_math/main.py" cargo nextest run --test test_bundling_snapshots --cargo-quiet --cargo-quiet

# Run fixtures matching pattern
INSTA_GLOB_FILTER="**/ast_rewriting_*/main.py" cargo nextest run --test test_bundling_snapshots

# With debug output
INSTA_GLOB_FILTER="**/all_variable_handling/main.py" cargo nextest run --no-capture --test test_bundling_snapshots

# List all available fixtures
find crates/cribo/tests/fixtures -name "main.py" -type f | sed 's|.*/fixtures/||' | sed 's|/main.py||' | sort
```

#### Fixture Prefixes

- `pyfail_`: MUST fail when run directly with Python
- `xfail_`: Succeeds in Python but MUST fail after bundling
- Normal fixtures: Must succeed both before and after bundling

#### Updating Snapshots

```bash
# Accept all snapshot changes
cargo insta accept

# Update specific fixture's snapshots
INSTA_UPDATE=always INSTA_GLOB_FILTER="**/simple_math/main.py" cargo nextest run --test test_bundling_snapshots
```

### 📊 Coverage & Performance

```bash
# Coverage with text report
cargo coverage-text

# LCOV for CI
cargo coverage-lcov

# Benchmarking
./scripts/bench.sh                    # Run benchmarks
./scripts/bench.sh --save-baseline main  # Save baseline
./scripts/bench.sh --baseline main       # Compare against baseline
./scripts/bench.sh --open               # Open HTML report
```

## ⚠️ Critical Rules & Patterns

### Conventional Commits

**ALL commits MUST follow Conventional Commits format:**

```
<type>(<optional scope>): <description>

[optional body]

[optional footer(s)]
```

**Allowed types** (enforced by commitlint):
- `feat`: New features
- `fix`: Bug fixes  
- `docs`: Documentation changes
- `style`: Code style changes (formatting, missing semicolons, etc.)
- `refactor`: Code changes that neither fix bugs nor add features
- `perf`: Performance improvements
- `test`: Adding missing tests or correcting existing tests
- `build`: Changes to build system or dependencies
- `ci`: Changes to CI configuration
- `chore`: Other changes that don't modify src or test files
- `revert`: Reverts a previous commit
- `ai`: AI-generated changes or AI-related updates

**Examples:**
```bash
feat: add support for circular dependency detection
fix(parser): handle malformed import statements correctly
docs: update installation instructions for npm package
style: format code with cargo fmt
refactor(analyzer): extract symbol tracking into separate module
```

**Breaking changes** use `!` or `BREAKING CHANGE:` footer:
```bash
feat!: remove deprecated bundling API
feat(api): add new parser interface

BREAKING CHANGE: The legacy parser interface has been removed.
```

### Code Formatting & Git Hooks

**ALWAYS run `cargo fmt` on changed Rust files:**

```bash
# Format specific files
cargo fmt -- path/to/file.rs

# Format all files in workspace
cargo fmt --all
```

**NEVER bypass git hooks** - they ensure code quality:
- Pre-commit hooks run `cargo fmt`, `cargo clippy`, and other checks
- Commit-msg hooks validate conventional commit format
- Let hooks fix formatting automatically when possible

**If hooks fail:**
1. Fix the underlying issue (don't bypass with `--no-verify`)
2. For formatting: run `cargo fmt` and re-commit
3. For linting: fix clippy warnings with `cargo clippy --fix`
4. For commit message: follow conventional commit format

### Deterministic Output

- Use `IndexSet`/`IndexMap` instead of `HashSet`/`HashMap`
- Sort all user-visible output (imports, collections)
- Check `.clippy.toml` for disallowed types/methods

### Code Quality Gates

```bash
# MANDATORY before claiming any implementation complete:
cargo fmt --all                           # Format code first
cargo nextest run --workspace             # Run all tests
cargo clippy --workspace --all-targets    # Check for linting issues
```

**Git hooks automatically enforce:**
- Code formatting with `cargo fmt`
- Linting with `cargo clippy --fix`
- Conventional commit message format
- Various file format checks (Markdown, TOML, etc.)

### Circular Dependencies

- Detected via Tarjan's SCC algorithm
- Resolved using init functions with `@functools.cache`
- Forward references handled via string annotations

### Tree-Shaking Behavior

- Enabled by default
- Analyzes from entry point transitively
- Preserves all exports from directly imported modules
- May have issues with complex circular dependencies

## 🎯 Quick Command Reference

```bash
# Most common development workflow
cargo build && cargo nextest run --workspace
cargo run -- --entry test.py --stdout -vv  # Debug bundling

# Code formatting and linting
cargo fmt --all                            # Format all Rust code
cargo clippy --workspace --all-targets     # Run linter
cargo clippy --workspace --fix             # Auto-fix linting issues

# Snapshot testing a specific fixture
INSTA_GLOB_FILTER="**/my_test/main.py" cargo nextest run --test test_bundling_snapshots

# Performance check
./scripts/bench.sh --baseline main

# Coverage check
cargo coverage-text

# Full validation (MANDATORY before completing any implementation)
cargo nextest run --workspace && cargo clippy --workspace --all-targets

# Git workflow with conventional commits
git add .
git commit -m "feat: add new bundling feature"    # Use conventional format
git push                                          # Hooks will validate automatically
```

## 📝 Additional Resources

- Main instructions: `/CLAUDE.md` (comprehensive guidelines)
- Architecture docs: `/docs/` directory
- Test fixtures: `/crates/cribo/tests/fixtures/`
- Benchmarks: `/crates/cribo/benches/`

---

Remember: All tests on main branch ALWAYS pass. If tests fail, investigate the root cause - there are no "pre-existing issues".
