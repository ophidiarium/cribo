use ruff_python_ast::{Expr, ExprContext, Stmt, StmtImportFrom};

use crate::{
    ast_builder::{expressions, statements},
    code_generator::bundler::Bundler,
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

// Local copy of Bundler's ImportResolveParams (scoped for wrapper handler usage)
struct ImportResolveParams<'a> {
    module_expr: Expr,
    module_name: &'a str,
    imported_name: &'a str,
    at_module_level: bool,
    inside_wrapper_init: bool,
    current_module: Option<&'a str>,
    symbol_renames: &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
}

/// Handle wrapper module import transformations
pub struct WrapperHandler;

impl WrapperHandler {
    /// Same as `rewrite_from_import_for_wrapper_module` but accepts explicit context.
    pub(in crate::code_generator::import_transformer) fn rewrite_from_import_for_wrapper_module_with_context(
        bundler: &Bundler,
        import_from: &StmtImportFrom,
        module_name: &str,
        inside_wrapper_init: bool,
        at_module_level: bool,
        current_module: Option<&str>,
        symbol_renames: &crate::types::FxIndexMap<
            crate::resolver::ModuleId,
            crate::types::FxIndexMap<String, String>,
        >,
        function_body: Option<&[Stmt]>,
    ) -> Vec<Stmt> {
        log::debug!(
            "transform_bundled_import_from_multiple: module_name={}, imports={:?}, \
             inside_wrapper_init={}",
            module_name,
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            inside_wrapper_init
        );

        // Early dispatch: wildcard imports handled separately
        if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
            let context = crate::code_generator::bundler::BundledImportContext {
                inside_wrapper_init,
                at_module_level,
                current_module,
            };
            return bundler.transform_bundled_import_from_multiple_with_current_module(
                import_from,
                module_name,
                context,
                symbol_renames,
                function_body,
            );
        }

        // Defer to alias/symbol handling path
        let context = crate::code_generator::bundler::BundledImportContext {
            inside_wrapper_init,
            at_module_level,
            current_module,
        };
        bundler.transform_bundled_import_from_multiple_with_current_module(
            import_from,
            module_name,
            context,
            symbol_renames,
            function_body,
        )
    }

    /// Handle wildcard-from imports (`from X import *`) for wrapper modules
    pub(in crate::code_generator::import_transformer) fn handle_wildcard_import_from_multiple(
        bundler: &Bundler,
        _import_from: &StmtImportFrom,
        module_name: &str,
        inside_wrapper_init: bool,
        current_module: Option<&str>,
        at_module_level: bool,
    ) -> Vec<Stmt> {
        let mut assignments = Vec::new();

        if let Some(module_id) = bundler.get_module_id(module_name)
            && bundler.module_synthetic_names.contains_key(&module_id)
        {
            let current_module_id = current_module.and_then(|m| bundler.get_module_id(m));
            assignments.extend(
                bundler.create_module_initialization_for_import_with_current_module(
                    module_id,
                    current_module_id,
                    if inside_wrapper_init {
                        true
                    } else {
                        at_module_level
                    },
                ),
            );
        }

        let module_exports = if let Some(module_id) = bundler.get_module_id(module_name) {
            if let Some(Some(export_list)) = bundler.module_exports.get(&module_id) {
                export_list.clone()
            } else if let Some(semantic_exports) = bundler.semantic_exports.get(&module_id) {
                semantic_exports.iter().cloned().collect()
            } else {
                vec![]
            }
        } else {
            let module_expr = expressions::module_reference(module_name, ExprContext::Load);
            let attr_var = "__cribo_attr";
            let dir_call = expressions::call(
                expressions::name("dir", ExprContext::Load),
                vec![module_expr.clone()],
                vec![],
            );
            let for_loop = statements::for_loop(
                attr_var,
                dir_call,
                vec![statements::if_stmt(
                    expressions::unary_op(
                        ruff_python_ast::UnaryOp::Not,
                        expressions::call(
                            expressions::attribute(
                                expressions::name(attr_var, ExprContext::Load),
                                "startswith",
                                ExprContext::Load,
                            ),
                            vec![expressions::string_literal("_")],
                            vec![],
                        ),
                    ),
                    vec![statements::subscript_assign(
                        expressions::call(
                            expressions::name("globals", ExprContext::Load),
                            vec![],
                            vec![],
                        ),
                        expressions::name(attr_var, ExprContext::Load),
                        expressions::call(
                            expressions::name("getattr", ExprContext::Load),
                            vec![
                                module_expr.clone(),
                                expressions::name(attr_var, ExprContext::Load),
                            ],
                            vec![],
                        ),
                    )],
                    vec![],
                )],
                vec![],
            );
            assignments.push(for_loop);
            return assignments;
        };

        let module_expr = if module_name.contains('.') {
            let parts: Vec<&str> = module_name.split('.').collect();
            expressions::dotted_name(&parts, ExprContext::Load)
        } else {
            expressions::name(module_name, ExprContext::Load)
        };

        let explicit_all = bundler
            .get_module_id(module_name)
            .and_then(|id| bundler.module_exports.get(&id))
            .and_then(|exports| exports.as_ref());

        for symbol_name in &module_exports {
            if symbol_name.starts_with('_')
                && !explicit_all.is_some_and(|all| all.contains(symbol_name))
            {
                continue;
            }
            assignments.push(statements::simple_assign(
                symbol_name,
                expressions::attribute(module_expr.clone(), symbol_name, ExprContext::Load),
            ));
            if inside_wrapper_init && let Some(current_mod) = current_module {
                let module_var =
                    crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                        current_mod,
                    );
                assignments.push(
                    crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                        &module_var,
                        symbol_name,
                        symbol_name,
                    ),
                );
            }
        }

        assignments
    }

    /// Local copy of `Bundler::handle_symbol_imports_from_multiple` specialized for wrapper imports
    fn handle_symbol_imports_from_wrapper(
        bundler: &Bundler,
        import_from: &StmtImportFrom,
        module_name: &str,
        context: crate::code_generator::bundler::BundledImportContext<'_>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        function_body: Option<&[Stmt]>,
    ) -> Vec<Stmt> {
        let inside_wrapper_init = context.inside_wrapper_init;
        let at_module_level = context.at_module_level;
        let current_module = context.current_module;
        let mut assignments = Vec::new();
        let initialized_modules: FxIndexSet<ModuleId> = FxIndexSet::default();
        let mut locally_initialized: FxIndexSet<ModuleId> = FxIndexSet::default();

        let used_symbols = if let Some(body) = function_body {
            if at_module_level {
                None
            } else {
                Some(crate::visitors::SymbolUsageVisitor::collect_used_symbols(
                    body,
                ))
            }
        } else {
            None
        };

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let target_name = alias.asname.as_ref().unwrap_or(&alias.name);
            let full_module_path = format!("{module_name}.{imported_name}");

            let parent_is_wrapper = bundler.has_synthetic_name(module_name);
            let submodule_exists = bundler.get_module_id(&full_module_path).is_some_and(|id| {
                bundler.bundled_modules.contains(&id)
                    && (bundler.has_synthetic_name(&full_module_path)
                        || bundler.inlined_modules.contains(&id))
            });

            let importing_submodule = if parent_is_wrapper && submodule_exists {
                if let Some(Some(export_list)) = bundler
                    .get_module_id(module_name)
                    .and_then(|id| bundler.module_exports.get(&id))
                {
                    !export_list.contains(&imported_name.to_string())
                } else {
                    submodule_exists
                }
            } else {
                submodule_exists
            };

            if importing_submodule {
                log::debug!(
                    "Importing submodule '{imported_name}' from '{module_name}' via from import"
                );

                let is_submodule_of_target =
                    current_module.is_some_and(|curr| curr.starts_with(&format!("{module_name}.")));

                let parent_module_id = bundler.get_module_id(module_name);
                let should_initialize_parent = parent_module_id.is_some_and(|id| {
                    bundler.has_synthetic_name(module_name)
                        && !locally_initialized.contains(&id)
                        && (inside_wrapper_init || bundler.module_init_functions.contains_key(&id))
                });

                if should_initialize_parent {
                    assignments.extend(
                        bundler.create_module_initialization_for_import_with_current_module(
                            parent_module_id.expect("module id exists"),
                            current_module.and_then(|m| bundler.get_module_id(m)),
                            if inside_wrapper_init {
                                true
                            } else {
                                at_module_level
                            },
                        ),
                    );
                    if let Some(id) = parent_module_id {
                        locally_initialized.insert(id);
                    }
                }

                if let Some(submodule_id) = bundler.get_module_id(&full_module_path)
                    && bundler.has_synthetic_name(&full_module_path)
                    && !locally_initialized.contains(&submodule_id)
                {
                    let current_module_id = current_module.and_then(|m| bundler.get_module_id(m));
                    assignments.extend(
                        bundler.create_module_initialization_for_import_with_current_module(
                            submodule_id,
                            current_module_id,
                            if inside_wrapper_init {
                                true
                            } else {
                                at_module_level
                            },
                        ),
                    );
                    locally_initialized.insert(submodule_id);

                    let submodule_var =
                        crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                            &full_module_path,
                        );
                    let assignment = statements::simple_assign(
                        target_name.as_str(),
                        expressions::attribute(
                            expressions::name(&submodule_var, ExprContext::Load),
                            imported_name,
                            ExprContext::Load,
                        ),
                    );
                    assignments.push(assignment);
                    continue;
                }

                if bundler
                    .get_module_id(module_name)
                    .is_some_and(|id| bundler.inlined_modules.contains(&id))
                    && !inside_wrapper_init
                {
                    let full_submodule_path = format!("{module_name}.{imported_name}");
                    if bundler.has_synthetic_name(&full_submodule_path) {
                        log::warn!(
                            "Unexpected: importing wrapper submodule '{imported_name}' from \
                             inlined module '{module_name}' in \
                             transform_bundled_import_from_multiple - should have been deferred"
                        );
                        let namespace_expr = if full_submodule_path.contains('.') {
                            let parts: Vec<&str> = full_submodule_path.split('.').collect();
                            expressions::dotted_name(&parts, ExprContext::Load)
                        } else {
                            expressions::name(&full_submodule_path, ExprContext::Load)
                        };
                        assignments.push(statements::simple_assign(
                            target_name.as_str(),
                            namespace_expr,
                        ));
                        continue;
                    }
                }

                if let Some(ref used) = used_symbols
                    && !used.contains(target_name.as_str())
                {
                    let module_id = bundler.get_module_id(module_name);
                    let is_bundled_or_inlined = module_id.is_some_and(|id| {
                        bundler.bundled_modules.contains(&id)
                            || bundler.inlined_modules.contains(&id)
                    });
                    let is_wrapper =
                        module_id.is_some_and(|id| bundler.wrapper_modules.contains(&id));
                    if is_wrapper
                        && !locally_initialized.contains(
                            &module_id.expect("module_id should exist when is_wrapper is true"),
                        )
                    {
                        // keep
                    } else if is_bundled_or_inlined {
                        continue;
                    } else {
                        // keep
                    }
                }

                let needs_init = if let Some(mid) = bundler.get_module_id(module_name) {
                    let is_parent_of_current = current_module
                        .is_some_and(|curr| curr.starts_with(&format!("{module_name}.")));
                    bundler.has_synthetic_name(module_name)
                        && !locally_initialized.contains(&mid)
                        && current_module != Some(module_name)
                        && !is_parent_of_current
                        && (inside_wrapper_init || bundler.module_init_functions.contains_key(&mid))
                } else {
                    false
                };
                if needs_init {
                    let module_init_exists = assignments.iter().any(|stmt| {
                        if let Stmt::Assign(assign) = stmt
                            && assign.targets.len() == 1
                            && let Expr::Call(call) = &assign.value.as_ref()
                            && let Expr::Name(func_name) = &call.func.as_ref()
                            && crate::code_generator::module_registry::is_init_function(func_name.id.as_str())
                        {
                            match &assign.targets[0] {
                                Expr::Attribute(attr) => {
                                    let attr_path = crate::code_generator::expression_handlers::extract_attribute_path(attr);
                                    attr_path == module_name
                                }
                                Expr::Name(name) => name.id.as_str() == module_name,
                                _ => false,
                            }
                        } else { false }
                    });
                    if !module_init_exists
                        && let Some(mid) = bundler.get_module_id(module_name) {
                            let current_module_id =
                                current_module.and_then(|m| bundler.get_module_id(m));
                            if at_module_level {
                                assignments.extend(
                                    bundler.create_module_initialization_for_import_with_current_module(
                                        mid,
                                        current_module_id,
                                        at_module_level,
                                    ),
                                );
                                locally_initialized.insert(mid);
                            }
                        }
                }

                let canonical_module_name = bundler
                    .get_module_id(module_name)
                    .and_then(|id| bundler.resolver.get_module_name(id))
                    .unwrap_or_else(|| module_name.to_string());

                let prefer_submodule_var = inside_wrapper_init
                    && current_module
                        .is_some_and(|curr| canonical_module_name.starts_with(&format!("{curr}.")));
                log::debug!(
                    "Creating module expression for '{canonical_module_name}': \
                     prefer_submodule_var={prefer_submodule_var}, \
                     at_module_level={at_module_level}, inside_wrapper_init={inside_wrapper_init}"
                );

                let module_expr = if prefer_submodule_var {
                    let var =
                        crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                            &canonical_module_name,
                        );
                    expressions::name(&var, ExprContext::Load)
                } else if canonical_module_name.contains('.') {
                    let parts: Vec<&str> = canonical_module_name.split('.').collect();
                    Self::create_dotted_module_expr(
                        bundler,
                        &parts,
                        at_module_level,
                        &locally_initialized,
                    )
                } else if at_module_level {
                    expressions::name(&canonical_module_name, ExprContext::Load)
                } else if inside_wrapper_init {
                    let current_module_name = current_module.unwrap_or("");
                    log::debug!(
                        "Inside wrapper init: current_module={current_module_name}, \
                         accessing={canonical_module_name}"
                    );
                    Self::create_wrapper_init_module_expr(
                        bundler,
                        &canonical_module_name,
                        current_module,
                        &locally_initialized,
                    )
                } else {
                    Self::create_function_module_expr(
                        bundler,
                        &canonical_module_name,
                        &locally_initialized,
                    )
                };

                let value_expr = Self::resolve_import_value_expr(
                    bundler,
                    ImportResolveParams {
                        module_expr,
                        module_name,
                        imported_name,
                        at_module_level,
                        inside_wrapper_init,
                        current_module,
                        symbol_renames,
                    },
                );

                let assignment =
                    statements::simple_assign(target_name.as_str(), value_expr.clone());
                assignments.push(assignment);

                if inside_wrapper_init
                    && let Some(curr_name) = current_module
                    && let Some(curr_id) = bundler.get_module_id(curr_name)
                    && let Some(Some(exports)) = bundler.module_exports.get(&curr_id)
                    && exports.contains(&target_name.as_str().to_string())
                {
                    assignments.push(statements::assign_attribute(
                        "self",
                        target_name.as_str(),
                        expressions::name(target_name.as_str(), ExprContext::Load),
                    ));
                }
            }
        }

        assignments
    }

    fn resolve_import_value_expr(bundler: &Bundler, params: ImportResolveParams) -> Expr {
        if params.inside_wrapper_init {
            log::debug!(
                "resolve_import_value_expr: inside wrapper init, module_name='{}', \
                 imported_name='{}'",
                params.module_name,
                params.imported_name
            );
            if let Some(target_id) = bundler.get_module_id(params.module_name) {
                log::debug!(
                    "  Found module ID {:?} for '{}', is_inlined={}",
                    target_id,
                    params.module_name,
                    bundler.inlined_modules.contains(&target_id)
                );
                if target_id.is_entry() {
                    log::debug!(
                        "Inside wrapper init: accessing '{}' from entry module '{}' through \
                         namespace",
                        params.imported_name,
                        params.module_name
                    );
                    return expressions::attribute(
                        params.module_expr,
                        params.imported_name,
                        ExprContext::Load,
                    );
                }
                if bundler.inlined_modules.contains(&target_id) {
                    if let Some(renames) = params.symbol_renames.get(&target_id)
                        && let Some(renamed) = renames.get(params.imported_name)
                    {
                        log::debug!(
                            "Inside wrapper init: using renamed symbol '{}' directly for '{}' \
                             from inlined module '{}'",
                            renamed,
                            params.imported_name,
                            params.module_name
                        );
                        return expressions::name(renamed, ExprContext::Load);
                    }
                    log::debug!(
                        "Inside wrapper init: using symbol '{}' directly from inlined module '{}'",
                        params.imported_name,
                        params.module_name
                    );
                    return expressions::attribute(
                        expressions::name(params.module_name, ExprContext::Load),
                        params.imported_name,
                        ExprContext::Load,
                    );
                }
            }
        }
        expressions::attribute(params.module_expr, params.imported_name, ExprContext::Load)
    }

    fn create_dotted_module_expr(
        _bundler: &Bundler,
        parts: &[&str],
        _at_module_level: bool,
        _locally_initialized: &FxIndexSet<ModuleId>,
    ) -> Expr {
        let mut expr = expressions::name(parts[0], ExprContext::Load);
        for part in &parts[1..] {
            expr = expressions::attribute(expr, part, ExprContext::Load);
        }
        expr
    }

    fn create_wrapper_init_module_expr(
        bundler: &Bundler,
        canonical_module_name: &str,
        _current_module: Option<&str>,
        locally_initialized: &FxIndexSet<ModuleId>,
    ) -> Expr {
        let Some(module_id) = bundler.get_module_id(canonical_module_name) else {
            return expressions::name(canonical_module_name, ExprContext::Load);
        };
        if locally_initialized.contains(&module_id) {
            return expressions::name(canonical_module_name, ExprContext::Load);
        }
        let Some(init_func_name) = bundler.module_init_functions.get(&module_id) else {
            return expressions::name(canonical_module_name, ExprContext::Load);
        };
        let globals_call = expressions::call(
            expressions::name("globals", ExprContext::Load),
            vec![],
            vec![],
        );
        let key_name = if canonical_module_name.contains('.') {
            crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                canonical_module_name,
            )
        } else {
            canonical_module_name.to_string()
        };
        let module_ref = expressions::subscript(
            globals_call,
            expressions::string_literal(&key_name),
            ExprContext::Load,
        );
        expressions::call(
            expressions::name(init_func_name, ExprContext::Load),
            vec![module_ref],
            vec![],
        )
    }

    fn create_function_module_expr(
        bundler: &Bundler,
        canonical_module_name: &str,
        locally_initialized: &FxIndexSet<ModuleId>,
    ) -> Expr {
        if !bundler.has_synthetic_name(canonical_module_name) {
            return expressions::name(canonical_module_name, ExprContext::Load);
        }
        let Some(module_id) = bundler.get_module_id(canonical_module_name) else {
            return expressions::name(canonical_module_name, ExprContext::Load);
        };
        if locally_initialized.contains(&module_id) {
            return expressions::name(canonical_module_name, ExprContext::Load);
        }
        let Some(init_func_name) = bundler.module_init_functions.get(&module_id) else {
            return expressions::name(canonical_module_name, ExprContext::Load);
        };
        let globals_call = expressions::call(
            expressions::name("globals", ExprContext::Load),
            vec![],
            vec![],
        );
        let key_name = if canonical_module_name.contains('.') {
            crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                canonical_module_name,
            )
        } else {
            canonical_module_name.to_string()
        };
        let module_ref = expressions::subscript(
            globals_call,
            expressions::string_literal(&key_name),
            ExprContext::Load,
        );
        expressions::call(
            expressions::name(init_func_name, ExprContext::Load),
            vec![module_ref],
            vec![],
        )
    }

    /// Log information about wrapper wildcard exports (keeps previous behavior without generating
    /// code)
    pub(in crate::code_generator::import_transformer) fn log_wrapper_wildcard_info(
        resolved: &str,
        bundler: &Bundler,
    ) {
        log::debug!("  Handling wildcard import from wrapper module '{resolved}'");
        if let Some(exports) = bundler
            .get_module_id(resolved)
            .and_then(|id| bundler.module_exports.get(&id))
        {
            if let Some(export_list) = exports {
                log::debug!("  Wrapper module '{resolved}' exports: {export_list:?}");
                for export in export_list {
                    if export == "*" {
                        continue;
                    }
                }
            } else {
                log::debug!(
                    "  Wrapper module '{resolved}' has no explicit exports; importing all public \
                     symbols"
                );
                log::warn!(
                    "  Warning: Wildcard import from wrapper module without explicit __all__ may \
                     not import all symbols correctly"
                );
            }
        } else {
            log::warn!("  Warning: Could not find exports for wrapper module '{resolved}'");
        }
    }

    /// Check if a module is a wrapper module (has init function but is not inlined)
    pub(in crate::code_generator::import_transformer) fn is_wrapper_module(
        module_name: &str,
        bundler: &Bundler,
    ) -> bool {
        bundler
            .get_module_id(module_name)
            .is_some_and(|id| bundler.module_init_functions.contains_key(&id))
    }

    /// Track wrapper module imports for later rewriting
    pub(in crate::code_generator::import_transformer) fn track_wrapper_imports(
        import_from: &StmtImportFrom,
        module_name_for_tracking: &str,
        wrapper_module_imports: &mut FxIndexMap<String, (String, String)>,
    ) {
        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Store mapping: local_name -> (wrapper_module, imported_name)
            wrapper_module_imports.insert(
                local_name.to_string(),
                (
                    module_name_for_tracking.to_string(),
                    imported_name.to_string(),
                ),
            );

            log::debug!(
                "  Tracking wrapper import: {local_name} -> \
                 {module_name_for_tracking}.{imported_name}"
            );
        }
    }

    /// Handle from-import on resolved wrapper module
    pub(in crate::code_generator::import_transformer) fn handle_from_import_on_resolved_wrapper(
        transformer: &mut crate::code_generator::import_transformer::RecursiveImportTransformer,
        import_from: &StmtImportFrom,
        resolved: &str,
    ) -> Option<Vec<Stmt>> {
        // Check if this is a wrapper module (in module_registry)
        // This check must be after the inlined module check to avoid double-handling
        // A module is a wrapper module if it has an init function
        if Self::is_wrapper_module(resolved, transformer.state.bundler) {
            log::debug!("  Module '{resolved}' is a wrapper module");

            // For modules importing from wrapper modules, we may need to defer
            // the imports to ensure proper initialization order
            let current_module_is_inlined = transformer
                .state
                .bundler
                .inlined_modules
                .contains(&transformer.state.module_id);

            // When an inlined module imports from a wrapper module, we need to
            // track the imports and rewrite all usages within the module
            if !transformer.state.module_id.is_entry() && current_module_is_inlined {
                log::debug!(
                    "  Tracking wrapper module imports for rewriting in module '{}' (inlined: {})",
                    transformer
                        .state
                        .bundler
                        .resolver
                        .get_module_name(transformer.state.module_id)
                        .unwrap_or_else(|| format!("module#{}", transformer.state.module_id)),
                    current_module_is_inlined
                );

                // First, ensure the wrapper module is initialized
                // This is crucial for lazy imports inside functions
                let mut init_stmts = Vec::new();

                // Check if the parent module needs handling
                if let Some((parent, child)) = resolved.rsplit_once('.') {
                    // If the parent is also a wrapper module, DO NOT initialize it here
                    // It will be initialized when accessed
                    if transformer
                        .state
                        .bundler
                        .get_module_id(parent)
                        .is_some_and(|id| {
                            transformer
                                .state
                                .bundler
                                .module_init_functions
                                .contains_key(&id)
                        })
                    {
                        log::debug!(
                            "  Parent '{parent}' is a wrapper module - skipping immediate \
                             initialization"
                        );
                        // Don't initialize parent wrapper module here
                    }

                    // If the parent is an inlined module, the submodule assignment is handled
                    // by its own initialization, so we only need to log
                    if transformer
                        .state
                        .bundler
                        .get_module_id(parent)
                        .is_some_and(|id| transformer.state.bundler.inlined_modules.contains(&id))
                    {
                        log::debug!(
                            "Parent '{parent}' is inlined, submodule '{child}' assignment already \
                             handled"
                        );
                    }
                }

                // Check if this is a wildcard import
                let is_wildcard =
                    import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*";

                // With correct topological ordering, we can safely initialize wrapper modules
                // right where the import statement was. This ensures the wrapper module is
                // initialized before its symbols are used (e.g., in class inheritance).
                // CRITICAL: Only generate init calls for actual wrapper modules that have init
                // functions BUT skip if this is an inlined submodule
                // importing from its parent package
                let is_parent_import = if current_module_is_inlined {
                    // Check if resolved is a parent of the current module
                    transformer
                        .state
                        .bundler
                        .resolver
                        .get_module_name(transformer.state.module_id)
                        .unwrap_or_else(|| format!("module#{}", transformer.state.module_id))
                        .starts_with(&format!("{resolved}."))
                } else {
                    false
                };

                // Get module ID if it exists and has an init function
                let wrapper_module_id = if !is_wildcard && !is_parent_import {
                    transformer
                        .state
                        .bundler
                        .get_module_id(resolved)
                        .filter(|id| {
                            transformer
                                .state
                                .bundler
                                .module_init_functions
                                .contains_key(id)
                        })
                } else {
                    None
                };

                if let Some(module_id) = wrapper_module_id {
                    // Do not emit init calls for the entry package (__init__ or __main__).
                    // Initializing the entry package from submodules can create circular init.
                    let is_entry_pkg = if transformer.state.bundler.entry_is_package_init_or_main {
                        let entry_pkg = [
                            crate::python::constants::INIT_STEM,
                            crate::python::constants::MAIN_STEM,
                        ]
                        .iter()
                        .find_map(|stem| {
                            transformer
                                .state
                                .bundler
                                .entry_module_name
                                .strip_suffix(&format!(".{stem}"))
                        });
                        entry_pkg.is_some_and(|pkg| pkg == resolved)
                    } else {
                        false
                    };
                    if is_entry_pkg {
                        log::debug!(
                            "  Skipping init call for entry package '{resolved}' to avoid \
                             circular initialization"
                        );
                    } else {
                        log::debug!(
                            "  Generating initialization call for wrapper module '{resolved}' at \
                             import location"
                        );

                        // Use ast_builder helper to generate wrapper init call
                        use crate::{
                            ast_builder::module_wrapper,
                            code_generator::module_registry::get_module_var_identifier,
                        };

                        let module_var = get_module_var_identifier(
                            module_id,
                            transformer.state.bundler.resolver,
                        );

                        // If we're not at module level (i.e., inside any local scope), we need
                        // to declare the module variable as global to avoid UnboundLocalError.
                        // However, skip if it conflicts with a local variable (like function
                        // parameters).
                        if transformer.state.at_module_level {
                            init_stmts
                                .push(module_wrapper::create_wrapper_module_init_call(&module_var));
                        } else if !transformer.state.local_variables.contains(&module_var) {
                            // Only initialize if no conflict with local variable
                            log::debug!(
                                "  Adding global declaration for '{module_var}' (inside local \
                                 scope)"
                            );
                            init_stmts.push(statements::global(vec![module_var.as_str()]));
                            init_stmts
                                .push(module_wrapper::create_wrapper_module_init_call(&module_var));
                        } else {
                            log::debug!(
                                "  Initializing wrapper via globals() to avoid local shadow: \
                                 {module_var}"
                            );
                            // globals()[module_var] =
                            // globals()[module_var].__init__(globals()[module_var])
                            let g_call = expressions::call(
                                expressions::name("globals", ExprContext::Load),
                                vec![],
                                vec![],
                            );
                            let key = expressions::string_literal(&module_var);
                            let lhs = expressions::subscript(
                                g_call.clone(),
                                key.clone(),
                                ExprContext::Store,
                            );
                            let rhs_self = expressions::subscript(
                                g_call.clone(),
                                key.clone(),
                                ExprContext::Load,
                            );
                            let rhs_call = expressions::call(
                                expressions::attribute(
                                    rhs_self.clone(),
                                    crate::ast_builder::module_wrapper::MODULE_INIT_ATTR,
                                    ExprContext::Load,
                                ),
                                vec![rhs_self],
                                vec![],
                            );
                            init_stmts.push(statements::assign(vec![lhs], rhs_call));
                        }
                    }
                } else if is_parent_import && !is_wildcard {
                    log::debug!(
                        "  Skipping init call for parent package '{resolved}' from inlined \
                         submodule '{}'",
                        transformer
                            .state
                            .bundler
                            .resolver
                            .get_module_name(transformer.state.module_id)
                            .unwrap_or_else(|| format!("module#{}", transformer.state.module_id))
                    );
                }

                // Handle wildcard import export assignments
                if is_wildcard {
                    Self::log_wrapper_wildcard_info(resolved, transformer.state.bundler);
                    log::debug!(
                        "  Returning {} parent-init statements for wildcard import; wrapper init \
                         + assignments were deferred",
                        init_stmts.len()
                    );
                    return Some(init_stmts);
                }

                // Track each imported symbol for rewriting
                // Use the canonical module name if we have a wrapper module ID
                let module_name_for_tracking = if let Some(module_id) = wrapper_module_id {
                    transformer
                        .state
                        .bundler
                        .resolver
                        .get_module_name(module_id)
                        .unwrap_or_else(|| resolved.to_string())
                } else {
                    resolved.to_string()
                };

                Self::track_wrapper_imports(
                    import_from,
                    &module_name_for_tracking,
                    &mut transformer.state.wrapper_module_imports,
                );

                // If we skipped initialization due to a conflict, also skip the assignments
                if !transformer.state.at_module_level {
                    use crate::code_generator::module_registry::get_module_var_identifier;
                    let module_var = if let Some(module_id) = wrapper_module_id {
                        get_module_var_identifier(module_id, transformer.state.bundler.resolver)
                    } else {
                        crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                            resolved,
                        )
                    };

                    if transformer.state.local_variables.contains(&module_var) {
                        // Only skip if alias isn't used at runtime
                        if transformer.should_skip_assignments_for_type_only_imports(import_from) {
                            log::debug!(
                                "  Skipping wrapper import assignments (type-only use) for \
                                 '{module_var}'"
                            );
                            return Some(Vec::new());
                        }
                        log::debug!(
                            "  Conflict with local variable but alias is used at runtime; keeping \
                             assignments"
                        );
                    }
                }

                // Defer to the standard bundled-wrapper transformation to generate proper
                // alias assignments and ensure initialization ordering. This keeps behavior
                // consistent and avoids missing local aliases needed for class bases.
                // The rewrite_import_from will handle creating the proper assignments
                // after the wrapper module is initialized.
                let mut result =
                    super::super::rewrite_import_from(super::super::RewriteImportFromParams {
                        bundler: transformer.state.bundler,
                        import_from: import_from.clone(),
                        current_module: &transformer
                            .state
                            .bundler
                            .resolver
                            .get_module_name(transformer.state.module_id)
                            .unwrap_or_else(|| format!("module#{}", transformer.state.module_id)),
                        module_path: transformer
                            .state
                            .bundler
                            .resolver
                            .get_module_path(transformer.state.module_id)
                            .as_deref(),
                        symbol_renames: transformer.state.symbol_renames,
                        inside_wrapper_init: transformer.state.is_wrapper_init,
                        at_module_level: transformer.state.at_module_level,
                        python_version: transformer.state.python_version,
                        function_body: transformer.state.current_function_body.as_deref(),
                    });

                // Prepend the init statements to ensure wrapper is initialized before use
                init_stmts.append(&mut result);
                return Some(init_stmts);
            }
            // For wrapper modules importing from other wrapper modules,
            // let it fall through to standard transformation
        }

        None
    }

    /// Check if entry module wrapper imports should be skipped due to deduplication
    pub(in crate::code_generator::import_transformer) fn maybe_skip_entry_wrapper_if_all_deferred(
        transformer: &mut crate::code_generator::import_transformer::RecursiveImportTransformer,
        import_from: &StmtImportFrom,
        resolved: &str,
    ) -> bool {
        // For entry module, check if this import would duplicate deferred imports
        if transformer.state.module_id.is_entry() {
            // Check if this is a wrapper module
            if transformer
                .state
                .bundler
                .get_module_id(resolved)
                .is_some_and(|id| {
                    transformer
                        .state
                        .bundler
                        .module_info_registry
                        .as_ref()
                        .is_some_and(|reg| reg.contains_module(&id))
                })
            {
                // Check if we have access to global deferred imports
                if let Some(global_deferred) = transformer.state.global_deferred_imports {
                    // Check each symbol to see if it's already been deferred
                    let mut all_symbols_deferred = true;
                    if let Some(module_id) = transformer
                        .state
                        .bundler
                        .resolver
                        .get_module_id_by_name(resolved)
                    {
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str(); // The actual name being imported
                            if !global_deferred
                                .contains_key(&(module_id, imported_name.to_string()))
                            {
                                all_symbols_deferred = false;
                                break;
                            }
                        }
                    } else {
                        // Module not found, can't be deferred
                        all_symbols_deferred = false;
                    }

                    if all_symbols_deferred {
                        log::debug!(
                            "  Skipping import from '{resolved}' in entry module - all symbols \
                             already deferred by inlined modules"
                        );
                        return true;
                    }
                }
            }
        }

        false
    }
}
