# Import Handling and AST Rewriting System Design (Revised)

## Executive Summary

This revised document incorporates deep architectural analysis and addresses key design challenges for implementing import handling and AST rewriting in Cribo's bundling system. The design now features a multi-pass analysis phase, declarative execution structure, and robust handling of edge cases.

## Key Architectural Changes

### 1. Multi-Pass Analysis Phase

The analysis phase is now structured as three sequential passes to resolve the symbol resolution timing paradox:

```
Pass 1: Potential Export Analysis → PotentialExportsMap
Pass 2: Dependency Graph & Tree-Shaking → CriboGraph + live_items  
Pass 3: Final Export Resolution & Plan Generation → BundlePlan
```

### 2. Declarative Execution Structure

Replace the imperative `ExecutionStep` enum with a declarative `FinalBundleLayout`:

```rust
pub struct FinalBundleLayout {
    /// `from __future__ import ...` statements. Must be first.
    pub future_imports: Vec<String>,

    /// Hoisted stdlib and third-party imports.
    pub hoisted_imports: Vec<HoistedImport>,

    /// Namespace object creations (`moduleA = SimpleNamespace()`)
    pub namespace_creations: Vec<String>,

    /// The main body of code, topologically sorted.
    pub inlined_code: Vec<(ModuleId, ItemId)>,

    /// Namespace population steps (`moduleA.foo = moduleA_foo`)
    pub namespace_populations: Vec<NamespacePopulationStep>,
}
```

### 3. Action-Oriented Import Classification

Replace `ImportCategory` with `ImportAction` that describes the semantic operation:

```rust
pub enum ImportAction {
    /// Hoist the import statement verbatim (e.g., `from __future__ import annotations`)
    HoistVerbatim,

    /// Hoist a standard/external import (e.g., `import os`)
    HoistExternal,

    /// A first-party `import my_module`. Requires namespace creation and population.
    CreateNamespace { module_id: ModuleId },

    /// A first-party `from my_module import my_func`. Requires symbol inlining.
    InlineSymbol {
        source_module: ModuleId,
        source_symbol: GlobalBindingId,
    },

    /// Function-scoped imports that should be left untouched
    LeaveInPlace,

    /// An import that could not be resolved or is unsupported
    Unsupported { reason: String },
}
```

## Detailed Design

### Pass 1: Potential Export Analysis

Before tree-shaking, catalog all possible exports from every module:

```rust
pub struct PotentialModuleExports {
    /// Maps symbol name to its unique ID. Contains ALL top-level bindings.
    pub symbols: FxHashMap<String, GlobalBindingId>,

    /// If `__all__` is defined, this holds the list of exported names.
    /// If `None`, default export rules apply (all symbols not starting with '_').
    pub all_declaration: Option<Vec<String>>,
}

pub type PotentialExportsMap = FxHashMap<ModuleId, PotentialModuleExports>;
```

This pass walks the AST of each module and:

1. Identifies all top-level function, class, and variable definitions
2. Detects `__all__` declarations for explicit export control
3. Creates a complete catalog of potential exports

### Pass 2: Dependency Graph Construction & Tree-Shaking

This pass remains largely unchanged but now has access to `PotentialExportsMap`:

- When resolving `from module import symbol`, verify the symbol exists
- When resolving `from module import *`, use `__all__` if present
- Build accurate dependency edges for precise tree-shaking

### Pass 3: Final Export Resolution & Plan Generation

With tree-shaking results and potential exports, we can now:

1. Determine actual exports (symbols that are both potential exports AND in live_items)
2. Build namespace population instructions with exact symbol lists
3. Generate the complete `FinalBundleLayout`

### Two-Stage Namespace Initialization

To handle circular imports correctly:

```python
# Stage 1: Create all namespace objects (hoisted to top)
moduleA = SimpleNamespace()
moduleB = SimpleNamespace()

# Stage 2: Inline all module code
# ... all transformed module code here ...

# Stage 3: Populate namespaces (at the very end)
moduleA.func_a = moduleA_func_a
moduleA.val_a = moduleA_val_a
moduleB.func_b = moduleB_func_b
moduleB.val_b = moduleB_val_b
```

### Import Transformation Examples

#### Direct Import

```python
# Original
import mymodule

# Transformed (Stage 1)
mymodule = SimpleNamespace()

# Transformed (Stage 3)
mymodule.func = mymodule_func
mymodule.Class = mymodule_Class
```

#### From Import

```python
# Original
from mymodule import func, Class

# Transformed
# (import statement removed)
# All references to 'func' → 'mymodule_func'
# All references to 'Class' → 'mymodule_Class'
```

#### Aliased Import

```python
# Original
from mymodule import func as f

# Transformed
# (import statement removed)
# All references to 'f' → 'mymodule_func'
```

## Edge Case Handling

### 1. Module-Level Side Effects

Side effects are preserved through topological sorting of the module dependency graph. The `inlined_code` list in `FinalBundleLayout` maintains execution order.

### 2. Function-Scoped Imports

Any import not at module level is ignored by transformation and left in place:

```python
def lazy_import():
    from expensive_module import func  # Left untouched
    return func()
```

### 3. Relative Imports

Resolved to absolute imports during analysis:

- `from . import utils` → Resolved to absolute ModuleId
- Original import statement removed
- Dependencies tracked using resolved ModuleId

### 4. `__name__` Handling

- Entry point: `if __name__ == "__main__"` treated as `True`
- Other modules: Treated as `False`, body is dead code
- Namespace population adds: `moduleA.__name__ = "mypackage.module_a"`

### 5. `TYPE_CHECKING` Handling

Always treated as `False` at runtime:

```python
if TYPE_CHECKING:  # Body never included in bundle
    from typing import Protocol
else:  # This branch is included
    Protocol = object
```

## Implementation Plan (Revised)

### Phase 1: Multi-Pass Analysis Foundation (2-3 days)

1. Implement `PotentialExportsMap` generation
2. Add `__all__` detection and handling
3. Integrate with existing dependency graph construction

### Phase 2: Declarative Execution Structure (2-3 days)

1. Design and implement `FinalBundleLayout` struct
2. Refactor `BundlePlan` to use declarative structure
3. Update plan builder to populate all sections
4. Migrate from `ExecutionStep` to declarative approach

### Phase 3: Import Action System (2-3 days)

1. Implement `ImportAction` enum and classification logic
2. Handle all import transformation cases
3. Integrate with symbol renaming system
4. Test with complex import scenarios

### Phase 4: Two-Stage Namespace System (1-2 days)

1. Implement namespace creation generation
2. Implement namespace population generation
3. Ensure correct ordering in `FinalBundleLayout`
4. Test with circular dependencies

### Phase 5: Edge Case Handling (2-3 days)

1. Implement `TYPE_CHECKING` and `__name__` handling
2. Add function-scoped import detection
3. Implement relative import resolution
4. Comprehensive testing of all edge cases

## Error Handling Strategy

Implement a diagnostics-based approach:

```rust
pub struct AnalysisOutput {
    pub bundle_plan: Option<BundlePlan>, // None if fatal errors
    pub diagnostics: Vec<Diagnostic>,
}

pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub location: Option<SourceLocation>,
    pub notes: Vec<String>,
}

pub enum DiagnosticSeverity {
    Error,   // Prevents bundling
    Warning, // Bundling continues but may have issues
    Info,    // Informational messages
}
```

## Success Criteria

1. **Correctness**: Bundled code behavior matches original
2. **Import Resolution**: All imports correctly transformed or preserved
3. **Circular Handling**: Two-stage initialization handles all circular cases
4. **Side Effect Order**: Module execution order preserved
5. **Edge Case Support**: All documented edge cases handled correctly
6. **Clear Diagnostics**: Actionable error messages for unsupported cases

## Risk Mitigation

1. **AST Corruption**: Validate all transformations with round-trip tests
2. **Semantic Changes**: Comprehensive test suite with real-world code
3. **Performance**: Profile SimpleNamespace overhead, optimize if needed
4. **Complexity**: Incremental implementation with thorough testing

## Future Extensions

1. **Star Import Support**: Full `from module import *` handling
2. **Dynamic Imports**: Support for `importlib.import_module()`
3. **Performance Optimizations**: Direct inlining for simple constant exports
4. **Package Support**: Enhanced `__init__.py` handling
