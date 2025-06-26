# Cribo Bundling Architecture & Phases

This document outlines the current architecture of Cribo's Python bundling system and describes the distinct phases of the bundling process.

## Overview

Cribo uses a multi-phase pipeline architecture to transform a multi-module Python codebase into a single bundled file. The system is designed to handle complex scenarios including circular dependencies, conditional imports, namespace packages, and tree-shaking optimization.

## Core Components

### 1. BundleOrchestrator

The central coordinator that manages the entire bundling pipeline. It owns:

- **ModuleRegistry**: Central source of truth for module identity
- **SemanticBundler**: Semantic analysis engine
- **Module Cache**: Prevents redundant parsing

### 2. ModuleRegistry

A centralized registry that maintains the authoritative mapping between:

- Module IDs (internal numeric identifiers)
- Canonical module names (e.g., "requests.compat")
- Resolved file paths
- Original source code and AST
- Module metadata (is_wrapper, etc.)

### 3. CriboGraph

The dependency graph that tracks:

- Module dependencies
- Import relationships
- Circular dependency detection
- Topological ordering

### 4. SemanticBundler

Leverages `ruff_python_semantic` to:

- Build semantic models for each module
- Track symbol definitions and usage
- Detect symbol conflicts
- Identify module-scope vs function-scope symbols

### 5. HybridStaticBundler

The code generator that:

- Transforms ASTs
- Handles import rewriting
- Manages module initialization
- Generates the final bundled output

## Bundling Phases

### Phase 1: Discovery & Graph Construction

**Purpose**: Find all modules and build the dependency graph

1. **Entry Point Resolution**
   - Resolve entry file path (handle directories with `__main__.py` or `__init__.py`)
   - Detect package structure
   - Configure source directories

2. **Module Discovery**
   - Parse entry module
   - Extract imports using `ImportDiscoveryVisitor`
   - Resolve each import to a file path
   - Add discovered modules to processing queue
   - Recursively process until all dependencies are found

3. **Graph Building**
   - Create `ModuleId` for each module
   - Add modules to `CriboGraph`
   - Track dependencies between modules
   - Populate `ModuleRegistry` with module information

**Key Output**: Complete dependency graph and populated module registry

### Phase 2: Semantic Analysis

**Purpose**: Understand the code structure and symbols

1. **Semantic Model Building**
   - For each module, build a `SemanticModel` using ruff
   - Track all bindings (variables, functions, classes)
   - Identify module-level vs function-level symbols
   - Track import aliases and their resolutions

2. **Symbol Collection**
   - Extract exported symbols (public API)
   - Collect ALL module-scope symbols (including conditional)
   - Register symbols in global symbol registry
   - Track which modules define which symbols

3. **Conflict Detection**
   - Identify symbols defined in multiple modules
   - Generate rename strategies for conflicts
   - Mark modules with conflicts for special handling

**Key Output**: Semantic understanding of all modules with symbol information

### Phase 3: Circular Dependency Analysis

**Purpose**: Detect and classify circular dependencies

1. **Cycle Detection**
   - Use graph algorithms to find cycles
   - Classify cycle types:
     - FunctionLevel (safe, can be resolved)
     - ClassLevel (risky, may fail)
     - ModuleConstants (likely unresolvable)
     - ImportTime (depends on execution order)

2. **Resolution Strategy**
   - For resolvable cycles: plan import rewriting
   - For unresolvable cycles: fail with clear error
   - Mark circular modules for special handling

**Key Output**: Circular dependency analysis and resolution strategies

### Phase 4: Tree-Shaking Analysis (Optional)

**Purpose**: Identify unused code for elimination

1. **Side Effect Detection**
   - Identify modules with side effects
   - Mark them as excluded from tree-shaking

2. **Usage Analysis**
   - Start from entry module
   - Trace symbol usage transitively
   - Mark used symbols and their dependencies
   - Respect `__all__` declarations

3. **Dead Code Identification**
   - Identify unused symbols
   - Plan their removal from final output

**Key Output**: Set of symbols to keep/remove

### Phase 5: AST Transformation

**Purpose**: Prepare modules for bundling

1. **Import Normalization**
   - Normalize stdlib imports (e.g., `json as js` → `json`)
   - Track import aliases for rewriting

2. **Import Rewriting**
   - For circular dependencies: move imports into functions
   - Transform imports of bundled modules
   - Handle namespace imports

3. **Symbol Renaming**
   - Apply rename strategy for conflicting symbols
   - Update all references to renamed symbols

4. **Conditional Import Handling**
   - Process if/else and try/except blocks
   - Ensure conditional imports are exposed in module namespace
   - Add `module.symbol = symbol` assignments after conditional imports

**Key Output**: Transformed ASTs ready for bundling

### Phase 6: Code Generation

**Purpose**: Generate the final bundled Python file

1. **Header Generation**
   - Add shebang and bundle metadata
   - Collect and hoist safe stdlib imports
   - Generate `__future__` imports

2. **Module Initialization**
   - Generate init functions for each module
   - Create module namespaces using `types.SimpleNamespace`
   - Set up `sys.modules` cache (*this is redundant? - not a project goal*)

3. **Module Ordering**
   - Use topological sort (or cycle-aware ordering)
   - Ensure dependencies are defined before use
   - Handle wrapper modules (with side effects) specially

4. **Body Generation**
   - Inline module code according to strategy:
     - Inlinable modules: merge into global scope
     - Wrapper modules: keep in init functions
   - Apply tree-shaking results
   - Handle deferred imports

5. **Entry Point**
   - Execute entry module code
   - Set up proper `__name__ == "__main__"` handling

**Key Output**: Complete bundled Python file

### Phase 7: Post-Processing

**Purpose**: Finalize the bundle

1. **Requirements Generation** (optional)
   - Extract third-party dependencies
   - Generate `requirements.txt`

2. **Validation**
   - Ensure all imports are resolved
   - Verify symbol availability
   - Check for potential runtime issues

3. **Output**
   - Write bundled code to file or stdout
   - Apply any final formatting

## Data Flow

```
Entry Path → Discovery → CriboGraph → SemanticBundler → Transformations → HybridStaticBundler → Output
                ↓                           ↓
          ModuleRegistry ←──────────────────┘
                ↓
         (Shared across all phases for module identity resolution)
```

## Key Design Decisions

1. **Central Module Registry**: Eliminates fragile path-based lookups
2. **Hybrid Static Bundling**: Combines static analysis with runtime module registration
3. **Semantic Analysis First**: Understand code before transforming
4. **Multiple Transformation Passes**: Each pass has a specific purpose
5. **Configurable Optimization**: Tree-shaking and other optimizations are optional
6. **Deterministic Output**: Consistent ordering and naming for reproducible builds

## Error Handling

Each phase has specific error conditions:

- **Discovery**: Module not found, import errors
- **Semantic**: Parse errors, invalid Python
- **Circular**: Unresolvable cycles
- **Transform**: Rewriting conflicts
- **Generation**: Name collisions

Errors are reported with context about which phase failed and why.

## Architectural Enhancement Plan

Based on deep analysis with Gemini, we've identified key weaknesses and opportunities for simplification:

### Current Weaknesses

1. **Side-Channel Communication**
   - SemanticBundler stores conflicts in internal state
   - CircularDependencyAnalysis passed separately to CodeGenerator
   - TreeShaker results passed as separate objects
   - **Impact**: Tight coupling, reduced testability, hidden complexity

2. **Monolithic Mutable Graph**
   - Single CriboGraph object mutated by each phase sequentially
   - **Impact**: Implicit dependencies, prevents parallelism, difficult debugging

3. **Redundant Analysis Phases**
   - Semantic, Circular Dependency, and Tree-Shaking analyses are separate
   - Multiple full graph traversals for related operations
   - **Impact**: Performance overhead, conceptual duplication

### Proposed Solution: BundlePlan Architecture

Introduce a unified `BundlePlan` data structure that consolidates all bundling decisions:

```rust
pub struct BundlePlan {
    // Core planning data
    pub final_statement_order: Vec<ItemId>,
    pub live_items: FxHashSet<ItemId>,
    pub symbol_renames: FxHashMap<SymbolId, String>,
    pub hoisted_imports: Vec<String>,

    // Advanced scenarios
    pub module_metadata: FxHashMap<ModuleId, ModuleBundleType>,
    pub synthetic_namespaces: FxHashMap<ItemId, Vec<SymbolId>>,
    pub lifted_globals: FxHashMap<ItemId, FxHashSet<String>>,
    pub import_rewrite_map: FxHashMap<ItemId, ItemId>,
}
```

### Simplified 3-Phase Architecture

1. **Representation Phase**
   - Build immutable CriboGraph from source code
   - No mutations after this phase

2. **Unified Analysis Phase**
   - Consume immutable graph
   - Perform ALL analyses (semantic, cycles, tree-shaking)
   - Output single BundlePlan

3. **Synthesis Phase**
   - CodeGenerator becomes a "dumb executor"
   - Only inputs: immutable graph + BundlePlan
   - No knowledge of why decisions were made

### Benefits

- **Eliminates all side-channels**: All information in BundlePlan
- **Enables true incrementalism**: Can update parts of graph and re-analyze
- **Improves testability**: Each phase can be tested in isolation
- **Clearer architecture**: Explicit data flow, no hidden dependencies

### Implementation Roadmap

1. **Phase 1: BundlePlan Prototype**
   - Create BundlePlan struct
   - Refactor CircularDependencyAnalysis to output import_rewrite_map
   - Test with existing CodeGenerator

2. **Phase 2: Unified Analysis**
   - Consolidate three analysis phases
   - Make CriboGraph immutable after construction
   - Update CodeGenerator to use only BundlePlan

3. **Phase 3: Performance & Polish**
   - Profile and optimize unified analysis
   - Update documentation
   - Add comprehensive tests

This enhancement maintains all current capabilities while significantly improving maintainability, testability, and performance.
