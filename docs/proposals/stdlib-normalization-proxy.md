# Stdlib Normalization via Dynamic Proxy

## Overview

This proposal replaces the complex stdlib import hoisting mechanism with a simple dynamic proxy pattern that intercepts stdlib module access at runtime. This approach eliminates hundreds of lines of complex static analysis and generation code while providing more robust and comprehensive stdlib support.

## Current Problems with Static Hoisting

1. **Complexity**: Requires detecting all stdlib usage upfront, generating imports, handling nested modules
2. **Timing Issues**: Namespace objects may reference `_cribo` before stdlib imports are generated
3. **Incomplete Coverage**: Only "safe" stdlib modules are hoisted, maintaining an allowlist
4. **Edge Cases**: Dotted module names (e.g., `collections.abc`) require special handling
5. **Order Dependencies**: Complex interdependencies between namespace creation and import generation

## Proposed Solution: Dynamic Proxy

### Core Implementation

```python
# Inserted at the top of every bundled file
import sys as _sys, importlib as _importlib


class _CriboModule:
    def __init__(self, m, p):
        self._m, self._p = m, p

    def __getattr__(self, n):
        f = f"{self._p}.{n}"
        try:
            return _CriboModule(_importlib.import_module(f), f)
        except ImportError:
            return getattr(self._m, n)

    def __getattribute__(self, n):
        return object.__getattribute__(self, n) if n in ("_m", "_p", "__getattr__") else getattr(object.__getattribute__(self, "_m"), n)


class _Cribo:
    def __getattr__(self, n):
        m = _sys.modules.get(n) or _importlib.import_module(n)
        return _CriboModule(m, n)


_cribo = _Cribo()
```

### How It Works

1. `_cribo` is a proxy object that intercepts attribute access
2. When code accesses `_cribo.json`, it dynamically imports `json` module
3. Returns a `_CriboModule` wrapper that supports nested attribute access
4. Nested modules like `_cribo.collections.abc` work through recursive wrapping
5. Uses `sys.modules` cache to avoid re-importing

## Integration Points

### 1. Import Statement Removal (KEEP)

- **Location**: `import_deduplicator.rs`
- **Current**: Removes stdlib import statements from bundled code
- **Change**: No change needed, continue removing stdlib imports

### 2. Alias Tracking (KEEP)

- **Location**: `bundler.rs` - `stdlib_module_aliases` field
- **Current**: Tracks `import json as j` → `j` maps to `json`
- **Change**: No change needed, still need to track aliases for reference transformation

### 3. Reference Transformation (KEEP)

- **Location**: `import_transformer.rs`
- **Current**: Transforms `j.dumps()` → `_cribo.json.dumps()`
- **Change**: No change needed, transformation logic remains the same

### 4. Stdlib Collection from Graph (REMOVE)

- **Location**: `bundler.rs` - `collect_stdlib_imports_from_graph()`
- **Current**: Analyzes dependency graph to find stdlib usage
- **Change**: Remove entirely, no longer needed

### 5. Stdlib Import Generation (REMOVE)

- **Location**: `bundler.rs` - `generate_stdlib_imports()`
- **Current**: Generates import statements and `_cribo` namespace
- **Change**: Replace with simple proxy code injection

### 6. Safe Module Allowlist (REMOVE)

- **Location**: `side_effects.rs` - `is_safe_stdlib_module()`
- **Current**: Maintains list of stdlib modules safe to hoist
- **Change**: Remove entirely, proxy handles all modules

### 7. Namespace Creation References (UPDATE)

- **Location**: `expressions.rs` - `simple_namespace_ctor()`
- **Current**: Always generates `_cribo.types.SimpleNamespace`
- **Change**: Keep as-is, proxy ensures `_cribo` always exists

## Implementation Steps

### Phase 1: Add Proxy Generation

1. Create new function `generate_cribo_proxy()` that returns the proxy code as AST
2. Insert proxy at the very beginning of the bundle
3. Keep all existing hoisting code temporarily for comparison

### Phase 2: Remove Hoisting Logic

1. Remove `stdlib_modules_to_import` field and related methods
2. Remove `generate_stdlib_imports()` method
3. Remove `collect_stdlib_imports_from_graph()`
4. Remove stdlib-related namespace timing fixes
5. Remove `is_safe_stdlib_module()` and allowlist

### Phase 3: Cleanup

1. Update tests to expect proxy instead of hoisted imports
2. Remove any stdlib-specific logic from namespace generation
3. Simplify bundler initialization

## Edge Cases and Considerations

### 1. Import Errors

- **Issue**: User code might catch ImportError when checking module availability
- **Solution**: Proxy preserves normal import behavior, ImportError propagates correctly

### 2. Module Attributes vs Submodules

- **Issue**: `os.path` is both an attribute and a submodule
- **Solution**: Proxy tries import first, falls back to getattr

### 3. Performance

- **Issue**: Dynamic imports have slight overhead vs pre-imported
- **Impact**: Negligible - only happens once per module due to caching
- **Benefit**: Only imports what's actually used (lazy loading)

### 4. Static Analysis

- **Issue**: Tools might not understand dynamic imports
- **Impact**: Only affects bundled output, source code unchanged
- **Note**: Bundled code is typically not analyzed by tools

### 5. Circular Imports

- **Issue**: Stdlib modules might have circular dependencies
- **Solution**: Python's import system handles this naturally

### 6. Special Module Names

- **Issue**: Some modules like `__future__` have special names
- **Solution**: These aren't accessed via `_cribo` as they require compile-time handling

### 7. Import Hooks and Metapath

- **Issue**: Custom import hooks might interfere
- **Solution**: Using standard `importlib` respects all hooks

### 8. Module Reload

- **Issue**: `importlib.reload()` behavior
- **Solution**: Would need to reload via original module reference, not proxy

## Benefits

1. **Simplicity**: ~15 lines of runtime code replaces 500+ lines of complex analysis
2. **Completeness**: Works with ALL stdlib modules, no allowlist needed
3. **Robustness**: No timing issues, namespace ordering problems eliminated
4. **Maintainability**: No need to update for new Python versions/modules
5. **Performance**: Lazy loading, only imports what's used
6. **Correctness**: Preserves exact Python import semantics

## Migration Path

Since the proxy approach is primarily code removal with a small addition, we'll implement it immediately without a feature flag:

1. Add proxy generation function that returns the proxy code as AST
2. Insert proxy at the beginning of bundles
3. Remove all hoisting-related code in the same commit
4. Update tests to expect proxy instead of hoisted imports
5. Verify all tests pass with the new implementation

## Testing Strategy

1. **Unit Tests**: Verify proxy handles all stdlib module patterns
2. **Integration Tests**: Ensure bundled code works with proxy
3. **Regression Tests**: All existing tests should pass
4. **Edge Cases**: Test error handling, special modules
5. **Performance**: Benchmark proxy overhead (expected: negligible)

## Example Transformations

### Before (Current Hoisting)

```python
# Generated hoisted imports
import json as _cribo_json
import os as _cribo_os
import collections.abc as _cribo_collections_abc

_cribo = types.SimpleNamespace(
    json=_cribo_json,
    os=_cribo_os,
    # Complex handling for dotted names...
)
setattr(_cribo, "collections.abc", _cribo_collections_abc)

# User code with removed imports and transformed references
data = _cribo.json.dumps({"test": "data"})
```

### After (Dynamic Proxy)

```python
# Simple proxy implementation
import sys as _sys, importlib as _importlib
class _CriboModule: # ... (minimal implementation)
class _Cribo: # ... (minimal implementation)  
_cribo = _Cribo()

# User code with removed imports and transformed references (unchanged)
data = _cribo.json.dumps({"test": "data"})
```

## Implementation Details

### Additional Transformations Required

During implementation, several areas requiring transformation were discovered beyond the initial scope:

#### 1. Type Annotations

**Issue**: Function parameter and return type annotations weren't being transformed.

**Example**:

```python
def query(self, sql: str) -> Optional[Dict[str, Any]]:
    pass
```

**Solution**: Extended `RecursiveImportTransformer` to traverse and transform:

- Function parameter annotations (`func_def.parameters`)
- Return type annotations (`func_def.returns`)
- Annotated assignments (`StmtAnnAssign`)

#### 2. Decorators

**Issue**: Decorator expressions weren't being transformed, causing `NameError` for stdlib decorators.

**Example**:

```python
@abstractmethod
def render(self):
    pass
```

**Solution**: Added transformation for both function and class decorators:

```rust
// Transform decorators
for decorator in &mut func_def.decorator_list {
    self.transform_expr(&mut decorator.expression);
}
```

#### 3. Importlib Removal Prevention

**Issue**: Initial implementation incorrectly removed `importlib` import since the proxy uses it.

**Solution**: The proxy generator creates separate aliased imports:

```python
import sys as _sys
import importlib as _importlib
```

These are never removed since they're generated after import deduplication.

### Edge Cases Discovered

#### 1. Empty Try/Except Blocks

**Issue**: When stdlib imports are removed from try/except blocks, empty blocks cause `IndentationError`.

**Example**:

```python
try:
    import simplejson as json
except ImportError:
    import json  # This gets removed, leaving empty except block
```

**Solution Implemented**: Added logic to insert `pass` statements in empty blocks:

```rust
// Ensure exception handler body is not empty
if eh.body.is_empty() {
    eh.body.push(crate::ast_builder::statements::pass());
}
```

#### 2. Wrapper Modules Re-exporting Stdlib Symbols

**Issue**: Wrapper modules (modules with side effects that become init functions) that re-export stdlib symbols don't properly populate their namespace.

**Example**:

```python
# compat.py - becomes wrapper due to print side effect
print("Loading compat module...")
from collections.abc import MutableMapping, Mapping

__all__ = ["MutableMapping", "Mapping"]
```

**Solution Implemented**: Track stdlib imports in wrapper modules and add them to the module namespace:

```python
def _cribo_init_compat():
    _cribo_module = _cribo.types.SimpleNamespace()
    _cribo_module.__name__ = "compat"
    print("Loading compat module...")
    # Add re-exported stdlib symbols
    _cribo_module.MutableMapping = _cribo.collections.abc.MutableMapping
    _cribo_module.Mapping = _cribo.collections.abc.Mapping
    return _cribo_module
```

Implementation:

1. Track stdlib imports during AST traversal
2. Check if symbols should be re-exported (based on `__all__` or public naming)
3. Generate proxy-based attribute assignments before returning the module

#### 3. Stdlib Local Bindings in Wrapper Modules

**Issue**: ANY stdlib import that creates a local binding becomes a module export. When we remove these imports, we lose the exports.

**Key Insight**: The problem isn't about imports - it's about EXPORTS. In Python, any import that creates a local binding becomes part of the module's exports:

- `from json import JSONDecodeError` → creates local binding `JSONDecodeError`
- `import json as j` → creates local binding `j`
- `import json` → creates local binding `json`

**Example**:

```python
# In wrapper module
import sys
import json as j
from typing import Optional

# All three create local bindings that are module exports:
# - sys
# - j
# - Optional
```

**Current Behavior**: When stdlib imports are removed, all these local bindings (exports) disappear.

**Solution**: Replace ALL stdlib imports with proxy-based assignments that preserve local bindings:

```python
# Transform regular import:
import sys

# To:
sys = _cribo.sys
_cribo_module.sys = sys

# Transform aliased import:
import json as j

# To:
j = _cribo.json
_cribo_module.j = j

# Transform from import:
from typing import Optional

# To:
Optional = _cribo.typing.Optional
_cribo_module.Optional = Optional
```

For conditional contexts, the same pattern applies:

```python
try:
    import simplejson as json

    _cribo_module.json = json
except ImportError:
    # Replace stdlib import with proxy assignment
    json = _cribo.json
    _cribo_module.json = json
```

**Implementation Approach**:

1. In wrapper modules, for ANY stdlib import statement
2. Instead of removing it, replace with:
   - Local variable assignment: `binding_name = _cribo.path.to.symbol`
   - Module namespace update: `_cribo_module.binding_name = binding_name`
3. The binding_name is:
   - For `import module`: use `module`
   - For `import module as alias`: use `alias`
   - For `from module import symbol`: use `symbol`
   - For `from module import symbol as alias`: use `alias`

**Why This Works**:

- Preserves all local bindings for use within the module
- Maintains all module exports for external access
- The proxy provides the actual stdlib functionality
- Works uniformly in any context (top-level, conditional, try/except)

## Conclusion

The dynamic proxy approach dramatically simplifies stdlib normalization while providing more complete and robust functionality. It eliminates complex static analysis, timing issues, and maintenance burden while preserving all existing bundler behavior from the user's perspective.

The implementation revealed additional transformation requirements beyond the initial design, but these were straightforward to address by extending the existing import transformer. The only remaining edge case involves wrapper modules that re-export stdlib symbols, which has a clear solution path.
