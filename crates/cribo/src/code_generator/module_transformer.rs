//! Module transformation logic for converting Python modules into init functions
//!
//! This module handles the complex transformation of Python module ASTs into
//! initialization functions that can be called to create module objects.

use std::path::Path;

use anyhow::Result;
use log::debug;
#[allow(unused_imports)] // These imports are used in pattern matching
use ruff_python_ast::{
    Arguments, AtomicNodeIndex, ExceptHandler, Expr, ExprAttribute, ExprAwait, ExprBoolOp,
    ExprCall, ExprCompare, ExprContext, ExprDictComp, ExprFString, ExprGenerator, ExprLambda,
    ExprListComp, ExprName, ExprNamed, ExprSet, ExprSetComp, ExprSlice, ExprStarred,
    ExprStringLiteral, ExprYield, ExprYieldFrom, Identifier, ModModule, Stmt, StmtAnnAssign,
    StmtAssert, StmtAssign, StmtAugAssign, StmtClassDef, StmtDelete, StmtFunctionDef, StmtGlobal,
    StmtMatch, StmtRaise, StmtReturn, StmtTry, StmtWhile, StmtWith, StringLiteral,
    StringLiteralFlags, StringLiteralValue,
};
use ruff_text_size::TextRange;

use crate::{
    ast_builder,
    code_generator::{
        bundler::HybridStaticBundler,
        context::ModuleTransformContext,
        globals::{GlobalsLifter, transform_globals_in_stmt},
        import_deduplicator,
        import_transformer::{
            RecursiveImportTransformer, RecursiveImportTransformerParams,
            resolve_relative_import_with_context,
        },
    },
    types::{FxIndexMap, FxIndexSet},
};

/// Transforms a module AST into an initialization function
pub fn transform_module_to_init_function<'a>(
    bundler: &'a HybridStaticBundler<'a>,
    ctx: ModuleTransformContext,
    mut ast: ModModule,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
) -> Result<Stmt> {
    let init_func_name = &bundler.init_functions[ctx.synthetic_name];
    let mut body = Vec::new();

    // Create module object (returns multiple statements)
    body.extend(create_module_object_stmt(ctx.module_name, ctx.module_path));

    // Apply globals lifting if needed
    let lifted_names = if let Some(ref global_info) = ctx.global_info {
        if !global_info.global_declarations.is_empty() {
            let globals_lifter = GlobalsLifter::new(global_info);
            let lifted_names = globals_lifter.get_lifted_names().clone();

            // Transform the AST to use lifted globals
            transform_ast_with_lifted_globals(bundler, &mut ast, &lifted_names, global_info);

            Some(lifted_names)
        } else {
            None
        }
    } else {
        None
    };

    // First, recursively transform all imports in the AST
    // For wrapper modules, we don't need to defer imports since they run in their own scope
    let mut wrapper_deferred_imports = Vec::new();
    let mut transformer = RecursiveImportTransformer::new(RecursiveImportTransformerParams {
        bundler,
        module_name: ctx.module_name,
        module_path: Some(ctx.module_path),
        symbol_renames,
        deferred_imports: &mut wrapper_deferred_imports,
        is_entry_module: false,        // This is not the entry module
        is_wrapper_init: true,         // This IS a wrapper init function
        global_deferred_imports: None, // No need for global deferred imports in wrapper modules
    });

    // Track imports from inlined modules before transformation
    let mut imports_from_inlined = Vec::new();
    let entry_path = bundler.entry_path.as_deref();
    let bundled_modules = &bundler.bundled_modules;

    for stmt in &ast.body {
        if let Stmt::ImportFrom(import_from) = stmt {
            // Resolve the module to check if it's inlined
            let resolved_module = resolve_relative_import_with_context(
                import_from,
                ctx.module_name,
                Some(ctx.module_path),
                entry_path,
                bundled_modules,
            );

            if let Some(ref module) = resolved_module {
                // Check if the module is bundled (either inlined or wrapper)
                let is_bundled = bundler.inlined_modules.contains(module)
                    || bundler.module_registry.contains_key(module);

                debug!(
                    "Checking if resolved module '{}' is bundled (inlined: {}, wrapper: {})",
                    module,
                    bundler.inlined_modules.contains(module),
                    bundler.module_registry.contains_key(module)
                );

                if is_bundled {
                    // Track all imported names from this bundled module
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        debug!(
                            "Tracking imported name '{imported_name}' from bundled module \
                             '{module}'"
                        );
                        imports_from_inlined.push(imported_name.to_string());
                    }
                }
            }
        }
    }

    transformer.transform_module(&mut ast);

    // If namespace objects were created, we need types import
    // (though wrapper modules already have types import)
    if transformer.created_namespace_objects() {
        debug!("Namespace objects were created in wrapper module, types import already present");
    }

    // Store deferred imports to add after module body
    let deferred_imports_to_add = wrapper_deferred_imports.clone();

    // IMPORTANT: Add import alias assignments FIRST, before processing the module body
    // This ensures that aliases like 'helper_validate = validate' are available when
    // the module body code tries to use them (e.g., helper_validate.__name__)
    for stmt in &deferred_imports_to_add {
        if let Stmt::Assign(assign) = stmt {
            // Check if this is a simple name-to-name assignment (import alias)
            if let [Expr::Name(_target)] = assign.targets.as_slice()
                && let Expr::Name(_value) = &*assign.value
            {
                // This is an import alias assignment, add it immediately
                body.push(stmt.clone());
            }
        }
    }

    // Collect all variables that are referenced by exported functions
    let mut vars_used_by_exported_functions: FxIndexSet<String> = FxIndexSet::default();
    for stmt in &ast.body {
        if let Stmt::FunctionDef(func_def) = stmt
            && bundler.should_export_symbol(func_def.name.as_ref(), ctx.module_name)
        {
            // This function will be exported, collect variables it references
            crate::visitors::VariableCollector::collect_referenced_vars(
                &func_def.body,
                &mut vars_used_by_exported_functions,
            );
        }
    }

    // Now process the transformed module
    // We'll do the in-place symbol export as we process each statement
    let module_scope_symbols = if let Some(semantic_bundler) = ctx.semantic_bundler {
        debug!(
            "Looking up module ID for '{}' in semantic bundler",
            ctx.module_name
        );
        // Use the central module registry for fast, reliable lookup
        let module_id = if let Some(registry) = bundler.module_info_registry {
            let id = registry.get_id_by_name(ctx.module_name);
            if id.is_some() {
                debug!(
                    "Found module ID for '{}' using module registry",
                    ctx.module_name
                );
            } else {
                debug!("Module '{}' not found in module registry", ctx.module_name);
            }
            id
        } else {
            log::warn!("No module registry available for module ID lookup");
            None
        };

        if let Some(module_id) = module_id {
            if let Some(module_info) = semantic_bundler.get_module_info(&module_id) {
                debug!(
                    "Found module-scope symbols for '{}': {:?}",
                    ctx.module_name, module_info.module_scope_symbols
                );
                Some(&module_info.module_scope_symbols)
            } else {
                log::warn!(
                    "No semantic info found for module '{}' (module_id: {:?})",
                    ctx.module_name,
                    module_id
                );
                None
            }
        } else {
            log::warn!(
                "Could not find module ID for '{}' in semantic bundler",
                ctx.module_name
            );
            None
        }
    } else {
        debug!(
            "No semantic bundler provided for module '{}'",
            ctx.module_name
        );
        None
    };

    // Process the body with a new recursive approach
    let processed_body =
        bundler.process_body_recursive(ast.body, ctx.module_name, module_scope_symbols);

    // Process each statement from the transformed module body
    for stmt in processed_body {
        match &stmt {
            Stmt::Import(_import_stmt) => {
                // Skip imports that are already hoisted
                if !import_deduplicator::is_hoisted_import(bundler, &stmt) {
                    body.push(stmt.clone());
                }
            }
            Stmt::ImportFrom(import_from) => {
                // Skip __future__ imports - they cannot appear inside functions
                if import_from.module.as_ref().map(|m| m.as_str()) == Some("__future__") {
                    continue;
                }

                // Skip imports that are already hoisted
                if !import_deduplicator::is_hoisted_import(bundler, &stmt) {
                    body.push(stmt.clone());
                }

                // Module attribute assignments for imported names are already handled by
                // process_body_recursive in the bundler, so we don't need to add them here
            }
            Stmt::ClassDef(class_def) => {
                // Add class definition
                body.push(stmt.clone());
                // Set as module attribute only if it should be exported
                let symbol_name = class_def.name.to_string();
                if bundler.should_export_symbol(&symbol_name, ctx.module_name) {
                    body.push(
                        crate::code_generator::module_registry::create_module_attr_assignment(
                            "module",
                            &symbol_name,
                        ),
                    );
                }
            }
            Stmt::FunctionDef(func_def) => {
                // Clone the function for transformation
                let mut func_def_clone = func_def.clone();

                // Transform nested functions to use module attributes for module-level vars
                if let Some(ref global_info) = ctx.global_info {
                    bundler.transform_nested_function_for_module_vars(
                        &mut func_def_clone,
                        &global_info.module_level_vars,
                    );
                }

                // Add transformed function definition
                body.push(Stmt::FunctionDef(func_def_clone));

                // Set as module attribute only if it should be exported
                let symbol_name = func_def.name.to_string();
                if bundler.should_export_symbol(&symbol_name, ctx.module_name) {
                    body.push(
                        crate::code_generator::module_registry::create_module_attr_assignment(
                            "module",
                            &symbol_name,
                        ),
                    );
                }
            }
            Stmt::Assign(assign) => {
                // Skip __all__ assignments - they have no meaning for types.SimpleNamespace
                if let Some(name) = bundler.extract_simple_assign_target(assign)
                    && name == "__all__"
                {
                    continue;
                }

                // Skip self-referential assignments like `process = process`
                // These are meaningless in the init function context and cause errors
                if !bundler.is_self_referential_assignment(assign) {
                    // Clone and transform the assignment to handle __name__ references
                    let mut assign_clone = assign.clone();
                    // Use actual module-level variables if available, but filter to only
                    // exported ones
                    let module_level_vars = if let Some(ref global_info) = ctx.global_info {
                        let all_vars = &global_info.module_level_vars;
                        let mut exported_vars = rustc_hash::FxHashSet::default();
                        for var in all_vars {
                            if bundler.should_export_symbol(var, ctx.module_name) {
                                exported_vars.insert(var.clone());
                            }
                        }
                        exported_vars
                    } else {
                        rustc_hash::FxHashSet::default()
                    };
                    transform_expr_for_module_vars(
                        &mut assign_clone.value,
                        &module_level_vars,
                        ctx.python_version,
                    );

                    // For simple assignments, also set as module attribute if it should be
                    // exported
                    body.push(Stmt::Assign(assign_clone));

                    // Check if this assignment came from a transformed import
                    if let Some(name) = bundler.extract_simple_assign_target(assign) {
                        debug!(
                            "Checking assignment '{}' in module '{}' (imports_from_inlined: {:?})",
                            name, ctx.module_name, imports_from_inlined
                        );
                        if imports_from_inlined.contains(&name) {
                            // This was imported from an inlined module, export it
                            debug!("Exporting imported symbol '{name}' as module attribute");
                            body.push(crate::code_generator::module_registry::create_module_attr_assignment("module", &name));
                        } else if let Some(name) = bundler.extract_simple_assign_target(assign) {
                            // Check if this variable is used by exported functions
                            if vars_used_by_exported_functions.contains(&name) {
                                debug!("Exporting '{name}' as it's used by exported functions");
                                body.push(crate::code_generator::module_registry::create_module_attr_assignment("module", &name));
                            } else {
                                // Regular assignment, use the normal export logic
                                add_module_attr_if_exported(
                                    bundler,
                                    assign,
                                    ctx.module_name,
                                    &mut body,
                                );
                            }
                        } else {
                            // Not a simple assignment
                            add_module_attr_if_exported(
                                bundler,
                                assign,
                                ctx.module_name,
                                &mut body,
                            );
                        }
                    }
                } else {
                    debug!(
                        "Skipping self-referential assignment in module '{}': {:?}",
                        ctx.module_name,
                        assign.targets.first().and_then(|t| match t {
                            Expr::Name(name) => Some(name.id.as_str()),
                            _ => None,
                        })
                    );
                }
            }
            Stmt::Try(_try_stmt) => {
                // Let the new conditional logic in bundler.rs handle try/except processing
                // This avoids duplicate module attribute assignments
                body.push(stmt.clone());
            }
            _ => {
                // Clone and transform other statements to handle __name__ references
                let mut stmt_clone = stmt.clone();
                // Use actual module-level variables if available, but filter to only exported
                // ones
                let module_level_vars = if let Some(ref global_info) = ctx.global_info {
                    let all_vars = &global_info.module_level_vars;
                    let mut exported_vars = rustc_hash::FxHashSet::default();
                    for var in all_vars {
                        if bundler.should_export_symbol(var, ctx.module_name) {
                            exported_vars.insert(var.clone());
                        }
                    }
                    exported_vars
                } else {
                    rustc_hash::FxHashSet::default()
                };
                transform_stmt_for_module_vars(
                    &mut stmt_clone,
                    &module_level_vars,
                    ctx.python_version,
                );
                body.push(stmt_clone);
            }
        }
    }

    // Initialize lifted globals if any
    if let Some(ref lifted_names) = lifted_names {
        for (original_name, lifted_name) in lifted_names {
            // global __cribo_module_var
            body.push(ast_builder::statements::global(vec![lifted_name]));

            // __cribo_module_var = original_var
            body.push(ast_builder::statements::assign(
                vec![ast_builder::expressions::name(
                    lifted_name,
                    ExprContext::Store,
                )],
                ast_builder::expressions::name(original_name, ExprContext::Load),
            ));
        }
    }

    // Set submodules as attributes on this module BEFORE processing deferred imports
    // This is needed because deferred imports may reference these submodules
    let current_module_prefix = format!("{}.", ctx.module_name);
    let mut submodules_to_add = Vec::new();

    // Collect all direct submodules
    for (module_name, _) in &bundler.module_registry {
        if module_name.starts_with(&current_module_prefix) {
            let relative_name = &module_name[current_module_prefix.len()..];
            // Only handle direct children, not nested submodules
            if !relative_name.contains('.') {
                submodules_to_add.push((module_name.clone(), relative_name.to_string()));
            }
        }
    }

    // Also check inlined modules
    for module_name in &bundler.inlined_modules {
        if module_name.starts_with(&current_module_prefix) {
            let relative_name = &module_name[current_module_prefix.len()..];
            // Only handle direct children, not nested submodules
            if !relative_name.contains('.') {
                submodules_to_add.push((module_name.clone(), relative_name.to_string()));
            }
        }
    }

    // Now add the submodules as attributes
    for (full_name, relative_name) in submodules_to_add {
        debug!(
            "Setting submodule {} as attribute {} on {}",
            full_name, relative_name, ctx.module_name
        );

        if bundler.inlined_modules.contains(&full_name) {
            // For inlined submodules, we create a types.SimpleNamespace with the exported
            // symbols
            let create_namespace_stmts = create_namespace_for_inlined_submodule(
                bundler,
                &full_name,
                &relative_name,
                symbol_renames,
            );
            body.extend(create_namespace_stmts);
        } else {
            // For wrapped submodules, we'll set them up later when they're initialized
            // For now, just skip - the parent module will get the submodule reference
            // when the submodule's init function is called
        }
    }

    // Add remaining deferred imports after submodule namespaces are created
    // Skip import alias assignments since they were already added at the beginning
    for stmt in &deferred_imports_to_add {
        // Skip simple name-to-name assignments (import aliases) as they were already added
        let is_import_alias = if let Stmt::Assign(assign) = stmt {
            matches!(
                (assign.targets.as_slice(), &*assign.value),
                ([Expr::Name(_)], Expr::Name(_))
            )
        } else {
            false
        };

        if is_import_alias {
            continue; // Already added at the beginning
        }

        if let Stmt::Assign(assign) = stmt
            && !bundler.is_self_referential_assignment(assign)
        {
            // For deferred imports that are assignments, also set as module attribute if
            // exported
            body.push(stmt.clone());
            add_module_attr_if_exported(bundler, assign, ctx.module_name, &mut body);
        } else {
            body.push(stmt.clone());
        }
    }

    // Skip __all__ generation - it has no meaning for types.SimpleNamespace objects

    // For imports from inlined modules that don't create assignments,
    // we still need to set them as module attributes if they're exported
    for imported_name in imports_from_inlined {
        if bundler.should_export_symbol(&imported_name, ctx.module_name) {
            // Check if we already have a module attribute assignment for this
            let already_assigned = body.iter().any(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && let [Expr::Attribute(attr)] = assign.targets.as_slice()
                    && let Expr::Name(name) = &*attr.value
                {
                    return name.id == "module" && attr.attr == imported_name;
                }
                false
            });

            if !already_assigned {
                body.push(
                    crate::code_generator::module_registry::create_module_attr_assignment(
                        "module",
                        &imported_name,
                    ),
                );
            }
        }
    }

    // Transform globals() calls to module.__dict__ in the entire body
    for stmt in &mut body {
        transform_globals_in_stmt(stmt);
    }

    // Return the module object
    body.push(ast_builder::statements::return_stmt(Some(
        ast_builder::expressions::name("module", ExprContext::Load),
    )));

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

/// Transform an expression to use module attributes for module-level variables
fn transform_expr_for_module_vars(
    expr: &mut Expr,
    module_level_vars: &rustc_hash::FxHashSet<String>,
    python_version: u8,
) {
    match expr {
        Expr::Name(name) if name.ctx == ExprContext::Load => {
            // Special case: transform __name__ to module.__name__
            if name.id.as_str() == "__name__" {
                // Transform __name__ -> module.__name__
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name("module", ExprContext::Load),
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
                    ast_builder::expressions::name("module", ExprContext::Load),
                    name.id.as_str(),
                    ExprContext::Load,
                );
            }
        }
        // Recursively handle other expressions
        Expr::Call(call) => {
            transform_expr_for_module_vars(&mut call.func, module_level_vars, python_version);
            for arg in &mut call.arguments.args {
                transform_expr_for_module_vars(arg, module_level_vars, python_version);
            }
            for kw in &mut call.arguments.keywords {
                transform_expr_for_module_vars(&mut kw.value, module_level_vars, python_version);
            }
        }
        Expr::Attribute(attr) => {
            transform_expr_for_module_vars(&mut attr.value, module_level_vars, python_version);
        }
        Expr::BinOp(binop) => {
            transform_expr_for_module_vars(&mut binop.left, module_level_vars, python_version);
            transform_expr_for_module_vars(&mut binop.right, module_level_vars, python_version);
        }
        Expr::UnaryOp(unop) => {
            transform_expr_for_module_vars(&mut unop.operand, module_level_vars, python_version);
        }
        Expr::If(if_expr) => {
            transform_expr_for_module_vars(&mut if_expr.test, module_level_vars, python_version);
            transform_expr_for_module_vars(&mut if_expr.body, module_level_vars, python_version);
            transform_expr_for_module_vars(&mut if_expr.orelse, module_level_vars, python_version);
        }
        Expr::List(list) => {
            for elem in &mut list.elts {
                transform_expr_for_module_vars(elem, module_level_vars, python_version);
            }
        }
        Expr::Tuple(tuple) => {
            for elem in &mut tuple.elts {
                transform_expr_for_module_vars(elem, module_level_vars, python_version);
            }
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(key) = &mut item.key {
                    transform_expr_for_module_vars(key, module_level_vars, python_version);
                }
                transform_expr_for_module_vars(&mut item.value, module_level_vars, python_version);
            }
        }
        Expr::Subscript(sub) => {
            transform_expr_for_module_vars(&mut sub.value, module_level_vars, python_version);
            transform_expr_for_module_vars(&mut sub.slice, module_level_vars, python_version);
        }
        Expr::Set(set) => {
            for elem in &mut set.elts {
                transform_expr_for_module_vars(elem, module_level_vars, python_version);
            }
        }
        Expr::Lambda(lambda) => {
            // Note: Lambda parameters create a new scope, so we don't transform them
            transform_expr_for_module_vars(&mut lambda.body, module_level_vars, python_version);
        }
        Expr::Compare(cmp) => {
            transform_expr_for_module_vars(&mut cmp.left, module_level_vars, python_version);
            for comp in &mut cmp.comparators {
                transform_expr_for_module_vars(comp, module_level_vars, python_version);
            }
        }
        Expr::BoolOp(boolop) => {
            for value in &mut boolop.values {
                transform_expr_for_module_vars(value, module_level_vars, python_version);
            }
        }
        Expr::ListComp(comp) => {
            transform_expr_for_module_vars(&mut comp.elt, module_level_vars, python_version);
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(if_clause, module_level_vars, python_version);
                }
            }
        }
        Expr::SetComp(comp) => {
            transform_expr_for_module_vars(&mut comp.elt, module_level_vars, python_version);
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(if_clause, module_level_vars, python_version);
                }
            }
        }
        Expr::DictComp(comp) => {
            transform_expr_for_module_vars(&mut comp.key, module_level_vars, python_version);
            transform_expr_for_module_vars(&mut comp.value, module_level_vars, python_version);
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(if_clause, module_level_vars, python_version);
                }
            }
        }
        Expr::Generator(r#gen) => {
            transform_expr_for_module_vars(&mut r#gen.elt, module_level_vars, python_version);
            for generator in &mut r#gen.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(if_clause, module_level_vars, python_version);
                }
            }
        }
        Expr::Await(await_expr) => {
            transform_expr_for_module_vars(
                &mut await_expr.value,
                module_level_vars,
                python_version,
            );
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                transform_expr_for_module_vars(value, module_level_vars, python_version);
            }
        }
        Expr::YieldFrom(yield_from) => {
            transform_expr_for_module_vars(
                &mut yield_from.value,
                module_level_vars,
                python_version,
            );
        }
        Expr::Starred(starred) => {
            transform_expr_for_module_vars(&mut starred.value, module_level_vars, python_version);
        }
        Expr::Named(named) => {
            transform_expr_for_module_vars(&mut named.value, module_level_vars, python_version);
        }
        Expr::Slice(slice) => {
            if let Some(ref mut lower) = slice.lower {
                transform_expr_for_module_vars(lower, module_level_vars, python_version);
            }
            if let Some(ref mut upper) = slice.upper {
                transform_expr_for_module_vars(upper, module_level_vars, python_version);
            }
            if let Some(ref mut step) = slice.step {
                transform_expr_for_module_vars(step, module_level_vars, python_version);
            }
        }
        Expr::FString(_fstring) => {
            // F-strings require special handling due to their immutable structure
            // For now, we skip transforming f-strings as they would need to be rebuilt
            // TODO: Implement f-string transformation if needed
        }
        // Literals don't contain variable references
        Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_)
        | Expr::TString(_)
        | Expr::IpyEscapeCommand(_) => {}
        // Name expressions that don't match the conditional pattern (e.g., Store context)
        Expr::Name(_) => {}
    }
}

/// Transform a statement to use module attributes for module-level variables
fn transform_stmt_for_module_vars(
    stmt: &mut Stmt,
    module_level_vars: &rustc_hash::FxHashSet<String>,
    python_version: u8,
) {
    match stmt {
        Stmt::FunctionDef(nested_func) => {
            // Recursively transform nested functions
            transform_nested_function_for_module_vars(
                nested_func,
                module_level_vars,
                python_version,
            );
        }
        Stmt::Assign(assign) => {
            // Transform assignment targets and values
            for target in &mut assign.targets {
                transform_expr_for_module_vars(target, module_level_vars, python_version);
            }
            transform_expr_for_module_vars(&mut assign.value, module_level_vars, python_version);
        }
        Stmt::Expr(expr_stmt) => {
            transform_expr_for_module_vars(&mut expr_stmt.value, module_level_vars, python_version);
        }
        Stmt::Return(return_stmt) => {
            if let Some(value) = &mut return_stmt.value {
                transform_expr_for_module_vars(value, module_level_vars, python_version);
            }
        }
        Stmt::If(if_stmt) => {
            transform_expr_for_module_vars(&mut if_stmt.test, module_level_vars, python_version);
            for stmt in &mut if_stmt.body {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(condition) = &mut clause.test {
                    transform_expr_for_module_vars(condition, module_level_vars, python_version);
                }
                for stmt in &mut clause.body {
                    transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
                }
            }
        }
        Stmt::For(for_stmt) => {
            transform_expr_for_module_vars(&mut for_stmt.target, module_level_vars, python_version);
            transform_expr_for_module_vars(&mut for_stmt.iter, module_level_vars, python_version);
            for stmt in &mut for_stmt.body {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
            for stmt in &mut for_stmt.orelse {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
        }
        Stmt::While(while_stmt) => {
            transform_expr_for_module_vars(&mut while_stmt.test, module_level_vars, python_version);
            for stmt in &mut while_stmt.body {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
            for stmt in &mut while_stmt.orelse {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
        }
        Stmt::With(with_stmt) => {
            for item in &mut with_stmt.items {
                transform_expr_for_module_vars(
                    &mut item.context_expr,
                    module_level_vars,
                    python_version,
                );
                if let Some(ref mut optional_vars) = item.optional_vars {
                    transform_expr_for_module_vars(
                        optional_vars,
                        module_level_vars,
                        python_version,
                    );
                }
            }
            for stmt in &mut with_stmt.body {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
        }
        Stmt::Try(try_stmt) => {
            for stmt in &mut try_stmt.body {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
            for handler in &mut try_stmt.handlers {
                let ExceptHandler::ExceptHandler(except_handler) = handler;
                if let Some(ref mut type_) = except_handler.type_ {
                    transform_expr_for_module_vars(type_, module_level_vars, python_version);
                }
                for stmt in &mut except_handler.body {
                    transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
                }
            }
            for stmt in &mut try_stmt.orelse {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
            for stmt in &mut try_stmt.finalbody {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
        }
        Stmt::Raise(raise_stmt) => {
            if let Some(ref mut exc) = raise_stmt.exc {
                transform_expr_for_module_vars(exc, module_level_vars, python_version);
            }
            if let Some(ref mut cause) = raise_stmt.cause {
                transform_expr_for_module_vars(cause, module_level_vars, python_version);
            }
        }
        Stmt::ClassDef(class_def) => {
            // Transform decorators
            for decorator in &mut class_def.decorator_list {
                transform_expr_for_module_vars(
                    &mut decorator.expression,
                    module_level_vars,
                    python_version,
                );
            }
            // Transform class arguments (base classes and keyword arguments)
            if let Some(ref mut arguments) = class_def.arguments {
                for arg in arguments.args.iter_mut() {
                    transform_expr_for_module_vars(arg, module_level_vars, python_version);
                }
                for keyword in arguments.keywords.iter_mut() {
                    transform_expr_for_module_vars(
                        &mut keyword.value,
                        module_level_vars,
                        python_version,
                    );
                }
            }
            // Transform class body
            for stmt in &mut class_def.body {
                transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
            }
        }
        Stmt::AugAssign(aug_assign) => {
            transform_expr_for_module_vars(
                &mut aug_assign.target,
                module_level_vars,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut aug_assign.value,
                module_level_vars,
                python_version,
            );
        }
        Stmt::AnnAssign(ann_assign) => {
            transform_expr_for_module_vars(
                &mut ann_assign.target,
                module_level_vars,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut ann_assign.annotation,
                module_level_vars,
                python_version,
            );
            if let Some(ref mut value) = ann_assign.value {
                transform_expr_for_module_vars(value, module_level_vars, python_version);
            }
        }
        Stmt::Delete(delete_stmt) => {
            for target in &mut delete_stmt.targets {
                transform_expr_for_module_vars(target, module_level_vars, python_version);
            }
        }
        Stmt::Match(match_stmt) => {
            transform_expr_for_module_vars(
                &mut match_stmt.subject,
                module_level_vars,
                python_version,
            );
            // Match cases have complex patterns that may need specialized handling
            // For now, we'll focus on transforming the guard expressions and bodies
            for case in &mut match_stmt.cases {
                if let Some(ref mut guard) = case.guard {
                    transform_expr_for_module_vars(guard, module_level_vars, python_version);
                }
                for stmt in &mut case.body {
                    transform_stmt_for_module_vars(stmt, module_level_vars, python_version);
                }
            }
        }
        Stmt::Assert(assert_stmt) => {
            transform_expr_for_module_vars(
                &mut assert_stmt.test,
                module_level_vars,
                python_version,
            );
            if let Some(ref mut msg) = assert_stmt.msg {
                transform_expr_for_module_vars(msg, module_level_vars, python_version);
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

/// Transform nested function to use module attributes for module-level variables
fn transform_nested_function_for_module_vars(
    func_def: &mut StmtFunctionDef,
    module_level_vars: &rustc_hash::FxHashSet<String>,
    python_version: u8,
) {
    // Collect local variables defined in this function
    let mut local_vars = rustc_hash::FxHashSet::default();

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
            python_version,
        );
    }
}

/// Collect local variables defined in a list of statements
fn collect_local_vars(stmts: &[Stmt], local_vars: &mut rustc_hash::FxHashSet<String>) {
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
    module_level_vars: &rustc_hash::FxHashSet<String>,
    local_vars: &rustc_hash::FxHashSet<String>,
    python_version: u8,
) {
    match stmt {
        Stmt::FunctionDef(nested_func) => {
            // Recursively transform nested functions
            transform_nested_function_for_module_vars(
                nested_func,
                module_level_vars,
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
                    python_version,
                );
            }
            transform_expr_for_module_vars_with_locals(
                &mut assign.value,
                module_level_vars,
                local_vars,
                python_version,
            );
        }
        Stmt::Expr(expr_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut expr_stmt.value,
                module_level_vars,
                local_vars,
                python_version,
            );
        }
        Stmt::Return(return_stmt) => {
            if let Some(value) = &mut return_stmt.value {
                transform_expr_for_module_vars_with_locals(
                    value,
                    module_level_vars,
                    local_vars,
                    python_version,
                );
            }
        }
        Stmt::If(if_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut if_stmt.test,
                module_level_vars,
                local_vars,
                python_version,
            );
            for stmt in &mut if_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
                    python_version,
                );
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(condition) = &mut clause.test {
                    transform_expr_for_module_vars_with_locals(
                        condition,
                        module_level_vars,
                        local_vars,
                        python_version,
                    );
                }
                for stmt in &mut clause.body {
                    transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
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
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut for_stmt.iter,
                module_level_vars,
                local_vars,
                python_version,
            );
            for stmt in &mut for_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
                    python_version,
                );
            }
        }
        Stmt::While(while_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut while_stmt.test,
                module_level_vars,
                local_vars,
                python_version,
            );
            for stmt in &mut while_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
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
    module_level_vars: &rustc_hash::FxHashSet<String>,
    local_vars: &rustc_hash::FxHashSet<String>,
    python_version: u8,
) {
    match expr {
        Expr::Name(name_expr) => {
            let name_str = name_expr.id.as_str();

            // Special case: transform __name__ to module.__name__
            if name_str == "__name__" && matches!(name_expr.ctx, ExprContext::Load) {
                // Transform __name__ -> module.__name__
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name("module", ExprContext::Load),
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
                    ast_builder::expressions::name("module", ExprContext::Load),
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
                python_version,
            );
            for arg in &mut call.arguments.args {
                transform_expr_for_module_vars_with_locals(
                    arg,
                    module_level_vars,
                    local_vars,
                    python_version,
                );
            }
            for keyword in &mut call.arguments.keywords {
                transform_expr_for_module_vars_with_locals(
                    &mut keyword.value,
                    module_level_vars,
                    local_vars,
                    python_version,
                );
            }
        }
        Expr::BinOp(binop) => {
            transform_expr_for_module_vars_with_locals(
                &mut binop.left,
                module_level_vars,
                local_vars,
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut binop.right,
                module_level_vars,
                local_vars,
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
                        python_version,
                    );
                }
                transform_expr_for_module_vars_with_locals(
                    &mut item.value,
                    module_level_vars,
                    local_vars,
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
                    python_version,
                );
            }
        }
        Expr::Attribute(attr) => {
            transform_expr_for_module_vars_with_locals(
                &mut attr.value,
                module_level_vars,
                local_vars,
                python_version,
            );
        }
        Expr::Subscript(subscript) => {
            transform_expr_for_module_vars_with_locals(
                &mut subscript.value,
                module_level_vars,
                local_vars,
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut subscript.slice,
                module_level_vars,
                local_vars,
                python_version,
            );
        }
        _ => {
            // Handle other expression types as needed
        }
    }
}

/// Create module object statements (types.SimpleNamespace)
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

/// Transform AST to use lifted globals
/// This is a thin wrapper around the bundler method to maintain module boundaries
pub fn transform_ast_with_lifted_globals(
    bundler: &HybridStaticBundler,
    ast: &mut ModModule,
    lifted_names: &FxIndexMap<String, String>,
    global_info: &crate::semantic_bundler::ModuleGlobalInfo,
) {
    bundler.transform_ast_with_lifted_globals(ast, lifted_names, global_info);
}

/// Add module attribute assignment if the symbol should be exported
fn add_module_attr_if_exported(
    bundler: &HybridStaticBundler,
    assign: &StmtAssign,
    module_name: &str,
    body: &mut Vec<Stmt>,
) {
    if let Some(name) = bundler.extract_simple_assign_target(assign)
        && bundler.should_export_symbol(&name, module_name)
    {
        body.push(
            crate::code_generator::module_registry::create_module_attr_assignment("module", &name),
        );
    }
}

/// Create namespace for inlined submodule
fn create_namespace_for_inlined_submodule(
    bundler: &HybridStaticBundler,
    full_module_name: &str,
    attr_name: &str,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    // Create a types.SimpleNamespace() for the inlined module
    stmts.push(ast_builder::statements::assign(
        vec![ast_builder::expressions::name(
            attr_name,
            ExprContext::Store,
        )],
        ast_builder::expressions::call(
            ast_builder::expressions::simple_namespace_ctor(),
            vec![],
            vec![],
        ),
    ));

    // Get the module exports for this inlined module
    let exported_symbols = bundler
        .module_exports
        .get(full_module_name)
        .cloned()
        .flatten();

    // Add all exported symbols from the inlined module to the namespace
    if let Some(exports) = exported_symbols {
        for symbol in exports {
            // For re-exported symbols, check if the original symbol is kept by tree-shaking
            let should_include = if let Some(ref kept_symbols) = bundler.tree_shaking_keep_symbols {
                // First check if this symbol is directly defined in this module
                if kept_symbols.contains(&(full_module_name.to_string(), symbol.clone())) {
                    true
                } else {
                    // If not, check if this is a re-exported symbol from another module
                    // For modules with __all__, we always include symbols that are re-exported
                    // even if they're not directly defined in the module
                    let module_has_all_export = bundler
                        .module_exports
                        .get(full_module_name)
                        .and_then(|exports| exports.as_ref())
                        .map(|exports| exports.contains(&symbol))
                        .unwrap_or(false);

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
            let renamed_symbol = if let Some(module_renames) = symbol_renames.get(full_module_name)
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
                    "Skipping namespace assignment {attr_name}.{symbol} = {renamed_symbol} - \
                     renamed symbol doesn't exist after tree-shaking"
                );
                continue;
            }

            // attr_name.symbol = renamed_symbol
            log::debug!("Creating namespace assignment: {attr_name}.{symbol} = {renamed_symbol}");
            stmts.push(ast_builder::statements::assign(
                vec![ast_builder::expressions::attribute(
                    ast_builder::expressions::name(attr_name, ExprContext::Load),
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

    stmts
}

/// Check if a renamed symbol exists after tree-shaking
fn renamed_symbol_exists(
    bundler: &HybridStaticBundler,
    renamed_symbol: &str,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
) -> bool {
    // If not using tree-shaking, all symbols exist
    let Some(ref kept_symbols) = bundler.tree_shaking_keep_symbols else {
        return true;
    };

    // Check all modules to see if any have this renamed symbol
    for (module, renames) in symbol_renames {
        for (original, renamed) in renames {
            if renamed == renamed_symbol {
                // Found the renamed symbol, check if it's kept
                if kept_symbols.contains(&(module.clone(), original.clone())) {
                    return true;
                }
            }
        }
    }

    false
}
