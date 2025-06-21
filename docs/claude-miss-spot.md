# AST Rewriting Collision Patterns Analysis

## Overview

This analysis compares three AST rewriting test fixtures:

1. `ast_rewriting_globals_collision` - Tests global variable access patterns
2. `ast_rewriting_symbols_collision` - Tests symbol collision handling
3. `ast_rewriting_mixed_collisions` - Tests complex patterns expected to fail

## Key Patterns Missing in First Two Fixtures

After analyzing all three fixtures, the `ast_rewriting_mixed_collisions` fixture exercises several critical patterns that the first two fixtures miss:

### 1. Dynamic Global Dictionary Access with `globals()`

**Pattern**: Direct runtime access to the global namespace dictionary

```python
# In ast_rewriting_mixed_collisions/main.py:100
"total": result + str(globals()["result"])  # Runtime globals() access

# In ast_rewriting_mixed_collisions/services/auth/manager.py:90
module_validate = globals().get("validate")

# In ast_rewriting_mixed_collisions/services/auth/manager.py:131
User = globals()["User"]  # Get class from globals dictionary
```

**Why it matters**: The `globals()` function returns the actual runtime namespace dictionary, making it impossible to statically rewrite references. This creates a fundamental challenge for AST rewriting since the access is dynamic and happens at runtime.

### 2. Complex Cross-Module Global State Dependencies

**Pattern**: Multiple modules modifying globals with same names but different types/purposes

```python
# In ast_rewriting_mixed_collisions/services/auth/manager.py:64,79
def process(data: Any) -> str:
    global result
    result = f"{result}_processed"  # String concatenation

# In ast_rewriting_mixed_collisions/core/database/connection.py:36,52
def process(data):
    global result  # Different module, same name
    result.append(processed)  # List append - different type!

# In ast_rewriting_mixed_collisions/core/utils/helpers.py:35,47
def process(data: Any) -> str:
    global result
    result += 1  # Integer increment - yet another type!
```

**Why it matters**: Unlike the simpler global patterns in `ast_rewriting_globals_collision` (which also uses the `global` keyword), this fixture has multiple modules with globals of the same name but incompatible types and operations. The bundler must maintain module isolation while handling these type conflicts.

### 3. Complex Import-Time Side Effects

**Pattern**: Modules with lambda assignments and type() calls at import time

```python
# In ast_rewriting_mixed_collisions/models/base.py:40-42
def initialize():
    global Connection
    Connection = type("Connection", (), {"type": "base_connection"})
```

**Why it matters**: Dynamic type creation and global modifications during import create ordering dependencies that must be preserved in the bundled output.

### 4. Shadowing with Runtime Type Introspection

**Pattern**: Functions that check if their own name exists in globals

```python
# In ast_rewriting_mixed_collisions/services/auth/manager.py:89-96
def validate(data: Any) -> str:
    module_validate = globals().get("validate")
    if (module_validate 
        and module_validate is not validate  # Self-reference check
        and callable(module_validate)):
        lambda_result = module_validate(data)
```

**Why it matters**: The function introspects its own existence in the global namespace and handles self-references, creating complex circular dependencies.

### 5. Parameter Names Shadowing Global/Class Names

**Pattern**: Function parameters that shadow important module-level names

```python
# In ast_rewriting_mixed_collisions/services/auth/manager.py:53
def add_user(self, User: "User") -> None:  # Parameter shadows class
    self.users.append(User)  # Uses parameter, not class

# In ast_rewriting_mixed_collisions/services/auth/manager.py:103
def connect(User: Optional["User"] = None) -> Connection:
    if User:  # Parameter usage
        connection.add_user(User)
```

**Why it matters**: The bundler must correctly resolve scoping rules when parameter names conflict with module-level definitions.

### 6. Loop Variables Shadowing Class Names

**Pattern**: Iterator variables that shadow important names in their scope

```python
# In ast_rewriting_mixed_collisions/services/auth/manager.py:141
for User in self.users:  # Loop var shadows class name
    user_result = process(User.username)  # User here is the loop variable
```

**Why it matters**: The bundler must maintain proper scoping for loop variables even when they shadow module-level names.

### 7. Cross-Package Relative Imports

**Pattern**: Complex import paths crossing package boundaries

```python
# In ast_rewriting_mixed_collisions/core/database/connection.py:5-6
from models.user import process_user  # Cross-package import
from ..utils.helpers import validate as helper_validate  # Relative import
```

**Why it matters**: These create complex dependency graphs that must be properly resolved and maintained in the bundled output.

## Summary

The key difference is that `ast_rewriting_mixed_collisions` tests **runtime-dependent** patterns that cannot be resolved through static AST analysis alone:

1. **Dynamic namespace access** via `globals()` - The most critical differentiator
2. **Type-conflicting cross-module globals** - Same name, incompatible types across modules
3. **Runtime type introspection** and self-reference checks
4. **Complex shadowing** at multiple scopes (parameters, loop variables)
5. **Import-time side effects** that modify global state dynamically

These patterns represent fundamental challenges for AST rewriting because they rely on Python's dynamic runtime behavior rather than static code structure. While both `ast_rewriting_globals_collision` and `ast_rewriting_mixed_collisions` use the `global` keyword, the mixed collisions fixture specifically combines this with `globals()` dictionary access and type-incompatible operations, creating patterns that break static transformation approaches.
