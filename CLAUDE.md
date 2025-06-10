# CLAUDE.md

Guidance for Claude Code (claude.ai/code) when working with cribo repository.

## 🚨 CRITICAL WORKFLOWS (NEVER SKIP)

### Mandatory Rules

- **Complex tasks (3+ steps)**: Create comprehensive TodoWrite before starting
- **Git operations**: Use complete Git Flow Todo Template below
- **Validation**: Never claim completion without full validation suite
- **Clean state**: Always verify no failing tests/clippy warnings before diagnosing issues

### GitHub Interaction Rules

**NEVER**: Use web API calls or direct GitHub API without authentication\
**ALWAYS USE** (in order):

1. **MCP tools**: `mcp__github__*` functions (authenticated)
2. **GitHub CLI**: `gh` commands (authenticated)

### Git Flow Todo Template

#### Phase 0: Pre-Work Baseline

- [ ] GitHub tools check: Verify `gh` CLI and MCP available
- [ ] git MCP: set working directory
- [ ] Coverage baseline: `cargo coverage-text` - record numbers
- [ ] Performance baseline: `cargo bench-save`
- [ ] Current state: `git status` and `git branch`
- [ ] Dependencies: `cargo test --workspace` (must pass)

#### Phase 1: Implementation

- [ ] Create branch: `git checkout -b fix/descriptive-name`
- [ ] Implement changes with coverage in mind
- [ ] Test validation: `cargo test --workspace`
- [ ] Clippy validation: `cargo clippy --workspace --all-targets`
- [ ] Coverage check: No >2% drops, patch >80%
- [ ] Performance check: No >5% regressions
- [ ] Commit with conventional message
- [ ] Push: `git push -u origin <branch-name>`

#### Phase 2: PR Creation

- [ ] Use MCP/gh: `mcp__github__create_pull_request` or `gh pr create`
- [ ] Include comprehensive description
- [ ] Check status: `mcp__github__get_pull_request_status`
- [ ] Verify ALL CI GREEN

#### Phase 3: CI Monitoring

- [ ] Monitor CI: Check status every few minutes
- [ ] Verify: Build ✅, Tests ✅, Coverage ✅, Clippy ✅
- [ ] If ANY red: STOP, fix, push, re-verify

#### Phase 4: Review Response

- [ ] Check comments: `mcp__github__get_pull_request`
- [ ] For each comment: understand, implement, test, commit
- [ ] Push fixes and verify CI still GREEN

#### Phase 5: Pre-Merge

- [ ] Final status check
- [ ] Verify: `"mergeable": true`, `"statusCheckRollup": {"state": "SUCCESS"}`, `"reviewDecision": "APPROVED"`
- [ ] NEVER merge with failed/pending checks

#### Phase 6: Merge & Cleanup

- [ ] Merge: `mcp__github__merge_pull_request` or `gh pr merge`
- [ ] Switch: `git checkout main`
- [ ] Pull: `git pull origin main`
- [ ] Delete branch: `git branch -d <branch-name>`

#### Phase 7: Post-Merge

- [ ] Coverage check on main
- [ ] Test validation on main
- [ ] Mark todos complete

### Coverage & Performance Discipline

**Baseline Protocol**:

```bash
# Before changes
cargo coverage-text      # Record baseline
cargo bench-save        # Save performance
```

**CI Failure Triggers**:

- Patch coverage <80%
- File coverage drops >2%
- Overall coverage drops >1%
- Performance regression >5% without justification

**Recovery**: Add tests for uncovered paths, optimize algorithmic issues

### PR Status Monitoring

**Status Commands**:

```bash
mcp__github__get_pull_request_status --owner=ophidiarium --repo=cribo --pullNumber=<NUM>
gh pr checks <PR-number>
```

**Merge Requirements**:

- `"mergeable": true`
- `"statusCheckRollup": {"state": "SUCCESS"}`
- `"reviewDecision": "APPROVED"`
- All CI checks GREEN

## 🛠️ PROJECT TECHNICAL DETAILS

### Overview

cribo - Python source bundler (Rust) producing single .py files by inlining first-party sources.

**Features**: Tree-shaking, unused import detection, requirements.txt generation, configurable import classification.

### Commands

**Build**:

```bash
cargo build [--release]
cargo run -- --entry main.py --output bundle.py [-vvv]
uvx maturin develop        # Python package
./scripts/build-npm-binaries.sh  # npm package
```

**Test**:

```bash
cargo test --workspace
INSTA_GLOB_FILTER="**/fixture_name/main.py" cargo test test_bundling_fixtures
cargo insta accept         # Accept snapshots
```

**Benchmarks**:

```bash
cargo bench --bench bundling
cargo bench-save          # Save baseline
cargo bench-compare       # Compare
```

**Coverage**:

```bash
cargo coverage-text       # Text report
cargo coverage           # HTML report
cargo coverage-lcov      # LCOV format
```

### Architecture

1. **Bundle Orchestration** (`orchestrator.rs`): Coordinates bundling workflow
2. **Code Generation** (`code_generator.rs`): sys.modules-based bundling, AST transformations
3. **Module Resolution** (`resolver.rs`): Import classification, path resolution
4. **Dependency Graph** (`dependency_graph.rs`): Topological sorting, circular detection
5. **Unused Imports** (`unused_imports.rs`): AST-based import trimming

### CLI Usage

```bash
cribo --entry src/main.py --output bundle.py [options]
# Options: --emit-requirements, -v[vv], --stdout, --target-version
```

### Development Guidelines

#### Technical Decisions

**MANDATORY**: Never factor human limitations (time, complexity) into technical decisions. Choose optimal solutions based on:

- Performance (latency, throughput)
- Resource constraints (memory, disk, I/O)
- Maintainability and scalability

#### Logging

- Use structured logging: `log::{debug, info, warn, error}`
- Never use `println!` for debug output
- Keep debug logging that helped find bugs

#### Deterministic Output

**CRITICAL**: Bundle output must be reproducible for deployment scenarios.

- Sort imports: `from foo import d, a, b` → `from foo import a, b, d`
- Use `IndexMap`/`IndexSet` instead of `HashMap`/`HashSet`
- Apply consistent formatting regardless of input order

#### Generic Snapshot Testing

Location: `crates/cribo/tests/test_bundling_snapshots.rs`

**Usage**:

1. Create fixture: `mkdir crates/cribo/tests/fixtures/bundling/my_feature`
2. Add `main.py` and supporting modules
3. Run: `cargo test test_all_bundling_fixtures`
4. Accept: `cargo insta accept`

#### Code Standards

- Rust 2024 edition, idiomatic practices
- Strong typing, safety principles
- Doc comments on all functions
- **NEVER** hardcode test values in production
- **ALWAYS** remove unused code immediately (no deprecation)
- **ENFORCE** `.clippy.toml` disallowed lists

#### Git Operations

**MANDATORY**: Use MCP Git tools (`mcp__git__*`) instead of bash commands.

#### Conventional Commits

Format: `<type>(<scope>): <description>`

- `fix:` → patch
- `feat:` → minor
- `feat!:` or `BREAKING CHANGE:` → major

Scopes: `parser`, `bundler`, `resolver`, `ast`, `test`, `ci`, `docs`, `build`

### Tool Usage

#### LSP MCP Tools

**Rust LSP** (Working):

- ✅ `mcp__lsp-rust__diagnostics`
- ✅ `mcp__lsp-rust__definition`
- ✅ `mcp__lsp-rust__references`
- ✅ `mcp__lsp-rust__edit_file`

**YAML/TOML LSP** (Limited):

- ✅ `diagnostics` and `edit_file` work
- ❌ Symbol operations not supported

**Docs-Manager**:

- ✅ All core operations work
- ⚠️ Large repos may hit token limits

### Final Validation (MANDATORY)

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
# Both MUST pass - no exceptions
```

## Memories

- No timing estimations in documents
- macOS: use `gsed` for GNU sed
- Never use `#[allow]` - refactor instead
- Ruff repo cloned at references/type-strip/ruff
- Use bun for Node.js dependencies
- Reply to PR comments directly, don't create new reviews
- Never create files unless absolutely necessary
