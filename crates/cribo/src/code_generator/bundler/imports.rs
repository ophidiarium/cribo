//! Import routing, resolution, wrapper module initialization, and namespace creation.

use std::path::PathBuf;

use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprContext, Keyword, ModModule, Stmt, StmtImportFrom,
};
use ruff_text_size::TextRange;

use super::{BundledImportContext, Bundler, ImportResolveParams};
use crate::{
    ast_builder::{expressions, other, statements},
    code_generator::module_registry::{INIT_RESULT_VAR, sanitize_module_name_for_identifier},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

impl Bundler<'_> {
    /// Helper: resolve a relative import target to an absolute module name
    pub(super) fn resolve_from_import_target(
        &self,
        module_name: &str,
        from_module: &str,
        level: u32,
    ) -> String {
        if level == 0 {
            return from_module.to_owned();
        }

        // Determine the path of the current module for proper relative resolution.
        // Prefer the resolver (always available) over module_asts (only set after prepare_modules).
        let module_path = self.get_module_id(module_name).and_then(|id| {
            self.resolver.get_module_path(id).or_else(|| {
                self.module_asts
                    .as_ref()
                    .and_then(|asts| asts.get(&id).map(|(_, path, _)| path.clone()))
            })
        });

        let fallback = || {
            let mut pkg = module_name.to_owned();
            // For a level-N relative import, go up N levels from the current module's path
            for _ in 0..level {
                if let Some(pos) = pkg.rfind('.') {
                    pkg.truncate(pos);
                } else {
                    pkg.clear();
                    break;
                }
            }

            let clean = from_module.trim_start_matches('.');
            if pkg.is_empty() {
                clean.to_owned()
            } else if clean.is_empty() {
                pkg
            } else {
                format!("{pkg}.{clean}")
            }
        };

        module_path.map_or_else(fallback, |path| {
            let clean = from_module.trim_start_matches('.');
            let module_str = if clean.is_empty() { None } else { Some(clean) };
            self.resolver
                .resolve_relative_to_absolute_module_name(level, module_str, &path)
                .unwrap_or_else(fallback)
        })
    }

    /// Helper: check if `resolved` is an inlined submodule of `parent`
    fn is_inlined_submodule_of(&self, parent: &str, resolved: &str) -> bool {
        if !resolved.starts_with(&format!("{parent}.")) {
            return false;
        }
        self.get_module_id(resolved)
            .is_some_and(|id| self.inlined_modules.contains(&id))
    }

    /// Helper: collect wrapper-needed-by-inlined from a single `ImportFrom` statement
    pub(crate) fn collect_wrapper_needed_from_importfrom_for_inlinable(
        &self,
        module_id: ModuleId,
        import_from: &StmtImportFrom,
        module_path: &std::path::Path,
        wrapper_modules_saved: &[(ModuleId, ModModule, PathBuf, String)],
        needed: &mut FxIndexSet<ModuleId>,
    ) {
        // Handle "from . import X" pattern
        if import_from.level > 0 && import_from.module.is_none() {
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let parent_module = self.resolver.resolve_relative_to_absolute_module_name(
                    import_from.level,
                    None,
                    module_path,
                );
                let Some(parent) = parent_module else {
                    continue;
                };
                let potential_module = format!("{parent}.{imported_name}");
                if let Some(potential_module_id) = self.get_module_id(&potential_module)
                    && wrapper_modules_saved
                        .iter()
                        .any(|(id, _, _, _)| *id == potential_module_id)
                {
                    needed.insert(potential_module_id);
                    let module_name_str = self
                        .resolver
                        .get_module_name(module_id)
                        .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
                    log::debug!(
                        "Inlined module '{module_name_str}' imports wrapper module \
                         '{potential_module}' via 'from . import'"
                    );
                }
            }
        }

        // Resolve other relative/absolute imports
        let resolved_module = if import_from.level > 0 {
            self.resolver.resolve_relative_to_absolute_module_name(
                import_from.level,
                import_from
                    .module
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str),
                module_path,
            )
        } else {
            import_from.module.as_ref().map(|m| m.as_str().to_owned())
        };

        if let Some(ref resolved) = resolved_module
            && let Some(resolved_id) = self.get_module_id(resolved)
            && wrapper_modules_saved
                .iter()
                .any(|(id, _, _, _)| *id == resolved_id)
        {
            needed.insert(resolved_id);
            let module_name_str = self
                .resolver
                .get_module_name(module_id)
                .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
            log::debug!(
                "Inlined module '{module_name_str}' imports from wrapper module '{resolved}'"
            );
        }
    }

    /// Helper: collect wrapper->wrapper dependencies from a single `ImportFrom` statement
    pub(crate) fn collect_wrapper_to_wrapper_deps_from_stmt(
        &self,
        module_id: ModuleId,
        import_from: &StmtImportFrom,
        module_path: &std::path::Path,
        wrapper_modules_saved: &[(ModuleId, ModModule, PathBuf, String)],
        deps: &mut FxIndexMap<ModuleId, FxIndexSet<ModuleId>>,
    ) {
        // Handle from . import X
        if import_from.level > 0 && import_from.module.is_none() {
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let parent_module = self.resolver.resolve_relative_to_absolute_module_name(
                    import_from.level,
                    None,
                    module_path,
                );
                let Some(parent) = parent_module else {
                    continue;
                };
                let potential_module = format!("{parent}.{imported_name}");
                if let Some(potential_module_id) = self.get_module_id(&potential_module)
                    && wrapper_modules_saved
                        .iter()
                        .any(|(id, _, _, _)| *id == potential_module_id)
                {
                    deps.entry(module_id)
                        .or_default()
                        .insert(potential_module_id);
                }
            }
        }

        // Handle other imports
        let resolved_module = if import_from.level > 0 {
            self.resolver.resolve_relative_to_absolute_module_name(
                import_from.level,
                import_from
                    .module
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str),
                module_path,
            )
        } else {
            import_from.module.as_ref().map(|m| m.as_str().to_owned())
        };
        if let Some(ref resolved) = resolved_module
            && let Some(resolved_id) = self.get_module_id(resolved)
            && wrapper_modules_saved
                .iter()
                .any(|(id, _, _, _)| *id == resolved_id)
        {
            deps.entry(module_id).or_default().insert(resolved_id);
        }
    }

    /// Transform bundled import-from with explicit context (wrapper modules)
    ///
    /// Dispatches to wildcard or symbol import handlers while preserving context flags.
    pub(in crate::code_generator) fn transform_bundled_import_from_multiple_with_current_module(
        &self,
        import_from: &StmtImportFrom,
        module_name: &str,
        context: &BundledImportContext<'_>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        function_body: Option<&[Stmt]>,
    ) -> Vec<Stmt> {
        let inside_wrapper_init = context.inside_wrapper_init;
        let at_module_level = context.at_module_level;
        let current_module = context.current_module;
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

        if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
            return crate::code_generator::import_transformer::transform_wrapper_wildcard_import(
                self,
                import_from,
                module_name,
                inside_wrapper_init,
                current_module,
                at_module_level,
            );
        }

        let new_context = BundledImportContext {
            inside_wrapper_init,
            at_module_level,
            current_module,
            current_function_used_symbols: context.current_function_used_symbols,
        };
        crate::code_generator::import_transformer::transform_wrapper_symbol_imports(
            self,
            import_from,
            module_name,
            &new_context,
            symbol_renames,
            function_body,
        )
    }

    /// Check if a symbol is re-exported from an inlined submodule
    pub(crate) fn is_symbol_from_inlined_submodule(
        &self,
        module_name: &str,
        local_name: &str,
    ) -> Option<(String, String)> {
        // We need to check if this symbol is imported from a submodule and re-exported
        let graph = self.graph?;
        let module = graph.get_module_by_name(module_name)?;

        for item_data in module.items.values() {
            let crate::dependency_graph::ItemType::FromImport {
                module: from_module,
                names,
                level,
                ..
            } = &item_data.item_type
            else {
                continue;
            };

            let resolved_module = self.resolve_from_import_target(module_name, from_module, *level);
            if !self.is_inlined_submodule_of(module_name, &resolved_module) {
                continue;
            }

            // Check if this import includes our symbol
            for (imported_name, alias) in names {
                let local = alias.as_ref().unwrap_or(imported_name);
                if local == local_name {
                    log::debug!(
                        "Symbol '{local_name}' in module '{module_name}' is re-exported from \
                         inlined submodule '{resolved_module}' (original name: '{imported_name}')"
                    );
                    return Some((resolved_module, imported_name.clone()));
                }
            }
        }

        None
    }

    /// Create the entire namespace chain for a module with proper parent-child assignments
    /// For example, for "services.auth.manager", this creates:
    /// - services namespace (if needed)
    /// - `services_auth` namespace (if needed)
    /// - services.auth = `services_auth` assignment
    /// - `services_auth.manager` = `services_auth_manager` assignment
    pub(crate) fn create_namespace_chain_for_module(
        &mut self,
        module_name: &str,
        module_var: &str,
        stmts: &mut Vec<Stmt>,
    ) {
        log::debug!(
            "[NAMESPACE_CHAIN] Called for module_name='{module_name}', module_var='{module_var}'"
        );

        // Split the module name into parts
        let parts: Vec<&str> = module_name.split('.').collect();

        // If it's a top-level module, nothing to do
        if parts.len() <= 1 {
            return;
        }

        // First, ensure ALL parent namespaces exist, including the top-level one
        // We need to create the top-level namespace first if it doesn't exist
        let top_level = parts[0];
        let top_level_var = sanitize_module_name_for_identifier(top_level);
        if !self.created_namespaces.contains(&top_level_var) {
            log::debug!("Creating top-level namespace: {top_level}");
            let namespace_stmts = crate::ast_builder::module_wrapper::create_wrapper_module(
                top_level, "",   // No synthetic name needed for namespace-only
                None, // No init function
                true, // Root namespace must behave like a package (emit __path__)
            );
            // Only the namespace statement should be generated
            if let Some(namespace_stmt) = namespace_stmts.first() {
                stmts.push(namespace_stmt.clone());
            }
            self.created_namespaces.insert(top_level_var);
        }

        // Now create intermediate namespaces
        for i in 1..parts.len() - 1 {
            let current_path = parts[0..=i].join(".");
            let current_var = sanitize_module_name_for_identifier(&current_path);

            // Create namespace if it doesn't exist
            if !self.created_namespaces.contains(&current_var) {
                log::debug!("Creating intermediate namespace: {current_path} (var: {current_var})");
                let namespace_stmts = crate::ast_builder::module_wrapper::create_wrapper_module(
                    &current_path,
                    "",   // No synthetic name needed for namespace-only
                    None, // No init function
                    true, // Mark as package since it has children
                );
                // Only the namespace statement should be generated
                if let Some(namespace_stmt) = namespace_stmts.first() {
                    stmts.push(namespace_stmt.clone());
                }
                self.created_namespaces.insert(current_var.clone());
            }
        }

        // Now create parent.child assignments for the entire chain
        for i in 1..parts.len() {
            let parent_path = parts[0..i].join(".");
            let parent_var = sanitize_module_name_for_identifier(&parent_path);
            let child_name = parts[i];

            // Check if this parent.child assignment has already been made
            let assignment_key = (parent_var.clone(), child_name.to_owned());
            if self.parent_child_assignments_made.contains(&assignment_key) {
                log::debug!(
                    "Skipping duplicate namespace chain assignment: {parent_var}.{child_name} \
                     (already created)"
                );
                continue;
            }

            // Determine the current path and variable
            let current_path = parts[0..=i].join(".");
            let current_var = if i == parts.len() - 1 {
                // This is the leaf module, use the provided module_var
                module_var.to_owned()
            } else {
                // This is an intermediate namespace
                sanitize_module_name_for_identifier(&current_path)
            };

            log::debug!(
                "Creating namespace chain assignment: {parent_var}.{child_name} = {current_var}"
            );

            // Create the assignment: parent.child = child_var
            let assignment = statements::assign(
                vec![expressions::attribute(
                    expressions::name(&parent_var, ExprContext::Load),
                    child_name,
                    ExprContext::Store,
                )],
                expressions::name(&current_var, ExprContext::Load),
            );
            stmts.push(assignment);

            // Track that we've made this assignment
            self.parent_child_assignments_made.insert(assignment_key);
        }
    }
}

impl Bundler<'_> {
    /// Check if a symbol is exported by a module, considering both explicit __all__ and semantic
    /// exports
    fn is_symbol_exported(&self, module_id: ModuleId, symbol_name: &str) -> bool {
        if self.modules_with_explicit_all.contains(&module_id) {
            self.module_exports
                .get(&module_id)
                .and_then(|e| e.as_ref())
                .is_some_and(|exports| exports.contains(&symbol_name.to_owned()))
        } else {
            // Fallback to semantic exports when __all__ is not defined
            self.semantic_exports
                .get(&module_id)
                .is_some_and(|set| set.contains(symbol_name))
        }
    }

    /// Find the source module ID for a symbol that comes from an inlined submodule
    /// This handles wildcard re-exports where a wrapper module imports symbols from inlined modules
    fn find_symbol_source_in_inlined_submodules(
        &self,
        wrapper_id: ModuleId,
        symbol_name: &str,
    ) -> Option<ModuleId> {
        let Some(module_asts) = &self.module_asts else {
            return None;
        };

        let (ast, _, _) = module_asts.get(&wrapper_id)?;

        // Look for wildcard imports in the wrapper module
        for stmt in &ast.body {
            if let Stmt::ImportFrom(import_from) = stmt {
                // Check if this is a wildcard import
                for alias in &import_from.names {
                    if alias.name.as_str() == "*" {
                        // Resolve the imported module
                        let Some(_module_name) = &import_from.module else {
                            continue;
                        };

                        use crate::code_generator::symbol_source::resolve_import_module;

                        let Some(wrapper_path) = self.resolver.get_module_path(wrapper_id) else {
                            continue;
                        };

                        let Some(resolved_module) =
                            resolve_import_module(self.resolver, import_from, &wrapper_path)
                        else {
                            continue;
                        };

                        let Some(source_id) = self.get_module_id(&resolved_module) else {
                            continue;
                        };

                        // Check if this module is inlined and exports the symbol we're looking for
                        if self.inlined_modules.contains(&source_id) {
                            let exported = self.is_symbol_exported(source_id, symbol_name);
                            if exported {
                                log::debug!(
                                    "Found symbol '{symbol_name}' in inlined module \
                                     '{resolved_module}' via wildcard import"
                                );
                                return Some(source_id);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Resolve the value expression for an import, handling special cases for circular dependencies
    pub(in crate::code_generator) fn resolve_import_value_expr(
        &self,
        params: ImportResolveParams<'_>,
    ) -> Expr {
        // Special case: inside wrapper init importing from inlined parent
        if params.inside_wrapper_init {
            log::debug!(
                "resolve_import_value_expr: inside wrapper init, module_name='{}', \
                 imported_name='{}'",
                params.module_name,
                params.imported_name
            );

            // Check if the module we're importing from is inlined
            if let Some(target_id) = self.get_module_id(params.module_name) {
                log::debug!(
                    "  Found module ID {:?} for '{}', is_inlined={}",
                    target_id,
                    params.module_name,
                    self.inlined_modules.contains(&target_id)
                );

                // Entry modules are special - their namespace is populated at runtime,
                // so we should access through the namespace object
                if target_id.is_entry() {
                    log::debug!(
                        "Inside wrapper init: accessing '{}' from entry module '{}' through \
                         namespace",
                        params.imported_name,
                        params.module_name
                    );
                    // Use the namespace object for entry module
                    return expressions::attribute(
                        params.module_expr,
                        params.imported_name,
                        ExprContext::Load,
                    );
                }

                // Check if explicitly inlined (not entry)
                if self.inlined_modules.contains(&target_id) {
                    // The parent module is inlined, so its symbols should be accessed directly
                    // Check if there's a renamed version of this symbol
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

                    // No rename, use the symbol directly
                    log::debug!(
                        "Inside wrapper init: using symbol '{}' directly from inlined module '{}'",
                        params.imported_name,
                        params.module_name
                    );
                    return expressions::name(params.imported_name, ExprContext::Load);
                }
            }
            // Module is not inlined, use normal attribute access
            return expressions::attribute(
                params.module_expr,
                params.imported_name,
                ExprContext::Load,
            );
        }

        // Not at module level, use normal attribute access
        if !params.at_module_level {
            return expressions::attribute(
                params.module_expr,
                params.imported_name,
                ExprContext::Load,
            );
        }

        // Check if current module is inlined and importing from a wrapper parent
        let Some(current_id) = params.current_module.and_then(|m| self.get_module_id(m)) else {
            return expressions::attribute(
                params.module_expr,
                params.imported_name,
                ExprContext::Load,
            );
        };

        if !self.inlined_modules.contains(&current_id) {
            return expressions::attribute(
                params.module_expr,
                params.imported_name,
                ExprContext::Load,
            );
        }

        // Check if the module we're importing from is a wrapper
        let Some(target_id) = self.get_module_id(params.module_name) else {
            return expressions::attribute(
                params.module_expr,
                params.imported_name,
                ExprContext::Load,
            );
        };

        if !self.wrapper_modules.contains(&target_id) {
            return expressions::attribute(
                params.module_expr,
                params.imported_name,
                ExprContext::Load,
            );
        }

        // Try to find if this symbol actually comes from an inlined module
        // First check if there's a renamed version of this symbol
        if let Some(renames) = params.symbol_renames.get(&target_id)
            && let Some(renamed) = renames.get(params.imported_name)
        {
            log::debug!(
                "Using global symbol '{renamed}' directly instead of accessing through wrapper \
                 '{}'",
                params.module_name
            );
            return expressions::name(renamed, ExprContext::Load);
        }

        // Check if this symbol comes from an inlined submodule that was imported via wildcard
        // This handles cases where a wrapper module re-exports symbols from inlined modules
        if let Some(source_module_id) =
            self.find_symbol_source_in_inlined_submodules(target_id, params.imported_name)
        {
            // Check if the source module has a renamed version of this symbol
            if let Some(renames) = params.symbol_renames.get(&source_module_id)
                && let Some(renamed) = renames.get(params.imported_name)
            {
                log::debug!(
                    "Using global symbol '{renamed}' for '{}' from inlined submodule (source: {})",
                    params.imported_name,
                    self.resolver
                        .get_module_name(source_module_id)
                        .unwrap_or_else(|| "unknown".to_owned())
                );
                return expressions::name(renamed, ExprContext::Load);
            }

            // Use the symbol name directly if no rename is needed
            log::debug!(
                "Using global symbol '{}' directly from inlined submodule (source: {})",
                params.imported_name,
                self.resolver
                    .get_module_name(source_module_id)
                    .unwrap_or_else(|| "unknown".to_owned())
            );
            return expressions::name(params.imported_name, ExprContext::Load);
        }

        // Symbol not found as a global, use normal attribute access
        expressions::attribute(params.module_expr, params.imported_name, ExprContext::Load)
    }

    /// Create a module reference assignment
    pub(in crate::code_generator) fn create_module_reference_assignment(
        &self,
        target_name: &str,
        module_name: &str,
    ) -> Stmt {
        // Simply assign the module reference: target_name = module_name
        statements::simple_assign(
            target_name,
            expressions::name(module_name, ExprContext::Load),
        )
    }

    /// Helper method to create dotted module expression with initialization if needed
    pub(in crate::code_generator) fn create_dotted_module_expr(
        &self,
        parts: &[&str],
        at_module_level: bool,
        locally_initialized: &FxIndexSet<ModuleId>,
    ) -> Expr {
        // Module-level or empty: plain dotted expr
        if at_module_level || parts.is_empty() {
            return expressions::dotted_name(parts, ExprContext::Load);
        }

        // Prefer initializing the LEAF module if it's a wrapper and not yet initialized
        // Scan from longest to shortest prefix to find the deepest module that needs init
        for prefix_len in (1..=parts.len()).rev() {
            let prefix_parts = &parts[0..prefix_len];
            let prefix_module = prefix_parts.join(".");

            if let Some(prefix_id) = self.get_module_id(&prefix_module)
                && self.has_synthetic_name(&prefix_module)
                && !locally_initialized.contains(&prefix_id)
                && let Some(init_func_name) = self.module_init_functions.get(&prefix_id)
            {
                // Found a module that needs initialization
                use crate::code_generator::module_registry::get_module_var_identifier;
                let module_var = get_module_var_identifier(prefix_id, self.resolver);

                let globals_call = expressions::call(
                    expressions::name("globals", ExprContext::Load),
                    vec![],
                    vec![],
                );
                let module_ref = expressions::subscript(
                    globals_call,
                    expressions::string_literal(&module_var),
                    ExprContext::Load,
                );
                let mut result = expressions::call(
                    expressions::name(init_func_name, ExprContext::Load),
                    vec![module_ref],
                    vec![],
                );

                // Add remaining attribute access for parts beyond the initialized prefix
                for part in &parts[prefix_len..] {
                    result = expressions::attribute(result, part, ExprContext::Load);
                }

                return result;
            }
        }

        // Fallback: plain dotted expr
        expressions::dotted_name(parts, ExprContext::Load)
    }

    /// Helper method to create module expression for regular function context
    pub(in crate::code_generator) fn create_function_module_expr(
        &self,
        canonical_module_name: &str,
        locally_initialized: &FxIndexSet<ModuleId>,
    ) -> Expr {
        // Check if it's a wrapper module that needs initialization
        if !self.has_synthetic_name(canonical_module_name) {
            // Non-wrapper module
            return expressions::name(canonical_module_name, ExprContext::Load);
        }

        let Some(module_id) = self.get_module_id(canonical_module_name) else {
            return expressions::name(canonical_module_name, ExprContext::Load);
        };

        if locally_initialized.contains(&module_id) {
            return expressions::name(canonical_module_name, ExprContext::Load);
        }

        let Some(init_func_name) = self.module_init_functions.get(&module_id) else {
            return expressions::name(canonical_module_name, ExprContext::Load);
        };

        // Call the init function with the module accessed via globals()
        // to avoid conflicts with local variables
        let globals_call = expressions::call(
            expressions::name("globals", ExprContext::Load),
            vec![],
            vec![],
        );
        let key_name = if canonical_module_name.contains('.') {
            sanitize_module_name_for_identifier(canonical_module_name)
        } else {
            canonical_module_name.to_owned()
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

    /// Create module initialization statements for wrapper modules when they are imported
    pub(in crate::code_generator) fn create_module_initialization_for_import(
        &self,
        module_id: ModuleId,
    ) -> Vec<Stmt> {
        let mut locally_initialized = FxIndexSet::default();
        self.create_module_initialization_for_import_with_tracking(
            module_id,
            &mut locally_initialized,
            None, // No current module context
            true, // At module level by default
        )
    }

    /// Create module initialization statements with current module context
    pub(in crate::code_generator) fn create_module_initialization_for_import_with_current_module(
        &self,
        module_id: ModuleId,
        current_module: Option<ModuleId>,
        at_module_level: bool,
    ) -> Vec<Stmt> {
        let mut locally_initialized = FxIndexSet::default();
        self.create_module_initialization_for_import_with_tracking(
            module_id,
            &mut locally_initialized,
            current_module,
            at_module_level,
        )
    }

    /// Create module initialization statements with tracking to avoid duplicates
    fn create_module_initialization_for_import_with_tracking(
        &self,
        module_id: ModuleId,
        locally_initialized: &mut FxIndexSet<ModuleId>,
        current_module: Option<ModuleId>,
        at_module_level: bool,
    ) -> Vec<Stmt> {
        let mut stmts = Vec::new();

        // Skip if already initialized in this context
        if locally_initialized.contains(&module_id) {
            return stmts;
        }

        // Determine the module name early for checks
        let target_module_name = self
            .resolver
            .get_module(module_id)
            .map_or_else(|| "<unknown>".to_owned(), |m| m.name);

        // If attempting to initialize the entry package from within one of its submodules,
        // skip to avoid circular initialization (e.g., initializing 'requests' while inside
        // 'requests.exceptions'). Python import semantics guarantee the parent package object
        // exists; it shouldn't be (re)initialized by the child.
        if self.entry_is_package_init_or_main
            && self
                .entry_package_name()
                .is_some_and(|pkg| pkg == target_module_name)
            && current_module.is_some()
            && let Some(curr_name) = current_module.and_then(|id| self.resolver.get_module_name(id))
            && curr_name.starts_with(&format!("{target_module_name}."))
        {
            log::debug!(
                "Skipping initialization of entry package '{target_module_name}' from its \
                 submodule '{curr_name}' to avoid circular init"
            );
            return stmts;
        }

        // Skip if we're trying to initialize the current module
        // (we're already inside its init function)
        if let Some(current) = current_module
            && module_id == current
        {
            let module_name = self
                .resolver
                .get_module(module_id)
                .map_or_else(|| "<unknown>".to_owned(), |m| m.name);
            log::debug!(
                "Skipping initialization of module '{module_name}' - already inside its init \
                 function"
            );
            return stmts;
        }

        // Get module name for logging and processing
        let module_name = target_module_name;

        // If this is a child module (contains '.'), ensure parent is initialized first
        if module_name.contains('.')
            && let Some((parent_name, _)) = module_name.rsplit_once('.')
        {
            // Check if parent is also a wrapper module
            if let Some(parent_id) = self.get_module_id(parent_name)
                && self.module_synthetic_names.contains_key(&parent_id)
            {
                // Avoid initializing the entry package (__init__) from within its own submodules.
                // During package initialization, Python allows submodules to import the parent
                // package without re-running its __init__. Re-initializing here can cause
                // circular init (e.g., requests.exceptions -> requests.__init__ ->
                // requests.exceptions).
                let is_entry_parent = self.entry_is_package_init_or_main
                    && self
                        .entry_package_name()
                        .is_some_and(|pkg| pkg == parent_name);

                // Check if parent has an init function and isn't the entry package parent
                // Also avoid initializing a parent namespace when we're currently inside one of
                // its child modules (wrapper init). The child should not re-initialize the parent.
                let in_child_context = current_module
                    .and_then(|id| self.resolver.get_module_name(id))
                    .is_some_and(|curr| curr.starts_with(&format!("{parent_name}.")));

                if self.module_init_functions.contains_key(&parent_id)
                    && !is_entry_parent
                    && !in_child_context
                {
                    log::debug!(
                        "Ensuring parent '{parent_name}' is initialized before child \
                         '{module_name}'"
                    );

                    // Recursively ensure parent is initialized
                    // This will handle multi-level packages like foo.bar.baz
                    stmts.extend(self.create_module_initialization_for_import_with_tracking(
                        parent_id,
                        locally_initialized,
                        current_module,
                        at_module_level,
                    ));
                } else if is_entry_parent || in_child_context {
                    log::debug!(
                        "Skipping initialization of parent '{parent_name}' while initializing \
                         child '{module_name}' to avoid circular init"
                    );
                }
            }
        }

        // Check if this is a wrapper module that needs initialization
        if let Some(synthetic_name) = self.module_synthetic_names.get(&module_id) {
            // Check if the init function has been defined yet
            // (wrapper modules are processed in dependency order, so it might not exist yet)
            log::debug!(
                "Checking if wrapper module '{}' has been processed (has init function: {})",
                module_name,
                self.module_init_functions.contains_key(&module_id)
            );

            // Generate the init call
            let init_func_name =
                crate::code_generator::module_registry::get_init_function_name(synthetic_name);

            // Call the init function with the module as the self argument
            let module_var = sanitize_module_name_for_identifier(&module_name);
            let self_arg = if at_module_level {
                expressions::name(&module_var, ExprContext::Load)
            } else {
                // Use globals()[module_var] to avoid local-name shadowing inside functions
                let globals_call = expressions::call(
                    expressions::name("globals", ExprContext::Load),
                    vec![],
                    vec![],
                );
                expressions::subscript(
                    globals_call,
                    expressions::string_literal(&module_var),
                    ExprContext::Load,
                )
            };
            let init_call = expressions::call(
                expressions::name(&init_func_name, ExprContext::Load),
                vec![self_arg],
                vec![],
            );

            // Generate the appropriate assignment based on module type and scope
            stmts.extend(self.generate_module_assignment_from_init(
                module_id,
                init_call,
                at_module_level,
            ));

            // Mark as initialized to avoid duplicates
            locally_initialized.insert(module_id);

            // Log the initialization for debugging
            if module_name.contains('.') {
                log::debug!(
                    "Created module initialization: {} = {}()",
                    module_name,
                    &init_func_name
                );
            }
        }

        stmts
    }

    /// Generate module assignment from init function result
    fn generate_module_assignment_from_init(
        &self,
        module_id: ModuleId,
        init_call: Expr,
        at_module_level: bool,
    ) -> Vec<Stmt> {
        let mut stmts = Vec::new();

        // Get module name for processing
        let module_name = self
            .resolver
            .get_module(module_id)
            .map_or_else(|| "<unknown>".to_owned(), |m| m.name);

        // Check if this module is a parent namespace that already exists
        let is_parent_namespace = self.bundled_modules.iter().any(|other_module_id| {
            let Some(module_info) = self.resolver.get_module(*other_module_id) else {
                return false;
            };
            let name = &module_info.name;
            name != &module_name && name.starts_with(&format!("{module_name}."))
        });

        if is_parent_namespace {
            // Use temp variable and merge attributes for parent namespaces
            // Store init result in temp variable
            stmts.push(statements::simple_assign(INIT_RESULT_VAR, init_call));

            // Merge attributes from init result into existing namespace
            stmts.push(
                crate::ast_builder::module_attr_merge::generate_merge_module_attributes(
                    &module_name,
                    INIT_RESULT_VAR,
                ),
            );
        } else {
            // Direct assignment for simple and dotted modules
            // For wrapper modules with dots, use the sanitized name
            if at_module_level {
                let target_expr =
                    if module_name.contains('.') && self.has_synthetic_name(&module_name) {
                        // Use sanitized name for wrapper modules
                        let sanitized = sanitize_module_name_for_identifier(&module_name);
                        expressions::name(&sanitized, ExprContext::Store)
                    } else if module_name.contains('.') {
                        // Create attribute expression for dotted modules (inlined)
                        let parts: Vec<&str> = module_name.split('.').collect();
                        expressions::dotted_name(&parts, ExprContext::Store)
                    } else {
                        // Simple name expression
                        expressions::name(&module_name, ExprContext::Store)
                    };
                stmts.push(statements::assign(vec![target_expr], init_call));
            } else {
                // Assign into globals() to avoid creating a local that shadows the module name
                // Determine the key for globals(): sanitized for wrapper dotted modules, or the
                // plain module name otherwise.
                let key_name = if module_name.contains('.') && self.has_synthetic_name(&module_name)
                {
                    sanitize_module_name_for_identifier(&module_name)
                } else {
                    module_name
                };
                let globals_call = expressions::call(
                    expressions::name("globals", ExprContext::Load),
                    vec![],
                    vec![],
                );
                let key_expr = expressions::string_literal(&key_name);
                stmts.push(statements::subscript_assign(
                    globals_call,
                    key_expr,
                    init_call,
                ));
            }
        }

        stmts
    }

    /// Create parent namespaces for dotted imports
    pub(in crate::code_generator) fn create_parent_namespaces(
        &self,
        parts: &[&str],
        result_stmts: &mut Vec<Stmt>,
    ) {
        for i in 1..parts.len() {
            let parent_path = parts[..i].join(".");

            if self.has_synthetic_name(&parent_path) {
                // Parent is a wrapper module, create reference to it
                result_stmts
                    .push(self.create_module_reference_assignment(&parent_path, &parent_path));
            } else if !self
                .get_module_id(&parent_path)
                .is_some_and(|id| self.bundled_modules.contains(&id))
            {
                // Check if this namespace is registered in the centralized system
                let sanitized = sanitize_module_name_for_identifier(&parent_path);

                // Check if we haven't already created this namespace globally or locally
                let already_created = self.created_namespaces.contains(&sanitized)
                    || self.is_namespace_already_created(&parent_path, result_stmts);

                if !already_created {
                    // This parent namespace wasn't registered during initial discovery
                    // This can happen for intermediate namespaces in deeply nested imports
                    // We need to create it inline since we can't register it now (immutable
                    // context)
                    log::debug!(
                        "Creating unregistered parent namespace '{parent_path}' inline during \
                         import transformation"
                    );
                    // Create: parent_path = types.SimpleNamespace(__name__='parent_path')
                    let keywords = vec![Keyword {
                        node_index: AtomicNodeIndex::NONE,
                        arg: Some(other::identifier("__name__")),
                        value: expressions::string_literal(&parent_path),
                        range: TextRange::default(),
                    }];
                    let ns_expr =
                        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords);
                    let path_parts: Vec<&str> = parent_path.split('.').collect();
                    if path_parts.len() == 1 {
                        result_stmts.push(statements::simple_assign(&parent_path, ns_expr));
                    } else {
                        // For dotted paths, construct a proper attribute chain target
                        // e.g. "a.b" → Attribute(Name("a"), "b") instead of Name("a.b")
                        let target_expr = expressions::dotted_name(&path_parts, ExprContext::Store);
                        result_stmts.push(statements::assign(vec![target_expr], ns_expr));
                    }
                }
            }
        }
    }

    /// Check if a namespace module was already created
    fn is_namespace_already_created(&self, parent_path: &str, result_stmts: &[Stmt]) -> bool {
        result_stmts.iter().any(|stmt| {
            let Stmt::Assign(assign) = stmt else {
                return false;
            };
            match assign.targets.first() {
                Some(Expr::Name(name)) => name.id.as_str() == parent_path,
                Some(Expr::Attribute(attr)) => {
                    crate::code_generator::expression_handlers::extract_attribute_path(attr)
                        == parent_path
                }
                _ => false,
            }
        })
    }

    /// Create all namespace objects including the leaf for a dotted import
    pub(in crate::code_generator) fn create_all_namespace_objects(
        &self,
        parts: &[&str],
        result_stmts: &mut Vec<Stmt>,
    ) {
        // For "import a.b.c", we need to create namespace objects for "a", "a.b", and "a.b.c"
        for i in 1..=parts.len() {
            let partial_module = parts[..i].join(".");
            let sanitized_partial = sanitize_module_name_for_identifier(&partial_module);

            // Skip if this module is already a wrapper module
            if self.has_synthetic_name(&partial_module) {
                continue;
            }

            // Skip if this namespace was already created globally
            if self.created_namespaces.contains(&sanitized_partial) {
                log::debug!(
                    "Skipping namespace creation for '{partial_module}' - already created globally"
                );
                continue;
            }

            // Check if we should use a flattened namespace instead of creating an empty one
            let should_use_flattened = self
                .get_module_id(&partial_module)
                .is_some_and(|id| self.inlined_modules.contains(&id));

            // If this namespace already exists as a flattened variable, it was already processed
            // during module inlining, including any parent.child assignments
            if should_use_flattened {
                log::debug!(
                    "Module '{partial_module}' should use flattened namespace \
                     '{sanitized_partial}'. Already created: {}",
                    self.created_namespaces.contains(&sanitized_partial)
                );
                if self.created_namespaces.contains(&sanitized_partial) {
                    log::debug!(
                        "Skipping assignment for '{partial_module}' - already exists as flattened \
                         namespace '{sanitized_partial}'"
                    );
                    continue;
                }
            }

            let namespace_expr = if should_use_flattened {
                // Use the flattened namespace variable
                expressions::name(&sanitized_partial, ExprContext::Load)
            } else {
                // Create empty namespace object
                expressions::call(expressions::simple_namespace_ctor(), vec![], vec![])
            };

            // Assign to the first part of the name
            if i == 1 {
                result_stmts.push(statements::simple_assign(parts[0], namespace_expr));
            } else {
                // For deeper levels, create attribute assignments
                let target_parts = &parts[0..i];
                let target_expr = expressions::dotted_name(target_parts, ExprContext::Store);

                result_stmts.push(statements::assign(vec![target_expr], namespace_expr));
            }
        }
    }

    /// Create a namespace object for an inlined module
    pub(in crate::code_generator) fn create_namespace_object_for_module(
        &self,
        target_name: &str,
        module_name: &str,
    ) -> Stmt {
        // Check if this is an aliased import (target_name != module_name)
        if target_name != module_name {
            // This is an aliased import like `import nested_package.submodule as sub`
            // We should reference the actual module namespace, not create a new one

            if module_name.contains('.') {
                // For dotted module names, reference the namespace hierarchy
                // e.g., for `import a.b.c as alias`, create `alias = a.b.c`
                let parts: Vec<&str> = module_name.split('.').collect();
                return statements::simple_assign(
                    target_name,
                    expressions::dotted_name(&parts, ExprContext::Load),
                );
            }
            // Simple module name, check if it has a flattened variable
            let flattened_name = sanitize_module_name_for_identifier(module_name);
            let should_use_flattened = self
                .get_module_id(module_name)
                .is_some_and(|id| self.inlined_modules.contains(&id));

            if should_use_flattened {
                // Reference the flattened namespace
                return statements::simple_assign(
                    target_name,
                    expressions::name(&flattened_name, ExprContext::Load),
                );
            }
            // Reference the module directly
            return statements::simple_assign(
                target_name,
                expressions::name(module_name, ExprContext::Load),
            );
        }

        // For non-aliased imports, check if we should use a flattened namespace
        let flattened_name = sanitize_module_name_for_identifier(module_name);
        let should_use_flattened = self
            .get_module_id(module_name)
            .is_some_and(|id| self.inlined_modules.contains(&id));

        if should_use_flattened {
            // Create assignment: target_name = flattened_name
            return statements::simple_assign(
                target_name,
                expressions::name(&flattened_name, ExprContext::Load),
            );
        }

        // For inlined modules, we need to return a vector of statements:
        // 1. Create the namespace object
        // 2. Add all the module's symbols to it

        // First, create the empty namespace
        let namespace_expr =
            expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]);

        // For now, return just the namespace creation
        // The actual symbol population needs to happen after all symbols are available
        statements::simple_assign(target_name, namespace_expr)
    }

    /// Derive the parent package for a relative import at the given level.
    pub(super) fn derive_parent_package_for_relative_import(
        &self,
        module_name: &str,
        level: u32,
    ) -> String {
        // First try to resolve using the module's actual path.
        // Prefer the resolver (always available) over module_asts (only set after prepare_modules).
        if let Some(module_id) = self.get_module_id(module_name) {
            let path = self.resolver.get_module_path(module_id).or_else(|| {
                self.module_asts
                    .as_ref()
                    .and_then(|asts| asts.get(&module_id).map(|(_, p, _)| p.clone()))
            });
            if let Some(path) = path
                && let Some(resolved) = self
                    .resolver
                    .resolve_relative_to_absolute_module_name(level, None, &path)
            {
                return resolved;
            }
        }

        // Fallback: strip `level` components from module_name
        let mut pkg = module_name.to_owned();
        for _ in 0..level {
            if let Some((p, _)) = pkg.rsplit_once('.') {
                pkg = p.to_owned();
            } else {
                break;
            }
        }
        pkg
    }
}
