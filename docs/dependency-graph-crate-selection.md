# Dependency Graph Crate Selection for Cribo

## Executive Summary

After analyzing the dependency graph implementations in Rolldown, Rspack, and Turbopack, I recommend adopting **Turbopack's `DepGraph`** from their analyzer module. This document explains why it's the best fit for Python bundling and how to integrate it effectively.

## The Recommendation: Turbopack's DepGraph

### Why Turbopack's DepGraph?

Turbopack's dependency graph implementation offers unique advantages for Python bundling that the alternatives lack:

1. **Statement-level granularity** instead of module-level
2. **Variable state tracking** for dynamic imports
3. **Side effect ordering** preservation
4. **Strong/Weak dependency model** perfect for Python's conditional imports
5. **Clean abstraction** that maps well to Python's semantics

## Detailed Analysis

### Current Cribo Implementation

```rust
pub struct DependencyGraph {
    graph: DiGraph<String, ()>,
    node_indices: IndexMap<String, NodeIndex>,
}
```

**Limitations**:

- Module-level only (too coarse for Python)
- No tracking of which symbols are actually used
- No side effect ordering
- String-based (performance issues)

### Turbopack's DepGraph Architecture

```rust
pub struct DepGraph {
    // Fine-grained items (functions, classes, statements)
    items: FxHashMap<ItemId, Item>,

    // Dependencies between items
    deps: FxHashMap<ItemId, Vec<Dep>>,

    // Execution order for side effects
    side_effects: Vec<ItemId>,

    // Variable state tracking
    var_states: FxHashMap<Id, VarState>,
}

pub enum Dep {
    Strong(ItemId), // Always needed
    Weak(ItemId),   // Only if target is included
}

pub struct VarState {
    declarator: Option<ItemId>,
    last_writes: Vec<ItemId>,
    last_reads: Vec<ItemId>,
    last_op: Option<VarOp>,
}
```

### Why This Fits Python Perfectly

#### 1. Fine-Grained Import Tracking

Python imports can be selective:

```python
from module import specific_function, SpecificClass
```

Turbopack's item-level tracking naturally handles this, while module-level graphs force us to include the entire module.

#### 2. Side Effect Preservation

Python modules execute on import:

```python
# module.py
print("Module loading...")  # Side effect
config = load_config()      # Side effect

def pure_function():        # No side effect
    return 42
```

The `side_effects: Vec<ItemId>` preserves execution order, critical for Python.

#### 3. Conditional Import Support

The Strong/Weak dependency model maps perfectly:

```python
# Strong dependency - always needed
from core import essential_function

# Weak dependency - only if TYPE_CHECKING is used
if TYPE_CHECKING:
    from typing import Protocol
```

#### 4. Variable State Tracking

Python allows import reassignment:

```python
import os
os = MockOS()  # Tracked by VarState
```

This is impossible to handle correctly without variable state tracking.

## Comparison with Alternatives

### Rolldown's Approach

```rust
// Module-centric with numeric indices
modules: IndexVec<ModuleIdx, Module>
```

**Pros**:

- Fast index-based lookups
- Memory efficient

**Cons**:

- Module-level only
- Would require major Cribo refactoring
- No fine-grained dependency tracking

### Rspack's ModuleGraph

```rust
pub struct ModuleGraph {
    modules: IdentifierMap<Option<BoxModule>>,
    connections: HashMap<DependencyId, Option<ModuleGraphConnection>>,
}
```

**Pros**:

- Rich connection metadata
- Good for plugin systems

**Cons**:

- Overly complex for Python's needs
- Designed for webpack compatibility
- Still module-centric

### Turbopack's DepGraph

**Pros**:

- Statement-level granularity
- Variable tracking built-in
- Side effect ordering
- Clean abstraction

**Cons**:

- May need adaptation for Python-specific features
- Slightly more memory usage than module-only approaches

## Integration Strategy

### Phase 1: Hybrid Approach

Keep Cribo's module-level graph while adding item-level analysis:

```rust
pub struct EnhancedDependencyGraph {
    // Existing module-level graph
    module_graph: DiGraph<ModuleId, ImportType>,
    module_indices: IndexMap<String, ModuleId>,

    // New: Item-level analysis per module
    module_deps: FxHashMap<ModuleId, DepGraph>,
}
```

### Phase 2: Python Adaptations

Create Python-specific wrappers:

```rust
pub struct PythonDepGraph {
    // Core Turbopack functionality
    inner: turbopack_core::DepGraph,

    // Python-specific tracking
    import_aliases: FxHashMap<String, String>,
    star_imports: Vec<ModuleId>,
    __all__: Option<Vec<String>>,
}

impl PythonDepGraph {
    /// Track Python's __all__ exports
    pub fn set_explicit_exports(&mut self, exports: Vec<String>) {
        self.__all__ = Some(exports);
    }

    /// Handle from module import *
    pub fn add_star_import(&mut self, module: ModuleId) {
        self.star_imports.push(module);
    }
}
```

### Phase 3: Enhanced Tree Shaking

Leverage the fine-grained dependencies:

```rust
pub fn tree_shake_module(
    graph: &PythonDepGraph,
    used_symbols: &HashSet<String>,
) -> TreeShakenModule {
    let mut required_items = HashSet::new();

    // Start from used symbols
    for symbol in used_symbols {
        if let Some(item_id) = graph.symbol_to_item(symbol) {
            collect_dependencies(graph, item_id, &mut required_items);
        }
    }

    // Include side effects in order
    let side_effects = graph
        .side_effects
        .iter()
        .filter(|id| required_items.contains(id))
        .collect();

    TreeShakenModule {
        items: required_items,
        side_effects,
    }
}
```

## Implementation Roadmap

### Week 1: Basic Integration

1. Add Turbopack dependency
2. Create `PythonDepGraph` wrapper
3. Implement basic item tracking

### Week 2: Python Semantics

1. Add `__all__` support
2. Handle star imports
3. Track import aliases

### Week 3: Tree Shaking

1. Implement symbol-to-item mapping
2. Add dependency collection
3. Generate tree-shaken output

### Week 4: Optimization

1. Performance tuning
2. Memory optimization
3. Integration testing

## Benefits for Cribo

### Immediate Benefits

- **Accurate dependency tracking**: Know exactly which symbols are used
- **Better tree shaking**: Remove truly unused code
- **Side effect preservation**: Maintain Python's execution semantics

### Future Benefits

- **Incremental bundling**: Only rebundle changed items
- **Advanced optimizations**: Dead code elimination, constant folding
- **Better error reporting**: Precise location of circular dependencies

## Configuration

```toml
[dependencies]
# Core dependency graph functionality
turbopack-core = { 
    git = "https://github.com/vercel/turbo", 
    package = "turbopack-core",
    features = ["analyzer"]
}

# For FxHashMap and other utilities
rustc-hash = "1.1"
```

## Migration Path

### Step 1: Add Alongside Existing

Run both graphs in parallel to verify correctness:

```rust
let old_result = self.dependency_graph.analyze();
let new_result = self.turbo_graph.analyze();
assert_eq!(old_result, new_result);
```

### Step 2: Gradual Replacement

Replace functionality piece by piece:

1. Import detection
2. Dependency resolution
3. Circular dependency detection
4. Tree shaking

### Step 3: Remove Old Implementation

Once confidence is built, remove the old graph entirely.

## Conclusion

Turbopack's DepGraph provides the right abstraction level for Python bundling. While Rolldown and Rspack have excellent module-level graphs, Python's dynamic nature and fine-grained imports require the statement-level tracking that Turbopack provides.

The integration effort is justified by:

1. More accurate dependency tracking
2. Better tree shaking capabilities
3. Proper handling of Python's execution model
4. Foundation for future optimizations

By adopting Turbopack's DepGraph, Cribo can achieve JavaScript bundler-level sophistication while respecting Python's unique semantics.
