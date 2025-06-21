# Recursive Bundler Planning Document

## Overview

This document outlines a fundamentally different approach to Python source bundling: a **recursive bundler** that processes modules bottom-up rather than top-down. Instead of analyzing all modules together and resolving conflicts globally, we bundle each module individually first, then combine already-bundled modules.

## Current Approach (For Reference)

The current bundler:

1. Discovers all modules starting from entry point
2. Builds a complete dependency graph
3. Detects circular dependencies
4. Performs topological sort
5. Transforms all modules together, resolving naming conflicts globally
6. Outputs a single bundled file

## New Recursive Approach

### Core Concept

1. **Start with leaf modules** (modules with no first-party dependencies)
2. **Bundle each module independently** into a self-contained unit
3. **Replace imports** in dependent modules with references to bundled versions
4. **Recursively bundle** up the dependency tree
5. **Entry point** becomes a thin wrapper importing bundled modules

### Key Innovation

Each bundled module becomes a **complete, standalone unit** that:

- Has no first-party imports (they've been inlined)
- Can be imported like a regular module
- Contains all its dependencies already bundled within it

### Algorithm

```python
def bundle_recursively(module_name, already_bundled=None):
    if already_bundled is None:
        already_bundled = {}
    
    if module_name in already_bundled:
        return already_bundled[module_name]
    
    # Get module's first-party dependencies
    deps = get_first_party_dependencies(module_name)
    
    # Recursively bundle all dependencies first
    bundled_deps = {}
    for dep in deps:
        bundled_deps[dep] = bundle_recursively(dep, already_bundled)
    
    # Now bundle this module with its dependencies already bundled
    bundled_module = bundle_single_module(module_name, bundled_deps)
    
    already_bundled[module_name] = bundled_module
    return bundled_module
```

### Module Bundling Strategy

When bundling a single module:

1. **Parse the module AST**
2. **For each first-party import:**
   - Replace with the content of the already-bundled dependency
   - Inline the bundled code directly
3. **Transform identifiers** to avoid conflicts
4. **Wrap in a module-like structure** (e.g., using a class or namespace)

### Example Transformation

**Original `utils/calc.py`:**

```python
def add(a, b):
    return a + b
```

**Original `utils/helpers.py`:**

```python
from .calc import add

def add_many(*args):
    result = 0
    for arg in args:
        result = add(result, arg)
    return result
```

**Bundled `utils/calc.py`:**

```python
# No dependencies, bundled as-is but wrapped
class __bundled_utils_calc:
    @staticmethod
    def add(a, b):
        return a + b

# Export interface
add = __bundled_utils_calc.add
```

**Bundled `utils/helpers.py`:**

```python
# calc dependency already bundled and inlined
class __bundled_utils_calc:
    @staticmethod
    def add(a, b):
        return a + b

class __bundled_utils_helpers:
    @staticmethod
    def add_many(*args):
        result = 0
        for arg in args:
            result = __bundled_utils_calc.add(result, arg)
        return result

# Export interface
add_many = __bundled_utils_helpers.add_many
```

## Advantages

1. **Simpler mental model**: Each module is bundled independently
2. **No global conflict resolution**: Conflicts are resolved locally within each module
3. **Natural handling of circular dependencies**: They become impossible since dependencies are bundled before dependents
4. **Incremental bundling**: Can cache bundled modules and reuse them
5. **Clear module boundaries**: Each bundled module is self-contained

## Challenges and Solutions

### Challenge 1: Circular Dependencies

**Problem**: Module A imports from B, and B imports from A.

**Solution**:

- Detect circular dependency groups (SCCs in the graph)
- Bundle the entire SCC as a single unit
- Within the SCC, use delayed binding or function-level imports

### Challenge 2: Module State and Side Effects

**Problem**: Modules with side effects need to execute only once.

**Solution**:

- Wrap bundled modules in a lazy initialization pattern
- Use a registry to ensure single execution
- Similar to current `sys.modules` approach but at bundle level

### Challenge 3: Dynamic Imports and Introspection

**Problem**: Code that uses `__name__`, `__file__`, or dynamic imports.

**Solution**:

- Preserve module metadata in bundled form
- Provide compatibility shims for introspection
- Transform dynamic imports to use bundled registry

## Implementation Plan

### Phase 1: Core Infrastructure

1. **Create `RecursiveBundler` struct**
   - Dependency graph traversal
   - Module bundling cache
   - Bundled module registry

2. **Implement single module bundler**
   - AST transformation for import replacement
   - Identifier conflict resolution
   - Module wrapper generation

### Phase 2: Dependency Resolution

1. **Implement dependency analyzer**
   - Extract first-party imports
   - Build dependency graph
   - Detect circular dependencies (SCCs)

2. **Implement recursive bundling algorithm**
   - Bottom-up traversal
   - Caching mechanism
   - SCC handling

### Phase 3: Code Generation

1. **Design bundled module format**
   - Wrapper structure (class vs namespace)
   - Export mechanism
   - Metadata preservation

2. **Implement AST transformers**
   - Import replacement
   - Identifier renaming
   - Module reference updates

### Phase 4: Integration

1. **Create compatibility layer**
   - Bridge with existing bundler interface
   - Configuration support
   - Output format compatibility

2. **Testing and refinement**
   - Complex dependency scenarios
   - Performance optimization
   - Edge case handling

## Code Structure

```
crates/cribo/src/
├── recursive_bundler/
│   ├── mod.rs              # Main recursive bundler
│   ├── module_bundler.rs   # Single module bundling logic
│   ├── dep_analyzer.rs     # Dependency analysis
│   ├── scc_handler.rs      # Circular dependency handling
│   └── code_gen.rs         # Bundled module generation
```

## Success Criteria

1. **Functional correctness**: Bundled code behaves identically to original
2. **Simplicity**: Easier to understand and debug than current approach
3. **Performance**: Comparable or better bundling time
4. **Maintainability**: Clear separation of concerns, modular design
5. **Compatibility**: Can handle all current test cases

## Next Steps

1. Review and refine this plan
2. Create basic prototype of single module bundler
3. Implement dependency analysis
4. Build recursive bundling algorithm
5. Test with increasingly complex scenarios
