# Analysis of Import Resolution Logic in Pyrefly and Ruff

This document analyzes the value of reusing import resolution logic from [Pyrefly](https://github.com/facebook/pyrefly) and [Ruff](https://github.com/astral-sh/ruff) for the Cribo Python bundler project.

## Summary

- **Pyrefly** implements a comprehensive import resolver as part of its type-checker. It classifies first-party imports via `search_path` and third-party imports via `site_package_path`. While precise, the logic is interwoven with type analysis and is not modular or readily reusable without significant adaptation.
- **Ruff** provides a simpler, lighter-weight system for classifying imports using Rust. It identifies first-party imports via heuristics, project structure scanning, and optional configuration (e.g., `known-first-party`). The logic is well-scoped and more easily reusable.

## Recommendation

For **Cribo**, Ruff’s logic is more beneficial and easier to adapt:
- Uses the same parser ecosystem (RustPython).
- Implements fast, pragmatic first-party detection.
- Easy to integrate or replicate for bundling purposes.

If precise module resolution is needed (e.g., to trace symlinks or follow stub logic), Pyrefly’s design may serve as a conceptual reference, but its codebase is not directly pluggable.

## Integration Ideas for Cribo

- Reuse Ruff-style logic to identify which imports are from first-party modules.
- Extend Ruff’s logic slightly to map import names to filesystem paths for bundling.
- Optionally allow users to configure `known-first-party`, `src` roots, etc., in a `cribo.toml`.

## Sources

- Pyrefly: [source](https://github.com/facebook/pyrefly)
- Ruff: [source](https://github.com/astral-sh/ruff)