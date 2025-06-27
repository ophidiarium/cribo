# Import Handling and AST Rewriting System Design

## Executive Summary

This document outlines the design and implementation plan for the next essential features in Cribo's bundling system: import handling, AST rewriting for symbol renaming, and hoisted import generation. These features are interdependent and should be implemented together to maintain consistency in the bundled output.

## Problem Statement

Currently, the dumb plan executor can only inline statements as-is. To create functional bundles from multi-module Python projects, we need:

1. **Import Resolution**: Transform import statements to work in a single-file context
2. **Symbol Renaming**: Apply AST transformations to resolve naming conflicts
3. **Import Hoisting**: Move certain imports to the top of the bundled file
4. **Module Merging**: Combine multiple modules while preserving semantics

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        Import & AST Processing Pipeline                   │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  BundlePlan                    Import Processor                          │
│  ┌─────────────┐              ┌────────────────┐                       │
│  │ execution_plan│ ──────────> │ Import Filter  │                       │
│  │ import_rewrites│            │ & Transformer │                       │
│  │ hoisted_imports│            └────────────────┘                       │
│  └─────────────┘                      │                                 │
│         │                             ▼                                 │
│         │                      ┌────────────────┐                       │
│         │                      │ Import Context │                       │
│         │                      │   Builder      │                       │
│         │                      └────────────────┘                       │
│         │                             │                                 │
│         ▼                             ▼                                 │
│  ┌─────────────┐              ┌────────────────┐                       │
│  │ ast_node_   │              │ AST Rewriter   │                       │
│  │ renames     │ ──────────> │ & Transformer  │                       │
│  └─────────────┘              └────────────────┘                       │
│                                       │                                 │
│                                       ▼                                 │
│                               ┌────────────────┐                       │
│                               │ Statement      │                       │
│                               │ Processor      │                       │
│                               └────────────────┘                       │
│                                       │                                 │
│                                       ▼                                 │
│                               ┌────────────────┐                       │
│                               │ Final Bundle   │                       │
│                               └────────────────┘                       │
└─────────────────────────────────────────────────────────────────────────┘
```

## Detailed Component Design

### 1. Import Processing System

#### 1.1 Import Categories

We need to handle different types of imports differently:

```rust
enum ImportCategory {
    /// Should be hoisted and preserved (e.g., __future__)
    HoistedPreserved,

    /// Should be hoisted but may be transformed (e.g., stdlib)
    HoistedTransformed,

    /// First-party imports that get inlined
    InlinedModule,

    /// Third-party imports that are preserved
    PreservedExternal,

    /// Imports that need special handling (e.g., circular deps)
    SpecialCase(ImportRewriteAction),
}
```

#### 1.2 Import Context Builder

Builds context needed for import decisions:

```rust
struct ImportContext {
    /// Maps original module names to their bundled representation
    module_name_map: FxHashMap<String, String>,

    /// Tracks which symbols are available at module level
    available_symbols: FxHashMap<ModuleId, FxHashSet<String>>,

    /// Namespace objects for direct module imports
    namespace_objects: FxHashMap<String, Vec<String>>,

    /// Import ordering dependencies
    import_order: Vec<(ModuleId, ItemId)>,
}
```

#### 1.3 Import Transformation Rules

1. **Remove First-Party Imports**:
   - `import mymodule` → (removed, symbols inlined)
   - `from mymodule import func` → (removed, `func` available directly)

2. **Preserve External Imports**:
   - `import numpy` → `import numpy` (unchanged)
   - `from pandas import DataFrame` → `from pandas import DataFrame`

3. **Transform Relative Imports**:
   - `from . import utils` → (removed, utils symbols inlined)
   - `from ..lib import helper` → (removed, helper inlined)

4. **Create Namespace Objects**:
   - `import mymodule` → `mymodule = SimpleNamespace(x=x, y=y)`
   - Preserves attribute access: `mymodule.func()`

### 2. AST Rewriting System

#### 2.1 AST Visitor Pattern

Implement a comprehensive AST visitor that can transform nodes:

```rust
trait AstTransformer {
    fn transform_stmt(&mut self, stmt: Stmt) -> Stmt;
    fn transform_expr(&mut self, expr: Expr) -> Expr;
    fn transform_name(&mut self, name: &str, range: TextRange) -> String;
}

struct SymbolRenamer<'a> {
    renames: &'a FxHashMap<(ModuleId, TextRange), String>,
    current_module: ModuleId,
}

impl<'a> AstTransformer for SymbolRenamer<'a> {
    fn transform_name(&mut self, name: &str, range: TextRange) -> String {
        self.renames
            .get(&(self.current_module, range))
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    // ... implement other transform methods
}
```

#### 2.2 Transformation Phases

1. **Phase 1: Import Removal**
   - Remove imports that will be inlined
   - Track what symbols they provided

2. **Phase 2: Symbol Renaming**
   - Apply all renames from `ast_node_renames`
   - Handle all name contexts (definitions, references, attributes)

3. **Phase 3: Namespace Creation**
   - Insert namespace objects for direct module imports
   - Ensure correct initialization order

4. **Phase 4: Import Reordering**
   - Move hoisted imports to top
   - Preserve relative order within categories

### 3. Statement Processing Enhancement

Enhance the current `execute_step` to handle imports:

```rust
fn execute_step(
    step: &ExecutionStep,
    plan: &BundlePlan,
    context: &ExecutionContext,
    import_context: &ImportContext,
) -> Result<Option<Stmt>> {
    match step {
        ExecutionStep::InlineStatement { module_id, item_id } => {
            let stmt = get_statement(&context.source_asts, *module_id, *item_id, context)?;

            // Check if this is an import that should be filtered
            if should_filter_import(&stmt, import_context) {
                return Ok(None);
            }

            // Apply AST transformations
            let mut transformer = create_transformer(plan, *module_id, import_context);
            let transformed = transformer.transform_stmt(stmt);

            Ok(Some(transformed))
        }

        ExecutionStep::HoistFutureImport { name } => {
            // Already implemented
        }

        ExecutionStep::CreateNamespace {
            module_name,
            exports,
        } => Ok(Some(create_namespace_assignment(module_name, exports))), // ... other variants
    }
}
```

### 4. Integration Points

#### 4.1 BundlePlan Extensions

Add new fields to track import handling:

```rust
pub struct BundlePlan {
    // ... existing fields ...
    /// Maps module names to their namespace exports
    pub module_namespaces: FxHashMap<String, Vec<String>>,

    /// Import filtering decisions
    pub import_filters: FxHashSet<(ModuleId, ItemId)>,

    /// Order for namespace creation
    pub namespace_init_order: Vec<String>,
}
```

#### 4.2 ExecutionStep Extensions

Add new variants for import handling:

```rust
pub enum ExecutionStep {
    // ... existing variants ...
    /// Create a namespace object for a module
    CreateNamespace {
        module_name: String,
        exports: Vec<String>,
    },

    /// Apply a specific import transformation
    TransformImport {
        module_id: ModuleId,
        item_id: ItemId,
        transformation: ImportTransformation,
    },
}
```

## Implementation Plan

### Phase 1: AST Transformation Foundation (2-3 days)

1. Implement base `AstTransformer` trait
2. Create `SymbolRenamer` implementation
3. Add comprehensive AST visiting for all node types
4. Test with simple renaming cases

### Phase 2: Import Analysis (2-3 days)

1. Implement `ImportCategory` classification
2. Build `ImportContext` from BundlePlan
3. Create import filtering logic
4. Add import transformation rules

### Phase 3: Namespace Object Generation (1-2 days)

1. Implement `CreateNamespace` execution step
2. Generate SimpleNamespace assignments
3. Handle nested module access patterns
4. Ensure correct initialization order

### Phase 4: Integration & Testing (2-3 days)

1. Integrate all components into plan executor
2. Update BundlePlan building to include import decisions
3. Comprehensive testing with real fixtures
4. Handle edge cases and error conditions

## Technical Considerations

### 1. AST Transformation Challenges

- **Preserving Source Locations**: Transformed AST nodes need valid TextRange
- **Nested Scoping**: Symbol renames must respect Python scoping rules
- **Attribute Access**: `module.func` needs different handling than `func`

### 2. Import Ordering Constraints

- `__future__` imports must be first
- Stdlib imports should come before third-party
- Namespace objects must be created before use
- Circular dependencies need special handling

### 3. Namespace Object Design

Using `types.SimpleNamespace` for module objects:

```python
# Original: import mymodule
# Transformed:
from types import SimpleNamespace
mymodule = SimpleNamespace(
    func1=func1,
    func2=func2,
    CLASS1=CLASS1
)
```

Benefits:

- Preserves attribute access syntax
- No runtime overhead
- Clear in bundled output

### 4. Error Handling

- Invalid import transformations should fail gracefully
- Missing symbols should produce clear error messages
- Preserve Python's import semantics where possible

## Testing Strategy

### 1. Unit Tests

- AST transformer with known inputs/outputs
- Import classification logic
- Namespace object generation

### 2. Integration Tests

- Use existing test fixtures
- Start with simple import cases
- Progress to complex circular dependencies

### 3. Snapshot Tests

- Ensure deterministic output
- Track AST transformation results
- Validate import ordering

## Success Criteria

1. **Correctness**: Bundled code behavior matches original
2. **Import Resolution**: All first-party imports properly inlined
3. **Symbol Conflicts**: All naming conflicts resolved via AST rewriting
4. **Import Organization**: Logical grouping and ordering of imports
5. **Performance**: No significant slowdown in bundling process

## Risk Mitigation

1. **AST Corruption**: Extensive validation after transformation
2. **Semantic Changes**: Comprehensive test suite
3. **Performance**: Profile transformation passes
4. **Complexity**: Incremental implementation with early testing

## Dependencies on Existing System

This implementation depends on:

- Current BundlePlan structure
- ExecutionStep enum
- Plan executor framework
- ItemId to statement mapping
- Symbol rename decisions from analysis

## Future Extensions

1. **Dynamic Import Handling**: Support for `importlib.import_module()`
2. **Conditional Imports**: Handle `if TYPE_CHECKING:` blocks
3. **Star Imports**: Resolve `from module import *`
4. **Package Imports**: Handle `__init__.py` semantics
