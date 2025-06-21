# The Missing Spotter

This document consolidates and organizes all missing AST rewriting patterns identified in the `ast_rewriting_globals_collision`, `ast_rewriting_symbols_collision`, and `ast_rewriting_mixed_collisions` fixtures. Patterns are ordered by their impact on static analysis and bundling correctness, from most critical to least critical.

## 1. Critical Runtime Patterns

### 1.1 Dynamic Global Dictionary Access via `globals()`

Direct runtime lookups into the module namespace that cannot be resolved statically.

- `main.py`: `result + str(globals()["result"])` (mixed fixture)
- `services/auth/manager.py`: `module_validate = globals().get("validate")`, `User = globals()["User"]`
- `models/base.py`: rebinding of `validate`, `process`, `Logger` via `globals()`
- `models/user.py`: recovery of `Logger` and class definitions after shadowing

### 1.2 Recovery of Shadowed Names via `globals()`

Dynamic re-import of original definitions following parameter or loop-variable shadowing.

- `models/base.shadow_test()`: restore `validate`, `process`, `Logger` (mixed)
- `models/user.complex_operation()`: rebind class references from `globals()`
- `AuthManager.add_user()`: recover `User` despite local parameter shadowing

### 1.3 Runtime Type Introspection and Self-Reference

Functions that inspect their own binding in the global namespace and handle self-references.

- `services/auth/manager.py`.`validate()`: checks `globals().get("validate") is not validate`

## 2. Global State Mutation Patterns

### 2.1 Module-Level `global` Mutation

Functions and methods mutating module-scoped variables at runtime.

- `core/database/connection.py`: `global connection`, `global result` in multiple scopes
- `core/utils/helpers.py`: `global result` increment
- `models/base.py`: `global result` in initializers and processors
- `models/user.py`: `global connection`, `global result` in `process_user()`
- `services/auth/manager.py`: `global result` updates auth state
- `main.py`: `global connection` capture

### 2.2 Complex Cross-Module Global Dependencies

Same global name used across modules with incompatible types or semantics.

- `core/database/connection.py`: `result` as list vs string vs integer in other modules
- `services/auth/manager.py`, `core/utils/helpers.py`: divergent `result` operations

### 2.3 Import-Time Side Effects Creating Globals

Dynamic modifications to module-level bindings at import time.

- `models/base.py`: `Connection = type("Connection", ..., {"type": "base_connection"})` in initializer

## 3. Import Resolution Patterns

### 3.1 Combined Absolute and Relative Imports

Modules combining deep absolute imports with sibling-relative imports in one file.

- `core/database/connection.py`: `from models.user import process_user` + `from ..utils.helpers import validate`

### 3.2 Cross-Package Relative Imports

Complex import paths that cross package or directory boundaries, affecting bundler graph resolution.

- Tests in mixed fixture include multiple cross-package patterns not present in simpler fixtures

## 4. Scoping and Shadowing Patterns

### 4.1 Parameter Shadowing of Globals/Classes

Function signatures shadowing important module-level names, altering resolution.

- `add_user(self, User: "User")`
- `connect(User: Optional["User"] = None)`

### 4.2 Loop Variable Shadowing Class Names

Iterator variables shadowing class or function names within loops.

- `for User in self.users:` in `AuthManager.add_user()`

## Summary

The `ast_rewriting_mixed_collisions` fixture introduces all of the above patterns, exercising runtime-dependent behaviors that cannot be handled by static AST transformations alone. Addressing these patterns is essential to ensure correct name resolution, state management, and bundling outcomes in dynamic Python code.
