use ruff_python_ast::{Expr, ExprContext, Identifier, Stmt, StmtImportFrom};
use ruff_text_size::TextRange;

use crate::{
    ast_builder::{expressions, statements},
    code_generator::{
        bundler::Bundler,
        expression_handlers,
        module_registry::{is_init_function, sanitize_module_name_for_identifier},
        module_transformer::SELF_PARAM,
    },
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Simplified wrapper context to reduce parameter count
pub(crate) struct WrapperContext<'a> {
    pub bundler: &'a Bundler<'a>,
    pub symbol_renames: &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
    pub is_wrapper_init: bool,
    pub at_module_level: bool,
    pub current_module_name: String,
    pub function_body: Option<&'a [Stmt]>,
    /// Cached set of symbols used at runtime in the current function.
    /// When present (Some), this takes precedence over deriving usage from `function_body`.
    /// This is a borrowed reference from the transformer's cached analysis, avoiding
    /// redundant recomputation via `SymbolUsageVisitor`.
    pub current_function_used_symbols: Option<&'a FxIndexSet<String>>,
}

// No local ImportResolveParams needed; heavy lifting stays in Bundler for now

/// Handle wrapper module import transformations
pub(crate) struct WrapperHandler;

impl WrapperHandler {
    /// Handle wrapper from-import in absolute context (rel→abs conversion + handler dispatch)
    pub(in crate::code_generator::import_transformer) fn handle_wrapper_from_import_absolute_context(
        context: &WrapperContext<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
    ) -> Vec<Stmt> {
        // Check if this module is in the registry (wrapper approach)
        if Self::is_wrapper_module(module_name, context.bundler) {
            // Module uses wrapper approach - transform to sys.modules access
            // For relative imports, we need to create an absolute import
            let mut absolute_import = import_from.clone();
            if import_from.level > 0 {
                // If module_name is empty, this is a critical error
                if module_name.is_empty() {
                    panic!(
                        "Relative import 'from {} import {:?}' in module '{}' resolved to empty \
                         module name. This is a bug - relative imports must resolve to a valid \
                         module.",
                        ".".repeat(import_from.level as usize),
                        import_from
                            .names
                            .iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>(),
                        &context.current_module_name
                    );
                } else {
                    // Convert relative import to absolute
                    absolute_import.level = 0;
                    absolute_import.module =
                        Some(Identifier::new(module_name, TextRange::default()));
                }
            }
            Self::rewrite_from_import_for_wrapper_module_with_context(
                context,
                &absolute_import,
                module_name,
            )
        } else {
            panic!(
                "handle_wrapper_from_import_absolute_context called on non-wrapper module: \
                 {module_name}"
            )
        }
    }

    /// Same as `rewrite_from_import_for_wrapper_module` but accepts explicit context.
    pub(in crate::code_generator::import_transformer) fn rewrite_from_import_for_wrapper_module_with_context(
        context: &WrapperContext<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
    ) -> Vec<Stmt> {
        log::debug!(
            "transform_bundled_import_from_multiple: module_name={}, imports={:?}, \
             context.is_wrapper_init={}",
            module_name,
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            context.is_wrapper_init
        );

        // Early dispatch: wildcard imports handled separately
        if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
            let bundled_context = crate::code_generator::bundler::BundledImportContext {
                inside_wrapper_init: context.is_wrapper_init,
                at_module_level: context.at_module_level,
                current_module: Some(&context.current_module_name),
                current_function_used_symbols: context.current_function_used_symbols,
            };
            return context
                .bundler
                .transform_bundled_import_from_multiple_with_current_module(
                    import_from,
                    module_name,
                    &bundled_context,
                    context.symbol_renames,
                    context.function_body,
                );
        }

        // Defer to alias/symbol handling path
        let bundled_context = crate::code_generator::bundler::BundledImportContext {
            inside_wrapper_init: context.is_wrapper_init,
            at_module_level: context.at_module_level,
            current_module: Some(&context.current_module_name),
            current_function_used_symbols: context.current_function_used_symbols,
        };
        context
            .bundler
            .transform_bundled_import_from_multiple_with_current_module(
                import_from,
                module_name,
                &bundled_context,
                context.symbol_renames,
                context.function_body,
            )
    }

    /// Handle non-wildcard from-imports (symbols/aliases) for wrapper modules.
    ///
    /// This method now uses the extracted logic from the bundler to handle
    /// individual symbol imports from wrapper modules.
    pub(in crate::code_generator::import_transformer) fn handle_symbol_imports_from_multiple(
        bundler: &Bundler<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
        context: &crate::code_generator::bundler::BundledImportContext<'_>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        function_body: Option<&[Stmt]>,
    ) -> Vec<Stmt> {
        // Build context for the new handler method
        let wrapper_context = WrapperContext {
            bundler,
            symbol_renames,
            is_wrapper_init: context.inside_wrapper_init,
            at_module_level: context.at_module_level,
            current_module_name: context.current_module.unwrap_or("").to_owned(),
            function_body,
            current_function_used_symbols: context.current_function_used_symbols,
        };

        Self::handle_symbol_imports_from_wrapper(&wrapper_context, import_from, module_name)
    }

    /// Handle wildcard-from imports (`from X import *`) for wrapper modules
    pub(in crate::code_generator::import_transformer) fn handle_wildcard_import_from_multiple(
        bundler: &Bundler<'_>,
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
                            vec![module_expr, expressions::name(attr_var, ExprContext::Load)],
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
            // Only add explicit module attribute assignment for wrapper inits to ensure proper
            // symbol propagation. The module_transformer's add_module_attr_if_exported handles
            // regular cases, but wrapper wildcard imports need explicit handling.
            if inside_wrapper_init && let Some(current_mod) = current_module {
                let module_var = sanitize_module_name_for_identifier(current_mod);
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

    /// Log information about wrapper wildcard exports (keeps previous behavior without generating
    /// code)
    pub(in crate::code_generator::import_transformer) fn log_wrapper_wildcard_info(
        resolved: &str,
        bundler: &Bundler<'_>,
    ) {
        log::debug!("  Handling wildcard import from wrapper module '{resolved}'");
        if let Some(exports) = bundler
            .get_module_id(resolved)
            .and_then(|id| bundler.module_exports.get(&id))
        {
            if let Some(export_list) = exports {
                log::debug!("  Wrapper module '{resolved}' exports: {export_list:?}");
                // (no-op loop removed)
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

    /// Check if a module is a wrapper module (bundled but not inlined)
    pub(in crate::code_generator::import_transformer) fn is_wrapper_module(
        module_name: &str,
        bundler: &Bundler<'_>,
    ) -> bool {
        bundler.get_module_id(module_name).is_some_and(|id| {
            bundler.bundled_modules.contains(&id) && !bundler.inlined_modules.contains(&id)
        })
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
                local_name.to_owned(),
                (
                    module_name_for_tracking.to_owned(),
                    imported_name.to_owned(),
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
        transformer: &mut crate::code_generator::import_transformer::RecursiveImportTransformer<'_>,
        import_from: &StmtImportFrom,
        resolved: &str,
    ) -> Option<Vec<Stmt>> {
        use crate::{
            ast_builder::module_wrapper, code_generator::module_registry::get_module_var_identifier,
        };

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
                            let rhs_self = expressions::subscript(g_call, key, ExprContext::Load);
                            let rhs_call = expressions::call(
                                expressions::attribute(
                                    rhs_self.clone(),
                                    module_wrapper::MODULE_INIT_ATTR,
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
                    let current_mod_name = transformer
                        .state
                        .bundler
                        .resolver
                        .get_module_name(transformer.state.module_id)
                        .unwrap_or_else(|| format!("module#{}", transformer.state.module_id));
                    let stmts = Self::handle_wildcard_import_from_multiple(
                        transformer.state.bundler,
                        import_from,
                        resolved,
                        transformer.state.is_wrapper_init,
                        Some(&current_mod_name),
                        transformer.state.at_module_level,
                    );
                    return Some(stmts);
                }

                // Track each imported symbol for rewriting
                // Use the canonical module name if we have a wrapper module ID
                let module_name_for_tracking = if let Some(module_id) = wrapper_module_id {
                    transformer
                        .state
                        .bundler
                        .resolver
                        .get_module_name(module_id)
                        .unwrap_or_else(|| resolved.to_owned())
                } else {
                    resolved.to_owned()
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
                        sanitize_module_name_for_identifier(resolved)
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
                        current_function_used_symbols: transformer
                            .state
                            .current_function_used_symbols
                            .as_ref(),
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

    /// Maybe handle wrapper absolute imports (non-resolved branch)
    pub(in crate::code_generator::import_transformer) fn maybe_handle_wrapper_absolute(
        context: &WrapperContext<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
    ) -> Option<Vec<Stmt>> {
        // Check if this module is in the module_registry (wrapper module)
        if Self::is_wrapper_module(module_name, context.bundler) {
            log::debug!("Module '{module_name}' is a wrapper module in module_registry");
            // Route wrapper-module from-import rewriting through the wrapper handler.
            return Some(Self::rewrite_from_import_for_wrapper_module_with_context(
                context,
                import_from,
                module_name,
            ));
        }

        None
    }

    /// Handle non-wildcard from-imports from wrapper modules
    ///
    /// This is the main logic extracted from `Bundler::handle_symbol_imports_from_multiple`
    /// for handling individual symbol imports from wrapper modules.
    pub(in crate::code_generator::import_transformer) fn handle_symbol_imports_from_wrapper(
        context: &WrapperContext<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
    ) -> Vec<Stmt> {
        let inside_wrapper_init = context.is_wrapper_init;
        let at_module_level = context.at_module_level;
        let current_module = context.current_module_name.as_str();
        let function_body = context.function_body;
        let symbol_renames = context.symbol_renames;
        let bundler = context.bundler;

        let mut assignments = Vec::new();
        let mut initialized_modules: FxIndexSet<ModuleId> = FxIndexSet::default();
        let mut locally_initialized: FxIndexSet<ModuleId> = FxIndexSet::default();

        // Use cached symbols if available, otherwise compute them (borrow when possible)
        let owned_used = if !at_module_level && context.current_function_used_symbols.is_none() {
            function_body.map(crate::visitors::SymbolUsageVisitor::collect_used_symbols)
        } else {
            None
        };

        let used_symbols: Option<&FxIndexSet<String>> = if at_module_level {
            None
        } else if let Some(cached) = context.current_function_used_symbols {
            Some(cached)
        } else {
            owned_used.as_ref()
        };

        // For wrapper modules, we always need to ensure they're initialized before accessing
        // attributes Don't create the temporary variable approach - it causes issues with
        // namespace reassignment

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

            // Check if we're importing a submodule (e.g., from greetings import greeting)
            let full_module_path = format!("{module_name}.{imported_name}");

            // First check if the parent module has an __init__.py (is a wrapper module)
            // and might re-export this name
            let parent_is_wrapper = bundler.has_synthetic_name(module_name);
            let submodule_exists = bundler.get_module_id(&full_module_path).is_some_and(|id| {
                bundler.bundled_modules.contains(&id)
                    && (bundler.has_synthetic_name(&full_module_path)
                        || bundler.inlined_modules.contains(&id))
            });

            // If both the parent is a wrapper and a submodule exists, we need to decide
            // In Python, attributes from __init__.py take precedence over submodules
            // So we should prefer the attribute unless we have evidence it's not re-exported
            let importing_submodule = if parent_is_wrapper && submodule_exists {
                // Check if the parent module explicitly exports this name
                if let Some(Some(export_list)) = bundler
                    .get_module_id(module_name)
                    .and_then(|id| bundler.module_exports.get(&id))
                {
                    // If __all__ is defined and doesn't include this name, it's the submodule
                    !export_list.contains(&imported_name.to_owned())
                } else {
                    // No __all__ defined - check if the submodule actually exists
                    // If it does, we're importing the submodule not an attribute
                    submodule_exists
                }
            } else {
                // Simple case: just check if it's a submodule
                submodule_exists
            };

            if importing_submodule {
                // We're importing a submodule, not an attribute
                log::debug!(
                    "Importing submodule '{imported_name}' from '{module_name}' via from import"
                );

                // Determine if current module is a submodule of the target module
                let is_submodule_of_target = current_module.starts_with(&format!("{module_name}."));

                // Check if parent module should be initialized
                let parent_module_id = bundler.get_module_id(module_name);
                let should_initialize_parent = parent_module_id.is_some_and(|id| {
                    bundler.has_synthetic_name(module_name)
                        && !locally_initialized.contains(&id)
                        && current_module != module_name // Prevent self-initialization
                        && !is_submodule_of_target // Prevent parent initialization from submodule
                });

                // Check if submodule should be initialized
                let submodule_id = bundler.get_module_id(&full_module_path);
                let should_initialize_submodule = submodule_id.is_some_and(|id| {
                    bundler.has_synthetic_name(&full_module_path)
                        && !locally_initialized.contains(&id)
                });

                // Always initialize parent first, then submodule (caller-driven order)
                if should_initialize_parent {
                    // Initialize parent module first
                    if let Some(module_id) = parent_module_id {
                        let current_module_id = bundler.get_module_id(current_module);
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
                        locally_initialized.insert(module_id);
                    }
                }

                if should_initialize_submodule && let Some(submodule_id) = submodule_id {
                    crate::code_generator::module_registry::initialize_submodule_if_needed(
                        submodule_id,
                        &bundler.module_init_functions,
                        bundler.resolver,
                        &mut assignments,
                        &mut locally_initialized,
                        &mut initialized_modules,
                    );
                }

                // Build the direct namespace reference
                log::debug!(
                    "Building namespace reference for '{}' (is_inlined: {}, has_dot: {})",
                    full_module_path,
                    bundler
                        .get_module_id(&full_module_path)
                        .is_some_and(|id| bundler.inlined_modules.contains(&id)),
                    full_module_path.contains('.')
                );
                let namespace_expr = if bundler
                    .get_module_id(&full_module_path)
                    .is_some_and(|id| bundler.inlined_modules.contains(&id))
                {
                    // For inlined modules, check if it's a dotted name
                    if full_module_path.contains('.') {
                        // For nested inlined modules like myrequests.compat, create dotted
                        // expression
                        let parts: Vec<&str> = full_module_path.split('.').collect();
                        log::debug!("Creating dotted name for inlined nested module: {parts:?}");
                        expressions::dotted_name(&parts, ExprContext::Load)
                    } else {
                        // Simple inlined module
                        log::debug!("Using simple name for inlined module: {full_module_path}");
                        expressions::name(&full_module_path, ExprContext::Load)
                    }
                } else if full_module_path.contains('.') {
                    // For nested modules like models.user, create models.user expression
                    let parts: Vec<&str> = full_module_path.split('.').collect();
                    log::debug!("Creating dotted name for nested module: {parts:?}");
                    expressions::dotted_name(&parts, ExprContext::Load)
                } else {
                    // Top-level module
                    log::debug!("Creating simple name for top-level module: {full_module_path}");
                    expressions::name(&full_module_path, ExprContext::Load)
                };

                log::debug!(
                    "Creating submodule import assignment: {} = {:?}",
                    target_name.as_str(),
                    namespace_expr
                );
                assignments.push(statements::simple_assign(
                    target_name.as_str(),
                    namespace_expr,
                ));
            } else {
                // Regular attribute import
                // Special case: if we're inside the wrapper init of a module importing its own
                // submodule
                if inside_wrapper_init && module_name.starts_with(&format!("{current_module}.")) {
                    // Check if this is actually a submodule
                    let full_submodule_path = format!("{module_name}.{imported_name}");
                    if bundler
                        .get_module_id(&full_submodule_path)
                        .is_some_and(|id| bundler.bundled_modules.contains(&id))
                        && bundler.has_synthetic_name(&full_submodule_path)
                    {
                        // This is a submodule that needs initialization
                        log::debug!(
                            "Special case: module '{module_name}' importing its own submodule \
                             '{imported_name}' - initializing submodule first"
                        );

                        // Initialize the submodule
                        if let Some(submodule_id) = bundler.get_module_id(&full_submodule_path) {
                            assignments.extend(
                                bundler.create_module_initialization_for_import(submodule_id),
                            );
                            if let Some(submodule_id) = bundler.get_module_id(&full_submodule_path)
                            {
                                locally_initialized.insert(submodule_id);
                            }
                        }

                        // Now create the assignment from the initialized submodule's namespace
                        // Use the submodule variable directly to avoid relying on parent namespace
                        let submodule_var =
                            sanitize_module_name_for_identifier(&full_submodule_path);
                        let assignment = statements::simple_assign(
                            target_name.as_str(),
                            expressions::attribute(
                                expressions::name(&submodule_var, ExprContext::Load),
                                imported_name,
                                ExprContext::Load,
                            ),
                        );
                        assignments.push(assignment);
                        continue; // Skip the rest of the regular attribute handling
                    }
                }

                // Check if we're importing from an inlined module and the target is a wrapper
                // submodule This happens when mypkg is inlined and does `from .
                // import compat` where compat uses init function
                if bundler
                    .get_module_id(module_name)
                    .is_some_and(|id| bundler.inlined_modules.contains(&id))
                    && !inside_wrapper_init
                {
                    let full_submodule_path = format!("{module_name}.{imported_name}");
                    if bundler.has_synthetic_name(&full_submodule_path) {
                        // This is importing a wrapper submodule from an inlined parent module
                        // This case should have been handled by the import transformer during
                        // inlining and deferred. If we get here, something
                        // went wrong.
                        log::warn!(
                            "Unexpected: importing wrapper submodule '{imported_name}' from \
                             inlined module '{module_name}' in handle_symbol_imports_from_wrapper \
                             - should have been deferred"
                        );

                        // Create direct assignment to where the module will be (fallback)
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
                        continue; // Skip the rest
                    }
                }

                // Only skip imports for bundled modules where we can be confident about side
                // effects For external/non-bundled imports, always preserve them to
                // maintain side effects
                if let Some(used) = used_symbols
                    && !used.contains(target_name.as_str())
                {
                    // Check if this is a bundled module where we control the behavior
                    let module_id = bundler.get_module_id(module_name);
                    let is_bundled_or_inlined = module_id.is_some_and(|id| {
                        bundler.bundled_modules.contains(&id)
                            || bundler.inlined_modules.contains(&id)
                    });
                    let is_wrapper =
                        module_id.is_some_and(|id| bundler.wrapper_modules.contains(&id));
                    let wrapper_is_circular =
                        module_id.is_some_and(|id| bundler.is_module_in_circular_deps(id));

                    // Only skip for inlined modules where we're certain about usage
                    // For wrapper modules, we need to be more careful about side effects
                    if is_bundled_or_inlined && !is_wrapper {
                        log::debug!(
                            "Skipping unused symbol '{target_name}' from inlined module \
                             '{module_name}' inside function"
                        );
                        continue;
                    } else if is_wrapper && wrapper_is_circular {
                        // For circular wrapper modules, we need to ensure they're initialized
                        // even if the imported symbol isn't used. However, we can skip if:
                        // 1. The module has already been initialized in this scope
                        // 2. AND the symbol is clearly not used at runtime

                        // Check if this module has already been initialized in this function's
                        // scope
                        let module_id = bundler
                            .get_module_id(module_name)
                            .expect("wrapper module should have ID");

                        if locally_initialized.contains(&module_id) {
                            // Module already initialized in this scope
                            // We already know the symbol is not in the used set (from the outer if
                            // condition) So we can skip it since it's
                            // not used at runtime
                            log::debug!(
                                "Skipping unused symbol '{target_name}' from already-initialized \
                                 circular wrapper module '{module_name}' inside function"
                            );
                            continue;
                        }
                        // Module not yet initialized - preserve import to trigger
                        // initialization
                        log::debug!(
                            "Preserving import for '{target_name}' from circular wrapper module \
                             '{module_name}' - needs initialization for side effects"
                        );
                        // Continue with normal processing to trigger initialization
                    } else if is_wrapper {
                        // Non-circular wrappers: preserve to avoid behavior changes
                        log::debug!(
                            "Preserving potentially unused symbol '{target_name}' from \
                             non-circular wrapper module '{module_name}'"
                        );
                        // Continue with normal processing
                    } else {
                        log::debug!(
                            "Preserving import for '{target_name}' from external module \
                             '{module_name}' - may have side effects even if unused"
                        );
                        // Continue with normal processing to preserve potential side effects
                    }
                }

                // Ensure the module is initialized first if it's a wrapper module
                // Only initialize if we're inside a wrapper init OR if the module's init
                // function has already been defined (to avoid forward references)
                let needs_init = bundler.get_module_id(module_name).is_some_and(|module_id| {
                    // Avoid initializing a parent namespace from within a child's wrapper init
                    let is_parent_of_current =
                        current_module.starts_with(&format!("{module_name}."));

                    bundler.has_synthetic_name(module_name)
                        && !locally_initialized.contains(&module_id)
                        && current_module != module_name // Prevent self-initialization
                        && !is_parent_of_current
                        && (inside_wrapper_init
                            || bundler.module_init_functions.contains_key(&module_id))
                });
                if needs_init {
                    // Check if this module is already initialized in any deferred imports
                    let module_init_exists = assignments.iter().any(|stmt| {
                        if let Stmt::Assign(assign) = stmt
                            && assign.targets.len() == 1
                            && let Expr::Call(call) = &assign.value.as_ref()
                            && let Expr::Name(func_name) = &call.func.as_ref()
                            && is_init_function(func_name.id.as_str())
                        {
                            // Check if the target matches our module
                            match &assign.targets[0] {
                                Expr::Attribute(attr) => {
                                    let attr_path =
                                        expression_handlers::extract_attribute_path(attr);
                                    attr_path == module_name
                                }
                                Expr::Name(name) => name.id.as_str() == module_name,
                                _ => false,
                            }
                        } else {
                            false
                        }
                    });

                    if !module_init_exists {
                        // Initialize the module before accessing its attributes
                        if let Some(module_id) = bundler.get_module_id(module_name) {
                            let current_module_id = bundler.get_module_id(current_module);

                            // If we're inside a function (not at module level), we should NOT add
                            // global declarations or module assignments. Instead, we'll inline the
                            // init call when creating the symbol assignment below.
                            if at_module_level {
                                // Only at module level do we need to initialize and assign the
                                // module
                                assignments.extend(
                                    bundler.create_module_initialization_for_import_with_current_module(
                                        module_id,
                                        current_module_id,
                                        at_module_level,
                                    ),
                                );
                                // Only mark as initialized if we actually initialized it
                                locally_initialized.insert(module_id);
                            }
                            // Don't mark as initialized if we're deferring initialization
                        }
                    }
                }

                // Check if this symbol is re-exported from an inlined submodule.
                // If it is, use the globally inlined symbol (respecting semantic renames)
                // instead of wrapper attribute access.
                if bundler.has_synthetic_name(module_name) {
                    // Keep current semantics: we don't attempt to detect "directly defined in
                    // wrapper" here.
                    let is_defined_in_wrapper = false;

                    if !is_defined_in_wrapper
                        && let Some((source_module, source_symbol)) = bundler
                            .is_symbol_from_inlined_submodule(module_name, target_name.as_str())
                    {
                        // Map to the effective global name considering semantic renames of the
                        // source module.
                        let source_module_id = bundler
                            .get_module_id(&source_module)
                            .expect("Source module should exist");
                        let global_name = symbol_renames
                            .get(&source_module_id)
                            .and_then(|m| m.get(&source_symbol))
                            .cloned()
                            .unwrap_or_else(|| source_symbol.clone());

                        log::debug!(
                            "Using global symbol '{}' from inlined submodule '{}' for re-exported \
                             symbol '{}' in wrapper '{}'",
                            global_name,
                            source_module,
                            target_name.as_str(),
                            module_name
                        );

                        // Only create assignment if the names differ (avoid X = X)
                        if target_name.as_str() == global_name {
                            log::debug!(
                                "Skipping self-referential assignment: {} = {}",
                                target_name.as_str(),
                                global_name
                            );
                        } else {
                            let assignment = statements::simple_assign(
                                target_name.as_str(),
                                expressions::name(&global_name, ExprContext::Load),
                            );
                            assignments.push(assignment);
                        }

                        // If we're inside a wrapper init, and this symbol is part of the module's
                        // exports, also expose it on the namespace (self.<name> = <name>).
                        if inside_wrapper_init
                            && let Some(curr_id) = bundler.get_module_id(current_module)
                            && let Some(Some(exports)) = bundler.module_exports.get(&curr_id)
                            && exports.contains(&target_name.as_str().to_owned())
                        {
                            assignments.push(statements::assign_attribute(
                                SELF_PARAM,
                                target_name.as_str(),
                                expressions::name(target_name.as_str(), ExprContext::Load),
                            ));
                        }
                        continue; // Skip the normal attribute assignment
                    }
                }

                // Create: target = module.imported_name
                // Resolve symlinks: get canonical module name if it exists
                let canonical_module_name = bundler
                    .get_module_id(module_name)
                    .and_then(|id| bundler.resolver.get_module_name(id))
                    .unwrap_or_else(|| module_name.to_owned());

                // Prefer submodule variable when importing from a child module inside a wrapper
                // init
                let prefer_submodule_var = inside_wrapper_init
                    && canonical_module_name.starts_with(&format!("{current_module}."));

                log::debug!(
                    "Creating module expression for '{canonical_module_name}': \
                     prefer_submodule_var={prefer_submodule_var}, \
                     at_module_level={at_module_level}, inside_wrapper_init={inside_wrapper_init}"
                );

                let module_expr = if prefer_submodule_var {
                    let var = bundler.get_module_id(&canonical_module_name).map_or_else(
                        || sanitize_module_name_for_identifier(&canonical_module_name),
                        |id| {
                            crate::code_generator::module_registry::get_module_var_identifier(
                                id,
                                bundler.resolver,
                            )
                        },
                    );
                    expressions::name(&var, ExprContext::Load)
                } else if canonical_module_name.contains('.') {
                    // For nested modules like models.user, create models.user expression
                    let parts: Vec<&str> = canonical_module_name.split('.').collect();

                    bundler.create_dotted_module_expr(&parts, at_module_level, &locally_initialized)
                } else {
                    // Top-level module
                    log::debug!(
                        "Top-level module '{canonical_module_name}', \
                         at_module_level={at_module_level}, \
                         inside_wrapper_init={inside_wrapper_init}"
                    );

                    if at_module_level {
                        expressions::name(&canonical_module_name, ExprContext::Load)
                    } else {
                        // Inside a function (either in wrapper init or regular module)
                        // Always use create_function_module_expr for function context
                        // since 'self' won't be available when the function is called
                        log::debug!(
                            "Inside function: accessing={canonical_module_name}, \
                             inside_wrapper_init={inside_wrapper_init}"
                        );
                        bundler.create_function_module_expr(
                            &canonical_module_name,
                            &locally_initialized,
                        )
                    }
                };

                // Special case: If we're at module level in an inlined module importing from a
                // wrapper parent, and the symbol being imported actually comes from
                // another inlined module, we should use the global symbol directly
                // instead of accessing through the wrapper module. This avoids
                // circular dependency issues where the wrapper hasn't been initialized yet.

                // Use the resolved module name (not the canonical name) for checking if it's
                // inlined
                let value_expr = bundler.resolve_import_value_expr(
                    crate::code_generator::bundler::ImportResolveParams {
                        module_expr,
                        module_name, // This is the original module name (e.g., "rich")
                        imported_name,
                        at_module_level,
                        inside_wrapper_init,
                        current_module: Some(current_module),
                        symbol_renames,
                    },
                );

                let assignment =
                    statements::simple_assign(target_name.as_str(), value_expr.clone());

                // Debug log to understand what we're actually generating
                let value_str = match &value_expr {
                    Expr::Name(n) => format!("{}", n.id),
                    Expr::Attribute(a) => {
                        if let Expr::Name(base) = a.value.as_ref() {
                            format!("{}.{}", base.id, a.attr)
                        } else {
                            format!("<expr>.{}", a.attr)
                        }
                    }
                    _ => "<other>".to_owned(),
                };

                log::debug!(
                    "Generating attribute assignment: {} = {} (inside_wrapper_init: {}, resolved \
                     from: {}, canonical: {})",
                    target_name.as_str(),
                    value_str,
                    inside_wrapper_init,
                    module_name,
                    canonical_module_name
                );

                assignments.push(assignment);

                // If we're inside a wrapper init, and this symbol is part of the module's exports,
                // also expose it on the namespace (self.<name> = <name>).
                if inside_wrapper_init
                    && let Some(curr_id) = bundler.get_module_id(current_module)
                    && let Some(Some(exports)) = bundler.module_exports.get(&curr_id)
                    && exports.contains(&target_name.as_str().to_owned())
                {
                    assignments.push(statements::assign_attribute(
                        SELF_PARAM,
                        target_name.as_str(),
                        expressions::name(target_name.as_str(), ExprContext::Load),
                    ));
                }
            }
        }

        assignments
    }
}
