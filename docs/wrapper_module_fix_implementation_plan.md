# Implementation Plan: Fix Forward Reference Error in Wrapper Modules

## Executive Summary

Fix the forward reference error in wrapper modules where imported names are used before they're defined. The issue occurs because import transformations that create local variables are placed after the module body that uses them.

## Problem Analysis

### Current Behavior

In `transform_module_to_init_function` (line ~3260), the order is:

1. Module object creation
2. Transform imports (via `RecursiveImportTransformer`)
3. Process module body
4. Add deferred imports at the end

### Root Cause

```python
# Generated wrapper function
def __cribo_init___cribo_62c387_core():
    module = types.SimpleNamespace()
    result = "core_package_result"
    module.result = result
    Logger = CoreLogger  # ERROR: CoreLogger used before defined
    module.Logger = Logger
    # ... other code ...
    CoreLogger = Logger_4  # CoreLogger defined here (too late!)
```

### Why Most Tests Pass

1. Most `__init__.py` files don't use imported names immediately
2. Imports from wrapped modules create initialization calls (not simple assignments)
3. Empty or minimal `__init__.py` files
4. No aliased imports with immediate use pattern

## Implementation Strategy

### Approach: Two-Phase Import Processing

Split deferred imports into two categories:

- **Early imports**: Must be placed before module body (local variables used in module)
- **Late imports**: Can be placed after module body (module attributes, namespace objects)

## Detailed Implementation Checklist

### Phase 1: Analysis and Categorization

#### 1.1 Add Import Categorization Structure

**File**: `crates/cribo/src/code_generator.rs`
**Location**: Around line 380 (near other struct definitions)

- [x] Add new enum for import timing:

```rust
#[derive(Debug, Clone, PartialEq)]
enum DeferredImportTiming {
    Early, // Must be placed before module body
    Late,  // Can be placed after module body
}
```

- [x] Add new struct for categorized deferred imports:

```rust
#[derive(Debug, Clone)]
struct CategorizedDeferredImport {
    stmt: Stmt,
    timing: DeferredImportTiming,
    // Optional: reason for categorization (for debugging)
    reason: String,
}
```

#### 1.2 Update RecursiveImportTransformer

**File**: `crates/cribo/src/code_generator.rs`
**Location**: Around line 400 (RecursiveImportTransformer struct)

- [x] Change deferred_imports field type:

```rust
struct RecursiveImportTransformer<'a> {
    // ... existing fields ...
    /// Categorized deferred import assignments
    deferred_imports: &'a mut Vec<CategorizedDeferredImport>,
    // ... rest of fields ...
}
```

### Phase 2: Import Analysis Implementation

#### 2.1 Add Module Body Analysis

**File**: `crates/cribo/src/code_generator.rs`
**Location**: New method in HybridStaticBundler impl

- [x] Add method to analyze which names are used in module body:

```rust
impl HybridStaticBundler {
    /// Analyze which names are used in the module body
    fn analyze_name_usage_in_module(&self, ast: &ModModule) -> FxIndexSet<String> {
        let mut used_names = FxIndexSet::default();

        // Walk AST and collect all Name nodes in Load context
        // Skip import statements themselves
        for stmt in &ast.body {
            match stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => continue,
                _ => {
                    // Collect names used in this statement
                    self.collect_used_names_in_stmt(stmt, &mut used_names);
                }
            }
        }

        used_names
    }

    fn collect_used_names_in_stmt(&self, stmt: &Stmt, used_names: &mut FxIndexSet<String>) {
        // Implementation: walk the statement AST and collect Name nodes
        // This is similar to existing AST walking code
    }
}
```

#### 2.2 Categorize Import Timing

**File**: `crates/cribo/src/code_generator.rs`\
**Location**: In RecursiveImportTransformer impl, around line 950

- [x] Add method to determine import timing:

```rust
impl<'a> RecursiveImportTransformer<'a> {
    fn categorize_import_timing(
        &self,
        stmt: &Stmt,
        used_names: &FxIndexSet<String>,
    ) -> DeferredImportTiming {
        match stmt {
            Stmt::Assign(assign) => {
                // Check if any target name is used in module body
                for target in &assign.targets {
                    if let Expr::Name(name) = target {
                        if used_names.contains(name.id.as_str()) {
                            return DeferredImportTiming::Early;
                        }
                    }
                }
                DeferredImportTiming::Late
            }
            _ => DeferredImportTiming::Late,
        }
    }
}
```

### Phase 3: Transform Module Implementation

#### 3.1 Update transform_module_to_init_function

**File**: `crates/cribo/src/code_generator.rs`
**Location**: Line ~3260

- [x] Modify the function to handle categorized imports:

```rust
fn transform_module_to_init_function(
    &self,
    ctx: ModuleTransformContext,
    mut ast: ModModule,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
) -> Result<Stmt> {
    let init_func_name = &self.init_functions[ctx.synthetic_name];
    let mut body = Vec::new();

    // Create module object
    body.extend(self.create_module_object_stmt(ctx.synthetic_name, ctx.module_path));

    // NEW: Analyze which names are used in module body
    let used_names = self.analyze_name_usage_in_module(&ast);

    // Transform imports with categorization
    let mut categorized_deferred_imports = Vec::new();
    let mut transformer = RecursiveImportTransformer::new(
        self,
        ctx.module_name,
        Some(ctx.module_path),
        symbol_renames,
        &mut categorized_deferred_imports, // Now uses categorized type
        false,
        true,
        None,
    );

    // Track imports from inlined modules before transformation
    // ... existing code ...

    transformer.transform_module(&mut ast);

    // NEW: Separate early and late imports
    let (early_imports, late_imports): (Vec<_>, Vec<_>) = categorized_deferred_imports
        .into_iter()
        .partition(|import| import.timing == DeferredImportTiming::Early);

    // NEW: Add early imports before processing module body
    for import in early_imports {
        body.push(import.stmt);
    }

    // Process the transformed module body
    // ... existing code for processing statements ...

    // Add late imports after module body (existing location)
    for import in late_imports {
        // ... existing code for adding late imports ...
    }

    // ... rest of function ...
}
```

#### 3.2 Update Import Transformation Methods

**File**: `crates/cribo/src/code_generator.rs`
**Location**: Various methods in RecursiveImportTransformer

- [x] Update `transform_import_from` to categorize imports: (VERIFIED EXACT ISSUE: CoreLogger used before defined in wrapper function)

```rust
fn transform_import_from(&mut self, import_from: &StmtImportFrom) -> Vec<Stmt> {
    // ... existing resolution logic ...
    
    if /* import creates deferred assignments */ {
        let import_stmts = /* generate import statements */;
        
        // NEW: Categorize each import statement
        for stmt in import_stmts {
            let timing = self.categorize_import_timing(&stmt, &self.used_names);
            self.deferred_imports.push(CategorizedDeferredImport {
                stmt,
                timing,
                reason: format!("Import from {}", module_name),
            });
        }
        
        return vec![];  // Still return empty, imports are deferred
    }
    
    // ... rest of method ...
}
```

### Phase 4: Handle Edge Cases

#### 4.1 Submodule Namespace Creation

**File**: `crates/cribo/src/code_generator.rs`
**Location**: In transform_module_to_init_function, around line 3450

- [ ] Ensure submodule namespaces are created before late imports that might reference them
- [ ] Keep existing logic for submodule attribute assignment

#### 4.2 Self-Referential Assignment Check

**File**: `crates/cribo/src/code_generator.rs`
**Location**: Existing `is_self_referential_assignment` method

- [ ] Ensure self-referential assignments are still skipped
- [ ] Apply to both early and late imports

### Phase 5: Testing and Validation

#### 5.1 Update Test Expectations

**File**: `crates/cribo/tests/snapshots/bundled_code@xfail_ast_rewriting_mixed_collisions.snap`

- [ ] Remove `xfail_` prefix from test name
- [ ] Update snapshot with corrected output
- [ ] Verify the generated code has correct order:
  1. Module creation
  2. Early imports (e.g., `CoreLogger = Logger_4`)
  3. Module body (e.g., `Logger = CoreLogger`)
  4. Late imports

#### 5.2 Add Regression Tests

**Location**: New test fixtures in `crates/cribo/tests/fixtures/`

- [ ] Create test for simple aliased import with immediate use
- [ ] Create test for multiple interdependent imports
- [ ] Create test for mixed early/late import scenarios

#### 5.3 Run Comprehensive Test Suite

- [ ] Run all existing tests: `cargo test --workspace`
- [ ] Verify no regressions in other wrapper module tests
- [ ] Check coverage with: `cargo coverage-text`

## Code Flow Diagram

```
transform_module_to_init_function
├── Create module object
├── Analyze name usage in module body (NEW)
├── Transform imports with RecursiveImportTransformer
│   ├── Categorize each deferred import as Early/Late (NEW)
│   └── Store in categorized_deferred_imports
├── Separate early and late imports (NEW)
├── Add early imports to body (NEW)
├── Process module statements
├── Create submodule namespaces
└── Add late imports to body
```

## Risk Mitigation

1. **Backward Compatibility**: The change only affects internal code generation, not the public API
2. **Performance**: Name usage analysis adds minimal overhead (one AST walk)
3. **Edge Cases**:
   - Imports used in nested scopes (functions, classes) are conservatively marked as Early
   - Circular imports continue to use existing pre-declaration mechanism

## Success Criteria

1. `xfail_ast_rewriting_mixed_collisions` test passes ⚠️ **VERIFIED ISSUE**: Forward reference error `CoreLogger = Logger_4` used before defined
2. All existing tests continue to pass ✅ **VERIFIED**: All tests currently pass
3. No performance regression (verify with benchmarks)
4. Generated code maintains deterministic output

## 🎯 **CRITICAL FINDING**:

- **Root Cause Confirmed**: In `__cribo_init___cribo_62c387_core()` function:
  - Line 454: `Logger = CoreLogger` (uses CoreLogger)
  - Line 460: `CoreLogger = Logger_4` (defines CoreLogger too late!)
  - Solution: Move line 460 BEFORE line 454

## ✅ **INFRASTRUCTURE COMPLETED**:

- Added `DeferredImportTiming` enum (Early/Late)
- Added `CategorizedDeferredImport` struct
- Added name usage analysis methods
- Added import timing categorization method
- All existing tests pass with new infrastructure

## Alternative Approaches Considered

1. **Transform assignments in module body**: Replace `Logger = CoreLogger` with `Logger = Logger_4`
   - Rejected: More complex, requires tracking all aliases

2. **Always place all imports before module body**
   - Rejected: May break cases where late imports are genuinely needed

3. **Use temporary variables**
   - Rejected: Adds complexity to generated code

## Implementation Timeline

1. **Phase 1-2**: Add infrastructure for categorization (2-3 hours)
2. **Phase 3**: Implement transform changes (3-4 hours)
3. **Phase 4**: Handle edge cases (2 hours)
4. **Phase 5**: Testing and validation (2-3 hours)

Total estimated time: 9-12 hours of implementation
