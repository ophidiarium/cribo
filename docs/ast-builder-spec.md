# AST Builder Module: Implementation Specification

## 1. Overview

This document provides a comprehensive implementation plan for a new `ast_builder.rs` module within the `cribo` crate. The module's purpose is to centralize the creation of synthetic Abstract Syntax Tree (AST) nodes, which are nodes not originating directly from parsed source files.

By providing a consistent set of factory functions, this module will:

- **Improve Consistency**: Ensure all generated nodes have uniform properties.
- **Enhance Readability**: Replace verbose, manual node construction with concise, intention-revealing function calls.
- **Simplify Maintenance**: Allow future changes to AST construction to be made in a single, centralized location.

All synthetic nodes created by this module will use `TextRange::default()` and `AtomicNodeIndex::NONE` to signify their generated nature.

## 2. Module Organization and Naming Conventions

### 2.1. Module Structure

To ensure scalability and maintainability, the `ast_builder` will be organized into submodules based on the type of AST node being constructed.

**Proposed Directory Structure**:

```
crates/cribo/src/
├── ast_builder/
│   ├── expressions.rs
│   ├── statements.rs
│   ├── other.rs
│   └── mod.rs
└── ...
```

### 2.2. Naming Conventions

- **Module Name**: The new module will be named `ast_builder`.
- **Function Names**: Functions will use short, direct names (e.g., `name`, `assign`) and will be scoped within the `ast_builder` module (e.g., `ast_builder::expressions::name(...)`).

## 3. Factory Functions API

The following functions are proposed based on a detailed analysis of the `code_generator`'s current implementation.

### 3.1. `expressions.rs`

- `name(name: &str, ctx: ExprContext) -> Expr`
- `attribute(value: Expr, attr: &str, ctx: ExprContext) -> Expr`
- `dotted_name(parts: &[&str], ctx: ExprContext) -> Expr`
- `call(func: Expr, args: Vec<Expr>, keywords: Vec<Keyword>) -> Expr`
- `string_literal(value: &str) -> Expr`
- `none_literal() -> Expr`
- `list(elts: Vec<Expr>, ctx: ExprContext) -> Expr`
- `tuple(elts: Vec<Expr>, ctx: ExprContext) -> Expr`
- `dict(pairs: Vec<(Expr, Expr)>) -> Expr`
- `bool_op(op: BoolOp, values: Vec<Expr>) -> Expr`
- `bin_op(left: Expr, op: Operator, right: Expr) -> Expr`
- `unary_op(op: UnaryOp, operand: Expr) -> Expr`
- `subscript(value: Expr, slice: Expr, ctx: ExprContext) -> Expr`
- `slice(lower: Option<Expr>, upper: Option<Expr>, step: Option<Expr>) -> Expr`

### 3.2. `statements.rs`

- `assign(targets: Vec<Expr>, value: Expr) -> Stmt`
- `simple_assign(target: &str, value: Expr) -> Stmt` (Convenience wrapper)
- `expr(expr: Expr) -> Stmt`
- `import(names: Vec<Alias>) -> Stmt`
- `import_from(module: Option<&str>, names: Vec<Alias>, level: u32) -> Stmt`
- `pass() -> Stmt`
- `return_stmt(value: Option<Expr>) -> Stmt`
- `global(names: Vec<&str>) -> Stmt`
- `if_stmt(test: Expr, body: Vec<Stmt>, orelse: Vec<Stmt>) -> Stmt`
- `raise(exc: Option<Expr>, cause: Option<Expr>) -> Stmt`
- `try_stmt(body: Vec<Stmt>, handlers: Vec<ExceptHandler>, orelse: Vec<Stmt>, finalbody: Vec<Stmt>) -> Stmt`
- `class_def(name: &str, arguments: Option<Arguments>, body: Vec<Stmt>) -> Stmt`

### 3.3. `other.rs`

- `alias(name: &str, asname: Option<&str>) -> Alias`
- `keyword(arg: &str, value: Expr) -> Keyword`
- `keyword_unpack(value: Expr) -> Keyword` (For `**kwargs` patterns)
- `arguments(posonlyargs: Vec<Parameter>, args: Vec<Parameter>, vararg: Option<Parameter>, kwonlyargs: Vec<Parameter>, kwarg: Option<Parameter>) -> Arguments`
- `except_handler(type_: Option<Expr>, name: Option<&str>, body: Vec<Stmt>) -> ExceptHandler`

## 4. Technical and Quality Requirements

### 4.1. Error Handling

The `ast_builder` functions will assume that callers provide valid inputs (e.g., valid identifiers for names). No runtime validation will be performed within the builder functions. This responsibility lies with the caller, which is consistent with the underlying `ruff_python_ast` library and avoids performance overhead.

### 4.2. Determinism

The module **must** produce deterministic output to align with the project's core requirements. This will be achieved by:

- Using deterministic collections (`Vec`, `IndexMap`) internally.
- Ensuring that the order of elements in generated code is based on a stable order. Callers are responsible for pre-sorting any collections where order matters.

### 4.3. `clippy.toml` Compliance

The implementation must adhere to all restrictions defined in the project's `.clippy.toml` file, avoiding any disallowed types or methods.

## 5. Testing and Validation Strategy

This is a refactoring initiative. The primary measure of success is that the **existing comprehensive test suite continues to pass without any changes to the tests themselves.** This ensures that the generated code remains functionally identical.

## 6. Implementation and Migration Plan

The implementation and subsequent refactoring will be performed in phases to ensure that no dead code is merged and that each phase results in a stable, complete state.

### Phase 1: Core Expression and Assignment Builders

- **Implement**:
  - `expressions::name`
  - `expressions::attribute`
  - `expressions::string_literal`
  - `expressions::none_literal`
  - `expressions::call`
  - `statements::assign`
  - `statements::simple_assign`
  - `statements::expr`
  - `other::alias`
  - `other::keyword`
- **Refactor**:
  - Migrate all usage of the corresponding manual AST constructions in `crates/cribo/src/code_generator/module_transformer.rs`.
- **Validate**:
  - Ensure all new unit tests and existing project tests pass.
  - Run benchmarks to establish a baseline and confirm no regressions.

### Phase 2: Import and Module-Level Builders

- **Implement**:
  - `expressions::dotted_name`
  - `expressions::subscript`
  - `statements::import`
  - `statements::import_from`
  - `statements::return_stmt`
  - `statements::pass`
- **Refactor**:
  - Migrate all usage of the corresponding manual AST constructions in `crates/cribo/src/code_generator/bundler.rs` and `crates/cribo/src/code_generator/import_transformer.rs`.
- **Validate**:
  - Ensure all new unit tests and existing project tests pass.
  - Run benchmarks and compare against the baseline.

### Phase 3: Control Flow and Remaining Builders

- **Implement**:
  - All remaining functions defined in Section 3.
- **Refactor**:
  - Migrate the remaining manual AST constructions throughout the `code_generator` module.
- **Validate**:
  - Ensure all tests pass and benchmarks remain within acceptable limits.

## 7. Documentation

- **Module-level**: The `ast_builder/mod.rs` file will contain documentation explaining the module's purpose, design, and usage patterns.
- **Function-level**: Each public function will have a doc comment with a clear explanation, parameter descriptions, and a concise usage example.
- **Examples for Complex Cases**: Documentation for functions like `import_from` will include examples for various scenarios, such as relative imports:
  ```rust
  // from foo import bar
  ast_builder::statements::import_from(Some("foo"), vec![alias("bar", None)], 0)

  // from . import foo
  ast_builder::statements::import_from(None, vec![alias("foo", None)], 1)

  // from ..parent import something
  ast_builder::statements::import_from(Some("parent"), vec![alias("something", None)], 2)
  ```

## 8. Future Considerations

While the primary goal is to centralize AST *creation*, this module provides a foundation for future enhancements, such as:

- AST validation utilities.
- AST pattern matching helpers.
- Tighter integration with `transformation_context.rs` for more complex AST manipulations.
