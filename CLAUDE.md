# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 🚨 CRITICAL: MANDATORY WORKFLOWS (NEVER SKIP)

### Workflow Discipline Requirements

**ABSOLUTE RULE**: For any complex task (3+ steps), immediately create comprehensive todo list using TodoWrite tool before starting work.

**ABSOLUTE RULE**: For any git operation, use the complete Git Flow Todo Template below.

**ABSOLUTE RULE**: Never declare task complete without running full validation suite.

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
- [ ] **Coverage Baseline**: Run `cargo coverage-text` and record current numbers
- [ ] **Record baseline**: Overall %, affected files %, note 80% patch requirement
- [ ] **Current state**: `git status` and `git branch` - verify clean main
- [ ] **Dependencies**: Run `cargo test --workspace` for clean starting state

#### Phase 1: Feature Branch Creation & Implementation

- [ ] Create feature branch: `git checkout -b fix/descriptive-name`
- [ ] Implement changes (with coverage in mind)
- [ ] **Coverage check**: `cargo coverage-text` after major changes
- [ ] **Test validation**: `cargo test --workspace` (must pass)
- [ ] **Clippy validation**: `cargo clippy --workspace --all-targets` (must be clean)
- [ ] **Coverage verification**: Ensure no >2% drops, patch >80%
- [ ] Commit with conventional message
- [ ] Push with upstream: `git push -u origin <branch-name>`

#### Phase 2: PR Creation

- [ ] **Use MCP/gh CLI**: `mcp__github__create_pull_request` or `gh pr create`
- [ ] Include comprehensive description (Summary, Changes, Test Results)
- [ ] Add coverage impact note if significant
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

### CODE COVERAGE DISCIPLINE

#### Coverage Baseline Protocol

**MANDATORY FIRST STEP** for any code changes:

```bash
# 1. Get baseline coverage (BEFORE any changes)
cargo coverage-text

# 2. Record these numbers (example format):
# Baseline Coverage: 
# - Overall: 73.2%
# - bundler.rs: 89.4% 
# - ast_rewriter.rs: 91.2%
# - emit.rs: 76.8%
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

### PR STATUS MONITORING (CRITICAL FAILURE PREVENTION)

#### My Historical Failures to Avoid:

- ❌ Assuming PR is ready based on "mergeable" status alone
- ❌ Missing failed GitHub Actions in CI pipeline
- ❌ Not checking coverage CI specifically
- ❌ Merging with yellow/pending checks

#### MANDATORY PR Status Commands

```bash
# PRIMARY: Use MCP for comprehensive status
mcp__github__get_pull_request_status --owner=tinovyatkin --repo=serpen --pullNumber=<NUM>

# SECONDARY: Use gh CLI for detailed breakdown
gh pr checks <PR-number>
gh pr view <PR-number> --json state,mergeable,statusCheckRollup,reviewDecision

# VERIFICATION: Get individual check details
gh run list --repo=tinovyatkin/serpen --branch=<branch-name>
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
gh run list --repo=tinovyatkin/serpen --limit=5

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

Serpen is a Python source bundler written in Rust that produces a single .py file from a multi-module Python project by inlining first-party source files. It's available as both a CLI tool and a Python library via PyPI and npm.

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

# Run specific test file
cargo test --package serpen --test integration_tests

# Run tests matching a pattern
cargo test --package serpen unused_imports

# Run a specific named test
cargo test --package serpen test_simple_project_bundling
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

The project is organized as a Rust workspace with the main crate in `crates/serpen`.

#### Key Components

1. **Module Resolution & Import Classification** (`resolver.rs`)
   - Classifies imports as standard library, first-party, or third-party
   - Resolves actual file paths for bundling

2. **Dependency Graph** (`dependency_graph.rs`)
   - Builds a directed graph of module dependencies
   - Uses topological sorting to determine bundling order

3. **AST Parsing & Rewriting** (various files)
   - Uses Ruff's Python AST parser (`ruff_python_parser`) for AST parsing
   - Implements AST rewriting to handle import statements

4. **Unused Import Detection** (`unused_import_trimmer.rs`)
   - Detects and removes unused imports
   - Handles various import formats (simple, from, aliased)

5. **Code Generation** (`emit.rs`)
   - Generates the final bundled Python file
   - Maintains code structure with proper formatting

#### Important Environment Variables

- `RUST_LOG` - Controls logging level (e.g., `RUST_LOG=debug`)
- `VIRTUAL_ENV` - Used for virtual environment support

### Configuration

Serpen can be configured via a `serpen.toml` file:

```toml
# Example configuration
[bundler]
source_dirs = ["src"]
first_party_modules = ["my_module"]
preserve_comments = true
preserve_type_hints = true
```

### CLI Usage

```bash
serpen --entry src/main.py --output bundle.py [options]

# Common options
--emit-requirements    # Generate requirements.txt with third-party dependencies
--verbose              # Enable verbose logging
--config               # Specify custom config file path
```

### Development Guidelines

#### Technical Decision-Making Requirements

**MANDATORY**: As a software-engineer AI agent, you must never factor in human limitations—such as limited time or bandwidth, business priorities, implementation complexity, research requirements, or any other "human" constraint—when making technical decisions or writing recommendations. Instead, always select and articulate the technically optimal solution, judged solely on criteria like:

- **Performance** (e.g., latency, throughput)
- **Physical resource constraints** (e.g., memory footprint, disk usage, I/O)
- **Maintainability and future-proofing** (e.g., scalability, extensibility, ease of refactoring)

Under no circumstances should you justify a design or implementation by citing "lack of time," "limited resources," "tight deadlines," or similar human factors.

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

**MANDATORY**: Since Serpen only exposes a binary CLI interface (not a library API), unused methods and functions MUST be removed immediately rather than annotated with deprecation markers.

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

#### LSP Tool Usage

- **MANDATORY**: Always use LSP rename_symbol tool when renaming functions, structs, traits, or any other symbols in Rust code
- This ensures all references across the codebase are updated consistently
- For simple text edits that don't involve symbol renaming, continue using standard Edit/MultiEdit tools

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

## 🧠 WORKFLOW MEMORY AIDS

### Git Flow State Verification Commands

```bash
# Check current branch and status
git status
git branch

# Verify remote sync
git fetch
git status

# Check for uncommitted changes
git diff
git diff --staged
```

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
- lefhook config is at .lefthook.yaml
- use bun to manage Node.js dependencies
- CRITICAL: When asked to "resolve PR comments that you attended" - DO NOT create a new review. Instead, reply directly to the original comment threads and mark them as resolved. Creating a new review adds duplicate comments instead of resolving the existing ones.

# important-instruction-reminders

Do what has been asked; nothing more, nothing less.
NEVER create files unless they're absolutely necessary for achieving your goal.
ALWAYS prefer editing an existing file to creating a new one.
NEVER proactively create documentation files (*.md) or README files. Only create documentation files if explicitly requested by the User.
