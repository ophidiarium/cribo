# Implementation Plan - Fix Import Issues

## Tasks

### 1. Merge ImportDiscoveryVisitor and SemanticImportAnalyzer

- [x] Add semantic analysis fields to `ImportDiscoveryVisitor`
  - [x] Add `imported_names: HashMap<String, String>` to track import aliases
  - [x] Add `name_usage: HashMap<String, Vec<ImportUsage>>` to track where names are used
  - [x] Add `semantic_bundler: Option<&SemanticBundler>` reference
  - [x] Add `module_id: Option<ModuleId>` for current module
- [x] Enhance `DiscoveredImport` struct to include semantic information
  - [x] Add `execution_contexts: HashSet<ExecutionContext>` field
  - [x] Add `is_used_in_init: bool` field
  - [x] Add `is_movable: bool` field computed from semantic analysis
- [x] Implement `visit_expr` in `ImportDiscoveryVisitor` to track name usage
  - [x] Track when imported names are used in expressions
  - [x] Track execution context (class init, function body, etc.)
- [x] Update `ImportLocation` to include execution context information
- [x] Migrate logic from `SemanticImportAnalyzer::analyze_module` to `ImportDiscoveryVisitor`
- [x] Update `orchestrator.rs` to pass `SemanticBundler` reference when available
- [x] Update `import_rewriter.rs` to use enhanced import information from single visitor
- [x] Remove `semantic_import_context.rs` file
- [x] Remove `SemanticImportAnalyzer` imports from other files

### 2. Fix Duplicate Module Initialization

- [x] Add `initialized_modules: FxIndexSet<String>` field to `HybridStaticBundler`
- [x] Initialize the field in `HybridStaticBundler::new()` or at start of `bundle_modules()`
- [x] Pass `initialized_modules` to `transform_bundled_import_from_multiple()` (changed to &mut self)
- [x] Update `transform_bundled_import_from_multiple()` to check global initialized set
- [x] Create utility method `get_init_function_name()` to avoid magic string duplication
- [x] Update all callers to use &mut self where needed
- [ ] Ensure all module init calls check the global tracker before adding init statements

### 3. Fix comprehensive_ast_rewrite Test Failure

- [ ] Debug the import alias resolution for `from .user import Logger as UserLogger`
- [ ] Trace how relative imports with aliases are resolved in bundled code
- [ ] Check if the issue is in `transform_import_from()` or import resolution
- [ ] Fix the import alias resolution to use correct source module
- [ ] Verify the fix resolves the AttributeError for `_log_process`

### 4. TYPE_CHECKING Import Tracking (Partially Implemented)

- [x] Add `is_type_checking_only` field to `DiscoveredImport`
- [x] Track when imports are within TYPE_CHECKING blocks in visitor
- [x] Add `ModuleDependencyInfo` struct to CriboGraph
- [x] Update graph edge type to store dependency metadata
- [x] Add `add_module_dependency_with_info` method
- [x] Add `is_type_checking_only_dependency` query method
- [ ] Refactor orchestrator to pass DiscoveredImport data to graph building
- [ ] Update dependency processing to use type checking info
- [ ] Use type checking info in requirements.txt generation

### 5. Testing and Validation

- [ ] Run `cargo test --workspace` to ensure all tests pass
- [ ] Run `cargo clippy --workspace --all-targets` to check for issues
- [ ] Verify no duplicate module initializations in test snapshots
- [ ] Verify comprehensive_ast_rewrite test passes
- [ ] Update test snapshots if needed with `cargo insta accept`
