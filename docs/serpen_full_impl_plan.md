# Serpen: Python Source Bundler Implementation & Testing Plan

## Overview

**Serpen** is a CLI and Python library that produces a single `.py` file from a multi-module Python project by inlining all *first-party* source files. This approach is inspired by JavaScript bundlers and aims to simplify deployment, especially in constrained environments like PySpark jobs, AWS Lambdas, and notebooks.

### Key Features
- Rust-based CLI using the RustPython parser (same as Ruff and Pyrefly).
- Supports Python 3.10 and above.
- Tree-shaking logic to inline only the modules that are actually used.
- Output includes optional `requirements.txt` with all third-party dependencies.
- Distributed as both a Rust binary and a Python package via `maturin`.

---

## Implementation Plan

### 1. CLI Interface

Use `clap` to build a CLI interface with the following arguments:
- `--entry <entry.py>`: Entry point script.
- `--output <bundle.py>`: Path to the output file.
- `--emit-requirements`: Optional flag to emit `requirements.txt`.
- `--verbose`: Enable logging/debug output.

### 2. Project Structure

```
serpen/
├── Cargo.toml
├── src/
│   ├── main.rs         # CLI
│   ├── bundler.rs      # Core bundling logic
│   ├── resolver.rs     # Import classification and path resolution
│   └── emit.rs         # Code generation
```

---

## 3. Module Resolution & Import Classification

Serpen will use **Ruff's import resolution logic** as a foundation. Ruff implements efficient import categorization into standard library, first-party, and third-party, relying on:
- Module name matching
- Project root scanning
- Configurable known-first-party lists

### Reuse Plan
- Reuse/adapt logic from:
  - [`is_first_party`](https://github.com/astral-sh/ruff/blob/main/crates/ruff_python_resolver/src/importer.rs#L287)
  - [`ImportMap` and import classification rules](https://github.com/astral-sh/ruff/blob/main/crates/ruff_python_resolver/src/importer.rs)
  - [`Settings.src`](https://github.com/astral-sh/ruff/blob/main/crates/ruff_python_resolver/src/settings.rs)

### Adaptations
- Extend Ruff’s logic to resolve actual source file paths.
- Implement a cache for resolved modules to avoid redundant disk I/O.
- Allow users to define `known-first-party`, `known-third-party`, and `src` in `serpen.toml`.

---

## 4. AST Parsing

- Use `rustpython-parser` to parse Python files into ASTs.
- Extract and walk `Import` and `ImportFrom` nodes.
- Preserve comments and type hints in v1; allow optional removal in v2.

---

## 5. Dependency Graph

- Traverse ASTs to construct a directed graph of modules.
- Use topological sorting to determine bundling order.
- Detect and handle cyclic imports (report error).

---

## 6. AST Rewriting & Emission

- Walk ASTs to remove inlined `import` statements (when targeting bundled modules).
- Output formatted code with headers between modules (e.g. `# ─ Module: utils/helpers.py ─`).
- Leave third-party and standard imports intact.

---

## 7. Requirements Generation

- Identify all `Import` statements not resolved as first-party.
- Optionally emit a `requirements.txt` file.
- Use heuristics (e.g. presence in `site-packages`, exclusion from standard library list).

---

## 8. Build & Distribution

- Build with `maturin` for Python packaging.
- Package will be installable via `pip install serpen`.
- Binary will also be published to GitHub Releases.

---

## 9. Testing Plan

- **Unit Tests**:
  - Module resolution and import classification.
  - AST traversal and dependency graph generation.
- **Integration Tests**:
  - Full bundle generation from small multi-module projects.
  - Compatibility with Pydantic and Pandera decorators.
  - Output validation on PySpark runtime.

- **Golden Tests**:
  - Match output `.py` files against expected snapshots.

---

## 10. Special Considerations

- **Pydantic Compatibility**:
  - Ensure class identity is preserved (no name shadowing).
- **Pandera Decorators**:
  - Inline decorators must retain `__module__` and definition structure.
- **PySpark**:
  - Avoid module name conflicts with built-in PySpark utilities.
  - Ensure `__main__` script structure is preserved.

---

## 11. Related Analysis

See [Import Resolution Analysis](serpen_import_resolution_analysis.md) for a deep comparison of Pyrefly vs Ruff logic.

---

## Appendix: Future Enhancements (v2)

- Strip comments and static type hints.
- Support source maps or tracebacks.
- Parallel parsing and bundling.
- Bundling mode for `__init__.py` package flattening.