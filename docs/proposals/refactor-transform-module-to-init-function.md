# Refactoring Proposal: `transform_module_to_init_function`

**Date:** 2025-10-16
**Author:** System Analysis
**Status:** Proposal
**Target:** `crates/cribo/src/code_generator/module_transformer.rs::transform_module_to_init_function`

## Executive Summary

The `transform_module_to_init_function` function is a 1,511-line monolithic function (34-1544) that transforms a Python module AST into an initialization function for wrapper modules. This function is critical to cribo's bundling pipeline but has become difficult to maintain, test, and reason about due to its size and complexity.

This proposal outlines a systematic refactoring plan to decompose this function into cohesive, manageable components while preserving its correctness and maintaining all existing behavior.

---

## Current State Analysis

### Function Signature

```rust
pub fn transform_module_to_init_function<'a>(
    bundler: &'a Bundler<'a>,
    ctx: &ModuleTransformContext,
    mut ast: ModModule,
    symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
) -> Stmt
```

### Primary Responsibilities

The function currently handles 13 distinct phases:

1. **Initialization** (lines 34-126): Setup guards, globals lifting
2. **Import Collection** (lines 128-311): Analyze all imports
3. **Import Transformation** (lines 313-388): Transform import statements
4. **Wrapper Symbol Setup** (lines 390-416): Create placeholder assignments
5. **Wildcard Import Processing** (lines 418-454): Handle `from module import *`
6. **Preparation** (lines 456-581): Analyze body for processing
7. **Body Processing** (lines 583-645): Transform module body
8. **Wrapper Globals Collection** (lines 647-715): Collect global declarations
9. **Statement Processing** (lines 717-1296): **580 lines** - Process each statement type
10. **Submodule Handling** (lines 1298-1416): Set up submodule namespaces
11. **Final Cleanup** (lines 1418-1471): Re-exports and remaining imports
12. **Globals/Locals Transform** (lines 1473-1492): Transform special calls
13. **Finalization** (lines 1494-1544): Return the function statement

### Key Problems

1. **Excessive Length**: 1,511 lines in a single function
2. **High Cyclomatic Complexity**: Deep nesting, many branches
3. **Phase Coupling**: State flows implicitly between phases
4. **Statement Processing Loop**: 580-line match expression (lines 717-1296)
5. **Testing Difficulty**: Cannot test individual phases in isolation
6. **Cognitive Load**: Understanding data flow requires reading entire function
7. **Maintenance Risk**: Changes to one phase may affect others unpredictably

---

## Data Flow Analysis

### Input Parameters

```rust
bundler: &'a Bundler<'a>                  // Read-only context with module info
ctx: &ModuleTransformContext              // Module metadata
ast: ModModule                            // Module AST (consumed)
symbol_renames: &FxIndexMap<...>          // Symbol renaming map
```

### Intermediate State

The function maintains complex state across phases:

```rust
// Accumulated output
body: Vec<Stmt>                           // The init function body

// Import tracking
imports_from_inlined: Vec<(String, String, Option<String>)>
inlined_import_bindings: Vec<String>
wrapper_module_symbols_global_only: Vec<(String, String)>
imported_symbols: FxIndexSet<String>
stdlib_reexports: FxIndexSet<(String, String)>

// Global variable tracking
lifted_names: Option<FxIndexMap<String, String>>
initialized_lifted_globals: FxIndexSet<String>
builtin_locals: FxIndexSet<String>

// Analysis results
module_scope_symbols: Option<&FxIndexSet<String>>
vars_used_by_exported_functions: FxIndexSet<String>
all_is_referenced: bool
```

### Output

```rust
Stmt::FunctionDef(...)                    // The complete init function
```

---

## Proposed Architecture

### High-Level Design Principles

1. **Single Responsibility**: Each component handles one phase
2. **Explicit State**: Phases communicate via explicit data structures
3. **Testability**: Each phase can be unit tested independently
4. **Composability**: Phases compose to form the complete transformation
5. **Maintainability**: Clear boundaries reduce coupling

### Phase Decomposition Strategy

We will extract each phase into a dedicated struct/module with a clear interface.

---

## Detailed Refactoring Plan

### Phase 1: Create State Container

**File:** `crates/cribo/src/code_generator/init_function/state.rs`

Create an explicit state container to replace scattered local variables:

```rust
/// State accumulated during init function transformation
pub struct InitFunctionState {
    /// Accumulated init function body statements
    pub body: Vec<Stmt>,

    /// Import tracking
    pub imports_from_inlined: Vec<(String, String, Option<String>)>,
    pub inlined_import_bindings: Vec<String>,
    pub wrapper_module_symbols_global_only: Vec<(String, String)>,
    pub imported_symbols: FxIndexSet<String>,
    pub stdlib_reexports: FxIndexSet<(String, String)>,

    /// Global variable management
    pub lifted_names: Option<FxIndexMap<String, String>>,
    pub initialized_lifted_globals: FxIndexSet<String>,
    pub builtin_locals: FxIndexSet<String>,

    /// Analysis results
    pub module_scope_symbols: Option<FxIndexSet<String>>,
    pub vars_used_by_exported_functions: FxIndexSet<String>,
    pub all_is_referenced: bool,
}

impl InitFunctionState {
    pub fn new() -> Self { ... }
}
```

**Benefits:**

- Explicit data flow between phases
- Single parameter to pass between functions
- Clear ownership of state
- Easy to serialize/debug

---

### Phase 2: Extract Initialization Phase

**File:** `crates/cribo/src/code_generator/init_function/initialization.rs`

```rust
pub struct InitializationPhase;

impl InitializationPhase {
    /// Add initialization guards and setup to the function body
    pub fn execute(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        ast: &mut ModModule,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 34-126
        // - Add __initialized__ check
        // - Add __initializing__ check
        // - Set __initializing__ = True
        // - Apply globals lifting
    }
}
```

**Extracted from:** Lines 34-126

---

### Phase 3: Extract Import Analysis Phase

**File:** `crates/cribo/src/code_generator/init_function/import_analysis.rs`

```rust
pub struct ImportAnalysisPhase;

impl ImportAnalysisPhase {
    /// Analyze all imports in the module and populate tracking state
    pub fn execute(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        ast: &ModModule,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 128-311
        // - Loop through all import statements
        // - Collect imported symbols
        // - Resolve modules
        // - Track stdlib imports
        // - Process wildcard imports
    }

    /// Helper: Process a single ImportFrom statement
    fn process_import_from(
        import_from: &StmtImportFrom,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) { ... }

    /// Helper: Process a single Import statement
    fn process_import(
        import: &StmtImport,
        state: &mut InitFunctionState,
    ) { ... }
}
```

**Extracted from:** Lines 128-311

---

### Phase 4: Extract Import Transformation Phase

**File:** `crates/cribo/src/code_generator/init_function/import_transformation.rs`

```rust
pub struct ImportTransformationPhase;

impl ImportTransformationPhase {
    /// Transform imports using RecursiveImportTransformer and add global declarations
    pub fn execute(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        ast: &mut ModModule,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 313-388
        // - Create and run RecursiveImportTransformer
        // - Add global declarations for inlined imports
    }
}
```

**Extracted from:** Lines 313-388

---

### Phase 5: Extract Wrapper Symbol Setup Phase

**File:** `crates/cribo/src/code_generator/init_function/wrapper_symbols.rs`

```rust
pub struct WrapperSymbolSetupPhase;

impl WrapperSymbolSetupPhase {
    /// Create placeholder assignments for wrapper module symbols
    pub fn execute(bundler: &Bundler, state: &mut InitFunctionState) -> Result<(), TransformError> {
        // Lines 390-416
        // - Add placeholder assignments (types.SimpleNamespace())
        // - Add module attribute assignments
    }
}
```

**Extracted from:** Lines 390-416

---

### Phase 6: Extract Wildcard Import Processing Phase

**File:** `crates/cribo/src/code_generator/init_function/wildcard_imports.rs`

```rust
pub struct WildcardImportPhase;

impl WildcardImportPhase {
    /// Process wildcard imports and add module attributes
    pub fn execute(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 418-454
        // - Deduplicate and sort wildcard imports
        // - Add module attributes for exported symbols
    }
}
```

**Extracted from:** Lines 418-454

---

### Phase 7: Extract Body Preparation Phase

**File:** `crates/cribo/src/code_generator/init_function/body_preparation.rs`

```rust
pub struct BodyPreparationPhase;

impl BodyPreparationPhase {
    /// Prepare for body processing by analyzing the module
    pub fn execute(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        ast: &ModModule,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 456-645
        // - Check if __all__ is referenced
        // - Collect variables used by exported functions
        // - Get module scope symbols
        // - Scan for built-in name shadowing
        // - Process body recursively
        // - Filter circular init attempts
        // - Declare lifted globals
    }
}
```

**Extracted from:** Lines 456-645

---

### Phase 8: Extract Wrapper Globals Collection Phase

**File:** `crates/cribo/src/code_generator/init_function/wrapper_globals.rs`

```rust
pub struct WrapperGlobalsPhase;

impl WrapperGlobalsPhase {
    /// Collect and declare wrapper module namespace variables
    pub fn execute(
        processed_body: &[Stmt],
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 647-715
        // - Use visitor to collect globals needed
        // - Add global declarations
    }
}

/// Visitor for collecting wrapper module globals
struct WrapperGlobalCollector {
    globals_needed: FxIndexSet<String>,
}
```

**Extracted from:** Lines 647-715

---

### Phase 9: Extract Statement Processing Phase (CRITICAL)

This is the largest and most complex phase. We'll decompose it into a trait-based system.

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/mod.rs`

```rust
/// Trait for processing different statement types
pub trait StatementProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool;

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError>;
}

/// Coordinator for statement processing
pub struct StatementProcessingPhase {
    processors: Vec<Box<dyn StatementProcessor>>,
}

impl StatementProcessingPhase {
    pub fn new() -> Self {
        Self {
            processors: vec![
                Box::new(ImportProcessor),
                Box::new(ImportFromProcessor),
                Box::new(ClassDefProcessor),
                Box::new(FunctionDefProcessor),
                Box::new(AssignProcessor),
                Box::new(AnnAssignProcessor),
                Box::new(TryProcessor),
                Box::new(DefaultProcessor),
            ],
        }
    }

    pub fn execute(
        &self,
        processed_body: Vec<Stmt>,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        for (idx, stmt) in processed_body.into_iter().enumerate() {
            // Find appropriate processor
            let processor = self
                .processors
                .iter()
                .find(|p| p.can_process(&stmt))
                .ok_or_else(|| TransformError::NoProcessor)?;

            processor.process(stmt, bundler, ctx, state)?;
        }
        Ok(())
    }
}
```

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/import.rs`

```rust
pub struct ImportProcessor;

impl StatementProcessor for ImportProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool {
        matches!(stmt, Stmt::Import(_))
    }

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 729-733
        // Handle import statement
    }
}
```

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/import_from.rs`

```rust
pub struct ImportFromProcessor;

impl StatementProcessor for ImportFromProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool {
        matches!(stmt, Stmt::ImportFrom(_))
    }

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 735-866
        // Complex logic for relative imports
    }
}
```

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/class_def.rs`

```rust
pub struct ClassDefProcessor;

impl StatementProcessor for ClassDefProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool {
        matches!(stmt, Stmt::ClassDef(_))
    }

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 868-891
        // Add class, set __module__, add as module attribute
    }
}
```

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/function_def.rs`

```rust
pub struct FunctionDefProcessor;

impl StatementProcessor for FunctionDefProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool {
        matches!(stmt, Stmt::FunctionDef(_))
    }

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 893-920
        // Transform nested functions, add as module attribute
    }
}
```

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/assign.rs`

```rust
pub struct AssignProcessor;

impl StatementProcessor for AssignProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool {
        matches!(stmt, Stmt::Assign(_))
    }

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 922-1062
        // Very complex: handle __all__, self-referential, lifted vars, etc.
    }
}
```

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/ann_assign.rs`

```rust
pub struct AnnAssignProcessor;

impl StatementProcessor for AnnAssignProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool {
        matches!(stmt, Stmt::AnnAssign(_))
    }

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 1064-1160
        // Similar to Assign but with annotations
    }
}
```

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/try_stmt.rs`

```rust
pub struct TryProcessor;

impl StatementProcessor for TryProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool {
        matches!(stmt, Stmt::Try(_))
    }

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 1162-1265
        // Collect exportable symbols from all branches
    }
}
```

**File:** `crates/cribo/src/code_generator/init_function/statement_processor/default.rs`

```rust
pub struct DefaultProcessor;

impl StatementProcessor for DefaultProcessor {
    fn can_process(&self, stmt: &Stmt) -> bool {
        true // Handles all other statement types
    }

    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 1267-1295
        // Transform module vars and add to body
    }
}
```

**Extracted from:** Lines 717-1296

---

### Phase 10: Extract Submodule Handling Phase

**File:** `crates/cribo/src/code_generator/init_function/submodules.rs`

```rust
pub struct SubmoduleHandlingPhase;

impl SubmoduleHandlingPhase {
    /// Set up submodules as attributes on the module
    pub fn execute(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 1298-1416
        // - Collect submodules
        // - Deduplicate
        // - Add as module attributes
        // - Handle inlined vs wrapped submodules
    }
}
```

**Extracted from:** Lines 1298-1416

---

### Phase 11: Extract Final Cleanup Phase

**File:** `crates/cribo/src/code_generator/init_function/cleanup.rs`

```rust
pub struct CleanupPhase;

impl CleanupPhase {
    /// Add final elements: stdlib re-exports and remaining imports
    pub fn execute(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 1418-1471
        // - Add stdlib re-exports
        // - Handle explicit imports from inlined modules
    }
}
```

**Extracted from:** Lines 1418-1471

---

### Phase 12: Extract Globals/Locals Transformation Phase

**File:** `crates/cribo/src/code_generator/init_function/globals_locals.rs`

```rust
pub struct GlobalsLocalsTransformPhase;

impl GlobalsLocalsTransformPhase {
    /// Transform globals() and locals() calls throughout the body
    pub fn execute(
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Lines 1473-1492
        // - Transform globals() -> self.__dict__
        // - Transform locals() -> vars(self)
    }
}
```

**Extracted from:** Lines 1473-1492

---

### Phase 13: Extract Finalization Phase

**File:** `crates/cribo/src/code_generator/init_function/finalization.rs`

```rust
pub struct FinalizationPhase;

impl FinalizationPhase {
    /// Finalize the init function and return as Stmt
    pub fn build_function_stmt(
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: InitFunctionState,
    ) -> Result<Stmt, TransformError> {
        // Lines 1494-1544
        // - Mark as initialized
        // - Return self
        // - Create function parameters
        // - Create and return function statement
    }
}
```

**Extracted from:** Lines 1494-1544

---

### Phase 14: New Orchestrator

**File:** `crates/cribo/src/code_generator/init_function/orchestrator.rs`

```rust
pub struct InitFunctionBuilder<'a> {
    bundler: &'a Bundler<'a>,
    ctx: &'a ModuleTransformContext,
    symbol_renames: &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
}

impl<'a> InitFunctionBuilder<'a> {
    pub fn new(
        bundler: &'a Bundler<'a>,
        ctx: &'a ModuleTransformContext,
        symbol_renames: &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
    ) -> Self {
        Self {
            bundler,
            ctx,
            symbol_renames,
        }
    }

    /// Transform module to init function using phase-based approach
    pub fn build(self, mut ast: ModModule) -> Result<Stmt, TransformError> {
        let mut state = InitFunctionState::new();

        // Phase 1: Initialization
        InitializationPhase::execute(self.bundler, self.ctx, &mut ast, &mut state)?;

        // Phase 2: Import Analysis
        ImportAnalysisPhase::execute(
            self.bundler,
            self.ctx,
            &ast,
            self.symbol_renames,
            &mut state,
        )?;

        // Phase 3: Import Transformation
        ImportTransformationPhase::execute(
            self.bundler,
            self.ctx,
            &mut ast,
            self.symbol_renames,
            &mut state,
        )?;

        // Phase 4: Wrapper Symbol Setup
        WrapperSymbolSetupPhase::execute(self.bundler, &mut state)?;

        // Phase 5: Wildcard Import Processing
        WildcardImportPhase::execute(self.bundler, self.ctx, &mut state)?;

        // Phase 6: Body Preparation
        let processed_body =
            BodyPreparationPhase::execute(self.bundler, self.ctx, &ast, &mut state)?;

        // Phase 7: Wrapper Globals Collection
        WrapperGlobalsPhase::execute(&processed_body, &mut state)?;

        // Phase 8: Statement Processing
        let processor = StatementProcessingPhase::new();
        processor.execute(processed_body, self.bundler, self.ctx, &mut state)?;

        // Phase 9: Submodule Handling
        SubmoduleHandlingPhase::execute(self.bundler, self.ctx, self.symbol_renames, &mut state)?;

        // Phase 10: Final Cleanup
        CleanupPhase::execute(self.bundler, self.ctx, &mut state)?;

        // Phase 11: Globals/Locals Transformation
        GlobalsLocalsTransformPhase::execute(self.ctx, &mut state)?;

        // Phase 12: Finalization
        FinalizationPhase::build_function_stmt(self.bundler, self.ctx, state)
    }
}
```

---

## New Module Structure

```
crates/cribo/src/code_generator/init_function/
├── mod.rs                          # Public interface
├── orchestrator.rs                 # InitFunctionBuilder
├── state.rs                        # InitFunctionState
├── initialization.rs               # Phase 1
├── import_analysis.rs              # Phase 2
├── import_transformation.rs        # Phase 3
├── wrapper_symbols.rs              # Phase 4
├── wildcard_imports.rs             # Phase 5
├── body_preparation.rs             # Phase 6
├── wrapper_globals.rs              # Phase 7
├── statement_processor/
│   ├── mod.rs                      # StatementProcessor trait & coordinator
│   ├── import.rs                   # ImportProcessor
│   ├── import_from.rs              # ImportFromProcessor
│   ├── class_def.rs                # ClassDefProcessor
│   ├── function_def.rs             # FunctionDefProcessor
│   ├── assign.rs                   # AssignProcessor
│   ├── ann_assign.rs               # AnnAssignProcessor
│   ├── try_stmt.rs                 # TryProcessor
│   └── default.rs                  # DefaultProcessor
├── submodules.rs                   # Phase 9
├── cleanup.rs                      # Phase 10
├── globals_locals.rs               # Phase 11
└── finalization.rs                 # Phase 12
```

---

## Migration Strategy

### Step 1: Add Error Handling Infrastructure ✅ COMPLETED

Create `TransformError` type for phase-specific errors.

**Status**: Implemented in `crates/cribo/src/code_generator/init_function/mod.rs`

- Created `TransformError` enum with appropriate error variants
- Implemented `Display` and `std::error::Error` traits
- All tests pass (148/148)
- Clippy clean (0 warnings)

### Step 2: Create State Container ✅ COMPLETED

Implement `InitFunctionState` with all current local variables.

**Status**: Implemented in `crates/cribo/src/code_generator/init_function/state.rs`

- Created `InitFunctionState` struct with all 12 state variables
- Added comprehensive documentation for each field
- Implemented `new()` constructor and `Default` trait
- All tests pass (148/148)
- Clippy clean (0 warnings)

### Step 3: Extract Phases Incrementally

Start with simplest phases (initialization, finalization) and work inward.

**Order:**

1. Initialization
2. Finalization
3. Import Analysis
4. Import Transformation
5. Wrapper Symbol Setup
6. Wildcard Import Processing
7. Body Preparation
8. Wrapper Globals Collection
9. Submodule Handling
10. Final Cleanup
11. Globals/Locals Transformation
12. Statement Processing (last, most complex)

### Step 4: Create Orchestrator

Build `InitFunctionBuilder` that coordinates phases.

### Step 5: Update Call Site

Replace current function call with new builder:

```rust
// Old:
let init_fn = transform_module_to_init_function(
    bundler, ctx, ast, symbol_renames
);

// New:
let init_fn = InitFunctionBuilder::new(bundler, ctx, symbol_renames)
    .build(ast)?;
```

### Step 6: Remove Old Function

After all tests pass, remove the original 1,511-line function.

---

## Testing Strategy

### Unit Tests

Each phase will have dedicated unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialization_phase_creates_guards() {
        let mut state = InitFunctionState::new();
        // ... setup
        InitializationPhase::execute(&bundler, &ctx, &mut ast, &mut state).unwrap();

        // Verify __initialized__ and __initializing__ checks added
        assert!(state.body.iter().any(|s| /* check for __initialized__ */));
    }

    #[test]
    fn test_import_analysis_tracks_inlined_modules() {
        let mut state = InitFunctionState::new();
        // ... setup with inlined module import
        ImportAnalysisPhase::execute(&bundler, &ctx, &ast, &symbol_renames, &mut state).unwrap();

        // Verify tracking
        assert_eq!(state.inlined_import_bindings.len(), 1);
    }
}
```

### Integration Tests

The existing snapshot tests in `crates/cribo/tests/test_bundling_snapshots.rs` will serve as integration tests, ensuring the refactored version produces identical output.

### Validation Strategy

1. Run full test suite after each phase extraction
2. Compare AST output byte-for-byte with original implementation
3. Run all fixtures with both implementations
4. Use `insta` snapshots to catch any behavioral changes

---

## Performance Considerations

### Expected Performance Impact

**Minimal to None:**

- Phase extraction adds function call overhead (~13 calls)
- Modern compilers will likely inline most phases
- State container uses references, avoiding copies

**Potential Improvements:**

- Easier to identify and optimize hot paths
- Can add targeted benchmarks per phase
- Parallel processing opportunities in future (e.g., import analysis)

### Benchmarking Plan

Before and after refactoring, benchmark:

1. Small modules (< 100 LOC)
2. Medium modules (100-500 LOC)
3. Large modules (> 500 LOC)
4. Modules with heavy imports
5. Modules with circular dependencies

---

## Benefits of Refactoring

### Maintainability

- **Before**: Changing wrapper symbol logic requires reading 1,511 lines
- **After**: Wrapper symbol logic is isolated in `wrapper_symbols.rs`

### Testability

- **Before**: Cannot test import analysis without running entire transform
- **After**: Each phase has isolated unit tests

### Clarity

- **Before**: Data flow implicit through 100+ local variables
- **After**: Explicit state container shows what data each phase uses

### Extensibility

- **Before**: Adding new statement type requires modifying 580-line match
- **After**: Implement new `StatementProcessor` trait

### Debuggability

- **Before**: Setting breakpoints requires knowing line numbers
- **After**: Set breakpoints at phase boundaries

---

## Risks and Mitigations

### Risk 1: Behavioral Changes

**Mitigation**: Comprehensive snapshot testing, byte-for-byte comparison

### Risk 2: Performance Regression

**Mitigation**: Benchmark suite, profiling before/after

### Risk 3: Incomplete Extraction

**Mitigation**: Incremental approach, tests at each step

### Risk 4: Missed Edge Cases

**Mitigation**: Code review, manual testing of complex fixtures

---

## Timeline Estimate

| Phase                   | Estimated Effort | Description                        |
| ----------------------- | ---------------- | ---------------------------------- |
| Infrastructure          | 2 days           | Error types, state container       |
| Simple Phases (1-7)     | 5 days           | Extract and test phases 1-7        |
| Statement Processing    | 5 days           | Extract and test phase 8 (complex) |
| Remaining Phases (9-12) | 3 days           | Extract and test phases 9-12       |
| Orchestrator            | 1 day            | Build coordinator                  |
| Testing & Validation    | 3 days           | Comprehensive testing              |
| Documentation           | 1 day            | Update CLAUDE.md                   |
| **Total**               | **20 days**      |                                    |

---

## Success Criteria

1. ✅ All existing tests pass without modification
2. ✅ Snapshot tests produce identical output
3. ✅ No performance regression (< 5% slowdown)
4. ✅ Each phase has unit tests with > 80% coverage
5. ✅ Original function removed from codebase
6. ✅ Documentation updated

---

## Future Enhancements

After successful refactoring, opportunities unlock:

1. **Parallel Import Analysis**: Analyze imports concurrently
2. **Caching**: Cache intermediate phase results
3. **Instrumentation**: Add detailed logging per phase
4. **Optimization**: Profile and optimize hot phases
5. **Alternative Strategies**: Swap out statement processors

---

## Appendix A: Helper Function Analysis

The following helper functions in `module_transformer.rs` support the main function:

| Function                                     | Lines     | Complexity | Proposed Location                        |
| -------------------------------------------- | --------- | ---------- | ---------------------------------------- |
| `transform_expr_for_module_vars`             | 1547-1942 | High       | Keep as utility, used by multiple phases |
| `transform_stmt_for_module_vars`             | 1945-2314 | High       | Keep as utility                          |
| `transform_nested_function_for_module_vars`  | 2360-2400 | Medium     | Keep as utility                          |
| `collect_local_vars`                         | 2402-2477 | Medium     | Move to utilities                        |
| `transform_stmt_for_module_vars_with_locals` | 2479-2621 | High       | Keep as utility                          |
| `transform_expr_for_module_vars_with_locals` | 2623-2762 | High       | Keep as utility                          |
| `transform_expr_for_builtin_shadowing`       | 2795-2966 | High       | Move to `body_preparation.rs`            |
| `should_include_symbol`                      | 2968-3036 | Medium     | Move to `statement_processor/helpers.rs` |
| `add_module_attr_if_exported`                | 3038-3056 | Low        | Move to `statement_processor/assign.rs`  |
| `emit_module_attr_if_exportable`             | 3058-3092 | Low        | Move to `statement_processor/helpers.rs` |
| `create_namespace_for_inlined_submodule`     | 3094-3231 | High       | Move to `submodules.rs`                  |
| `renamed_symbol_exists`                      | 3233-3257 | Medium     | Move to `submodules.rs`                  |
| `process_wildcard_import`                    | 3259-3472 | Very High  | Move to `import_analysis.rs`             |
| `symbol_comes_from_wrapper_module`           | 3474-3556 | High       | Move to `import_analysis.rs`             |

**Strategy**: Keep general-purpose utilities in `module_transformer.rs`, move phase-specific helpers into phase modules.

---

## Appendix B: Dependency Graph

```
┌─────────────────────┐
│ InitFunctionBuilder │
└──────────┬──────────┘
           │
           ├──> Phase 1: Initialization
           │      └──> Uses: globals lifting
           │
           ├──> Phase 2: Import Analysis
           │      └──> Uses: process_wildcard_import, symbol_comes_from_wrapper_module
           │
           ├──> Phase 3: Import Transformation
           │      └──> Uses: RecursiveImportTransformer
           │
           ├──> Phase 4: Wrapper Symbol Setup
           │
           ├──> Phase 5: Wildcard Import Processing
           │
           ├──> Phase 6: Body Preparation
           │      └──> Uses: transform_expr_for_builtin_shadowing
           │
           ├──> Phase 7: Wrapper Globals Collection
           │      └──> Uses: WrapperGlobalCollector visitor
           │
           ├──> Phase 8: Statement Processing
           │      └──> Uses: Multiple statement processors (trait-based)
           │
           ├──> Phase 9: Submodule Handling
           │      └──> Uses: create_namespace_for_inlined_submodule, renamed_symbol_exists
           │
           ├──> Phase 10: Final Cleanup
           │
           ├──> Phase 11: Globals/Locals Transformation
           │      └──> Uses: transform_globals_in_stmt, transform_locals_in_stmt
           │
           └──> Phase 12: Finalization
```

---

## Appendix C: Statement Processor Detailed Design

The statement processor phase is the most complex. Here's detailed design:

### Trait Definition

```rust
pub trait StatementProcessor {
    /// Check if this processor can handle the statement
    fn can_process(&self, stmt: &Stmt) -> bool;

    /// Process the statement and update state
    fn process(
        &self,
        stmt: Stmt,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError>;
}
```

### Chain of Responsibility Pattern

Processors are tried in order until one accepts the statement:

```rust
for processor in &self.processors {
    if processor.can_process(&stmt) {
        return processor.process(stmt, bundler, ctx, state);
    }
}
```

### Extensibility

To add new statement handling:

1. Create new processor implementing `StatementProcessor`
2. Add to `StatementProcessingPhase::new()` list
3. No changes to existing processors needed

---

## Conclusion

This refactoring transforms a 1,511-line monolithic function into a well-structured, maintainable, and testable system of 12-13 cohesive phases. While ambitious, the systematic approach with incremental extraction and comprehensive testing minimizes risk.

The result will be:

- **Easier to understand**: Each phase has a clear purpose
- **Easier to test**: Unit tests for individual phases
- **Easier to modify**: Changes localized to specific phases
- **Easier to optimize**: Profile and improve hot phases
- **Easier to extend**: Add new behavior via traits

The investment in refactoring will pay dividends in reduced maintenance burden and increased development velocity for future enhancements to cribo's bundling system.
