# Further Code Generator Bundler Split Proposal

## Current State

The `bundler.rs` file currently contains **94,431 tokens** across **10,571 lines**. To reach our target of 25,000 tokens, we need to extract approximately **70,000 tokens** (74% of the current content).

## Analysis Summary

The bundler contains several distinct functional areas that can be cleanly separated:

### Method Categories by Prefix (most common):

- **create_*** (19 methods): AST node creation and statement generation
- **transform_*** (13 methods): AST transformation logic
- **collect_*** (13 methods): Data collection from modules
- **generate_*** (10 methods): Code generation for specific constructs
- **rewrite_*** (7 methods): Import rewriting logic
- **process_*** (5 methods): Module and statement processing

### Largest Methods by Line Count:

- `bundle_modules`: ~1,500+ lines (main orchestration)
- `inline_module`: ~600+ lines
- `rewrite_import_from`: ~500+ lines
- `transform_bundled_import_from_multiple_with_context`: ~400+ lines
- `handle_imports_from_inlined_module_with_context`: ~350+ lines

## Proposed Split Strategy

### 1. **Module Transformation Engine** (`module_transformer.rs`) - ~20,000 tokens

Extract all AST transformation logic for modules:

```rust
// Move these methods:
- transform_module_to_init_function()
- transform_module_to_cache_init_function()
- transform_ast_with_lifted_globals()
- transform_stmt_for_lifted_globals()
- transform_expr_for_lifted_globals()
- transform_nested_function_for_module_vars()
- transform_stmt_for_module_vars_with_locals()
- transform_expr_for_module_vars_with_locals()
- transform_function_body_for_lifted_globals()
- transform_fstring_for_lifted_globals()
- transform_fstring_expression()
- process_wrapper_module_globals()
- process_entry_module_statement()
- process_entry_module_function()
- process_entry_module_class()
- process_body_recursive()
```

**Rationale**: These methods form a cohesive unit for transforming module ASTs, handling lifted globals, and module variable transformations.

### 2. **Import Resolution & Rewriting** (`import_resolver.rs`) - ~18,000 tokens

Extract all import resolution and rewriting logic:

```rust
// Move these methods:
- resolve_relative_import()
- resolve_relative_import_with_context()
- rewrite_import_from()
- rewrite_import_with_renames()
- rewrite_import_in_stmt_multiple_with_full_context()
- transform_bundled_import_from_multiple_with_context()
- handle_imports_from_inlined_module()
- handle_imports_from_inlined_module_with_context()
- transform_namespace_package_imports()
- create_assignments_for_inlined_imports()
- collect_imports_from_module()
- collect_direct_imports()
- collect_direct_imports_recursive()
- collect_direct_relative_imports()
- find_directly_imported_modules()
- find_namespace_imported_modules()
- collect_namespace_imports()
- is_package_init_reexport()
- filter_exports_by_tree_shaking()
```

**Rationale**: Import handling is a complex, self-contained subsystem that deserves its own module.

### 3. **Namespace Management** (`namespace_manager.rs`) - ~12,000 tokens

Extract namespace creation management:

```rust
// Move these methods:
- identify_required_namespaces()
- create_namespace_statements()
- create_namespace_attribute()
- create_namespace_with_name()
- create_namespace_for_inlined_module_static()
- create_namespace_module()
- ensure_namespace_exists()
- create_dotted_attribute_assignment()
- generate_module_namespace_class()
- generate_submodule_attributes_with_exclusions()
```

**Note:** Module registry methods (`get_synthetic_module_name`, `generate_unique_name`, `sanitize_module_name_for_identifier`, `check_local_name_conflict`) have been extracted to `module_registry.rs`.

**Rationale**: Namespace management is a distinct concern that can be encapsulated separately.

### 4. **Symbol & Dependency Analysis** (`symbol_analyzer.rs`) - ~12,000 tokens

Extract symbol resolution and dependency tracking:

```rust
// Move these methods:
- collect_global_symbols()
- collect_module_renames()
- find_symbol_module()
- build_symbol_dependency_graph()
- detect_hard_dependencies()
- sort_wrapper_modules_by_dependencies()
- sort_wrapped_modules_by_dependencies()
- find_matching_module_name_namespace()
- should_export_symbol()
- should_inline_symbol()
- extract_all_exports()
- extract_string_list_from_expr()
- collect_referenced_vars()
- is_self_referential_assignment()
- extract_simple_assign_target()
- assignment_references_namespace_module()
```

**Rationale**: Symbol analysis and dependency tracking form a logical unit for understanding module relationships.

### 5. **Code Generation Helpers** (`codegen_helpers.rs`) - ~8,000 tokens

Extract low-level code generation utilities:

```rust
// Move these methods:
- create_string_literal()
- create_reassignment()
- create_module_attr_assignment()
- generate_module_cache_init()
- generate_module_cache_population()
- generate_sys_modules_sync()
- generate_registries_and_hook()
- generate_module_init_call()
- add_hoisted_imports()
- add_stdlib_import()
- is_safe_stdlib_module()
- is_hoisted_import()
- is_import_in_hoisted_stdlib()
- collect_future_imports_from_ast()
- collect_unique_imports()
```

**Rationale**: These are utility functions for generating specific Python constructs.

### 6. **Import Deduplication & Cleanup** (`import_cleanup.rs`) - ~7,000 tokens

Extract import deduplication and cleanup logic:

```rust
// Move these methods:
- deduplicate_deferred_imports_with_existing()
- is_duplicate_import_from()
- is_duplicate_import()
- import_names_match()
- should_remove_import_stmt()
- trim_unused_imports_from_modules()
- remove_unused_importlib()
- stmt_uses_importlib()
- expr_uses_importlib()
- log_unused_imports_details()
```

**Rationale**: Import cleanup and deduplication is a focused concern that can be isolated.

## Remaining in `bundler.rs` (~20,000 tokens)

After extraction, the main bundler will retain:

- Core `HybridStaticBundler` struct definition
- Main orchestration method `bundle_modules()`
- `inline_module()` method (core bundling logic)
- Field management and initialization
- High-level coordination between extracted modules

## Implementation Order

1. **Phase 1**: Extract `codegen_helpers.rs` and `import_cleanup.rs` (smallest, most independent)
2. **Phase 2**: Extract `namespace_manager.rs` (relatively independent)
3. **Phase 3**: Extract `symbol_analyzer.rs` (some interdependencies)
4. **Phase 4**: Extract `import_resolver.rs` (complex, many dependencies)
5. **Phase 5**: Extract `module_transformer.rs` (largest, most complex)

## Benefits

1. **Improved Maintainability**: Each module has a clear, focused responsibility
2. **Better Testing**: Smaller modules are easier to unit test
3. **Parallel Development**: Different aspects can be worked on independently
4. **Reduced Cognitive Load**: Developers can focus on one aspect at a time
5. **Performance**: Smaller compilation units may improve build times

## Module Dependencies

```
bundler.rs (orchestration)
    ├── module_transformer.rs (AST transformations) - PARTIALLY EXISTS
    ├── import_resolver.rs (import handling) - OVERLAPS WITH import_transformer.rs
    ├── namespace_manager.rs (namespace creation)
    ├── symbol_analyzer.rs (symbol/dependency analysis)
    ├── codegen_helpers.rs (code generation utilities)
    └── import_cleanup.rs (import deduplication)
```

## Success Metrics

- Final `bundler.rs` size: ~20,000 tokens (under 25,000 target)
- Each extracted module: 7,000-20,000 tokens (manageable size)
- Clear module boundaries with minimal circular dependencies
- All tests continue to pass after each extraction phase

## Concrete Implementation Plan

### Phase 1: Expression Handlers (~15,000 tokens)

**New file: `expression_handlers.rs`**

Extract all expression transformation and analysis methods:

```rust
// Expression transformation methods:
- transform_expr_for_lifted_globals()
- transform_fstring_for_lifted_globals()
- transform_fstring_expression()
- resolve_import_aliases_in_expr()
- rewrite_aliases_in_expr()
- rewrite_aliases_in_expr_impl() // standalone function

// Expression analysis methods:
- expr_uses_importlib()
- extract_string_list_from_expr()
- extract_attribute_path()
- expr_equals()
- collect_vars_in_expr()
- expr_to_dotted_name() // standalone function

// Expression creation methods:
- create_string_literal()
- create_namespace_attribute()
- create_dotted_attribute_assignment()
```

### Phase 2: Namespace Management (~18,000 tokens)

**New file: `namespace_manager.rs`**

Extract all namespace-related functionality:

```rust
// Namespace creation:
- create_namespace_statements()
- create_namespace_with_name()
- create_namespace_for_inlined_module_static()
- create_namespace_module()
- ensure_namespace_exists()
- generate_module_namespace_class()

// Namespace attributes:
- create_namespace_attribute()
- create_dotted_attribute_assignment()
- generate_submodule_attributes_with_exclusions()

// Namespace analysis:
- identify_required_namespaces()
- find_matching_module_name_namespace()
- transform_namespace_package_imports()
```

### Phase 3: Symbol Analysis & Collection (~12,000 tokens)

**New file: `symbol_collector.rs`**

Extract symbol collection and analysis:

```rust
// Symbol collection:
- collect_global_symbols()
- collect_module_renames()
- collect_referenced_vars()
- collect_vars_in_stmt()
- collect_function_globals()
- collect_direct_imports()
- collect_direct_imports_recursive()
- collect_direct_relative_imports()
- collect_namespace_imports()
- collect_unique_imports()
- collect_future_imports_from_ast()

// Symbol analysis:
- find_symbol_module()
- should_export_symbol()
- should_inline_symbol()
- extract_all_exports()
- is_self_referential_assignment()
- extract_simple_assign_target()
- assignment_references_namespace_module()
```

### Phase 4: Module Registry & Code Generation (~15,000 tokens) ✅ COMPLETED

**New file: `module_registry.rs`**

Extract module registration and code generation:

```rust
// Module registration:
- generate_module_cache_init() ✅
- generate_module_cache_population() ✅
- generate_sys_modules_sync() ✅
- generate_registries_and_hook() ✅
- generate_module_init_call() ✅

// Module naming:
- get_synthetic_module_name() ✅
- generate_unique_name() ✅
- sanitize_module_name_for_identifier() ✅
- check_local_name_conflict() ✅

// Assignment creation:
- create_module_attr_assignment() ✅
- create_reassignment() ✅
- create_assignments_for_inlined_imports() ✅
```

**Results:**

- Created `module_registry.rs` with 541 lines (4,754 tokens)
- Reduced `bundler.rs` from 94,431 to 90,226 tokens (-4,205 tokens)
- All tests passing, 0 clippy warnings
- Successfully removed unnecessary wrapper methods

### Phase 5: Import Deduplication (~8,000 tokens)

**New file: `import_deduplicator.rs`**

Extract import cleanup functionality:

```rust
// Import deduplication:
- deduplicate_deferred_imports_with_existing()
- is_duplicate_import_from()
- is_duplicate_import()
- import_names_match()

// Import cleanup:
- should_remove_import_stmt()
- trim_unused_imports_from_modules()
- remove_unused_importlib()
- stmt_uses_importlib()
- log_unused_imports_details()

// Import utilities:
- is_safe_stdlib_module()
- is_hoisted_import()
- is_import_in_hoisted_stdlib()
- add_hoisted_imports()
- add_stdlib_import()
```

### Phase 6: Dependency Analysis (~10,000 tokens)

**New file: `dependency_analyzer.rs`**

Extract dependency graph functionality:

```rust
// Dependency graph:
- build_symbol_dependency_graph()
- detect_hard_dependencies()
- sort_wrapper_modules_by_dependencies()
- sort_wrapped_modules_by_dependencies()

// Package analysis:
- is_package_init_reexport()
- filter_exports_by_tree_shaking()
- find_directly_imported_modules()
- find_namespace_imported_modules()
```

### Phase 7: Enhance Existing Files

**Enhance `module_transformer.rs`** (already exists with ~1,500 lines):

- Move remaining `transform_*` methods for modules
- Move `process_*` methods for module processing
- Keep module-specific transformations together

**Enhance `import_transformer.rs`** (already exists with ~1,900 lines):

- Move `rewrite_import_*` methods
- Move `handle_imports_from_inlined_module*` methods
- Move import resolution logic

### Implementation Order (Revised)

1. ✅ **COMPLETED**: Extract `module_registry.rs` (most independent methods)
2. **Next**: Extract `expression_handlers.rs` (most independent)
3. **Next**: Extract `import_deduplicator.rs` (relatively independent)
4. **Week 2**: Extract `symbol_collector.rs` (some dependencies on expressions)
5. **Week 2**: Extract `namespace_manager.rs` (depends on expressions)
6. **Week 3**: Extract `dependency_analyzer.rs` (depends on symbols)
7. **Week 4**: Enhance existing files and final cleanup

### Validation Steps for Each Phase

1. Run full test suite: `cargo test --workspace`
2. Check clippy: `cargo clippy --workspace --all-targets`
3. Verify token count reduction in bundler.rs
4. Run bundling on complex test cases
5. Benchmark performance (should remain same or improve)

### Expected Final Structure

```
code_generator/
├── mod.rs
├── bundler.rs (~20,000 tokens - orchestration only)
├── module_transformer.rs (~20,000 tokens - enhanced)
├── import_transformer.rs (~22,000 tokens - enhanced)
├── expression_handlers.rs (~15,000 tokens - NEW)
├── namespace_manager.rs (~18,000 tokens - NEW)
├── symbol_collector.rs (~12,000 tokens - NEW)
├── module_registry.rs (4,754 tokens - NEW) ✅
├── import_deduplicator.rs (~8,000 tokens - NEW)
├── dependency_analyzer.rs (~10,000 tokens - NEW)
├── circular_deps.rs (unchanged)
├── globals.rs (unchanged)
└── context.rs (unchanged)
```

Total: ~140,000 tokens distributed across 12 files (average ~11,700 tokens per file)
