# Environment Variables Reference

This document provides a reference for all environment variables supported by
Cribo.

## Supported Variables

| Variable      | Type | Effect                           | Documentation                          |
| ------------- | ---- | -------------------------------- | -------------------------------------- |
| `PYTHONPATH`  | Std  | First-party module discovery     | [PYTHONPATH](./pythonpath_support.md)  |
| `VIRTUAL_ENV` | Std  | Third-party dependency detection | [VIRTUAL_ENV](./virtualenv_support.md) |

## Configuration

Python version targeting is now configured via CLI arguments and configuration files rather than environment variables.

### Target Python Version

**CLI Argument**: `--target-version` (alias: `--python-version`)\
**Config File**: `target-version` in `cribo.toml`\
**Type**: String (e.g., "py38", "py39", "py310", "py311", "py312", "py313")\
**Default**: "py310" (Python 3.10)

Specifies which Python version to use for:

- Standard library module detection
- AST rewriting compatibility
- Language feature availability checks

```bash
# Set Python 3.11 as target version
cribo --target-version py311 -e main.py -o bundle.py

# Set Python 3.12 as target version using alias
cribo --python-version py312 -e main.py -o bundle.py

# Use default Python 3.10 (no argument needed)
cribo -e main.py -o bundle.py
```

**TOML Configuration**:

```toml
# cribo.toml
target-version = "py311"
preserve_comments = true
preserve_type_hints = false
```

**Note**: The string format follows Ruff's convention (e.g., "py310" = Python 3.10, "py311" = Python 3.11).

### `PYTHONPATH`

**Purpose**: First-party module discovery and bundling\
**Documentation**: [Python PYTHONPATH][python-pythonpath]

Directories in PYTHONPATH are scanned for modules that should be bundled as
first-party code.

```bash
# Unix/Linux/macOS (colon-separated)
PYTHONPATH="/path/to/dir1:/path/to/dir2"

# Windows (semicolon-separated) 
PYTHONPATH="C:\path\to\dir1;C:\path\to\dir2"

# Usage
export PYTHONPATH="/external/modules"
cribo --entry main.py --output bundle.py
```

### `VIRTUAL_ENV`

**Purpose**: Third-party dependency detection\
**Documentation**: [Python Virtual Environments][python-venv]

Used to identify packages installed in virtual environments. These modules are
excluded from bundling.

**Fallback Detection**: When `VIRTUAL_ENV` is not set, Cribo automatically
searches for common virtual environment directory names (`.venv`, `venv`, `env`, etc.)
in the current working directory.

```bash
# Automatic when venv is activated
source venv/bin/activate
cribo --entry main.py --output bundle.py

# Manual override
VIRTUAL_ENV=/path/to/venv cribo --entry main.py --output bundle.py

# Automatic fallback detection (no VIRTUAL_ENV needed)
# Cribo automatically detects .venv, venv, env, etc.
cribo --entry main.py --output bundle.py
```

```bash
# Automatic when venv is activated
source venv/bin/activate
cribo --entry main.py --output bundle.py

# Manual override
VIRTUAL_ENV=/path/to/venv cribo --entry main.py --output bundle.py
```

```bash
# Automatic when venv is activated
source venv/bin/activate
cribo --entry main.py --output bundle.py

# Manual override
VIRTUAL_ENV=/path/to/venv cribo --entry main.py --output bundle.py
```

## Module Resolution Priority

When modules with the same name exist in multiple locations, Cribo follows
Python's import resolution order:

1. **First-party modules** (from `src` directories and `PYTHONPATH`) -
   Highest Priority
2. **Virtual environment packages** (from `VIRTUAL_ENV`) - Lower Priority
3. **Standard library modules** - Lowest Priority

### Shadowing Example

```python
# Directory structure:
# ├── src/requests.py              (local module)
# ├── /pythonpath/numpy.py         (PYTHONPATH module)  
# └── venv/site-packages/
#     ├── requests/                (virtual env package)
#     ├── numpy/                   (virtual env package)
#     └── flask/                   (virtual env package)

import requests      # → FirstParty (shadowed by src/requests.py)
import numpy         # → FirstParty (shadowed by PYTHONPATH numpy.py)
import flask         # → ThirdParty (no shadowing, from virtual env)
```

## Environment Variable Interaction

When both `PYTHONPATH` and `VIRTUAL_ENV` are set:

```bash
export PYTHONPATH="/external/modules"
export VIRTUAL_ENV="/path/to/venv"
cribo --entry main.py --output bundle.py
```

**Result**:

- Modules from `/external/modules/` → **FirstParty** (bundled)
- Modules from `/path/to/venv/lib/python*/site-packages/` → **ThirdParty**
  (not bundled)
- Configured `src` directories → **FirstParty** (bundled)

## Platform Compatibility

### Path Separators

| Platform         | PYTHONPATH Separator | Example                      |
| ---------------- | -------------------- | ---------------------------- |
| Unix/Linux/macOS | `:` (colon)          | `/path1:/path2:/path3`       |
| Windows          | `;` (semicolon)      | `C:\path1;C:\path2;C:\path3` |

### Virtual Environment Paths

**Unix/Linux/macOS**:

- Structure: `venv/lib/python*/site-packages`
- Location: `$VIRTUAL_ENV/lib/python3.11/site-packages`

**Windows**:

- Structure: `venv\Lib\site-packages`
- Location: `%VIRTUAL_ENV%\Lib\site-packages`

## Troubleshooting

### Common Issues

1. **PYTHONPATH not recognized**

- Verify path separator matches your platform (`:` vs `;`)
- Ensure directories exist and are readable
- Check for typos in environment variable name

2. **Virtual environment not detected**

- Verify `VIRTUAL_ENV` points to virtual environment root
- Ensure virtual environment structure is standard
- Check virtual environment activation

3. **Module shadowing unexpected**

- Review module resolution priority order
- Check for name conflicts between local and virtual environment modules
- Use `--verbose` flag for detailed module discovery information

### Debug Information

Enable verbose logging to see environment variable processing:

```bash
cribo --verbose --entry main.py --output bundle.py
```

## Related Documentation

- [PYTHONPATH Support](./pythonpath_support.md) - Detailed implementation
- [VIRTUAL_ENV Support](./virtualenv_support.md) - Detailed implementation
- [Import Resolution Analysis](./cribo_import_resolution_analysis.md) -
  Overall strategy
- [Configuration Guide](../README.md) - General configuration options

## External References

- [Python PYTHONPATH Documentation][python-pythonpath]
- [Python Virtual Environments Tutorial][python-venv]
- [PEP 405 - Python Virtual Environments][pep-405]

[python-pythonpath]: https://docs.python.org/3/using/cmdline.html#envvar-PYTHONPATH
[python-venv]: https://docs.python.org/3/tutorial/venv.html
[pep-405]: https://peps.python.org/pep-0405/
