# Implementation Plan: Refactor bundle_modules Function

## Executive Summary

The `bundler.rs` file contains 7,672 lines of code, with the main `bundle_modules` function alone spanning 735 lines (lines 1546-2281). This plan describes how to refactor this monolithic structure into well-organized, maintainable modules.

## Goal

Break down the massive `bundler.rs` file into smaller, focused modules while maintaining all existing functionality and ensuring the code remains easy to understand and maintain.

## Current Structure Analysis

The `bundle_modules` function currently handles five major phases:

1. **Initialization** (lines 1548-1716): Sets up tree-shaking, tracks module access, detects entry modules, collects future imports, trims imports, and indexes ASTs.

2. **Module Classification** (lines 1718-1891): Detects import types, identifies circular dependencies, separates modules into inlinable vs wrapper categories, and collects exports.

3. **Semantic Analysis** (lines 2025-2188): Collects symbol renames, builds circular dependency graphs, and generates pre-declarations.

4. **Code Generation** (lines 2190-2259): Creates namespaces, processes wrapper modules, inlines modules, handles imports, and processes the entry module.

5. **Finalization** (lines 2260-2279): Hoists imports, fixes forward references, deduplicates code, and finalizes the AST.

## Refactoring Plan

### New File Structure

The refactoring will create ONE new file and enhance existing modules:

1. **bundler.rs** (~3,000 lines)
   - Bundler struct definition
   - Main orchestration logic
   - Module classification methods
   - Semantic analysis methods
   - Entry module processing
   - Finalization logic

2. **inliner.rs** (NEW - ~2,000 lines)
   - All module inlining functions
   - Class, assignment, and annotation inlining
   - Namespace management during inlining

3. **Existing modules** (enhanced with moved functions):
   - `module_transformer.rs`: Wrapper module processing
   - `expression_handlers.rs`: AST expression utilities
   - `namespace_manager.rs`: Namespace creation functions

### Function Migrations

#### To the new `inliner.rs`:

- `inline_module()` (lines 1058-1289)
- `inline_class()` (lines 6237-6373)
- `inline_assignment()` (lines 6376-6512)
- `inline_ann_assignment()` (lines 6515-6592)
- Main inlining loop logic (lines 2123-2188)
- Related helper functions

#### To `module_transformer.rs`:

- `transform_module_to_cache_init_function()` (lines 989-1019)
- `sort_wrapper_modules_by_dependencies()` (lines 950-986)
- New function: `process_wrapper_modules()`

#### To `expression_handlers.rs`:

- `extract_simple_assign_target()` (lines 4509-4516)
- `is_self_referential_assignment()` (lines 4519-4549)

#### Stay in `bundler.rs`:

- `find_directly_imported_modules()` (lines 4456-4463)
- `find_namespace_imported_modules()` (lines 4466-4478)
- `should_export_symbol()` (lines 4481-4506)
- `is_valid_python_identifier()` (lines 704-707)
- `module_accesses_imported_attributes()` (lines 711-831)

### New Methods in bundler.rs

```rust
impl Bundler {
    // Extract from lines 1718-1891
    fn classify_modules(&mut self, modules: &[(String, ModModule, PathBuf, String)], 
                       params: &BundleParams<'_>) -> Result<ClassificationResult>
    
    // Extract from lines 2025-2035
    fn collect_symbol_renames(&mut self, modules: &[(String, ModModule, PathBuf, String)], 
                             params: &BundleParams<'_>) -> Result<FxIndexMap<String, FxIndexMap<String, String>>>
    
    // Extract from lines 2037-2188
    fn generate_circular_predeclarations(&mut self, /*...*/) -> Result<Vec<Stmt>>
    
    // Extract from lines 1548-1716
    fn initialize_bundler(&mut self, params: &BundleParams<'_>) -> Result<()>
    
    // Extract import trimming and AST indexing logic
    fn prepare_modules(&mut self, params: &BundleParams<'_>) -> Result<Vec<(String, ModModule, PathBuf, String)>>
    
    // Keep existing, but may need minor adjustments
    fn process_entry_module(&mut self, /*...*/) -> Result<Vec<Stmt>>
    
    // Extract from lines 2260-2279
    fn finalize_module(&mut self, body: Vec<Stmt>, params: &BundleParams<'_>) -> Result<ModModule>
}
```

### New Public Functions

In `inliner.rs`:

```rust
pub fn inline_all_modules(
    bundler: &mut Bundler,
    inlinable_modules: Vec<(String, ModModule, PathBuf, String)>,
    module_exports_map: &FxIndexMap<String, Option<Vec<String>>>,
    symbol_renames: &mut FxIndexMap<String, FxIndexMap<String, String>>,
    params: &BundleParams<'_>,
) -> Result<InliningResult>

pub struct InliningResult {
    pub statements: Vec<Stmt>,
    pub deferred_imports: Vec<Stmt>,
}
```

In `module_transformer.rs`:

```rust
pub fn process_wrapper_modules(
    bundler: &mut Bundler,
    wrapper_modules: &[(String, ModModule, PathBuf, String)],
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    params: &BundleParams<'_>,
) -> Result<Vec<Stmt>>
```

In `namespace_manager.rs`:

```rust
pub fn create_required_namespaces(
    bundler: &mut Bundler,
    modules: &[(String, ModModule, PathBuf, String)],
    params: &BundleParams<'_>,
) -> Result<Vec<Stmt>>
```

### Refactored bundle_modules Function

```rust
pub fn bundle_modules(&mut self, params: &BundleParams<'_>) -> Result<ModModule> {
    // Phase 1: Initialization
    self.initialize_bundler(params)?;
    let mut modules = self.prepare_modules(params)?;

    // Phase 2: Module Classification
    let classification = self.classify_modules(&modules, params)?;

    // Phase 3: Semantic Analysis
    let mut symbol_renames = self.collect_symbol_renames(&modules, params)?;
    let predeclarations = self.generate_circular_predeclarations(
        &modules,
        &classification.inlinable_modules,
        &symbol_renames,
        params,
    )?;

    // Phase 4: Code Generation
    let mut final_body = Vec::new();

    final_body.extend(predeclarations);
    final_body.extend(namespace_manager::create_required_namespaces(
        self, &modules, params,
    )?);

    if !classification.wrapper_modules.is_empty() {
        let wrapper_stmts = module_transformer::process_wrapper_modules(
            self,
            &classification.wrapper_modules,
            &symbol_renames,
            params,
        )?;
        final_body.extend(wrapper_stmts);
    }

    let inlining_result = inliner::inline_all_modules(
        self,
        classification.inlinable_modules,
        &classification.module_exports_map,
        &mut symbol_renames,
        params,
    )?;

    final_body.extend(inlining_result.statements);

    let entry_stmts =
        self.process_entry_module(params, &symbol_renames, inlining_result.deferred_imports)?;
    final_body.extend(entry_stmts);

    // Phase 5: Finalization
    self.finalize_module(final_body, params)
}
```

## Implementation Steps

1. **Create `inliner.rs`**
   - Add module declaration to `code_generator/mod.rs`
   - Create the file with proper imports
   - Move the four inline_* functions
   - Extract the main inlining loop into `inline_all_modules`

2. **Move utility functions**
   - Move expression-related functions to `expression_handlers.rs`
   - Move module transformation functions to `module_transformer.rs`

3. **Extract bundler methods**
   - Create `classify_modules` method
   - Create `collect_symbol_renames` method
   - Create `generate_circular_predeclarations` method
   - Create `initialize_bundler` and `prepare_modules` methods
   - Create `finalize_module` method

4. **Enhance existing modules**
   - Add `process_wrapper_modules` to `module_transformer.rs`
   - Add `create_required_namespaces` to `namespace_manager.rs`

5. **Update bundle_modules**
   - Replace inline code with calls to new methods and functions
   - Ensure all imports are correct

6. **Test thoroughly**
   - Run `cargo test --workspace`
   - Run `cargo clippy --workspace --all-targets`
   - Verify snapshot tests still pass

## Success Criteria

- All existing tests pass
- No clippy warnings
- `bundler.rs` reduced from 7,672 to ~3,000 lines
- Code organization is clearer and more maintainable
- No change in bundling behavior or output

## Notes for Implementation

- When moving functions, update all imports and visibility modifiers
- Maintain the exact same logic - this is a refactoring, not a rewrite
- If a function needs access to Bundler fields, pass `&mut Bundler` as the first parameter
- Keep related functions together in the same module
- Add appropriate documentation comments to new public functions
