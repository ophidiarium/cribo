# Code Generator Split - Missing Updates

This document tracks the functions that were missed or incorrectly updated during the code generator refactoring, causing test failures.

## Context

During the refactoring to split `code_generator.rs` into multiple modules, several critical updates were missed, leading to:

- Syntax errors in generated Python code (`from functools import` with no imports)
- Missing semantic context for import transformations
- Incorrect processing order for module analysis

## Functions Requiring Fixes

### In `crates/cribo/src/code_generator/bundler.rs`

- [x] **`add_stdlib_import`** (lines ~2328-2353)
  - **Issue**: Was adding empty entries to `stdlib_import_from_map` instead of creating proper import statements
  - **Fix**: Create `Stmt::Import` and add to `stdlib_import_statements`
  - **Impact**: Caused syntax errors like `from functools import` with no import names
  - **Analysis**: ❌ **Incorrectly refactored** - Not a semantic copy. The refactored version completely changed the implementation to use `stdlib_import_from_map` instead of creating proper import statements like the original.
  - **Status**: ✅ Fixed - Restored to original implementation

- [ ] **`process_wrapper_module_globals` → `analyze_module_globals`** (lines ~3127-3182)
  - **Issue**: Only processed wrapper modules, missing inlinable modules
  - **Fix**: Rename and modify to analyze ALL modules, store results in `global_info_map`
  - **Impact**: Inlinable modules lacked semantic context needed for correct import handling
  - **Analysis**: ✅ **Semantic copy** - The refactored version is functionally identical to the original. Both:
    - Get module from graph by name
    - Get module_id from the module
    - Call semantic_bundler.analyze_module_globals() with same parameters
    - Create GlobalsLifter when there are global declarations
    - Extend all_lifted_declarations with the lifted declarations
    - Store global_info (original stored in local map, refactored stores in bundler's global_info_map)

- [ ] **`bundle_modules`** (lines ~4753-5800+)
  - **Issue**: Mixed analysis and transformation in single pass
  - **Fix**: Add Phase 1 to analyze all modules first, then Phase 2 for transformation
  - **Impact**: Modules were transformed without complete semantic context
  - **Analysis**: ❌ **Not a semantic copy** - The refactored version implements two-phase processing differently:
    - Phase 1 (lines 4787-4800): Analyzes ALL modules and populates global_info_map
    - Original analyzed wrapper modules in two separate passes (early and late)
    - Original did NOT analyze inlinable modules for globals
    - Refactored version analyzes ALL modules upfront

- [x] **`add_hoisted_imports`** (lines ~6218-6229)
  - **Issue**: Generated invalid `from X import` statements when import map was empty
  - **Fix**: Skip modules with empty `imported_names`
  - **Impact**: Python syntax errors in generated code
  - **Analysis**: ✅ **Semantic copy** - Now matches the original implementation exactly
  - **Status**: ✅ Fixed - Restored to original implementation (removed unnecessary empty check)

- [ ] **`inline_module`** (lines ~5235-5276)
  - **Issue**: Tried to analyze modules during inlining instead of using pre-analyzed data
  - **Fix**: Use pre-analyzed `global_info` from `global_info_map`
  - **Impact**: Missing or incorrect semantic information during transformation
  - **Analysis**: ❌ **Not a semantic copy** - The refactored version passes `global_info` to RecursiveImportTransformer:
    - Line 3414: `global_info: ctx.global_info.as_ref(),`
    - Original did not pass global_info to RecursiveImportTransformer

### Struct Modifications

- [ ] **`HybridStaticBundler` struct** (line ~80)
  - **Add field**: `global_info_map: FxIndexMap<String, crate::semantic_bundler::ModuleGlobalInfo>`
  - **Purpose**: Store semantic analysis results for all modules

### In `crates/cribo/src/code_generator/context.rs`

- [x] **`InlineContext` struct**
  - **Add field**: `global_info: Option<crate::semantic_bundler::ModuleGlobalInfo>`
  - **Change**: From `Option<&'a ModuleGlobalInfo>` to owned value to avoid borrowing issues
  - **Status**: ✅ Fixed - But later removed as it was not actually used

### In `crates/cribo/src/code_generator/import_transformer.rs`

- [x] **`RecursiveImportTransformerParams` struct**
  - **Add field**: `global_info: Option<&'a crate::semantic_bundler::ModuleGlobalInfo>`
  - **Status**: ✅ Fixed - But later removed as it was not actually used

- [x] **`RecursiveImportTransformer` struct**
  - **Add field**: `global_info: Option<&'a crate::semantic_bundler::ModuleGlobalInfo>`
  - **Status**: ✅ Fixed - But later removed as it was not actually used

## Root Cause

The fundamental issue was **NOT** about missing semantic analysis results in RecursiveImportTransformer (as evidenced by the unused `global_info` field). The actual root cause was the incorrect refactoring of `add_stdlib_import`:

1. The original `add_stdlib_import` created proper `Stmt::Import` statements
2. The refactored version incorrectly added empty entries to `stdlib_import_from_map`
3. This caused `add_hoisted_imports` to generate invalid Python syntax like `from functools import` with no import names

The fix was simply restoring `add_stdlib_import` to its original implementation.

## Solution Summary

The primary fix was restoring `add_stdlib_import` to its original implementation. The two-phase processing that was added for analyzing all modules upfront may not have been necessary for fixing the immediate issue, but it represents a different approach than the original code which only analyzed wrapper modules.
