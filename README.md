# Cribo: Python Source Bundler

[![PyPI](https://img.shields.io/pypi/v/cribo.svg)](https://pypi.org/project/cribo/)
[![npm](https://img.shields.io/npm/v/cribo.svg)](https://www.npmjs.com/package/cribo)
[![codecov](https://codecov.io/gh/ophidiarium/cribo/graph/badge.svg?token=Lt1VqlIEqV)](https://codecov.io/gh/ophidiarium/cribo)
[![Maintainability Rating](https://sonarcloud.io/api/project_badges/measure?project=ophidiarium_cribo&metric=sqale_rating)](https://sonarcloud.io/summary/new_code?id=ophidiarium_cribo)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Cribo** is a Rust-based CLI tool that, via fast, heuristically proven bundling, consolidates a scattered Python codebase—from a single entry point or monorepo—into one idiomatic `.py` file. This not only streamlines deployment in environments like PySpark, AWS Lambda, and notebooks but also makes ingesting Python codebases into AI models easier and more cost-effective while preserving full functional insights.

## What is "Cribo"?

*Cribo* is named after the [Mussurana snake](https://a-z-animals.com/animals/mussurana-snake/) (*Clelia clelia*), nicknamed "Cribo" in Latin America. Just like the real Cribo specializes in hunting and neutralizing venomous snakes (with a diet that's 70-80% other snakes!), our tool wrangles Python dependencies and circular imports with ease. Brazilian farmers even keep Cribos around for natural pest control—think of this as the Python ecosystem's answer to dependency chaos. In short:*Cribo eats tricky imports for breakfast, so your code doesn't have to*!

## Features

- 🦀 **Rust-based CLI** based on Ruff's Python AST parser
- 🐍 Can be installed via `pip install cribo` or `npm install cribo`
- 😎 Contemporary minds can also use `uvx cribo` or `bunx cribo`
- 🌲 **Tree-shaking** (enabled by default) to inline only the modules that are actually used
- 🔄 **Circular dependency resolution** using Tarjan's strongly connected components (SCC) analysis and function-level lazy import transformations, with detailed diagnostics
- 🧹 **Unused import trimming** to clean up Python files standalone
- 📦 **Requirements generation** with optional `requirements.txt` output
- 🔧 **Configurable** import classification and source directories
- 🚀 **Fast** and memory-efficient

## Reliability and Production Readiness

Cribo is built with production use cases in mind and is rigorously tested to ensure reliability and performance. You can confidently use it for production-grade code, backed by the following guarantees:

- **Comprehensive Test Suite**: Cribo is continuously validated against a set of approximately 100 test fixtures that cover the full spectrum of Python's import system—from simple relative imports to complex scenarios involving circular dependencies and `importlib` constructs.

- **Real-World Ecosystem Testing**: As part of every pull request, we run an "ecosystem" test suite. This involves bundling several popular open-source libraries (such as `requests`, `httpx`, `pyyaml`, `idna`, and `rich`) and executing test code against the resulting bundle to ensure real-world compatibility.

- **Performance Monitoring**: We monitor microbenchmark regressions and ecosystem build time/size performance with every change. This ensures that Cribo's performance and efficiency are maintained and improved over time, preventing regressions from making their way into releases.

## Installation

> **🔐 Supply Chain Security**: All npm and pypi packages include provenance attestations for enhanced security and verification.

### From PyPI (Python Package)

```bash
pip install cribo
```

### From npm (Node.js CLI)

```bash
# Global installation
npm install -g cribo

# One-time use
bunx cribo --help
```

### Binary Downloads

Download pre-built binaries for your platform from the [latest release](https://github.com/ophidiarium/cribo/releases/latest):

- **Linux x86_64**: `cribo_<version>_linux_x86_64.tar.gz`
- **Linux ARM64**: `cribo_<version>_linux_arm64.tar.gz`
- **macOS x86_64**: `cribo_<version>_darwin_x86_64.tar.gz`
- **macOS ARM64**: `cribo_<version>_darwin_arm64.tar.gz`
- **Windows x86_64**: `cribo_<version>_windows_x86_64.zip`
- **Windows ARM64**: `cribo_<version>_windows_arm64.zip`

Each binary includes a SHA256 checksum file for verification.

### Package Manager Installation

#### Aqua

If you use [Aqua](https://aquaproj.github.io/), add to your `aqua.yaml`:

```yaml
registries:
  - type: standard
    ref: latest
packages:
  - name: ophidiarium/cribo@latest
```

Then run:

```bash
aqua install
```

#### UBI (Universal Binary Installer)

Using [UBI](https://github.com/houseabsolute/ubi):

```bash
# Install latest version
ubi --project ophidiarium/cribo

# Install specific version
ubi --project ophidiarium/cribo --tag v0.4.1

# Install to specific directory
ubi --project ophidiarium/cribo --in /usr/local/bin
```

### From Source

```bash
git clone https://github.com/ophidiarium/cribo.git
cd cribo
cargo build --release
```

## Quick Start

### Command Line Usage

```bash
# Basic bundling
cribo --entry src/main.py --output bundle.py

# Bundle a package directory (looks for __main__.py or __init__.py)
cribo --entry mypackage/ --output bundle.py

# Generate requirements.txt
cribo --entry src/main.py --output bundle.py --emit-requirements

# Verbose output (can be repeated for more detail: -v, -vv, -vvv)
cribo --entry src/main.py --output bundle.py -v
cribo --entry src/main.py --output bundle.py -vv    # debug level
cribo --entry src/main.py --output bundle.py -vvv   # trace level

# Custom config file
cribo --entry src/main.py --output bundle.py --config my-cribo.toml
```

### CLI Options

- `-e, --entry <PATH>`: Entry point Python script or package directory (required). When pointing to a directory, Cribo will look for `__main__.py` first, then `__init__.py`
- `-o, --output <PATH>`: Output bundled Python file (required)
- `-v, --verbose...`: Increase verbosity level. Can be repeated for more detail:
  - No flag: warnings and errors only
  - `-v`: informational messages
  - `-vv`: debug messages
  - `-vvv` or more: trace messages
- `-c, --config <PATH>`: Custom configuration file path
- `--emit-requirements`: Generate requirements.txt with third-party dependencies
- `--no-tree-shake`: Disable tree-shaking optimization (tree-shaking is enabled by default)
- `--target-version <VERSION>`: Target Python version (e.g., py38, py39, py310, py311, py312, py313)
- `-h, --help`: Print help information
- `-V, --version`: Print version information

The verbose flag is particularly useful for debugging bundling issues. Each level provides progressively more detail:

```bash
# Default: only warnings and errors
cribo --entry main.py --output bundle.py

# Info level: shows progress messages
cribo --entry main.py --output bundle.py -v

# Debug level: shows detailed processing steps
cribo --entry main.py --output bundle.py -vv

# Trace level: shows all internal operations
cribo --entry main.py --output bundle.py -vvv
```

The verbose levels map directly to Rust's log levels and can also be controlled via the `RUST_LOG` environment variable for more fine-grained control:

```bash
# Equivalent to -vv
RUST_LOG=debug cribo --entry main.py --output bundle.py

# Module-specific logging
RUST_LOG=cribo::bundler=trace,cribo::resolver=debug cribo --entry main.py --output bundle.py
```

### Tree-Shaking

Tree-shaking is enabled by default to reduce bundle size by removing unused code:

```bash
# Bundle with tree-shaking (default behavior)
cribo --entry main.py --output bundle.py

# Disable tree-shaking to include all code
cribo --entry main.py --output bundle.py --no-tree-shake
```

**How it works:**

- Analyzes your code starting from the entry point
- Tracks which functions, classes, and variables are actually used
- Removes unused symbols while preserving functionality
- Respects `__all__` declarations and module side effects
- Preserves all symbols from directly imported modules (`import module`)

**When to disable tree-shaking:**

- If you encounter undefined symbol errors with complex circular dependencies
- When you need to preserve all code for dynamic imports or reflection
- For debugging purposes to see the complete bundled output

## Configuration

Cribo supports hierarchical configuration with the following precedence (highest to lowest):

1. **CLI-provided config** (`--config` flag)
2. **Environment variables** (with `CRIBO_` prefix)
3. **Project config** (`cribo.toml` in current directory)
4. **User config** (`~/.config/cribo/cribo.toml`)
5. **System config** (`/etc/cribo/cribo.toml` on Unix, `%SYSTEMDRIVE%\ProgramData\cribo\cribo.toml` on Windows)
6. **Default values**

### Configuration File Format

Create a `cribo.toml` file:

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

All configuration options can be overridden using environment variables with the `CRIBO_` prefix:

```bash
# Comma-separated lists
export CRIBO_SRC="src,lib,custom_dir"
export CRIBO_KNOWN_FIRST_PARTY="mypackage,myotherpackage"
export CRIBO_KNOWN_THIRD_PARTY="requests,numpy"

# Boolean values (true/false, 1/0, yes/no, on/off)
export CRIBO_PRESERVE_COMMENTS="false"
export CRIBO_PRESERVE_TYPE_HINTS="true"

# String values
export CRIBO_TARGET_VERSION="py312"
```

### Configuration Locations

- **Project**: `./cribo.toml`
- **User**:
  - Linux/macOS: `~/.config/cribo/cribo.toml`
  - Windows: `%APPDATA%\cribo\cribo.toml`
- **System**:
  - Linux/macOS: `/etc/cribo/cribo.toml` or `/etc/xdg/cribo/cribo.toml`
  - Windows: `%SYSTEMDRIVE%\ProgramData\cribo\cribo.toml`

## How It Works

1. **Module Discovery**: Scans configured source directories to discover first-party Python modules
2. **Import Classification**: Classifies imports as first-party, third-party, or standard library
3. **Dependency Graph**: Builds a dependency graph and performs topological sorting
4. **Circular Dependency Resolution**: Detects and intelligently resolves function-level circular imports
5. **Tree Shaking**: Removes unused code by analyzing which symbols are actually used (enabled by default)
6. **Code Generation**: Generates a single Python file with proper module separation
7. **Requirements**: Optionally generates `requirements.txt` with third-party dependencies

## Output Structure

The bundled output follows this structure:

```python
#!/usr/bin/env python3
# Generated by Cribo - Python Source Bundler

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
cribo --entry spark_job.py --output dist/spark_job_bundle.py --emit-requirements
spark-submit dist/spark_job_bundle.py
```

### AWS Lambda

Package Python Lambda functions with all dependencies:

```bash
cribo --entry lambda_handler.py --output deployment/handler.py
# Upload handler.py + requirements.txt to Lambda
```

## Special Considerations

### Pydantic Compatibility

Cribo preserves class identity and module structure to ensure Pydantic models work correctly:

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

Cribo intelligently handles circular dependencies with advanced detection and resolution:

#### Resolvable Cycles (Function-Level)

Function-level circular imports are automatically resolved and bundled successfully:

```python
# module_a.py
from module_b import process_b


def process_a():
    return process_b() + "->A"


# module_b.py
from module_a import get_value_a


def process_b():
    return f"B(using_{get_value_a()})"
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
| Cribo       | Rust     | ✅ Default   | ✅             | ✅ Smart Resolution | ✅            | ✅         |
| PyInstaller | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |
| Nuitka      | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |
| Pex         | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |

## Contributing

Please see our [Contributing Guidelines](CONTRIBUTING.md) for details on how to contribute to the project.

## License

This project uses a dual licensing approach:

- **Source Code**: Licensed under the [MIT License](LICENSE)
- **Documentation**: Licensed under the [Creative Commons Attribution 4.0 International License (CC BY 4.0)](docs/LICENSE)

### What this means

- **For the source code**: You can freely use, modify, and distribute the code for any purpose with minimal restrictions under the MIT license.
- **For the documentation**: You can share, adapt, and use the documentation for any purpose (including commercially) as long as you provide appropriate attribution under CC BY 4.0.

See the [LICENSE](LICENSE) file for the MIT license text and [docs/LICENSE](docs/LICENSE) for the CC BY 4.0 license text.

## Acknowledgments

- **Ruff**: Python AST parsing and import resolution logic inspiration
- **Maturin**: Python-Rust integration
