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

### GitHub CLI for PR Review Management

#### Replying to PR Review Comments

To reply to an existing PR review comment:

```bash
gh api repos/{owner}/{repo}/pulls/{pull_number}/comments \
  -X POST \
  -f body="Your reply text" \
  -F in_reply_to={comment_id}
```

#### Resolving PR Review Comments

To mark a PR review thread as resolved using GraphQL:

```bash
gh api graphql -f query='
  mutation {
    resolveReviewThread(input: {
      threadId: "THREAD_ID_HERE"
    }) {
      thread {
        id
        isResolved
      }
    }
  }'
```

To mark a PR review thread as unresolved:

```bash
gh api graphql -f query='
  mutation {
    unresolveReviewThread(input: {
      threadId: "THREAD_ID_HERE"
    }) {
      thread {
        id
        isResolved
      }
    }
  }'
```

**Note**: You'll need to obtain the thread ID from the review thread. Each review comment belongs to a thread, and the thread ID can be retrieved through GitHub's API or GraphQL queries.

#### Re-requesting Reviews (Clear "Blocked" Status)

When CodeRabbit or other reviewers request changes, the PR becomes "blocked" even after addressing all comments. To clear this status, re-request review from the same reviewers:

```bash
# Get the pull request Node ID and reviewer user IDs first
gh api graphql -f query='
  query {
    repository(owner: "OWNER", name: "REPO") {
      pullRequest(number: PULL_NUMBER) {
        id
        reviews(last: 10) {
          nodes {
            author {
              login
              ... on User {
                id
              }
            }
            state
          }
        }
      }
    }
  }'

# Then re-request reviews using the PR ID and user IDs
gh api graphql -f query='
  mutation {
    requestReviews(input: {
      pullRequestId: "PULL_REQUEST_NODE_ID"
      userIds: ["USER_ID_1", "USER_ID_2"]
    }) {
      pullRequest {
        id
        reviewRequests(first: 10) {
          nodes {
            requestedReviewer {
              ... on User {
                login
              }
            }
          }
        }
      }
    }
  }'
```

This clears the "blocked" status by converting the "changes requested" state back to a "review requested" state.

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

#### 1. **Create Feature Branch**

```bash
# Use MCP Git tools to create and switch to feature branch
mcp__git__branch_create --name "feat/<feature-name>"
# Or for other types: fix/<issue>, chore/<task>, docs/<topic>
```

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
  # Reply to a specific review comment thread using gh CLI:
  gh api repos/{owner}/{repo}/pulls/{pull_number}/comments \
    -X POST \
    -f body="Your reply explaining what was done" \
    -F in_reply_to={comment_id}

  # Note: This creates a reply in the thread, but doesn't mark it as resolved
  # Marking as resolved still requires the web interface
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

#### 8. **Merge PR**

- Once all checks pass and comments are addressed
- The PR will be merged automatically or by maintainers
- Delete the feature branch after merge

#### Important Notes:

- **Never skip CI checks** - always wait for them to complete
- **Address ALL review comments** - including nitpicks and suggestions
- **Keep commits atomic** - each commit should represent a complete, working change
- **Update tests** - new features must include tests
- **Document changes** - update relevant documentation
- **Use conventional commits** - for automated versioning and changelog generation

## Memories

- Don't add timing complexity estimation to any documents - you don't know the team velocity
- When running on macOS, you should try `gsed` instead of `sed` for GNU sed compatibility on macOS
- MANDATORY: When addressing a clippy issue, never treat `#[allow]` annotations as a solution—perform actual refactoring to resolve the issue
- Remember you have full ruff repository cloned locally at references/type-strip/ruff so you may search in files easier
- lefhook config is at .lefthook.yaml
- use bun to manage Node.js dependencies
- CRITICAL: When asked to "resolve PR comments that you attended" - DO NOT create a new review. Instead, reply directly to the original comment threads and mark them as resolved. Creating a new review adds duplicate comments instead of resolving the existing ones.
