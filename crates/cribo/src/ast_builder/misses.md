# AST Builder Refactoring Opportunities

This document outlines opportunities for refactoring code in `crates/cribo/src/code_generator` to use the `ast_builder` module for creating synthetic AST nodes.

## 1. `expression_handlers.rs`

### `create_namespace_attribute`

This function manually constructs an `Assign` statement.

**Current Implementation:**

```rust
pub(super) fn create_namespace_attribute(bundler: &mut Bundler, parent: &str, child: &str) -> Stmt {
    // Create: parent.child = types.SimpleNamespace()
    let mut stmt = statements::assign(
        vec![expressions::attribute(
            expressions::name(parent, ExprContext::Load),
            child,
            ExprContext::Store,
        )],
        expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
    );

    // Set the node index for transformation tracking
    if let Stmt::Assign(assign) = &mut stmt {
        assign.node_index = bundler
            .transformation_context
            .create_new_node(format!("Create namespace attribute {parent}.{child}"));
    }

    stmt
}
```

**Refactoring suggestion:**

This function can be simplified by using `ast_builder::statements::simple_assign` and `ast_builder::expressions::call` with `simple_namespace_ctor`. The node index creation can be handled within the builder or after the fact.

### `create_dotted_attribute_assignment`

This function manually constructs an `Assign` statement with a dotted attribute target.

**Current Implementation:**

```rust
pub(super) fn create_dotted_attribute_assignment(
    bundler: &mut Bundler,
    dotted_name: &str,
    value_expr: Expr,
) -> Result<Stmt, String> {
    let parts: Vec<&str> = dotted_name.split('.').collect();
    if parts.is_empty() {
        return Err("Empty dotted name".to_string());
    }

    let target_expr = if parts.len() == 1 {
        expressions::name(parts[0], ExprContext::Store)
    } else {
        let mut expr = expressions::name(parts[0], ExprContext::Load);
        for part in &parts[1..parts.len() - 1] {
            expr = expressions::attribute(expr, part, ExprContext::Load);
        }
        expressions::attribute(expr, parts[parts.len() - 1], ExprContext::Store)
    };

    let mut stmt = statements::assign(vec![target_expr], value_expr);

    // Set the node index for transformation tracking
    if let Stmt::Assign(assign) = &mut stmt {
        assign.node_index = bundler
            .transformation_context
            .create_new_node(format!("create_dotted_attribute_assignment({dotted_name})"));
    }

    Ok(stmt)
}
```

**Refactoring suggestion:**

The logic for creating a dotted name expression can be extracted into a new builder function in `ast_builder::expressions`. The assignment itself can then use `ast_builder::statements::assign`.

## 2. `globals.rs`

### `transform_globals_in_expr`

This function replaces `globals()` calls with `module.__dict__`.

**Current Implementation:**

```rust
pub fn transform_globals_in_expr(expr: &mut Expr) {
    match expr {
        Expr::Call(call_expr) => {
            // Check if this is a globals() call
            if let Expr::Name(name_expr) = &*call_expr.func
                && name_expr.id.as_str() == "globals"
                && call_expr.arguments.args.is_empty()
                && call_expr.arguments.keywords.is_empty()
            {
                // Replace the entire expression with module.__dict__
                *expr = expressions::attribute(
                    expressions::name("module", ExprContext::Load),
                    "__dict__",
                    ExprContext::Load,
                );
                return;
            }
            // ...
        } // ...
    }
}
```

**Refactoring suggestion:**

The creation of `module.__dict__` can be done using `ast_builder::expressions::attribute` and `ast_builder::expressions::name`.

## 3. `module_registry.rs`

### `generate_module_init_call`

This function creates `Assign` and `Pass` statements.

**Current Implementation:**

```rust
pub fn generate_module_init_call(
    _synthetic_name: &str,
    module_name: &str,
    init_func_name: Option<&str>,
    module_registry: &FxIndexMap<String, String>,
    generate_merge_module_attributes: impl Fn(&mut Vec<Stmt>, &str, &str),
) -> Vec<Stmt> {
    // ...
    // ...
    // Direct assignment for modules that aren't parent namespaces
    let target_expr = if module_name.contains('.') {
        // For dotted modules like models.base, create an attribute expression
        let parts: Vec<&str> = module_name.split('.').collect();
        ast_builder::expressions::dotted_name(&parts, ExprContext::Store)
    } else {
        // For simple modules, use direct name
        ast_builder::expressions::name(module_name, ExprContext::Store)
    };

    // Generate: module_name = <cribo_init_prefix>synthetic_name()
    // or: parent.child = <cribo_init_prefix>synthetic_name()
    statements.push(ast_builder::statements::assign(
        vec![target_expr],
        ast_builder::expressions::call(
            ast_builder::expressions::name(init_func_name, ExprContext::Load),
            vec![],
            vec![],
        ),
    ));
    // ...
}
```

**Refactoring suggestion:**

The creation of the `Assign` statement can be simplified by using `ast_builder::statements::assign` and other builders. The `Pass` statement can be created with `ast_builder::statements::pass`.

### `create_module_attr_assignment`

This function creates an `Assign` statement to set a module attribute.

**Current Implementation:**

```rust
pub fn create_module_attr_assignment(module_var: &str, attr_name: &str) -> Stmt {
    ast_builder::statements::assign(
        vec![ast_builder::expressions::attribute(
            ast_builder::expressions::name(module_var, ExprContext::Load),
            attr_name,
            ExprContext::Store,
        )],
        ast_builder::expressions::name(attr_name, ExprContext::Load),
    )
}
```

**Refactoring suggestion:**

This function is already using the `ast_builder`, but it could be moved into the `ast_builder::statements` module itself to be reused in other places.

## 4. `module_transformer.rs`

### `transform_module_to_init_function`

This function creates a `FunctionDef` statement.

**Current Implementation:**

```rust
pub fn transform_module_to_init_function<'a>(// ...
) -> Result<Stmt> {
    // ...
    // ...
    // Create the init function WITHOUT decorator - we're not using module cache
    Ok(Stmt::FunctionDef(StmtFunctionDef {
        node_index: AtomicNodeIndex::dummy(),
        name: Identifier::new(init_func_name, TextRange::default()),
        type_params: None,
        parameters: Box::new(ruff_python_ast::Parameters {
            node_index: AtomicNodeIndex::dummy(),
            posonlyargs: vec![],
            args: vec![],
            vararg: None,
            kwonlyargs: vec![],
            kwarg: None,
            range: TextRange::default(),
        }),
        returns: None,
        body,
        decorator_list: vec![], // No decorator for non-cache mode
        is_async: false,
        range: TextRange::default(),
    }))
}
```

**Refactoring suggestion:**

The creation of the `FunctionDef` can be moved to a new builder function in `ast_builder::statements`.

### `create_module_object_stmt`

This function creates `Assign` statements for the module object.

**Current Implementation:**

```rust
pub fn create_module_object_stmt(module_name: &str, _module_path: &Path) -> Vec<Stmt> {
    let module_call = ast_builder::expressions::call(
        ast_builder::expressions::simple_namespace_ctor(),
        vec![],
        vec![],
    );

    vec![
        // module = types.SimpleNamespace()
        ast_builder::statements::assign(
            vec![ast_builder::expressions::name("module", ExprContext::Store)],
            module_call,
        ),
        // module.__name__ = "module_name"
        ast_builder::statements::assign(
            vec![ast_builder::expressions::attribute(
                ast_builder::expressions::name("module", ExprContext::Load),
                "__name__",
                ExprContext::Store,
            )],
            ast_builder::expressions::string_literal(module_name),
        ),
    ]
}
```

**Refactoring suggestion:**

This function is already using the `ast_builder`, but it could be moved into the `ast_builder::statements` module itself to be reused in other places.

## 5. `import_deduplicator.rs`

### `collect_unique_imports_for_hoisting`

This function manually constructs an `Import` statement.

**Current Implementation:**

```rust
fn collect_unique_imports_for_hoisting(
    _bundler: &Bundler,
    import_stmt: &StmtImport,
    seen_modules: &mut crate::types::FxIndexSet<String>,
    unique_imports: &mut Vec<(String, Stmt)>,
) {
    for alias in &import_stmt.names {
        let module_name = alias.name.as_str();
        if seen_modules.contains(module_name) {
            continue;
        }
        seen_modules.insert(module_name.to_string());
        // Create import statement preserving the original alias
        unique_imports.push((
            module_name.to_string(),
            Stmt::Import(StmtImport {
                node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                names: vec![Alias {
                    node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                    name: ruff_python_ast::Identifier::new(
                        module_name,
                        ruff_text_size::TextRange::default(),
                    ),
                    asname: alias.asname.clone(),
                    range: ruff_text_size::TextRange::default(),
                }],
                range: ruff_text_size::TextRange::default(),
            }),
        ));
    }
}
```

**Refactoring suggestion:**

The creation of the `Import` statement can be simplified by using `ast_builder::statements::import` and `ast_builder::other::alias`.
