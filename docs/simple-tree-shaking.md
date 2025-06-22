# Simple Tree-Shaking for Inlined Modules

## Executive Summary

This document outlines the design and implementation plan for adding simple module-level tree-shaking to Cribo. The feature will eliminate unused symbols from inlined modules that have no side effects, reducing bundle size and improving code clarity for LLM agents.

## Key Insight: Reusing Existing Infrastructure

Cribo already implements comprehensive semantic analysis that tracks "eventually referenced" variables - exactly what we need for tree-shaking! The existing system:

- **Tracks all variable references** via `read_vars` and `eventual_read_vars`
- **Distinguishes immediate vs. deferred references** (crucial for correct Python semantics)
- **Handles circular dependencies** gracefully
- **Analyzes function bodies** with `collect_vars_in_body()`

We can extend this infrastructure rather than building from scratch, significantly reducing implementation complexity.

## Motivation

Currently, Cribo inlines entire modules even when only a subset of symbols are actually used. As demonstrated in the `simple_treeshaking_inlining` test fixture, the `speaking.py` module defines multiple unused symbols:

- **Used**: `ALICE_NAME`, `create_ms`, `say`, `Person`, `PersonTitle`, `Phrase`
- **Unused**: `Sex`, `Pet`, `create_mr`, `BOB_NAME`, `scream`

By implementing tree-shaking, we can exclude the unused symbols from the final bundle, resulting in:

1. Smaller bundle sizes
2. Cleaner code for LLM comprehension
3. Reduced runtime overhead from unnecessary definitions

## Design Goals

1. **Correctness**: Only remove symbols that are provably unused
2. **Performance**: Minimal impact on bundling time
3. **Simplicity**: Focus on module-level dead code elimination without complex flow analysis
4. **Determinism**: Consistent output given the same input
5. **Safety**: Only apply to modules without side effects

## Technical Approach

### Leveraging Existing Infrastructure

Cribo already implements sophisticated semantic analysis with "eventually referenced" variable tracking in `graph_builder.rs`. We can extend this existing system for tree-shaking:

#### Existing Components to Reuse:

1. **`ItemData` structure**: Already tracks `read_vars`, `write_vars`, `eventual_read_vars`, and `eventual_write_vars`
2. **`collect_vars_in_body()`**: Analyzes function/class bodies for variable references
3. **Two-phase tracking**: Distinguishes immediate vs. deferred references
4. **Dependency graph**: Already builds comprehensive module dependencies

### Phase 1: Extend Symbol Usage Analysis

#### 1.1 Enhance Existing Symbol Collection

Extend the current `ItemData` to track symbol definitions:

- Add `defined_symbols: IndexSet<String>` to track all top-level definitions
- Leverage existing AST visitors in `semantic_analysis.rs`
- Reuse `NameBinding` infrastructure for accurate scoping

#### 1.2 Augment Usage Tracking

Build upon existing variable reference tracking:

- Extend `read_vars`/`eventual_read_vars` to track cross-module references
- Add symbol-level granularity (currently tracks module-level imports)
- Preserve existing distinction between immediate and eventual references

#### 1.3 Tree-Shaking Algorithm

Adapt the existing dependency resolution:

1. Start with entry module's `read_vars` and `eventual_read_vars`
2. For each referenced symbol, transitively include its dependencies
3. Leverage existing circular dependency handling from `DependencyGraph`

### Phase 2: AST Transformation

#### 2.1 Safe Removal Criteria

Only remove top-level definitions that:

- Are not marked as used
- Have no side effects in their definition
- Are not exported via `__all__`
- Are not accessed dynamically (e.g., via `getattr`)

#### 2.2 AST Visitor Implementation

Create a specialized AST transformer that:

- Preserves module structure and ordering
- Removes unused function and class definitions
- Removes unused variable assignments
- Maintains proper indentation and formatting

### Phase 3: Integration with Bundler

#### 3.1 Configuration

Add opt-in tree-shaking via CLI flag:

```bash
cribo --entry main.py --output bundle.py --tree-shake
```

#### 3.2 Module Classification

Extend existing module analyzer to detect side-effect-free modules:

- No top-level function calls (except decorators)
- No I/O operations at module level
- No mutations of external state

#### 3.3 Pipeline Integration

Modify the bundling pipeline:

1. **Discovery**: Find all modules (existing)
2. **Analysis**: Build dependency graph (existing)
3. **Tree-shaking**: Analyze usage and mark symbols (new)
4. **Generation**: Generate code with unused symbols removed (modified)

## Implementation Plan

### Step 1: Core Infrastructure (Week 1)

- [ ] Extend `ItemData` in `semantic_analysis.rs` with symbol tracking fields
- [ ] Enhance `graph_builder.rs` to collect symbol definitions
- [ ] Create `tree_shaking.rs` module that leverages existing analysis

### Step 2: Usage Analysis (Week 1-2)

- [ ] Implement symbol usage detector
- [ ] Build cross-module reference tracker
- [ ] Create mark-and-sweep algorithm

### Step 3: AST Transformation (Week 2)

- [ ] Implement AST transformer for removing unused symbols
- [ ] Add safety checks for side effects
- [ ] Ensure deterministic output ordering

### Step 4: Integration (Week 3)

- [ ] Add CLI flag for tree-shaking
- [ ] Integrate with existing bundler pipeline
- [ ] Update code generator to use transformed ASTs

### Step 5: Testing & Refinement (Week 3-4)

- [ ] Create comprehensive test fixtures
- [ ] Benchmark performance impact
- [ ] Handle edge cases and error conditions

## Data Structures

### Extending Existing Structures

```rust
// In semantic_analysis.rs - extend existing ItemData
#[derive(Debug, Clone)]
pub struct ItemData {
    // Existing fields
    pub read_vars: IndexSet<String>,
    pub write_vars: IndexSet<String>,
    pub eventual_read_vars: IndexSet<String>,
    pub eventual_write_vars: IndexSet<String>,

    // New fields for tree-shaking
    pub defined_symbols: IndexSet<String>, // Top-level definitions in this item
    pub referenced_symbols: IndexMap<String, IndexSet<String>>, // symbol -> which symbols it uses
}

// New structure for tree-shaking analysis
pub struct TreeShaker {
    /// Reuse existing module_items from semantic analysis
    module_items: IndexMap<String, ItemData>,
    /// Track which symbols are used across module boundaries  
    cross_module_refs: IndexMap<(String, String), IndexSet<String>>,
    /// Final set of symbols to keep
    used_symbols: IndexSet<(String, String)>,
}
```

### Integration Points

```rust
// Extend existing graph_builder.rs
impl GraphBuilder {
    // Enhance existing analyze_module to also collect defined_symbols
    pub fn analyze_module(&mut self, module_path: &str) -> Result<ItemData> {
        let mut item_data = self.existing_analysis(module_path)?;

        // New: collect top-level definitions
        item_data.defined_symbols = self.collect_definitions(module_ast)?;
        item_data.referenced_symbols = self.analyze_symbol_deps(module_ast)?;

        Ok(item_data)
    }
}
```

## Algorithm: Mark and Sweep

```rust
// Leveraging existing semantic analysis
impl TreeShaker {
    pub fn mark_used_symbols(&mut self, entry_module: &str) -> Result<()> {
        let mut worklist = VecDeque::new();

        // Start with entry module's dependencies (reuse existing data)
        if let Some(entry_data) = self.module_items.get(entry_module) {
            // Add all read_vars and eventual_read_vars from entry
            for var in &entry_data.read_vars {
                if let Some((module, symbol)) = self.resolve_import(var) {
                    worklist.push_back((module, symbol));
                }
            }
        }

        // Process worklist using existing dependency info
        while let Some((module, symbol)) = worklist.pop_front() {
            if self
                .used_symbols
                .contains(&(module.clone(), symbol.clone()))
            {
                continue;
            }

            self.used_symbols.insert((module.clone(), symbol.clone()));

            // Use existing referenced_symbols data
            if let Some(item_data) = self.module_items.get(&module) {
                if let Some(deps) = item_data.referenced_symbols.get(&symbol) {
                    for dep in deps {
                        worklist.push_back((module.clone(), dep.clone()));
                    }
                }
            }
        }

        Ok(())
    }
}
```

## Edge Cases and Considerations

### 1. Dynamic Access

Cannot remove symbols accessed dynamically:

```python
# Cannot remove any PersonTitle member
title = getattr(PersonTitle, user_input)
```

### 2. Inheritance Dependencies

Must preserve entire inheritance chain:

```python
class Base: pass
class Used(Base): pass  # Both Base and Used must be kept
```

### 3. Type Annotations

Type annotations create usage dependencies:

```python
def process(p: Person) -> None:  # Person is used
    pass
```

### 4. Module Attributes

Preserve symbols accessed via module attributes:

```python
import speaking
print(speaking.ALICE_NAME)  # ALICE_NAME is used
```

### 5. Side Effects in Definitions

Skip modules with side effects:

```python
# This module has side effects - skip tree-shaking
print("Loading module...")  # Side effect!
class Logger: pass
```

## Success Metrics

1. **Correctness**: All tests pass with tree-shaking enabled
2. **Size Reduction**: 20-40% reduction in bundle size for typical projects
3. **Performance**: <10% increase in bundling time
4. **Compatibility**: Existing bundles remain unchanged when feature is disabled

## Transformation Pipeline Integration

### Current Pipeline Overview

The existing bundling pipeline in Cribo follows these steps:

1. **Module Discovery** (`orchestrator.rs`)
   - Discover all Python modules
   - Build initial dependency graph

2. **Semantic Analysis** (`graph_builder.rs`)
   - Analyze each module's AST
   - Track variable declarations and references
   - Build `CriboGraph` with `ItemData` for each module item

3. **Unused Import Detection** (`cribo_graph.rs`)
   - Analyze imported names vs. used variables
   - Mark imports as unused based on local module analysis

4. **Code Generation** (`code_generator.rs`)
   - Remove unused imports via `trim_unused_imports_from_modules`
   - Generate final bundled code

### Problem: Cascading Effects

The current unused import detection is **module-local** and runs before tree-shaking. This creates a problem:

```python
# speaking.py - BEFORE tree-shaking
from abc import ABC, abstractmethod  # Used by Pet class
from enum import Enum
from typing import TypedDict

class Pet(ABC):  # Will be removed by tree-shaking
    @abstractmethod
    def speak(self) -> str:
        pass

# Other symbols...
```

After tree-shaking removes `Pet`, the `abc` import becomes unused, but the current pipeline can't detect this because:

1. Unused import detection runs before tree-shaking
2. At detection time, `Pet` still exists and uses `ABC`

### Solution: Integrated Pipeline

#### New Pipeline Flow

```rust
// In orchestrator.rs
pub async fn bundle(&self, output_writer: &mut dyn OutputWriter) -> Result<()> {
    // Phase 1: Discovery and Analysis (unchanged)
    let modules = self.discover_modules().await?;
    let mut dep_graph = self.build_dependency_graph(&modules).await?;

    // Phase 2: Tree-Shaking Analysis (new)
    let tree_shaker = if self.config.tree_shake {
        let mut shaker = TreeShaker::from_graph(&dep_graph);
        shaker.analyze(self.config.entry_point)?;
        Some(shaker)
    } else {
        None
    };

    // Phase 3: Integrated Unused Import Detection (modified)
    if self.config.trim_unused_imports {
        self.trim_unused_imports_integrated(&mut dep_graph, &tree_shaker)?;
    }

    // Phase 4: Code Generation with Tree-Shaking (modified)
    self.generate_code(&dep_graph, &tree_shaker, output_writer)
        .await?;
}
```

#### Key Integration Points

1. **Enhanced Unused Import Detection**

```rust
// In cribo_graph.rs - new method
impl CriboGraph {
    pub fn find_unused_imports_with_tree_shaking(
        &self,
        tree_shaker: Option<&TreeShaker>,
    ) -> IndexMap<String, IndexSet<String>> {
        let mut unused_imports = IndexMap::new();

        for (module_path, module_data) in &self.modules {
            let used_symbols = if let Some(shaker) = tree_shaker {
                // Get symbols that survive tree-shaking
                shaker.get_used_symbols_for_module(module_path)
            } else {
                // Fall back to all symbols if no tree-shaking
                self.get_all_symbols_for_module(module_path)
            };

            // Check each import
            for (import_name, import_info) in &module_data.imported_names {
                if self.is_import_used_by_symbols(import_name, &used_symbols) {
                    continue;
                }

                // Import is unused after tree-shaking
                unused_imports
                    .entry(module_path.clone())
                    .or_insert_with(IndexSet::new)
                    .insert(import_name.clone());
            }
        }

        unused_imports
    }
}
```

2. **Tree-Shaker Integration**

```rust
// In tree_shaking.rs
impl TreeShaker {
    /// Returns symbols that survive tree-shaking for a module
    pub fn get_used_symbols_for_module(&self, module_path: &str) -> IndexSet<String> {
        self.used_symbols
            .iter()
            .filter(|(module, _)| module == module_path)
            .map(|(_, symbol)| symbol.clone())
            .collect()
    }

    /// Checks if an import is required by any surviving symbol
    pub fn is_import_required(
        &self,
        module_path: &str,
        import_name: &str,
        import_source: &str,
    ) -> bool {
        // Check if any surviving symbol in this module uses this import
        for symbol in self.get_used_symbols_for_module(module_path) {
            if let Some(deps) = self.get_symbol_dependencies(module_path, &symbol) {
                if deps.contains(import_name) {
                    return true;
                }
            }
        }
        false
    }
}
```

3. **AST Transformation Coordination**

```rust
// In code_generator.rs
impl<'a> CodeGenerator<'a> {
    pub fn generate_with_tree_shaking(
        &mut self,
        tree_shaker: Option<&TreeShaker>,
    ) -> Result<String> {
        // First pass: Remove unused symbols
        if let Some(shaker) = tree_shaker {
            self.apply_tree_shaking(shaker)?;
        }

        // Second pass: Remove imports that became unused
        // This happens automatically via the integrated pipeline

        // Generate final code
        self.generate_bundle()
    }
}
```

### Example: Cascading Removal

Consider the `speaking.py` example:

1. **Initial State**:
   ```python
   from abc import ABC, abstractmethod  # Used by Pet
   from enum import Enum                # Used by PersonTitle, Sex
   from typing import TypedDict         # Used by Phrase

   class Pet(ABC): ...                  # Unused
   class Sex(Enum): ...                 # Unused
   ```

2. **After Tree-Shaking**:
   - `Pet` class removed → `ABC, abstractmethod` no longer needed
   - `Sex` enum removed → `Enum` still needed (used by `PersonTitle`)

3. **Integrated Import Detection**:
   - Detects `abc` import is now unused
   - Preserves `enum` import (still used)
   - Preserves `typing` import (still used)

4. **Final Output**:
   ```python
   from enum import Enum         # Still needed
   from typing import TypedDict  # Still needed
   # abc import removed!

   class PersonTitle(Enum): ...
   class Person: ...
   # Pet and Sex removed
   ```

### Configuration and Backwards Compatibility

```rust
#[derive(Debug, Clone)]
pub struct BundlerConfig {
    // Existing options
    pub trim_unused_imports: bool,

    // New options
    pub tree_shake: bool,
    pub tree_shake_aggressive: bool, // Future: remove symbols with side effects
}
```

**Behavior Matrix**:

| tree_shake | trim_unused_imports | Behavior                            |
| ---------- | ------------------- | ----------------------------------- |
| false      | false               | No optimization (current default)   |
| false      | true                | Local unused import removal only    |
| true       | false               | Tree-shaking only (not recommended) |
| true       | true                | Full optimization with cascading    |

### Import Dependency Tracking

To properly handle cascading import removal, we need to track which symbols depend on which imports:

```rust
// Enhanced ItemData structure
#[derive(Debug, Clone)]
pub struct ItemData {
    // Existing fields...

    // New: Map symbols to their import dependencies
    pub symbol_import_deps: IndexMap<String, IndexSet<ImportDep>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportDep {
    pub module: Option<String>, // None for "import foo" style
    pub name: String,           // The imported name
    pub alias: Option<String>,  // If aliased with "as"
}
```

#### Pattern Detection Examples

1. **Base Class Dependencies**
   ```python
   from abc import ABC, abstractmethod

   class Pet(ABC):  # Pet depends on ABC import
       @abstractmethod  # Also depends on abstractmethod
       def speak(self): pass
   ```

2. **Type Annotation Dependencies**
   ```python
   from typing import TypedDict, Optional

   class Config(TypedDict):  # Config depends on TypedDict
       name: str
       value: Optional[int]  # Also depends on Optional
   ```

3. **Decorator Dependencies**
   ```python
   from functools import cache

   @cache  # process depends on cache import
   def process(x):
       return x * 2
   ```

4. **Transitive Dependencies**
   ```python
   from base import BaseClass
   from typing import List

   class MyClass(BaseClass):  # If BaseClass is removed, base import can go
       items: List[str]        # But typing stays if other symbols use it
   ```

### Testing the Integration

New test fixtures needed:

```
crates/cribo/tests/fixtures/
├── tree_shake_cascade_imports/
│   ├── main.py              # Uses subset of library
│   └── library.py           # Has classes with different imports
├── tree_shake_transitive_imports/
│   ├── main.py              # Uses A which uses B
│   ├── module_a.py          # Uses subset of B
│   └── module_b.py          # Has many imports
└── tree_shake_reexport_imports/
    ├── main.py              # Uses reexported symbols
    └── __init__.py          # Reexports with unused imports
```

## Future Enhancements

1. **Expression-level tree-shaking**: Remove unused expressions within functions
2. **Cross-module constant folding**: Inline and remove constant definitions
3. **Side-effect analysis**: More sophisticated detection of pure modules
4. **Partial class shaking**: Remove unused methods from classes

## Testing Strategy

### Unit Tests

- Symbol collection accuracy
- Usage tracking correctness
- AST transformation safety

### Integration Tests

- End-to-end bundling with tree-shaking
- Comparison with non-tree-shaken output
- Performance benchmarks

### Snapshot Tests

Extend existing snapshot framework:

```
crates/cribo/tests/fixtures/
├── simple_treeshaking_inlining/     # Existing
├── treeshaking_complex_deps/        # Complex dependencies
├── treeshaking_with_side_effects/   # Should skip shaking
├── treeshaking_dynamic_access/      # Preserve all symbols
└── treeshaking_type_annotations/    # Type usage tracking
```

## Risks and Mitigations

| Risk                              | Mitigation                               |
| --------------------------------- | ---------------------------------------- |
| Incorrectly removing used symbols | Conservative analysis, extensive testing |
| Performance regression            | Benchmark suite, opt-in feature          |
| Breaking existing bundles         | Feature flag, backward compatibility     |
| Complex edge cases                | Start simple, iterate based on feedback  |

## Conclusion

Simple tree-shaking for Cribo will provide significant value by reducing bundle sizes and improving code clarity. The implementation focuses on correctness and simplicity, with room for future enhancements as the feature matures.
