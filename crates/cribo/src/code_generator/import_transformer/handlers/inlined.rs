use ruff_python_ast::{Expr, ExprContext, Stmt, StmtImportFrom};

use crate::{
    ast_builder::{expressions, statements},
    code_generator::bundler::Bundler,
    types::{FxIndexMap, FxIndexSet},
};

/// Handle inlined module import transformations
pub(crate) struct InlinedHandler;

impl InlinedHandler {
    /// Check if importing from an inlined module
    pub(in crate::code_generator::import_transformer) fn is_importing_from_inlined_module(
        module_name: &str,
        bundler: &Bundler<'_>,
    ) -> bool {
        bundler
            .get_module_id(module_name)
            .is_some_and(|id| bundler.inlined_modules.contains(&id))
    }

    /// Create namespace call for inlined module with all its symbols
    pub(in crate::code_generator::import_transformer) fn create_namespace_call_for_inlined_module(
        module_name: &str,
        module_renames: Option<&FxIndexMap<String, String>>,
        bundler: &Bundler<'_>,
    ) -> Expr {
        // Create a types.SimpleNamespace with all the module's symbols
        let mut keywords = Vec::new();
        let mut seen_args = FxIndexSet::default();

        // Add all renamed symbols as keyword arguments, avoiding duplicates
        if let Some(renames) = module_renames {
            for (original_name, renamed_name) in renames {
                // Check if the renamed name was already added
                if seen_args.contains(renamed_name) {
                    log::debug!(
                        "Skipping duplicate namespace argument '{renamed_name}' (from \
                         '{original_name}') for module '{module_name}'"
                    );
                    continue;
                }

                // Check if this symbol survived tree-shaking
                let module_id = bundler
                    .get_module_id(module_name)
                    .expect("Module should exist");
                if !bundler.is_symbol_kept_by_tree_shaking(module_id, original_name) {
                    log::debug!(
                        "Skipping tree-shaken symbol '{original_name}' from namespace for module \
                         '{module_name}'"
                    );
                    continue;
                }

                seen_args.insert(renamed_name.clone());

                keywords.push(expressions::keyword(
                    Some(original_name),
                    expressions::name(renamed_name, ExprContext::Load),
                ));
            }
        }

        // Also check if module has module-level variables that weren't renamed
        if let Some(module_id) = bundler.get_module_id(module_name)
            && let Some(exports) = bundler.module_exports.get(&module_id)
            && let Some(export_list) = exports
        {
            for export in export_list {
                // Check if this export was already added as a renamed symbol
                let was_renamed =
                    module_renames.is_some_and(|renames| renames.contains_key(export));
                if !was_renamed && !seen_args.contains(export) {
                    // Check if this symbol survived tree-shaking
                    if !bundler.is_symbol_kept_by_tree_shaking(module_id, export) {
                        log::debug!(
                            "Skipping tree-shaken export '{export}' from namespace for module \
                             '{module_name}'"
                        );
                        continue;
                    }

                    // This export wasn't renamed and wasn't already added, add it directly
                    seen_args.insert(export.clone());
                    keywords.push(expressions::keyword(
                        Some(export),
                        expressions::name(export, ExprContext::Load),
                    ));
                }
            }
        }

        // Create types.SimpleNamespace(**kwargs) call
        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords)
    }

    /// Create `local = namespace_var` if names differ
    pub(in crate::code_generator::import_transformer) fn alias_local_to_namespace_if_needed(
        local_name: &str,
        namespace_var: &str,
        result_stmts: &mut Vec<Stmt>,
    ) {
        if local_name == namespace_var {
            return;
        }
        log::debug!("  Creating immediate local alias: {local_name} = {namespace_var}");
        result_stmts.push(statements::simple_assign(
            local_name,
            expressions::name(namespace_var, ExprContext::Load),
        ));
    }

    /// Handle imports from inlined modules
    ///
    /// This function handles import statements that import from modules that have been inlined
    /// into the bundle. It generates appropriate assignment statements to make the inlined
    /// symbols available under their expected names.
    pub(in crate::code_generator::import_transformer) fn handle_imports_from_inlined_module_with_context(
        bundler: &Bundler<'_>,
        import_from: &StmtImportFrom,
        source_module_id: crate::resolver::ModuleId,
        symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
        is_wrapper_init: bool,
        importing_module_id: Option<crate::resolver::ModuleId>,
    ) -> Vec<Stmt> {
        let module_name = bundler
            .resolver
            .get_module_name(source_module_id)
            .unwrap_or_else(|| format!("module#{source_module_id}"));
        log::debug!(
            "handle_imports_from_inlined_module_with_context: source_module={}, \
             available_renames={:?}",
            module_name,
            symbol_renames.get(&source_module_id)
        );
        let mut result_stmts = Vec::new();

        // Check if this is a wildcard import
        if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
            // Handle wildcard import from inlined module
            log::debug!("Handling wildcard import from inlined module '{module_name}'");

            // Get the module's exports (either from __all__ or all non-private symbols)
            let module_exports = if let Some(Some(export_list)) =
                bundler.module_exports.get(&source_module_id)
            {
                // Module has __all__ defined, use it
                export_list.clone()
            } else if let Some(semantic_exports) = bundler.semantic_exports.get(&source_module_id) {
                // Use semantic exports from analysis
                semantic_exports.iter().cloned().collect()
            } else {
                // No export information available
                log::warn!(
                    "No export information available for inlined module '{module_name}' with \
                     wildcard import"
                );
                return result_stmts;
            };

            log::debug!(
                "Generating wildcard import assignments for {} symbols from inlined module '{}'",
                module_exports.len(),
                module_name
            );

            // Get symbol renames for this module
            let module_renames = symbol_renames.get(&source_module_id);

            // Cache explicit __all__ (if any) to avoid repeated lookups
            let explicit_all = bundler
                .module_exports
                .get(&source_module_id)
                .and_then(|exports| exports.as_ref());

            for symbol_name in &module_exports {
                // Skip private symbols unless explicitly in __all__
                if symbol_name.starts_with('_')
                    && !explicit_all.is_some_and(|all| all.contains(symbol_name))
                {
                    continue;
                }

                // Check if the source symbol was tree-shaken
                if !bundler.is_symbol_kept_by_tree_shaking(source_module_id, symbol_name) {
                    log::debug!(
                        "Skipping wildcard import for tree-shaken symbol '{symbol_name}' from \
                         module '{module_name}'"
                    );
                    continue;
                }

                // Get the renamed symbol name if it was renamed
                let renamed_symbol = module_renames.map_or_else(
                    || symbol_name.clone(),
                    |renames| {
                        renames
                            .get(symbol_name)
                            .cloned()
                            .unwrap_or_else(|| symbol_name.clone())
                    },
                );

                // For wildcard imports, we always need to create assignments for renamed symbols
                // For non-renamed symbols, we only skip assignment if they're actually available
                // in the current scope (i.e., they are in the module_exports list which respects
                // __all__)
                if renamed_symbol == *symbol_name {
                    // Symbol wasn't renamed - it's already accessible in scope for symbols
                    // that are in module_exports (which respects __all__)
                    log::debug!(
                        "Symbol '{symbol_name}' is accessible directly from inlined module"
                    );
                } else {
                    // Symbol was renamed, create an alias assignment
                    result_stmts.push(statements::simple_assign(
                        symbol_name,
                        expressions::name(&renamed_symbol, ExprContext::Load),
                    ));
                    log::debug!(
                        "Created wildcard import alias for renamed symbol: {symbol_name} = \
                         {renamed_symbol}"
                    );
                }
            }

            return result_stmts;
        }

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // First check if we're importing a submodule (e.g., from package import submodule)
            let full_module_path = format!("{module_name}.{imported_name}");
            if let Some(submodule_id) = bundler.get_module_id(&full_module_path)
                && bundler.bundled_modules.contains(&submodule_id)
            {
                // This is importing a submodule, not a symbol
                // When the current module is inlined, we need to create a local alias
                // to the submodule's namespace variable
                if bundler.inlined_modules.contains(&submodule_id) {
                    // The submodule is inlined, create alias: local_name = module_var
                    use crate::code_generator::module_registry::get_module_var_identifier;
                    let module_var = get_module_var_identifier(submodule_id, bundler.resolver);

                    log::debug!(
                        "Creating submodule alias in inlined module: {local_name} = {module_var}"
                    );

                    // Create the assignment
                    result_stmts.push(statements::simple_assign(
                        local_name,
                        expressions::name(&module_var, ExprContext::Load),
                    ));
                } else {
                    log::debug!(
                        "Skipping submodule import '{imported_name}' from '{module_name}' - \
                         wrapper module import should be handled elsewhere"
                    );
                }
                continue;
            }

            // Prefer precise re-export detection from inlined submodules
            let renamed_symbol = if let Some((source_module, source_symbol)) =
                bundler.is_symbol_from_inlined_submodule(&module_name, imported_name)
            {
                // Apply symbol renames from the source module if they exist
                let source_module_id = bundler
                    .get_module_id(&source_module)
                    .expect("Source module should exist");
                let global_name = symbol_renames
                    .get(&source_module_id)
                    .and_then(|renames| renames.get(&source_symbol))
                    .cloned()
                    .unwrap_or(source_symbol);

                log::debug!(
                    "Resolved re-exported symbol via inlined submodule: \
                     {module_name}.{imported_name} -> {global_name}"
                );
                global_name
            } else {
                // Fallback: package re-export heuristic only if there is no explicit rename
                let is_package_reexport = is_package_init_reexport(bundler, &module_name);
                let has_rename = symbol_renames
                    .get(&source_module_id)
                    .and_then(|renames| renames.get(imported_name))
                    .is_some();

                log::debug!(
                    "  is_package_reexport for module '{module_name}': {is_package_reexport}, \
                     has_rename: {has_rename}"
                );

                if is_package_reexport && !has_rename {
                    log::debug!(
                        "Using original name '{imported_name}' for symbol imported from package \
                         '{module_name}' (no rename found)"
                    );
                    imported_name.to_owned()
                } else {
                    symbol_renames
                        .get(&source_module_id)
                        .and_then(|renames| renames.get(imported_name))
                        .cloned()
                        .unwrap_or_else(|| imported_name.to_owned())
                }
            };

            log::debug!(
                "Processing import: module={}, imported_name={}, local_name={}, \
                 renamed_symbol={}, available_renames={:?}",
                module_name,
                imported_name,
                local_name,
                renamed_symbol,
                symbol_renames.get(&source_module_id)
            );

            // Check if the source symbol was tree-shaken.
            // IMPORTANT: Do not skip symbols in wrapper init functions (__init__.py).
            // Re-exports from package __init__ must be preserved even if not used by entry.
            if !is_wrapper_init
                && !bundler.is_symbol_kept_by_tree_shaking(source_module_id, imported_name)
            {
                log::debug!(
                    "Skipping import assignment for tree-shaken symbol '{imported_name}' from \
                     module '{module_name}' (non-wrapper context)"
                );
                continue;
            }

            // Handle wrapper init functions specially
            if is_wrapper_init {
                // When importing from an inlined module, we need to create the local alias FIRST
                // before setting the module attribute, because the module attribute assignment
                // uses the local name which won't exist until we create the alias

                // Note: source_module_id always corresponds to an inlined module when this function
                // is called, so we can simplify the logic by removing the redundant
                // is_from_inlined check.

                // When importing from an inlined module inside a wrapper init,
                // prefer qualifying with the module's namespace when the names are identical
                // to avoid creating a self-referential assignment like `x = x`.
                let source_expr = if local_name == renamed_symbol {
                    let module_namespace =
                        crate::code_generator::module_registry::get_module_var_identifier(
                            source_module_id,
                            bundler.resolver,
                        );
                    log::debug!(
                        "Creating local alias from namespace: {local_name} = \
                         {module_namespace}.{imported_name}"
                    );
                    expressions::attribute(
                        expressions::name(&module_namespace, ExprContext::Load),
                        imported_name,
                        ExprContext::Load,
                    )
                } else {
                    log::debug!(
                        "Creating local alias from global symbol: {local_name} = {renamed_symbol} \
                         (imported from inlined module {module_name})"
                    );
                    expressions::name(&renamed_symbol, ExprContext::Load)
                };
                result_stmts.push(statements::simple_assign(local_name, source_expr));

                // Now set the module attribute using the local name (which now exists)
                if let Some(current_mod_id) = importing_module_id {
                    let current_mod_name = bundler
                        .resolver
                        .get_module_name(current_mod_id)
                        .unwrap_or_else(|| format!("module#{current_mod_id}"));
                    let module_var =
                        crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                            &current_mod_name,
                        );
                    // When importing from an inlined module (which is always the case for this
                    // function), use the local name we just created
                    let attr_value = local_name;
                    log::debug!(
                        "Creating module attribute assignment in wrapper init: \
                         {module_var}.{local_name} = {attr_value}"
                    );
                    result_stmts.push(
                    crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                        &module_var,
                        local_name,
                        attr_value,
                    ),
                );

                    // Note: We skip the self.<name> = <name> assignment for imports from inlined
                    // modules to avoid redundant assignments inside inlined
                    // init functions. Since this function only handles inlined
                    // modules, we don't add the self assignment.
                } else {
                    log::warn!(
                        "is_wrapper_init is true but current_module is None, skipping module \
                         attribute assignment"
                    );
                }
            } else if local_name != renamed_symbol {
                // For non-wrapper contexts, only create assignment if names differ
                // For inlined modules, reference the namespace attribute instead of the renamed
                // symbol directly This avoids ordering issues where the renamed
                // symbol might not be defined yet
                let module_namespace =
                    crate::code_generator::module_registry::get_module_var_identifier(
                        source_module_id,
                        bundler.resolver,
                    );
                log::debug!(
                    "Creating assignment: {local_name} = {module_namespace}.{imported_name}"
                );
                result_stmts.push(statements::simple_assign(
                    local_name,
                    expressions::attribute(
                        expressions::name(&module_namespace, ExprContext::Load),
                        imported_name,
                        ExprContext::Load,
                    ),
                ));
            } else if local_name == renamed_symbol && local_name != imported_name {
                // Even when local_name == renamed_symbol, if it differs from imported_name,
                // we need to create an assignment to the namespace attribute
                let module_namespace =
                    crate::code_generator::module_registry::get_module_var_identifier(
                        source_module_id,
                        bundler.resolver,
                    );
                log::debug!(
                    "Creating assignment: {local_name} = {module_namespace}.{imported_name}"
                );
                result_stmts.push(statements::simple_assign(
                    local_name,
                    expressions::attribute(
                        expressions::name(&module_namespace, ExprContext::Load),
                        imported_name,
                        ExprContext::Load,
                    ),
                ));
            }
        }

        result_stmts
    }

    /// Handle from-import on resolved inlined module
    pub(in crate::code_generator::import_transformer) fn handle_from_import_on_resolved_inlined(
        transformer: &crate::code_generator::import_transformer::RecursiveImportTransformer<'_>,
        import_from: &StmtImportFrom,
        resolved: &str,
    ) -> Option<Vec<Stmt>> {
        // Check if this is an inlined module
        if let Some(resolved_id) = transformer.state.bundler.get_module_id(resolved)
            && transformer
                .state
                .bundler
                .inlined_modules
                .contains(&resolved_id)
        {
            // Check if this is a circular module with pre-declarations
            if transformer
                .state
                .bundler
                .circular_modules
                .contains(&resolved_id)
            {
                log::debug!("  Module '{resolved}' is a circular module with pre-declarations");
                log::debug!(
                    "  Current module '{}' is circular: {}, is inlined: {}",
                    transformer
                        .state
                        .bundler
                        .resolver
                        .get_module_name(transformer.state.module_id)
                        .unwrap_or_else(|| format!("module#{}", transformer.state.module_id)),
                    transformer
                        .state
                        .bundler
                        .circular_modules
                        .contains(&transformer.state.module_id),
                    transformer
                        .state
                        .bundler
                        .inlined_modules
                        .contains(&transformer.state.module_id)
                );
                // Special handling for imports between circular inlined modules
                // If the current module is also a circular inlined module, we need to defer or
                // transform differently
                if transformer
                    .state
                    .bundler
                    .circular_modules
                    .contains(&transformer.state.module_id)
                    && transformer
                        .state
                        .bundler
                        .inlined_modules
                        .contains(&transformer.state.module_id)
                {
                    log::debug!(
                        "  Both modules are circular and inlined - transforming to direct \
                         assignments"
                    );
                    // Generate direct assignments since both modules will be in the same scope
                    let mut assignments = Vec::new();
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                        // Check if this is actually a submodule import
                        let full_submodule_path = format!("{resolved}.{imported_name}");
                        log::debug!(
                            "  Checking if '{full_submodule_path}' is a submodule (bundled: {}, \
                             inlined: {})",
                            transformer
                                .state
                                .bundler
                                .get_module_id(&full_submodule_path)
                                .is_some_and(|id| transformer
                                    .state
                                    .bundler
                                    .bundled_modules
                                    .contains(&id)),
                            transformer
                                .state
                                .bundler
                                .get_module_id(&full_submodule_path)
                                .is_some_and(|id| transformer
                                    .state
                                    .bundler
                                    .inlined_modules
                                    .contains(&id))
                        );
                        if transformer
                            .state
                            .bundler
                            .get_module_id(&full_submodule_path)
                            .is_some_and(|id| {
                                transformer.state.bundler.bundled_modules.contains(&id)
                            })
                            || transformer
                                .state
                                .bundler
                                .get_module_id(&full_submodule_path)
                                .is_some_and(|id| {
                                    transformer.state.bundler.inlined_modules.contains(&id)
                                })
                        {
                            log::debug!(
                                "  Skipping assignment for '{imported_name}' - it's a submodule, \
                                 not a symbol"
                            );
                            // This is a submodule import, not a symbol import
                            // The submodule will be handled separately, so we don't create an
                            // assignment
                            continue;
                        }

                        // Check if the symbol was renamed during bundling
                        let actual_name = if let Some(resolved_id) =
                            transformer.state.bundler.get_module_id(resolved)
                            && let Some(renames) =
                                transformer.state.symbol_renames.get(&resolved_id)
                        {
                            renames
                                .get(imported_name)
                                .map_or(imported_name, String::as_str)
                        } else {
                            imported_name
                        };

                        // Create assignment: local_name = actual_name
                        if local_name != actual_name {
                            assignments.push(statements::simple_assign(
                                local_name,
                                expressions::name(actual_name, ExprContext::Load),
                            ));
                        }
                    }
                    return Some(assignments);
                }
                // Original behavior for non-circular modules importing from circular
                // modules
                return Some(Self::handle_imports_from_inlined_module_with_context(
                    transformer.state.bundler,
                    import_from,
                    resolved_id,
                    transformer.state.symbol_renames,
                    transformer.state.is_wrapper_init,
                    Some(transformer.state.module_id),
                ));
            }
            log::debug!("  Module '{resolved}' is inlined, handling import assignments");
            // For the entry module, we should not defer these imports
            // because they need to be available when the entry module's code runs
            let import_stmts = Self::handle_imports_from_inlined_module_with_context(
                transformer.state.bundler,
                import_from,
                resolved_id,
                transformer.state.symbol_renames,
                transformer.state.is_wrapper_init,
                Some(transformer.state.module_id),
            );

            // Only defer if we're not in the entry module or wrapper init
            if transformer.state.module_id.is_entry() || transformer.state.is_wrapper_init {
                // For entry module and wrapper init functions, return the imports
                // immediately In wrapper init functions, module
                // attributes need to be set where the import was
                if !import_stmts.is_empty() {
                    return Some(import_stmts);
                }
                // If handle_imports_from_inlined_module returned empty (e.g., for submodule
                // imports), fall through to check if we need to
                // handle it differently
                log::debug!(
                    "  handle_imports_from_inlined_module returned empty for entry module or \
                     wrapper init, checking for submodule imports"
                );
            } else {
                // Return the import statements immediately
                // These were previously deferred but now need to be added immediately
                return Some(import_stmts);
            }
        }

        None
    }

    /// Handle inlined from-import in absolute context (namespace/submodules + assignments)
    pub(in crate::code_generator::import_transformer) fn handle_inlined_from_import_absolute_context(
        bundler: &Bundler<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
        inside_wrapper_init: bool,
        python_version: u8,
    ) -> Vec<Stmt> {
        // Module was inlined - but first check if we're importing bundled submodules
        // e.g., from my_package import utils where my_package.utils is a bundled module
        if super::super::has_bundled_submodules(import_from, module_name, bundler) {
            log::debug!(
                "Inlined module '{module_name}' has bundled submodules, using \
                 transform_namespace_package_imports"
            );
            // Use namespace package imports for bundled submodules
            return crate::code_generator::namespace_manager::transform_namespace_package_imports(
                bundler,
                import_from.clone(),
                module_name,
                symbol_renames,
            );
        }

        // Module was inlined - create assignments for imported symbols
        log::debug!(
            "Module '{module_name}' was inlined, creating assignments for imported symbols"
        );

        let params = crate::code_generator::module_registry::InlinedImportParams {
            symbol_renames,
            module_registry: bundler.module_info_registry,
            inlined_modules: &bundler.inlined_modules,
            bundled_modules: &bundler.bundled_modules,
            resolver: bundler.resolver,
            python_version,
            is_wrapper_init: inside_wrapper_init,
            tree_shaking_check: Some(&|module_id, symbol| {
                bundler.is_symbol_kept_by_tree_shaking(module_id, symbol)
            }),
        };

        // Create assignments for inlined imports
        // Namespace requirements are handled dynamically during transformation
        crate::code_generator::module_registry::create_assignments_for_inlined_imports(
            import_from,
            module_name,
            &params,
        )
    }

    /// Handle entry-module resolution as inlined fast-path
    pub(in crate::code_generator::import_transformer) fn handle_entry_relative_as_inlined(
        bundler: &Bundler<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
        inside_wrapper_init: bool,
        current_module: &str,
    ) -> Option<Vec<Stmt>> {
        // Check if this is the entry module or entry.__main__
        let entry_module_id = bundler.get_module_id(module_name).map_or_else(
            || {
                if module_name.ends_with(".__main__") {
                    // Check if this is <entry>.__main__ where <entry> is the entry module
                    let base_module = module_name
                        .strip_suffix(".__main__")
                        .expect("checked with ends_with above");
                    log::debug!("  Checking if base module '{base_module}' is entry");
                    let base_id = bundler.get_module_id(base_module);
                    log::debug!("  Base module ID: {base_id:?}");
                    base_id.filter(|id| id.is_entry())
                } else {
                    None
                }
            },
            |module_id| {
                if module_id.is_entry() {
                    Some(module_id)
                } else {
                    None
                }
            },
        );

        log::debug!(
            "Checking if '{module_name}' is entry module: entry_module_id={entry_module_id:?}"
        );

        if let Some(module_id) = entry_module_id {
            log::debug!(
                "Relative import resolves to entry module '{module_name}' (ID {module_id}), \
                 treating as inlined"
            );
            // Get the importing module's ID
            let importing_module_id = bundler.resolver.get_module_id_by_name(current_module);
            // Handle imports from the entry module
            return Some(Self::handle_imports_from_inlined_module_with_context(
                bundler,
                import_from,
                module_id,
                symbol_renames,
                inside_wrapper_init,
                importing_module_id,
            ));
        }

        None
    }

    /// Transform imports if the module has bundled submodules
    pub(in crate::code_generator::import_transformer) fn transform_if_has_bundled_submodules(
        bundler: &Bundler<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    ) -> Option<Vec<Stmt>> {
        if crate::code_generator::import_transformer::has_bundled_submodules(
            import_from,
            module_name,
            bundler,
        ) {
            // We have bundled submodules, need to transform them
            log::debug!("Module '{module_name}' has bundled submodules, transforming imports");
            log::debug!("  Found bundled submodules:");
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let full_module_path = format!("{module_name}.{imported_name}");
                if bundler
                    .get_module_id(&full_module_path)
                    .is_some_and(|id| bundler.bundled_modules.contains(&id))
                {
                    log::debug!("    - {full_module_path}");
                }
            }
            // Transform each submodule import
            return Some(
                crate::code_generator::namespace_manager::transform_namespace_package_imports(
                    bundler,
                    import_from.clone(),
                    module_name,
                    symbol_renames,
                ),
            );
        }

        None
    }

    /// Maybe handle inlined absolute imports (non-bundled case)
    pub(in crate::code_generator::import_transformer) fn maybe_handle_inlined_absolute(
        bundler: &Bundler<'_>,
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
        inside_wrapper_init: bool,
        current_module: &str,
    ) -> Option<Vec<Stmt>> {
        // Check if this module is inlined
        if let Some(source_module_id) = bundler.get_module_id(module_name)
            && bundler.inlined_modules.contains(&source_module_id)
        {
            log::debug!(
                "Module '{module_name}' is an inlined module, \
                 inside_wrapper_init={inside_wrapper_init}"
            );
            // Get the importing module's ID
            let importing_module_id = bundler.resolver.get_module_id_by_name(current_module);
            // Handle imports from inlined modules
            return Some(Self::handle_imports_from_inlined_module_with_context(
                bundler,
                import_from,
                source_module_id,
                symbol_renames,
                inside_wrapper_init,
                importing_module_id,
            ));
        }

        None
    }
}

/// Check if a module is a package __init__.py that re-exports from submodules
fn is_package_init_reexport(bundler: &Bundler<'_>, module_name: &str) -> bool {
    // Special handling for package __init__.py files
    // If we're importing from "greetings" and there's a "greetings.X" module
    // that could be the source of the symbol

    // For now, check if this looks like a package (no dots) and if there are
    // any inlined submodules
    if !module_name.contains('.') {
        // Check if any inlined module starts with module_name.
        if bundler.inlined_modules.iter().any(|inlined_id| {
            bundler
                .resolver
                .get_module_name(*inlined_id)
                .is_some_and(|name| name.starts_with(&format!("{module_name}.")))
        }) {
            log::debug!("Module '{module_name}' appears to be a package with inlined submodules");
            // For the specific case of greetings/__init__.py importing from
            // greetings.english, we assume the symbol should use its
            // original name
            return true;
        }
    }
    false
}
