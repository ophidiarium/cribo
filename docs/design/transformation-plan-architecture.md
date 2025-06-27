# Transformation Plan Architecture

## Overview

The Transformation Plan Architecture is a two-phase system that separates the identification of code changes (detection phase) from their execution (compilation phase). This architecture replaces direct AST mutations with a declarative transformation plan, improving maintainability, testability, and architectural clarity.

## Core Concepts

### Two-Phase Processing

1. **Detection Phase**: Analyzes the code and produces a declarative plan of transformations
2. **Compilation Phase**: Executes the transformation plan to generate the final bundled code

### Key Principles

- **Immutable ASTs**: Original ASTs are never mutated; transformations create new code
- **Declarative Plans**: Transformations are described as data, not imperative operations
- **Separation of Concerns**: Analysis logic is isolated from execution logic
- **Precise Addressing**: Every transformation targets a specific AST node via NodeIndex

## Architecture Components

### TransformationMetadata

The core data structure that describes a transformation to be applied:

```rust
pub enum TransformationMetadata {
    /// Remove an import statement entirely
    RemoveImport { reason: RemovalReason },

    /// Remove some symbols from a from-import
    PartialImportRemoval {
        remaining_symbols: Vec<(String, Option<String>)>,
        removed_symbols: Vec<String>,
    },

    /// Normalize a stdlib import
    StdlibImportRewrite {
        canonical_module: String,
        symbols: Vec<(String, String)>, // (original, canonical)
    },

    /// Rewrite symbol usages
    SymbolRewrite {
        rewrites: FxHashMap<NodeIndex, String>,
    },

    /// Move an import to resolve circular dependencies
    CircularDepImportMove {
        target_scope: ItemId,
        import_data: ImportData, // Semantic representation
    },
}
```

### Transformation Map

A mapping from AST nodes to their transformations:

```rust
transformations: FxHashMap<NodeIndex, Vec<TransformationMetadata>>
```

Key insights:

- Uses `NodeIndex` for precise, stable addressing
- Multiple transformations can apply to a single node
- Transformations are ordered by priority

### NodeIndex System

Every AST node receives a unique index during parsing:

- Stable across transformations
- Module-relative addressing (1M indices per module)
- Enables precise transformation targeting
- Foundation for source map generation

## Detection Phase

### TransformationDetector

Responsible for analyzing code and identifying needed transformations:

1. **Input**: CriboGraph, SemanticModelProvider, TreeShakeResults
2. **Process**:
   - Walks the graph analyzing each item
   - Identifies patterns requiring transformation
   - Queries semantic model for symbol usage
   - Creates TransformationMetadata instances
3. **Output**: Populated transformation map

### Detection Patterns

#### Unused Import Detection

- Leverages tree-shaking results
- Checks if import is in `included_items`
- Creates `RemoveImport` transformation

#### Stdlib Normalization

- Identifies stdlib imports with aliases
- Finds all symbol usages via semantic model
- Creates `StdlibImportRewrite` + `SymbolRewrite` transformations

#### Circular Dependency Resolution

- Analyzes import usage patterns
- Determines safe relocation targets
- Creates `CircularDepImportMove` transformations

## Compilation Phase

### BundleCompiler

Executes the transformation plan:

1. **Input**: Analysis results including transformation map
2. **Process**:
   - Iterates through live items in topological order
   - Applies transformations to AST nodes
   - Renders transformed ASTs to code
3. **Output**: ExecutionStep instructions for the VM

### Transformation Execution

The compiler uses a recursive AST transformer:

```rust
fn transform_and_render(
    node: &Stmt,
    transformations: &FxHashMap<NodeIndex, Vec<TransformationMetadata>>,
) -> String {
    // 1. Check for transformations on this node
    // 2. Apply transformations in priority order
    // 3. Recursively transform child nodes
    // 4. Pretty-print the result
}
```

### ExecutionStep Evolution

New execution step for transformed code:

```rust
pub enum ExecutionStep {
    /// Insert pre-built AST statement
    InsertStatement { stmt: Stmt },

    /// Insert fully rendered code (NEW)
    InsertRenderedCode {
        source_module: ModuleId,
        original_item_id: ItemId,
        code: String,
    },

    /// Copy statement without transformation (RARE)
    CopyStatement {
        source_module: ModuleId,
        item_id: ItemId,
    },
}
```

## Transformation Types

### RemoveImport

**Purpose**: Eliminate unnecessary imports

- Unused imports (from tree-shaking)
- Type-only imports (when type stripping enabled)
- First-party imports (will be inlined)

**Detection**: Check tree-shake results and import classification
**Execution**: Skip the import entirely

### PartialImportRemoval

**Purpose**: Remove unused symbols from from-imports

- `from typing import Any, List` → `from typing import Any`

**Detection**: Per-symbol usage analysis
**Execution**: Generate new from-import with remaining symbols

### StdlibImportRewrite

**Purpose**: Normalize stdlib imports for consistency

- `from typing import Any` → `import typing` + rewrite `Any` to `typing.Any`

**Detection**: Identify stdlib imports and find all symbol usages
**Execution**:

1. Replace import with canonical form
2. Apply SymbolRewrite to all usages

### SymbolRewrite

**Purpose**: Rename or requalify symbol usages

- Support for stdlib normalization
- Resolve naming conflicts
- Handle circular dependency moves

**Detection**: Via semantic model's reference tracking
**Execution**: Replace AST nodes at specific NodeIndices

### CircularDepImportMove

**Purpose**: Break circular dependencies by moving imports

- Move import from module scope to function scope

**Detection**: Analyze where imported symbols are used
**Execution**:

1. Remove import from original location
2. Insert inside target function/class

## Benefits

### Architectural Clarity

- Clear separation between "what" and "how"
- Each phase has a single responsibility
- Easy to understand and reason about

### Testability

- Detection can be tested without code generation
- Transformations can be tested in isolation
- Easy to verify transformation plans

### Maintainability

- Adding new transformations is straightforward
- Changes don't cascade across phases
- Debugging is simplified

### Robustness

- NodeIndex provides stable addressing
- No string-based text manipulation
- Preserves AST structure integrity

## Integration Points

### With Semantic Analysis

- Reads from SemanticModelProvider
- Uses GlobalBindingId for symbol resolution
- Leverages reference tracking

### With Tree Shaking

- Consumes TreeShakeResults
- Respects included_items decisions
- Coordinates import removal

### With Bundle VM

- Produces simple ExecutionSteps
- VM remains text-based and simple
- Clear execution boundary

## Future Extensions

### Source Maps

- NodeIndex provides foundation
- Track transformations for debugging
- Map bundled code to original

### Incremental Compilation

- Transformation map can be cached
- Only recompute for changed modules
- Enable faster rebuilds

### Advanced Transformations

- Constant folding
- Dead code elimination beyond imports
- Optimization passes
