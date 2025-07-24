# Analysis of bundler.rs Refactoring Misses

This document outlines the discrepancies between the proposed refactoring in `further-split.md` and the current state of `crates/cribo/src/code_generator/bundler.rs`.

## 1. Functions Not Yet Extracted

The following functions were identified as candidates for extraction in `further-split.md` but remain fully implemented in `bundler.rs`. They are sorted by their approximate line count (from largest to smallest).

- ✅ `rewrite_import_from` (~150 lines)
- ✅ `rewrite_import_with_renames` (~150 lines)
- ✅ `resolve_relative_import_with_context` (~100 lines)
- `transform_namespace_package_imports` (~80 lines)
- 🛑 `collect_module_renames` (~80 lines)
- `create_namespace_for_inlined_module_static` (~80 lines)
- `ensure_namespace_exists` (~50 lines)
- ✅ `extract_all_exports` (~50 lines)
- `handle_imports_from_inlined_module_with_context` (~50 lines)
- `add_hoisted_imports` (~50 lines)
- `collect_imports_from_module` (~40 lines)
- `sort_wrapper_modules_by_dependencies` (~25 lines)
- `transform_module_to_cache_init_function` (~20 lines)
- `process_wrapper_module_globals` (~20 lines)
- `is_package_init_reexport` (~20 lines)
- `create_namespace_with_name` (~15 lines)
- `collect_future_imports_from_ast` (~15 lines)
- `create_namespace_statements` (~15 lines)
- `create_namespace_attribute` (~15 lines)
- `sort_wrapped_modules_by_dependencies` (~10 lines)
- `filter_exports_by_tree_shaking` (~10 lines)
- `should_inline_symbol` (~10 lines)

## 2. Functions Using Wrappers Instead of Direct Calls

The following functions were moved out of `bundler.rs`, but the bundler still contains a local wrapper function to call the moved implementation instead of using a direct `submodule::function()` call.

- `rewrite_aliases_in_stmt`: Wraps a call to `rewrite_aliases_in_stmt_impl`.
- `rewrite_aliases_in_expr`: Wraps a call to `rewrite_aliases_in_expr_impl`.
- `resolve_import_aliases_in_stmt`: Wraps a call to `expression_handlers::resolve_import_aliases_in_stmt`.

### Partially Refactored Wrappers

These functions are wrappers around other functions that are *also* still present in `bundler.rs`, indicating an incomplete refactoring step.

- `resolve_relative_import`: Wraps `resolve_relative_import_with_context`.
- `handle_imports_from_inlined_module`: Wraps `handle_imports_from_inlined_module_with_context`.
