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
- 🔎 **Dependency detection** (`cribo deps`) that reports the third-party requirements of any Python file or directory without bundling
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

# Detect third-party dependencies without bundling (see "Dependency Detection")
cribo deps --entry src/main.py

# Resolve requirement names from a specific Python environment
cribo --entry src/main.py --output bundle.py --emit-requirements \
  --python .venv/bin/python

# Verbose output (can be repeated for more detail: -v, -vv, -vvv)
cribo --entry src/main.py --output bundle.py -v
cribo --entry src/main.py --output bundle.py -vv    # debug level
cribo --entry src/main.py --output bundle.py -vvv   # trace level

# Custom config file
cribo --entry src/main.py --output bundle.py --config my-cribo.toml

# Generate a Source Map v3 and remap runtime tracebacks to original sources
cribo --entry src/main.py --output bundle.py --sourcemap
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
- `--python <PATH>`: Python interpreter whose installed distribution metadata is used for requirements
- `--no-tree-shake`: Disable tree-shaking optimization (tree-shaking is enabled by default)
- `--target-version <VERSION>`: Target Python version (e.g., py38, py39, py310, py311, py312, py313)
- `--sourcemap[=<MODE>]`: Generate a Source Map v3 for the bundle and inject a traceback-remapping runtime. Modes: `linked` (default; writes `<output>.map` plus a `# sourceMappingURL=` comment), `inline` (embeds the map as a base64 comment; the default with `--stdout`), `external` (writes `<output>.map` with no comment). See [Source Maps](#source-maps)
- `--sources-content=<BOOL>`: Force embedding original sources in the map (default: omitted for `inline`, included for `linked`/`external`)
- `-h, --help`: Print help information
- `-V, --version`: Print version information

Cribo also provides a `deps` subcommand for third-party dependency detection without bundling — see [Dependency Detection](#dependency-detection-cribo-deps).

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

### Source Maps

Cribo can emit a [Source Map v3](https://tc39.es/ecma426/) (the language-agnostic
format used across the JavaScript ecosystem) mapping every statement in the bundle
back to its original file and line, plus an injected runtime that remaps uncaught
exception tracebacks to the original sources — analogous to
`node --enable-source-maps`:

```bash
# linked (default): writes bundle.py.map + a trailing sourceMappingURL comment
cribo --entry src/main.py --output bundle.py --sourcemap

# inline: the map travels inside the bundle as a base64 comment
cribo --entry src/main.py --output bundle.py --sourcemap=inline

# external: writes bundle.py.map, no comment in the bundle
cribo --entry src/main.py --output bundle.py --sourcemap=external
```

With source maps enabled, a crash inside the bundle prints the original locations:

```text
Traceback (most recent call last):
  File "/app/src/main.py", line 3, in <module>
    boom()
  File "/app/src/helper.py", line 5, in inner
    raise ValueError("kaboom")
ValueError: kaboom
```

Runtime activation follows the delivery mode:

- `inline`: active by default; `CRIBO_SOURCE_MAPS=0` disables it
- `linked`: active exactly when `<bundle>.map` exists next to the bundle at run
  time — delete the map to ship without remapping, drop it back to re-enable
- `external`: dormant unless `CRIBO_SOURCE_MAPS=1` is set (or the variable holds a
  path to the map file)
- In every mode, `CRIBO_SOURCE_MAPS=<path>` points the runtime at an explicit map
  file — the only way to remap a bundle executed via `python -` (stdin), whose
  inline map cannot be re-read at run time

The runtime is lazy (zero file access, parsing, or decoding until the first
uncaught exception), streams the map in constant memory so it works under resource
pressure, covers `sys.excepthook`, `threading.excepthook`, and
`sys.unraisablehook`, chains to any pre-installed custom hooks, preserves the
default silent handling of `SystemExit` in worker threads, and falls back to the
standard traceback on any failure. Known limitations: user code that formats
tracebacks itself (e.g. `traceback.format_exc()`) is not remapped;
`ExceptionGroup` chains defer to the standard (unremapped but complete) rendering;
and under a hard out-of-memory condition no pure-Python hook can run.
Configuration-file equivalents: `sourcemap = "linked" | "inline" | "external"` and
`sources-content = true|false` in `cribo.toml`; environment equivalents:
`CRIBO_SOURCEMAP` and `CRIBO_SOURCES_CONTENT`. Full design:
[docs/source-maps.md](docs/source-maps.md).

### Dependency Detection (`cribo deps`)

The `deps` subcommand analyzes a Python file or directory and reports the third-party
distributions it requires — without producing a bundle. It reuses the same machinery
as bundling: module discovery, dependency-graph construction, import classification
against your environment (entry directory, `PYTHONPATH`, configured `src`
directories, and virtualenv site-packages), distribution-metadata resolution, and
tree-shaking.

```bash
# Print requirements for a script to stdout
cribo deps --entry src/main.py

# Analyze a package directory (follows __init__.py / __main__.py)
cribo deps --entry mypackage/

# Analyze a plain source directory: every script and package inside is scanned,
# including package submodules that nothing imports
cribo deps --entry src/

# Write a requirements.txt file
cribo deps --entry src/main.py --output requirements.txt

# Structured JSON with per-import detail (requirement mapping, which first-party
# modules import it, and whether it is TYPE_CHECKING-only or conditional)
cribo deps --entry src/main.py --format json

# Skip imports used only under `if TYPE_CHECKING:`
cribo deps --entry src/main.py --exclude-type-checking

# Skip imports that only appear inside conditional control flow
# (if/elif/else, try/except, loops)
cribo deps --entry src/main.py --exclude-conditional

# Include imports even when only unused code references them
cribo deps --entry src/main.py --no-tree-shake

# Resolve requirement names from a specific Python environment
cribo deps --entry src/main.py --python .venv/bin/python
```

**Options:**

- `-e, --entry <PATH>`: Entry point — a Python file, a package directory, or a plain directory of sources (required)
- `-o, --output <PATH>`: Write the report to a file instead of stdout
- `--format <FORMAT>`: `requirements` (default; one PEP 508 requirement per line) or `json` (structured report with per-import detail)
- `--exclude-type-checking`: Exclude imports used only within `if TYPE_CHECKING:` blocks
- `--exclude-conditional`: Exclude imports that appear only inside conditional control flow; `TYPE_CHECKING` blocks are governed by `--exclude-type-checking` instead
- `--no-tree-shake`: Include third-party imports even when they are only referenced by unused code

**Behavior notes:**

- Import names are mapped to distribution names (e.g. `cv2` → `opencv-python`) through installed distribution metadata, honoring `requirements.module-map` overrides from your configuration
- With tree-shaking enabled (the default), imports that only unused code references are omitted — matching what a bundle would actually need
- `TYPE_CHECKING`-only and conditional (try/except, if-block) imports are included by default and are controlled exclusively by their dedicated flags, since such imports are genuine (if optional) dependencies
- When scanning a plain directory, files that fail to parse are skipped with a warning instead of aborting the whole scan; a single-file entry still fails fast

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

[requirements]
# Interpreter used to map imports to installed distribution names
python = ".venv/bin/python"

# Explicit longest-prefix mappings for ambiguous or unavailable metadata
[requirements.module-map]
sklearn = "scikit-learn"
"google.cloud.storage" = "google-cloud-storage>=2"
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
export CRIBO_PYTHON=".venv/bin/python"
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
