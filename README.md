# Serpen: Python Source Bundler

[![PyPI](https://img.shields.io/pypi/v/serpen.svg)](https://pypi.org/project/serpen/)
[![npm](https://img.shields.io/npm/v/serpen.svg)](https://www.npmjs.com/package/serpen)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Serpen** is a CLI and Python library that produces a single `.py` file from a multi-module Python project by inlining all *first-party* source files. This approach is inspired by JavaScript bundlers and aims to simplify deployment, especially in constrained environments like PySpark jobs, AWS Lambdas, and notebooks.

## Features

- 🦀 **Rust-based CLI** using Ruff's Python AST parser
- 🐍 **Python 3.10+** support
- 🌲 **Tree-shaking logic** to inline only the modules that are actually used
- 🔄 **Smart circular dependency resolution** with detailed diagnostics
- 🧹 **Unused import trimming** to clean up Python files standalone
- 📦 **Requirements generation** with optional `requirements.txt` output
- 🔧 **Configurable** import classification and source directories
- 🚀 **Fast** and memory-efficient
- 🐍 **Python API** available via maturin packaging

## Installation

### From PyPI (Python Package)

```bash
pip install serpen
```

### From npm (Node.js CLI)

```bash
# Global installation
npm install -g serpen

# One-time use
npx serpen --help
```

> **🔐 Supply Chain Security**: All npm packages include [provenance attestations](docs/NPM_PROVENANCE.md) for enhanced security and verification.

### Binary Downloads

Download pre-built binaries for your platform from the [latest release](https://github.com/tinovyatkin/serpen/releases/latest):

- **Linux x86_64**: `serpen_*_linux_x86_64.tar.gz`
- **Linux ARM64**: `serpen_*_linux_arm64.tar.gz`
- **macOS x86_64**: `serpen_*_darwin_x86_64.tar.gz`
- **macOS ARM64**: `serpen_*_darwin_arm64.tar.gz`
- **Windows x86_64**: `serpen_*_windows_x86_64.zip`
- **Windows ARM64**: `serpen_*_windows_arm64.zip`

Each binary includes a SHA256 checksum file for verification.

### Package Manager Installation

#### Aqua

If you use [Aqua](https://aquaproj.github.io/), add to your `aqua.yaml`:

```yaml
registries:
  - type: standard
    ref: latest
packages:
  - name: tinovyatkin/serpen@latest
```

Then run:

```bash
aqua install
```

#### UBI (Universal Binary Installer)

Using [UBI](https://github.com/houseabsolute/ubi):

```bash
# Install latest version
ubi --project tinovyatkin/serpen

# Install specific version
ubi --project tinovyatkin/serpen --tag v0.4.1

# Install to specific directory
ubi --project tinovyatkin/serpen --in /usr/local/bin
```

### From Source

```bash
git clone https://github.com/tinovyatkin/serpen.git
cd serpen
cargo build --release
```

## Quick Start

### Command Line Usage

```bash
# Basic bundling
serpen --entry src/main.py --output bundle.py

# Generate requirements.txt
serpen --entry src/main.py --output bundle.py --emit-requirements

# Verbose output
serpen --entry src/main.py --output bundle.py --verbose

# Custom config file
serpen --entry src/main.py --output bundle.py --config my-serpen.toml
```

## Configuration

Serpen supports hierarchical configuration with the following precedence (highest to lowest):

1. **CLI-provided config** (`--config` flag)
2. **Environment variables** (with `SERPEN_` prefix)
3. **Project config** (`serpen.toml` in current directory)
4. **User config** (`~/.config/serpen/serpen.toml`)
5. **System config** (`/etc/serpen/serpen.toml` on Unix, `%SYSTEMDRIVE%\ProgramData\serpen\serpen.toml` on Windows)
6. **Default values**

### Configuration File Format

Create a `serpen.toml` file:

```toml
# Source directories to scan for first-party modules
src = ["src", ".", "lib"]

# Known first-party module names
known_first_party = [
    "my_internal_package",
]

# Known third-party module names
known_third_party = [
    "requests",
    "numpy",
    "pandas",
]

# Whether to preserve comments in the bundled output
preserve_comments = true

# Whether to preserve type hints in the bundled output
preserve_type_hints = true

# Target Python version for standard library checks
# Supported: "py38", "py39", "py310", "py311", "py312", "py313"
target-version = "py310"
```

### Environment Variables

All configuration options can be overridden using environment variables with the `SERPEN_` prefix:

```bash
# Comma-separated lists
export SERPEN_SRC="src,lib,custom_dir"
export SERPEN_KNOWN_FIRST_PARTY="mypackage,myotherpackage"
export SERPEN_KNOWN_THIRD_PARTY="requests,numpy"

# Boolean values (true/false, 1/0, yes/no, on/off)
export SERPEN_PRESERVE_COMMENTS="false"
export SERPEN_PRESERVE_TYPE_HINTS="true"

# String values
export SERPEN_TARGET_VERSION="py312"
```

### Configuration Locations

- **Project**: `./serpen.toml`
- **User**:
  - Linux/macOS: `~/.config/serpen/serpen.toml`
  - Windows: `%APPDATA%\serpen\serpen.toml`
- **System**:
  - Linux/macOS: `/etc/serpen/serpen.toml` or `/etc/xdg/serpen/serpen.toml`
  - Windows: `%SYSTEMDRIVE%\ProgramData\serpen\serpen.toml`

## How It Works

1. **Module Discovery**: Scans configured source directories to discover first-party Python modules
2. **Import Classification**: Classifies imports as first-party, third-party, or standard library
3. **Dependency Graph**: Builds a dependency graph and performs topological sorting
4. **Circular Dependency Resolution**: Detects and intelligently resolves function-level circular imports
5. **Tree Shaking**: Only includes modules that are actually imported (directly or transitively)
6. **Code Generation**: Generates a single Python file with proper module separation
7. **Requirements**: Optionally generates `requirements.txt` with third-party dependencies

## Output Structure

The bundled output follows this structure:

```python
#!/usr/bin/env python3
# Generated by Serpen - Python Source Bundler

# Preserved imports (stdlib and third-party)
import os
import sys
import requests

# ─ Module: utils/helpers.py ─
def greet(name: str) -> str:
    return f"Hello, {name}!"

# ─ Module: models/user.py ─
class User:
    def **init**(self, name: str):
        self.name = name

# ─ Entry Module: main.py ─
from utils.helpers import greet
from models.user import User

def main():
    user = User("Alice")
    print(greet(user.name))

if **name** == "**main**":
    main()
```

## Use Cases

### PySpark Jobs

Deploy complex PySpark applications as a single file:

```bash
serpen --entry spark_job.py --output dist/spark_job_bundle.py --emit-requirements
spark-submit dist/spark_job_bundle.py
```

### AWS Lambda

Package Python Lambda functions with all dependencies:

```bash
serpen --entry lambda_handler.py --output deployment/handler.py
# Upload handler.py + requirements.txt to Lambda
```

## Special Considerations

### Pydantic Compatibility

Serpen preserves class identity and module structure to ensure Pydantic models work correctly:

```python
# Original: models/user.py
class User(BaseModel):
    name: str

# Bundled output preserves **module** and class structure
```

### Pandera Decorators

Function and class decorators are preserved with their original module context:

```python
# Original: validators/schemas.py
@pa.check_types
def validate_dataframe(df: DataFrame[UserSchema]) -> DataFrame[UserSchema]:
    return df

# Bundled output maintains decorator functionality
```

### Circular Dependencies

Serpen intelligently handles circular dependencies with advanced detection and resolution:

#### Resolvable Cycles (Function-Level)

Function-level circular imports are automatically resolved and bundled successfully:

```python
# module_a.py
from module_b import process_b
def process_a(): return process_b() + "->A"

# module_b.py  
from module_a import get_value_a
def process_b(): return f"B(using_{get_value_a()})"
```

**Result**: ✅ Bundles successfully with warning log

#### Unresolvable Cycles (Module Constants)

Temporal paradox patterns are detected and reported with detailed diagnostics:

```python
# constants_a.py
from constants_b import B_VALUE
A_VALUE = B_VALUE + 1  # ❌ Unresolvable

# constants_b.py
from constants_a import A_VALUE  
B_VALUE = A_VALUE * 2  # ❌ Temporal paradox
```

**Result**: ❌ Fails with detailed error message and resolution suggestions:

```bash
Unresolvable circular dependencies detected:

Cycle 1: constants_b → constants_a
  Type: ModuleConstants
  Reason: Module-level constant dependencies create temporal paradox - cannot be resolved through bundling
```

## Comparison with Other Tools

| Tool        | Language | Tree Shaking | Import Cleanup | Circular Deps       | PySpark Ready | Type Hints |
| ----------- | -------- | ------------ | -------------- | ------------------- | ------------- | ---------- |
| Serpen      | Rust     | ✅           | ✅             | ✅ Smart Resolution | ✅            | ✅         |
| PyInstaller | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |
| Nuitka      | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |
| Pex         | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |

## Development

### Building from Source

```bash
git clone https://github.com/tinovyatkin/serpen.git
cd serpen

# Build Rust CLI
cargo build --release

# Build Python package
pip install maturin
maturin develop

# Run tests
cargo test
```

### Project Structure

```text
serpen/
├── src/                    # Rust source code
│   ├── main.rs            # CLI entry point
│   ├── bundler.rs         # Core bundling logic
│   ├── resolver.rs        # Import resolution
│   ├── emit.rs            # Code generation
│   └── ...
├── python/serpen/         # Python package
├── tests/                 # Test suites
│   └── fixtures/          # Test projects
├── docs/                  # Documentation
└── Cargo.toml            # Rust dependencies
```

## Contributing

### Development Setup

```bash
# Clone the repository
git clone https://github.com/tinovyatkin/serpen.git
cd serpen

# Install Rust toolchain and components
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov

# Build Rust CLI
cargo build --release

# Build Python package
pip install maturin
maturin develop

# Run tests
cargo test
```

### Code Coverage

The project uses `cargo-llvm-cov` for code coverage analysis:

```bash
# Generate text coverage report (Istanbul-style)
cargo coverage-text

# Generate HTML coverage report and open in browser
cargo coverage

# Generate LCOV format for CI
cargo coverage-lcov

# Clean coverage data
cargo coverage-clean
```

**Branch Coverage (Experimental)**:

```bash
# Requires nightly Rust for branch coverage
cargo +nightly coverage-branch
```

Coverage reports are automatically generated in CI and uploaded to Codecov. See [`docs/coverage.md`](docs/coverage.md) for detailed coverage documentation.

**Note**: If you see zeros in the "Branch Coverage" column in HTML reports, this is expected with stable Rust. Branch coverage requires nightly Rust and is experimental.

### Contributing Guidelines

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Submit a pull request

## License

This project uses a dual licensing approach:

- **Source Code**: Licensed under the [MIT License](LICENSE)
- **Documentation**: Licensed under the [Creative Commons Attribution 4.0 International License (CC BY 4.0)](docs/LICENSE)

### What this means:

- **For the source code**: You can freely use, modify, and distribute the code for any purpose with minimal restrictions under the MIT license.
- **For the documentation**: You can share, adapt, and use the documentation for any purpose (including commercially) as long as you provide appropriate attribution under CC BY 4.0.

See the [LICENSE](LICENSE) file for the MIT license text and [docs/LICENSE](docs/LICENSE) for the CC BY 4.0 license text.

## Acknowledgments

- **Ruff**: Python AST parsing and import resolution logic inspiration
- **Maturin**: Python-Rust integration

## Roadmap

- [x] **Smart circular dependency resolution** - ✅ Completed in v0.4.4+
- [ ] Source maps for debugging
- [ ] Parallel processing
- [ ] Package flattening mode
- [ ] Comment and type hint stripping
- [ ] Plugin system for custom transformations

---

For more examples and detailed documentation, visit our [documentation site](https://github.com/tinovyatkin/serpen#readme).

For detailed documentation on the unused import trimmer, see [`docs/unused_import_trimmer.md`](docs/unused_import_trimmer.md).
