//! Module transformation logic for converting Python modules into init functions
//!
//! This module handles the complex transformation of Python module ASTs into
//! initialization functions that can be called to create module objects.

/// Name of the module object parameter used in generated init functions.
pub(crate) const SELF_PARAM: &str = "self";

use log::debug;
use ruff_python_ast::{
    ExceptHandler, Expr, ExprContext, ModModule, Stmt, StmtAssign, StmtFunctionDef,
};

use crate::{
    ast_builder,
    code_generator::{
        bundler::Bundler, context::ModuleTransformContext, expression_handlers,
        import_deduplicator, module_registry::sanitize_module_name_for_identifier,
    },
    types::{FxIndexMap, FxIndexSet},
};

/// Process each statement from the transformed module body
/// and add appropriate module attributes for exported symbols
///
/// This function handles the core statement-by-statement processing within an init function,
/// applying transformations and adding module attributes as needed for different statement types.
#[expect(clippy::too_many_arguments)] // Necessary for extracting complex logic
pub(crate) fn process_statements_for_init_function(
    processed_body: Vec<Stmt>,
    bundler: &Bundler<'_>,
    ctx: &ModuleTransformContext<'_>,
    all_is_referenced: bool,
    vars_used_by_exported_functions: &FxIndexSet<String>,
    module_scope_symbols: Option<&FxIndexSet<String>>,
    builtin_locals: &FxIndexSet<String>,
    lifted_names: Option<&FxIndexMap<String, String>>,
    inlined_import_bindings: &[String],
    body: &mut Vec<Stmt>,
    initialized_lifted_globals: &mut FxIndexSet<String>,
) {
    // Helper function to get exported module-level variables
    let get_exported_module_vars =
        |bundler: &Bundler<'_>, ctx: &ModuleTransformContext<'_>| -> FxIndexSet<String> {
            ctx.global_info
                .as_ref()
                .map_or_else(FxIndexSet::default, |global_info| {
                    let all_vars = &global_info.module_level_vars;
                    let mut exported_vars = FxIndexSet::default();
                    for var in all_vars {
                        if bundler.should_export_symbol(var, ctx.module_name) {
                            exported_vars.insert(var.clone());
                        }
                    }
                    exported_vars
                })
        };

    // Process each statement from the transformed module body
    for (idx, stmt) in processed_body.into_iter().enumerate() {
        match &stmt {
            Stmt::Assign(_) => debug!("Processing statement {idx} in init function: Assign"),
            Stmt::ImportFrom(_) => {
                debug!("Processing statement {idx} in init function: ImportFrom");
            }
            Stmt::Expr(_) => debug!("Processing statement {idx} in init function: Expr"),
            Stmt::For(_) => debug!("Processing statement {idx} in init function: For"),
            _ => debug!("Processing statement {idx} in init function: Other"),
        }
        match &stmt {
            Stmt::Import(_import_stmt) => {
                // Skip imports that are already hoisted
                if !import_deduplicator::is_hoisted_import(bundler, &stmt) {
                    body.push(stmt.clone());
                }
            }
            Stmt::ImportFrom(import_from) => {
                // Skip __future__ imports - they cannot appear inside functions
                if import_from
                    .module
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str)
                    == Some("__future__")
                {
                    continue;
                }

                // Skip imports that are already hoisted
                if !import_deduplicator::is_hoisted_import(bundler, &stmt) {
                    // Handle relative imports that reference the same module (e.g., from . import
                    // errors) These should be converted to simple assignments
                    // since the symbols are already available
                    if import_from.level > 0 {
                        log::debug!(
                            "Found relative import in init function for module '{}': from {} \
                             import {:?}",
                            ctx.module_name,
                            ".".repeat(import_from.level as usize),
                            import_from
                                .names
                                .iter()
                                .map(|a| a.name.as_str())
                                .collect::<Vec<_>>()
                        );

                        // Get the module path for the current module
                        let module_id = bundler.resolver.get_module_id_by_name(ctx.module_name);
                        let module_path =
                            module_id.and_then(|id| bundler.resolver.get_module_path(id));

                        log::debug!("Module '{}' has path: {:?}", ctx.module_name, module_path);

                        // Resolve the relative import to absolute module name
                        let resolved_module = module_path.and_then(|path| {
                            bundler.resolver.resolve_relative_to_absolute_module_name(
                                import_from.level,
                                import_from
                                    .module
                                    .as_ref()
                                    .map(ruff_python_ast::Identifier::as_str),
                                &path,
                            )
                        });

                        log::debug!("Resolved relative import to: {resolved_module:?}");

                        // For wrapper modules doing `from . import`, we need to determine the
                        // correct base:
                        // - Package __init__ files: resolved module gives the package itself
                        // - Regular module files: resolved module is empty (""), use parent package
                        if import_from.level == 1 && import_from.module.is_none() {
                            log::debug!(
                                "Handling 'from . import' in wrapper module '{}', converting to \
                                 assignments",
                                ctx.module_name
                            );

                            // Determine the base module for imports:
                            // If resolved_module is empty, it means we're in a regular module file
                            // importing from its parent package, so extract the parent package
                            // name. Otherwise use the resolved module.
                            let base_module = match resolved_module.as_deref() {
                                Some("") | None => {
                                    // Regular module file: extract parent package
                                    let parent = ctx
                                        .module_name
                                        .rsplit_once('.')
                                        .map_or("", |(parent, _)| parent);
                                    log::debug!(
                                        "Using parent package '{}' as base for relative imports \
                                         in '{}'",
                                        parent,
                                        ctx.module_name
                                    );
                                    parent
                                }
                                Some(resolved) => {
                                    log::debug!(
                                        "Using resolved module '{}' as base for relative imports \
                                         in '{}'",
                                        resolved,
                                        ctx.module_name
                                    );
                                    resolved
                                }
                            };

                            // Use shared helper to transform relative import aliases
                            crate::code_generator::import_transformer::handlers::relative::transform_relative_import_aliases(
                                bundler,
                                import_from,
                                base_module,
                                ctx.module_name,
                                body,
                                false, // use emit_module_attr_if_exportable instead
                            );

                            // Handle module attributes with proper exportability checks
                            for alias in &import_from.names {
                                let imported_name = alias.name.as_str();
                                if imported_name != "*" {
                                    let local_name =
                                        alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                                    emit_module_attr_if_exportable(
                                        bundler,
                                        local_name,
                                        ctx.module_name,
                                        body,
                                        module_scope_symbols,
                                        None, // not a lifted var
                                    );
                                }
                            }
                            // Skip adding the original import statement
                            continue;
                        }

                        // For other relative imports that don't match our pattern, keep the
                        // original
                    } else {
                        // For non-relative imports, keep the original
                    }
                    body.push(stmt.clone());
                }

                // Module attribute assignments for imported names are already handled by
                // process_body_recursive in the bundler, so we don't need to add them here
            }
            Stmt::ClassDef(class_def) => {
                // Add class definition
                body.push(stmt.clone());

                let symbol_name = class_def.name.to_string();

                // Note: We set __module__ for the class, but Python still shows the full scope path
                // in the class repr when it's defined inside a function. This is expected behavior.
                // Setting __module__ helps with introspection but doesn't change the repr.
                body.push(ast_builder::statements::assign_attribute(
                    &symbol_name,
                    "__module__",
                    ast_builder::expressions::string_literal(ctx.module_name),
                ));

                // Set as module attribute via centralized helper
                emit_module_attr_if_exportable(
                    bundler,
                    &symbol_name,
                    ctx.module_name,
                    body,
                    module_scope_symbols,
                    None, // not a lifted var
                );
            }
            Stmt::FunctionDef(func_def) => {
                // Clone the function for transformation
                let mut func_def_clone = func_def.clone();

                // Transform nested functions to use module attributes for module-level vars
                if let Some(ref global_info) = ctx.global_info {
                    bundler.transform_nested_function_for_module_vars_with_global_info(
                        &mut func_def_clone,
                        &global_info.module_level_vars,
                        &global_info.global_declarations,
                        lifted_names,
                        SELF_PARAM,
                    );
                }

                // Add transformed function definition
                body.push(Stmt::FunctionDef(func_def_clone));

                // Set as module attribute via centralized helper
                let symbol_name = func_def.name.to_string();
                emit_module_attr_if_exportable(
                    bundler,
                    &symbol_name,
                    ctx.module_name,
                    body,
                    module_scope_symbols,
                    None, // not a lifted var
                );
            }
            Stmt::Assign(assign) => {
                // Handle __all__ assignments - skip unless it's referenced elsewhere
                if let Some(name) = expression_handlers::extract_simple_assign_target(assign)
                    && name == "__all__"
                {
                    if all_is_referenced {
                        // __all__ is referenced elsewhere, include the assignment
                        body.push(stmt.clone());
                    }
                    // Skip further processing for __all__ assignments
                    continue;
                }

                // Skip self-referential assignments like `process = process`
                // These are meaningless in the init function context and cause errors
                if expression_handlers::is_self_referential_assignment(assign, ctx.python_version) {
                    debug!(
                        "Skipping self-referential assignment in module '{}': {:?}",
                        ctx.module_name,
                        assign.targets.first().and_then(|t| match t {
                            Expr::Name(name) => Some(name.id.as_str()),
                            _ => None,
                        })
                    );
                } else {
                    // Clone and transform the assignment to handle __name__ references
                    let mut assign_clone = assign.clone();

                    // Use actual module-level variables if available, but filter to only
                    // exported ones
                    let module_level_vars = get_exported_module_vars(bundler, ctx);

                    // Special handling for assignments involving built-in types
                    // We need to transform any reference to a built-in that will be assigned
                    // as a local variable later in this function
                    transform_expr_for_builtin_shadowing(&mut assign_clone.value, builtin_locals);

                    // Also transform module-level variable references
                    // Inside the init function, use "self" to refer to the module
                    transform_expr_for_module_vars(
                        &mut assign_clone.value,
                        &module_level_vars,
                        SELF_PARAM, // Use "self" instead of module_var_name inside init function
                        ctx.python_version,
                    );

                    // For simple assignments, also set as module attribute if it should be
                    // exported
                    body.push(Stmt::Assign(assign_clone.clone()));

                    // If this variable is being lifted to a global, update the global
                    let lifted_var_handled = if let Some(lifted_names) = lifted_names
                        && let Some(name) =
                            expression_handlers::extract_simple_assign_target(&assign_clone)
                        && let Some(lifted_name) = lifted_names.get(&name)
                    {
                        // Always propagate to the lifted binding to keep it in sync
                        body.push(ast_builder::statements::assign(
                            vec![ast_builder::expressions::name(
                                lifted_name,
                                ExprContext::Store,
                            )],
                            ast_builder::expressions::name(&name, ExprContext::Load),
                        ));

                        // Keep the module attribute consistent with the current value
                        body.push(
                            crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                                SELF_PARAM,
                                &name,
                                lifted_name,
                            ),
                        );

                        if initialized_lifted_globals.insert(name.clone()) {
                            debug!("Initialized lifted global '{lifted_name}' from '{name}'");
                        } else {
                            debug!(
                                "Refreshed lifted global '{lifted_name}' after reassignment of \
                                 '{name}'"
                            );
                        }
                        true
                    } else {
                        false
                    };

                    // Skip further module attribute handling if this was a lifted variable
                    if lifted_var_handled {
                        // Already handled as a lifted variable
                    } else if let Some(name) =
                        expression_handlers::extract_simple_assign_target(assign)
                    {
                        debug!(
                            "Checking assignment '{}' in module '{}' (inlined_import_bindings: \
                             {:?})",
                            name, ctx.module_name, inlined_import_bindings
                        );

                        if inlined_import_bindings.contains(&name) {
                            // This was imported from an inlined module
                            // Module attributes for imports are now handled by import_transformer
                            // to ensure correct value assignment (original_name vs local_name)
                            debug!(
                                "Skipping module attribute for imported symbol '{name}' - handled \
                                 by import_transformer"
                            );
                        } else if vars_used_by_exported_functions.contains(&name) {
                            // Check if this variable is used by exported functions
                            // Use a special case: if no scope info available, include vars used
                            // by exported functions
                            let should_include =
                                module_scope_symbols.is_none_or(|symbols| symbols.contains(&name));

                            if should_include {
                                debug!("Exporting '{name}' as it's used by exported functions");
                                body.push(crate::code_generator::module_registry::create_module_attr_assignment(
                                    SELF_PARAM,
                                    &name,
                                ));
                            }
                        } else {
                            // Regular assignment, use the normal export logic
                            add_module_attr_if_exported(
                                bundler,
                                assign,
                                ctx.module_name,
                                body,
                                module_scope_symbols,
                            );
                        }
                    } else {
                        // Not a simple assignment
                        add_module_attr_if_exported(
                            bundler,
                            assign,
                            ctx.module_name,
                            body,
                            module_scope_symbols,
                        );
                    }
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Handle annotated assignments similar to regular assignments
                if ann_assign.value.is_some() {
                    // Skip __all__ annotated assignments unless it's referenced elsewhere
                    if let Expr::Name(target) = ann_assign.target.as_ref()
                        && target.id.as_str() == "__all__"
                        && !all_is_referenced
                    {
                        continue;
                    }

                    let mut ann_assign_clone = ann_assign.clone();

                    // Use actual module-level variables if available, but filter to only exported
                    // ones
                    let module_level_vars = get_exported_module_vars(bundler, ctx);

                    // Transform references to built-ins that will be shadowed
                    if let Some(ref mut value) = ann_assign_clone.value {
                        transform_expr_for_builtin_shadowing(value, builtin_locals);

                        // Also transform module-level variable references
                        // Inside the init function, use "self" to refer to the module
                        transform_expr_for_module_vars(
                            value,
                            &module_level_vars,
                            SELF_PARAM, /* Use "self" instead of module_var_name inside init
                                         * function */
                            ctx.python_version,
                        );
                    }

                    // Transform the annotation expression as well
                    transform_expr_for_builtin_shadowing(
                        &mut ann_assign_clone.annotation,
                        builtin_locals,
                    );
                    transform_expr_for_module_vars(
                        &mut ann_assign_clone.annotation,
                        &module_level_vars,
                        SELF_PARAM, // Use "self" instead of module_var_name inside init function
                        ctx.python_version,
                    );

                    body.push(Stmt::AnnAssign(ann_assign_clone.clone()));

                    // If this variable is being lifted to a global, handle it
                    if let Some(lifted_names) = lifted_names
                        && let Expr::Name(target) = ann_assign_clone.target.as_ref()
                        && let Some(lifted_name) = lifted_names.get(target.id.as_str())
                    {
                        // Always propagate to the lifted binding to keep it in sync
                        body.push(ast_builder::statements::assign(
                            vec![ast_builder::expressions::name(
                                lifted_name,
                                ExprContext::Store,
                            )],
                            ast_builder::expressions::name(&target.id, ExprContext::Load),
                        ));

                        // Keep the module attribute consistent with the current value
                        body.push(crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                            SELF_PARAM,
                            target.id.as_str(),
                            lifted_name,
                        ));

                        if initialized_lifted_globals.insert(target.id.to_string()) {
                            debug!(
                                "Initialized lifted global '{lifted_name}' from annotated \
                                 assignment '{}'",
                                target.id
                            );
                        } else {
                            debug!(
                                "Refreshed lifted global '{lifted_name}' after annotated \
                                 reassignment '{}'",
                                target.id
                            );
                        }
                    }

                    // Also set as module attribute if it should be exported (for non-lifted vars)
                    if let Expr::Name(target) = ann_assign.target.as_ref() {
                        emit_module_attr_if_exportable(
                            bundler,
                            &target.id,
                            ctx.module_name,
                            body,
                            module_scope_symbols,
                            lifted_names,
                        );
                    }
                } else {
                    // Type annotation without value, just add it
                    body.push(stmt.clone());
                }
            }
            Stmt::Try(try_stmt) => {
                // Clone the try statement for transformation
                let try_stmt_clone = try_stmt.clone();

                // Collect all function and class definitions from all branches
                let mut symbols_to_export = FxIndexSet::default();

                /// Recursively collects function and class definitions from control-flow
                /// statements.
                ///
                /// Traverses If, Try, For, While, With, and Match constructs to find function and
                /// class definitions that may be conditionally defined, but intentionally stops at
                /// function/class boundaries to avoid collecting internal methods.
                ///
                /// # Parameters
                /// - `stmts`: Slice of statements to traverse
                /// - `symbols`: Mutable set where collected symbol names are inserted
                fn collect_exportable_symbols(stmts: &[Stmt], symbols: &mut FxIndexSet<String>) {
                    for stmt in stmts {
                        match stmt {
                            Stmt::FunctionDef(f) => {
                                symbols.insert(f.name.to_string());
                                // Don't recurse into function bodies
                            }
                            Stmt::ClassDef(c) => {
                                symbols.insert(c.name.to_string());
                                // Don't recurse into class bodies
                            }
                            Stmt::If(if_stmt) => {
                                collect_exportable_symbols(&if_stmt.body, symbols);
                                for clause in &if_stmt.elif_else_clauses {
                                    collect_exportable_symbols(&clause.body, symbols);
                                }
                            }
                            Stmt::Try(try_stmt) => {
                                collect_exportable_symbols(&try_stmt.body, symbols);
                                for handler in &try_stmt.handlers {
                                    let ExceptHandler::ExceptHandler(except_handler) = handler;
                                    collect_exportable_symbols(&except_handler.body, symbols);
                                }
                                collect_exportable_symbols(&try_stmt.orelse, symbols);
                                collect_exportable_symbols(&try_stmt.finalbody, symbols);
                            }
                            Stmt::For(for_stmt) => {
                                collect_exportable_symbols(&for_stmt.body, symbols);
                                collect_exportable_symbols(&for_stmt.orelse, symbols);
                            }
                            Stmt::While(while_stmt) => {
                                collect_exportable_symbols(&while_stmt.body, symbols);
                                collect_exportable_symbols(&while_stmt.orelse, symbols);
                            }
                            Stmt::With(with_stmt) => {
                                collect_exportable_symbols(&with_stmt.body, symbols);
                            }
                            Stmt::Match(match_stmt) => {
                                for case_ in &match_stmt.cases {
                                    collect_exportable_symbols(&case_.body, symbols);
                                }
                            }
                            _ => {
                                // Other statements don't contain nested definitions we care about
                            }
                        }
                    }
                }

                // Collect from try body
                collect_exportable_symbols(&try_stmt.body, &mut symbols_to_export);

                // Collect from except handlers
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(except_handler) = handler;
                    collect_exportable_symbols(&except_handler.body, &mut symbols_to_export);
                }

                // Collect from else clause
                collect_exportable_symbols(&try_stmt.orelse, &mut symbols_to_export);

                // Collect from finally clause
                collect_exportable_symbols(&try_stmt.finalbody, &mut symbols_to_export);

                // Add the try statement
                body.push(Stmt::Try(try_stmt_clone));

                // After the try block, assign all collected symbols to self
                // This ensures they're available regardless of which branch was taken
                for symbol_name in symbols_to_export {
                    // Use should_export_symbol directly for dynamically discovered symbols
                    // from try-except blocks, as they won't be in module_scope_symbols.
                    // The emit_module_attr_if_exportable helper is designed for statically
                    // known symbols and would incorrectly filter these out.
                    if bundler.should_export_symbol(&symbol_name, ctx.module_name) {
                        debug!(
                            "Exporting symbol '{}' from try-except block in module '{}'",
                            symbol_name, ctx.module_name
                        );
                        body.push(
                            crate::code_generator::module_registry::create_module_attr_assignment(
                                SELF_PARAM,
                                &symbol_name,
                            ),
                        );
                    }
                }
            }
            _ => {
                // Clone and transform other statements to handle __name__ references
                let mut stmt_clone = stmt.clone();
                // Use actual module-level variables if available, but filter to only exported
                // ones
                let module_level_vars = get_exported_module_vars(bundler, ctx);
                let transform_ctx = ModuleVarTransformContext {
                    bundler,
                    module_level_vars: &module_level_vars,
                    module_var_name: SELF_PARAM, /* Use "self" instead of module_var_name inside
                                                  * init function */
                    global_declarations: ctx.global_info.as_ref().map(|g| &g.global_declarations),
                    lifted_names,
                    python_version: ctx.python_version,
                };
                transform_stmt_for_module_vars_with_bundler(&mut stmt_clone, &transform_ctx);
                body.push(stmt_clone);
            }
        }
    }
}

/// Transform an expression to use module attributes for module-level variables
pub(crate) fn transform_expr_for_module_vars(
    expr: &mut Expr,
    module_level_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    match expr {
        Expr::Name(name) if name.ctx == ExprContext::Load => {
            // Special case: transform __name__ to module.__name__
            if name.id.as_str() == "__name__" {
                // Transform __name__ -> module.__name__
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name(module_var_name, ExprContext::Load),
                    "__name__",
                    ExprContext::Load,
                );
            }
            // Check if this is a reference to a module-level variable
            // BUT exclude Python builtins from transformation
            else if module_level_vars.contains(name.id.as_str())
                && !ruff_python_stdlib::builtins::is_python_builtin(
                    name.id.as_str(),
                    python_version,
                    false,
                )
            {
                // Transform to module.var
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name(module_var_name, ExprContext::Load),
                    name.id.as_str(),
                    ExprContext::Load,
                );
            }
        }
        // Recursively handle other expressions
        Expr::Call(call) => {
            transform_expr_for_module_vars(
                &mut call.func,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for arg in &mut call.arguments.args {
                transform_expr_for_module_vars(
                    arg,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for kw in &mut call.arguments.keywords {
                transform_expr_for_module_vars(
                    &mut kw.value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Attribute(attr) => {
            transform_expr_for_module_vars(
                &mut attr.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::BinOp(binop) => {
            transform_expr_for_module_vars(
                &mut binop.left,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut binop.right,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::UnaryOp(unop) => {
            transform_expr_for_module_vars(
                &mut unop.operand,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::If(if_expr) => {
            transform_expr_for_module_vars(
                &mut if_expr.test,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut if_expr.body,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut if_expr.orelse,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::List(list) => {
            for elem in &mut list.elts {
                transform_expr_for_module_vars(
                    elem,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Tuple(tuple) => {
            for elem in &mut tuple.elts {
                transform_expr_for_module_vars(
                    elem,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(key) = &mut item.key {
                    transform_expr_for_module_vars(
                        key,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                transform_expr_for_module_vars(
                    &mut item.value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Subscript(sub) => {
            transform_expr_for_module_vars(
                &mut sub.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut sub.slice,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Set(set) => {
            for elem in &mut set.elts {
                transform_expr_for_module_vars(
                    elem,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Lambda(lambda) => {
            // Note: Lambda parameters create a new scope, so we don't transform them
            transform_expr_for_module_vars(
                &mut lambda.body,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Compare(cmp) => {
            transform_expr_for_module_vars(
                &mut cmp.left,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for comp in &mut cmp.comparators {
                transform_expr_for_module_vars(
                    comp,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::BoolOp(boolop) => {
            for value in &mut boolop.values {
                transform_expr_for_module_vars(
                    value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::ListComp(comp) => {
            transform_expr_for_module_vars(
                &mut comp.elt,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(
                        if_clause,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Expr::SetComp(comp) => {
            transform_expr_for_module_vars(
                &mut comp.elt,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(
                        if_clause,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Expr::DictComp(comp) => {
            transform_expr_for_module_vars(
                &mut comp.key,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut comp.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(
                        if_clause,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Expr::Generator(r#gen) => {
            transform_expr_for_module_vars(
                &mut r#gen.elt,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for generator in &mut r#gen.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(
                        if_clause,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Expr::Await(await_expr) => {
            transform_expr_for_module_vars(
                &mut await_expr.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                transform_expr_for_module_vars(
                    value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::YieldFrom(yield_from) => {
            transform_expr_for_module_vars(
                &mut yield_from.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Starred(starred) => {
            transform_expr_for_module_vars(
                &mut starred.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Named(named) => {
            transform_expr_for_module_vars(
                &mut named.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Slice(slice) => {
            if let Some(ref mut lower) = slice.lower {
                transform_expr_for_module_vars(
                    lower,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            if let Some(ref mut upper) = slice.upper {
                transform_expr_for_module_vars(
                    upper,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            if let Some(ref mut step) = slice.step {
                transform_expr_for_module_vars(
                    step,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::FString(_fstring) => {
            // F-strings require special handling due to their immutable structure
            // For now, we skip transforming f-strings as they would need to be rebuilt
            // TODO: Implement f-string transformation if needed
        }
        // Literals and name expressions that don't need transformation
        // - Literals don't contain variable references
        // - Name expressions that don't match the conditional pattern (e.g., Store context)
        Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_)
        | Expr::TString(_)
        | Expr::IpyEscapeCommand(_)
        | Expr::Name(_) => {}
    }
}

/// Transform a statement to use module attributes for module-level variables
fn transform_stmt_for_module_vars(
    stmt: &mut Stmt,
    module_level_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    match stmt {
        Stmt::FunctionDef(nested_func) => {
            // Recursively transform nested functions
            transform_nested_function_for_module_vars(
                nested_func,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Assign(assign) => {
            // Transform assignment targets and values
            for target in &mut assign.targets {
                transform_expr_for_module_vars(
                    target,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            transform_expr_for_module_vars(
                &mut assign.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Expr(expr_stmt) => {
            transform_expr_for_module_vars(
                &mut expr_stmt.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Return(return_stmt) => {
            if let Some(value) = &mut return_stmt.value {
                transform_expr_for_module_vars(
                    value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::If(if_stmt) => {
            transform_expr_for_module_vars(
                &mut if_stmt.test,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut if_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(condition) = &mut clause.test {
                    transform_expr_for_module_vars(
                        condition,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for stmt in &mut clause.body {
                    transform_stmt_for_module_vars(
                        stmt,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Stmt::For(for_stmt) => {
            transform_expr_for_module_vars(
                &mut for_stmt.target,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut for_stmt.iter,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut for_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for stmt in &mut for_stmt.orelse {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::While(while_stmt) => {
            transform_expr_for_module_vars(
                &mut while_stmt.test,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut while_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for stmt in &mut while_stmt.orelse {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::With(with_stmt) => {
            for item in &mut with_stmt.items {
                transform_expr_for_module_vars(
                    &mut item.context_expr,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                if let Some(ref mut optional_vars) = item.optional_vars {
                    transform_expr_for_module_vars(
                        optional_vars,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
            for stmt in &mut with_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::Try(try_stmt) => {
            for stmt in &mut try_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for handler in &mut try_stmt.handlers {
                let ExceptHandler::ExceptHandler(except_handler) = handler;
                if let Some(ref mut type_) = except_handler.type_ {
                    transform_expr_for_module_vars(
                        type_,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for stmt in &mut except_handler.body {
                    transform_stmt_for_module_vars(
                        stmt,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
            for stmt in &mut try_stmt.orelse {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for stmt in &mut try_stmt.finalbody {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::Raise(raise_stmt) => {
            if let Some(ref mut exc) = raise_stmt.exc {
                transform_expr_for_module_vars(
                    exc,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            if let Some(ref mut cause) = raise_stmt.cause {
                transform_expr_for_module_vars(
                    cause,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::ClassDef(class_def) => {
            // Transform decorators
            for decorator in &mut class_def.decorator_list {
                transform_expr_for_module_vars(
                    &mut decorator.expression,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            // Transform class arguments (base classes and keyword arguments)
            if let Some(ref mut arguments) = class_def.arguments {
                for arg in &mut arguments.args {
                    transform_expr_for_module_vars(
                        arg,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for keyword in &mut arguments.keywords {
                    transform_expr_for_module_vars(
                        &mut keyword.value,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
            // Transform class body
            for stmt in &mut class_def.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::AugAssign(aug_assign) => {
            transform_expr_for_module_vars(
                &mut aug_assign.target,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut aug_assign.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::AnnAssign(ann_assign) => {
            transform_expr_for_module_vars(
                &mut ann_assign.target,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut ann_assign.annotation,
                module_level_vars,
                module_var_name,
                python_version,
            );
            if let Some(ref mut value) = ann_assign.value {
                transform_expr_for_module_vars(
                    value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::Delete(delete_stmt) => {
            for target in &mut delete_stmt.targets {
                transform_expr_for_module_vars(
                    target,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::Match(match_stmt) => {
            transform_expr_for_module_vars(
                &mut match_stmt.subject,
                module_level_vars,
                module_var_name,
                python_version,
            );
            // Match cases have complex patterns that may need specialized handling
            // For now, we'll focus on transforming the guard expressions and bodies
            for case in &mut match_stmt.cases {
                if let Some(ref mut guard) = case.guard {
                    transform_expr_for_module_vars(
                        guard,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for stmt in &mut case.body {
                    transform_stmt_for_module_vars(
                        stmt,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Stmt::Assert(assert_stmt) => {
            transform_expr_for_module_vars(
                &mut assert_stmt.test,
                module_level_vars,
                module_var_name,
                python_version,
            );
            if let Some(ref mut msg) = assert_stmt.msg {
                transform_expr_for_module_vars(
                    msg,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::TypeAlias(_)
        | Stmt::Import(_)
        | Stmt::ImportFrom(_)
        | Stmt::Global(_)
        | Stmt::Nonlocal(_)
        | Stmt::Pass(_)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::IpyEscapeCommand(_) => {
            // These statement types don't contain expressions that need transformation
        }
    }
}

/// Context for transforming statements with module variable awareness
struct ModuleVarTransformContext<'a> {
    bundler: &'a Bundler<'a>,
    module_level_vars: &'a FxIndexSet<String>,
    module_var_name: &'a str,
    global_declarations: Option<&'a FxIndexMap<String, Vec<ruff_text_size::TextRange>>>,
    lifted_names: Option<&'a FxIndexMap<String, String>>,
    python_version: u8,
}

/// Transform a statement to use module attributes for module-level variables,
/// with awareness of lifted globals for nested functions
fn transform_stmt_for_module_vars_with_bundler(
    stmt: &mut Stmt,
    ctx: &ModuleVarTransformContext<'_>,
) {
    if let Stmt::FunctionDef(nested_func) = stmt {
        // For function definitions, use the global-aware transformation
        if let Some(globals_map) = ctx.global_declarations {
            ctx.bundler
                .transform_nested_function_for_module_vars_with_global_info(
                    nested_func,
                    ctx.module_level_vars,
                    globals_map,
                    ctx.lifted_names,
                    ctx.module_var_name,
                );
        } else {
            // Fallback to legacy path when no global info is available
            transform_nested_function_for_module_vars(
                nested_func,
                ctx.module_level_vars,
                ctx.module_var_name,
                ctx.python_version,
            );
        }
        return;
    }
    // Non-function statements: reuse the existing traversal
    transform_stmt_for_module_vars(
        stmt,
        ctx.module_level_vars,
        ctx.module_var_name,
        ctx.python_version,
    );
}

/// Transform nested function to use module attributes for module-level variables
fn transform_nested_function_for_module_vars(
    func_def: &mut StmtFunctionDef,
    module_level_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    // Collect local variables defined in this function
    let mut local_vars = FxIndexSet::default();

    // Add function parameters to local variables
    for param in &func_def.parameters.args {
        local_vars.insert(param.parameter.name.to_string());
    }
    for param in &func_def.parameters.posonlyargs {
        local_vars.insert(param.parameter.name.to_string());
    }
    for param in &func_def.parameters.kwonlyargs {
        local_vars.insert(param.parameter.name.to_string());
    }
    if let Some(ref vararg) = func_def.parameters.vararg {
        local_vars.insert(vararg.name.to_string());
    }
    if let Some(ref kwarg) = func_def.parameters.kwarg {
        local_vars.insert(kwarg.name.to_string());
    }

    // Collect all local variables assigned in the function body
    collect_local_vars(&func_def.body, &mut local_vars);

    // Transform the function body, excluding local variables
    for stmt in &mut func_def.body {
        transform_stmt_for_module_vars_with_locals(
            stmt,
            module_level_vars,
            &local_vars,
            module_var_name,
            python_version,
        );
    }
}

/// Collect local variables defined in a list of statements
fn collect_local_vars(stmts: &[Stmt], local_vars: &mut FxIndexSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Assign(assign) => {
                // Collect assignment targets as local variables
                for target in &assign.targets {
                    if let Expr::Name(name) = target {
                        local_vars.insert(name.id.to_string());
                    }
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Collect annotated assignment targets
                if let Expr::Name(name) = ann_assign.target.as_ref() {
                    local_vars.insert(name.id.to_string());
                }
            }
            Stmt::For(for_stmt) => {
                // Collect for loop targets
                if let Expr::Name(name) = for_stmt.target.as_ref() {
                    local_vars.insert(name.id.to_string());
                }
                // Recursively collect from body
                collect_local_vars(&for_stmt.body, local_vars);
                collect_local_vars(&for_stmt.orelse, local_vars);
            }
            Stmt::If(if_stmt) => {
                // Recursively collect from branches
                collect_local_vars(&if_stmt.body, local_vars);
                for clause in &if_stmt.elif_else_clauses {
                    collect_local_vars(&clause.body, local_vars);
                }
            }
            Stmt::While(while_stmt) => {
                collect_local_vars(&while_stmt.body, local_vars);
                collect_local_vars(&while_stmt.orelse, local_vars);
            }
            Stmt::With(with_stmt) => {
                // Collect with statement targets
                for item in &with_stmt.items {
                    if let Some(ref optional_vars) = item.optional_vars
                        && let Expr::Name(name) = optional_vars.as_ref()
                    {
                        local_vars.insert(name.id.to_string());
                    }
                }
                collect_local_vars(&with_stmt.body, local_vars);
            }
            Stmt::Try(try_stmt) => {
                collect_local_vars(&try_stmt.body, local_vars);
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    // Collect exception name if present
                    if let Some(ref name) = eh.name {
                        local_vars.insert(name.to_string());
                    }
                    collect_local_vars(&eh.body, local_vars);
                }
                collect_local_vars(&try_stmt.orelse, local_vars);
                collect_local_vars(&try_stmt.finalbody, local_vars);
            }
            Stmt::FunctionDef(func_def) => {
                // Function definitions create local names
                local_vars.insert(func_def.name.to_string());
            }
            Stmt::ClassDef(class_def) => {
                // Class definitions create local names
                local_vars.insert(class_def.name.to_string());
            }
            _ => {
                // Other statements don't introduce new local variables
            }
        }
    }
}

/// Transform a statement with awareness of local variables
fn transform_stmt_for_module_vars_with_locals(
    stmt: &mut Stmt,
    module_level_vars: &FxIndexSet<String>,
    local_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    match stmt {
        Stmt::FunctionDef(nested_func) => {
            // Recursively transform nested functions
            transform_nested_function_for_module_vars(
                nested_func,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Assign(assign) => {
            // Transform assignment targets and values
            for target in &mut assign.targets {
                transform_expr_for_module_vars_with_locals(
                    target,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
            transform_expr_for_module_vars_with_locals(
                &mut assign.value,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Expr(expr_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut expr_stmt.value,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Return(return_stmt) => {
            if let Some(value) = &mut return_stmt.value {
                transform_expr_for_module_vars_with_locals(
                    value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::If(if_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut if_stmt.test,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut if_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(condition) = &mut clause.test {
                    transform_expr_for_module_vars_with_locals(
                        condition,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for stmt in &mut clause.body {
                    transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Stmt::For(for_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut for_stmt.target,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut for_stmt.iter,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut for_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::While(while_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut while_stmt.test,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut while_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        _ => {
            // Handle other statement types as needed
        }
    }
}

/// Transform an expression with awareness of local variables
fn transform_expr_for_module_vars_with_locals(
    expr: &mut Expr,
    module_level_vars: &FxIndexSet<String>,
    local_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    match expr {
        Expr::Name(name_expr) => {
            let name_str = name_expr.id.as_str();

            // Special case: transform __name__ to module.__name__
            if name_str == "__name__" && matches!(name_expr.ctx, ExprContext::Load) {
                // Transform __name__ -> module.__name__
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name(module_var_name, ExprContext::Load),
                    "__name__",
                    ExprContext::Load,
                );
            }
            // If this is a module-level variable being read AND NOT a local variable AND NOT a
            // builtin, transform to module.var
            else if module_level_vars.contains(name_str)
                && !local_vars.contains(name_str)
                && !ruff_python_stdlib::builtins::is_python_builtin(name_str, python_version, false)
                && matches!(name_expr.ctx, ExprContext::Load)
            {
                // Transform foo -> module.foo
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name(module_var_name, ExprContext::Load),
                    name_str,
                    ExprContext::Load,
                );
            }
        }
        Expr::Call(call) => {
            transform_expr_for_module_vars_with_locals(
                &mut call.func,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            for arg in &mut call.arguments.args {
                transform_expr_for_module_vars_with_locals(
                    arg,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
            for keyword in &mut call.arguments.keywords {
                transform_expr_for_module_vars_with_locals(
                    &mut keyword.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::BinOp(binop) => {
            transform_expr_for_module_vars_with_locals(
                &mut binop.left,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut binop.right,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(key) = &mut item.key {
                    transform_expr_for_module_vars_with_locals(
                        key,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                        python_version,
                    );
                }
                transform_expr_for_module_vars_with_locals(
                    &mut item.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_expr_for_module_vars_with_locals(
                    elem,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Attribute(attr) => {
            transform_expr_for_module_vars_with_locals(
                &mut attr.value,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Subscript(subscript) => {
            transform_expr_for_module_vars_with_locals(
                &mut subscript.value,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut subscript.slice,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        _ => {
            // Handle other expression types as needed
        }
    }
}

// pub fn create_module_object_stmt(module_name: &str, _module_path: &Path) -> Vec<Stmt> {
//     let module_call = ast_builder::expressions::call(
//         ast_builder::expressions::simple_namespace_ctor(),
//         vec![],
//         vec![],
//     );
//
//     vec![
//         // self = types.SimpleNamespace()
//         ast_builder::statements::simple_assign("self", module_call),
//         // self.__name__ = "module_name"
//         ast_builder::statements::assign_attribute(
//             "self",
//             "__name__",
//             ast_builder::expressions::string_literal(module_name),
//         ),
//     ]
// }

/// Transform AST to use lifted globals
/// This is a thin wrapper around the bundler method to maintain module boundaries
pub(crate) fn transform_ast_with_lifted_globals(
    bundler: &Bundler<'_>,
    ast: &mut ModModule,
    lifted_names: &FxIndexMap<String, String>,
    global_info: &crate::symbol_conflict_resolver::ModuleGlobalInfo,
    module_name: Option<&str>,
) {
    bundler.transform_ast_with_lifted_globals(ast, lifted_names, global_info, module_name);
}

/// Transform expressions to handle built-in name shadowing in init functions
/// When a built-in name like 'str' is assigned as a local variable in the function,
/// any reference to that built-in before the assignment needs to use __builtins__.name
pub(crate) fn transform_expr_for_builtin_shadowing(
    expr: &mut Expr,
    builtin_locals: &FxIndexSet<String>,
) {
    match expr {
        Expr::Name(name) if name.ctx == ExprContext::Load => {
            // If this name refers to a built-in that will be shadowed by a local assignment,
            // transform it to use __builtins__.name
            if builtin_locals.contains(name.id.as_str()) {
                debug!(
                    "Transforming built-in reference '{}' to avoid UnboundLocalError",
                    name.id
                );
                // Use builtins module which is more reliable than __builtins__
                // Generate: __import__('builtins').name
                let import_call = ast_builder::expressions::call(
                    ast_builder::expressions::name("__import__", ExprContext::Load),
                    vec![ast_builder::expressions::string_literal("builtins")],
                    vec![],
                );
                *expr = ast_builder::expressions::attribute(
                    import_call,
                    name.id.as_str(),
                    ExprContext::Load,
                );
            }
        }
        // Recursively handle other expressions
        Expr::Call(call) => {
            transform_expr_for_builtin_shadowing(&mut call.func, builtin_locals);
            for arg in &mut call.arguments.args {
                transform_expr_for_builtin_shadowing(arg, builtin_locals);
            }
            for kw in &mut call.arguments.keywords {
                transform_expr_for_builtin_shadowing(&mut kw.value, builtin_locals);
            }
        }
        Expr::Attribute(attr) => {
            transform_expr_for_builtin_shadowing(&mut attr.value, builtin_locals);
        }
        Expr::Tuple(tuple) => {
            for elem in &mut tuple.elts {
                transform_expr_for_builtin_shadowing(elem, builtin_locals);
            }
        }
        Expr::List(list) => {
            for elem in &mut list.elts {
                transform_expr_for_builtin_shadowing(elem, builtin_locals);
            }
        }
        Expr::BinOp(binop) => {
            transform_expr_for_builtin_shadowing(&mut binop.left, builtin_locals);
            transform_expr_for_builtin_shadowing(&mut binop.right, builtin_locals);
        }
        Expr::UnaryOp(unaryop) => {
            transform_expr_for_builtin_shadowing(&mut unaryop.operand, builtin_locals);
        }
        Expr::If(if_expr) => {
            transform_expr_for_builtin_shadowing(&mut if_expr.test, builtin_locals);
            transform_expr_for_builtin_shadowing(&mut if_expr.body, builtin_locals);
            transform_expr_for_builtin_shadowing(&mut if_expr.orelse, builtin_locals);
        }
        Expr::Lambda(lambda) => {
            // Don't transform inside lambda bodies as they have their own scope
            // Only transform default arguments
            if let Some(ref mut params) = lambda.parameters {
                for arg in &mut params.args {
                    if let Some(ref mut default) = arg.default {
                        transform_expr_for_builtin_shadowing(default, builtin_locals);
                    }
                }
                for arg in &mut params.posonlyargs {
                    if let Some(ref mut default) = arg.default {
                        transform_expr_for_builtin_shadowing(default, builtin_locals);
                    }
                }
                for arg in &mut params.kwonlyargs {
                    if let Some(ref mut default) = arg.default {
                        transform_expr_for_builtin_shadowing(default, builtin_locals);
                    }
                }
            }
        }
        Expr::Compare(compare) => {
            transform_expr_for_builtin_shadowing(&mut compare.left, builtin_locals);
            for comparator in &mut compare.comparators {
                transform_expr_for_builtin_shadowing(comparator, builtin_locals);
            }
        }
        Expr::Subscript(subscript) => {
            transform_expr_for_builtin_shadowing(&mut subscript.value, builtin_locals);
            transform_expr_for_builtin_shadowing(&mut subscript.slice, builtin_locals);
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(ref mut key) = item.key {
                    transform_expr_for_builtin_shadowing(key, builtin_locals);
                }
                transform_expr_for_builtin_shadowing(&mut item.value, builtin_locals);
            }
        }
        Expr::Set(set) => {
            for elem in &mut set.elts {
                transform_expr_for_builtin_shadowing(elem, builtin_locals);
            }
        }
        Expr::ListComp(comp) => {
            // Only transform the iterator - the comprehension body has its own scope
            for generator in &mut comp.generators {
                transform_expr_for_builtin_shadowing(&mut generator.iter, builtin_locals);
            }
        }
        Expr::SetComp(comp) => {
            for generator in &mut comp.generators {
                transform_expr_for_builtin_shadowing(&mut generator.iter, builtin_locals);
            }
        }
        Expr::DictComp(comp) => {
            for generator in &mut comp.generators {
                transform_expr_for_builtin_shadowing(&mut generator.iter, builtin_locals);
            }
        }
        Expr::Generator(gen_expr) => {
            for generator in &mut gen_expr.generators {
                transform_expr_for_builtin_shadowing(&mut generator.iter, builtin_locals);
            }
        }
        Expr::BoolOp(boolop) => {
            for value in &mut boolop.values {
                transform_expr_for_builtin_shadowing(value, builtin_locals);
            }
        }
        Expr::Await(await_expr) => {
            transform_expr_for_builtin_shadowing(&mut await_expr.value, builtin_locals);
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                transform_expr_for_builtin_shadowing(value, builtin_locals);
            }
        }
        Expr::YieldFrom(yield_from) => {
            transform_expr_for_builtin_shadowing(&mut yield_from.value, builtin_locals);
        }
        Expr::Starred(starred) => {
            transform_expr_for_builtin_shadowing(&mut starred.value, builtin_locals);
        }
        Expr::Named(named) => {
            transform_expr_for_builtin_shadowing(&mut named.value, builtin_locals);
        }
        Expr::Slice(slice) => {
            if let Some(ref mut lower) = slice.lower {
                transform_expr_for_builtin_shadowing(lower, builtin_locals);
            }
            if let Some(ref mut upper) = slice.upper {
                transform_expr_for_builtin_shadowing(upper, builtin_locals);
            }
            if let Some(ref mut step) = slice.step {
                transform_expr_for_builtin_shadowing(step, builtin_locals);
            }
        }
        Expr::FString(_fstring) => {
            // F-strings are immutable in ruff AST and require special handling
            // We would need to rebuild the entire f-string structure to transform expressions
            // inside interpolations. For now, we skip transforming f-strings as they are less
            // likely to reference shadowed built-ins before assignment.
            // TODO: Implement f-string transformation if needed by reconstructing the f-string
        }
        _ => {
            // Other expression types don't need transformation
        }
    }
}

/// Helper function to determine if a symbol should be included in the module namespace
pub(crate) fn should_include_symbol(
    bundler: &Bundler<'_>,
    symbol_name: &str,
    module_name: &str,
    module_scope_symbols: Option<&FxIndexSet<String>>,
) -> bool {
    // If we have module_scope_symbols, check if the symbol is in that set
    // But also check special cases
    if let Some(symbols) = module_scope_symbols {
        if symbols.contains(symbol_name) {
            return true;
        }
        // Even if not in module_scope_symbols, check if it's a private symbol imported by others
        if symbol_name.starts_with('_')
            && let Some(module_asts) = &bundler.module_asts
            && let Some(module_id) = bundler.resolver.get_module_id_by_name(module_name)
            && crate::analyzers::ImportAnalyzer::is_symbol_imported_by_other_modules(
                module_asts,
                module_id,
                symbol_name,
                Some(&bundler.module_exports),
                bundler.resolver,
            )
        {
            log::debug!(
                "Private symbol '{symbol_name}' from module '{module_name}' is not in \
                 module_scope_symbols but is imported by other modules, so including it"
            );
            return true;
        }
        // Also include all-caps constants as they're often used internally in comprehensions
        // and other module-level code. Include digits to handle constants like HTTP2, TLS1_3, etc.
        let is_constant_like = symbol_name.len() > 1
            && symbol_name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
            && symbol_name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if is_constant_like {
            log::debug!(
                "Constant '{symbol_name}' from module '{module_name}' is not in \
                 module_scope_symbols but including as it appears to be a constant"
            );
            return true;
        }

        // Include common dunder variables that are often expected to be visible on modules
        const COMMON_DUNDERS: &[&str] = &[
            "__version__",
            "__author__",
            "__license__",
            "__description__",
            "__doc__",
            "__all__",
        ];
        if COMMON_DUNDERS.contains(&symbol_name) {
            log::debug!(
                "Common dunder '{symbol_name}' from module '{module_name}' is not in \
                 module_scope_symbols but including as it's a standard module attribute"
            );
            return true;
        }

        false
    } else {
        // No module_scope_symbols provided, use bundler's should_export_symbol
        bundler.should_export_symbol(symbol_name, module_name)
    }
}

/// Add module attribute assignment if the symbol should be exported
pub(crate) fn add_module_attr_if_exported(
    bundler: &Bundler<'_>,
    assign: &StmtAssign,
    module_name: &str,
    body: &mut Vec<Stmt>,
    module_scope_symbols: Option<&FxIndexSet<String>>,
) {
    if let Some(name) = expression_handlers::extract_simple_assign_target(assign) {
        emit_module_attr_if_exportable(
            bundler,
            &name,
            module_name,
            body,
            module_scope_symbols,
            None, // No lifted_names check needed for regular assigns
        );
    }
}

/// Helper to emit module attribute if a symbol should be exported
/// This centralizes the logic for both Assign and `AnnAssign` paths
pub(crate) fn emit_module_attr_if_exportable(
    bundler: &Bundler<'_>,
    symbol_name: &str,
    module_name: &str,
    body: &mut Vec<Stmt>,
    module_scope_symbols: Option<&FxIndexSet<String>>,
    lifted_names: Option<&FxIndexMap<String, String>>,
) {
    // Check if this is a lifted variable (only relevant for AnnAssign)
    if let Some(names) = lifted_names
        && names.contains_key(symbol_name)
    {
        debug!("Symbol '{symbol_name}' is a lifted variable, skipping module attribute");
        return;
    }

    let should_export =
        should_include_symbol(bundler, symbol_name, module_name, module_scope_symbols);
    debug!(
        "Symbol '{symbol_name}' in module '{module_name}' should_include_symbol returned: \
         {should_export}"
    );

    if should_export {
        debug!("Adding module attribute for symbol '{symbol_name}'");
        body.push(
            crate::code_generator::module_registry::create_module_attr_assignment(
                SELF_PARAM,
                symbol_name,
            ),
        );
    }
}

/// Create namespace for inlined submodule
pub(crate) fn create_namespace_for_inlined_submodule(
    bundler: &Bundler<'_>,
    full_module_name: &str,
    attr_name: &str,
    parent_module_var: &str,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    // Use the sanitized module name for inlined modules to match the global namespace object
    let namespace_var = sanitize_module_name_for_identifier(full_module_name);

    log::debug!(
        "create_namespace_for_inlined_submodule: full_module_name='{full_module_name}', \
         attr_name='{attr_name}', namespace_var='{namespace_var}'"
    );

    // Create a types.SimpleNamespace() for the inlined module
    stmts.push(ast_builder::statements::assign(
        vec![ast_builder::expressions::name(
            &namespace_var,
            ExprContext::Store,
        )],
        ast_builder::expressions::call(
            ast_builder::expressions::simple_namespace_ctor(),
            vec![],
            vec![],
        ),
    ));

    // Get the module ID for this module
    let module_id = bundler
        .resolver
        .get_module_id_by_name(full_module_name)
        .expect("Module should exist in resolver");

    // Get the module exports for this inlined module
    let exported_symbols = bundler.module_exports.get(&module_id).cloned().flatten();

    // Add all exported symbols from the inlined module to the namespace
    if let Some(exports) = exported_symbols {
        for symbol in exports {
            // For re-exported symbols, check if the original symbol is kept by tree-shaking
            let should_include = if bundler.tree_shaking_keep_symbols.is_some() {
                // First check if this symbol is directly defined in this module
                if bundler.is_symbol_kept_by_tree_shaking(module_id, &symbol) {
                    true
                } else {
                    // If not, check if this is a re-exported symbol from another module
                    // For modules with __all__, we always include symbols that are re-exported
                    // even if they're not directly defined in the module
                    let module_has_all_export = bundler
                        .module_exports
                        .get(&module_id)
                        .and_then(|exports| exports.as_ref())
                        .is_some_and(|exports| exports.contains(&symbol));

                    if module_has_all_export {
                        log::debug!(
                            "Including re-exported symbol {symbol} from module {full_module_name} \
                             (in __all__)"
                        );
                        true
                    } else {
                        false
                    }
                }
            } else {
                // No tree-shaking, include everything
                true
            };

            if !should_include {
                log::debug!(
                    "Skipping namespace assignment for {full_module_name}.{symbol} - removed by \
                     tree-shaking"
                );
                continue;
            }

            // Get the renamed version of this symbol
            let renamed_symbol = if let Some(module_id) = bundler.get_module_id(full_module_name)
                && let Some(module_renames) = symbol_renames.get(&module_id)
            {
                module_renames
                    .get(&symbol)
                    .cloned()
                    .unwrap_or_else(|| symbol.clone())
            } else {
                symbol.clone()
            };

            // Before creating the assignment, check if the renamed symbol exists after
            // tree-shaking
            if !renamed_symbol_exists(bundler, &renamed_symbol, symbol_renames) {
                log::warn!(
                    "Skipping namespace assignment {namespace_var}.{symbol} = {renamed_symbol} - \
                     renamed symbol doesn't exist after tree-shaking"
                );
                continue;
            }

            // namespace_var.symbol = renamed_symbol
            log::debug!(
                "Creating namespace assignment: {namespace_var}.{symbol} = {renamed_symbol}"
            );
            stmts.push(ast_builder::statements::assign(
                vec![ast_builder::expressions::attribute(
                    ast_builder::expressions::name(&namespace_var, ExprContext::Load),
                    &symbol,
                    ExprContext::Store,
                )],
                ast_builder::expressions::name(&renamed_symbol, ExprContext::Load),
            ));
        }
    } else {
        // If no explicit exports, we still need to check if this module defines symbols
        // This is a fallback for modules that don't have __all__ defined
        // For now, log a warning since we can't determine exports without module analysis
        log::warn!(
            "Inlined module '{full_module_name}' has no explicit exports (__all__). Namespace \
             will be empty unless symbols are added elsewhere."
        );
    }

    // Finally, set module.attr_name = namespace_var (e.g., module.compat = pkg_compat)
    // This allows the parent module to access the submodule via the expected attribute name
    stmts.push(
        crate::code_generator::module_registry::create_module_attr_assignment_with_value(
            parent_module_var,
            attr_name,
            &namespace_var,
        ),
    );

    stmts
}

/// Check if a renamed symbol exists after tree-shaking
fn renamed_symbol_exists(
    bundler: &Bundler<'_>,
    renamed_symbol: &str,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
) -> bool {
    // If not using tree-shaking, all symbols exist
    if bundler.tree_shaking_keep_symbols.is_none() {
        return true;
    }

    // Check all modules to see if any have this renamed symbol
    for (module_id, renames) in symbol_renames {
        for (original, renamed) in renames {
            if renamed == renamed_symbol {
                // Found the renamed symbol, check if it's kept
                if bundler.is_symbol_kept_by_tree_shaking(*module_id, original) {
                    return true;
                }
            }
        }
    }

    false
}

/// Process wildcard import from an inlined module
/// Returns a list of symbols from wrapper modules that need deferred assignment
pub(crate) fn process_wildcard_import(
    bundler: &Bundler<'_>,
    module: &str,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    imports_from_inlined: &mut Vec<(String, String, Option<String>)>,
    current_module: &str,
) -> Vec<(String, String)> {
    debug!("Processing wildcard import from inlined module '{module}'");

    // Track symbols from wrapper modules that need deferred handling
    let mut wrapper_module_symbols = Vec::new();

    // Get all exported symbols from this module
    let module_id = bundler.get_module_id(module);
    let exports = module_id.and_then(|id| bundler.module_exports.get(&id));

    if let Some(Some(export_list)) = exports {
        let module_id = module_id.expect("Module ID should exist if exports found");

        // Module has explicit __all__, use it
        // Determine if the importing module accesses __all__ dynamically
        let importer_accesses_all = bundler.get_module_id(current_module).is_some_and(|id| {
            bundler
                .modules_with_accessed_all
                .iter()
                .any(|(mid, _)| *mid == id)
        });

        // Detect if importing module is a package (__init__.py)
        let importer_is_package = bundler
            .get_module_id(current_module)
            .and_then(|id| bundler.resolver.get_module(id))
            .is_some_and(|m| m.is_package);

        for symbol in export_list {
            if symbol != "*" {
                // A symbol is kept if it's kept in the re-exporting module itself,
                // or if it's re-exported from a submodule and kept in that source module.
                let is_kept_final = importer_accesses_all
                    || importer_is_package
                    || bundler.is_symbol_kept_by_tree_shaking(module_id, symbol)
                    || {
                        let mut found_in_submodule = false;
                        for (potential_module_id, module_exports) in &bundler.module_exports {
                            // Check if this is a submodule by comparing names
                            let potential_module_name = bundler
                                .resolver
                                .get_module_name(*potential_module_id)
                                .expect("Module name must exist");
                            if potential_module_name.starts_with(&format!("{module}."))
                                && let Some(exports) = module_exports
                                && exports.contains(symbol)
                                && bundler
                                    .is_symbol_kept_by_tree_shaking(*potential_module_id, symbol)
                            {
                                debug!(
                                    "Symbol '{symbol}' is kept in source module \
                                     '{potential_module_name}'"
                                );
                                found_in_submodule = true;
                                break;
                            }
                        }
                        found_in_submodule
                    };

                if is_kept_final {
                    // Check if this symbol comes from a wrapper module
                    // If it does, we should NOT add it as a module attribute immediately
                    // because the wrapper module hasn't been initialized yet
                    if symbol_comes_from_wrapper_module(bundler, module, symbol) {
                        debug!(
                            "Symbol '{symbol}' from inlined module '{module}' comes from a \
                             wrapper module - deferring assignment"
                        );
                        // Track for deferred assignment after wrapper module initialization
                        let value_name = bundler
                            .get_module_id(module)
                            .and_then(|id| symbol_renames.get(&id))
                            .and_then(|m| m.get(symbol))
                            .cloned()
                            .unwrap_or_else(|| symbol.clone());
                        wrapper_module_symbols.push((symbol.clone(), value_name));
                        continue;
                    }

                    // Get the actual value name (might be renamed to avoid collisions)
                    let value_name = bundler
                        .get_module_id(module)
                        .and_then(|id| symbol_renames.get(&id))
                        .and_then(|m| m.get(symbol))
                        .cloned()
                        .unwrap_or_else(|| symbol.clone());

                    debug!(
                        "Tracking wildcard-imported symbol '{symbol}' (value: '{value_name}') \
                         from inlined module '{module}'"
                    );
                    // Track the source module for proper namespace access
                    imports_from_inlined.push((
                        symbol.clone(),
                        value_name,
                        Some(module.to_owned()),
                    ));
                } else {
                    debug!(
                        "Skipping wildcard-imported symbol '{symbol}' from inlined module \
                         '{module}' - removed by tree-shaking"
                    );
                }
            }
        }
        return wrapper_module_symbols;
    }

    if exports.is_some() {
        // Module exists but has no explicit __all__
        // Look at the symbol renames which contains all symbols from the module
        if let Some(module_id) = bundler.get_module_id(module)
            && let Some(renames) = symbol_renames.get(&module_id)
        {
            for (original_name, renamed_name) in renames {
                // Track the renamed symbol (which is what will be in the global scope)
                if !renamed_name.starts_with('_') {
                    // Check if the original symbol was kept by tree-shaking
                    if bundler.is_symbol_kept_by_tree_shaking(module_id, original_name) {
                        // Check if this symbol comes from a wrapper module
                        if symbol_comes_from_wrapper_module(bundler, module, original_name) {
                            debug!(
                                "Symbol '{original_name}' from inlined module '{module}' comes \
                                 from a wrapper module - deferring assignment"
                            );
                            // Track for deferred assignment
                            wrapper_module_symbols
                                .push((original_name.clone(), renamed_name.clone()));
                            continue;
                        }

                        debug!(
                            "Tracking wildcard-imported symbol '{renamed_name}' (renamed from \
                             '{original_name}') from inlined module '{module}'"
                        );
                        // For renamed symbols, use original as exported name, renamed as value
                        // Track the source module for proper namespace access
                        imports_from_inlined.push((
                            original_name.clone(),
                            renamed_name.clone(),
                            Some(module.to_owned()),
                        ));
                    } else {
                        debug!(
                            "Skipping wildcard-imported symbol '{renamed_name}' (renamed from \
                             '{original_name}') from inlined module '{module}' - removed by \
                             tree-shaking"
                        );
                    }
                }
            }
            return wrapper_module_symbols;
        }

        // Fallback to semantic exports when no renames are available
        if let Some(module_id) = bundler.get_module_id(module)
            && let Some(semantic) = bundler.semantic_exports.get(&module_id)
        {
            for symbol in semantic {
                if !symbol.starts_with('_') {
                    // Check if the symbol was kept by tree-shaking
                    if bundler.is_symbol_kept_by_tree_shaking(module_id, symbol) {
                        // Check if this symbol comes from a wrapper module
                        if symbol_comes_from_wrapper_module(bundler, module, symbol) {
                            debug!(
                                "Symbol '{symbol}' from inlined module '{module}' comes from a \
                                 wrapper module - deferring assignment"
                            );
                            // Track for deferred assignment
                            wrapper_module_symbols.push((symbol.clone(), symbol.clone()));
                            continue;
                        }

                        debug!(
                            "Tracking wildcard-imported symbol '{symbol}' (from semantic exports) \
                             from inlined module '{module}'"
                        );
                        // No rename, so exported name and value are the same
                        // Track the source module for proper namespace access
                        imports_from_inlined.push((
                            symbol.clone(),
                            symbol.clone(),
                            Some(module.to_owned()),
                        ));
                    } else {
                        debug!(
                            "Skipping wildcard-imported symbol '{symbol}' (from semantic exports) \
                             from inlined module '{module}' - removed by tree-shaking"
                        );
                    }
                }
            }
            return wrapper_module_symbols;
        }

        log::warn!(
            "No symbol renames or semantic exports found for inlined module '{module}' — wildcard \
             import may miss symbols"
        );
    } else {
        log::warn!("Could not find exports for inlined module '{module}'");
    }

    wrapper_module_symbols
}

/// Check if a symbol from an inlined module actually comes from a wrapper module
fn symbol_comes_from_wrapper_module(
    bundler: &Bundler<'_>,
    inlined_module: &str,
    symbol_name: &str,
) -> bool {
    // Find the module's AST in the module_asts if available
    let module_id = bundler.get_module_id(inlined_module);
    let module_data = module_id.and_then(|id| bundler.module_asts.as_ref()?.get(&id));

    if let Some((ast, module_path, _)) = module_data {
        // Check all import statements in the module
        for stmt in &ast.body {
            if let Stmt::ImportFrom(import_from) = stmt {
                // Check if this import includes our symbol
                for alias in &import_from.names {
                    let is_wildcard = alias.name.as_str() == "*";
                    let is_direct_import = alias.name.as_str() == symbol_name;

                    if is_wildcard || is_direct_import {
                        // Resolve the module this import is from
                        let resolved_module = if import_from.level > 0 {
                            // Relative import - need to resolve it
                            bundler.resolver.resolve_relative_to_absolute_module_name(
                                import_from.level,
                                import_from
                                    .module
                                    .as_ref()
                                    .map(ruff_python_ast::Identifier::as_str),
                                module_path,
                            )
                        } else {
                            import_from.module.as_ref().map(ToString::to_string)
                        };

                        let Some(ref source_module) = resolved_module else {
                            continue;
                        };

                        // Check if the source module is a wrapper module
                        let Some(source_module_id) = bundler.get_module_id(source_module) else {
                            continue;
                        };

                        if !bundler.bundled_modules.contains(&source_module_id)
                            || bundler.inlined_modules.contains(&source_module_id)
                        {
                            continue;
                        }

                        // For wildcard imports, verify the symbol is actually exported
                        if is_wildcard {
                            if let Some(Some(exports)) =
                                bundler.module_exports.get(&source_module_id)
                                && exports.iter().any(|s| s == symbol_name)
                            {
                                debug!(
                                    "Symbol '{symbol_name}' in inlined module '{inlined_module}' \
                                     comes from wrapper module '{source_module}' via wildcard \
                                     import"
                                );
                                return true;
                            }
                        } else {
                            // Direct import - we know this symbol comes from the wrapper module
                            debug!(
                                "Symbol '{symbol_name}' in inlined module '{inlined_module}' \
                                 comes from wrapper module '{source_module}'"
                            );
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}
