//! Symbol/export decisions, entry module processing, and circular dependency handling.

use std::path::Path;

use ruff_python_ast::{
    Expr, ExprContext, ExprUnaryOp, Stmt, StmtAssign, StmtClassDef, StmtFunctionDef,
    StmtImportFrom, UnaryOp,
    visitor::{self, Visitor},
};

use super::{Bundler, TypeCheckingImportIndex};
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
                    match &stmt {
                        Stmt::Assign(assign) => {
                            pending_reassignment =
                                self.check_renamed_assignment(assign, entry_module_renames);
                        }
                        Stmt::AnnAssign(ann_assign) => {
                            // Annotated assignment: `name: Type = expr`
                            if let Expr::Name(name_expr) = ann_assign.target.as_ref() {
                                let assigned_name = name_expr.id.as_str();
                                for (original, renamed) in entry_module_renames {
                                    if assigned_name == renamed {
                                        pending_reassignment =
                                            Some((original.clone(), renamed.clone()));
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {}
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

        // Apply renames to the entire class including method bodies.
        //
        // Unlike standalone functions (where bodies are skipped because Python resolves
        // names at call time via the local/global scope chain), class method bodies
        // must be rewritten because `global` statements reference module-level names
        // by identity. If a module-level variable is renamed (e.g., `connection` →
        // `connection_1`), any `global connection` inside a method must become
        // `global connection_1` to reference the correct variable at bundle scope.
        if !entry_module_renames.is_empty() {
            // Decorators
            for dec in &mut class_def.decorator_list {
                expression_handlers::rewrite_aliases_in_expr(
                    &mut dec.expression,
                    entry_module_renames,
                );
            }
            // Base classes, keyword arguments, and body (including methods)
            if let Some(arguments) = &mut class_def.arguments {
                for base in &mut arguments.args {
                    expression_handlers::rewrite_aliases_in_expr(base, entry_module_renames);
                }
                for keyword in &mut arguments.keywords {
                    expression_handlers::rewrite_aliases_in_expr(
                        &mut keyword.value,
                        entry_module_renames,
                    );
                }
            }
            for stmt in &mut class_def.body {
                expression_handlers::rewrite_aliases_in_stmt(stmt, entry_module_renames);
            }
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
        matches!(Self::type_checking_branch(expr), Some(true))
    }

    /// Determine which branch of an if statement is guarded by `TYPE_CHECKING`.
    ///
    /// Returns:
    /// - `Some(true)` for `if TYPE_CHECKING:`
    /// - `Some(false)` for `if not TYPE_CHECKING:` (the else branch is type-checking)
    /// - `None` for any other condition
    fn type_checking_branch(expr: &Expr) -> Option<bool> {
        match expr {
            Expr::Name(name) if name.id.as_str() == "TYPE_CHECKING" => Some(true),
            Expr::Attribute(attr)
                if attr.attr.as_str() == "TYPE_CHECKING"
                    && matches!(&*attr.value, Expr::Name(name) if name.id.as_str() == "typing") =>
            {
                Some(true)
            }
            Expr::UnaryOp(ExprUnaryOp {
                op: UnaryOp::Not,
                operand,
                ..
            }) => match Self::type_checking_branch(operand) {
                Some(true) => Some(false),
                Some(false) => Some(true),
                None => None,
            },
            _ => None,
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

        if self.is_symbol_imported_in_type_checking_block(module_id, symbol_name) {
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

        let module_name = self
            .resolver
            .get_module_name(module_id)
            .unwrap_or_else(|| "<unknown>".to_owned());

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

    /// Preserve statement order for circular modules.
    ///
    /// The legacy symbol-level reordering graph was removed because the current bundling
    /// architecture never relies on it at runtime: circular modules are wrapped rather than
    /// inlined, and the entry-module path preserves original order.
    pub(crate) fn reorder_statements_for_circular_module(
        &self,
        module_name: &str,
        statements: Vec<Stmt>,
        _python_version: u8,
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
        } else {
            log::debug!(
                "Preserving statement order for circular module '{module_name}'; \
                 symbol-level reordering is disabled"
            );
        }

        statements
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

        for (other_id, other_ast) in module_asts {
            // Check if the other module is a wrapper
            if !self.wrapper_modules.contains(other_id) {
                continue;
            }

            // Resolve wrapper path once per module (avoids repeated locking/allocation)
            let Some(other_path) = self.resolver.get_module_path(*other_id) else {
                continue;
            };

            // Check if this wrapper imports the symbol
            for stmt in &other_ast.body {
                let Stmt::ImportFrom(import_from) = stmt else {
                    continue;
                };

                use crate::code_generator::symbol_source::resolve_import_module;
                let Some(resolved) =
                    resolve_import_module(self.resolver, import_from, Some(&other_path))
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

    /// Preserve symbols imported solely from `TYPE_CHECKING` blocks until the dependency graph
    /// can model type-only imports precisely.
    fn is_symbol_imported_in_type_checking_block(
        &self,
        module_id: ModuleId,
        symbol_name: &str,
    ) -> bool {
        if self.type_checking_import_index.borrow().is_none() {
            let index = self.build_type_checking_import_index();
            *self.type_checking_import_index.borrow_mut() = Some(index);
        }

        let is_imported = self
            .type_checking_import_index
            .borrow()
            .as_ref()
            .is_some_and(|index| {
                index.get(&module_id).is_some_and(|symbols| {
                    symbols.contains(symbol_name) || symbols.contains(TYPE_CHECKING_WILDCARD)
                })
            });

        if is_imported {
            let module_name = self
                .resolver
                .get_module_name(module_id)
                .unwrap_or_else(|| format!("<unknown module {module_id}>"));
            log::debug!(
                "Keeping symbol '{symbol_name}' from module '{module_name}' because a bundled \
                 module imports it in a TYPE_CHECKING block"
            );
        }

        is_imported
    }

    fn build_type_checking_import_index(&self) -> TypeCheckingImportIndex {
        let Some(module_asts) = &self.module_asts else {
            return TypeCheckingImportIndex::default();
        };

        let mut index = TypeCheckingImportIndex::default();
        for (other_id, other_ast) in module_asts {
            if !self.bundled_modules.contains(other_id) {
                continue;
            }

            let module_path = self.resolver.get_module_path(*other_id);
            let mut collector =
                TypeCheckingImportCollector::new(self.resolver, module_path.as_deref());
            collector.visit_body(&other_ast.body);

            for (imported_module_id, imported_symbols) in collector.into_imports() {
                index
                    .entry(imported_module_id)
                    .or_default()
                    .extend(imported_symbols);
            }
        }

        index
    }
}

const TYPE_CHECKING_WILDCARD: &str = "*";

struct TypeCheckingImportCollector<'a> {
    resolver: &'a crate::resolver::ModuleResolver,
    module_path: Option<&'a Path>,
    imports: TypeCheckingImportIndex,
    type_checking_depth: usize,
}

impl<'a> TypeCheckingImportCollector<'a> {
    fn new(resolver: &'a crate::resolver::ModuleResolver, module_path: Option<&'a Path>) -> Self {
        Self {
            resolver,
            module_path,
            imports: TypeCheckingImportIndex::default(),
            type_checking_depth: 0,
        }
    }

    fn into_imports(self) -> TypeCheckingImportIndex {
        self.imports
    }

    fn collect_import(&mut self, import_from: &StmtImportFrom) {
        use crate::code_generator::symbol_source::resolve_import_module;

        let Some(resolved_module_name) =
            resolve_import_module(self.resolver, import_from, self.module_path)
        else {
            return;
        };

        let Some(imported_module_id) = self.resolver.get_module_id_by_name(&resolved_module_name)
        else {
            return;
        };

        let imported_symbols = self.imports.entry(imported_module_id).or_default();
        for alias in &import_from.names {
            imported_symbols.insert(alias.name.as_str().to_owned());
        }
    }

    fn visit_type_checking_body(&mut self, body: &'a [Stmt], is_type_checking: bool) {
        if is_type_checking {
            self.type_checking_depth += 1;
        }
        self.visit_body(body);
        if is_type_checking {
            self.type_checking_depth -= 1;
        }
    }
}

impl<'a> Visitor<'a> for TypeCheckingImportCollector<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::ImportFrom(import_from) if self.type_checking_depth > 0 => {
                self.collect_import(import_from);
            }
            Stmt::If(if_stmt) => {
                let branch = Bundler::type_checking_branch(&if_stmt.test);

                self.visit_type_checking_body(&if_stmt.body, matches!(branch, Some(true)));

                for clause in &if_stmt.elif_else_clauses {
                    let clause_is_type_checking = matches!(branch, Some(false))
                        || matches!(
                            clause.test.as_ref().and_then(Bundler::type_checking_branch),
                            Some(true)
                        );

                    self.visit_type_checking_body(&clause.body, clause_is_type_checking);
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::Expr;

    use super::*;
    use crate::{config::Config, resolver::ModuleResolver};

    fn parse_assignment_statements(source: &str) -> Vec<Stmt> {
        ruff_python_parser::parse_module(source)
            .expect("test module should parse")
            .into_syntax()
            .body
    }

    fn assignment_targets(statements: &[Stmt]) -> Vec<String> {
        statements
            .iter()
            .map(|stmt| match stmt {
                Stmt::Assign(assign) => match assign.targets.as_slice() {
                    [Expr::Name(name)] => name.id.to_string(),
                    _ => panic!("expected simple assignment target"),
                },
                _ => panic!("expected assignment statement"),
            })
            .collect()
    }

    #[test]
    fn test_type_checking_import_collector_treats_not_type_checking_continuations_as_type_only() {
        let resolver = ModuleResolver::new(Config::default());
        let imported_id = resolver.register_module("types_mod", Path::new("types_mod.py"));

        let statements = ruff_python_parser::parse_module(
            r"
if not TYPE_CHECKING:
    runtime_value = 1
elif condition:
    from types_mod import T
else:
    from types_mod import U
",
        )
        .expect("test module should parse")
        .into_syntax()
        .body;

        let mut collector = TypeCheckingImportCollector::new(&resolver, None);
        collector.visit_body(&statements);
        let imports = collector.into_imports();
        let symbols = imports
            .get(&imported_id)
            .expect("TYPE_CHECKING imports should be collected");

        assert!(symbols.contains("T"));
        assert!(symbols.contains("U"));
    }

    #[test]
    fn test_reorder_statements_for_circular_module_preserves_entry_order() {
        let resolver = ModuleResolver::new(Config::default());
        let mut bundler = Bundler::new(None, &resolver);
        bundler.entry_module_name = "entry_module".to_owned();

        let statements = parse_assignment_statements("first = 1\nsecond = 2\nthird = 3\n");
        let original_order = assignment_targets(&statements);
        let reordered =
            bundler.reorder_statements_for_circular_module("entry_module", statements, 11);

        assert_eq!(assignment_targets(&reordered), original_order);
    }

    #[test]
    fn test_reorder_statements_for_circular_module_preserves_non_entry_order() {
        let resolver = ModuleResolver::new(Config::default());
        let mut bundler = Bundler::new(None, &resolver);
        bundler.entry_module_name = "entry_module".to_owned();

        let statements = parse_assignment_statements("first = 1\nsecond = 2\nthird = 3\n");
        let original_order = assignment_targets(&statements);
        let reordered =
            bundler.reorder_statements_for_circular_module("other_module", statements, 11);

        assert_eq!(assignment_targets(&reordered), original_order);
    }
}
