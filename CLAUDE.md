# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Serpen is a Python source bundler written in Rust that produces a single .py file from a multi-module Python project by inlining first-party source files. It's available as both a CLI tool and a Python library via PyPI and npm.

Key features:

- Tree-shaking to include only needed modules
- Unused import detection and trimming
- Requirements.txt generation
- Configurable import classification
- PYTHONPATH and VIRTUAL_ENV support

## Build Commands

### Rust Binary

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run the tool directly
cargo run -- --entry path/to/main.py --output bundle.py
```

### Python Package

```bash
# Build for development (creates a local installable package)
uvx maturin develop

# Build release package
uvx maturin build --release
```

### npm Package

```bash
# Generate npm packages
node scripts/generate-npm-packages.js

# Build npm binaries
./scripts/build-npm-binaries.sh
```

## Testing Commands

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

## Coverage Commands

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

## Architecture Overview

The project is organized as a Rust workspace with the main crate in `crates/serpen`.

### Key Components

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

### Important Environment Variables

- `RUST_LOG` - Controls logging level (e.g., `RUST_LOG=debug`)
- `VIRTUAL_ENV` - Used for virtual environment support

## Configuration

Serpen can be configured via a `serpen.toml` file:

```toml
# Example configuration
[bundler]
source_dirs = ["src"]
first_party_modules = ["my_module"]
preserve_comments = true
preserve_type_hints = true
```

## CLI Usage

```bash
serpen --entry src/main.py --output bundle.py [options]

# Common options
--emit-requirements    # Generate requirements.txt with third-party dependencies
--verbose              # Enable verbose logging
--config               # Specify custom config file path
```

## Development Guidelines

### Technical Decision-Making Requirements

**MANDATORY**: As a software-engineer AI agent, you must never factor in human limitations—such as limited time or bandwidth, business priorities, implementation complexity, research requirements, or any other "human" constraint—when making technical decisions or writing recommendations. Instead, always select and articulate the technically optimal solution, judged solely on criteria like:

- **Performance** (e.g., latency, throughput)
- **Physical resource constraints** (e.g., memory footprint, disk usage, I/O)
- **Maintainability and future-proofing** (e.g., scalability, extensibility, ease of refactoring)

Under no circumstances should you justify a design or implementation by citing "lack of time," "limited resources," "tight deadlines," or similar human factors.

### General Coding Standards

- Follow Rust idiomatic practices and use the Rust 2024 edition or later
- Use strong typing and leverage Rust's safety principles
- Write testable, extensible code; prefer pure functions where possible
- Ensure all functions are properly documented with Rust doc comments
- Take the opportunity to refactor code to improve readability and maintainability

### Git Operations

**MANDATORY**: Always use MCP Git tools instead of direct bash git commands for all git operations.

- **Use MCP Git tools**: Prefer `mcp__git__*` tools (e.g., `mcp__git__git_status`, `mcp__git__git_add`, `mcp__git__git_commit`) over bash `git` commands
- **Better integration**: MCP Git tools provide better integration with the development environment and error handling
- **Consistent workflow**: This ensures consistent git operations across all development workflows

### Conventional Commits Requirements

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

### Immediate Code Removal Over Deprecation

**MANDATORY**: Since Serpen only exposes a binary CLI interface (not a library API), unused methods and functions MUST be removed immediately rather than annotated with deprecation markers.

- **No deprecation annotations**: Do not use `#[deprecated]`, `#[allow(dead_code)]`, or similar annotations to preserve unused code
- **Binary-only interface**: This project does not maintain API compatibility for external consumers - all code must serve the current CLI functionality
- **Dead code elimination**: Aggressively remove any unused functions, methods, structs, or modules during refactoring
- **Immediate cleanup**: When refactoring or implementing features, remove unused code paths immediately rather than marking them for future removal

### Documentation Research Hierarchy

When implementing or researching functionality, follow this order:

1. **FIRST**: Generate and examine local documentation
   ```bash
   cargo doc --document-private-items
   ```

2. **SECOND**: Use Context7 for external libraries (only if local docs insufficient)

3. **FINAL**: Use GitHub MCP tools for implementation patterns (only when steps 1&2 insufficient)
   - ALWAYS prefer GitHub search tools (like `mcp__github__search_code`) over other methods when accessing GitHub repositories
   - When searching large repos, use specific path and filename filters to avoid token limit errors

### Reference Patterns from Established Repositories

When implementing functionality, consult these high-quality repositories:

- **[astral-sh/ruff](https://github.com/astral-sh/ruff)** - For Python AST handling, rule implementation, configuration patterns
- **[astral-sh/uv](https://github.com/astral-sh/uv)** - For package resolution, dependency management, Python ecosystem integration
- **[web-infra-dev/rspack](https://github.com/web-infra-dev/rspack)** - For module graph construction, dependency resolution

### Snapshot Testing with Insta

Accept new or updated snapshots using:

```bash
cargo insta accept
```

DO NOT use `cargo insta review` as that requires interactive input.

### Coverage Requirements

- Run baseline coverage check before implementing features:
  ```bash
  cargo coverage-text  # Get current coverage baseline
  ```
- Ensure coverage doesn't drop by more than 2% for any file or overall project
- New files should aim for >90% line coverage
- Critical paths should have 100% coverage for error handling and edge cases

### Workflow Best Practices

- Always run tests and clippy after implementing a feature to make sure everything is working as expected
- **ALWAYS fix all clippy errors in the code you editing after finishing implementing a feature**

### LSP Tool Usage

- **MANDATORY**: Always use LSP rename_symbol tool when renaming functions, structs, traits, or any other symbols in Rust code
- This ensures all references across the codebase are updated consistently
- For simple text edits that don't involve symbol renaming, continue using standard Edit/MultiEdit tools

### MANDATORY: Final Validation Before Claiming Success

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

### Git Workflow for Feature Development

**MANDATORY**: Follow this standardized git workflow when implementing new features:

#### 1. **Prepare and Create Feature Branch**

**CRITICAL**: Always ensure main branch is up-to-date before starting new work:

```bash
# 1. Switch to main branch
mcp__git__checkout --target "main"

# 2. Pull latest changes from origin/main
mcp__git__pull --branch "main"

# 3. Create and switch to new feature branch from updated main
mcp__git__branch_create --name "feat/<feature-name>"
# Or for other types: fix/<issue>, chore/<task>, docs/<topic>
```

This prevents merge conflicts by ensuring your feature branch starts from the latest main branch state.

#### 2. **Implement Feature**

- Write code following all guidelines above
- Add comprehensive tests for new functionality
- Update documentation as needed
- Run tests and clippy frequently during development

#### 3. **Commit Changes**

```bash
# Stage files using MCP Git tools
mcp__git__add --files ["path/to/file1", "path/to/file2"]

# Commit with conventional commit message
mcp__git__commit --message "feat(scope): add new feature

- Detailed description of what was added
- Why it was needed
- Any technical details

🤖 Generated with [Claude Code](https://claude.ai/code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

#### 4. **Push and Create PR**

```bash
# Push branch to remote
mcp__git__push --branch "feat/<feature-name>"

# Create pull request
mcp__github__create_pull_request \
  --title "feat(scope): add new feature" \
  --body "## Summary
- Brief description of changes

## Test plan
- [ ] All tests pass
- [ ] Clippy warnings resolved
- [ ] Documentation updated

🤖 Generated with [Claude Code](https://claude.ai/code)" \
  --head "feat/<feature-name>" \
  --base "main"
```

#### 5. **Wait for CI Checks**

- Monitor PR for all checks to pass (tests, clippy, commit validation)
- The repository has automated AI-powered review (CodeRabbit)
- Wait for all checks to complete before proceeding

#### 6. **Address Review Comments**

- Review all automated CodeRabbit comments
- Address each comment with code changes
- **IMPORTANT**: When resolving comments after making changes:
  - **DO NOT** create a new review with `mcp__github__create_pending_pull_request_review`
  - **DO** reply directly to the original comment thread
  - **DO** resolve/close the thread after responding

  **Correct approach for resolving attended comments:**
  ```bash
  # Reply to the specific comment thread (not yet available in MCP tools)
  # For now, use GitHub web interface to:
  # 1. Reply to each comment explaining what was done
  # 2. Click "Resolve conversation" on each addressed comment
  ```

  **Incorrect approach (creates new review comments):**
  ```bash
  # DON'T DO THIS when resolving existing comments:
  mcp__github__create_pending_pull_request_review
  mcp__github__add_pull_request_review_comment_to_pending_review
  mcp__github__submit_pending_pull_request_review --event "COMMENT"
  ```

- The pending review approach is only for adding NEW review comments, not for responding to existing ones
- Provide detailed explanations of what was changed and why when replying

#### 7. **Final Validation**

- Ensure all CI checks still pass after addressing comments
- Run final validation locally:
  ```bash
  cargo test --workspace
  cargo clippy --workspace --all-targets
  ```
- Push any additional fixes

#### 8. **Merge PR and Cleanup**

- Once all checks pass and comments are addressed
- The PR will be merged automatically or by maintainers

**MANDATORY Post-Merge Cleanup** (prevents future merge conflicts):

```bash
# 1. Switch back to main branch
mcp__git__checkout --target "main"

# 2. Pull latest changes including the merged PR
mcp__git__pull --branch "main"

# 3. Delete the local feature branch (no longer needed)
mcp__git__branch_delete --name "feat/<feature-name>"

# 4. Optional: Run garbage collection to clean up
git gc --aggressive --prune=now
```

This ensures your local main branch stays synchronized and prevents merge conflicts in future PRs.

#### Important Notes:

- **Never skip CI checks** - always wait for them to complete
- **Address ALL review comments** - including nitpicks and suggestions
- **Keep commits atomic** - each commit should represent a complete, working change
- **Update tests** - new features must include tests
- **Document changes** - update relevant documentation
- **Use conventional commits** - for automated versioning and changelog generation

### MANDATORY: PR Status and CI Checks Verification

**🚨 CRITICAL REQUIREMENT 🚨**: When checking PR status, you MUST verify both high-level status AND detailed CI check results. The high-level GitHub status can be misleading.

#### Complete PR Status Check Process

**ALWAYS follow this complete verification sequence:**

```bash
# 1. Get overall PR status (may not show all details)
mcp__github__get_pull_request_status --owner <owner> --repo <repo> --pullNumber <pr_number>

# 2. Check detailed workflow runs for the specific commit
gh run list --repo <owner>/<repo> --commit <commit_sha>

# 3. If any runs show 'failure', get detailed failure information
gh run view <failed_run_id> --repo <owner>/<repo>

# 4. For failed runs, get the actual failure logs
gh run view <failed_run_id> --log-failed --repo <owner>/<repo>
```

#### Critical Verification Points

1. **Check ALL Workflow Runs**: Don't rely solely on `mcp__github__get_pull_request_status` as it may only show limited status checks (like CodeRabbit reviews)

2. **Look for Platform-Specific Failures**:
   - Windows builds may fail due to line endings, path separators, or platform-specific behavior
   - macOS builds may have different behavior than Linux
   - Different Python versions (3.10, 3.11, 3.12) may exhibit different failures

3. **Common CI Failure Patterns**:
   - **Snapshot test failures**: Often due to line ending differences (CRLF vs LF)
   - **Clippy warnings/errors**: Must be fixed with actual code changes, not `#[allow]` annotations
   - **Test failures**: Check for platform-specific test issues
   - **Build failures**: Dependency conflicts, compilation errors, missing dependencies

#### Status Check Interpretation

**❌ These indicate failures requiring attention:**

- `status: "completed", conclusion: "failure"` - Actual test/build failure
- `status: "completed", conclusion: "action_required"` - Manual intervention needed
- Any workflow run showing `X` or `failure` status

**✅ These indicate successful runs:**

- `status: "completed", conclusion: "success"` - All checks passed
- `status: "completed", conclusion: "skipped"` - Intentionally skipped (often due to path filters)

#### Failure Response Protocol

When CI failures are detected:

1. **Identify Root Cause**: Use failure logs to understand the specific issue
2. **Fix Locally**: Implement the necessary fix in your local branch
3. **Test Thoroughly**: Ensure the fix works locally before pushing
4. **Push Fix**: Commit and push the fix to trigger new CI runs
5. **Verify Fix**: Wait for new CI runs and verify all checks pass

#### Example Failure Scenarios

**Snapshot Test Failure (Windows line endings):**

```bash
# Failure log will show something like:
# -expected content with \n
# +actual content with \r\n
```

**Fix**: Normalize line endings in output generation code

**Clippy Warnings:**

```bash
# Failure log shows clippy warnings/errors
```

**Fix**: Refactor code to address warnings, never use `#[allow]` annotations

**Platform-Specific Test Failure:**

```bash
# Tests pass on Linux/macOS but fail on Windows
```

**Fix**: Investigate platform-specific behavior and implement cross-platform solution

#### Never Make These Mistakes

- ❌ **Don't rely only on high-level PR status** - always check detailed workflow runs
- ❌ **Don't ignore "skipped" workflows** - verify they were skipped for valid reasons
- ❌ **Don't assume "action_required" means manual approval** - often indicates test failures
- ❌ **Don't merge with any failing checks** - all platforms must pass

This comprehensive approach ensures robust CI verification and prevents broken code from being merged.

### MANDATORY: Review Comment Response Protocol

**🚨 CRITICAL REQUIREMENT 🚨**: When addressing review comments on PRs, you MUST follow the individual comment reply protocol. Never post a single summary comment.

#### Required Comment Response Flow

**For EVERY individual review comment you address:**

1. **Reply to the specific comment thread** - Use GitHub's inline comment reply feature
2. **Document your fix clearly** - Explain exactly what you changed
3. **Reference specific commits/files** - Provide commit SHA and file paths when relevant
4. **Resolve the conversation** - Mark the conversation as resolved after providing your response

#### Correct Protocol Example

```
For each individual CodeRabbit/Copilot comment:

1. Reply directly to that comment with specific fix details
2. Example reply: "✅ Fixed: Added comprehensive documentation to CircularDependencyAnalysis struct in dependency_graph.rs:16-31. Commit: c666055"
3. Mark the conversation as resolved
```

#### NEVER Do This

❌ **Don't post a single large summary comment** - This doesn't follow GitHub's conversation resolution workflow

❌ **Don't group multiple fixes into one response** - Each comment thread needs individual attention

❌ **Don't leave conversations unresolved** - Every addressed comment must be marked as resolved

#### Proper Individual Response Template

For each review comment:

```markdown
✅ **Addressed in [commit-sha]**

[Specific description of the fix made]

Changes made in `path/to/file.rs:line-numbers`:

- [Detailed change 1]
- [Detailed change 2]

[Any additional context about the fix]
```

#### Comment Resolution Workflow

1. **Find the specific review comment** that needs addressing
2. **Make the code changes** to fix the issue
3. **Commit the changes** with a clear commit message
4. **Reply to the individual comment thread** with fix details
5. **Mark the conversation as resolved** using GitHub's resolve feature
6. **Repeat for each comment** until all are addressed

#### MANDATORY: Correct API Usage for Comment Replies

**🚨 CRITICAL REQUIREMENT 🚨**: Always use the correct GitHub API method for replying to comments.

**CORRECT API for Pull Request Review Comments:**

- Use `mcp__github__add_pull_request_review_comment_to_pending_review` for new review comments
- Use the correct GitHub CLI command for replies:

```bash
# For replying to existing review comments (line-level comments)
gh api repos/OWNER/REPO/pulls/PR_NUMBER/comments/COMMENT_ID/replies \
  --method POST \
  --field body="Your reply message here"

# For replying to general PR comments  
gh api repos/OWNER/REPO/issues/PR_NUMBER/comments \
  --method POST \
  --field body="Your reply message here"
```

**NEVER use these incorrect approaches:**
❌ **Don't edit existing comments** - This overwrites the original reviewer's comment
❌ **Don't use the wrong API endpoint** - Each comment type requires specific endpoints
❌ **Don't create new top-level comments** - These don't thread properly with the original comment

**Debugging Comment API Issues:**

1. **Identify comment type first**: Review comment (line-level) vs. general PR comment
2. **Get the correct comment ID**: Use `gh pr view PR_NUMBER --comments` to find comment IDs
3. **Use the appropriate API endpoint**: Review comments vs. issue comments have different APIs
4. **Test the reply**: Verify the reply appears threaded under the original comment

This ensures proper traceability and follows GitHub's intended review workflow for collaborative development.

## Memories

- Don't add timing complexity estimation to any documents - you don't know the team velocity
- When running on macOS, you should try `gsed` instead of `sed` for GNU sed compatibility on macOS
- MANDATORY: When addressing a clippy issue, never treat `#[allow]` annotations as a solution—perform actual refactoring to resolve the issue
- Remember you have full ruff repository cloned locally at references/type-strip/ruff so you may search in files easier
- lefhook config is at .lefthook.yaml
- use bun to manage Node.js dependencies
- CRITICAL: When asked to "resolve PR comments that you attended" - DO NOT create a new review. Instead, reply directly to the original comment threads and mark them as resolved. Creating a new review adds duplicate comments instead of resolving the existing ones.
