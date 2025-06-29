# Bundler.rs Function Call Graph - External-Only Unique Calls

This document maps which functions from `bundler.rs` are called **exclusively** by each source file in the `code_generator` module AND are **not** called internally within bundler.rs itself.

## crates/cribo/src/code_generator/import_transformer.rs

Functions called only from import_transformer.rs (and not within bundler.rs):

- `bundler.create_module_access_expr()` - Create expression to access a module from cache

## crates/cribo/src/code_generator/module_transformer.rs

Functions called only from module_transformer.rs (and not within bundler.rs):

- `bundler.create_module_object_stmt()` - Create module object initialization statement
- `bundler.transform_ast_with_lifted_globals()` - Transform AST with lifted global declarations
- `bundler.find_module_id_in_semantic_bundler()` - Find module ID in semantic bundler
- `bundler.add_module_attr_if_exported()` - Add module attribute if symbol is exported
- `bundler.create_namespace_for_inlined_submodule()` - Create namespace for inlined submodules

## Functions Called from Multiple Modules or Within bundler.rs (Removed)

The following functions were removed from the lists above because they are called from multiple sources:

### Called from multiple external modules:

- `bundler.is_hoisted_import()` - Called from both import_transformer.rs and module_transformer.rs
- `bundler.filter_exports_by_tree_shaking()` - Called from both import_transformer.rs and module_transformer.rs

### Called both externally and within bundler.rs:

- `bundler.resolve_relative_import()` - Called from import_transformer.rs and within bundler.rs
- `bundler.rewrite_import_with_renames()` - Called from import_transformer.rs and within bundler.rs
- `bundler.handle_imports_from_inlined_module()` - Called from import_transformer.rs and within bundler.rs
- `bundler.rewrite_import_from()` - Called from import_transformer.rs and within bundler.rs
- `bundler.should_export_symbol()` - Called from module_transformer.rs and within bundler.rs
- `bundler.collect_referenced_vars()` - Called from module_transformer.rs and within bundler.rs
- `bundler.process_body_recursive()` - Called from module_transformer.rs and within bundler.rs
- `bundler.create_module_attr_assignment()` - Called from module_transformer.rs and within bundler.rs
- `bundler.extract_simple_assign_target()` - Called from module_transformer.rs and within bundler.rs
- `bundler.is_self_referential_assignment()` - Called from module_transformer.rs and within bundler.rs
- `bundler.transform_nested_function_for_module_vars()` - Called from module_transformer.rs and within bundler.rs
- `bundler.resolve_relative_import_with_context()` - Called from import_transformer.rs, module_transformer.rs, and within bundler.rs

## crates/cribo/src/code_generator/circular_deps.rs

No direct calls to bundler.rs functions. This module is self-contained and handles circular dependency analysis independently.

## crates/cribo/src/code_generator/context.rs

No calls to bundler.rs functions. This module only defines context structs and types used by other modules:

- `BundleParams`
- `HardDependency`
- `InlineContext`
- `ModuleTransformContext`
- `ProcessGlobalsParams`
- `SemanticContext`

## crates/cribo/src/code_generator/globals.rs

No calls to bundler.rs functions. Contains standalone functions for global variable transformation:

- `process_globals_in_function`
- `transform_name_expr`
- `collect_function_globals`

## crates/cribo/src/code_generator/mod.rs

No calls to bundler.rs functions. This is the module declaration file that:

- Declares submodules
- Re-exports `HybridStaticBundler` from bundler
- Re-exports `bundle_modules` function from bundler
- Re-exports context types

## Summary

After removing functions that are called from multiple modules or within bundler.rs itself, only 6 functions remain as truly unique external calls:

1. **import_transformer.rs** - 1 unique external function
2. **module_transformer.rs** - 5 unique external functions

This analysis reveals that most of bundler.rs's functions are either:

- Used internally for its own logic
- Shared across multiple modules
- Both internally used and externally called

The other modules serve supporting roles:

- **circular_deps.rs** - Independent circular dependency analysis
- **context.rs** - Shared data structures
- **globals.rs** - Independent global variable processing
- **mod.rs** - Module organization and re-exports
