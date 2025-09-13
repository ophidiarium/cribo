use ruff_python_ast::{ExprContext, Stmt, StmtImportFrom};

use crate::{
    ast_builder::{expressions, statements},
    code_generator::bundler::Bundler,
    types::FxIndexMap,
};

/// Handle wrapper module import transformations
pub struct WrapperHandler;

impl WrapperHandler {
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
}
