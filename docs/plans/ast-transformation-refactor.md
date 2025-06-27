# AST Transformation Refactoring Plan

## Objective

Refactor the current AST transformation pipeline to implement the Transformation Plan architecture, eliminating pre-processing mutations and establishing clean architectural boundaries.

## Current State

### Problems

1. `stdlib_normalization::normalize_stdlib_imports()` mutates ASTs after analysis
2. `ImportRewriter::rewrite()` mutates ASTs for circular dependency handling
3. Graph metadata becomes stale after transformations
4. Orchestrator violates architectural boundaries by performing transformations

### Code Locations

- `crates/cribo/src/orchestrator.rs`: Lines calling normalization and rewriter
- `crates/cribo/src/stdlib_normalization.rs`: AST mutation logic
- `crates/cribo/src/import_rewriter.rs`: Circular dep rewriting
- `crates/cribo/src/bundle_compiler/compiler.rs`: Current compilation logic

## Implementation Steps

### Step 1: Define Transformation Infrastructure ✓

**Files to create/modify:**

- `crates/cribo/src/transformations.rs` (new file) ✓

```rust
use ruff_python_ast::Stmt;
use ruff_text_size::TextRange;
use rustc_hash::FxHashMap;

use crate::cribo_graph::ItemId;

#[derive(Debug, Clone)]
pub enum TransformationMetadata {
    /// Stdlib import normalization
    StdlibImportRewrite {
        canonical_module: String,
        imports: Vec<ImportTransform>,
    },

    /// Symbol usage rewriting
    SymbolRewrite {
        rewrites: FxHashMap<TextRange, SymbolTransform>,
    },

    /// Circular dependency import relocation
    CircularDepImportMove {
        target_scope: ItemId,
        original_location: TextRange,
    },

    /// Import removal (unused, type-only, or bundled)
    RemoveImport { reason: RemovalReason },

    /// Partial import removal - some symbols unused
    PartialImportRemoval {
        remaining_symbols: Vec<(String, Option<String>)>,
        removed_symbols: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub enum RemovalReason {
    Unused,   // Never referenced
    TypeOnly, // Only in type annotations
    Bundled,  // First-party, will be inlined
}

#[derive(Debug, Clone)]
pub struct ImportTransform {
    pub original_name: String,
    pub canonical_name: String,
    pub is_aliased: bool,
}

#[derive(Debug, Clone)]
pub struct SymbolTransform {
    pub new_name: String,
    pub requires_attribute: bool, // true if changing Name to Attribute
}
```

### Step 2: Extend AnalysisResults ✓

**File:** `crates/cribo/src/analysis/mod.rs` ✓

```rust
pub struct AnalysisResults {
    // ... existing fields ...
    /// The dependency graph (move from being passed separately)
    pub graph: CriboGraph,

    /// Entry module ID for the bundle
    pub entry_module: ModuleId,

    /// Module registry
    pub module_registry: ModuleRegistry,

    /// Transformation plan for items requiring changes
    pub transformations: FxHashMap<ItemId, Vec<TransformationMetadata>>,
}
```

**Note**: This change also requires moving graph, entry_module, and module_registry into AnalysisResults instead of passing them separately to BundleCompiler.

### Step 3: Create Transformation Detection ✓

**New file:** `crates/cribo/src/transformation_detector.rs` ✓

This module will analyze the SemanticModel during the analysis phase and populate the transformations map.

Key responsibilities:

1. Detect stdlib imports with aliases
2. Track symbol usage that needs rewriting
3. Identify imports that need relocation for circular deps
4. Mark unused imports for removal
5. Identify type-only imports (when type stripping is enabled)
6. Mark first-party imports that will be bundled
7. Detect namespace collisions for inlining and add preemptive renames
8. De-sugar star imports into specific transformations based on usage

### Step 4: Update Analysis Pipeline ✓

**File:** `crates/cribo/src/orchestrator.rs` ✓
**File:** `crates/cribo/src/analysis/pipeline.rs` ✓

1. Remove calls to `normalize_stdlib_imports` and `ImportRewriter::rewrite` ✓
2. Add transformation detection to the analysis pipeline ✓
3. Pass transformations to BundleCompiler ✓

### Step 5: Enhance BundleCompiler ✓

**File:** `crates/cribo/src/bundle_compiler/compiler.rs` ✓

Add state management and transformation execution:

```rust
pub struct BundleCompiler {
    // ... existing fields ...
    hoisted_imports: FxHashSet<String>, // Track global imports
}

struct CompilationContext<'a> {
    compiler: &'a mut BundleCompiler,
    destination_module_id: ModuleId,
    symbol_resolution_map: FxHashMap<String, String>,
    processed_items: FxHashSet<ItemId>,
}

impl<'a> BundleCompiler<'a> {
    fn process_item(&mut self, item_id: ItemId) -> Option<ExecutionStep> {
        let transformations = self.get_sorted_transformations(item_id);

        // Process transformations in priority order
        for transformation in transformations {
            match transformation {
                RemoveImport { .. } => return None,
                // ... handle other transformations
            }
        }

        // Fast path for clean items
        Some(ExecutionStep::CopyStatement { .. })
    }
}
```

### Step 6: Implement AST Builders ✓ (Partial)

**File:** `crates/cribo/src/ast_builder.rs` ✓

Implemented builders:

- ✓ `from_import_specific()`: Creates partial from-import statements
- ✓ Various basic AST builders (import, assign, etc.)

Still needed:

- `build_normalized_import()`: Creates canonical import statements for stdlib
- `transform_symbol_usage()`: Rewrites Names to Attributes
- `relocate_import()`: Handles circular dep import movement

## Implementation Approach

Since we're doing a complete architectural change in a single branch:

### Direct Implementation

1. Create new transformation infrastructure
2. Implement transformation detection in analysis
3. Update BundleCompiler to execute transformations
4. Remove ALL old AST mutation code
5. Update all affected tests

### No Parallel Paths

- No feature flags or compatibility modes
- Direct replacement of the mutation-based system
- All changes in a single coherent commit set

## Testing Plan

### Unit Tests

1. Transformation detection accuracy
2. AST builder correctness
3. Compiler transformation execution

### Integration Tests

1. Full bundling with transformations
2. Circular dependency handling
3. Stdlib normalization completeness

### Snapshot Tests

All existing bundling snapshots must remain unchanged.

## Quality Assurance

Since this is a direct replacement:

1. All existing tests must pass unchanged
2. Snapshot tests validate identical output
3. No degradation in performance
4. Clean architectural boundaries verified

## Success Criteria

1. No AST mutations outside BundleCompiler
2. All bundling tests pass unchanged
3. Performance neutral or better
4. Clean architectural boundaries
5. Improved testability of each phase

## Implementation Order

1. Infrastructure (TransformationMetadata, updated AnalysisResults)
2. Transformation detection logic
3. BundleCompiler transformation execution
4. Remove old mutation code
5. Update documentation

## Implementation Notes

### Critical First Step

Change function signatures from mutating to analytical:

```rust
// OLD - violates architecture
pub fn normalize_stdlib_imports(ast: &mut ModModule) -> NormalizationResult

// NEW - enforces architecture  
pub fn analyze_stdlib_imports(ast: &ModModule) -> Vec<TransformationMetadata>
```

### Error Handling Policy

- Analysis phase uses debug_assert! to catch logic errors
- BundleCompiler fails hard on internal errors (e.g., invalid TextRange)
- No silent recovery that could produce incorrect code

### Transformation Conflicts

- Handled by fixed priority order in BundleCompiler
- Analysis phase should not generate direct conflicts
- If conflicts exist, priority determines resolution

## Implementation Status (Updated 2025-06-27)

### Completed ✓

1. **Transformation Infrastructure**: Created `TransformationMetadata` enum with all variants
2. **Extended AnalysisResults**: Added transformations map (though graph/registry still passed separately)
3. **Transformation Detection**: Implemented `transformation_detector.rs` with:
   - Unused import detection (including tree-shaking integration)
   - First-party import detection for bundling
   - Stdlib import normalization detection
   - Importlib static call detection
4. **Analysis Pipeline**: Updated to populate transformations during analysis
5. **BundleCompiler**: Enhanced to execute transformations:
   - RemoveImport transformation
   - PartialImportRemoval transformation
   - Proper handling of dotted imports and namespace creation
6. **Removed Mutation Calls**: No longer calling `normalize_stdlib_imports` or `ImportRewriter::rewrite` from orchestrator

### Still Needed

1. **Function Signature Changes**: Old mutation functions still exist but unused:
   - `stdlib_normalization::normalize_stdlib_imports` still mutates AST
   - `import_rewriter::ImportRewriter::rewrite` still mutates AST
   - Should be converted to analytical functions or removed
2. **Missing AST Builders**:
   - `build_normalized_import()` for stdlib normalization
   - `transform_symbol_usage()` for symbol rewrites
   - `relocate_import()` for circular dependency handling
3. **Missing Transformations**:
   - StdlibImportRewrite execution
   - SymbolRewrite execution
   - CircularDepImportMove execution
4. **Partial Import Removal**: Currently removes entire from-import, need per-symbol removal

### Test Results

- Multiple xfail tests now passing and renamed
- Core transformation architecture working correctly
- Some tests still failing due to missing transformation implementations

## Next Steps

1. Complete remaining transformation implementations
2. Convert or remove old mutation functions
3. Implement missing AST builders
4. Fix partial import removal to handle individual symbols
