# Circular Dependencies in Python Bundling

## Overview

Cribo detects and attempts to handle circular dependencies in Python code during the bundling process. However, certain patterns of circular dependencies, particularly those involving parent-child module relationships with cross-imports, are currently not fully supported.

## Known Limitations

### Parent-Child Module Circular Dependencies

When a child module imports from its parent module using relative imports (e.g., `from . import something`), and the parent module also imports from the child, this creates a circular dependency that the current bundler architecture cannot resolve.

#### Example Pattern (Currently Unsupported)

```python
# core/database/__init__.py
from .connection import connect  # Parent imports from child

# core/database/connection.py  
from . import _registered_types  # Child imports from parent
```

This pattern is tested in `xfail_cross_package_mixed_import` test fixture, which is expected to fail.

### Why This Happens

In Python's normal execution model:

1. A module starts initializing
2. It can define some attributes
3. It can then import from submodules
4. Submodules can access the already-defined attributes from the parent

However, Cribo's bundling approach tries to fully initialize each module before moving to the next, which doesn't work with circular dependencies where modules need partial initialization states.

## Detection and Warnings

Cribo will detect circular dependencies and classify them:

- **Resolvable cycles**: Function-level or class-level imports that can be handled through import rewriting
- **Unresolvable cycles**: Module-level circular dependencies that cannot be automatically resolved

When circular dependencies are detected, you'll see warnings like:

```
[WARN] Detected N potentially resolvable circular dependencies
[WARN] Cycle 1: core.database → core.database.connection (Type: FunctionLevel)
```

## Workarounds

1. **Avoid circular imports**: Restructure your code to eliminate circular dependencies
2. **Use lazy imports**: Move imports inside functions where they're needed
3. **Extract common code**: Move shared code to a separate module that both modules can import from

## Technical Details

The issue stems from how Python module initialization order is handled in the bundler:

1. The bundler needs to determine module initialization order
2. When modules have circular dependencies, there's no valid topological sort
3. The current approach of using temporary variables (`_cribo_temp_*`) for module initialization doesn't handle cases where child modules need to access parent module attributes during their own initialization

This is an architectural limitation that would require significant changes to the bundling approach to fully resolve.
