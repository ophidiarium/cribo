use std::path::Path;

use cow_utils::CowUtils;
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprContext, ExprName, ModModule, Stmt, StmtClassDef, StmtImport,
    StmtImportFrom,
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
pub(crate) mod handlers;
mod state;
mod statement;

use expr_rewriter::ExpressionRewriter;
use handlers::{
    inlined::InlinedHandler, statements::StatementsHandler, stdlib::StdlibHandler,
    submodule::SubmoduleHandler, wrapper::WrapperHandler,
};
// Re-export the params struct for external use
pub(crate) use state::RecursiveImportTransformerParams;
use state::TransformerState;

/// Transformer that recursively handles import statements and module references
pub(crate) struct RecursiveImportTransformer<'a> {
    state: TransformerState<'a>,
}

/// Public bridge for Bundler to delegate wrapper wildcard from-import handling
pub(crate) fn transform_wrapper_wildcard_import(
    bundler: &Bundler<'_>,
    import_from: &StmtImportFrom,
    module_name: &str,
    inside_wrapper_init: bool,
    current_module: Option<&str>,
    at_module_level: bool,
) -> Vec<Stmt> {
    WrapperHandler::handle_wildcard_import_from_multiple(
        bundler,
        import_from,
        module_name,
        inside_wrapper_init,
        current_module,
        at_module_level,
    )
}

/// Public bridge for Bundler to delegate wrapper non-wildcard from-import handling
pub(in crate::code_generator) fn transform_wrapper_symbol_imports(
    bundler: &Bundler<'_>,
    import_from: &StmtImportFrom,
    module_name: &str,
    context: &crate::code_generator::bundler::BundledImportContext<'_>,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    function_body: Option<&[Stmt]>,
) -> Vec<Stmt> {
    // Delegate via handler to centralize routing; implementation currently lives in Bundler.
    WrapperHandler::handle_symbol_imports_from_multiple(
        bundler,
        import_from,
        module_name,
        context,
        symbol_renames,
        function_body,
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
            module_id,
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
        self.state
            .current_function_used_symbols
            .as_ref()
            .is_some_and(|used_symbols| {
                let uses_alias = import_from.names.iter().any(|a| {
                    let local = a.asname.as_ref().unwrap_or(&a.name).as_str();
                    used_symbols.contains(local)
                });
                !uses_alias
            })
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
            .is_some_and(|exports| exports.contains(&attr_name.to_owned()))
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
            .is_some_and(|exports| exports.contains(&imported_name.to_owned()));
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
                        .is_some_and(|reg| reg.contains_module(id))
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
    pub(crate) fn new(params: &RecursiveImportTransformerParams<'a>) -> Self {
        Self {
            state: TransformerState::new(params),
        }
    }

    /// Get whether any types.SimpleNamespace objects were created
    pub(crate) const fn created_namespace_objects(&self) -> bool {
        self.state.created_namespace_objects
    }

    /// Get the import aliases map
    pub(crate) const fn import_aliases(&self) -> &FxIndexMap<String, String> {
        &self.state.import_aliases
    }

    /// Get mutable access to the import aliases map
    pub(crate) const fn import_aliases_mut(&mut self) -> &mut FxIndexMap<String, String> {
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
                        if !StatementsHandler::handle_assign(self, assign_stmt) {
                            i += 1;
                            continue;
                        }
                    }
                    Stmt::FunctionDef(func_def) => {
                        StatementsHandler::handle_function_def(self, func_def);
                    }
                    Stmt::ClassDef(class_def) => {
                        StatementsHandler::handle_class_def(self, class_def);
                    }
                    Stmt::If(if_stmt) => {
                        StatementsHandler::handle_if(self, if_stmt);
                    }
                    Stmt::While(while_stmt) => {
                        StatementsHandler::handle_while(self, while_stmt);
                    }
                    Stmt::For(for_stmt) => {
                        StatementsHandler::handle_for(self, for_stmt);
                    }
                    Stmt::With(with_stmt) => {
                        StatementsHandler::handle_with(self, with_stmt);
                    }
                    Stmt::Try(try_stmt) => {
                        StatementsHandler::handle_try(self, try_stmt);
                    }
                    Stmt::AnnAssign(ann_assign) => {
                        StatementsHandler::handle_ann_assign(self, ann_assign);
                    }
                    Stmt::AugAssign(aug_assign) => {
                        StatementsHandler::handle_aug_assign(self, aug_assign);
                    }
                    Stmt::Expr(expr_stmt) => {
                        StatementsHandler::handle_expr_stmt(self, expr_stmt);
                    }
                    Stmt::Return(ret_stmt) => {
                        StatementsHandler::handle_return(self, ret_stmt);
                    }
                    Stmt::Raise(raise_stmt) => {
                        StatementsHandler::handle_raise(self, raise_stmt);
                    }
                    Stmt::Assert(assert_stmt) => {
                        StatementsHandler::handle_assert(self, assert_stmt);
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
                .insert(local_name.to_owned(), full_module_path.to_owned());
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
                                .insert(module_name.to_owned());
                            // Also track parent modules for dotted imports (e.g., collections.abc
                            // imports collections too)
                            if let Some(dot_pos) = module_name.find('.') {
                                let parent = &module_name[..dot_pos];
                                self.state.imported_stdlib_modules.insert(parent.to_owned());
                            }
                            stdlib_imports.push((
                                module_name.to_owned(),
                                alias.asname.as_ref().map(|n| n.as_str().to_owned()),
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

                    // If all imports were stdlib, we need to handle them appropriately
                    if non_stdlib_imports.is_empty() {
                        // Create local assignments for simple stdlib imports (both aliased and
                        // non-aliased) For dotted imports like
                        // "xml.etree.ElementTree", don't create assignments
                        // - let the expression rewriter handle them
                        let mut assignments = Vec::new();
                        for (module_name, alias) in &stdlib_imports {
                            // Only create assignments for simple (non-dotted) module names
                            // or when there's an explicit alias
                            if alias.is_some() || !module_name.contains('.') {
                                let local_name = alias.as_ref().unwrap_or(module_name);

                                // Create assignment: local_name = _cribo.module_name
                                let proxy_path =
                                    format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX);
                                let proxy_parts: Vec<&str> = proxy_path.split('.').collect();
                                let value_expr =
                                    expressions::dotted_name(&proxy_parts, ExprContext::Load);
                                let target =
                                    expressions::name(local_name.as_str(), ExprContext::Store);
                                let assign_stmt = statements::assign(vec![target], value_expr);
                                assignments.push(assign_stmt);

                                // Track the alias for import_module resolution
                                if module_name == "importlib" {
                                    log::debug!(
                                        "Tracking importlib alias: {local_name} -> importlib"
                                    );
                                    self.state
                                        .import_aliases
                                        .insert(local_name.clone(), "importlib".to_owned());
                                }
                            }
                            // For dotted imports without aliases, don't create assignments
                            // Let the expression rewriter handle them via _cribo proxy
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
                                .insert(local_name.to_owned(), module_name.to_owned());
                        }
                        // Also track importlib aliases for static import resolution (in any module)
                        else if module_name == "importlib" && alias.asname.is_some() {
                            log::debug!("Tracking importlib alias: {local_name} -> importlib");
                            self.state
                                .import_aliases
                                .insert(local_name.to_owned(), "importlib".to_owned());
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
                            self.state.local_variables.insert(local_name.to_owned());
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
                                    local_name.to_owned(),
                                    "importlib.import_module".to_owned(),
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
                        import_from.module.as_ref().map(ToString::to_string)
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
            import_from.module.as_ref().map(ToString::to_string)
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
            current_function_used_symbols: self.state.current_function_used_symbols.as_ref(),
        })
    }

    /// Transform an expression, rewriting module attribute access to direct references
    fn transform_expr(&mut self, expr: &mut Expr) {
        ExpressionRewriter::transform_expr(self, expr);
    }

    /// Create module access expression
    pub(crate) fn create_module_access_expr(&self, module_name: &str) -> Expr {
        // Check if this is a wrapper module
        self.state
            .bundler
            .get_module_id(module_name)
            .and_then(|id| self.state.bundler.module_synthetic_names.get(&id))
            .map_or_else(
                || {
                    if self
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
                        log::warn!(
                            "Module '{module_name}' referenced in static import but not bundled"
                        );
                        expressions::none_literal()
                    }
                },
                |synthetic_name| {
                    // This is a wrapper module - we need to call its init function
                    // This handles modules with invalid Python identifiers like "my-module"
                    let init_func_name = self
                        .state
                        .bundler
                        .get_module_id(module_name)
                        .and_then(|id| self.state.bundler.module_init_functions.get(&id).cloned())
                        .unwrap_or_else(|| {
                            crate::code_generator::module_registry::get_init_function_name(
                                synthetic_name,
                            )
                        });

                    // Create init function call with module as self argument
                    let module_var = sanitize_module_name_for_identifier(module_name);
                    expressions::call(
                        expressions::name(&init_func_name, ExprContext::Load),
                        vec![expressions::name(&module_var, ExprContext::Load)],
                        vec![],
                    )
                },
            )
    }
}

/// Emit `parent.attr = <full_path>` assignment for dotted imports when needed (free function)
fn emit_dotted_assignment_if_needed_for(
    bundler: &Bundler<'_>,
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
        parent.to_owned()
    };
    if bundler
        .parent_child_assignments_made
        .contains(&(parent_key, attr.to_owned()))
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
    bundler: &Bundler<'_>,
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
            let ctx = create_namespace_population_context(bundler);
            let new_stmts =
                crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                    &ctx,
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
    bundler: &Bundler<'_>,
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
                                let ctx = create_namespace_population_context(bundler);
                                let new_stmts = crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                                    &ctx,
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
            let Some(module_id) = bundler.get_module_id(module_name) else {
                handled_all = false;
                continue;
            };

            if !bundler.bundled_modules.contains(&module_id) {
                handled_all = false;
                continue;
            }

            let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

            if bundler
                .module_info_registry
                .is_some_and(|reg| reg.contains_module(module_id))
            {
                // Module uses wrapper approach - need to initialize it now

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
                    let ctx = create_namespace_population_context(bundler);
                    let new_stmts = crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                        &ctx,
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
const fn create_namespace_population_context<'a>(
    bundler: &'a Bundler<'_>,
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
        module_init_functions: &bundler.module_init_functions,
        resolver: bundler.resolver,
    }
}

/// Check if an import statement is importing bundled submodules
fn has_bundled_submodules(
    import_from: &StmtImportFrom,
    module_name: &str,
    bundler: &Bundler<'_>,
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
    current_function_used_symbols: Option<&'a FxIndexSet<String>>,
}

/// Rewrite import from statement with proper handling for bundled modules
fn rewrite_import_from(params: RewriteImportFromParams<'_>) -> Vec<Stmt> {
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
        current_function_used_symbols,
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
        import_from.module.as_ref().map(ToString::to_string)
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
        return handlers::fallback::keep_original_from_import(&import_from);
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

        if let Some(stmts) = InlinedHandler::transform_if_has_bundled_submodules(
            bundler,
            &import_from,
            &module_name,
            symbol_renames,
        ) {
            return stmts;
        }

        if let Some(stmts) = InlinedHandler::maybe_handle_inlined_absolute(
            bundler,
            &import_from,
            &module_name,
            symbol_renames,
            inside_wrapper_init,
            current_module,
        ) {
            return stmts;
        }

        let context = handlers::wrapper::WrapperContext {
            bundler,
            symbol_renames,
            is_wrapper_init: inside_wrapper_init,
            at_module_level,
            current_module_name: current_module.to_owned(),
            function_body,
            current_function_used_symbols,
        };
        if let Some(stmts) =
            WrapperHandler::maybe_handle_wrapper_absolute(&context, &import_from, &module_name)
        {
            return stmts;
        }

        // Relative imports are ALWAYS first-party and should never be preserved as import
        // statements
        if import_from.level > 0 {
            // Special case: if this resolves to the entry module, treat it as inlined
            // The entry module is always part of the bundle but might not be in bundled_modules set
            if let Some(stmts) = InlinedHandler::handle_entry_relative_as_inlined(
                bundler,
                &import_from,
                &module_name,
                symbol_renames,
                inside_wrapper_init,
                current_module,
            ) {
                return stmts;
            }

            return handlers::relative::handle_unbundled_relative_import(
                bundler,
                &import_from,
                &module_name,
                current_module,
            );
        }
        // For absolute imports from non-bundled modules, keep original import
        return handlers::fallback::keep_original_from_import(&import_from);
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
        let context = handlers::wrapper::WrapperContext {
            bundler,
            symbol_renames,
            is_wrapper_init: inside_wrapper_init,
            at_module_level,
            current_module_name: current_module.to_owned(),
            function_body,
            current_function_used_symbols,
        };
        WrapperHandler::handle_wrapper_from_import_absolute_context(
            &context,
            &import_from,
            &module_name,
        )
    } else {
        InlinedHandler::handle_inlined_from_import_absolute_context(
            bundler,
            &import_from,
            &module_name,
            symbol_renames,
            inside_wrapper_init,
            python_version,
        )
    }
}
