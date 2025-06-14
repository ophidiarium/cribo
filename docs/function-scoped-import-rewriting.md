# Function-Scoped Import Rewriting: Problem Analysis and Proposed Solution

## Executive Summary

This document describes a proposed enhancement to the cribo Python bundler to automatically resolve certain circular dependencies by rewriting module-level imports as function-scoped imports. This transformation can break circular dependency cycles while preserving program semantics.

## Problem Statement

### Current Situation

The cribo bundler currently detects and classifies circular dependencies but cannot resolve them automatically. When circular dependencies are detected, the bundler fails with an error message suggesting manual intervention:

```
Error: Circular dependencies detected in the module graph:

Cycle 1: module_a → module_b → module_c → module_a
  Type: FunctionLevel
  Suggestion: Move imports inside functions to enable lazy loading
```

### The Challenge

Python's import system evaluates modules at import time, which means circular imports can cause `ImportError` when modules try to import each other during initialization:

```python
# module_a.py
from module_b import function_b

def function_a():
    return function_b() + 1

# module_b.py
from module_a import function_a  # ImportError: cannot import name 'function_a'

def function_b():
    return function_a() + 2
```

### Missed Opportunity

Many circular dependencies can be resolved by moving imports from module-level to function-level. This defers the import until the function is called, breaking the initialization-time cycle:

```python
# module_a.py - FIXED
def function_a():
    from module_b import function_b  # Import moved inside function
    return function_b() + 1

# module_b.py - FIXED
def function_b():
    from module_a import function_a  # Import moved inside function
    return function_a() + 2
```

## Current Implementation Analysis

### Circular Dependency Detection

The bundler successfully:

1. Detects circular dependencies using Tarjan's algorithm
2. Classifies them into types (FunctionLevel, ClassLevel, ImportTime, ModuleConstants)
3. Tracks import usage via `read_vars` (module-level) vs `eventual_read_vars` (function-level)

### Classification Logic

```rust
// In cribo_graph.rs
fn classify_cycle_type(
    &self,
    module_names: &[String],
    import_chain: &[ImportEdge],
) -> CircularDependencyType {
    // ...
    if analysis_result.imports_used_in_functions_only {
        CircularDependencyType::FunctionLevel // Can be resolved!
    }
    // ...
}
```

### Missing Piece: Automatic Resolution

While the bundler identifies resolvable cycles, it doesn't actually resolve them. Users must manually edit their code, which defeats the purpose of automated bundling.

## Proposed Solution

### High-Level Approach

1. **Identify Resolvable Imports**: During dependency analysis, track which imports can be safely moved to function scope
2. **AST Transformation**: Rewrite the AST to move qualifying imports inside the functions that use them
3. **Deduplication**: Ensure imports aren't duplicated if used in multiple functions
4. **Preserve Semantics**: Maintain program behavior and import side effects

### Detailed Design

#### Phase 1: Enhanced Import Tracking

Extend the current graph analysis to track:

```rust
struct ImportUsage {
    /// The import statement
    import_stmt: ImportStatement,
    /// Functions that use this import
    used_in_functions: Vec<FunctionId>,
    /// Whether the import is used at module level
    used_at_module_level: bool,
    /// Whether the import has side effects
    has_side_effects: bool,
    /// Whether this import participates in a circular dependency
    in_circular_dependency: bool,
}
```

#### Phase 2: Import Movement Eligibility

An import can be moved to function scope if:

1. It's only used within function bodies (`used_at_module_level == false`)
2. It participates in a circular dependency (`in_circular_dependency == true`)
3. It doesn't have side effects (`has_side_effects == false`)
4. It's not used for class inheritance or decorators

#### Phase 3: AST Transformation

Implement a new AST transformer:

```rust
pub struct ImportRewriter {
    /// Imports to move and their target functions
    imports_to_move: HashMap<ImportId, Vec<FunctionId>>,
    /// Import deduplication strategy
    dedup_strategy: ImportDeduplicationStrategy,
}

impl ImportRewriter {
    pub fn rewrite_module(&mut self, module: &mut ast::Module) -> Result<()> {
        // 1. Remove module-level imports that will be moved
        self.remove_module_imports(module)?;

        // 2. Add imports to function bodies
        self.add_function_imports(module)?;

        // 3. Update __all__ if necessary
        self.update_module_exports(module)?;

        Ok(())
    }
}
```

#### Phase 4: Import Placement Strategy

Multiple strategies for where to place the import within a function:

```python
# Strategy 1: At function start (simplest)
def function_a():
    from module_b import function_b
    return function_b() + 1

# Strategy 2: Just before first use (optimal)
def function_a():
    x = compute_something()
    from module_b import function_b  # Right before use
    return function_b() + x

# Strategy 3: Conditional import (when in conditional blocks)
def function_a(use_b=True):
    if use_b:
        from module_b import function_b
        return function_b() + 1
    return 0
```

### Implementation Plan

#### Step 1: Extend Graph Analysis

```rust
// In graph_builder.rs
fn track_import_usage(&mut self, import: &Import, usage_context: &UsageContext) {
    match usage_context {
        UsageContext::FunctionBody(func_id) => {
            self.import_usage
                .entry(import.id)
                .or_default()
                .used_in_functions
                .push(func_id);
        }
        UsageContext::ModuleLevel => {
            self.import_usage
                .entry(import.id)
                .or_default()
                .used_at_module_level = true;
        } // ... other contexts
    }
}
```

#### Step 2: Identify Movable Imports

```rust
// In orchestrator.rs
fn identify_movable_imports(&self, cycles: &[CircularDependencyGroup]) -> Vec<MovableImport> {
    let mut movable = Vec::new();

    for cycle in cycles {
        if matches!(cycle.cycle_type, CircularDependencyType::FunctionLevel) {
            // Find imports that can be moved to break the cycle
            for module in &cycle.modules {
                let imports = self.get_module_imports(module);
                for import in imports {
                    if self.can_move_import(&import) {
                        movable.push(MovableImport {
                            import: import.clone(),
                            target_functions: self.get_import_usage_functions(&import),
                            source_module: module.clone(),
                        });
                    }
                }
            }
        }
    }

    movable
}
```

#### Step 3: Transform AST

```rust
// In code_generator.rs
fn apply_import_rewrites(&mut self, module_ast: &mut ast::Module, rewrites: &[ImportRewrite]) {
    for rewrite in rewrites {
        // Remove from module level
        self.remove_import_statement(module_ast, &rewrite.import);

        // Add to each function that uses it
        for func_id in &rewrite.target_functions {
            self.add_import_to_function(module_ast, func_id, &rewrite.import);
        }
    }
}
```

### Edge Cases and Considerations

#### 1. Import Side Effects

Some imports have side effects that must execute at module load time:

```python
# config.py
import os
os.environ['CONFIG_LOADED'] = 'true'  # Side effect!

# This cannot be moved to function scope
```

#### 2. Performance Implications

Function-scoped imports have a small performance cost on repeated calls:

```python
def frequently_called():
    from heavy_module import func  # Re-evaluated each call
    return func()
```

Solution: Add a configuration option to control optimization vs compatibility.

#### 3. Type Annotations

Imports used in type annotations need special handling:

```python
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from module_b import ClassB  # Only for type checking

def function_a(param: 'ClassB'):  # String annotation
    from module_b import ClassB  # Runtime import
    return isinstance(param, ClassB)
```

#### 4. Import Variations

Handle all import forms:

```python
# Simple import
import module_b
# From import
from module_b import func
# Aliased import
from module_b import func as b_func
# Star import (generally avoid moving these)
from module_b import *
```

### Testing Strategy

#### Unit Tests

1. Test import movement eligibility detection
2. Test AST transformation correctness
3. Test various import forms and edge cases

#### Integration Tests

1. Create fixtures with resolvable circular dependencies
2. Verify bundled output executes correctly
3. Test performance characteristics

#### Fixture Example

```python
# test/fixtures/auto_resolve_cycles/module_a.py
from module_b import helper_b

def func_a():
    return helper_b() + 1

# test/fixtures/auto_resolve_cycles/module_b.py
from module_a import func_a

def helper_b():
    return func_a() * 2

# Expected bundled output should have imports moved inside functions
```

### Configuration Options

Add new configuration options to `cribo.toml`:

```toml
[bundler]
# Enable automatic import rewriting
auto_resolve_circular_imports = true

# Strategy for placing imports in functions
import_placement_strategy = "function_start" # or "before_use"

# Optimize for performance vs compatibility
prefer_performance = false # Keep imports at module level when possible

# List of modules where imports should not be moved (side effects)
preserve_module_imports = ["config", "settings"]
```

### Success Metrics

1. **Functionality**: Successfully resolve 90%+ of FunctionLevel circular dependencies
2. **Compatibility**: Bundled code executes identically to original
3. **Performance**: Less than 5% slowdown for function-scoped imports
4. **Usability**: Zero configuration required for common cases

## Implementation Timeline

1. **Phase 1** (1-2 days): Extend graph analysis to track detailed import usage
2. **Phase 2** (2-3 days): Implement import movement eligibility detection
3. **Phase 3** (3-4 days): Implement AST transformation for import rewriting
4. **Phase 4** (2-3 days): Add configuration options and optimization strategies
5. **Phase 5** (2-3 days): Comprehensive testing and documentation

## Conclusion

Function-scoped import rewriting would be a powerful addition to cribo, automatically resolving a common class of circular dependencies without user intervention. The implementation leverages existing dependency analysis infrastructure while adding targeted AST transformations to produce working, bundled Python code.

This feature would make cribo more robust and user-friendly, handling real-world Python codebases that often contain circular dependencies that are technically resolvable but tedious to fix manually.
