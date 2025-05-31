---
applyTo: "python/**/*"
---

# Python SDK (serpen package) Development Guidelines

This document provides specific instructions for developing the Python SDK.

- The Python SDK uses `uv` for dependency management and `uvx` for tools run.
- The core `serpen` package, located in `python/serpen/`, is a mixed Rust/Python project built with PyO3 and maturin.

### Maturin Project Structure (for `python/serpen/`)

- The `python/serpen/` directory, which houses the core `serpen` Python package, is structured as a Maturin mixed Rust/Python project.
- Key configuration files, `pyproject.toml` (for Python packaging and Maturin settings) and `Cargo.toml` (for Rust crate definition), are located at the root of this directory (`python/serpen/`).
- When a user imports the package in Python (e.g., `import serpen`), the primary Python module loaded is `python/serpen/__init__.py` (and other Python files within `python/serpen/`).
- The Rust-native components, compiled by Maturin, are made available as a submodule, typically `serpen._serpen_rust`. This submodule contains the functions and structs exposed from the Rust side via PyO3.
