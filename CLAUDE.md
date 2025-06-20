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
- [ ] **Dependencies**: Run `cargo nextest run --workspace` for clean starting state

#### Phase 1: Feature Branch Creation & Implementation

- [ ] Create feature branch: `git checkout -b fix/descriptive-name origin/main`
- [ ] Implement changes
- [ ] **Test validation**: `cargo test --workspace` (must pass)
- [ ] **Clippy validation**: `cargo clippy --workspace --all-targets` (must be clean)
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
INSTA_GLOB_FILTER="**/stickytape_single_file/main.py" cargo nextest run --test test_bundling_snapshots --cargo-quiet --cargo-quiet --nocapture

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

#### Git Operations

**MANDATORY**: Always use MCP Git tools instead of direct bash git commands for all git operations.

- **Use MCP Git tools**: Prefer `mcp__git__*` tools (e.g., `mcp__git__status`, `mcp__git__add`, `mcp__git__commit`) over bash `git` commands
- **Better integration**: MCP Git tools provide better integration with the development environment and error handling
- **Consistent workflow**: This ensures consistent git operations across all development workflows

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

Check `references/` directory for local clones of above and some other examples

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
