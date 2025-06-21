# Module Resolution Algorithm

This document describes how Cribo resolves Python imports during the bundling process. The algorithm follows Python's import resolution semantics while adapting them for static analysis.

## Overview

Cribo's resolver identifies and classifies Python modules as:

- **First-party**: Modules that belong to your project (bundled)
- **Standard library**: Python's built-in modules (not bundled)
- **Third-party**: External dependencies (not bundled)

## Resolution Process

### 1. Import Discovery

When analyzing a Python file, Cribo extracts all import statements:

- `import module`
- `import package.submodule`
- `from package import module`
- `from . import relative`
- `from ..parent import module`

### 2. Search Path Construction

For bundling purposes, the "current directory" is **the directory containing the entry file**, not the directory where Cribo is executed.

For example, if the entry file is `/project/src/app/main.py`, the search path is:

```
1. /project/src/app/          # Directory containing the entry file
2. [PYTHONPATH directories]   # From PYTHONPATH environment variable
3. [Configured src dirs]      # From cribo.toml or defaults
```

This matches Python's behavior where `sys.path[0]` is the directory containing the script being run.

#### Example with Entry File

Command:

```bash
cribo --entry /home/user/myproject/src/main.py --output bundle.py
```

Search path for imports in `main.py`:

```
1. /home/user/myproject/src/  # Entry file's directory
2. /home/user/libs/           # From PYTHONPATH (if set)
3. [configured directories]    # From cribo.toml
```

**Note**: This behavior is currently not configurable. The entry file's directory is always the first in the search path.

### 3. Module Location Algorithm

For each import (e.g., `import tada`), Cribo searches each directory in the search path:

#### Step 1: Check for Package

```
Look for: <search_dir>/tada/__init__.py
If found: Load as package module
```

#### Step 2: Check for File Module

```
Look for: <search_dir>/tada.py
If found: Load as file module
```

#### Step 3: Check for Namespace Package (PEP 420)

```
Look for: <search_dir>/tada/ (directory)
If found: Continue searching other paths
Only use if no __init__.py version exists anywhere
```

**First match wins** - the search stops as soon as a module is found.

### 4. Relative Import Resolution

Relative imports are resolved within the current package structure:

#### Single Dot (`.`)

```python
# In /project/src/utils/helper.py
from . import tada
```

- Searches only in `/project/src/utils/`
- Does not fall back to other search paths

#### Multiple Dots (`..`)

```python
# In /project/src/utils/deep/helper.py
from ...data import tada
```

- Goes up two levels: `/project/src/`
- Looks for `/project/src/data/` (must be a package)
- Then resolves `tada` within that package

### 5. Module Classification

After finding a module, Cribo classifies it:

1. **First-party** if found in:
   - The entry file's directory (or subdirectories)
   - Any PYTHONPATH directory
   - Any configured source directory

2. **Standard library** if:
   - Not found in first-party paths
   - Matches Python's standard library list for the target version

3. **Third-party** if:
   - Not first-party
   - Not standard library
   - Would be found in site-packages at runtime

## Configuration

### Source Directories

Configure which directories contain first-party code:

```toml
# cribo.toml
src = ["src", "lib", "app"]
```

Default configuration:

```toml
src = ["src", "."] # Note: "." can cause performance issues
```

### Known Modules

Explicitly classify modules:

```toml
# cribo.toml
known_first_party = ["mycompany", "internal_lib"]
known_third_party = ["requests", "numpy"]
```

### Environment Variables

- `PYTHONPATH`: Additional directories to search for first-party modules
- `CRIBO_SRC`: Override source directories (comma-separated)

## Examples

### Example 1: Simple Import

Entry file: `/project/src/main.py`

```python
import helper
import requests
```

Resolution (search path starts from `/project/src/`):

1. `helper`:
   - Check `/project/src/helper/__init__.py` ❌
   - Check `/project/src/helper.py` ✅
   - Found → First-party

2. `requests`:
   - Check `/project/src/requests/__init__.py` ❌
   - Check `/project/src/requests.py` ❌
   - Not in PYTHONPATH ❌
   - Not in standard library ❌
   - → Third-party (not bundled)

### Example 2: Package Import

Entry file: `/project/app/main.py`
Content of another file `/project/app/views.py`:

```python
from utils.database import connect
```

Resolution (search path starts from `/project/app/`):

1. Look for `utils` package:
   - Check `/project/app/utils/__init__.py` ✅

2. Within `utils` package, find `database`:
   - Check `/project/app/utils/database/__init__.py` ✅

3. Import `connect` from that module

### Example 3: Relative Import

Entry file: `/project/src/main.py`
In file `/project/src/utils/helpers/string.py`:

```python
from ..database import Connection
from . import formatters
```

Resolution:

1. `from ..database`:
   - Parent package: `/project/src/utils/`
   - Check `/project/src/utils/database/__init__.py` ✅
   - Import `Connection` from that module

2. `from . import formatters`:
   - Current package: `/project/src/utils/helpers/`
   - Check `/project/src/utils/helpers/formatters/__init__.py` ❌
   - Check `/project/src/utils/helpers/formatters.py` ✅

## Differences from Python Runtime

1. **No dynamic imports**: All imports must be statically analyzable
2. **No sys.path modifications**: Search paths are fixed at bundle time
3. **Third-party exclusion**: External dependencies are not bundled
4. **Entry-relative paths**: First search path is always the entry file's directory
