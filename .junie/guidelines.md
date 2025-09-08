# Cribo – Project‑specific Development Guidelines

This document captures non‑obvious, project‑specific knowledge to speed up development and debugging for advanced contributors. It is complementary to the top‑level README and CLAUDE.md.

## Build and Configuration

- Toolchain
  - The Rust toolchain is pinned via rust-toolchain.toml to nightly-2025-08-25 and requires the components: rustfmt, clippy, llvm-tools-preview, miri, rust-src, rust-analyzer, rustc-codegen-cranelift-preview. rustup will auto-select this nightly when you run cargo in the repo.
  - Verify components if needed: rustup component list --installed
- Workspace layout
  - Cargo workspace at the repo root with members = ["crates/*"]. The primary crate is crates/cribo.
  - crates/cribo defines a binary target (name = "cribo") and a library target that is gated behind the feature bench. The binary is what end-users run; the library is used only for benchmarks.
- Build commands
  - Debug: cargo build
  - Release: cargo build --release
  - Notes:
    - The workspace depends on ruff_* crates from Git (tag 0.12.5). Building requires network access and Git available on PATH.
    - Python 3 is required for tests (bundling tests execute Python); ensure python3 or python is installed and discoverable.
- Python packaging (via maturin)
  - Local develop install (recommended for working on the Python CLI wrapper): uvx maturin develop
  - Build wheels: uvx maturin build --release
  - pyproject config: [tool.maturin] points manifest-path to crates/cribo/Cargo.toml with bindings = "bin" (the Rust binary is exposed as a Python console_script).
- Node packaging (if you touch npm distribution)
  - Generation scripts live in scripts/. See CLAUDE.md; typical flow is:
    - node scripts/generate-npm-packages.js
    - ./scripts/build-npm-binaries.sh

## Testing

The test suite is Rust‑centric and comprehensive. Tests also invoke the system Python to validate generated bundles.

- Pre-requisites
  - Python 3 available as python3 or python on PATH (tests auto-detect using the helper in tests).
  - rustup will select the pinned nightly automatically.

- Run all tests
  - cargo test
  - We also support nextest (used in docs/CI): cargo nextest run --workspace
    - Install: cargo binstall cargo-nextest or cargo install cargo-nextest

- Snapshot bundling tests (Insta‑driven)
  - Framework file: crates/cribo/tests/test_bundling_snapshots.rs
  - Fixtures live under: crates/cribo/tests/fixtures/**/main.py
  - Auto‑discovery: new fixtures are picked up automatically; just add a directory with a main.py.
  - Filter which fixtures to run:
    - INSTA_GLOB_FILTER="**/stickytape_single_file/main.py" cargo nextest run --test test_bundling_snapshots
    - Or with cargo test: INSTA_GLOB_FILTER=... cargo test --test test_bundling_snapshots

- CLI behavior tests and snapshots
  - See crates/cribo/tests/test_cli_stdout.rs and snapshots in crates/cribo/tests/snapshots/.

- Ignored ecosystem tests
  - Some ecosystem checks are flagged #[ignore] (e.g., test_ecosystem_requests). Run them explicitly:
    - cargo test -- --ignored
  - These may require additional Python deps (see pyproject dependency groups) or network access.

- Benchmarks (Criterion)
  - Benches are gated behind the feature bench and have harness = false:
    - cargo bench -F bench --bench bundling
    - cargo bench -F bench --bench ecosystem
  - HTML reports are produced in target/criterion.

- Adding new tests (Rust)
  - Unit tests inside modules with #[cfg(test)] mod tests {} are fine for narrow units.
  - Prefer integration tests in crates/cribo/tests for end‑to‑end coverage. Example skeleton:

    // crates/cribo/tests/test_example.rs
    #[test]
    fn bundles_minimal_project() {
    // Arrange input under target/tmp (see below), invoke the binary, assert output
    }

- Demonstrated process (validated now)
  - We created a temporary integration test at crates/cribo/tests/test_demo.rs containing:

    #[test]
    fn test_demo_sanity() { assert_eq!(2 + 2, 4); }

  - cargo test executed it successfully along with the suite.
  - The file was then removed as it was only for demonstration.

- Adding Python tests (optional)
  - The Python package is a thin wrapper around the Rust binary (bindings = "bin"). If you add Python‑side behavior, place tests under python/ and run with:
    - uv run pytest
  - Note that [tool.uv] in pyproject.toml excludes ecosystem from the Python workspace and sets default groups (dev, fixtures, ecosystem). Use uv sync to install those groups locally when you need them.

## Development conventions and tips

- Deterministic output is mandatory
  - Use stable iteration data structures (IndexMap/IndexSet) and sort user‑visible outputs.
  - Avoid timestamps/randomness in emitted bundles. See CLAUDE.md for detailed rationale and rules.

- Logging discipline (do not break --stdout)
  - Use the log crate (debug!/info!/warn!/error!) instead of println!.
  - Keep diagnostically useful debug logs in place at appropriate levels.

- Lints and style
  - clippy pedantic and numerous additional lints are enabled at the workspace level. Keep code clippy‑clean; prefer running:
    - cargo clippy --workspace --all-targets -- -W clippy::pedantic
  - Format with rustfmt: cargo fmt
  - Python formatting (used in hooks): uv tool run ruff format --config pyproject.toml

- Git hooks (lefthook)
  - Pre‑commit hooks are defined in lefthook.yml (markdownlint, dprint, taplo, ruff, clippy, fmt, etc.). Enable them locally:
    - uvx lefthook install
  - Hooks expect tools like bunx, uv, markdownlint-cli2, taplo, yamllint to be available; see pyproject/dev dependencies and ensure they are installed (uv sync).

- Temporary files
  - Use target/tmp for any scratch input/output created by tests or tooling.

- Python packaging notes
  - maturin [tool.maturin] is configured with bindings = "bin" and manifest-path = crates/cribo/Cargo.toml. The Python package exposes the Rust binary CLI; there is no Python FFI layer to import.

- CI/local parity
  - Building requires network (ruff_* git deps). If you need reproducible offline builds, consider vendoring or a local registry mirror.

## Quick reference

- Build: cargo build --release
- Test: cargo nextest run --workspace
- Subset of bundling fixtures: INSTA_GLOB_FILTER="pattern" cargo test --test test_bundling_snapshots
- Run ignored ecosystem tests: cargo test -- --ignored
- Benchmarks: cargo bench -F bench --bench bundling
- Python: uvx maturin develop (then use cribo CLI via Python entry points if desired)
