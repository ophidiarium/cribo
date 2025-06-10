# Cribo: Python Source Bundler

[![PyPI](https://img.shields.io/pypi/v/cribo.svg)](https://pypi.org/project/cribo/)
[![npm](https://img.shields.io/npm/v/cribo.svg)](https://www.npmjs.com/package/cribo)
[![codecov](https://codecov.io/gh/ophidiarium/cribo/graph/badge.svg?token=Lt1VqlIEqV)](https://codecov.io/gh/ophidiarium/cribo)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Cribo** is a CLI and Python library that produces a single `.py` file from a multi-module Python project by inlining all *first-party* source files. This approach is inspired by JavaScript bundlers and aims to simplify deployment, especially in constrained environments like PySpark jobs, AWS Lambdas, and notebooks.

## Features

- 🦀 **Rust-based CLI** using Ruff's Python AST parser
- 🐍 **Python 3.10+** support
- 🌲 **Tree-shaking logic** to inline only the modules that are actually used
- 🔄 **Circular dependency resolution** using Tarjan's strongly connected components (SCC) analysis and function-level lazy import transformations, with detailed diagnostics
- 🧹 **Unused import trimming** to clean up Python files standalone
- 📦 **Requirements generation** with optional `requirements.txt` output
- 🔧 **Configurable** import classification and source directories
- 🚀 **Fast** and memory-efficient
- 📊 **Performance tracking** with built-in benchmarking

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

- `-e, --entry <PATH>`: Entry point Python script (required)
- `-o, --output <PATH>`: Output bundled Python file (required)
- `-v, --verbose...`: Increase verbosity level. Can be repeated for more detail:
  - No flag: warnings and errors only
  - `-v`: informational messages
  - `-vv`: debug messages
  - `-vvv` or more: trace messages
- `-c, --config <PATH>`: Custom configuration file path
- `--emit-requirements`: Generate requirements.txt with third-party dependencies
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
5. **Tree Shaking**: Only includes modules that are actually imported (directly or transitively)
6. **Code Generation**: Generates a single Python file with proper module separation
7. **Requirements**: Optionally generates `requirements.txt` with third-party dependencies

### Architecture Overview

Cribo uses a two-stage architecture for clean separation of concerns:

- **BundleOrchestrator** (`orchestrator.rs`): Handles the high-level bundling workflow
  - Module discovery and import resolution
  - Dependency graph construction and analysis
  - Circular dependency detection using Tarjan's algorithm
  - Coordination of the overall bundling process

- **HybridStaticBundler** (`code_generator.rs`): Manages Python code generation
  - Implements the sys.modules-based bundling approach
  - Generates deterministic module names using content hashing
  - Handles AST transformations and import rewriting
  - Integrates unused import trimming
  - Produces the final bundled Python output

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
| Cribo       | Rust     | ✅           | ✅             | ✅ Smart Resolution | ✅            | ✅         |
| PyInstaller | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |
| Nuitka      | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |
| Pex         | Python   | ❌           | ❌             | ❌ Fails            | ❌            | ✅         |

## Development

### Building from Source

```bash
git clone https://github.com/ophidiarium/cribo.git
cd cribo

# Build Rust CLI
cargo build --release

# Build Python package
pip install maturin
maturin develop

# Run tests
cargo test
```

### Performance Benchmarking

Cribo uses [Bencher.dev](https://bencher.dev) for comprehensive performance tracking with statistical analysis and regression detection:

```bash
# Run all benchmarks
cargo bench

# Save a performance baseline
./scripts/bench.sh --save-baseline main

# Compare against baseline
./scripts/bench.sh --baseline main

# View detailed HTML report
./scripts/bench.sh --open
```

**Key benchmarks:**

- **End-to-end bundling**: Full project bundling performance (Criterion.rs)
- **AST parsing**: Python code parsing speed (Criterion.rs)
- **Module resolution**: Import resolution efficiency (Criterion.rs)
- **CLI performance**: Command-line interface speed (Hyperfine)

**CI Integration:**

- Automated PR comments with performance comparisons and visual charts
- Historical performance tracking with trend analysis
- Statistical significance testing to prevent false positives
- Dashboard available at [bencher.dev/perf/cribo](https://bencher.dev/perf/cribo)

See [docs/benchmarking.md](docs/benchmarking.md) for detailed benchmarking guide.

### Project Structure

```text
cribo/
├── src/                    # Rust source code
│   ├── main.rs            # CLI entry point
│   ├── orchestrator.rs    # Bundle orchestration and coordination
│   ├── code_generator.rs  # Python code generation (sys.modules approach)
│   ├── resolver.rs        # Import resolution
│   ├── dependency_graph.rs # Dependency analysis and circular detection
│   ├── unused_imports.rs  # Unused import trimming
│   └── ...
├── python/cribo/          # Python package
├── tests/                 # Test suites
│   └── fixtures/          # Test projects
├── docs/                  # Documentation
└── Cargo.toml            # Rust dependencies
```

## Contributing

### Development Setup

```bash
# Clone the repository
git clone https://github.com/ophidiarium/cribo.git
cd cribo

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

For more examples and detailed documentation, visit our [documentation site](https://github.com/ophidiarium/cribo#readme).

For detailed documentation on the unused import trimmer, see [`docs/unused_import_trimmer.md`](docs/unused_import_trimmer.md).
