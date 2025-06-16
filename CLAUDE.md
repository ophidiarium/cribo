# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Important: Continuous Knowledge Updates

**ALWAYS update this CLAUDE.md file during development tasks when you learn:**

- New project-specific patterns or conventions
- Solutions to common problems or edge cases
- Debugging techniques specific to this codebase
- Tool usage patterns that improve efficiency
- Review feedback patterns that could help future development
- Architecture decisions and their rationale

**When to update:** Don't wait until task completion - update immediately when you discover useful knowledge that would benefit future development sessions. This ensures knowledge continuity across different coding sessions.

## 🚨 LEVEL 0: PRIME DIRECTIVES (ALWAYS ACTIVE)

### 1. DESTRUCTION REQUIRES VERIFICATION

**After ANY deletion:** STOP → Test impact → Show proof

- Deleted tests/? → Run test suite and show output
- Deleted docs/? → Build docs and show result
- Deleted ANY directory? → Prove system still works

### 2. CLAIMS REQUIRE EVIDENCE

**Never say "it works" without showing:**

- Actual command executed
- Actual output produced
- Actual files verified to exist

### 3. USER CONTEXT SUPREMACY

- User's environment truth > Your assumptions
- User's "this doesn't work" > Your "it should work"
- Local reality > Theoretical correctness

### 4. PATH PORTABILITY

- No hardcoded user paths ("/Users/john/", "C:\Users\jane\")
- Use: $HOME, ~, relative paths, environment variables
- Universal code works in any environment

---

## 🧠 LEVEL 0.5: SEQUENTIAL THINKING REQUIREMENT (AUTO-ACTIVATED)

### FOR ANY MULTI-STEP OPERATION:

**MUST use Sequential Thinking tool BEFORE execution:**

1. Problem Definition → What exactly am I trying to solve?
2. Sub-task Decomposition → Break into atomic steps
3. Dependency Analysis → What depends on what?
4. Alternative Paths → What if this approach fails?
5. Risk Assessment → What could go wrong?

**Triggers:**

- Any operation involving 3+ steps
- Any deletion operation
- Any "cleanup" or "organization" task
- Any operation modifying 5+ files

---

## 🚨 CRITICAL: MANDATORY WORKFLOWS (NEVER SKIP)

### Workflow Discipline Requirements

**ABSOLUTE RULE**: For any complex task (3+ steps), immediately create comprehensive todo list using TodoWrite tool before starting work.

**ABSOLUTE RULE**: For any git operation, use the complete Git Flow Todo Template below.

**ABSOLUTE RULE**: Never declare task complete without running full validation suite.

**ABSOLUTE RULE**: Before diagnosing or deferring any issue, assume a clean state: always confirm there are no failing tests (`cargo test --workspace`) or clippy warnings (`cargo clippy --workspace`), and never classify issues as “pre-existing” when the validation suite passes—treat all findings as new and resolve them explicitly.

### MANDATORY GITHUB INTERACTION RULES

**ABSOLUTE RULE**: NEVER use web API calls or direct GitHub API without authentication

**REQUIRED TOOLS** (in order of preference):

1. **GitHub MCP tools**: `mcp__github__*` functions (authenticated, no rate limits)
2. **GitHub CLI**: `gh` commands (authenticated via CLI)
3. **NEVER**: Direct API calls, web scraping, or unauthenticated requests

**EXAMPLES**:
✅ **Correct**: `mcp__github__get_pull_request` or `gh pr view`
❌ **Wrong**: Direct API calls to `api.github.com`

**PR Creation**: Always use `mcp__github__create_pull_request` or `gh pr create`
**PR Status**: Always use `mcp__github__get_pull_request` or `gh pr view`
**Comments**: Always use `mcp__github__add_issue_comment` or `gh pr comment`

### MANDATORY GIT FLOW TODO TEMPLATE

**CRITICAL**: Use this exact template for ANY git operation

#### Phase 0: Pre-Work Baseline (MANDATORY)

- [ ] **GitHub Tools Check**: Verify `gh` CLI authenticated and MCP tools available
- [ ] **git MCP**: set current working directory for git MCP
- [ ] **Coverage Baseline**: Run `cargo coverage-text` and record current numbers
- [ ] **Performance Baseline**: Run `cargo bench-save` to save performance baseline
- [ ] **Record baseline**: Overall %, affected files %, note 80% patch requirement
- [ ] **Current state**: `git status` and `git branch` - verify clean main
- [ ] **Dependencies**: Run `cargo test --workspace` for clean starting state

#### Phase 1: Feature Branch Creation & Implementation

- [ ] Create feature branch: `git checkout -b fix/descriptive-name`
- [ ] Implement changes (with coverage in mind)
- [ ] **Coverage check**: `cargo coverage-text` after major changes
- [ ] **Performance check**: `cargo bench-compare` after major changes
- [ ] **Test validation**: `cargo test --workspace` (must pass)
- [ ] **Clippy validation**: `cargo clippy --workspace --all-targets` (must be clean)
- [ ] **Coverage verification**: Ensure no >2% drops, patch >80%
- [ ] **Performance verification**: Ensure no >5% regressions without justification
- [ ] Commit with conventional message
- [ ] Push with upstream: `git push -u origin <branch-name>`

#### Phase 2: PR Creation

- [ ] **Use MCP/gh CLI**: `mcp__github__create_pull_request` or `gh pr create`
- [ ] Include comprehensive description (Summary, Changes, Test Results)
- [ ] Add coverage impact note if significant
- [ ] Add performance impact note if benchmarks show changes
- [ ] **IMMEDIATE status check**: `mcp__github__get_pull_request_status`
- [ ] **Verify ALL CI GREEN**: No failed GitHub Actions allowed

#### Phase 3: CI Monitoring (CRITICAL)

- [ ] **Monitor initial CI**: `mcp__github__get_pull_request_status` every few minutes
- [ ] **Verify specific checks**: Build ✅, Tests ✅, Coverage ✅, Clippy ✅
- [ ] **If ANY red check**: STOP, investigate, fix before proceeding
- [ ] **Coverage CI**: Must show patch coverage >80%
- [ ] **Wait for all GREEN**: Do not proceed until ALL checks pass

#### Phase 4: Code Review Response Cycle

- [ ] **Check for comments**: `mcp__github__get_pull_request` for review comments
- [ ] **For EACH comment**:
  - [ ] Read and understand fully
  - [ ] Implement requested change
  - [ ] Test the change locally
  - [ ] Commit with descriptive message
- [ ] **After fixes**: Push and verify CI still GREEN
- [ ] **Re-check status**: `mcp__github__get_pull_request_status`
- [ ] **Ensure coverage**: Still meets 80% patch requirement

#### Phase 5: Pre-Merge Verification (ENHANCED)

- [ ] **Final status check**: `mcp__github__get_pull_request_status`
- [ ] **Verify ALL criteria**:
  - [ ] `"mergeable": true`
  - [ ] `"statusCheckRollup": {"state": "SUCCESS"}`
  - [ ] `"reviewDecision": "APPROVED"`
  - [ ] Coverage CI showing GREEN
  - [ ] No pending or failed checks
- [ ] **NEVER merge with failed/pending checks**

#### Phase 6: Merge and Cleanup (CRITICAL FINAL STEPS)

- [ ] **Merge via MCP/gh**: `mcp__github__merge_pull_request` or `gh pr merge`
- [ ] **IMMEDIATELY switch**: `git checkout main`
- [ ] **Pull latest**: `git pull origin main`
- [ ] **Verify state**: `git status` shows "up to date with origin/main"
- [ ] **Delete branch**: `git branch -d <branch-name>`
- [ ] **Final verification**: `git status` shows clean working tree

#### Phase 7: Post-Merge Validation

- [ ] **Coverage check**: `cargo coverage-text` on main
- [ ] **Test validation**: `cargo test --workspace` on main
- [ ] **Clippy check**: `cargo clippy --workspace --all-targets` on main
- [ ] **Verify no regressions**: Compare with baseline measurements
- [ ] **Mark todos complete**: All git flow items ✅

**ABSOLUTE RULES**:

- NEVER use unauthenticated GitHub API calls
- NEVER merge with failed CI checks
- NEVER skip coverage verification
- NEVER declare success without full validation suite

### CODE COVERAGE & PERFORMANCE DISCIPLINE

#### Baseline Protocol (Coverage + Performance)

**MANDATORY FIRST STEP** for any code changes:

```bash
# 1. Get baseline coverage (BEFORE any changes)
cargo coverage-text

# 2. Record these numbers (example format):
# Baseline Coverage: 
# - Overall: 73.2%
# - orchestrator.rs: 89.4% 
# - code_generator.rs: 91.2%
# - unused_imports.rs: 76.8%

# 3. Get baseline performance (BEFORE any changes)
cargo bench-save     # Save current performance as baseline
# or
./scripts/bench.sh --save-baseline main
```

#### Coverage Targets and CI Requirements

**CI FAILURE TRIGGERS**:

- 🚨 **Patch coverage <80%**: CI will fail, PR cannot merge
- 🚨 **File coverage drops >2%**: Indicates insufficient testing
- 🚨 **Overall coverage drops >1%**: Major regression

**DEVELOPMENT RULES**:

- **New files**: Must achieve >90% line coverage
- **Modified files**: Coverage must not decrease
- **Critical paths**: Must have 100% coverage for error handling

#### Coverage Verification Commands

```bash
# During development (frequent checks)
cargo coverage-text

# Detailed coverage analysis
cargo coverage

# For CI-style validation
cargo coverage-lcov
```

#### Coverage Recovery Procedures

**If coverage drops**:

1. Identify uncovered lines: `cargo coverage`
2. Add targeted tests for missed paths
3. Focus on error conditions and edge cases
4. Re-run coverage until targets met
5. NEVER proceed with failing coverage

**If CI coverage check fails**:

1. Check CI logs for specific coverage failure
2. Run local coverage to reproduce
3. Add tests for uncovered code paths
4. Verify fix with `cargo coverage-text`
5. Push fix and re-check CI status

### PERFORMANCE REGRESSION TRACKING

#### Performance Baseline Management

**MANDATORY**: Track performance alongside code coverage for all significant changes.

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

#### Performance Targets

**ACCEPTABLE REGRESSIONS**:

- ≤3% for individual benchmarks (within noise margin)
- ≤1% for overall bundling performance
- Must be justified by significant feature additions

**UNACCEPTABLE REGRESSIONS**:

- 5% for any core operation without justification
- 10% for any benchmark (indicates algorithmic issue)
- Any regression in AST parsing (critical path)

#### Benchmark Categories

1. **Core Operations** (CRITICAL):
   - `bundle_simple_project`: End-to-end bundling
   - `parse_python_ast`: AST parsing performance
   - `resolve_module_path`: Module resolution speed

2. **Supporting Operations**:
   - `extract_imports`: Import extraction
   - `build_dependency_graph`: Graph construction

#### Performance Recovery Procedures

**If benchmarks show regression**:

1. **Identify**: Run `cargo bench-compare` to pinpoint specific regressions
2. **Profile**: Use `cargo flamegraph` or `perf` to find hotspots
3. **Optimize**: Focus on algorithmic improvements first
4. **Verify**: Re-run benchmarks to confirm improvement
5. **Document**: Note any trade-offs in commit message

**CI Performance Checks** (via Bencher.dev):

- Automated benchmark runs on PRs with statistical analysis
- Comprehensive PR comments with visual charts and regression alerts
- Historical performance tracking with trend analysis
- Block merge for statistically significant regressions

### PR STATUS MONITORING (CRITICAL FAILURE PREVENTION)

#### My Historical Failures to Avoid:

- ❌ Assuming PR is ready based on "mergeable" status alone
- ❌ Missing failed GitHub Actions in CI pipeline
- ❌ Not checking coverage CI specifically
- ❌ Merging with yellow/pending checks

#### MANDATORY PR Status Commands

```bash
# PRIMARY: Use MCP for comprehensive status
mcp__github__get_pull_request_status --owner=ophidiarium --repo=cribo --pullNumber=<NUM>

# SECONDARY: Use gh CLI for detailed breakdown
gh pr checks <PR-number>
gh pr view <PR-number> --json state,mergeable,statusCheckRollup,reviewDecision

# VERIFICATION: Get individual check details
gh run list --repo=ophidiarium/cribo --branch=<branch-name>
```

#### Status Interpretation Guide

**GREEN LIGHT** (safe to merge):

```json
{
    "mergeable": true,
    "statusCheckRollup": {
        "state": "SUCCESS" // ALL checks must be SUCCESS
    },
    "reviewDecision": "APPROVED"
}
```

**RED LIGHT** (DO NOT MERGE):

```json
{
    "statusCheckRollup": {
        "state": "FAILURE" // ANY failure means STOP
    }
}
```

**YELLOW LIGHT** (WAIT):

```json
{
    "statusCheckRollup": {
        "state": "PENDING" // Wait for completion
    }
}
```

#### Specific CI Checks to Monitor

**MUST BE GREEN**:

- ✅ **Build**: All platforms compile successfully
- ✅ **Test**: All test suites pass
- ✅ **Coverage**: Patch coverage >80%
- ✅ **Clippy**: No warnings or errors
- ✅ **Format**: Code formatting correct
- ✅ **Dependencies**: No security issues

#### CI Failure Response Protocol

**When ANY check fails**:

1. **STOP** - Do not proceed with merge
2. **Investigate**: Check CI logs for specific failure
3. **Fix**: Address the root cause locally
4. **Test**: Verify fix with local commands
5. **Push**: Commit fix and push to PR branch
6. **Monitor**: Wait for CI to re-run and verify GREEN
7. **Only then**: Proceed with merge consideration

#### Emergency CI Commands

```bash
# Check latest CI run status
gh run list --repo=ophidiarium/cribo --limit=5

# Get details of failed run
gh run view <run-id>

# Re-run failed checks (if appropriate)
gh run rerun <run-id>
```

### CHECKPOINT INSTRUCTIONS

#### Major Workflow Transitions

Before moving between phases, MUST verify:

**Implementation → Git Flow**:

- [ ] All tests passing: `cargo test --workspace` ✅
- [ ] All clippy issues resolved: `cargo clippy --workspace --all-targets` ✅
- [ ] Working directory clean: `git status` ✅

**Git Flow → Code Review**:

- [ ] PR created with comprehensive description ✅
- [ ] All files correctly included in PR ✅
- [ ] CI checks passing ✅

**Code Review → Merge**:

- [ ] ALL reviewer comments addressed ✅
- [ ] Final approval received ✅
- [ ] No outstanding review requests ✅

**Merge → Cleanup**:

- [ ] On main branch: `git branch` shows `* main` ✅
- [ ] Up to date: `git status` shows "up to date with origin/main" ✅
- [ ] Feature branch deleted ✅
- [ ] Working tree clean ✅

### Context Preservation Rules

**MANDATORY PRACTICES**:

- Always check `TodoRead` before starting new work
- Update todos immediately when scope changes
- When resuming work, first verify current state with `git status`
- Mark todos completed IMMEDIATELY when finished, not in batches

## 🛠️ PROJECT TECHNICAL DETAILS

### Project Overview

cribo is a Python source bundler written in Rust that produces a single .py file from a multi-module Python project by inlining first-party source files. It's available as both a CLI tool and a Python library via PyPI and npm.

Key features:

- Tree-shaking to include only needed modules
- Unused import detection and trimming
- Requirements.txt generation
- Configurable import classification
- PYTHONPATH and VIRTUAL_ENV support

### Build Commands

#### Rust Binary

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run the tool directly
cargo run -- --entry path/to/main.py --output bundle.py

# Run with verbose output for debugging
cargo run -- --entry path/to/main.py --output bundle.py -vv

# Run with trace-level output for detailed debugging
cargo run -- --entry path/to/main.py --output bundle.py -vvv
```

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
cargo test --workspace
```

#### Running Specific Bundling Fixtures with Insta Glob

The bundling snapshot tests use Insta's glob feature for automatic fixture discovery. You can run specific fixtures using glob filters:

```bash
# Run a specific fixture using environment variable
INSTA_GLOB_FILTER="**/stickytape_single_file/main.py" cargo test -p cribo --test test_bundling_snapshots test_bundling_fixtures

# Or using command line flag
cargo test test_bundling_fixtures -- --glob-filter "**/stickytape_single_file/main.py"

# Run multiple specific fixtures (use regex OR pattern)
INSTA_GLOB_FILTER="**/simple_math/main.py|**/future_imports_basic/main.py" cargo test test_bundling_fixtures

# Run all fixtures matching a pattern
INSTA_GLOB_FILTER="**/future_imports_*/main.py" cargo test test_bundling_fixtures

# Run fixture with debug output to see which fixture is running
INSTA_GLOB_FILTER="**/stickytape_single_file/main.py" cargo test test_bundling_fixtures -- --nocapture

# List available fixtures (useful for finding fixture names)
find crates/cribo/tests/fixtures -name "main.py" -type f | sed 's|.*/fixtures/||' | sed 's|/main.py||' | sort
```

**Common fixture patterns:**

- `stickytape_*` - Compatibility tests from stickytape project
- `future_imports_*` - Tests for **future** import handling
- `ast_rewriting_*` - Tests for AST transformation features
- `xfail_*` - Expected failure fixtures (prefix with xfail_)

**Tips:**

- The glob filter matches against the full path relative to the glob base directory
- Use `**` to match any number of directories
- The fixture name is the directory name containing `main.py`
- Fixtures are automatically discovered - just add a new directory with `main.py`

### Benchmarking Commands

```bash
# Run all benchmarks
cargo bench --bench bundling
# or
./scripts/bench.sh

# Save performance baseline
cargo bench-save
# or
./scripts/bench.sh --save-baseline main

# Compare against baseline
cargo bench-compare
# or
./scripts/bench.sh --baseline main

# Open HTML report
./scripts/bench.sh --open

# Run with Bencher.dev cloud tracking
./scripts/bench-bencher.sh
# Results viewable at: https://bencher.dev/console/projects/cribo/perf
```

### Coverage Commands

```bash
# Text coverage report
cargo coverage-text
# or
./scripts/coverage.sh coverage

# HTML coverage report (opens in browser)
cargo coverage
# or
./scripts/coverage.sh coverage-html

# LCOV format (for CI tools)
cargo coverage-lcov
# or
./scripts/coverage.sh coverage-lcov
```

### Architecture Overview

The project is organized as a Rust workspace with the main crate in `crates/cribo`.

#### Key Components

1. **Bundle Orchestration** (`orchestrator.rs`)
   - Coordinates the entire bundling workflow
   - Manages module discovery and dependency resolution
   - Handles circular dependency detection
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

#### Important Environment Variables

- `RUST_LOG` - Controls logging level (e.g., `RUST_LOG=debug`)
- `VIRTUAL_ENV` - Used for virtual environment support

### CLI Usage

```bash
cribo --entry src/main.py --output bundle.py [options]

# Output to stdout instead of file (useful for debugging)
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

### Development Guidelines

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

#### Deterministic Output Requirements (CRITICAL FOR DEPLOYMENT)

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

**Examples**:

```rust
// ❌ Non-deterministic (HashMap iteration order varies)
for import in imports.iter() { ... }

// ✅ Deterministic (sorted output)
let mut sorted_imports: Vec<_> = imports.iter().collect();
sorted_imports.sort();
for import in sorted_imports { ... }
```

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

**Usage Pattern**:

```bash
# 1. Create fixture directory
mkdir crates/cribo/tests/fixtures/my_new_feature

# 2. Add test files (main.py + any supporting modules)
echo "print('Hello Feature')" > crates/cribo/tests/fixtures/my_new_feature/main.py

# 3. Run tests - automatically discovered and tested
cargo test test_all_bundling_fixtures

# 4. Accept snapshots
cargo insta accept
```

**Generated Snapshots**:

- **`bundled_code@my_new_feature.snap`**: Clean Python code showing bundling structure
- **`execution_results@my_new_feature.snap`**: Structured execution results with status/output

**ALWAYS** prefer this framework when creating a new functionality or fixing a newly discovered regression.

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
- Ensure all functions are properly documented with Rust doc comments

- **Temporary Directory Usage**: When the agent requires a temporary directory for input or output files, it MUST use the `target/tmp` directory.
- **Stdout Output Support**: Tools support the `--stdout` argument and can output the bundle to stdout.

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

#### Git Operations

**MANDATORY**: Always use MCP Git tools instead of direct bash git commands for all git operations.

- **Use MCP Git tools**: Prefer `mcp__git__*` tools (e.g., `mcp__git__status`, `mcp__git__add`, `mcp__git__commit`) over bash `git` commands
- **Better integration**: MCP Git tools provide better integration with the development environment and error handling
- **Consistent workflow**: This ensures consistent git operations across all development workflows

#### Conventional Commits Requirements

**MANDATORY**: This repository uses automated release management with release-please. ALL commit messages MUST follow the Conventional Commits specification.

- **Format**: `<type>(<optional scope>): <description>`
- **Common types**: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `ci`
- **Breaking changes**: Use `!` after type (e.g., `feat!:`) or include `BREAKING CHANGE:` in footer
- **Version bumping**:
  - `fix:` → patch version (0.4.1 → 0.4.2)
  - `feat:` → minor version (0.4.1 → 0.5.0)
  - `feat!:` or `BREAKING CHANGE:` → major version (0.4.1 → 1.0.0)
- **Examples**:
  - `feat(parser): add support for new syntax`
  - `fix: handle null pointer exception in module resolver`
  - `chore: update dependencies`
  - `docs: improve CLI usage examples`
  - `feat(ai): enhance Claude Code integration`
  - `docs(ai): update CLAUDE.md configuration`

- **Available scopes**:
  - **Core components**: `parser`, `bundler`, `resolver`, `ast`, `emit`, `deps`, `config`, `cli`
  - **Testing & CI**: `test`, `ci`
  - **Documentation & AI**: `docs`, `ai`
  - **Build & packaging**: `build`, `npm`, `pypi`, `release`

**Enforcement**:

- Local validation via lefthook + commitlint prevents invalid commits
- CI checks all PR commits for compliance
- Release-please generates changelogs and releases automatically from commit history

**Never manually**:

- Edit `Cargo.toml` version numbers
- Edit `CHANGELOG.md`
- Create release tags
- The automated system handles all versioning and releases

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
   - ALWAYS prefer GitHub search tools (like `mcp__github__search_code`) over other methods when accessing GitHub repositories
   - When searching large repos, use specific path and filename filters to avoid token limit errors

#### Reference Patterns from Established Repositories

When implementing functionality, consult these high-quality repositories:

- **[astral-sh/ruff](https://github.com/astral-sh/ruff)** - For Python AST handling, rule implementation, configuration patterns
- **[astral-sh/uv](https://github.com/astral-sh/uv)** - For package resolution, dependency management, Python ecosystem integration
- **[web-infra-dev/rspack](https://github.com/web-infra-dev/rspack)** - For module graph construction, dependency resolution

#### Snapshot Testing with Insta

Accept new or updated snapshots using:

```bash
cargo insta accept
```

DO NOT use `cargo insta review` as that requires interactive input.

**Managing Unreferenced Snapshots:**

```bash
# List unreferenced snapshots without deleting them
cargo insta test --unreferenced=reject

# Auto-delete unreferenced snapshots
cargo insta test --unreferenced=auto

# Warn about unreferenced snapshots (default behavior)
cargo insta test --unreferenced=warn
```

**When to use:**

- After refactoring tests that change snapshot names
- After deleting tests that had associated snapshots
- When migrating snapshot locations (e.g., moving to test-specific directories)
- To clean up orphaned snapshots from renamed fixtures

#### Coverage Requirements

- Run baseline coverage check before implementing features:
  ```bash
  cargo coverage-text  # Get current coverage baseline
  ```
- Ensure coverage doesn't drop by more than 2% for any file or overall project
- New files should aim for >90% line coverage
- Critical paths should have 100% coverage for error handling and edge cases

#### Workflow Best Practices

- Always run tests and clippy after implementing a feature to make sure everything is working as expected
- **ALWAYS fix all clippy errors in the code you editing after finishing implementing a feature**

#### Docs-Manager MCP Tools (`mcp__docs-manager__*`)

**Fully Working Documentation Tools** (USE THESE):

- ✅ **`mcp__docs-manager__list_documents`** - List all markdown documents
  - Use for: Discovering available documentation files
  - Example: `mcp__docs-manager__list_documents` or `mcp__docs-manager__list_documents --path=docs`
  - Returns relative paths to all documents
- ✅ **`mcp__docs-manager__read_document`** - Read complete document content
  - Use for: Examining markdown files including frontmatter
  - Example: `mcp__docs-manager__read_document --path=README.md`
  - Returns full content with proper formatting
- ✅ **`mcp__docs-manager__search_documents`** - Search document content
  - Use for: Finding documents containing specific text or concepts
  - Example: `mcp__docs-manager__search_documents --query=bundling`
  - Searches both content and frontmatter across all documents
- ✅ **`mcp__docs-manager__write_document`** - Create new documents
  - Use for: Creating new markdown documentation files
  - Example: `mcp__docs-manager__write_document --path=new-doc.md --content="# Title\nContent"`
  - Can create parent directories automatically
- ✅ **`mcp__docs-manager__edit_document`** - Apply precise edits
  - Use for: Making targeted changes to existing documents
  - Example: `mcp__docs-manager__edit_document --path=doc.md --edits=[...]`
  - Provides git-style diff output showing changes

**Docs-Manager Recommendations**:

- **Use for core documentation workflows**: Reading, writing, editing, listing, and searching work excellently
- **Use filesystem MCP tools instead**: For folder creation, file moves, and renames until implemented
- **Consider path-specific queries**: For large repositories to avoid token limits
- **Excellent for content management**: High-quality diff output and comprehensive search capabilities

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
https://github.com/ophidiarium/cribo/pull/<PR#>#discussion_r<COMMENT_ID>
```

- `<PR#>` → digits between `/pull/` and `#`.
- `<COMMENT_ID>` → digits after `discussion_r`.

2. Fetch the comment body

```bash
GH_PAGER=cat gh api repos/ophidiarium/cribo/pulls/comments/<COMMENT_ID> \
  --template '
id:          {{ printf "%.0f" .id }}
author:      {{ .user.login }}
created_at:  {{ .created_at }}
--- BEGIN_BODY ---
{{ .body }}
--- END_BODY ---'
```

The text between BEGIN_BODY/END_BODY is what you must act on.

3. Apply every suggestion immediately

- Treat even “nitpick” remarks as mandatory.
- Do not leave TODOs, placeholders, or defer fixes.

4. Commit & push

Use `mcp__git__git_add`, `mcp__git__git_commit` and `mcp__git__git_push` MCP tools.

5.Reply directly to that comment

- DO NOT create a new review
- DO NOT add a top level issue comment
- REPLY DIRECTLY AND SPECIFICALLY to the original comment:

```bash
gh api repos/ophidiarium/cribo/pulls/<PR#>/comments/<COMMENT_ID>/replies \
  -X POST -f body="✅ Addressed in <hash>. Thanks!"
```

*Replace `<hash>` with the short commit SHA.*

**Follow these five steps exactly to process a GitHub review comment.**

NOTE: if asked to attend all comments use `mcp__github__get_pull_request_comments` to fetch comments, organized them, but then attend each one systematically following this workflow.

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
- lefhook (git hooks) config is at .lefthook.yaml
- use `bun` to manage Node.js dependencies and `bunx` to run npm packages
- use ast-grep if needed
- NEVER drop stashes!
- There are NEVER pre-existing test failures. Every feature development starts from the `main` branch, which is always in a clean state with all tests passing. If any test fails during or after a change, immediately investigate the root cause—do not assume the failure was present before your work. Never waste time considering the possibility of a pre-existing broken test.
