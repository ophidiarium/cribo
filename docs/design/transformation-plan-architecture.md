# Transformation Plan Architecture

## Overview

This document describes the architectural pattern for handling AST transformations in the Cribo bundler. Instead of mutating ASTs in-place during pre-processing, we use a declarative "Transformation Plan" that is executed by the BundleCompiler.

## Problem Statement

The current architecture has AST transformation steps (stdlib normalization, import rewriting) that run after semantic analysis but before compilation. This creates several issues:

1. **Metadata Desynchronization**: The dependency graph is built from original ASTs, but the compiler receives transformed ASTs
2. **Architectural Violation**: Mutations happen outside the compiler, breaking the clean separation of concerns
3. **Timing Issues**: Import aliases are removed by normalization but the graph still contains the original import info

## Architectural Principles

### Three-Stage Compiler Model

```
┌─────────────┐     ┌────────────────┐     ┌──────────┐
│   Frontend  │     │    Mid-end     │     │ Backend  │
│  (Analysis) │ --> │ (BundleCompiler)│ --> │   (VM)   │
└─────────────┘     └────────────────┘     └──────────┘
      |                    |                     |
      v                    v                     v
AnalysisResults      BundleProgram         Final AST
(High-Level IR)      (Low-Level IR)
```

### Key Principles

1. **Single Traversal**: AST analysis happens exactly once in the frontend
2. **Declarative Plans**: Analysis produces a plan of *what* to transform, not *how*
3. **Compiler Intelligence**: BundleCompiler knows *how* to execute transformations
4. **VM Simplicity**: The VM remains a mechanical executor of simple instructions

## Design

### TransformationMetadata

The analysis phase produces transformation metadata for items that need changes:

```rust
#[derive(Debug, Clone)]
pub enum TransformationMetadata {
    /// Stdlib import needs normalization
    StdlibImportRewrite {
        // Original: from typing import Any, List
        // Target: import typing
        canonical_module: String,
        symbols: Vec<(String, String)>, // (original, canonical)
    },

    /// Partial import removal - remove specific symbols from a from-import
    PartialImportRemoval {
        // from foo import One, Two, Three -> from foo import Two
        // (if One and Three are unused)
        remaining_symbols: Vec<(String, Option<String>)>, // (name, alias)
        removed_symbols: Vec<String>,                     // For debugging/logging
    },

    /// Symbol usage needs rewriting (generic for all symbol transformations)
    SymbolRewrite {
        // Map of TextRange -> new text
        // Handles: qualifications (Any -> typing.Any), renames (foo -> _b_foo),
        // attribute rewrites (j.dumps -> json.dumps)
        rewrites: FxHashMap<TextRange, String>,
    },

    /// Import needs moving for circular deps
    CircularDepImportMove {
        target_scope: ItemId, // Function to move import into
        import_stmt: Stmt,    // The import to move
    },

    /// Import should be removed (unused)
    RemoveImport { reason: RemovalReason },
}

#[derive(Debug, Clone)]
pub enum RemovalReason {
    /// Import is completely unused
    Unused,
    /// Import is only used in type annotations (and we're stripping types)
    TypeOnly,
    /// Import was inlined/bundled
    Bundled,
}
```

### AnalysisResults Extension

```rust
pub struct AnalysisResults {
    // ... existing fields ...
    /// The dependency graph (currently passed separately - should be here!)
    pub graph: CriboGraph,

    /// Transformation plan: ItemId -> required transformation
    pub transformations: FxHashMap<ItemId, TransformationMetadata>,
}
```

**Note**: Currently the graph is passed separately from AnalysisResults, which violates the principle of having analysis produce a complete result. The graph should be part of AnalysisResults since:

1. It's produced during analysis
2. It's needed by the compiler
3. Transformations reference ItemIds from the graph

### BundleCompiler Execution Model

The compiler maintains state for handling transformations:

```rust
// Global state for the entire bundle
pub struct BundleCompiler {
    // ... other fields ...
    hoisted_imports: FxHashSet<String>, // Prevents duplicate imports
}

// Local state for recursive compilation
struct CompilationContext<'a> {
    compiler: &'a mut BundleCompiler,
    destination_module_id: ModuleId,
    symbol_resolution_map: FxHashMap<String, String>, // e.g., {"Any": "typing.Any"}
    processed_items: FxHashSet<ItemId>,
}
```

For each item, transformations are processed in priority order:

```rust
// Process transformations with fixed priority
fn process_item(&mut self, item_id: ItemId) -> Option<ExecutionStep> {
    let transformations = self.get_sorted_transformations(item_id);

    for transformation in transformations {
        match transformation {
            RemoveImport { .. } => return None, // Highest priority - skip item
            CircularDepImportMove { .. } => { /* relocate */ }
            StdlibImportRewrite { .. } => {
                self.update_symbol_resolution_map(..);
                return Some(build_normalized_import(..));
            }
            PartialImportRemoval { .. } => return Some(build_partial_import(..)),
            SymbolRewrite { rewrites } => {
                // Apply all rewrites to the statement
                return Some(build_transformed_statement(stmt, rewrites));
            }
        }
    }

    // No transformations - fast path
    Some(ExecutionStep::CopyStatement { .. })
}
```

## Benefits

1. **Clean Boundaries**: Each phase has clear responsibilities
2. **No Re-traversal**: AST analysis happens once
3. **Testability**: Each phase can be tested independently
4. **Maintainability**: Transformation logic is centralized in the compiler
5. **Performance**: Fast path for unmodified code

## Implementation Strategy

Since we're implementing this as a complete architectural change in a single branch, the implementation will proceed as follows:

### Core Changes

1. Add TransformationMetadata enum and extend AnalysisResults
2. Create transformation detection as part of the analysis pipeline
3. Remove ALL AST mutation code from orchestrator
4. Update BundleCompiler to execute transformation plans
5. Implement AST builders for each transformation type

### Key Principles

- No parallel paths or feature flags
- Direct replacement of the old system
- All tests must pass with the new architecture
- No backwards compatibility needed

## Example: Stdlib Normalization

### Before (Current Architecture)

```
1. Analysis: Builds graph with "import json as j"
2. Normalization: Mutates AST to "import json" + rewrites "j" -> "json"
3. Compiler: Sees transformed AST but uses stale graph data
```

### After (Transformation Plan)

```
1. Analysis: 
   - Builds graph with original import
   - Detects alias, adds to transformations:
     StdlibImportRewrite { canonical_module: "json", symbols: [] }
     SymbolRewrite { rewrites: {j@123 -> "json"} }
   
2. Compiler:
   - Sees transformation for import statement
   - Builds new "import json" statement
   - Emits InsertStatement
   
3. VM:
   - Mechanically inserts the pre-built statement
```

## Handling Import Removal

The transformation architecture naturally handles import removal through the `RemoveImport` transformation. This unifies several current mechanisms:

### Current State (Fragmented)

1. Tree-shaking marks unused imports
2. Import classification filters some imports
3. Bundle compiler skips classified imports
4. Multiple places decide what to include/exclude

### New Architecture (Unified)

1. Analysis phase detects unused imports and adds `RemoveImport` transformation
2. Bundle compiler sees transformation and emits no ExecutionStep
3. Import simply doesn't appear in final bundle

### Types of Import Removal

#### Complete Removal (`RemoveImport`)

- **Unused**: Import never referenced in code
- **TypeOnly**: Import only used in type annotations (when stripping types)
- **Bundled**: First-party import that's been inlined

#### Partial Removal (`PartialImportRemoval`)

Handles cases like `from foo import One, Two, Three` where only some symbols are used:

- Analysis determines which symbols are actually used
- Transformation specifies remaining symbols
- Compiler rebuilds import with only needed symbols

Example:

```python
# Original
from utils import helper1, helper2, helper3, DEBUG_FLAG

# Analysis finds helper2 and DEBUG_FLAG unused
# Transformation: PartialImportRemoval { 
#     remaining_symbols: [("helper1", None), ("helper3", None)],
#     removed_symbols: ["helper2", "DEBUG_FLAG"]
# }

# Result
from utils import helper1, helper3
```

This approach ensures all import removal decisions are made in one place (analysis) and executed uniformly (compiler).

## Key Architectural Decisions

### 1. No ItemId Remapping

When inlining module B into module A, items retain their original ItemIds. The CompilationContext tracks where output goes, not item identity.

### 2. Transformation Priority Order

Transformations are processed in a fixed priority order per item:

1. RemoveImport (highest - if present, skip all others)
2. CircularDepImportMove
3. StdlibImportRewrite
4. PartialImportRemoval
5. SymbolRewrite (lowest - changes to usages)

### 3. Generic SymbolRewrite

We keep SymbolRewrite generic (TextRange → String) rather than splitting into specific types. The compiler is responsible for parsing the string and building the appropriate AST node.

### 4. Fail-Fast Error Handling

The BundleCompiler must fail immediately on internal errors (e.g., invalid TextRange). Silent recovery could produce syntactically valid but semantically incorrect code.

### 5. Analysis Detects Conflicts

Semantic conflicts (e.g., namespace collisions from inlining) must be detected by the analysis phase and resolved via preemptive transformations. The compiler only executes valid plans.

## Future Extensions

This architecture easily supports new transformation types:

- Dead code elimination markers
- Optimization hints
- Module merging directives
- Performance annotations
- Type stripping transformations

Simply add new variants to TransformationMetadata and corresponding logic to BundleCompiler.
