//! Symbol/export decisions, entry module processing, and circular dependency handling.

use ruff_python_ast::{
    Expr, ExprContext, Stmt, StmtAssign, StmtClassDef, StmtFunctionDef, StmtImportFrom,
};

use super::Bundler;
use crate::{
    analyzers::ImportAnalyzer,
    ast_builder::{expressions, other, statements},
    code_generator::{expression_handlers, module_registry::sanitize_module_name_for_identifier},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

impl Bundler<'_> {
    fn is_duplicate_simple_module_attr_assignment(stmt: &Stmt, final_body: &[Stmt]) -> bool {
        let Stmt::Assign(assign) = stmt else {
            return false;
        };

        if assign.targets.len() != 1 {
            return false;
        }

        let Expr::Attribute(target_attr) = &assign.targets[0] else {
            return false;
        };

        let target_path = expression_handlers::extract_attribute_path(target_attr);

        final_body.iter().any(|stmt| {
            let Stmt::Assign(existing) = stmt else {
                return false;
            };
            let [Expr::Attribute(existing_attr)] = existing.targets.as_slice() else {
                return false;
            };

            // Check if target paths match
            if expression_handlers::extract_attribute_path(existing_attr) == target_path {
                // Only a duplicate if the value expressions are also equal
                return expression_handlers::expressions_are_equal(&existing.value, &assign.value);
            }
            false
        })
    }

    /// Helper: push module attribute assignment `module.local = local`
    fn push_module_attr_assignment(result: &mut Vec<Stmt>, module_name: &str, local_name: &str) {
        let module_var = sanitize_module_name_for_identifier(module_name);
        result.push(
            crate::code_generator::module_registry::create_module_attr_assignment(
                &module_var,
                local_name,
            ),
        );
    }

    /// Helper: handle non-conditional `ImportFrom` exports based on `module_scope_symbols`
    pub(super) fn handle_nonconditional_from_import_exports(
        &self,
        import_from: &StmtImportFrom,
        module_scope_symbols: Option<&FxIndexSet<String>>,
        module_name: &str,
        result: &mut Vec<Stmt>,
    ) {
        let Some(symbols) = module_scope_symbols else {
            return;
        };
        for alias in &import_from.names {
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            if !symbols.contains(local_name) {
                continue;
            }
            if !self.should_export_symbol(local_name, module_name) {
                continue;
            }
            log::debug!("Adding module.{local_name} = {local_name} after non-conditional import");
            Self::push_module_attr_assignment(result, module_name, local_name);
        }
    }

    pub(crate) fn process_entry_module_statement(
        &self,
        stmt: &mut Stmt,
        entry_module_renames: &FxIndexMap<String, String>,
        final_body: &mut Vec<Stmt>,
    ) {
        // For non-import statements in the entry module, apply symbol renames
        let mut pending_reassignment: Option<(String, String)> = None;

        if !entry_module_renames.is_empty() {
            // We need special handling for different statement types
            match stmt {
                Stmt::FunctionDef(func_def) => {
                    pending_reassignment =
                        self.process_entry_module_function(func_def, entry_module_renames);
                }
                Stmt::ClassDef(class_def) => {
                    pending_reassignment =
                        self.process_entry_module_class(class_def, entry_module_renames);
                }
                _ => {
                    // For other statements, use the existing rewrite method
                    expression_handlers::rewrite_aliases_in_stmt(stmt, entry_module_renames);

                    // Check if this is an assignment that was renamed
                    if let Stmt::Assign(assign) = &stmt {
                        pending_reassignment =
                            self.check_renamed_assignment(assign, entry_module_renames);
                    }
                }
            }
        }

        final_body.push(stmt.clone());

        // Add reassignment if needed, but skip if original and renamed are the same
        // or if the reassignment already exists
        if let Some((original, renamed)) = pending_reassignment
            && original != renamed
        {
            // Avoid reintroducing namespace shadowing for the entry module variable name
            let entry_var = sanitize_module_name_for_identifier(&self.entry_module_name);
            if original == entry_var {
                log::debug!(
                    "Skipping alias reassignment '{original}' = '{renamed}' to avoid shadowing \
                     entry namespace"
                );
                return;
            }
            // Check if this reassignment already exists in final_body
            let assignment_exists = final_body.iter().any(|stmt| {
                if let Stmt::Assign(assign) = stmt {
                    if assign.targets.len() == 1 {
                        if let (Expr::Name(target), Expr::Name(value)) =
                            (&assign.targets[0], assign.value.as_ref())
                        {
                            target.id.as_str() == original && value.id.as_str() == renamed
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            });

            if !assignment_exists {
                let reassign = crate::code_generator::module_registry::create_reassignment(
                    &original, &renamed,
                );
                final_body.push(reassign);
            }
        }
    }

    /// Check if a symbol should be exported from a module
    pub(crate) fn should_export_symbol(&self, symbol_name: &str, module_name: &str) -> bool {
        // Don't export __all__ itself as a module attribute
        if symbol_name == "__all__" {
            return false;
        }

        // Get module ID once for reuse
        let module_id = self.get_module_id(module_name);

        // Check if the module has explicit __all__ exports
        // For wrapper modules (which use init functions), do NOT restrict exports to __all__.
        // Wrapper modules should expose public symbols regardless of __all__ to preserve
        // attribute access patterns like `rich.console.Console`.
        let is_wrapper_module =
            module_id.is_some_and(|id| self.module_init_functions.contains_key(&id));
        if !is_wrapper_module
            && let Some(Some(exports)) = module_id.and_then(|id| self.module_exports.get(&id))
        {
            // Module defines __all__, check if symbol is listed there
            if exports.iter().any(|s| s == symbol_name) {
                // Symbol is in __all__. Check per-module tree-shaking to avoid false
                // positives from identically-named symbols in other modules.
                let should_export = module_id
                    .is_some_and(|id| self.is_symbol_kept_by_tree_shaking(id, symbol_name));

                if should_export {
                    log::debug!(
                        "Symbol '{symbol_name}' is in module '{module_name}' __all__ list, \
                         exporting"
                    );
                } else {
                    log::debug!(
                        "Symbol '{symbol_name}' is in __all__ but was completely removed by \
                         tree-shaking, not exporting"
                    );
                }
                return should_export;
            }
        }

        // For symbols not in __all__ (or if no __all__ is defined), check tree-shaking
        let is_kept_by_tree_shaking =
            module_id.is_some_and(|id| self.is_symbol_kept_by_tree_shaking(id, symbol_name));
        if !is_kept_by_tree_shaking {
            log::debug!(
                "Symbol '{symbol_name}' from module '{module_name}' was removed by tree-shaking; \
                 not exporting"
            );
            return false;
        }

        // When tree-shaking is enabled, if a symbol is kept it means it's imported/used somewhere
        // For private symbols (starting with _), we should export them if tree-shaking kept them
        // This handles the case where a private symbol is imported by another module
        if self.tree_shaking_keep_symbols.is_some() {
            // Tree-shaking is enabled and the symbol was kept, so export it
            log::debug!(
                "Symbol '{symbol_name}' from module '{module_name}' kept by tree-shaking, \
                 exporting despite visibility"
            );
            return true;
        }

        // Special case: if a symbol is imported by another module in the bundle, export it
        // even if it starts with underscore. This is necessary for symbols like
        // _is_single_cell_widths in rich.cells that are imported by rich.segment
        if symbol_name.starts_with('_') {
            log::debug!(
                "Checking if private symbol '{symbol_name}' from module '{module_name}' is \
                 imported by other modules"
            );
            if let Some(module_asts) = &self.module_asts {
                // Get the module ID for the current module
                if let Some(module_id) = self.get_module_id(module_name)
                    && ImportAnalyzer::is_symbol_imported_by_other_modules(
                        module_asts,
                        module_id,
                        symbol_name,
                        Some(&self.module_exports),
                        self.resolver,
                    )
                {
                    log::debug!(
                        "Private symbol '{symbol_name}' from module '{module_name}' is imported \
                         by other modules, exporting"
                    );
                    return true;
                }
            }
        }

        // No tree-shaking or no __all__ defined, use default Python visibility rules
        // Export all symbols that don't start with underscore
        let result = !symbol_name.starts_with('_');
        log::debug!(
            "Module '{module_name}' symbol '{symbol_name}' using default visibility: {result}"
        );
        result
    }

    /// Extract simple assignment target name
    /// Check if an assignment references a module that will be created as a namespace
    pub(crate) fn assignment_references_namespace_module(
        &self,
        assign: &StmtAssign,
        module_name: &str,
        _ctx: &crate::code_generator::context::InlineContext<'_>,
    ) -> bool {
        // Check if the RHS is an attribute access on a name
        if let Expr::Attribute(attr) = assign.value.as_ref()
            && let Expr::Name(name) = attr.value.as_ref()
        {
            let base_name = name.id.as_str();

            // First check if this is a stdlib import - if so, it's not a namespace module
            // With proxy approach, stdlib imports are accessed via _cribo and don't conflict
            // with local module names, so we don't need to check for stdlib imports

            // For the specific case we're fixing: if the name "messages" is used
            // and there's a bundled module "greetings.messages", then this assignment
            // needs to be deferred
            for bundled_module_id in &self.bundled_modules {
                // Get the module name to check if it ends with .base_name
                if let Some(module_info) = self.resolver.get_module(*bundled_module_id) {
                    let module_name = &module_info.name;
                    if module_name.ends_with(&format!(".{base_name}")) {
                        // Check if this is an inlined module (will be a namespace)
                        if self.inlined_modules.contains(bundled_module_id) {
                            log::debug!(
                                "Assignment references namespace module: {module_name} (via name \
                                 {base_name})"
                            );
                            return true;
                        }
                    }
                }
            }

            // Also check if the base name itself is an inlined module
            if self
                .get_module_id(base_name)
                .is_some_and(|id| self.inlined_modules.contains(&id))
            {
                log::debug!("Assignment references namespace module directly: {base_name}");
                return true;
            }
        }

        // Also check if the RHS is a plain name that references a namespace module
        if let Expr::Name(name) = assign.value.as_ref() {
            let name_str = name.id.as_str();

            // Check if this name refers to a sibling inlined module that will become a namespace
            // For example, in mypkg.api, "sessions" refers to mypkg.sessions
            if let Some(current_package) = module_name.rsplit_once('.').map(|(pkg, _)| pkg) {
                let potential_sibling = format!("{current_package}.{name_str}");
                if self
                    .get_module_id(&potential_sibling)
                    .is_some_and(|id| self.inlined_modules.contains(&id))
                {
                    log::debug!(
                        "Assignment references sibling namespace module: {potential_sibling} (via \
                         name {name_str})"
                    );
                    return true;
                }
            }

            // Also check if the name itself is an inlined module
            if self
                .get_module_id(name_str)
                .is_some_and(|id| self.inlined_modules.contains(&id))
            {
                log::debug!("Assignment references namespace module directly: {name_str}");
                return true;
            }
        }

        false
    }

    /// Emit namespace attachments for entry module exports
    pub(crate) fn emit_entry_namespace_attachments(
        &mut self,
        entry_pkg: &str,
        final_body: &mut Vec<Stmt>,
        entry_module_symbols: &FxIndexSet<String>,
        entry_module_renames: &FxIndexMap<String, String>,
    ) {
        let namespace_var = sanitize_module_name_for_identifier(entry_pkg);
        log::debug!(
            "Attaching entry module exports to namespace '{namespace_var}' for package \
             '{entry_pkg}'"
        );

        // Ensure the namespace exists before attaching exports
        // This is crucial for packages without submodules where the namespace
        // might not have been created yet
        if !self.created_namespaces.contains(&namespace_var) {
            log::debug!("Creating namespace '{namespace_var}' for entry package exports");
            let namespace_stmt = statements::simple_assign(
                &namespace_var,
                expressions::call(
                    expressions::simple_namespace_ctor(),
                    vec![],
                    vec![
                        expressions::keyword(
                            Some("__name__"),
                            expressions::string_literal(entry_pkg),
                        ),
                        expressions::keyword(
                            Some("__initializing__"),
                            expressions::bool_literal(false),
                        ),
                        expressions::keyword(
                            Some("__initialized__"),
                            expressions::bool_literal(false),
                        ),
                    ],
                ),
            );
            final_body.push(namespace_stmt);
            self.created_namespaces.insert(namespace_var.clone());
        }

        // Collect all top-level symbols defined in the entry module
        // that should be attached to the namespace
        let mut exports_to_attach = Vec::new();

        // Check if module has explicit __all__ to determine exports
        if let Some(Some(all_exports)) = self.module_exports.get(&ModuleId::ENTRY) {
            // Module has __all__: respect export policy and tree-shaking
            for export_name in all_exports {
                if self.should_export_symbol(export_name, &self.entry_module_name) {
                    exports_to_attach.push(export_name.clone());
                }
            }
            log::debug!("Using __all__ exports for namespace attachment: {exports_to_attach:?}");
        } else {
            // No __all__: defer to should_export_symbol for visibility + tree-shaking
            for symbol in entry_module_symbols {
                if self.should_export_symbol(symbol, &self.entry_module_name) {
                    exports_to_attach.push(symbol.clone());
                }
            }
            log::debug!(
                "Attaching public symbols from entry module to namespace: {exports_to_attach:?}"
            );
        }

        // Sort and deduplicate exports
        exports_to_attach.sort();
        exports_to_attach.dedup();

        // Generate attachment statements: namespace.symbol = symbol
        for symbol_name in exports_to_attach {
            // Check if this symbol was renamed due to conflicts
            let actual_name = entry_module_renames
                .get(&symbol_name)
                .unwrap_or(&symbol_name);

            log::debug!(
                "Attaching '{symbol_name}' (actual: '{actual_name}') to namespace \
                 '{namespace_var}'"
            );

            let attach_stmt = statements::assign(
                vec![expressions::attribute(
                    expressions::name(&namespace_var, ExprContext::Load),
                    &symbol_name,
                    ExprContext::Store,
                )],
                expressions::name(actual_name, ExprContext::Load),
            );

            // Only add if not a duplicate
            if !Self::is_duplicate_simple_module_attr_assignment(&attach_stmt, final_body) {
                final_body.push(attach_stmt);
            }
        }
    }

    /// Process a function definition in the entry module
    fn process_entry_module_function(
        &self,
        func_def: &mut StmtFunctionDef,
        entry_module_renames: &FxIndexMap<String, String>,
    ) -> Option<(String, String)> {
        let func_name = func_def.name.to_string();
        let needs_reassignment = if let Some(new_name) = entry_module_renames.get(&func_name) {
            log::debug!("Renaming function '{func_name}' to '{new_name}' in entry module");
            func_def.name = other::identifier(new_name);
            true
        } else {
            false
        };

        // Rewrite definition-time expressions that may reference renamed symbols.
        // The function BODY is NOT rewritten — Python resolves those at call time.
        if !entry_module_renames.is_empty() {
            // Decorators
            for dec in &mut func_def.decorator_list {
                expression_handlers::rewrite_aliases_in_expr(
                    &mut dec.expression,
                    entry_module_renames,
                );
            }
            // Default parameter values and annotations
            for param in func_def
                .parameters
                .args
                .iter_mut()
                .chain(func_def.parameters.posonlyargs.iter_mut())
                .chain(func_def.parameters.kwonlyargs.iter_mut())
            {
                if let Some(ref mut default) = param.default {
                    expression_handlers::rewrite_aliases_in_expr(default, entry_module_renames);
                }
                if let Some(ref mut ann) = param.parameter.annotation {
                    expression_handlers::rewrite_aliases_in_expr(ann, entry_module_renames);
                }
            }
            if let Some(ref mut vararg) = func_def.parameters.vararg
                && let Some(ref mut ann) = vararg.annotation
            {
                expression_handlers::rewrite_aliases_in_expr(ann, entry_module_renames);
            }
            if let Some(ref mut kwarg) = func_def.parameters.kwarg
                && let Some(ref mut ann) = kwarg.annotation
            {
                expression_handlers::rewrite_aliases_in_expr(ann, entry_module_renames);
            }
            // Return type annotation
            if let Some(ref mut returns) = func_def.returns {
                expression_handlers::rewrite_aliases_in_expr(returns, entry_module_renames);
            }
        }

        if needs_reassignment {
            Some((func_name.clone(), entry_module_renames[&func_name].clone()))
        } else {
            None
        }
    }

    /// Process a class definition in the entry module
    fn process_entry_module_class(
        &self,
        class_def: &mut StmtClassDef,
        entry_module_renames: &FxIndexMap<String, String>,
    ) -> Option<(String, String)> {
        let class_name = class_def.name.to_string();
        let needs_reassignment = if let Some(new_name) = entry_module_renames.get(&class_name) {
            log::debug!("Renaming class '{class_name}' to '{new_name}' in entry module");
            class_def.name = other::identifier(new_name);
            true
        } else {
            false
        };

        // Apply renames to class body - classes don't create new scopes for globals
        // Apply renames to the entire class (including base classes and body)
        // We need to create a temporary Stmt to pass to rewrite_aliases_in_stmt
        let mut temp_stmt = Stmt::ClassDef(class_def.clone());
        expression_handlers::rewrite_aliases_in_stmt(&mut temp_stmt, entry_module_renames);
        if let Stmt::ClassDef(updated_class) = temp_stmt {
            *class_def = updated_class;
        }

        if needs_reassignment {
            Some((
                class_name.clone(),
                entry_module_renames[&class_name].clone(),
            ))
        } else {
            None
        }
    }

    /// Check if an assignment statement needs a reassignment due to renaming
    fn check_renamed_assignment(
        &self,
        assign: &StmtAssign,
        entry_module_renames: &FxIndexMap<String, String>,
    ) -> Option<(String, String)> {
        if assign.targets.len() != 1 {
            return None;
        }

        let Expr::Name(name_expr) = &assign.targets[0] else {
            return None;
        };

        let assigned_name = name_expr.id.as_str();
        // Check if this is a renamed variable (e.g., Logger_1)
        for (original, renamed) in entry_module_renames {
            if assigned_name == renamed {
                // This is a renamed assignment, mark for reassignment
                return Some((original.clone(), renamed.clone()));
            }
        }
        None
    }

    /// Check if a condition is a `TYPE_CHECKING` check
    pub(super) fn is_type_checking_condition(expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
            Expr::Attribute(attr) => {
                attr.attr.as_str() == "TYPE_CHECKING"
                    && matches!(&*attr.value, Expr::Name(name) if name.id.as_str() == "typing")
            }
            _ => false,
        }
    }

    pub(crate) fn should_inline_symbol(
        &self,
        symbol_name: &str,
        module_id: ModuleId,
        module_exports_map: &FxIndexMap<ModuleId, Option<Vec<String>>>,
    ) -> bool {
        let kept_by_tree_shaking = self.is_symbol_kept_by_tree_shaking(module_id, symbol_name);
        let has_explicit_all = self.modules_with_explicit_all.contains(&module_id);

        // Wrapper modules need runtime access to imported symbols regardless of __all__.
        // Check this BEFORE __all__ exclusion to avoid discarding symbols that wrapper
        // init code relies on at runtime.
        if self.is_symbol_imported_by_wrapper(module_id, symbol_name) {
            return true;
        }

        // If module has explicit __all__ and symbol is not in it, don't inline it
        // even if tree-shaking kept it (it might be referenced but shouldn't be accessible)
        if has_explicit_all
            && let Some(Some(export_list)) = module_exports_map.get(&module_id)
            && !export_list.iter().any(|e| e == symbol_name)
        {
            log::debug!(
                "Not inlining symbol '{symbol_name}' from module with explicit __all__ - not in \
                 export list"
            );
            return false;
        }

        // If tree-shaking kept the symbol, include it
        if kept_by_tree_shaking {
            return true;
        }

        // From here, kept_by_tree_shaking is false.

        // Symbol in explicit __all__ should be kept (re-exported but not used internally)
        if has_explicit_all {
            let exports = module_exports_map.get(&module_id).and_then(|e| e.as_ref());
            if let Some(export_list) = exports
                && export_list.iter().any(|e| e == symbol_name)
            {
                return true;
            }
        }

        // If tree-shaking is disabled, check export list
        if self.tree_shaking_keep_symbols.is_none() {
            let exports = module_exports_map.get(&module_id).and_then(|e| e.as_ref());
            if let Some(export_list) = exports
                && export_list.iter().any(|e| e == symbol_name)
            {
                return true;
            }
        }

        // Fallback: keep symbols explicitly imported by other modules
        let module_name = self
            .resolver
            .get_module_name(module_id)
            .unwrap_or_else(|| "<unknown>".to_owned());

        if let Some(module_asts) = &self.module_asts
            && ImportAnalyzer::is_symbol_imported_by_other_modules(
                module_asts,
                module_id,
                symbol_name,
                Some(&self.module_exports),
                self.resolver,
            )
        {
            log::debug!(
                "Keeping symbol '{symbol_name}' from module '{module_name}' because it is \
                 imported by other modules"
            );
            return true;
        }

        log::trace!(
            "Tree shaking: removing unused symbol '{symbol_name}' from module '{module_name}'"
        );
        false
    }

    /// Get a unique name for a symbol, using the module suffix pattern
    pub(crate) fn get_unique_name_with_module_suffix(
        &self,
        base_name: &str,
        module_name: &str,
    ) -> String {
        let module_suffix = sanitize_module_name_for_identifier(module_name);
        format!("{base_name}_{module_suffix}")
    }

    /// Reorder statements in a module based on symbol dependencies for circular modules
    pub(crate) fn reorder_statements_for_circular_module(
        &self,
        module_name: &str,
        statements: Vec<Stmt>,
        python_version: u8,
    ) -> Vec<Stmt> {
        log::debug!(
            "reorder_statements_for_circular_module called for module: '{}' (entry_module_name: \
             '{}', entry_is_package_init_or_main: {})",
            module_name,
            self.entry_module_name,
            self.entry_is_package_init_or_main
        );

        // Check if this is the entry module - entry modules should not have their
        // statements reordered even if they're part of circular dependencies
        let is_entry_module = if self.entry_is_package_init_or_main {
            // If entry is __init__.py or __main__.py, the module might be identified
            // by its package name (e.g., 'yaml' instead of '__init__')
            self.entry_package_name().map_or_else(
                || module_name == self.entry_module_name,
                |entry_pkg| module_name == entry_pkg,
            )
        } else {
            // Direct comparison for regular entry modules
            module_name == self.entry_module_name
        };

        if is_entry_module {
            log::debug!(
                "Skipping statement reordering for entry module: '{module_name}' \
                 (entry_module_name: '{}', entry_is_package_init_or_main: {})",
                self.entry_module_name,
                self.entry_is_package_init_or_main
            );
            return statements;
        }

        log::debug!("Proceeding with statement reordering for module: '{module_name}'");

        // Get the ordered symbols for this module from the dependency graph
        let ordered_symbols = self
            .symbol_dep_graph
            .get_module_symbols_ordered(module_name);

        if ordered_symbols.is_empty() {
            // No ordering information, return statements as-is
            return statements;
        }

        log::debug!(
            "Reordering statements for circular module '{module_name}' based on symbol order: \
             {ordered_symbols:?}"
        );

        // Create a map from symbol name to statement
        let mut symbol_to_stmt: FxIndexMap<String, Stmt> = FxIndexMap::default();
        let mut other_stmts = Vec::new();
        let mut imports = Vec::new();

        for stmt in statements {
            match &stmt {
                Stmt::FunctionDef(func_def) => {
                    let name = func_def.name.to_string();
                    if symbol_to_stmt.contains_key(&name) {
                        other_stmts.push(stmt);
                    } else {
                        symbol_to_stmt.insert(name, stmt);
                    }
                }
                Stmt::ClassDef(class_def) => {
                    let name = class_def.name.to_string();
                    if symbol_to_stmt.contains_key(&name) {
                        other_stmts.push(stmt);
                    } else {
                        symbol_to_stmt.insert(name, stmt);
                    }
                }
                Stmt::Assign(assign) => {
                    if let Some(name) = expression_handlers::extract_simple_assign_target(assign) {
                        // Skip self-referential assignments - they'll be handled later
                        if expression_handlers::is_self_referential_assignment(
                            assign,
                            python_version,
                        ) {
                            log::debug!(
                                "Skipping self-referential assignment '{name}' in circular module \
                                 reordering"
                            );
                            other_stmts.push(stmt);
                        } else if symbol_to_stmt.contains_key(&name) {
                            // If we already have a function/class with this name, keep the
                            // function/class and treat the assignment
                            // as a regular statement
                            log::debug!(
                                "Assignment '{name}' conflicts with existing function/class, \
                                 keeping function/class"
                            );
                            other_stmts.push(stmt);
                        } else {
                            symbol_to_stmt.insert(name, stmt);
                        }
                    } else {
                        other_stmts.push(stmt);
                    }
                }
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // Keep imports at the beginning
                    imports.push(stmt);
                }
                _ => {
                    // Other statements maintain their relative order
                    other_stmts.push(stmt);
                }
            }
        }

        // Build the reordered statement list
        let mut reordered = Vec::new();

        // First, add all imports
        reordered.extend(imports);

        // Then add symbols in the specified order
        for symbol in &ordered_symbols {
            if let Some(stmt) = symbol_to_stmt.shift_remove(symbol) {
                reordered.push(stmt);
            }
        }

        // Add any remaining symbols that weren't in the ordered list
        reordered.extend(symbol_to_stmt.into_values());

        // Finally, add other statements
        reordered.extend(other_stmts);

        reordered
    }

    /// Resolve import aliases in a statement
    pub(crate) fn resolve_import_aliases_in_stmt(
        stmt: &mut Stmt,
        import_aliases: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::Expr(expr_stmt) => {
                expression_handlers::resolve_import_aliases_in_expr(
                    &mut expr_stmt.value,
                    import_aliases,
                );
            }
            Stmt::Assign(assign) => {
                expression_handlers::resolve_import_aliases_in_expr(
                    &mut assign.value,
                    import_aliases,
                );
                // Don't transform targets - we only resolve aliases in expressions
            }
            Stmt::Return(ret_stmt) => {
                if let Some(value) = &mut ret_stmt.value {
                    expression_handlers::resolve_import_aliases_in_expr(value, import_aliases);
                }
            }
            _ => {}
        }
    }
}

// Helper methods for import rewriting
impl Bundler<'_> {
    /// Check if a module is part of circular dependencies (unpruned check)
    /// This is more accurate than checking `circular_modules` which may be pruned
    pub(crate) fn is_module_in_circular_deps(&self, module_id: ModuleId) -> bool {
        self.all_circular_modules.contains(&module_id)
    }

    /// Check if a symbol is imported by any wrapper module
    fn is_symbol_imported_by_wrapper(&self, module_id: ModuleId, symbol_name: &str) -> bool {
        let Some(module_name) = self.resolver.get_module_name(module_id) else {
            return false;
        };

        let Some(module_asts) = &self.module_asts else {
            return false;
        };

        for (other_id, (other_ast, other_path, _)) in module_asts {
            // Check if the other module is a wrapper
            if !self.wrapper_modules.contains(other_id) {
                continue;
            }

            // Check if this wrapper imports the symbol
            for stmt in &other_ast.body {
                let Stmt::ImportFrom(import_from) = stmt else {
                    continue;
                };

                use crate::code_generator::symbol_source::resolve_import_module;
                let Some(resolved) = resolve_import_module(self.resolver, import_from, other_path)
                else {
                    continue;
                };

                if resolved != module_name {
                    continue;
                }

                // Check if this specific symbol is imported
                for alias in &import_from.names {
                    if alias.name.as_str() == symbol_name || alias.name.as_str() == "*" {
                        log::debug!(
                            "Keeping symbol '{symbol_name}' from module '{module_name}' because \
                             wrapper module imports it"
                        );
                        return true;
                    }
                }
            }
        }

        false
    }
}
