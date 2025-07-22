# Implementation Spec: Splitting the Code Generator

## 1. Introduction

This document provides a detailed implementation specification for refactoring the `crates/cribo/src/code_generator/bundler.rs` file. The goal is to break down the monolithic `bundler.rs` into smaller, functionally-cohesive modules, as outlined in the [further-split proposal](./further-split.md).

This refactoring will improve maintainability, testability, and developer velocity by creating a more organized and modular codebase.

## 2. General Guidelines & Refactoring Pattern

The refactoring will be conducted in phases. Each phase must result in a compilable state with all tests passing and no new clippy warnings.

- **Core Struct**: The `HybridStaticBundler` struct will remain in `bundler.rs`. It will serve as the central state holder and orchestrator.
- **Method to Function Conversion**: Methods currently in `impl HybridStaticBundler` will be moved to new modules as free-standing functions.
- **State Access**: To access the bundler's state, the new functions will accept a `&HybridStaticBundler` or `&mut HybridStaticBundler` as their first argument.

**Example Refactoring Pattern:**

**Before (in `bundler.rs`):**

```rust
impl HybridStaticBundler {
    fn my_helper_method(&self, arg: &str) -> bool {
        // logic using self.fields
    }
}
// Call site: self.my_helper_method("test")
```

**After (in `new_module.rs`):**

```rust
// In new_module.rs
use super::bundler::HybridStaticBundler;

pub(super) fn my_helper_function(bundler: &HybridStaticBundler, arg: &str) -> bool {
    // logic using bundler.fields
}
```

**After (in `bundler.rs`):**

```rust
// Call site: new_module::my_helper_function(self, "test")
```

- **Module Visibility**: New modules will be declared in `crates/cribo/src/code_generator/mod.rs`. The functions within them should be `pub(super)` to be accessible from `bundler.rs` but not outside the `code_generator` module.

## 3. Phased Implementation Plan

This plan follows the "Concrete Implementation Plan" from the proposal.

### Phase 1: Module Registry (`module_registry.rs`)

- **Status**: ✅ **COMPLETED**
- **Summary**: The `module_registry.rs` file was created, and functions related to module naming, registration, and cache generation were moved into it. This reduced the token count in `bundler.rs` by over 4,000 tokens and served as a successful proof-of-concept for this refactoring effort.

### Phase 2: Expression Handlers (`expression_handlers.rs`)

- **File**: `crates/cribo/src/code_generator/expression_handlers.rs`
- **Responsibility**: Encapsulates all logic for creating, analyzing, and transforming `rustpython_parser::ast::Expr` nodes. This is a foundational module as expressions are a core part of many other operations.
- **Functions to Move**:
  - **Expression Transformation**:
    - `transform_expr_for_lifted_globals`
    - `transform_fstring_for_lifted_globals`
    - `transform_fstring_expression`
    - `resolve_import_aliases_in_expr`
    - `rewrite_aliases_in_expr`
    - `rewrite_aliases_in_expr_impl` (will become a private helper in the new module)
  - **Expression Analysis**:
    - `expr_uses_importlib`
    - `extract_string_list_from_expr`
    - `extract_attribute_path`
    - `expr_equals`
    - `collect_vars_in_expr`
    - `expr_to_dotted_name` (standalone function)
  - **Expression Creation**:
    - `create_string_literal`
    - `create_namespace_attribute`
    - `create_dotted_attribute_assignment`

### Phase 3: Import Deduplication (`import_deduplicator.rs`)

- **File**: `crates/cribo/src/code_generator/import_deduplicator.rs`
- **Responsibility**: Manages the logic for finding and removing duplicate or unused imports, and other import-related cleanup tasks.
- **Functions to Move**:
  - **Import Deduplication**:
    - `deduplicate_deferred_imports_with_existing`
    - `is_duplicate_import_from`
    - `is_duplicate_import`
    - `import_names_match`
  - **Import Cleanup**:
    - `should_remove_import_stmt`
    - `trim_unused_imports_from_modules`
    - `remove_unused_importlib`
    - `stmt_uses_importlib`
    - `log_unused_imports_details`
  - **Import Utilities**:
    - `is_hoisted_import`
    - `is_import_in_hoisted_stdlib`
    - `add_hoisted_imports`
    - `add_stdlib_import`

### Phase 4: Consolidate AST Analysis in `visitors/`

- **Files**: `crates/cribo/src/visitors/*`
- **Responsibility**: This phase cancels the creation of `symbol_collector.rs` and instead consolidates all AST traversal and data collection logic within the existing `visitors/` directory. This avoids redundant logic and creates a clear separation between data collection (visitors) and data analysis (other modules).

- **Actions**:
  1. **Do NOT create `symbol_collector.rs`**.
  2. **Create a new visitor: `visitors/symbol_visitor.rs`**. This visitor will be responsible for collecting information about symbol definitions, references, and scopes. It will replace the need for the following functions from `bundler.rs`:
     - `collect_global_symbols`
     - `collect_module_renames`
     - `collect_referenced_vars`
     - `collect_vars_in_stmt`
     - `extract_all_exports`
  3. **Enhance `visitors/import_discovery.rs`** to be the sole source of import collection, replacing:
     - `collect_direct_imports` and its variants.
     - `collect_namespace_imports`
     - `collect_unique_imports`
     - `collect_future_imports_from_ast`
  4. **Re-assign Analysis Functions**: The remaining functions originally planned for `symbol_collector.rs` are analytical, not for collection. They will be moved to `dependency_analyzer.rs` in Phase 6, as they operate on the data collected by the visitors. These include:
     - `find_symbol_module`
     - `should_export_symbol`
     - `should_inline_symbol`
     - `is_self_referential_assignment`
     - `extract_simple_assign_target`
     - `assignment_references_namespace_module`

### Phase 5: Namespace Management (`namespace_manager.rs`)

- **File**: `crates/cribo/src/code_generator/namespace_manager.rs`
- **Responsibility**: Handles the creation and management of Python namespace objects, which are used to simulate module structures in the bundled output.
- **Functions to Move**:
  - **Namespace Creation**:
    - `create_namespace_statements`
    - `create_namespace_with_name`
    - `create_namespace_for_inlined_module_static`
    - `create_namespace_module`
    - `ensure_namespace_exists`
    - `generate_module_namespace_class`
  - **Namespace Attributes**:
    - `create_namespace_attribute` (already planned for `expression_handlers.rs`, should be moved here instead or shared)
    - `create_dotted_attribute_assignment` (same as above)
    - `generate_submodule_attributes_with_exclusions`
  - **Namespace Analysis**:
    - `identify_required_namespaces`
    - `find_matching_module_name_namespace`
    - `transform_namespace_package_imports`

### Phase 6: Dependency Analysis (`dependency_analyzer.rs`) & `cribo_graph.rs` Refinement

- **File**: `crates/cribo/src/code_generator/dependency_analyzer.rs` (New)
- **File**: `crates/cribo/src/cribo_graph.rs` (Refactored)
- **Responsibility**: This phase has two parts:
  1. Create a new `dependency_analyzer.rs` module to house all high-level, bundler-specific analysis logic.
  2. Refactor the existing `cribo_graph.rs` to be a pure data structure module, moving all high-level analysis logic out of it and into the new `dependency_analyzer.rs`.

- **`dependency_analyzer.rs` - Functions to Move/Create**:
  - **From `bundler.rs`**:
    - `build_symbol_dependency_graph`
    - `detect_hard_dependencies`
    - `sort_wrapper_modules_by_dependencies`
    - `sort_wrapped_modules_by_dependencies`
    - `filter_exports_by_tree_shaking`
    - `is_package_init_reexport`
    - `find_directly_imported_modules`
    - `find_namespace_imported_modules`
  - **From `cribo_graph.rs`**:
    - `analyze_circular_dependencies(graph: &CriboGraph) -> CircularDependencyAnalysis`
    - `find_unused_imports(module_graph: &ModuleDepGraph, is_init_py: bool) -> Vec<UnusedImportInfo>`
    - All private helpers associated with the above functions.

- **`cribo_graph.rs` - Refactoring**:
  - **Keep**: All `struct` definitions (`CriboGraph`, `ModuleDepGraph`, etc.) and core, generic graph operations (`add_module`, `add_module_dependency`, `topological_sort`, `find_strongly_connected_components`).
  - **Remove**: The high-level analysis functions listed above that are being moved to `dependency_analyzer.rs`. The `CriboGraph` will become a data container, not an analyzer.

### Phase 7: Enhance Existing Modules & Final Cleanup

- **Responsibility**: Move the remaining transformation and rewriting logic out of `bundler.rs` and into the existing `module_transformer.rs` and `import_transformer.rs` files. This is the final step to slim down `bundler.rs` to a pure orchestrator.
- **`module_transformer.rs` Enhancements**:
  - Move remaining `transform_*` methods related to module-level transformations.
  - Move `process_*` methods (`process_wrapper_module_globals`, `process_entry_module_statement`, etc.).
- **`import_transformer.rs` Enhancements**:
  - Move all `rewrite_import_*` methods.
  - Move `handle_imports_from_inlined_module*` methods.
  - Move remaining import resolution logic.
- **`bundler.rs` Final State**:
  - Should primarily contain `bundle_modules`, `inline_module`, and high-level orchestration logic that calls out to the new helper modules.

## 4. Validation

For each phase, the following steps must be completed to ensure the refactoring is successful and non-breaking:

1. **Full Test Suite**: Run `cargo test --workspace` and ensure all tests pass.
2. **Linter Checks**: Run `cargo clippy --workspace --all-targets -- -D warnings` and ensure there are no new warnings.
3. **Manual Verification**: Bundle a few complex fixtures and manually inspect the output to ensure correctness.
4. **Token Count**: Measure the token count of `bundler.rs` to track progress toward the goal of ~20,000 tokens.

## 5. Expected Final Structure

The final structure of the `code_generator` module will be:

```
code_generator/
├── mod.rs
├── bundler.rs                # Orchestration only (~20,000 tokens)
├── module_transformer.rs     # Module-level AST transformations
├── import_transformer.rs     # Import rewriting
├── expression_handlers.rs    # Expression creation, analysis, transformation
├── namespace_manager.rs      # Namespace object management
├── module_registry.rs        # Module naming and registration
├── import_deduplicator.rs    # Import cleanup and deduplication
├── dependency_analyzer.rs    # High-level dependency & symbol analysis
├── circular_deps.rs          # (Unchanged)
├── globals.rs                # (Unchanged)
└── context.rs                # (Unchanged)
```

Note: This list does not include the `visitors/` directory, which is at `crates/cribo/src/visitors/` and will be enhanced as part of this refactoring. The `cribo_graph.rs` file will also be refactored but remains at its current location.
