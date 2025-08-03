# AST Builder Refactoring Opportunities

This document outlines opportunities for refactoring code in `crates/cribo/src/code_generator` to use the `ast_builder` module for creating synthetic AST nodes.

## 1. `expression_handlers.rs`

### `create_namespace_attribute` ✅

This function manually constructs an `Assign` statement.

**Completed**: The function already uses ast_builder appropriately. The manual node_index setting is a bundler-specific requirement and not part of generic AST building, so this pattern is correct.

### `create_dotted_attribute_assignment` ✅

This function manually constructs an `Assign` statement with a dotted attribute target.

**Completed**: The function now uses `expressions::dotted_name` from the ast_builder to create the target expression, simplifying the code significantly.

## 2. `globals.rs`

### `transform_globals_in_expr` ✅

This function replaces `globals()` calls with `module.__dict__`.

**Completed**: The function already uses `ast_builder::expressions::attribute` and `ast_builder::expressions::name` for creating the replacement expression. No refactoring needed.

## 3. `module_registry.rs`

### `generate_module_init_call` ✅

This function creates `Assign` and `Pass` statements.

**Completed**: The function already uses `ast_builder::expressions::dotted_name`, `ast_builder::expressions::name`, `ast_builder::statements::assign`, and other ast_builder functions appropriately. No refactoring needed.

### `create_module_attr_assignment` ✅

This function creates an `Assign` statement to set a module attribute.

**Completed**: Added a new `assign_attribute` function to `ast_builder::statements` and refactored the function to use it.

## 4. `module_transformer.rs`

### `transform_module_to_init_function` ✅

This function creates a `FunctionDef` statement.

**Completed**: Added a new `function_def` function to `ast_builder::statements` and refactored the function to use it.

### `create_module_object_stmt` ✅

This function creates `Assign` statements for the module object.

**Completed**: Refactored to use `simple_assign` and our new `assign_attribute` function, making the code more concise and readable.

## 5. `import_deduplicator.rs`

### `collect_unique_imports_for_hoisting` ✅

This function manually constructs an `Import` statement.

**Completed**: Refactored to use `ast_builder::statements::import` and `ast_builder::other::alias`, making the code more concise and maintaining consistency with the rest of the codebase.
