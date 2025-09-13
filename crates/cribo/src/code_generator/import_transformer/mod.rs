use std::path::Path;

use cow_utils::CowUtils;
use ruff_python_ast::{
    AtomicNodeIndex, ExceptHandler, Expr, ExprContext, ExprName, Identifier, ModModule, Stmt,
    StmtClassDef, StmtImport, StmtImportFrom,
};
use ruff_text_size::TextRange;

use crate::{
    analyzers::symbol_analyzer::SymbolAnalyzer,
    ast_builder::{expressions, statements},
    code_generator::{
        bundler::Bundler, import_deduplicator, module_registry::sanitize_module_name_for_identifier,
    },
    types::{FxIndexMap, FxIndexSet},
};

mod expr_rewriter;
mod handlers;
mod state;
mod statement;

use expr_rewriter::ExpressionRewriter;
use handlers::{
    dynamic::DynamicHandler, inlined::InlinedHandler, stdlib::StdlibHandler,
    submodule::SubmoduleHandler, wrapper::WrapperHandler,
};
// Re-export the params struct for external use
pub use state::RecursiveImportTransformerParams;
use state::TransformerState;
use statement::StatementProcessor;

/// Transformer that recursively handles import statements and module references
pub struct RecursiveImportTransformer<'a> {
    state: TransformerState<'a>,
}

/// Public bridge for Bundler to delegate wrapper wildcard from-import handling
pub(crate) fn transform_wrapper_wildcard_import(
    bundler: &Bundler,
    import_from: &StmtImportFrom,
    module_name: &str,
    inside_wrapper_init: bool,
    current_module: Option<&str>,
    at_module_level: bool,
) -> Vec<Stmt> {
    handlers::wrapper::WrapperHandler::handle_wildcard_import_from_multiple(
        bundler,
        import_from,
        module_name,
        inside_wrapper_init,
        current_module,
        at_module_level,
    )
}

impl<'a> RecursiveImportTransformer<'a> {
    /// Get filtered exports for a full module path, if available
    fn get_filtered_exports_for_path(
        &self,
        full_module_path: &str,
    ) -> Option<(crate::resolver::ModuleId, Vec<String>)> {
        let module_id = self.state.bundler.get_module_id(full_module_path)?;
        let exports = self
            .state
            .bundler
            .module_exports
            .get(&module_id)
            .cloned()
            .flatten()?;
        let filtered: Vec<String> = SymbolAnalyzer::filter_exports_by_tree_shaking(
            &exports,
            &module_id,
            self.state.bundler.tree_shaking_keep_symbols.as_ref(),
            false,
            self.state.bundler.resolver,
        )
        .into_iter()
        .cloned()
        .collect();
        Some((module_id, filtered))
    }

    /// Check if wrapper import assignments should be skipped due to type-only usage
    fn should_skip_assignments_for_type_only_imports(&self, import_from: &StmtImportFrom) -> bool {
        if let Some(used_symbols) = &self.state.current_function_used_symbols {
            let uses_alias = import_from.names.iter().any(|a| {
                let local = a.asname.as_ref().unwrap_or(&a.name).as_str();
                used_symbols.contains(local)
            });
            !uses_alias
        } else {
            false
        }
    }

    /// Should emit __all__ for a local namespace binding
    fn should_emit_all_for_local(
        &self,
        module_id: crate::resolver::ModuleId,
        local_name: &str,
        filtered_exports: &[String],
    ) -> bool {
        !filtered_exports.is_empty()
            && self
                .state
                .bundler
                .modules_with_explicit_all
                .contains(&module_id)
            && self
                .state
                .bundler
                .modules_with_accessed_all
                .iter()
                .any(|(module, alias)| module == &self.state.module_id && alias == local_name)
    }

    /// Mark namespace as populated for a module path if needed (non-bundled, not yet marked)
    fn mark_namespace_populated_if_needed(&mut self, full_module_path: &str) {
        let full_module_id = self.state.bundler.get_module_id(full_module_path);
        let namespace_already_populated =
            full_module_id.is_some_and(|id| self.state.populated_modules.contains(&id));
        let is_bundled_module =
            full_module_id.is_some_and(|id| self.state.bundler.bundled_modules.contains(&id));
        if !is_bundled_module
            && !namespace_already_populated
            && let Some(id) = full_module_id
        {
            self.state.populated_modules.insert(id);
        }
    }

    /// Emit namespace symbols for a local binding from a full module path
    fn emit_namespace_symbols_for_local_from_path(
        &self,
        local_name: &str,
        full_module_path: &str,
        result_stmts: &mut Vec<Stmt>,
    ) {
        if let Some((module_id, filtered_exports)) =
            self.get_filtered_exports_for_path(full_module_path)
        {
            if self.should_emit_all_for_local(module_id, local_name, &filtered_exports) {
                let export_strings: Vec<&str> =
                    filtered_exports.iter().map(String::as_str).collect();
                result_stmts.push(statements::set_list_attribute(
                    local_name,
                    "__all__",
                    &export_strings,
                ));
            }

            for symbol in filtered_exports {
                let target = expressions::attribute(
                    expressions::name(local_name, ExprContext::Load),
                    &symbol,
                    ExprContext::Store,
                );
                let symbol_name = self
                    .state
                    .bundler
                    .get_module_id(full_module_path)
                    .and_then(|id| self.state.symbol_renames.get(&id))
                    .and_then(|renames| renames.get(&symbol))
                    .cloned()
                    .unwrap_or_else(|| symbol.clone());
                let value = expressions::name(&symbol_name, ExprContext::Load);
                result_stmts.push(statements::assign(vec![target], value));
            }
        }
    }

    /// Check if a module is used as a namespace object (imported as namespace)
    fn is_namespace_object(&self, module_name: &str) -> bool {
        self.state
            .bundler
            .get_module_id(module_name)
            .is_some_and(|id| {
                self.state
                    .bundler
                    .namespace_imported_modules
                    .contains_key(&id)
            })
    }

    /// Try to rewrite `base.attr_name` where base aliases an inlined module
    fn try_rewrite_single_attr_for_inlined_module_alias(
        &self,
        base: &str,
        actual_module: &str,
        attr_name: &str,
        ctx: ExprContext,
        range: TextRange,
    ) -> Option<Expr> {
        let potential_submodule = format!("{actual_module}.{attr_name}");
        // If this points to a wrapper module, don't transform
        if self
            .state
            .bundler
            .get_module_id(&potential_submodule)
            .is_some_and(|id| self.state.bundler.bundled_modules.contains(&id))
            && !self
                .state
                .bundler
                .get_module_id(&potential_submodule)
                .is_some_and(|id| self.state.bundler.inlined_modules.contains(&id))
        {
            log::debug!("Not transforming {base}.{attr_name} - it's a wrapper module access");
            return None;
        }

        // Don't transform if it's a namespace object
        if self.is_namespace_object(actual_module) {
            log::debug!(
                "Not transforming {base}.{attr_name} - accessing namespace object attribute"
            );
            return None;
        }

        // Prefer semantic rename map if available
        if let Some(module_id) = self.state.bundler.get_module_id(actual_module)
            && let Some(module_renames) = self.state.symbol_renames.get(&module_id)
        {
            if let Some(renamed) = module_renames.get(attr_name) {
                let renamed_str = renamed.clone();
                log::debug!("Rewrote {base}.{attr_name} to {renamed_str} (renamed)");
                return Some(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::NONE,
                    id: renamed_str.into(),
                    ctx,
                    range,
                }));
            }
            // Avoid collapsing to bare name if it would create self-referential assignment
            if let Some(lhs) = &self.state.current_assignment_targets
                && lhs.contains(attr_name)
            {
                log::debug!(
                    "Skipping collapse of {base}.{attr_name} to avoid self-referential assignment"
                );
                return None;
            }
            log::debug!("Rewrote {base}.{attr_name} to {attr_name} (not renamed)");
            return Some(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::NONE,
                id: attr_name.into(),
                ctx,
                range,
            }));
        }

        // Fallback: if module exports include the name, use it directly
        if self
            .state
            .bundler
            .get_module_id(actual_module)
            .and_then(|id| self.state.bundler.module_exports.get(&id))
            .and_then(|opt| opt.as_ref())
            .is_some_and(|exports| exports.contains(&attr_name.to_string()))
        {
            if let Some(lhs) = &self.state.current_assignment_targets
                && lhs.contains(attr_name)
            {
                log::debug!(
                    "Skipping collapse of {base}.{attr_name} (exported) to avoid self-reference"
                );
                return None;
            }
            log::debug!("Rewrote {base}.{attr_name} to {attr_name} (exported by module)");
            return Some(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::NONE,
                id: attr_name.into(),
                ctx,
                range,
            }));
        }

        None
    }

    /// Handle parent.child alias when importing from the same parent module, with early exits
    fn maybe_log_parent_child_assignment(
        &self,
        import_base: Option<&str>,
        imported_name: &str,
        local_name: &str,
    ) {
        if import_base
            != Some(
                self.state
                    .bundler
                    .resolver
                    .get_module_name(self.state.module_id)
                    .unwrap_or_else(|| format!("module#{}", self.state.module_id))
                    .as_str(),
            )
        {
            return;
        }

        // Check if this submodule is in the parent's __all__ exports
        let parent_exports = self
            .state
            .bundler
            .module_exports
            .get(&self.state.module_id)
            .and_then(|opt| opt.as_ref())
            .is_some_and(|exports| exports.contains(&imported_name.to_string()));
        if !parent_exports {
            return;
        }

        let full_submodule_path = format!(
            "{}.{}",
            self.state
                .bundler
                .resolver
                .get_module_name(self.state.module_id)
                .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
            imported_name
        );
        let is_inlined_submodule = self
            .state
            .bundler
            .get_module_id(&full_submodule_path)
            .is_some_and(|id| self.state.bundler.inlined_modules.contains(&id));
        let uses_init_function = self
            .state
            .bundler
            .get_module_id(&full_submodule_path)
            .and_then(|id| self.state.bundler.module_init_functions.get(&id))
            .is_some();

        log::debug!(
            "  Checking submodule status for {full_submodule_path}: \
             is_inlined={is_inlined_submodule}, uses_init={uses_init_function}"
        );

        if is_inlined_submodule || uses_init_function {
            log::debug!(
                "  Skipping parent module assignment for {}.{} - already handled by init function",
                self.state
                    .bundler
                    .resolver
                    .get_module_name(self.state.module_id)
                    .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
                local_name
            );
            return;
        }

        // Double-check if this is actually a module
        let is_actually_a_module = self
            .state
            .bundler
            .get_module_id(&full_submodule_path)
            .is_some_and(|id| {
                self.state.bundler.bundled_modules.contains(&id)
                    || self
                        .state
                        .bundler
                        .module_info_registry
                        .as_ref()
                        .is_some_and(|reg| reg.contains_module(&id))
                    || self.state.bundler.inlined_modules.contains(&id)
            });
        if is_actually_a_module {
            log::debug!(
                "Skipping assignment for {}.{} - it's a module, not a symbol",
                self.state
                    .bundler
                    .resolver
                    .get_module_name(self.state.module_id)
                    .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
                local_name
            );
            return;
        }

        // At this point, we would create parent.local = local if needed.
        // Original code only logged due to deferred imports removal.
        log::debug!(
            "Creating parent module assignment: {}.{} = {} (symbol exported from parent)",
            self.state
                .bundler
                .resolver
                .get_module_name(self.state.module_id)
                .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
            local_name,
            local_name
        );
    }

    /// If accessing attribute on an inlined submodule, rewrite to direct access symbol name
    fn maybe_rewrite_attr_for_inlined_submodule(
        &self,
        base: &str,
        actual_module: &str,
        attr_path: &[String],
        attr_ctx: ExprContext,
        attr_range: TextRange,
    ) -> Option<Expr> {
        // Check if base.attr_path[0] forms a complete module name
        let potential_module = format!("{}.{}", actual_module, attr_path[0]);
        if self
            .state
            .bundler
            .get_module_id(&potential_module)
            .is_some_and(|id| self.state.bundler.inlined_modules.contains(&id))
            && attr_path.len() == 2
        {
            let final_attr = &attr_path[1];
            if let Some(module_id) = self.state.bundler.get_module_id(&potential_module)
                && let Some(module_renames) = self.state.symbol_renames.get(&module_id)
                && let Some(renamed) = module_renames.get(final_attr)
            {
                log::debug!("Rewrote {base}.{}.{final_attr} to {renamed}", attr_path[0]);
                return Some(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::NONE,
                    id: renamed.clone().into(),
                    ctx: attr_ctx,
                    range: attr_range,
                }));
            }

            // No rename, use the original name with module prefix
            let direct_name = format!(
                "{final_attr}_{}",
                potential_module.cow_replace('.', "_").as_ref()
            );
            log::debug!(
                "Rewrote {base}.{}.{final_attr} to {direct_name}",
                attr_path[0]
            );
            return Some(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::NONE,
                id: direct_name.into(),
                ctx: attr_ctx,
                range: attr_range,
            }));
        }
        None
    }

    /// Create a new transformer from parameters
    pub fn new(params: RecursiveImportTransformerParams<'a>) -> Self {
        Self {
            state: TransformerState::new(params),
        }
    }

    /// Get whether any types.SimpleNamespace objects were created
    pub fn created_namespace_objects(&self) -> bool {
        self.state.created_namespace_objects
    }

    /// Get the import aliases map
    pub fn import_aliases(&self) -> &FxIndexMap<String, String> {
        &self.state.import_aliases
    }

    /// Get mutable access to the import aliases map
    pub fn import_aliases_mut(&mut self) -> &mut FxIndexMap<String, String> {
        &mut self.state.import_aliases
    }

    /// Transform a module recursively, handling all imports at any depth
    pub(crate) fn transform_module(&mut self, module: &mut ModModule) {
        log::debug!(
            "RecursiveImportTransformer::transform_module for '{}'",
            self.state
                .bundler
                .resolver
                .get_module_name(self.state.module_id)
                .unwrap_or_else(|| format!("module#{}", self.state.module_id))
        );
        // Transform all statements recursively
        self.transform_statements(&mut module.body);
    }

    /// Transform a list of statements recursively
    fn transform_statements(&mut self, stmts: &mut Vec<Stmt>) {
        log::debug!(
            "RecursiveImportTransformer::transform_statements: Processing {} statements",
            stmts.len()
        );
        let mut i = 0;
        while i < stmts.len() {
            // First check if this is an import statement that needs transformation
            let is_import = matches!(&stmts[i], Stmt::Import(_) | Stmt::ImportFrom(_));
            let is_hoisted = if is_import {
                import_deduplicator::is_hoisted_import(self.state.bundler, &stmts[i])
            } else {
                false
            };

            if is_import {
                log::debug!(
                    "transform_statements: Found import in module '{}', is_hoisted={}",
                    self.state
                        .bundler
                        .resolver
                        .get_module_name(self.state.module_id)
                        .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
                    is_hoisted
                );
            }

            let needs_transformation = is_import && !is_hoisted;

            if needs_transformation {
                // Transform the import statement
                let transformed = self.transform_statement(&mut stmts[i]);

                log::debug!(
                    "transform_statements: Transforming import in module '{}', got {} statements \
                     back",
                    self.state
                        .bundler
                        .resolver
                        .get_module_name(self.state.module_id)
                        .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
                    transformed.len()
                );

                // Remove the original statement
                stmts.remove(i);

                // Insert all transformed statements
                let num_inserted = transformed.len();
                for (j, new_stmt) in transformed.into_iter().enumerate() {
                    stmts.insert(i + j, new_stmt);
                }

                // Skip past the inserted statements
                i += num_inserted;
            } else {
                // For non-import statements, recurse into nested structures and transform
                // expressions
                match &mut stmts[i] {
                    Stmt::Assign(assign_stmt) => {
                        // Track assignment LHS names to prevent collapsing RHS to self
                        let mut lhs_names: FxIndexSet<String> = FxIndexSet::default();
                        for target in &assign_stmt.targets {
                            StatementProcessor::collect_assigned_names(target, &mut lhs_names);
                        }

                        let saved_targets = self.state.current_assignment_targets.clone();
                        self.state.current_assignment_targets = if lhs_names.is_empty() {
                            None
                        } else {
                            Some(lhs_names)
                        };

                        // Handle importlib.import_module() assignment tracking
                        if let Expr::Call(call) = &assign_stmt.value.as_ref()
                            && DynamicHandler::is_importlib_import_module_call(
                                call,
                                &self.state.import_aliases,
                            )
                        {
                            // Get assigned names to pass to the handler
                            let mut assigned_names = FxIndexSet::default();
                            for target in &assign_stmt.targets {
                                StatementProcessor::collect_assigned_names(
                                    target,
                                    &mut assigned_names,
                                );
                            }

                            DynamicHandler::handle_importlib_assignment(
                                &assigned_names,
                                call,
                                self.state.bundler,
                                &mut self.state.importlib_inlined_modules,
                            );
                        }

                        // Track local variable assignments
                        for target in &assign_stmt.targets {
                            if let Expr::Name(name) = target {
                                let var_name = name.id.to_string();
                                self.state.local_variables.insert(var_name.clone());
                            }
                        }

                        // Transform the targets
                        for target in &mut assign_stmt.targets {
                            self.transform_expr(target);
                        }

                        // Transform the RHS
                        self.transform_expr(&mut assign_stmt.value);

                        // Restore previous context
                        self.state.current_assignment_targets = saved_targets;

                        i += 1;
                        continue;
                    }
                    Stmt::FunctionDef(func_def) => {
                        log::debug!(
                            "RecursiveImportTransformer: Entering function '{}'",
                            func_def.name.as_str()
                        );

                        // Transform decorators
                        for decorator in &mut func_def.decorator_list {
                            self.transform_expr(&mut decorator.expression);
                        }

                        // Transform parameter annotations and default values
                        for param in &mut func_def.parameters.posonlyargs {
                            if let Some(annotation) = &mut param.parameter.annotation {
                                self.transform_expr(annotation);
                            }
                            if let Some(default) = &mut param.default {
                                self.transform_expr(default);
                            }
                        }
                        for param in &mut func_def.parameters.args {
                            if let Some(annotation) = &mut param.parameter.annotation {
                                self.transform_expr(annotation);
                            }
                            if let Some(default) = &mut param.default {
                                self.transform_expr(default);
                            }
                        }
                        if let Some(vararg) = &mut func_def.parameters.vararg
                            && let Some(annotation) = &mut vararg.annotation
                        {
                            self.transform_expr(annotation);
                        }
                        for param in &mut func_def.parameters.kwonlyargs {
                            if let Some(annotation) = &mut param.parameter.annotation {
                                self.transform_expr(annotation);
                            }
                            if let Some(default) = &mut param.default {
                                self.transform_expr(default);
                            }
                        }
                        if let Some(kwarg) = &mut func_def.parameters.kwarg
                            && let Some(annotation) = &mut kwarg.annotation
                        {
                            self.transform_expr(annotation);
                        }

                        // Transform return type annotation
                        if let Some(returns) = &mut func_def.returns {
                            self.transform_expr(returns);
                        }

                        // Save current local variables and create a new scope for the function
                        let saved_locals = self.state.local_variables.clone();

                        // Save the wrapper module imports - these should be scoped to each function
                        // to prevent imports from one function affecting another
                        let saved_wrapper_imports = self.state.wrapper_module_imports.clone();

                        // Track function parameters as local variables before transforming the body
                        // This prevents incorrect transformation of parameter names that shadow
                        // stdlib modules

                        // Track positional-only parameters
                        for param in &func_def.parameters.posonlyargs {
                            self.state
                                .local_variables
                                .insert(param.parameter.name.as_str().to_string());
                            log::debug!(
                                "Tracking function parameter as local (posonly): {}",
                                param.parameter.name.as_str()
                            );
                        }

                        // Track regular parameters
                        for param in &func_def.parameters.args {
                            self.state
                                .local_variables
                                .insert(param.parameter.name.as_str().to_string());
                            log::debug!(
                                "Tracking function parameter as local: {}",
                                param.parameter.name.as_str()
                            );
                        }

                        // Track *args if present
                        if let Some(vararg) = &func_def.parameters.vararg {
                            self.state
                                .local_variables
                                .insert(vararg.name.as_str().to_string());
                            log::debug!(
                                "Tracking function parameter as local (vararg): {}",
                                vararg.name.as_str()
                            );
                        }

                        // Track keyword-only parameters
                        for param in &func_def.parameters.kwonlyargs {
                            self.state
                                .local_variables
                                .insert(param.parameter.name.as_str().to_string());
                            log::debug!(
                                "Tracking function parameter as local (kwonly): {}",
                                param.parameter.name.as_str()
                            );
                        }

                        // Track **kwargs if present
                        if let Some(kwarg) = &func_def.parameters.kwarg {
                            self.state
                                .local_variables
                                .insert(kwarg.name.as_str().to_string());
                            log::debug!(
                                "Tracking function parameter as local (kwarg): {}",
                                kwarg.name.as_str()
                            );
                        }

                        // Save the current scope level and mark that we're entering a local scope
                        let saved_at_module_level = self.state.at_module_level;
                        self.state.at_module_level = false;

                        // Save current function context and compute symbol analysis once
                        let saved_function_body = self.state.current_function_body.take();
                        let saved_used_symbols = self.state.current_function_used_symbols.take();

                        // Compute used symbols once from the original body (before transformation)
                        self.state.current_function_used_symbols =
                            Some(crate::visitors::SymbolUsageVisitor::collect_used_symbols(
                                &func_def.body,
                            ));

                        // Set function body for compatibility with existing APIs
                        self.state.current_function_body = Some(func_def.body.clone());

                        // Transform the function body
                        self.transform_statements(&mut func_def.body);

                        // After all transformations, hoist and deduplicate any inserted
                        // `global` statements to the start of the function body (after a
                        // docstring if present) to ensure correct Python semantics.
                        StatementProcessor::hoist_function_globals(func_def);

                        // Restore the previous scope level
                        self.state.at_module_level = saved_at_module_level;

                        // Restore the previous function context
                        self.state.current_function_body = saved_function_body;
                        self.state.current_function_used_symbols = saved_used_symbols;

                        // Restore the wrapper module imports to prevent function-level imports from
                        // affecting other functions
                        self.state.wrapper_module_imports = saved_wrapper_imports;

                        // Restore the previous scope's local variables
                        self.state.local_variables = saved_locals;
                    }
                    Stmt::ClassDef(class_def) => {
                        // Transform decorators
                        for decorator in &mut class_def.decorator_list {
                            self.transform_expr(&mut decorator.expression);
                        }

                        // Transform base classes
                        self.transform_class_bases(class_def);

                        // Note: Class bodies in Python don't create a local scope that requires
                        // 'global' declarations for assignments. They
                        // execute in a temporary namespace but can
                        // still read from and assign to the enclosing scope without 'global'.
                        self.transform_statements(&mut class_def.body);
                    }
                    Stmt::If(if_stmt) => {
                        self.transform_expr(&mut if_stmt.test);
                        self.transform_statements(&mut if_stmt.body);

                        // Check if this is a TYPE_CHECKING block and ensure it has a body
                        if if_stmt.body.is_empty()
                            && StatementProcessor::is_type_checking_condition(&if_stmt.test)
                        {
                            log::debug!(
                                "Adding pass statement to empty TYPE_CHECKING block in import \
                                 transformer"
                            );
                            if_stmt.body.push(crate::ast_builder::statements::pass());
                        }

                        for clause in &mut if_stmt.elif_else_clauses {
                            if let Some(test_expr) = &mut clause.test {
                                self.transform_expr(test_expr);
                            }
                            self.transform_statements(&mut clause.body);

                            // Ensure non-empty body for elif/else clauses too
                            if clause.body.is_empty() {
                                log::debug!(
                                    "Adding pass statement to empty elif/else clause in import \
                                     transformer"
                                );
                                clause.body.push(crate::ast_builder::statements::pass());
                            }
                        }
                    }
                    Stmt::While(while_stmt) => {
                        self.transform_expr(&mut while_stmt.test);
                        self.transform_statements(&mut while_stmt.body);
                        self.transform_statements(&mut while_stmt.orelse);
                    }
                    Stmt::For(for_stmt) => {
                        // Track loop variable as local before transforming to prevent incorrect
                        // stdlib transformations
                        {
                            let mut loop_names = FxIndexSet::default();
                            StatementProcessor::collect_assigned_names(
                                &for_stmt.target,
                                &mut loop_names,
                            );
                            for n in loop_names {
                                self.state.local_variables.insert(n.clone());
                                log::debug!("Tracking for loop variable as local: {n}");
                            }
                        }

                        self.transform_expr(&mut for_stmt.target);
                        self.transform_expr(&mut for_stmt.iter);
                        self.transform_statements(&mut for_stmt.body);
                        self.transform_statements(&mut for_stmt.orelse);
                    }
                    Stmt::With(with_stmt) => {
                        for item in &mut with_stmt.items {
                            self.transform_expr(&mut item.context_expr);
                        }
                        self.transform_statements(&mut with_stmt.body);
                    }
                    Stmt::Try(try_stmt) => {
                        self.transform_statements(&mut try_stmt.body);

                        // Ensure try body is not empty
                        if try_stmt.body.is_empty() {
                            log::debug!(
                                "Adding pass statement to empty try body in import transformer"
                            );
                            try_stmt.body.push(crate::ast_builder::statements::pass());
                        }

                        for handler in &mut try_stmt.handlers {
                            let ExceptHandler::ExceptHandler(eh) = handler;
                            self.transform_statements(&mut eh.body);

                            // Ensure exception handler body is not empty
                            if eh.body.is_empty() {
                                log::debug!(
                                    "Adding pass statement to empty except handler in import \
                                     transformer"
                                );
                                eh.body.push(crate::ast_builder::statements::pass());
                            }
                        }
                        self.transform_statements(&mut try_stmt.orelse);
                        self.transform_statements(&mut try_stmt.finalbody);
                    }
                    Stmt::AnnAssign(ann_assign) => {
                        // Transform the annotation
                        self.transform_expr(&mut ann_assign.annotation);

                        // Transform the target
                        self.transform_expr(&mut ann_assign.target);

                        // Transform the value if present
                        if let Some(value) = &mut ann_assign.value {
                            self.transform_expr(value);
                        }
                    }
                    Stmt::AugAssign(aug_assign) => {
                        self.transform_expr(&mut aug_assign.target);
                        self.transform_expr(&mut aug_assign.value);
                    }
                    Stmt::Expr(expr_stmt) => {
                        self.transform_expr(&mut expr_stmt.value);
                    }
                    Stmt::Return(ret_stmt) => {
                        if let Some(value) = &mut ret_stmt.value {
                            self.transform_expr(value);
                        }
                    }
                    Stmt::Raise(raise_stmt) => {
                        if let Some(exc) = &mut raise_stmt.exc {
                            self.transform_expr(exc);
                        }
                        if let Some(cause) = &mut raise_stmt.cause {
                            self.transform_expr(cause);
                        }
                    }
                    Stmt::Assert(assert_stmt) => {
                        self.transform_expr(&mut assert_stmt.test);
                        if let Some(msg) = &mut assert_stmt.msg {
                            self.transform_expr(msg);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
    }

    /// Transform a class definition's base classes
    fn transform_class_bases(&mut self, class_def: &mut StmtClassDef) {
        let Some(ref mut arguments) = class_def.arguments else {
            return;
        };

        for base in &mut arguments.args {
            self.transform_expr(base);
        }
    }

    /// Track aliases for from-import statements
    fn track_from_import_aliases(&mut self, import_from: &StmtImportFrom, resolved_module: &str) {
        // Skip importlib tracking (handled separately)
        if resolved_module == "importlib" {
            return;
        }

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            self.track_single_from_import_alias(resolved_module, imported_name, local_name);
        }
    }

    /// Track a single from-import alias
    fn track_single_from_import_alias(
        &mut self,
        resolved_module: &str,
        imported_name: &str,
        local_name: &str,
    ) {
        let full_module_path = format!("{resolved_module}.{imported_name}");

        // Check if we're importing a submodule
        if let Some(module_id) = self.state.bundler.get_module_id(&full_module_path) {
            self.handle_submodule_import(module_id, local_name, &full_module_path);
        } else if InlinedHandler::is_importing_from_inlined_module(
            resolved_module,
            self.state.bundler,
        ) {
            // Importing from an inlined module - don't track as module alias
            log::debug!(
                "Not tracking symbol import as module alias: {local_name} is a symbol from \
                 {resolved_module}, not a module alias"
            );
        }
    }

    /// Handle submodule import tracking
    fn handle_submodule_import(
        &mut self,
        module_id: crate::resolver::ModuleId,
        local_name: &str,
        full_module_path: &str,
    ) {
        if !self.state.bundler.inlined_modules.contains(&module_id) {
            return;
        }

        // Check if this is a namespace-imported module
        if self
            .state
            .bundler
            .namespace_imported_modules
            .contains_key(&module_id)
        {
            log::debug!("Not tracking namespace import as alias: {local_name} (namespace module)");
        } else if !self.state.module_id.is_entry() {
            // Track as alias in non-entry modules
            log::debug!("Tracking module import alias: {local_name} -> {full_module_path}");
            self.state
                .import_aliases
                .insert(local_name.to_string(), full_module_path.to_string());
        } else {
            log::debug!(
                "Not tracking module import as alias in entry module: {local_name} -> \
                 {full_module_path} (namespace object)"
            );
        }
    }

    /// Transform a statement, potentially returning multiple statements
    fn transform_statement(&mut self, stmt: &mut Stmt) -> Vec<Stmt> {
        // Check if it's a hoisted import before matching
        let is_hoisted = import_deduplicator::is_hoisted_import(self.state.bundler, stmt);

        match stmt {
            Stmt::Import(import_stmt) => {
                log::debug!(
                    "RecursiveImportTransformer::transform_statement: Found Import statement"
                );
                if is_hoisted {
                    vec![stmt.clone()]
                } else {
                    // Check if this is a stdlib import that should be normalized
                    let mut stdlib_imports = Vec::new();
                    let mut non_stdlib_imports = Vec::new();

                    for alias in &import_stmt.names {
                        let module_name = alias.name.as_str();

                        // Normalize ALL stdlib imports, including those with aliases
                        if StdlibHandler::should_normalize_stdlib_import(
                            module_name,
                            self.state.python_version,
                        ) {
                            // Track that this stdlib module was imported
                            self.state
                                .imported_stdlib_modules
                                .insert(module_name.to_string());
                            // Also track parent modules for dotted imports (e.g., collections.abc
                            // imports collections too)
                            if let Some(dot_pos) = module_name.find('.') {
                                let parent = &module_name[..dot_pos];
                                self.state
                                    .imported_stdlib_modules
                                    .insert(parent.to_string());
                            }
                            stdlib_imports.push((
                                module_name.to_string(),
                                alias.asname.as_ref().map(|n| n.as_str().to_string()),
                            ));
                        } else {
                            non_stdlib_imports.push(alias.clone());
                        }
                    }

                    // Handle stdlib imports
                    if !stdlib_imports.is_empty() {
                        // Build rename map for expression rewriting
                        let rename_map = StdlibHandler::build_stdlib_rename_map(&stdlib_imports);

                        // Track these renames for expression rewriting
                        for (local_name, rewritten_path) in rename_map {
                            self.state.import_aliases.insert(local_name, rewritten_path);
                        }

                        // If we're in a wrapper module, create local assignments for stdlib imports
                        if self.state.is_wrapper_init {
                            let mut assignments = StdlibHandler::handle_wrapper_stdlib_imports(
                                &stdlib_imports,
                                self.state.is_wrapper_init,
                                self.state.module_id,
                                &self
                                    .state
                                    .bundler
                                    .resolver
                                    .get_module_name(self.state.module_id)
                                    .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
                                self.state.bundler,
                            );

                            // If there are non-stdlib imports, keep them and add assignments
                            if !non_stdlib_imports.is_empty() {
                                let new_import = StmtImport {
                                    names: non_stdlib_imports,
                                    ..import_stmt.clone()
                                };
                                assignments.insert(0, Stmt::Import(new_import));
                            }

                            return assignments;
                        }
                    }

                    // If all imports were stdlib, we need to handle aliased imports
                    if non_stdlib_imports.is_empty() {
                        // Create local assignments for aliased stdlib imports
                        let mut assignments = Vec::new();
                        for (module_name, alias) in &stdlib_imports {
                            if let Some(alias_name) = alias {
                                // Aliased import creates a local binding
                                let proxy_path =
                                    format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX);
                                let proxy_parts: Vec<&str> = proxy_path.split('.').collect();
                                let value_expr = crate::ast_builder::expressions::dotted_name(
                                    &proxy_parts,
                                    ExprContext::Load,
                                );
                                let target = crate::ast_builder::expressions::name(
                                    alias_name.as_str(),
                                    ExprContext::Store,
                                );
                                let assign_stmt = crate::ast_builder::statements::assign(
                                    vec![target],
                                    value_expr,
                                );
                                assignments.push(assign_stmt);

                                // Track the alias for import_module resolution
                                if module_name == "importlib" {
                                    log::debug!(
                                        "Tracking importlib alias: {alias_name} -> importlib"
                                    );
                                    self.state
                                        .import_aliases
                                        .insert(alias_name.clone(), "importlib".to_string());
                                }
                            }
                        }
                        return assignments;
                    }

                    // Otherwise, create a new import with only non-stdlib imports
                    let new_import = StmtImport {
                        names: non_stdlib_imports,
                        ..import_stmt.clone()
                    };

                    // Track import aliases before rewriting
                    for alias in &new_import.names {
                        let module_name = alias.name.as_str();
                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                        // Track if it's an aliased import of an inlined module (but not in entry
                        // module)
                        if !self.state.module_id.is_entry()
                            && alias.asname.is_some()
                            && self
                                .state
                                .bundler
                                .get_module_id(module_name)
                                .is_some_and(|id| self.state.bundler.inlined_modules.contains(&id))
                        {
                            log::debug!("Tracking import alias: {local_name} -> {module_name}");
                            self.state
                                .import_aliases
                                .insert(local_name.to_string(), module_name.to_string());
                        }
                        // Also track importlib aliases for static import resolution (in any module)
                        else if module_name == "importlib" && alias.asname.is_some() {
                            log::debug!("Tracking importlib alias: {local_name} -> importlib");
                            self.state
                                .import_aliases
                                .insert(local_name.to_string(), "importlib".to_string());
                        }
                    }

                    let result = rewrite_import_with_renames(
                        self.state.bundler,
                        new_import.clone(),
                        self.state.symbol_renames,
                        &mut self.state.populated_modules,
                    );

                    // Track any aliases created by the import to prevent incorrect stdlib
                    // transformations
                    for alias in &new_import.names {
                        if let Some(asname) = &alias.asname {
                            let local_name = asname.as_str();
                            self.state.local_variables.insert(local_name.to_string());
                            log::debug!(
                                "Tracking import alias as local variable: {} (from {})",
                                local_name,
                                alias.name.as_str()
                            );
                        }
                    }

                    log::debug!(
                        "rewrite_import_with_renames for module '{}': import {:?} -> {} statements",
                        self.state
                            .bundler
                            .resolver
                            .get_module_name(self.state.module_id)
                            .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
                        import_stmt
                            .names
                            .iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>(),
                        result.len()
                    );
                    result
                }
            }
            Stmt::ImportFrom(import_from) => {
                log::debug!(
                    "RecursiveImportTransformer::transform_statement: Found ImportFrom statement \
                     (is_hoisted: {is_hoisted})"
                );
                // Track import aliases before handling the import (even for hoisted imports)
                if let Some(module) = &import_from.module {
                    let module_str = module.as_str();
                    log::debug!(
                        "Processing ImportFrom in RecursiveImportTransformer: from {} import {:?} \
                         (is_entry_module: {})",
                        module_str,
                        import_from
                            .names
                            .iter()
                            .map(|a| format!(
                                "{}{}",
                                a.name.as_str(),
                                a.asname
                                    .as_ref()
                                    .map(|n| format!(" as {n}"))
                                    .unwrap_or_default()
                            ))
                            .collect::<Vec<_>>(),
                        self.state.module_id.is_entry()
                    );

                    // Special handling for importlib imports
                    if module_str == "importlib" {
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str();
                            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                            if imported_name == "import_module" {
                                log::debug!(
                                    "Tracking importlib.import_module alias: {local_name} -> \
                                     importlib.import_module"
                                );
                                self.state.import_aliases.insert(
                                    local_name.to_string(),
                                    "importlib.import_module".to_string(),
                                );
                            }
                        }
                    }

                    // Resolve relative imports first
                    let resolved_module = if import_from.level > 0 {
                        self.state
                            .bundler
                            .resolver
                            .get_module_path(self.state.module_id)
                            .as_deref()
                            .and_then(|path| {
                                self.state
                                    .bundler
                                    .resolver
                                    .resolve_relative_to_absolute_module_name(
                                        import_from.level,
                                        import_from
                                            .module
                                            .as_ref()
                                            .map(ruff_python_ast::Identifier::as_str),
                                        path,
                                    )
                            })
                    } else {
                        import_from
                            .module
                            .as_ref()
                            .map(std::string::ToString::to_string)
                    };

                    if let Some(resolved) = &resolved_module {
                        // Track aliases for imported symbols
                        self.track_from_import_aliases(import_from, resolved);
                    }
                }

                // Now handle the import based on whether it's hoisted
                if is_hoisted {
                    vec![stmt.clone()]
                } else {
                    self.handle_import_from(import_from)
                }
            }
            _ => vec![stmt.clone()],
        }
    }

    /// Handle `ImportFrom` statements
    fn handle_import_from(&mut self, import_from: &StmtImportFrom) -> Vec<Stmt> {
        log::debug!(
            "RecursiveImportTransformer::handle_import_from: from {:?} import {:?}",
            import_from
                .module
                .as_ref()
                .map(ruff_python_ast::Identifier::as_str),
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
        );

        // Check if this is a stdlib module that should be normalized
        if let Some(module) = &import_from.module {
            let module_str = module.as_str();
            if let Some(result) = StdlibHandler::handle_stdlib_from_import(
                import_from,
                module_str,
                self.state.python_version,
                &mut self.state.imported_stdlib_modules,
                &mut self.state.import_aliases,
            ) {
                return result;
            }
        }

        // Resolve relative imports
        let resolved_module = if import_from.level > 0 {
            self.state
                .bundler
                .resolver
                .get_module_path(self.state.module_id)
                .as_deref()
                .and_then(|path| {
                    self.state
                        .bundler
                        .resolver
                        .resolve_relative_to_absolute_module_name(
                            import_from.level,
                            import_from
                                .module
                                .as_ref()
                                .map(ruff_python_ast::Identifier::as_str),
                            path,
                        )
                })
        } else {
            import_from
                .module
                .as_ref()
                .map(std::string::ToString::to_string)
        };

        log::debug!(
            "handle_import_from: resolved_module={:?}, is_wrapper_init={}, current_module={}",
            resolved_module,
            self.state.is_wrapper_init,
            self.state
                .bundler
                .resolver
                .get_module_name(self.state.module_id)
                .unwrap_or_else(|| format!("module#{}", self.state.module_id))
        );

        // Check if entry module wrapper imports should be skipped due to deduplication
        if let Some(ref resolved) = resolved_module
            && WrapperHandler::maybe_skip_entry_wrapper_if_all_deferred(self, import_from, resolved)
        {
            return vec![];
        }

        // Check if this should be handled by the submodule handler
        if let Some(ref resolved_base) = resolved_module
            && let Some(stmts) =
                SubmoduleHandler::handle_from_import_submodules(self, import_from, resolved_base)
        {
            return stmts;
        }

        if let Some(ref resolved) = resolved_module {
            // Check if this should be handled by the inlined handler
            if let Some(stmts) =
                InlinedHandler::handle_from_import_on_resolved_inlined(self, import_from, resolved)
            {
                return stmts;
            }

            // Check if this should be handled by the wrapper handler
            if let Some(stmts) =
                WrapperHandler::handle_from_import_on_resolved_wrapper(self, import_from, resolved)
            {
                return stmts;
            }
        }

        // Otherwise, use standard transformation
        rewrite_import_from(RewriteImportFromParams {
            bundler: self.state.bundler,
            import_from: import_from.clone(),
            current_module: &self
                .state
                .bundler
                .resolver
                .get_module_name(self.state.module_id)
                .unwrap_or_else(|| format!("module#{}", self.state.module_id)),
            module_path: self
                .state
                .bundler
                .resolver
                .get_module_path(self.state.module_id)
                .as_deref(),
            symbol_renames: self.state.symbol_renames,
            inside_wrapper_init: self.state.is_wrapper_init,
            at_module_level: self.state.at_module_level,
            python_version: self.state.python_version,
            function_body: self.state.current_function_body.as_deref(),
        })
    }

    /// Transform an expression, rewriting module attribute access to direct references
    fn transform_expr(&mut self, expr: &mut Expr) {
        ExpressionRewriter::transform_expr(self, expr);
    }

    /// Create module access expression
    pub fn create_module_access_expr(&self, module_name: &str) -> Expr {
        // Check if this is a wrapper module
        if let Some(synthetic_name) = self
            .state
            .bundler
            .get_module_id(module_name)
            .and_then(|id| self.state.bundler.module_synthetic_names.get(&id))
        {
            // This is a wrapper module - we need to call its init function
            // This handles modules with invalid Python identifiers like "my-module"
            let init_func_name =
                crate::code_generator::module_registry::get_init_function_name(synthetic_name);

            // Create init function call with module as self argument
            let module_var = sanitize_module_name_for_identifier(module_name);
            expressions::call(
                expressions::name(&init_func_name, ExprContext::Load),
                vec![expressions::name(&module_var, ExprContext::Load)],
                vec![],
            )
        } else if self
            .state
            .bundler
            .get_module_id(module_name)
            .is_some_and(|id| self.state.bundler.inlined_modules.contains(&id))
        {
            // This is an inlined module - create namespace object
            let module_renames = self
                .state
                .bundler
                .get_module_id(module_name)
                .and_then(|id| self.state.symbol_renames.get(&id));
            InlinedHandler::create_namespace_call_for_inlined_module(
                module_name,
                module_renames,
                self.state.bundler,
            )
        } else {
            // This module wasn't bundled - shouldn't happen for static imports
            log::warn!("Module '{module_name}' referenced in static import but not bundled");
            expressions::none_literal()
        }
    }
}

/// Emit `parent.attr = <full_path>` assignment for dotted imports when needed (free function)
fn emit_dotted_assignment_if_needed_for(
    bundler: &Bundler,
    parent: &str,
    attr: &str,
    full_path: &str,
    result_stmts: &mut Vec<Stmt>,
) {
    let sanitized = sanitize_module_name_for_identifier(full_path);
    let has_namespace_var = bundler.created_namespaces.contains(&sanitized);
    let is_wrapper = bundler
        .get_module_id(full_path)
        .is_some_and(|id| bundler.bundled_modules.contains(&id));
    if !(has_namespace_var || is_wrapper) {
        log::debug!("Skipping redundant self-assignment: {parent}.{attr} = {full_path}");
        return;
    }

    // Avoid emitting duplicate parent.child assignments when the bundler has
    // already created the namespace chain for this module.
    // The Bundler tracks created parent->child links using a sanitized parent
    // variable for multi-level parents and the raw name for top-level parents.
    let parent_key = if parent.contains('.') {
        sanitize_module_name_for_identifier(parent)
    } else {
        parent.to_string()
    };
    if bundler
        .parent_child_assignments_made
        .contains(&(parent_key.clone(), attr.to_string()))
    {
        log::debug!(
            "Skipping duplicate dotted assignment: {parent}.{attr} (already created by bundler)"
        );
        return;
    }

    result_stmts.push(
        crate::code_generator::namespace_manager::create_attribute_assignment(
            bundler, parent, attr, full_path,
        ),
    );
}

/// Populate namespace levels for non-aliased dotted imports (free function)
fn populate_all_namespace_levels_for(
    bundler: &Bundler,
    parts: &[&str],
    populated_modules: &mut FxIndexSet<crate::resolver::ModuleId>,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    result_stmts: &mut Vec<Stmt>,
) {
    for i in 1..=parts.len() {
        let partial_module = parts[..i].join(".");
        if let Some(partial_module_id) = bundler.get_module_id(&partial_module) {
            let should_populate = bundler.bundled_modules.contains(&partial_module_id)
                && !populated_modules.contains(&partial_module_id)
                && !bundler
                    .modules_with_populated_symbols
                    .contains(&partial_module_id);
            if !should_populate {
                continue;
            }
            log::debug!(
                "Cannot track namespace assignments for '{partial_module}' in import transformer \
                 due to immutability"
            );
            let mut ctx = create_namespace_population_context(bundler);
            let new_stmts =
                crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                    &mut ctx,
                    &partial_module,
                    partial_module_id,
                    symbol_renames,
                );
            result_stmts.extend(new_stmts);
            populated_modules.insert(partial_module_id);
        }
    }
}

/// Rewrite import with renames
fn rewrite_import_with_renames(
    bundler: &Bundler,
    import_stmt: StmtImport,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    populated_modules: &mut FxIndexSet<crate::resolver::ModuleId>,
) -> Vec<Stmt> {
    // Check each import individually
    let mut result_stmts = Vec::new();
    let mut handled_all = true;

    for alias in &import_stmt.names {
        let module_name = alias.name.as_str();

        // Check if this module is classified as FirstParty but not bundled
        // This indicates a module that can't exist due to shadowing
        let import_type = bundler.resolver.classify_import(module_name);
        if import_type == crate::resolver::ImportType::FirstParty {
            // Check if it's actually bundled
            if let Some(module_id) = bundler.get_module_id(module_name) {
                if !bundler.bundled_modules.contains(&module_id) {
                    // This is a FirstParty module that failed to resolve (e.g., due to shadowing)
                    // Transform it to raise ImportError
                    log::debug!(
                        "Module '{module_name}' is FirstParty but not bundled - transforming to \
                         raise ImportError"
                    );
                    // Create a statement that raises ImportError
                    let error_msg = format!(
                        "No module named '{}'; '{}' is not a package",
                        module_name,
                        module_name.split('.').next().unwrap_or(module_name)
                    );
                    let raise_stmt = statements::raise(
                        Some(expressions::call(
                            expressions::name("ImportError", ExprContext::Load),
                            vec![expressions::string_literal(&error_msg)],
                            vec![],
                        )),
                        None,
                    );
                    result_stmts.push(raise_stmt);
                    continue;
                }
            } else {
                // No module ID means it wasn't resolved at all
                log::debug!(
                    "Module '{module_name}' is FirstParty but has no module ID - transforming to \
                     raise ImportError"
                );
                let parent = module_name.split('.').next().unwrap_or(module_name);
                let error_msg =
                    format!("No module named '{module_name}'; '{parent}' is not a package");
                let raise_stmt = statements::raise(
                    Some(expressions::call(
                        expressions::name("ImportError", ExprContext::Load),
                        vec![expressions::string_literal(&error_msg)],
                        vec![],
                    )),
                    None,
                );
                result_stmts.push(raise_stmt);
                continue;
            }
        }

        // Check if this is a dotted import (e.g., greetings.greeting)
        if module_name.contains('.') {
            // Handle dotted imports specially
            let parts: Vec<&str> = module_name.split('.').collect();

            // Check if the full module is bundled
            if let Some(module_id) = bundler.get_module_id(module_name) {
                if bundler.bundled_modules.contains(&module_id) {
                    // Check if this is a wrapper module (has a synthetic name)
                    // Note: ALL modules are in the registry, but only wrapper modules have
                    // synthetic names
                    if bundler.has_synthetic_name(module_name) {
                        log::debug!("Module '{module_name}' has synthetic name (wrapper module)");
                        // Create all parent namespaces if needed (e.g., for a.b.c.d, create a, a.b,
                        // a.b.c)
                        bundler.create_parent_namespaces(&parts, &mut result_stmts);

                        // Initialize the module at import time
                        if let Some(module_id) = bundler.get_module_id(module_name) {
                            result_stmts
                                .extend(bundler.create_module_initialization_for_import(module_id));
                        }

                        let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                        // If there's no alias, we need to handle the dotted name specially
                        if alias.asname.is_none() {
                            // Create assignments for each level of nesting
                            // For import a.b.c.d, we need:
                            // a.b = <module a.b>
                            // a.b.c = <module a.b.c>
                            // a.b.c.d = <module a.b.c.d>
                            for i in 2..=parts.len() {
                                let parent = parts[..i - 1].join(".");
                                let attr = parts[i - 1];
                                let full_path = parts[..i].join(".");
                                emit_dotted_assignment_if_needed_for(
                                    bundler,
                                    &parent,
                                    attr,
                                    &full_path,
                                    &mut result_stmts,
                                );
                            }
                        } else {
                            // For aliased imports or non-dotted imports, just assign to the target
                            // Skip self-assignments - the module is already initialized
                            if target_name.as_str() != module_name {
                                result_stmts.push(bundler.create_module_reference_assignment(
                                    target_name.as_str(),
                                    module_name,
                                ));
                            }
                        }
                    } else {
                        // Module was inlined - create a namespace object
                        log::debug!("Module '{module_name}' was inlined (not in registry)");
                        let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                        // For dotted imports, we need to create the parent namespaces
                        if alias.asname.is_none() && module_name.contains('.') {
                            // For non-aliased dotted imports like "import a.b.c"
                            // Create all parent namespace objects AND the leaf namespace
                            bundler.create_all_namespace_objects(&parts, &mut result_stmts);

                            populate_all_namespace_levels_for(
                                bundler,
                                &parts,
                                populated_modules,
                                symbol_renames,
                                &mut result_stmts,
                            );
                        } else {
                            // For simple imports or aliased imports, create namespace object with
                            // the module's exports

                            // Check if namespace already exists
                            if bundler.created_namespaces.contains(target_name.as_str()) {
                                log::debug!(
                                    "Skipping namespace creation for '{}' - already created \
                                     globally",
                                    target_name.as_str()
                                );
                            } else {
                                let namespace_stmt = bundler.create_namespace_object_for_module(
                                    target_name.as_str(),
                                    module_name,
                                );
                                result_stmts.push(namespace_stmt);
                            }

                            // Populate the namespace with symbols only if not already populated
                            if bundler.modules_with_populated_symbols.contains(&module_id) {
                                log::debug!(
                                    "Skipping namespace population for '{module_name}' - already \
                                     populated globally"
                                );
                            } else {
                                log::debug!(
                                    "Cannot track namespace assignments for '{module_name}' in \
                                     import transformer due to immutability"
                                );
                                // For now, we'll create the statements without tracking duplicates
                                let mut ctx = create_namespace_population_context(bundler);
                                let new_stmts = crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                                    &mut ctx,
                                    target_name.as_str(),
                                    module_id,
                                    symbol_renames,
                                );
                                result_stmts.extend(new_stmts);
                            }
                        }
                    }
                }
            } else {
                handled_all = false;
            }
        } else {
            // Non-dotted import - handle as before
            let module_id = if let Some(id) = bundler.get_module_id(module_name) {
                id
            } else {
                handled_all = false;
                continue;
            };

            if !bundler.bundled_modules.contains(&module_id) {
                handled_all = false;
                continue;
            }

            if bundler
                .module_info_registry
                .is_some_and(|reg| reg.contains_module(&module_id))
            {
                // Module uses wrapper approach - need to initialize it now
                let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                // First, ensure the module is initialized
                if let Some(module_id) = bundler.get_module_id(module_name) {
                    result_stmts.extend(bundler.create_module_initialization_for_import(module_id));
                }

                // Then create assignment if needed (skip self-assignments)
                if target_name.as_str() != module_name {
                    result_stmts.push(
                        bundler
                            .create_module_reference_assignment(target_name.as_str(), module_name),
                    );
                }
            } else {
                // Module was inlined - create a namespace object
                let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                // Create namespace object with the module's exports
                // Check if namespace already exists
                if bundler.created_namespaces.contains(target_name.as_str()) {
                    log::debug!(
                        "Skipping namespace creation for '{}' - already created globally",
                        target_name.as_str()
                    );
                } else {
                    let namespace_stmt = bundler
                        .create_namespace_object_for_module(target_name.as_str(), module_name);
                    result_stmts.push(namespace_stmt);
                }

                // Populate the namespace with symbols only if not already populated
                if populated_modules.contains(&module_id)
                    || bundler.modules_with_populated_symbols.contains(&module_id)
                {
                    log::debug!(
                        "Skipping namespace population for '{module_name}' - already populated"
                    );
                } else {
                    log::debug!(
                        "Cannot track namespace assignments for '{module_name}' in import \
                         transformer due to immutability"
                    );
                    // For now, we'll create the statements without tracking duplicates
                    let mut ctx = create_namespace_population_context(bundler);
                    let new_stmts = crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                        &mut ctx,
                        target_name.as_str(),
                        module_id,
                        symbol_renames,
                    );
                    result_stmts.extend(new_stmts);
                    populated_modules.insert(module_id);
                }
            }
        }
    }

    if handled_all {
        result_stmts
    } else {
        // Keep original import for non-bundled modules
        vec![Stmt::Import(import_stmt)]
    }
}

/// Create a `NamespacePopulationContext` for populating namespace symbols.
///
/// This helper function reduces code duplication when creating the context
/// for namespace population operations in import transformation.
fn create_namespace_population_context<'a>(
    bundler: &'a crate::code_generator::bundler::Bundler,
) -> crate::code_generator::namespace_manager::NamespacePopulationContext<'a> {
    crate::code_generator::namespace_manager::NamespacePopulationContext {
        inlined_modules: &bundler.inlined_modules,
        module_exports: &bundler.module_exports,
        tree_shaking_keep_symbols: &bundler.tree_shaking_keep_symbols,
        bundled_modules: &bundler.bundled_modules,
        modules_with_accessed_all: &bundler.modules_with_accessed_all,
        wrapper_modules: &bundler.wrapper_modules,
        modules_with_explicit_all: &bundler.modules_with_explicit_all,
        module_asts: &bundler.module_asts,
        global_deferred_imports: &bundler.global_deferred_imports,
        module_init_functions: &bundler.module_init_functions,
        resolver: bundler.resolver,
    }
}

/// Check if an import statement is importing bundled submodules
fn has_bundled_submodules(
    import_from: &StmtImportFrom,
    module_name: &str,
    bundler: &Bundler,
) -> bool {
    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let full_module_path = format!("{module_name}.{imported_name}");
        log::trace!("  Checking if '{full_module_path}' is in bundled_modules");
        if bundler
            .get_module_id(&full_module_path)
            .is_some_and(|id| bundler.bundled_modules.contains(&id))
        {
            log::trace!("    -> YES, it's bundled");
            return true;
        }
        log::trace!("    -> NO, not bundled");
    }
    false
}

/// Parameters for rewriting import from statements
struct RewriteImportFromParams<'a> {
    bundler: &'a Bundler<'a>,
    import_from: StmtImportFrom,
    current_module: &'a str,
    module_path: Option<&'a Path>,
    symbol_renames: &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    inside_wrapper_init: bool,
    at_module_level: bool,
    python_version: u8,
    function_body: Option<&'a [Stmt]>,
}

/// Rewrite import from statement with proper handling for bundled modules
fn rewrite_import_from(params: RewriteImportFromParams) -> Vec<Stmt> {
    let RewriteImportFromParams {
        bundler,
        import_from,
        current_module,
        module_path,
        symbol_renames,
        inside_wrapper_init,
        at_module_level,
        python_version,
        function_body,
    } = params;
    // Resolve relative imports to absolute module names
    log::debug!(
        "rewrite_import_from: Processing import {:?} in module '{}'",
        import_from
            .module
            .as_ref()
            .map(ruff_python_ast::Identifier::as_str),
        current_module
    );
    log::debug!(
        "  Importing names: {:?}",
        import_from
            .names
            .iter()
            .map(|a| (
                a.name.as_str(),
                a.asname.as_ref().map(ruff_python_ast::Identifier::as_str)
            ))
            .collect::<Vec<_>>()
    );
    log::trace!("  bundled_modules size: {}", bundler.bundled_modules.len());
    log::trace!("  inlined_modules size: {}", bundler.inlined_modules.len());
    let resolved_module_name = if import_from.level > 0 {
        module_path.and_then(|path| {
            log::debug!(
                "Resolving relative import: level={}, module={:?}, current_path={}",
                import_from.level,
                import_from
                    .module
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str),
                path.display()
            );
            let resolved = bundler.resolver.resolve_relative_to_absolute_module_name(
                import_from.level,
                import_from
                    .module
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str),
                path,
            );
            log::debug!("  Resolved to: {resolved:?}");
            resolved
        })
    } else {
        import_from
            .module
            .as_ref()
            .map(std::string::ToString::to_string)
    };

    let Some(module_name) = resolved_module_name else {
        // If we can't resolve a relative import, this is a critical error
        // Relative imports are ALWAYS first-party and must be resolvable
        assert!(
            import_from.level == 0,
            "Failed to resolve relative import 'from {} import {:?}' in module '{}'. Relative \
             imports are always first-party and must be resolvable.",
            ".".repeat(import_from.level as usize),
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            current_module
        );
        // For absolute imports that can't be resolved, return the original import
        log::warn!(
            "Could not resolve module name for import {:?}, keeping original import",
            import_from
                .module
                .as_ref()
                .map(ruff_python_ast::Identifier::as_str)
        );
        return vec![Stmt::ImportFrom(import_from)];
    };

    if !bundler
        .get_module_id(&module_name)
        .is_some_and(|id| bundler.bundled_modules.contains(&id))
    {
        log::trace!(
            "  bundled_modules contains: {:?}",
            bundler.bundled_modules.iter().collect::<Vec<_>>()
        );
        log::debug!(
            "Module '{module_name}' not found in bundled modules, checking if inlined or \
             importing submodules"
        );

        // First check if we're importing bundled submodules from a namespace package
        // This check MUST come before the inlined module check
        // e.g., from greetings import greeting where greeting is actually greetings.greeting
        if has_bundled_submodules(&import_from, &module_name, bundler) {
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
            return crate::code_generator::namespace_manager::transform_namespace_package_imports(
                bundler,
                import_from,
                &module_name,
                symbol_renames,
            );
        }

        // Check if this module is inlined
        if let Some(source_module_id) = bundler.get_module_id(&module_name)
            && bundler.inlined_modules.contains(&source_module_id)
        {
            log::debug!(
                "Module '{module_name}' is an inlined module, \
                 inside_wrapper_init={inside_wrapper_init}"
            );
            // Get the importing module's ID
            let importing_module_id = bundler.resolver.get_module_id_by_name(current_module);
            // Handle imports from inlined modules
            return handlers::inlined::InlinedHandler::handle_imports_from_inlined_module_with_context(
                bundler,
                &import_from,
                source_module_id,
                symbol_renames,
                inside_wrapper_init,
                importing_module_id,
            );
        }

        // Check if this module is in the module_registry (wrapper module)
        // A module is a wrapper if it's bundled but NOT inlined
        if bundler.get_module_id(&module_name).is_some_and(|id| {
            bundler.bundled_modules.contains(&id) && !bundler.inlined_modules.contains(&id)
        }) {
            log::debug!("Module '{module_name}' is a wrapper module in module_registry");
            // Route wrapper-module from-import rewriting through the wrapper handler.
            return handlers::wrapper::WrapperHandler::rewrite_from_import_for_wrapper_module_with_context(
                bundler,
                &import_from,
                &module_name,
                inside_wrapper_init,
                at_module_level,
                Some(current_module),
                symbol_renames,
                function_body,
            );
        }

        // Relative imports are ALWAYS first-party and should never be preserved as import
        // statements
        if import_from.level > 0 {
            // Special case: if this resolves to the entry module (ID 0), treat it as inlined
            // The entry module is always part of the bundle but might not be in bundled_modules set
            // Check if this is the entry module or entry.__main__
            let entry_module_id = if let Some(module_id) = bundler.get_module_id(&module_name) {
                if module_id.is_entry() {
                    Some(module_id)
                } else {
                    None
                }
            } else if module_name.ends_with(".__main__") {
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
            };

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
                return handlers::inlined::InlinedHandler::handle_imports_from_inlined_module_with_context(
                    bundler,
                    &import_from,
                    module_id,
                    symbol_renames,
                    inside_wrapper_init,
                    importing_module_id,
                );
            }

            // Special case: imports from __main__ modules that aren't the entry
            // These might not be discovered if the __main__.py wasn't explicitly imported
            if module_name.ends_with(".__main__") {
                log::warn!(
                    "Relative import 'from {}{}import {:?}' in module '{}' resolves to '{}' which \
                     is not bundled. This __main__ module may not have been discovered during \
                     bundling.",
                    ".".repeat(import_from.level as usize),
                    import_from
                        .module
                        .as_ref()
                        .map(|m| format!("{} ", m.as_str()))
                        .unwrap_or_default(),
                    import_from
                        .names
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>(),
                    current_module,
                    module_name
                );
                // Return the original import and let it fail at runtime if the module doesn't exist
                // This is better than panicking during bundling
                return vec![Stmt::ImportFrom(import_from)];
            }

            // Original panic for other non-entry relative imports
            panic!(
                "Relative import 'from {}{}import {:?}' in module '{}' resolves to '{}' which is \
                 not bundled or inlined. This is a bug - relative imports are always first-party \
                 and should be bundled.",
                ".".repeat(import_from.level as usize),
                import_from
                    .module
                    .as_ref()
                    .map(|m| format!("{} ", m.as_str()))
                    .unwrap_or_default(),
                import_from
                    .names
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>(),
                current_module,
                module_name
            );
        }
        // For absolute imports from non-bundled modules, keep original import
        return vec![Stmt::ImportFrom(import_from)];
    }

    log::debug!(
        "Transforming bundled import from module: {module_name}, is wrapper: {}",
        bundler
            .get_module_id(&module_name)
            .is_some_and(|id| bundler.bundled_modules.contains(&id)
                && !bundler.inlined_modules.contains(&id))
    );

    // Check if this module is in the registry (wrapper approach)
    // A module is a wrapper if it's bundled but NOT inlined
    if bundler.get_module_id(&module_name).is_some_and(|id| {
        bundler.bundled_modules.contains(&id) && !bundler.inlined_modules.contains(&id)
    }) {
        // Module uses wrapper approach - transform to sys.modules access
        // For relative imports, we need to create an absolute import
        let mut absolute_import = import_from.clone();
        if import_from.level > 0 {
            // If module_name is empty, this is a critical error
            if module_name.is_empty() {
                panic!(
                    "Relative import 'from {} import {:?}' in module '{}' resolved to empty \
                     module name. This is a bug - relative imports must resolve to a valid module.",
                    ".".repeat(import_from.level as usize),
                    import_from
                        .names
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>(),
                    current_module
                );
            } else {
                // Convert relative import to absolute
                absolute_import.level = 0;
                absolute_import.module = Some(Identifier::new(&module_name, TextRange::default()));
            }
        }
        handlers::wrapper::WrapperHandler::rewrite_from_import_for_wrapper_module_with_context(
            bundler,
            &absolute_import,
            &module_name,
            inside_wrapper_init,
            at_module_level,
            Some(current_module),
            symbol_renames,
            function_body,
        )
    } else {
        // Module was inlined - but first check if we're importing bundled submodules
        // e.g., from my_package import utils where my_package.utils is a bundled module
        if has_bundled_submodules(&import_from, &module_name, bundler) {
            log::debug!(
                "Inlined module '{module_name}' has bundled submodules, using \
                 transform_namespace_package_imports"
            );
            // Use namespace package imports for bundled submodules
            return crate::code_generator::namespace_manager::transform_namespace_package_imports(
                bundler,
                import_from,
                &module_name,
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
        let (assignments, namespace_requirements) =
            crate::code_generator::module_registry::create_assignments_for_inlined_imports(
                &import_from,
                &module_name,
                params,
            );

        // Check for unregistered namespaces - this indicates a bug in pre-detection
        let unregistered_namespaces: Vec<_> = namespace_requirements
            .iter()
            .filter(|ns_req| !bundler.namespace_registry.contains_key(&ns_req.var_name))
            .collect();

        assert!(
            unregistered_namespaces.is_empty(),
            "Unregistered namespaces detected: {:?}. This indicates a bug in \
             detect_namespace_requirements_from_imports",
            unregistered_namespaces
                .iter()
                .map(|ns| format!("{} (var: {})", ns.path, ns.var_name))
                .collect::<Vec<_>>()
        );

        // The namespaces are now pre-created by detect_namespace_requirements_from_imports
        // and the aliases are handled by create_assignments_for_inlined_imports,
        // so we just return the assignments
        assignments
    }
}

/// Transform relative import aliases into direct assignments for wrapper init functions
///
/// This handles the common pattern `from . import errors, themes` by converting it to
/// direct assignments to already-available wrapper module variables.
///
/// # Arguments
/// * `bundler` - The bundler instance for module lookups
/// * `import_from` - The import statement to process
/// * `parent_package` - The parent package name to use for building full module paths
/// * `current_module` - The current module name for module attribute assignments
/// * `result` - Vector to append generated statements to
/// * `add_module_attr` - Whether to add module attributes for non-private symbols
pub fn transform_relative_import_aliases(
    bundler: &Bundler,
    import_from: &StmtImportFrom,
    parent_package: &str,
    current_module: &str,
    result: &mut Vec<Stmt>,
    add_module_attr: bool,
) {
    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        if imported_name == "*" {
            continue;
        }

        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

        // Try to resolve the import to an actual file path
        // First, construct the expected module name for resolution
        let full_module_name = if parent_package.is_empty() {
            imported_name.to_string()
        } else {
            format!("{parent_package}.{imported_name}")
        };

        log::debug!("Attempting to resolve module '{full_module_name}' to a path");

        // Try to resolve the module to a path and then to a ModuleId
        let module_id = if let Ok(Some(module_path)) =
            bundler.resolver.resolve_module_path(&full_module_name)
        {
            log::debug!(
                "Resolved '{full_module_name}' to path: {}",
                module_path.display()
            );
            bundler.resolver.get_module_id_by_path(&module_path)
        } else {
            log::debug!(
                "Could not resolve '{full_module_name}' to a path - might be a symbol import, not \
                 a module"
            );
            None
        };

        // For relative imports in bundled code, we need to distinguish between:
        // 1. Importing a submodule (e.g., from . import errors where errors.py exists)
        // 2. Importing a symbol from parent package (e.g., from . import get_console where
        //    get_console is a function)

        // This is a critical error - the module was registered without its package prefix
        assert!(
            !parent_package.is_empty(),
            "CRITICAL: Module '{current_module}' is missing its package prefix. Relative import \
             'from . import {imported_name}' cannot be resolved. This is a bug in module \
             discovery - the module should have been registered with its full package name."
        );

        // If we couldn't find a module, this might be a symbol import from the parent package
        // In that case, we should just create a simple assignment
        let Some(module_id) = module_id else {
            log::debug!(
                "Import '{imported_name}' in module '{current_module}' is likely a symbol from \
                 parent package, not a submodule"
            );

            // When importing a symbol from the parent package, we need to check if the parent
            // is inlined and if the symbol needs to be accessed through the parent's namespace
            let parent_module_id = bundler.get_module_id(parent_package);

            // Common helper to add module attribute if exportable
            let add_module_attribute_if_needed = |result: &mut Vec<Stmt>| {
                if add_module_attr && !local_name.starts_with('_') {
                    let current_module_var = sanitize_module_name_for_identifier(current_module);
                    result.push(
                        crate::code_generator::module_registry::create_module_attr_assignment(
                            &current_module_var,
                            local_name,
                        ),
                    );
                }
            };

            // Check if parent is inlined and if we're in a wrapper context
            // In wrapper init functions, symbols from inlined parent modules need special handling
            if let Some(parent_id) = parent_module_id
                && bundler.inlined_modules.contains(&parent_id)
            {
                // The parent module is inlined, so its symbols are in the global scope
                // We need to access them through the parent's namespace object
                let parent_namespace = sanitize_module_name_for_identifier(parent_package);

                log::debug!(
                    "Parent package '{parent_package}' is inlined, accessing symbol \
                     '{imported_name}' through namespace '{parent_namespace}'"
                );

                // Create: local_name = parent_namespace.imported_name
                result.push(statements::simple_assign(
                    local_name,
                    expressions::attribute(
                        expressions::name(&parent_namespace, ExprContext::Load),
                        imported_name,
                        ExprContext::Load,
                    ),
                ));

                add_module_attribute_if_needed(result);
                continue;
            }

            // For non-inlined parent or if parent not found, create a simple assignment
            // The symbol should already be available in the bundled code
            if local_name != imported_name {
                result.push(statements::simple_assign(
                    local_name,
                    expressions::name(imported_name, ExprContext::Load),
                ));
            }

            add_module_attribute_if_needed(result);
            continue;
        };

        log::debug!("Found module ID {module_id:?} for '{full_module_name}'");
        let is_bundled = bundler.bundled_modules.contains(&module_id);
        let is_inlined = bundler.inlined_modules.contains(&module_id);

        if is_bundled || is_inlined {
            // This is a bundled or inlined module, create assignment to reference it
            let module_var = crate::code_generator::module_registry::get_module_var_identifier(
                module_id,
                bundler.resolver,
            );

            // For inlined modules, we need to create a namespace object if it doesn't exist
            if is_inlined && !bundler.created_namespaces.contains(&module_var) {
                log::debug!("Creating namespace for inlined module '{full_module_name}'");

                // Create a SimpleNamespace for the inlined module
                let namespace_stmt = statements::simple_assign(
                    &module_var,
                    expressions::call(
                        expressions::attribute(
                            expressions::name("_cribo", ExprContext::Load),
                            "types.SimpleNamespace",
                            ExprContext::Load,
                        ),
                        vec![],
                        vec![expressions::keyword(
                            Some("__name__"),
                            expressions::string_literal(&full_module_name),
                        )],
                    ),
                );
                result.push(namespace_stmt);

                // Note: We can't modify bundler.created_namespaces here as it's borrowed
                // immutably The namespace will be tracked elsewhere
            }

            log::debug!("Creating assignment: {local_name} = {module_var}");

            result.push(statements::simple_assign(
                local_name,
                expressions::name(&module_var, ExprContext::Load),
            ));

            // Add as module attribute
            if add_module_attr {
                let current_module_var = sanitize_module_name_for_identifier(current_module);
                result.push(
                    crate::code_generator::module_registry::create_module_attr_assignment(
                        &current_module_var,
                        local_name,
                    ),
                );
            }
            continue;
        }

        // If not a bundled module, still create an assignment assuming the symbol exists
        // Only create assignment if names differ to avoid redundant "x = x"
        if local_name != imported_name {
            log::debug!("Creating fallback assignment: {local_name} = {imported_name}");
            result.push(statements::simple_assign(
                local_name,
                expressions::name(imported_name, ExprContext::Load),
            ));
        }

        // Add as module attribute if exportable and not private
        if add_module_attr && !local_name.starts_with('_') {
            let current_module_var = sanitize_module_name_for_identifier(current_module);
            result.push(
                crate::code_generator::module_registry::create_module_attr_assignment(
                    &current_module_var,
                    local_name,
                ),
            );
        }
    }
}
