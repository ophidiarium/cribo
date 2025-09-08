use std::path::Path;

use cow_utils::CowUtils;
use ruff_python_ast::{
    AtomicNodeIndex, ExceptHandler, Expr, ExprCall, ExprContext, ExprFString, ExprName, FString,
    FStringValue, Identifier, InterpolatedElement, InterpolatedStringElement,
    InterpolatedStringElements, ModModule, Stmt, StmtClassDef, StmtFunctionDef, StmtGlobal,
    StmtImport, StmtImportFrom,
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

/// Collect assigned variable names from an assignment target expression.
/// Supports simple names and destructuring via tuples/lists.
fn collect_assigned_names(target: &Expr, out: &mut FxIndexSet<String>) {
    match target {
        Expr::Name(name) => {
            out.insert(name.id.as_str().to_string());
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                collect_assigned_names(elt, out);
            }
        }
        Expr::List(l) => {
            for elt in &l.elts {
                collect_assigned_names(elt, out);
            }
        }
        _ => {}
    }
}

/// Parameters for creating a `RecursiveImportTransformer`
#[derive(Debug)]
pub struct RecursiveImportTransformerParams<'a> {
    pub bundler: &'a Bundler<'a>,
    pub module_id: crate::resolver::ModuleId,
    pub symbol_renames: &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    pub is_wrapper_init: bool,
    pub global_deferred_imports:
        Option<&'a FxIndexMap<(crate::resolver::ModuleId, String), crate::resolver::ModuleId>>,
    pub python_version: u8,
}

/// Transformer that recursively handles import statements and module references
pub struct RecursiveImportTransformer<'a> {
    bundler: &'a Bundler<'a>,
    module_id: crate::resolver::ModuleId,
    symbol_renames: &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    /// Maps import aliases to their actual module names
    /// e.g., "`helper_utils`" -> "utils.helpers"
    pub(crate) import_aliases: FxIndexMap<String, String>,
    /// Flag indicating if we're inside a wrapper module's init function
    is_wrapper_init: bool,
    /// Reference to global deferred imports registry
    global_deferred_imports:
        Option<&'a FxIndexMap<(crate::resolver::ModuleId, String), crate::resolver::ModuleId>>,
    /// Track local variable assignments to avoid treating them as module aliases
    local_variables: FxIndexSet<String>,
    /// Track if any `importlib.import_module` calls were transformed
    pub(crate) importlib_transformed: bool,
    /// Track variables that were assigned from `importlib.import_module()` of inlined modules
    /// Maps variable name to the inlined module name
    importlib_inlined_modules: FxIndexMap<String, String>,
    /// Track if we created any types.SimpleNamespace calls
    pub(crate) created_namespace_objects: bool,
    /// Track imports from wrapper modules that need to be rewritten
    /// Maps local name to (`wrapper_module`, `original_name`)
    wrapper_module_imports: FxIndexMap<String, (String, String)>,
    /// Track which modules have already been populated with symbols in this transformation session
    /// This prevents duplicate namespace assignments when multiple imports reference the same
    /// module
    populated_modules: FxIndexSet<crate::resolver::ModuleId>,
    /// Track which stdlib modules were actually imported in this module
    /// This prevents transforming references to stdlib modules that weren't imported
    imported_stdlib_modules: FxIndexSet<String>,
    /// Python version for compatibility checks
    python_version: u8,
    /// Track whether we're at module level (false when inside any local scope like function,
    /// class, etc.)
    at_module_level: bool,
    /// Track names on the LHS of the current assignment while transforming its RHS.
    current_assignment_targets: Option<FxIndexSet<String>>,
}

impl<'a> RecursiveImportTransformer<'a> {
    /// Check if a condition is a `TYPE_CHECKING` check
    fn is_type_checking_condition(expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
            Expr::Attribute(attr) => {
                attr.attr.as_str() == "TYPE_CHECKING"
                    && match &*attr.value {
                        // Check for both typing.TYPE_CHECKING and _cribo.typing.TYPE_CHECKING
                        Expr::Name(name) => name.id.as_str() == "typing",
                        Expr::Attribute(inner_attr) => {
                            // Handle _cribo.typing.TYPE_CHECKING
                            inner_attr.attr.as_str() == "typing"
                                && matches!(&*inner_attr.value, Expr::Name(name) if name.id.as_str() == crate::ast_builder::CRIBO_PREFIX)
                        }
                        _ => false,
                    }
            }
            _ => false,
        }
    }

    /// Get filtered exports for a full module path, if available
    fn get_filtered_exports_for_path(
        &self,
        full_module_path: &str,
    ) -> Option<(crate::resolver::ModuleId, Vec<String>)> {
        let module_id = self.bundler.get_module_id(full_module_path)?;
        let exports = self
            .bundler
            .module_exports
            .get(&module_id)
            .cloned()
            .flatten()?;
        let filtered: Vec<String> = SymbolAnalyzer::filter_exports_by_tree_shaking(
            &exports,
            &module_id,
            self.bundler.tree_shaking_keep_symbols.as_ref(),
            false,
            self.bundler.resolver,
        )
        .into_iter()
        .cloned()
        .collect();
        Some((module_id, filtered))
    }

    /// Should emit __all__ for a local namespace binding
    fn should_emit_all_for_local(
        &self,
        module_id: crate::resolver::ModuleId,
        local_name: &str,
        filtered_exports: &[String],
    ) -> bool {
        !filtered_exports.is_empty()
            && self.bundler.modules_with_explicit_all.contains(&module_id)
            && self
                .bundler
                .modules_with_accessed_all
                .iter()
                .any(|(module, alias)| module == &self.module_id && alias == local_name)
    }

    /// Mark namespace as populated for a module path if needed (non-bundled, not yet marked)
    fn mark_namespace_populated_if_needed(&mut self, full_module_path: &str) {
        let full_module_id = self.bundler.get_module_id(full_module_path);
        let namespace_already_populated =
            full_module_id.is_some_and(|id| self.populated_modules.contains(&id));
        let is_bundled_module =
            full_module_id.is_some_and(|id| self.bundler.bundled_modules.contains(&id));
        if !is_bundled_module
            && !namespace_already_populated
            && let Some(id) = full_module_id
        {
            self.populated_modules.insert(id);
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
                    .bundler
                    .get_module_id(full_module_path)
                    .and_then(|id| self.symbol_renames.get(&id))
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
        self.bundler
            .get_module_id(module_name)
            .is_some_and(|id| self.bundler.namespace_imported_modules.contains_key(&id))
    }

    /// Log information about wrapper wildcard exports (keeps previous behavior without generating
    /// code)
    fn log_wrapper_wildcard_info(&self, resolved: &str) {
        log::debug!("  Handling wildcard import from wrapper module '{resolved}'");
        if let Some(exports) = self
            .bundler
            .get_module_id(resolved)
            .and_then(|id| self.bundler.module_exports.get(&id))
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
            .bundler
            .get_module_id(&potential_submodule)
            .is_some_and(|id| self.bundler.bundled_modules.contains(&id))
            && !self
                .bundler
                .get_module_id(&potential_submodule)
                .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
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
        if let Some(module_id) = self.bundler.get_module_id(actual_module)
            && let Some(module_renames) = self.symbol_renames.get(&module_id)
        {
            if let Some(renamed) = module_renames.get(attr_name) {
                let renamed_str = renamed.clone();
                log::debug!("Rewrote {base}.{attr_name} to {renamed_str} (renamed)");
                return Some(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: renamed_str.into(),
                    ctx,
                    range,
                }));
            }
            // Avoid collapsing to bare name if it would create self-referential assignment
            if let Some(lhs) = &self.current_assignment_targets
                && lhs.contains(attr_name)
            {
                log::debug!(
                    "Skipping collapse of {base}.{attr_name} to avoid self-referential assignment"
                );
                return None;
            }
            log::debug!("Rewrote {base}.{attr_name} to {attr_name} (not renamed)");
            return Some(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: attr_name.into(),
                ctx,
                range,
            }));
        }

        // Fallback: if module exports include the name, use it directly
        if self
            .bundler
            .get_module_id(actual_module)
            .and_then(|id| self.bundler.module_exports.get(&id))
            .and_then(|opt| opt.as_ref())
            .is_some_and(|exports| exports.contains(&attr_name.to_string()))
        {
            if let Some(lhs) = &self.current_assignment_targets
                && lhs.contains(attr_name)
            {
                log::debug!(
                    "Skipping collapse of {base}.{attr_name} (exported) to avoid self-reference"
                );
                return None;
            }
            log::debug!("Rewrote {base}.{attr_name} to {attr_name} (exported by module)");
            return Some(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: attr_name.into(),
                ctx,
                range,
            }));
        }

        None
    }

    /// Create `local = namespace_var` if names differ
    fn alias_local_to_namespace_if_needed(
        &mut self,
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

    /// Handle parent.child alias when importing from the same parent module, with early exits
    fn maybe_log_parent_child_assignment(
        &self,
        import_base: Option<&str>,
        imported_name: &str,
        local_name: &str,
    ) {
        if import_base != Some(self.get_module_name().as_str()) {
            return;
        }

        // Check if this submodule is in the parent's __all__ exports
        let parent_exports = self
            .bundler
            .module_exports
            .get(&self.module_id)
            .and_then(|opt| opt.as_ref())
            .is_some_and(|exports| exports.contains(&imported_name.to_string()));
        if !parent_exports {
            return;
        }

        let full_submodule_path = format!("{}.{}", self.get_module_name(), imported_name);
        let is_inlined_submodule = self
            .bundler
            .get_module_id(&full_submodule_path)
            .is_some_and(|id| self.bundler.inlined_modules.contains(&id));
        let uses_init_function = self
            .bundler
            .get_module_id(&full_submodule_path)
            .and_then(|id| self.bundler.module_init_functions.get(&id))
            .is_some();

        log::debug!(
            "  Checking submodule status for {full_submodule_path}: \
             is_inlined={is_inlined_submodule}, uses_init={uses_init_function}"
        );

        if is_inlined_submodule || uses_init_function {
            log::debug!(
                "  Skipping parent module assignment for {}.{} - already handled by init function",
                self.get_module_name(),
                local_name
            );
            return;
        }

        // Double-check if this is actually a module
        let is_actually_a_module = self
            .bundler
            .get_module_id(&full_submodule_path)
            .is_some_and(|id| {
                self.bundler.bundled_modules.contains(&id)
                    || self
                        .bundler
                        .module_info_registry
                        .as_ref()
                        .is_some_and(|reg| reg.contains_module(&id))
                    || self.bundler.inlined_modules.contains(&id)
            });
        if is_actually_a_module {
            log::debug!(
                "Skipping assignment for {}.{} - it's a module, not a symbol",
                self.get_module_name(),
                local_name
            );
            return;
        }

        // At this point, we would create parent.local = local if needed.
        // Original code only logged due to deferred imports removal.
        log::debug!(
            "Creating parent module assignment: {}.{} = {} (symbol exported from parent)",
            self.get_module_name(),
            local_name,
            local_name
        );
    }

    /// For importlib-imported module variables, rewrite `base.attr` to the inlined symbol
    fn rewrite_attr_for_importlib_var(
        &self,
        base: &str,
        attr_name: &str,
        module_name: &str,
        attr_ctx: ExprContext,
        attr_range: TextRange,
    ) -> Expr {
        if let Some(module_id) = self.bundler.get_module_id(module_name)
            && let Some(module_renames) = self.symbol_renames.get(&module_id)
            && let Some(renamed) = module_renames.get(attr_name)
        {
            let renamed_str = renamed.clone();
            log::debug!(
                "Rewrote {base}.{attr_name} to {renamed_str} (renamed symbol from importlib \
                 inlined module)"
            );
            return Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: renamed_str.into(),
                ctx: attr_ctx,
                range: attr_range,
            });
        }
        // no rename: fallthrough below
        log::debug!(
            "Rewrote {base}.{attr_name} to {attr_name} (symbol from importlib inlined module)"
        );
        Expr::Name(ExprName {
            node_index: AtomicNodeIndex::dummy(),
            id: attr_name.into(),
            ctx: attr_ctx,
            range: attr_range,
        })
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
            .bundler
            .get_module_id(&potential_module)
            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
            && attr_path.len() == 2
        {
            let final_attr = &attr_path[1];
            if let Some(module_id) = self.bundler.get_module_id(&potential_module)
                && let Some(module_renames) = self.symbol_renames.get(&module_id)
                && let Some(renamed) = module_renames.get(final_attr)
            {
                log::debug!("Rewrote {base}.{}.{final_attr} to {renamed}", attr_path[0]);
                return Some(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
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
                node_index: AtomicNodeIndex::dummy(),
                id: direct_name.into(),
                ctx: attr_ctx,
                range: attr_range,
            }));
        }
        None
    }

    /// Create a new transformer from parameters
    pub fn new(params: &RecursiveImportTransformerParams<'a>) -> Self {
        Self {
            bundler: params.bundler,
            module_id: params.module_id,
            symbol_renames: params.symbol_renames,
            import_aliases: FxIndexMap::default(),
            is_wrapper_init: params.is_wrapper_init,
            global_deferred_imports: params.global_deferred_imports,
            local_variables: FxIndexSet::default(),
            importlib_transformed: false,
            importlib_inlined_modules: FxIndexMap::default(),
            created_namespace_objects: false,
            wrapper_module_imports: FxIndexMap::default(),
            populated_modules: FxIndexSet::default(),
            imported_stdlib_modules: FxIndexSet::default(),
            python_version: params.python_version,
            at_module_level: true,
            current_assignment_targets: None,
        }
    }

    /// Get whether any types.SimpleNamespace objects were created
    pub fn created_namespace_objects(&self) -> bool {
        self.created_namespace_objects
    }

    /// Get the module name from the resolver
    fn get_module_name(&self) -> String {
        self.bundler
            .resolver
            .get_module_name(self.module_id)
            .unwrap_or_else(|| format!("module#{}", self.module_id))
    }

    /// Get the module path from the resolver
    fn get_module_path(&self) -> Option<std::path::PathBuf> {
        self.bundler.resolver.get_module_path(self.module_id)
    }

    /// Check if this is the entry module
    fn is_entry_module(&self) -> bool {
        self.module_id.is_entry()
    }

    /// Check if this is an `importlib.import_module()` call
    fn is_importlib_import_module_call(&self, call: &ExprCall) -> bool {
        match &call.func.as_ref() {
            // Direct call: importlib.import_module()
            Expr::Attribute(attr) if attr.attr.as_str() == "import_module" => {
                match &attr.value.as_ref() {
                    Expr::Name(name) => {
                        let name_str = name.id.as_str();
                        // Check if it's 'importlib' directly or an alias that maps to 'importlib'
                        name_str == "importlib"
                            || self.import_aliases.get(name_str) == Some(&"importlib".to_string())
                    }
                    _ => false,
                }
            }
            // Function call: im() where im is import_module
            Expr::Name(name) => {
                // Check if this name is an alias for importlib.import_module
                self.import_aliases
                    .get(name.id.as_str())
                    .is_some_and(|module| module == "importlib.import_module")
            }
            _ => false,
        }
    }

    /// Transform importlib.import_module("module-name") to direct module reference
    fn transform_importlib_import_module(&mut self, call: &ExprCall) -> Option<Expr> {
        // Get the first argument which should be the module name
        if let Some(arg) = call.arguments.args.first()
            && let Expr::StringLiteral(lit) = arg
        {
            let module_name = lit.value.to_str();

            // Handle relative imports with package context
            let resolved_name = if module_name.starts_with('.') && call.arguments.args.len() >= 2 {
                // Get the package context from the second argument
                if let Expr::StringLiteral(package_lit) = &call.arguments.args[1] {
                    let package = package_lit.value.to_str();

                    // Resolve package to path, then use resolver
                    if let Ok(Some(package_path)) =
                        self.bundler.resolver.resolve_module_path(package)
                    {
                        let level = module_name.chars().take_while(|&c| c == '.').count() as u32;
                        let name_part = module_name.trim_start_matches('.');

                        self.bundler
                            .resolver
                            .resolve_relative_to_absolute_module_name(
                                level,
                                if name_part.is_empty() {
                                    None
                                } else {
                                    Some(name_part)
                                },
                                &package_path,
                            )
                            .unwrap_or_else(|| module_name.to_string())
                    } else {
                        // Use resolver's method for package name resolution when path not found
                        let level = module_name.chars().take_while(|&c| c == '.').count() as u32;
                        let name_part = module_name.trim_start_matches('.');

                        self.bundler
                            .resolver
                            .resolve_relative_import_from_package_name(
                                level,
                                if name_part.is_empty() {
                                    None
                                } else {
                                    Some(name_part)
                                },
                                package,
                            )
                    }
                } else {
                    module_name.to_string()
                }
            } else {
                module_name.to_string()
            };

            // Check if this module was bundled
            if self
                .bundler
                .get_module_id(&resolved_name)
                .is_some_and(|id| self.bundler.bundled_modules.contains(&id))
            {
                log::debug!(
                    "Transforming importlib.import_module('{module_name}') to module access \
                     '{resolved_name}'"
                );

                self.importlib_transformed = true;

                // Check if this creates a namespace object
                if self
                    .bundler
                    .get_module_id(&resolved_name)
                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                {
                    self.created_namespace_objects = true;
                }

                // Use common logic for module access
                return Some(self.create_module_access_expr(&resolved_name));
            }
        }
        None
    }

    /// Check if this is a stdlib import that should be normalized
    fn should_normalize_stdlib_import(&self, module_name: &str) -> bool {
        // Recognize full stdlib module paths and submodules for the current Python version
        crate::resolver::is_stdlib_module(module_name, self.python_version)
    }

    /// Build a mapping of stdlib imports to their rewritten paths
    /// This mapping is used during expression rewriting
    fn build_stdlib_rename_map(
        &self,
        imports: &[(String, Option<String>)],
    ) -> FxIndexMap<String, String> {
        let mut rename_map = FxIndexMap::default();

        for (module_name, alias) in imports {
            let local_name = alias.as_ref().unwrap_or(module_name);
            let rewritten_path = Bundler::get_rewritten_stdlib_path(module_name);
            rename_map.insert(local_name.clone(), rewritten_path);
        }

        rename_map
    }

    /// Transform a module recursively, handling all imports at any depth
    pub(crate) fn transform_module(&mut self, module: &mut ModModule) {
        log::debug!(
            "RecursiveImportTransformer::transform_module for '{}'",
            self.get_module_name()
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
                import_deduplicator::is_hoisted_import(self.bundler, &stmts[i])
            } else {
                false
            };

            if is_import {
                log::debug!(
                    "transform_statements: Found import in module '{}', is_hoisted={}",
                    self.get_module_name(),
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
                    self.get_module_name(),
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
                            collect_assigned_names(target, &mut lhs_names);
                        }

                        let saved_targets = self.current_assignment_targets.clone();
                        self.current_assignment_targets = if lhs_names.is_empty() {
                            None
                        } else {
                            Some(lhs_names)
                        };

                        // First check if this is an assignment from importlib.import_module()
                        let mut importlib_module = None;
                        if let Expr::Call(call) = &assign_stmt.value.as_ref()
                            && self.is_importlib_import_module_call(call)
                        {
                            // Get the module name from the call
                            if let Some(arg) = call.arguments.args.first()
                                && let Expr::StringLiteral(lit) = arg
                            {
                                let module_name = lit.value.to_str();
                                // Only track if it's an inlined module (not a wrapper module)
                                if self
                                    .bundler
                                    .get_module_id(module_name)
                                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                                {
                                    importlib_module = Some(module_name.to_string());
                                }
                            }
                        }

                        // Track local variable assignments and importlib modules
                        for target in &assign_stmt.targets {
                            if let Expr::Name(name) = target {
                                let var_name = name.id.to_string();
                                self.local_variables.insert(var_name.clone());

                                // If this was an importlib.import_module assignment, add to
                                // tracking
                                if let Some(module_name) = &importlib_module {
                                    self.importlib_inlined_modules
                                        .insert(var_name.clone(), module_name.clone());
                                    log::debug!(
                                        "Tracking importlib module assignment: {var_name} = \
                                         importlib.import_module('{module_name}')"
                                    );
                                }
                            }
                        }

                        // Transform the targets
                        for target in &mut assign_stmt.targets {
                            self.transform_expr(target);
                        }

                        // Transform the RHS
                        self.transform_expr(&mut assign_stmt.value);

                        // Restore previous context
                        self.current_assignment_targets = saved_targets;

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
                        let saved_locals = self.local_variables.clone();

                        // Save the wrapper module imports - these should be scoped to each function
                        // to prevent imports from one function affecting another
                        let saved_wrapper_imports = self.wrapper_module_imports.clone();

                        // Track function parameters as local variables before transforming the body
                        // This prevents incorrect transformation of parameter names that shadow
                        // stdlib modules
                        for param in &func_def.parameters.args {
                            self.local_variables
                                .insert(param.parameter.name.as_str().to_string());
                            log::debug!(
                                "Tracking function parameter as local: {}",
                                param.parameter.name.as_str()
                            );
                        }

                        // Save the current scope level and mark that we're entering a local scope
                        let saved_at_module_level = self.at_module_level;
                        self.at_module_level = false;

                        // Transform the function body
                        self.transform_statements(&mut func_def.body);

                        // After all transformations, hoist and deduplicate any inserted
                        // `global` statements to the start of the function body (after a
                        // docstring if present) to ensure correct Python semantics.
                        Self::hoist_function_globals(func_def);

                        // Restore the previous scope level
                        self.at_module_level = saved_at_module_level;

                        // Restore the wrapper module imports to prevent function-level imports from
                        // affecting other functions
                        self.wrapper_module_imports = saved_wrapper_imports;

                        // Restore the previous scope's local variables
                        self.local_variables = saved_locals;
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
                            && Self::is_type_checking_condition(&if_stmt.test)
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
                        if let Expr::Name(name) = for_stmt.target.as_ref() {
                            self.local_variables.insert(name.id.as_str().to_string());
                            log::debug!(
                                "Tracking for loop variable as local: {}",
                                name.id.as_str()
                            );
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

    /// Move all `global` statements in a function to the start of the function body
    /// (after a leading docstring, if present) and deduplicate their names.
    fn hoist_function_globals(func_def: &mut StmtFunctionDef) {
        use ruff_python_ast::helpers::is_docstring_stmt;
        use ruff_text_size::TextRange;

        use crate::types::FxIndexSet;

        let mut names: FxIndexSet<String> = FxIndexSet::default();
        let mut has_global = false;

        for stmt in &func_def.body {
            if let Stmt::Global(g) = stmt {
                has_global = true;
                for ident in &g.names {
                    names.insert(ident.as_str().to_string());
                }
            }
        }

        if !has_global {
            return;
        }

        log::debug!(
            "Hoisting {} global name(s) to function start (import transformer)",
            names.len()
        );

        // Remove existing global statements
        let mut new_body: Vec<Stmt> = Vec::with_capacity(func_def.body.len());
        for stmt in func_def.body.drain(..) {
            if !matches!(stmt, Stmt::Global(_)) {
                new_body.push(stmt);
            }
        }

        // Insert after docstring if present
        let insert_at = usize::from(new_body.first().is_some_and(is_docstring_stmt));

        // Build combined global
        let global_stmt = Stmt::Global(StmtGlobal {
            names: names
                .into_iter()
                .map(|s| Identifier::new(s, TextRange::default()))
                .collect(),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        });

        new_body.insert(insert_at, global_stmt);
        func_def.body = new_body;
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
        if let Some(module_id) = self.bundler.get_module_id(&full_module_path) {
            self.handle_submodule_import(module_id, local_name, &full_module_path);
        } else if self.is_importing_from_inlined_module(resolved_module) {
            // Importing from an inlined module - don't track as module alias
            log::debug!(
                "Not tracking symbol import as module alias: {local_name} is a symbol from \
                 {resolved_module}, not a module alias"
            );
        }
    }

    /// Check if importing from an inlined module
    fn is_importing_from_inlined_module(&self, module_name: &str) -> bool {
        self.bundler
            .get_module_id(module_name)
            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
    }

    /// Handle submodule import tracking
    fn handle_submodule_import(
        &mut self,
        module_id: crate::resolver::ModuleId,
        local_name: &str,
        full_module_path: &str,
    ) {
        if !self.bundler.inlined_modules.contains(&module_id) {
            return;
        }

        // Check if this is a namespace-imported module
        if self
            .bundler
            .namespace_imported_modules
            .contains_key(&module_id)
        {
            log::debug!("Not tracking namespace import as alias: {local_name} (namespace module)");
        } else if !self.is_entry_module() {
            // Track as alias in non-entry modules
            log::debug!("Tracking module import alias: {local_name} -> {full_module_path}");
            self.import_aliases
                .insert(local_name.to_string(), full_module_path.to_string());
        } else {
            log::debug!(
                "Not tracking module import as alias in entry module: {local_name} -> \
                 {full_module_path} (namespace object)"
            );
        }
    }

    /// Handle stdlib from-imports
    fn handle_stdlib_from_import(
        &mut self,
        import_from: &StmtImportFrom,
        module_str: &str,
    ) -> Option<Vec<Stmt>> {
        if import_from.level != 0 || !self.should_normalize_stdlib_import(module_str) {
            return None;
        }

        // Track that this stdlib module was imported
        self.imported_stdlib_modules.insert(module_str.to_string());
        // Also track parent modules for dotted imports
        if let Some(dot_pos) = module_str.find('.') {
            let parent = &module_str[..dot_pos];
            self.imported_stdlib_modules.insert(parent.to_string());
        }

        let mut assignments = Vec::new();
        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            if imported_name == "*" {
                // Preserve wildcard imports from stdlib to avoid incorrect symbol drops
                return Some(vec![Stmt::ImportFrom(import_from.clone())]);
            }

            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            let full_path = format!(
                "{}.{module_str}.{imported_name}",
                crate::ast_builder::CRIBO_PREFIX
            );

            // Track this renaming for expression rewriting
            if module_str == "importlib" && imported_name == "import_module" {
                self.import_aliases.insert(
                    local_name.to_string(),
                    format!("{module_str}.{imported_name}"),
                );
            } else {
                self.import_aliases
                    .insert(local_name.to_string(), full_path.clone());
            }

            // Create local assignment: local_name = _cribo.module.symbol
            let proxy_parts: Vec<&str> = full_path.split('.').collect();
            let value_expr =
                crate::ast_builder::expressions::dotted_name(&proxy_parts, ExprContext::Load);
            let target = crate::ast_builder::expressions::name(local_name, ExprContext::Store);
            let assign_stmt = crate::ast_builder::statements::assign(vec![target], value_expr);
            assignments.push(assign_stmt);
        }

        Some(assignments)
    }

    /// Handle stdlib imports in wrapper modules
    fn handle_wrapper_stdlib_imports(
        &mut self,
        stdlib_imports: &[(String, Option<String>)],
    ) -> Vec<Stmt> {
        let mut assignments = Vec::new();

        for (module_name, alias) in stdlib_imports {
            // Determine the local name that the import creates
            let local_name = if let Some(alias_name) = alias {
                // Aliased import: "import json as j" creates local "j"
                alias_name.clone()
            } else if module_name.contains('.') {
                // Dotted import without alias doesn't create a binding
                continue;
            } else {
                // Simple import: "import json" creates local "json"
                module_name.clone()
            };

            // 1) Create local alias: local = _cribo.<stdlib_module>
            let proxy_path = format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX);
            let proxy_parts: Vec<&str> = proxy_path.split('.').collect();
            let value_expr =
                crate::ast_builder::expressions::dotted_name(&proxy_parts, ExprContext::Load);
            let target =
                crate::ast_builder::expressions::name(local_name.as_str(), ExprContext::Store);
            assignments.push(crate::ast_builder::statements::assign(
                vec![target],
                value_expr,
            ));

            // 2) Set module attribute: <current_module>.<local> = <local>
            // In wrapper init functions, use "self" instead of the module name
            let module_var = if self.is_wrapper_init {
                "self".to_string()
            } else {
                crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                    &self.get_module_name(),
                )
            };
            assignments.push(
                crate::code_generator::module_registry::create_module_attr_assignment(
                    &module_var,
                    local_name.as_str(),
                ),
            );

            // 3) Optionally expose on self if part of exports (__all__) for this module
            // Skip this for wrapper init since we already added it above
            if !self.is_wrapper_init
                && let Some(Some(exports)) = self.bundler.module_exports.get(&self.module_id)
                && exports.contains(&local_name)
            {
                assignments.push(crate::ast_builder::statements::assign_attribute(
                    "self",
                    local_name.as_str(),
                    crate::ast_builder::expressions::name(local_name.as_str(), ExprContext::Load),
                ));
            }
        }

        assignments
    }

    /// Transform a statement, potentially returning multiple statements
    fn transform_statement(&mut self, stmt: &mut Stmt) -> Vec<Stmt> {
        // Check if it's a hoisted import before matching
        let is_hoisted = import_deduplicator::is_hoisted_import(self.bundler, stmt);

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
                        if self.should_normalize_stdlib_import(module_name) {
                            // Track that this stdlib module was imported
                            self.imported_stdlib_modules.insert(module_name.to_string());
                            // Also track parent modules for dotted imports (e.g., collections.abc
                            // imports collections too)
                            if let Some(dot_pos) = module_name.find('.') {
                                let parent = &module_name[..dot_pos];
                                self.imported_stdlib_modules.insert(parent.to_string());
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
                        let rename_map = self.build_stdlib_rename_map(&stdlib_imports);

                        // Track these renames for expression rewriting
                        for (local_name, rewritten_path) in rename_map {
                            self.import_aliases.insert(local_name, rewritten_path);
                        }

                        // If we're in a wrapper module, create local assignments for stdlib imports
                        if self.is_wrapper_init {
                            let mut assignments =
                                self.handle_wrapper_stdlib_imports(&stdlib_imports);

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
                                    self.import_aliases
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
                        if !self.is_entry_module()
                            && alias.asname.is_some()
                            && self
                                .bundler
                                .get_module_id(module_name)
                                .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                        {
                            log::debug!("Tracking import alias: {local_name} -> {module_name}");
                            self.import_aliases
                                .insert(local_name.to_string(), module_name.to_string());
                        }
                        // Also track importlib aliases for static import resolution (in any module)
                        else if module_name == "importlib" && alias.asname.is_some() {
                            log::debug!("Tracking importlib alias: {local_name} -> importlib");
                            self.import_aliases
                                .insert(local_name.to_string(), "importlib".to_string());
                        }
                    }

                    let result = rewrite_import_with_renames(
                        self.bundler,
                        new_import.clone(),
                        self.symbol_renames,
                        &mut self.populated_modules,
                    );

                    // Track any aliases created by the import to prevent incorrect stdlib
                    // transformations
                    for alias in &new_import.names {
                        if let Some(asname) = &alias.asname {
                            let local_name = asname.as_str();
                            self.local_variables.insert(local_name.to_string());
                            log::debug!(
                                "Tracking import alias as local variable: {} (from {})",
                                local_name,
                                alias.name.as_str()
                            );
                        }
                    }

                    log::debug!(
                        "rewrite_import_with_renames for module '{}': import {:?} -> {} statements",
                        self.get_module_name(),
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
                        self.is_entry_module()
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
                                self.import_aliases.insert(
                                    local_name.to_string(),
                                    "importlib.import_module".to_string(),
                                );
                            }
                        }
                    }

                    // Resolve relative imports first
                    let resolved_module = if import_from.level > 0 {
                        self.get_module_path().as_deref().and_then(|path| {
                            self.bundler
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
            if let Some(result) = self.handle_stdlib_from_import(import_from, module_str) {
                return result;
            }
        }

        // Resolve relative imports
        let resolved_module = if import_from.level > 0 {
            self.get_module_path().as_deref().and_then(|path| {
                self.bundler
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
            self.is_wrapper_init,
            self.get_module_name()
        );

        // For entry module, check if this import would duplicate deferred imports
        if self.is_entry_module()
            && let Some(ref resolved) = resolved_module
        {
            // Check if this is a wrapper module
            if self.bundler.get_module_id(resolved).is_some_and(|id| {
                self.bundler
                    .module_info_registry
                    .as_ref()
                    .is_some_and(|reg| reg.contains_module(&id))
            }) {
                // Check if we have access to global deferred imports
                if let Some(global_deferred) = self.global_deferred_imports {
                    // Check each symbol to see if it's already been deferred
                    let mut all_symbols_deferred = true;
                    if let Some(module_id) = self.bundler.resolver.get_module_id_by_name(resolved) {
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
                        return vec![];
                    }
                }
            }
        }

        // Check if we're importing submodules that have been inlined
        // e.g., from utils import calculator where calculator is utils.calculator
        // This must be checked BEFORE checking if the parent module is inlined
        let mut result_stmts = Vec::new();
        let mut handled_any = false;

        // Handle both regular module imports and relative imports
        if let Some(ref resolved_base) = resolved_module {
            log::debug!(
                "RecursiveImportTransformer: Checking import from '{}' in module '{}'",
                resolved_base,
                self.get_module_name()
            );

            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                let full_module_path = format!("{resolved_base}.{imported_name}");

                log::debug!("  Checking if '{full_module_path}' is an inlined module");
                log::debug!(
                    "  inlined_modules contains '{}': {}",
                    full_module_path,
                    self.bundler
                        .get_module_id(&full_module_path)
                        .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                );

                // Check if this is importing a submodule (like from . import config)
                // First check if it's a wrapper submodule, then check if it's inlined
                let is_wrapper_submodule =
                    if let Some(submodule_id) = self.bundler.get_module_id(&full_module_path) {
                        crate::code_generator::module_registry::is_wrapper_submodule(
                            submodule_id,
                            self.bundler.module_info_registry,
                            &self.bundler.inlined_modules,
                        )
                    } else {
                        false
                    };

                if is_wrapper_submodule {
                    // This is a wrapper submodule
                    log::debug!("  '{full_module_path}' is a wrapper submodule");

                    // For wrapper modules importing wrapper submodules from the same package
                    if self.is_wrapper_init {
                        // Initialize the wrapper submodule if needed
                        // Pass the current module context to avoid recursive initialization
                        if let Some(module_id) = self.bundler.get_module_id(&full_module_path) {
                            let current_module_id =
                                self.bundler.get_module_id(&self.get_module_name());
                            result_stmts.extend(
                                self.bundler
                                    .create_module_initialization_for_import_with_current_module(
                                        module_id,
                                        current_module_id,
                                        /* at_module_level */ true,
                                    ),
                            );
                        }

                        // Create assignment: local_name = parent.submodule
                        let module_expr =
                            expressions::module_reference(&full_module_path, ExprContext::Load);

                        result_stmts.push(statements::simple_assign(local_name, module_expr));

                        // Track as local to avoid any accidental rewrites later in this transform
                        // pass
                        self.local_variables.insert(local_name.to_string());

                        log::debug!(
                            "  Created assignment for wrapper submodule: {local_name} = \
                             {full_module_path}"
                        );

                        // Note: The module attribute assignment (_cribo_module.<local_name> = ...)
                        // is handled later in create_assignments_for_inlined_imports to avoid
                        // duplication

                        handled_any = true;
                    } else if !self.is_entry_module()
                        && self.bundler.inlined_modules.contains(&self.module_id)
                    {
                        // This is an inlined module importing a wrapper submodule
                        // We need to defer this import because the wrapper module may not be
                        // initialized yet
                        log::debug!(
                            "  Inlined module '{}' importing wrapper submodule '{}' - deferring",
                            self.get_module_name(),
                            full_module_path
                        );

                        // Note: deferred imports functionality has been removed
                        // The wrapper module assignment was previously deferred but is no longer
                        // needed

                        // Track as local to avoid any accidental rewrites later in this transform
                        // pass
                        self.local_variables.insert(local_name.to_string());

                        handled_any = true;
                    }
                } else if let Some(module_id) = self.bundler.get_module_id(&full_module_path) {
                    if self.bundler.inlined_modules.contains(&module_id) {
                        log::debug!("  '{full_module_path}' is an inlined module");

                        // Check if this module was namespace imported
                        if self
                            .bundler
                            .namespace_imported_modules
                            .contains_key(&module_id)
                        {
                            // Create assignment: local_name = full_module_path_with_underscores
                            // But be careful about stdlib conflicts - only create in entry module
                            // if there's a conflict
                            // Use get_module_var_identifier to handle symlinks properly
                            use crate::code_generator::module_registry::get_module_var_identifier;
                            let namespace_var =
                                get_module_var_identifier(module_id, self.bundler.resolver);

                            // Check if this would shadow a stdlib module
                            let shadows_stdlib =
                                crate::resolver::is_stdlib_module(local_name, self.python_version);

                            // Only create the assignment if:
                            // 1. We're in the entry module (where user expects the shadowing), OR
                            // 2. The name doesn't conflict with stdlib
                            if self.is_entry_module() || !shadows_stdlib {
                                log::debug!(
                                    "  Creating namespace assignment: {local_name} = \
                                     {namespace_var}"
                                );
                                result_stmts.push(statements::simple_assign(
                                    local_name,
                                    expressions::name(&namespace_var, ExprContext::Load),
                                ));

                                // Track this as a local variable to prevent it from being
                                // transformed as a stdlib module
                                self.local_variables.insert(local_name.to_string());
                                log::debug!(
                                    "  Tracked '{local_name}' as local variable to prevent stdlib \
                                     transformation"
                                );
                            } else {
                                log::debug!(
                                    "  Skipping namespace assignment: {local_name} = \
                                     {namespace_var} - would shadow stdlib in non-entry module"
                                );
                            }
                            handled_any = true;
                        }
                    } else {
                        // This is importing an inlined submodule
                        // We need to handle this specially when the current module is being inlined
                        // (i.e., not the entry module and not a wrapper module)
                        let current_module_is_inlined =
                            self.bundler.inlined_modules.contains(&self.module_id);
                        let current_module_is_wrapper =
                            !current_module_is_inlined && !self.is_entry_module();

                        if !self.is_entry_module()
                            && (current_module_is_inlined || current_module_is_wrapper)
                        {
                            log::debug!(
                                "  Creating namespace for inlined submodule: {local_name} -> \
                                 {full_module_path}"
                            );

                            if current_module_is_inlined {
                                // For inlined modules importing other inlined modules, we need to
                                // defer the namespace creation
                                // until after all modules are inlined
                                log::debug!(
                                    "  Deferring namespace creation for inlined module import"
                                );

                                // Create the namespace and populate it as deferred imports
                                // For inlined modules, use the sanitized module name instead of
                                // local_name e.g., pkg_compat
                                // instead of compat
                                // Use get_module_var_identifier to handle symlinks properly
                                use crate::code_generator::module_registry::get_module_var_identifier;
                                let namespace_var =
                                    get_module_var_identifier(module_id, self.bundler.resolver);

                                // Deferred namespace creation removed; skip no-op branch

                                // IMPORTANT: Create the local alias immediately, not deferred
                                // This ensures the alias is available in the current module's
                                // context For example, when `from .
                                // import messages` in greetings.greeting,
                                // we need `messages = greetings_messages` to be available
                                // immediately
                                self.alias_local_to_namespace_if_needed(
                                    local_name,
                                    &namespace_var,
                                    &mut result_stmts,
                                );
                                self.created_namespace_objects = true;

                                // If this is a submodule being imported (from . import compat),
                                // and the parent module is also being used as a namespace
                                // externally, we need to create the
                                // parent.child assignment
                                self.maybe_log_parent_child_assignment(
                                    resolved_module.as_deref(),
                                    imported_name,
                                    local_name,
                                );

                                // Mark namespace populated if needed (keep deferred behavior)
                                self.mark_namespace_populated_if_needed(&full_module_path);
                            } else {
                                // For wrapper modules importing inlined modules, we need to create
                                // the namespace immediately since it's used in the module body
                                log::debug!(
                                    "  Creating immediate namespace for wrapper module import"
                                );

                                // Create: local_name = types.SimpleNamespace()
                                result_stmts.push(statements::simple_assign(
                                    local_name,
                                    expressions::call(
                                        expressions::simple_namespace_ctor(),
                                        vec![],
                                        vec![],
                                    ),
                                ));
                                self.created_namespace_objects = true;

                                self.emit_namespace_symbols_for_local_from_path(
                                    local_name,
                                    &full_module_path,
                                    &mut result_stmts,
                                );
                            }

                            handled_any = true;
                        } else if !self.is_entry_module() {
                            // This is a wrapper module importing an inlined module
                            log::debug!(
                                "  Deferring inlined submodule import in wrapper module: \
                                 {local_name} -> {full_module_path}"
                            );
                        } else {
                            // For entry module, create namespace object immediately

                            // Create the namespace object with symbols
                            // This mimics what happens in non-entry modules

                            // First create the empty namespace
                            result_stmts.push(statements::simple_assign(
                                local_name,
                                expressions::call(
                                    expressions::simple_namespace_ctor(),
                                    vec![],
                                    vec![],
                                ),
                            ));

                            // Track this as a local variable, not an import alias
                            self.local_variables.insert(local_name.to_string());

                            handled_any = true;
                        }
                    }
                }
            }
        }

        if handled_any {
            // For deferred imports, we return empty to remove the original import
            if result_stmts.is_empty() {
                log::debug!("  Import handling deferred, returning empty");
                return vec![];
            }
            log::debug!(
                "  Returning {} transformed statements for import",
                result_stmts.len()
            );
            log::debug!("  Statements: {result_stmts:?}");
            // We've already handled the import completely, don't fall through to other handling
            return result_stmts;
        }

        if let Some(ref resolved) = resolved_module {
            // Check if this is an inlined module
            if let Some(resolved_id) = self.bundler.get_module_id(resolved)
                && self.bundler.inlined_modules.contains(&resolved_id)
            {
                // Check if this is a circular module with pre-declarations
                if self.bundler.circular_modules.contains(&resolved_id) {
                    log::debug!("  Module '{resolved}' is a circular module with pre-declarations");
                    log::debug!(
                        "  Current module '{}' is circular: {}, is inlined: {}",
                        self.get_module_name(),
                        self.bundler.circular_modules.contains(&self.module_id),
                        self.bundler.inlined_modules.contains(&self.module_id)
                    );
                    // Special handling for imports between circular inlined modules
                    // If the current module is also a circular inlined module, we need to defer or
                    // transform differently
                    if self.bundler.circular_modules.contains(&self.module_id)
                        && self.bundler.inlined_modules.contains(&self.module_id)
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
                                "  Checking if '{full_submodule_path}' is a submodule (bundled: \
                                 {}, inlined: {})",
                                self.bundler
                                    .get_module_id(&full_submodule_path)
                                    .is_some_and(|id| self.bundler.bundled_modules.contains(&id)),
                                self.bundler
                                    .get_module_id(&full_submodule_path)
                                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                            );
                            if self
                                .bundler
                                .get_module_id(&full_submodule_path)
                                .is_some_and(|id| self.bundler.bundled_modules.contains(&id))
                                || self
                                    .bundler
                                    .get_module_id(&full_submodule_path)
                                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                            {
                                log::debug!(
                                    "  Skipping assignment for '{imported_name}' - it's a \
                                     submodule, not a symbol"
                                );
                                // This is a submodule import, not a symbol import
                                // The submodule will be handled separately, so we don't create an
                                // assignment
                                continue;
                            }

                            // Check if the symbol was renamed during bundling
                            let actual_name = if let Some(resolved_id) =
                                self.bundler.get_module_id(resolved)
                                && let Some(renames) = self.symbol_renames.get(&resolved_id)
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
                        return assignments;
                    }
                    // Original behavior for non-circular modules importing from circular
                    // modules
                    return handle_imports_from_inlined_module_with_context(
                        self.bundler,
                        import_from,
                        resolved_id,
                        self.symbol_renames,
                        self.is_wrapper_init,
                        Some(self.module_id),
                    );
                } else {
                    log::debug!("  Module '{resolved}' is inlined, handling import assignments");
                    // For the entry module, we should not defer these imports
                    // because they need to be available when the entry module's code runs
                    let import_stmts = handle_imports_from_inlined_module_with_context(
                        self.bundler,
                        import_from,
                        resolved_id,
                        self.symbol_renames,
                        self.is_wrapper_init,
                        Some(self.module_id),
                    );

                    // Only defer if we're not in the entry module or wrapper init
                    if self.is_entry_module() || self.is_wrapper_init {
                        // For entry module and wrapper init functions, return the imports
                        // immediately In wrapper init functions, module
                        // attributes need to be set where the import was
                        if !import_stmts.is_empty() {
                            return import_stmts;
                        }
                        // If handle_imports_from_inlined_module returned empty (e.g., for submodule
                        // imports), fall through to check if we need to
                        // handle it differently
                        log::debug!(
                            "  handle_imports_from_inlined_module returned empty for entry module \
                             or wrapper init, checking for submodule imports"
                        );
                    } else {
                        // Return the import statements immediately
                        // These were previously deferred but now need to be added immediately
                        return import_stmts;
                    }
                }
            }

            // Check if this is a wrapper module (in module_registry)
            // This check must be after the inlined module check to avoid double-handling
            // A module is a wrapper module if it has an init function
            if self
                .bundler
                .get_module_id(resolved)
                .is_some_and(|id| self.bundler.module_init_functions.contains_key(&id))
            {
                log::debug!("  Module '{resolved}' is a wrapper module");

                // For modules importing from wrapper modules, we may need to defer
                // the imports to ensure proper initialization order
                let current_module_is_inlined =
                    self.bundler.inlined_modules.contains(&self.module_id);

                // When an inlined module imports from a wrapper module, we need to
                // track the imports and rewrite all usages within the module
                if !self.is_entry_module() && current_module_is_inlined {
                    log::debug!(
                        "  Tracking wrapper module imports for rewriting in module '{}' (inlined: \
                         {})",
                        self.get_module_name(),
                        current_module_is_inlined
                    );

                    // First, ensure the wrapper module is initialized
                    // This is crucial for lazy imports inside functions
                    let mut init_stmts = Vec::new();

                    // Check if the parent module needs handling
                    if let Some((parent, child)) = resolved.rsplit_once('.') {
                        // If the parent is also a wrapper module, DO NOT initialize it here
                        // It will be initialized when accessed
                        if self
                            .bundler
                            .get_module_id(parent)
                            .is_some_and(|id| self.bundler.module_init_functions.contains_key(&id))
                        {
                            log::debug!(
                                "  Parent '{parent}' is a wrapper module - skipping immediate \
                                 initialization"
                            );
                            // Don't initialize parent wrapper module here
                        }

                        // If the parent is an inlined module, the submodule assignment is handled
                        // by its own initialization, so we only need to log
                        if self
                            .bundler
                            .get_module_id(parent)
                            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                        {
                            log::debug!(
                                "Parent '{parent}' is inlined, submodule '{child}' assignment \
                                 already handled"
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
                        self.get_module_name().starts_with(&format!("{resolved}."))
                    } else {
                        false
                    };

                    // Get module ID if it exists and has an init function
                    let wrapper_module_id = if !is_wildcard && !is_parent_import {
                        self.bundler
                            .get_module_id(resolved)
                            .filter(|id| self.bundler.module_init_functions.contains_key(id))
                    } else {
                        None
                    };

                    if let Some(module_id) = wrapper_module_id {
                        // Do not emit init calls for the entry package (__init__ or __main__).
                        // Initializing the entry package from submodules can create circular init.
                        let is_entry_pkg = if self.bundler.entry_is_package_init_or_main {
                            let entry_pkg = [
                                crate::python::constants::INIT_STEM,
                                crate::python::constants::MAIN_STEM,
                            ]
                            .iter()
                            .find_map(|stem| {
                                self.bundler
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
                                "  Generating initialization call for wrapper module '{resolved}' \
                                 at import location"
                            );

                            // Use ast_builder helper to generate wrapper init call
                            use crate::{
                                ast_builder::module_wrapper,
                                code_generator::module_registry::get_module_var_identifier,
                            };

                            let module_var =
                                get_module_var_identifier(module_id, self.bundler.resolver);

                            // If we're not at module level (i.e., inside any local scope), we need
                            // to declare the module variable as global
                            // to avoid UnboundLocalError.
                            if !self.at_module_level {
                                log::debug!(
                                    "  Adding global declaration for '{module_var}' (inside local \
                                     scope)"
                                );
                                init_stmts.push(crate::ast_builder::statements::global(vec![
                                    module_var.as_str(),
                                ]));
                            }

                            init_stmts
                                .push(module_wrapper::create_wrapper_module_init_call(&module_var));
                        }
                    } else if is_parent_import && !is_wildcard {
                        log::debug!(
                            "  Skipping init call for parent package '{resolved}' from inlined \
                             submodule '{}'",
                            self.get_module_name()
                        );
                    }

                    // Handle wildcard import export assignments
                    if is_wildcard {
                        self.log_wrapper_wildcard_info(resolved);
                        log::debug!(
                            "  Returning {} parent-init statements for wildcard import; wrapper \
                             init + assignments were deferred",
                            init_stmts.len()
                        );
                        return init_stmts;
                    }

                    // Track each imported symbol for rewriting
                    // Use the canonical module name if we have a wrapper module ID
                    let module_name_for_tracking = if let Some(module_id) = wrapper_module_id {
                        self.bundler
                            .resolver
                            .get_module_name(module_id)
                            .unwrap_or_else(|| resolved.clone())
                    } else {
                        resolved.clone()
                    };

                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                        // Store mapping: local_name -> (wrapper_module, imported_name)
                        self.wrapper_module_imports.insert(
                            local_name.to_string(),
                            (module_name_for_tracking.clone(), imported_name.to_string()),
                        );

                        log::debug!(
                            "    Tracking import: {local_name} -> \
                             {module_name_for_tracking}.{imported_name}"
                        );
                    }

                    // Defer to the standard bundled-wrapper transformation to generate proper
                    // alias assignments and ensure initialization ordering. This keeps behavior
                    // consistent and avoids missing local aliases needed for class bases.
                    // The rewrite_import_from will handle creating the proper assignments
                    // after the wrapper module is initialized.
                    let mut result = rewrite_import_from(RewriteImportFromParams {
                        bundler: self.bundler,
                        import_from: import_from.clone(),
                        current_module: &self.get_module_name(),
                        module_path: self.get_module_path().as_deref(),
                        symbol_renames: self.symbol_renames,
                        inside_wrapper_init: self.is_wrapper_init,
                        at_module_level: self.at_module_level,
                        python_version: self.python_version,
                    });

                    // Prepend the init statements to ensure wrapper is initialized before use
                    init_stmts.append(&mut result);
                    return init_stmts;
                }
                // For wrapper modules importing from other wrapper modules,
                // let it fall through to standard transformation
            }
        }

        // Otherwise, use standard transformation
        rewrite_import_from(RewriteImportFromParams {
            bundler: self.bundler,
            import_from: import_from.clone(),
            current_module: &self.get_module_name(),
            module_path: self.get_module_path().as_deref(),
            symbol_renames: self.symbol_renames,
            inside_wrapper_init: self.is_wrapper_init,
            at_module_level: self.at_module_level,
            python_version: self.python_version,
        })
    }

    /// Transform an expression, rewriting module attribute access to direct references
    fn transform_expr(&mut self, expr: &mut Expr) {
        // First check if this is an attribute expression and collect the path
        let attribute_info = if matches!(expr, Expr::Attribute(_)) {
            let info = self.collect_attribute_path(expr);
            log::debug!(
                "transform_expr: Found attribute expression - base: {:?}, path: {:?}, \
                 is_entry_module: {}",
                info.0,
                info.1,
                self.is_entry_module()
            );

            Some(info)
        } else {
            None
        };

        match expr {
            Expr::Attribute(attr_expr) => {
                // Special case: inside a wrapper module's init function, rewrite references
                // to the current module accessed via the parent namespace (e.g.,
                // rich.console.X inside rich.console __init__) to use `self` directly.
                // Do this before any other handling to avoid re-entrancy issues.
                if self.is_wrapper_init {
                    let current = self.get_module_name();
                    if let Some((root, rel)) = current.split_once('.') {
                        // Try to find the inner attribute node where value is Name(root)
                        // and attribute equals the current module's relative name (rel).
                        let mut cursor: &mut ruff_python_ast::ExprAttribute = attr_expr; // start at outer attribute
                        while let Expr::Attribute(inner) = cursor.value.as_mut() {
                            let is_base = matches!(
                                inner.value.as_ref(),
                                Expr::Name(n) if n.id.as_str() == root
                            ) && inner.attr.as_str() == rel;
                            if is_base {
                                inner.value =
                                    Box::new(expressions::name("self", ExprContext::Load));
                                return;
                            }
                            cursor = inner;
                        }
                    }
                }

                // First check if the base of this attribute is a wrapper module import
                if let Expr::Name(base_name) = &*attr_expr.value {
                    let name = base_name.id.as_str();

                    // Check if this is a stdlib module reference (e.g., collections.abc)
                    if crate::resolver::is_stdlib_module(name, self.python_version) {
                        // Check if this stdlib name is shadowed by local variables or imports
                        // In wrapper modules, we only track local_variables which includes imported
                        // names
                        let is_shadowed = self.local_variables.contains(name)
                            || self.import_aliases.contains_key(name);

                        if !is_shadowed {
                            // Transform stdlib module attribute access to use _cribo proxy
                            // e.g., collections.abc -> _cribo.collections.abc
                            log::debug!(
                                "Transforming stdlib attribute access: {}.{} -> _cribo.{}.{}",
                                name,
                                attr_expr.attr.as_str(),
                                name,
                                attr_expr.attr.as_str()
                            );

                            // Create _cribo.module.attr
                            let attr_name = attr_expr.attr.to_string();
                            let attr_ctx = attr_expr.ctx;
                            let attr_range = attr_expr.range;

                            // Create _cribo.name.attr_name
                            let base = expressions::name_attribute(
                                crate::ast_builder::CRIBO_PREFIX,
                                name,
                                ExprContext::Load,
                            );
                            let mut new_expr = expressions::attribute(base, &attr_name, attr_ctx);
                            // Preserve the original range
                            if let Expr::Attribute(attr) = &mut new_expr {
                                attr.range = attr_range;
                            }
                            *expr = new_expr;
                            return;
                        }
                    }

                    if let Some((wrapper_module, imported_name)) =
                        self.wrapper_module_imports.get(name)
                    {
                        // The base is a wrapper module import, rewrite the entire attribute access
                        // e.g., cookielib.CookieJar -> myrequests.compat.cookielib.CookieJar
                        log::debug!(
                            "Rewriting attribute '{}.{}' to '{}.{}.{}'",
                            name,
                            attr_expr.attr.as_str(),
                            wrapper_module,
                            imported_name,
                            attr_expr.attr.as_str()
                        );

                        // Create wrapper_module.imported_name.attr
                        let base = expressions::name_attribute(
                            wrapper_module,
                            imported_name,
                            ExprContext::Load,
                        );
                        let mut new_expr =
                            expressions::attribute(base, attr_expr.attr.as_str(), attr_expr.ctx);
                        // Preserve the original range
                        if let Expr::Attribute(attr) = &mut new_expr {
                            attr.range = attr_expr.range;
                        }
                        *expr = new_expr;
                        return; // Don't process further
                    }
                }

                // Handle nested attribute access using the pre-collected path
                if let Some((base_name, attr_path)) = attribute_info {
                    if let Some(base) = base_name {
                        // In the entry module, check if this is accessing a namespace object
                        // created by a dotted import
                        if self.is_entry_module() && attr_path.len() >= 2 {
                            // For "greetings.greeting.get_greeting()", we have:
                            // base: "greetings", attr_path: ["greeting", "get_greeting"]
                            // Check if "greetings.greeting" is a bundled module (created by "import
                            // greetings.greeting")
                            let namespace_path = format!("{}.{}", base, attr_path[0]);

                            if self
                                .bundler
                                .get_module_id(&namespace_path)
                                .is_some_and(|id| self.bundler.bundled_modules.contains(&id))
                            {
                                // This is accessing a method/attribute on a namespace object
                                // created by a dotted import
                                // Don't transform it - let the namespace object handle it
                                log::debug!(
                                    "Not transforming {base}.{} - accessing namespace object \
                                     created by dotted import",
                                    attr_path.join(".")
                                );
                                // Don't recursively transform - the whole expression should remain
                                // as-is
                                return;
                            }
                        }

                        // First check if the base is a variable assigned from
                        // importlib.import_module()
                        if let Some(module_name) = self.importlib_inlined_modules.get(&base) {
                            // This is accessing attributes on a variable that was assigned from
                            // importlib.import_module() of an inlined module
                            if attr_path.len() == 1 {
                                let attr_name = &attr_path[0];
                                log::debug!(
                                    "Transforming {base}.{attr_name} - {base} was assigned from \
                                     importlib.import_module('{module_name}') [inlined module]"
                                );
                                *expr = self.rewrite_attr_for_importlib_var(
                                    &base,
                                    attr_name,
                                    module_name,
                                    attr_expr.ctx,
                                    attr_expr.range,
                                );
                                return;
                            }
                        }
                        // Check if the base is a stdlib import alias (e.g., j for json)
                        else if let Some(stdlib_path) = self.import_aliases.get(&base) {
                            // This is accessing an attribute on a stdlib module alias
                            // Transform j.dumps to _cribo.json.dumps
                            if attr_path.len() == 1 {
                                let attr_name = &attr_path[0];
                                log::debug!(
                                    "Transforming {base}.{attr_name} to {stdlib_path}.{attr_name} \
                                     (stdlib import alias)"
                                );

                                // Create dotted name expression like _cribo.json.dumps
                                let full_path = format!("{stdlib_path}.{attr_name}");
                                let parts: Vec<&str> = full_path.split('.').collect();
                                let new_expr = crate::ast_builder::expressions::dotted_name(
                                    &parts,
                                    attr_expr.ctx,
                                );
                                *expr = new_expr;
                                return;
                            } else {
                                // For deeper paths like j.decoder.JSONDecoder, build the full path
                                let mut full_path = stdlib_path.clone();
                                for part in &attr_path {
                                    full_path.push('.');
                                    full_path.push_str(part);
                                }
                                log::debug!(
                                    "Transforming {base}.{} to {full_path} (stdlib import alias, \
                                     deep path)",
                                    attr_path.join(".")
                                );

                                let parts: Vec<&str> = full_path.split('.').collect();
                                let new_expr = crate::ast_builder::expressions::dotted_name(
                                    &parts,
                                    attr_expr.ctx,
                                );
                                *expr = new_expr;
                                return;
                            }
                        }
                        // Check if the base refers to an inlined module
                        else if let Some(actual_module) = self.find_module_for_alias(&base)
                            && self
                                .bundler
                                .get_module_id(&actual_module)
                                .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                        {
                            log::debug!(
                                "Found module alias: {base} -> {actual_module} (is_entry_module: \
                                 {})",
                                self.is_entry_module()
                            );

                            // For a single attribute access (e.g., greetings.message or
                            // config.DEFAULT_NAME)
                            if attr_path.len() == 1 {
                                let attr_name = &attr_path[0];
                                if let Some(new_expr) = self
                                    .try_rewrite_single_attr_for_inlined_module_alias(
                                        &base,
                                        &actual_module,
                                        attr_name,
                                        attr_expr.ctx,
                                        attr_expr.range,
                                    )
                                {
                                    *expr = new_expr;
                                    return;
                                }
                            }
                            // For nested attribute access (e.g., greetings.greeting.message)
                            // We need to handle the case where greetings.greeting is a submodule
                            else if attr_path.len() > 1
                                && let Some(new_name) = self
                                    .maybe_rewrite_attr_for_inlined_submodule(
                                        &base,
                                        &actual_module,
                                        &attr_path,
                                        attr_expr.ctx,
                                        attr_expr.range,
                                    )
                            {
                                *expr = new_name;
                                return;
                            }
                        }
                    }

                    // If we didn't handle it above, recursively transform the value
                    self.transform_expr(&mut attr_expr.value);
                } // Close the if let Some((base_name, attr_path)) = attribute_info
            }
            Expr::Call(call_expr) => {
                // Check if this is importlib.import_module() with a static string literal
                if self.is_importlib_import_module_call(call_expr)
                    && let Some(transformed) = self.transform_importlib_import_module(call_expr)
                {
                    *expr = transformed;
                    return;
                }

                self.transform_expr(&mut call_expr.func);
                for arg in &mut call_expr.arguments.args {
                    self.transform_expr(arg);
                }
                for keyword in &mut call_expr.arguments.keywords {
                    self.transform_expr(&mut keyword.value);
                }
            }
            Expr::BinOp(binop_expr) => {
                self.transform_expr(&mut binop_expr.left);
                self.transform_expr(&mut binop_expr.right);
            }
            Expr::UnaryOp(unaryop_expr) => {
                self.transform_expr(&mut unaryop_expr.operand);
            }
            Expr::BoolOp(boolop_expr) => {
                for value in &mut boolop_expr.values {
                    self.transform_expr(value);
                }
            }
            Expr::Compare(compare_expr) => {
                self.transform_expr(&mut compare_expr.left);
                for comparator in &mut compare_expr.comparators {
                    self.transform_expr(comparator);
                }
            }
            Expr::If(if_expr) => {
                self.transform_expr(&mut if_expr.test);
                self.transform_expr(&mut if_expr.body);
                self.transform_expr(&mut if_expr.orelse);
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    self.transform_expr(elem);
                }
            }
            Expr::Tuple(tuple_expr) => {
                for elem in &mut tuple_expr.elts {
                    self.transform_expr(elem);
                }
            }
            Expr::Dict(dict_expr) => {
                for item in &mut dict_expr.items {
                    if let Some(key) = &mut item.key {
                        self.transform_expr(key);
                    }
                    self.transform_expr(&mut item.value);
                }
            }
            Expr::Set(set_expr) => {
                for elem in &mut set_expr.elts {
                    self.transform_expr(elem);
                }
            }
            Expr::ListComp(listcomp_expr) => {
                self.transform_expr(&mut listcomp_expr.elt);
                for generator in &mut listcomp_expr.generators {
                    self.transform_expr(&mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        self.transform_expr(if_clause);
                    }
                }
            }
            Expr::DictComp(dictcomp_expr) => {
                self.transform_expr(&mut dictcomp_expr.key);
                self.transform_expr(&mut dictcomp_expr.value);
                for generator in &mut dictcomp_expr.generators {
                    self.transform_expr(&mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        self.transform_expr(if_clause);
                    }
                }
            }
            Expr::SetComp(setcomp_expr) => {
                self.transform_expr(&mut setcomp_expr.elt);
                for generator in &mut setcomp_expr.generators {
                    self.transform_expr(&mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        self.transform_expr(if_clause);
                    }
                }
            }
            Expr::Generator(genexp_expr) => {
                self.transform_expr(&mut genexp_expr.elt);
                for generator in &mut genexp_expr.generators {
                    self.transform_expr(&mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        self.transform_expr(if_clause);
                    }
                }
            }
            Expr::Subscript(subscript_expr) => {
                self.transform_expr(&mut subscript_expr.value);
                self.transform_expr(&mut subscript_expr.slice);
            }
            Expr::Slice(slice_expr) => {
                if let Some(lower) = &mut slice_expr.lower {
                    self.transform_expr(lower);
                }
                if let Some(upper) = &mut slice_expr.upper {
                    self.transform_expr(upper);
                }
                if let Some(step) = &mut slice_expr.step {
                    self.transform_expr(step);
                }
            }
            Expr::Lambda(lambda_expr) => {
                self.transform_expr(&mut lambda_expr.body);
            }
            Expr::Yield(yield_expr) => {
                if let Some(value) = &mut yield_expr.value {
                    self.transform_expr(value);
                }
            }
            Expr::YieldFrom(yieldfrom_expr) => {
                self.transform_expr(&mut yieldfrom_expr.value);
            }
            Expr::Await(await_expr) => {
                self.transform_expr(&mut await_expr.value);
            }
            Expr::Starred(starred_expr) => {
                self.transform_expr(&mut starred_expr.value);
            }
            Expr::FString(fstring_expr) => {
                // Transform expressions within the f-string
                let fstring_range = fstring_expr.range;
                // Preserve the original flags from the f-string
                let original_flags =
                    crate::ast_builder::expressions::get_fstring_flags(&fstring_expr.value);
                let mut transformed_elements = Vec::new();
                let mut any_transformed = false;

                for element in fstring_expr.value.elements() {
                    match element {
                        InterpolatedStringElement::Literal(lit_elem) => {
                            transformed_elements
                                .push(InterpolatedStringElement::Literal(lit_elem.clone()));
                        }
                        InterpolatedStringElement::Interpolation(expr_elem) => {
                            let mut new_expr = expr_elem.expression.clone();
                            self.transform_expr(&mut new_expr);

                            if !matches!(&new_expr, other if other == &expr_elem.expression) {
                                any_transformed = true;
                            }

                            let new_element = InterpolatedElement {
                                node_index: AtomicNodeIndex::dummy(),
                                expression: new_expr,
                                debug_text: expr_elem.debug_text.clone(),
                                conversion: expr_elem.conversion,
                                format_spec: expr_elem.format_spec.clone(),
                                range: expr_elem.range,
                            };
                            transformed_elements
                                .push(InterpolatedStringElement::Interpolation(new_element));
                        }
                    }
                }

                if any_transformed {
                    let new_fstring = FString {
                        node_index: AtomicNodeIndex::dummy(),
                        elements: InterpolatedStringElements::from(transformed_elements),
                        range: fstring_range,
                        flags: original_flags, // Preserve the original flags including quote style
                    };

                    let new_value = FStringValue::single(new_fstring);

                    *expr = Expr::FString(ExprFString {
                        node_index: AtomicNodeIndex::dummy(),
                        value: new_value,
                        range: fstring_range,
                    });
                }
            }
            // Check if Name expressions need to be rewritten for wrapper module imports or stdlib
            // imports
            Expr::Name(name_expr) => {
                let name = name_expr.id.as_str();

                // Check if this name is a stdlib import alias that needs rewriting
                // Only rewrite if it's not shadowed by a local variable
                if let Some(rewritten_path) = self.import_aliases.get(name) {
                    // Check if this is a stdlib module reference (starts with _cribo.)
                    if rewritten_path.starts_with(crate::ast_builder::CRIBO_PREFIX)
                        && rewritten_path
                            .chars()
                            .nth(crate::ast_builder::CRIBO_PREFIX.len())
                            == Some('.')
                    {
                        // Use semantic analysis to check if this is shadowed by a local variable
                        let is_shadowed =
                            if let Some(_semantic_bundler) = self.bundler.semantic_bundler {
                                // Try to find the module in the semantic bundler
                                // This is a simplified check - in reality we'd need to know the
                                // exact scope For now, we'll skip
                                // the semantic check if we don't have proper module info
                                false // TODO: Implement proper semantic check using SemanticModel
                            } else {
                                false
                            };

                        if !is_shadowed {
                            log::debug!(
                                "Rewriting stdlib reference '{name}' to '{rewritten_path}'"
                            );

                            // Parse the rewritten path to create attribute access
                            // e.g., "_cribo.json" becomes _cribo.json
                            let parts: Vec<&str> = rewritten_path.split('.').collect();
                            if parts.len() >= 2 {
                                *expr = expressions::dotted_name(&parts, name_expr.ctx);
                                return;
                            }
                        }
                    }
                }

                // Check if this name was imported from a wrapper module and needs rewriting
                if let Some((wrapper_module, imported_name)) = self.wrapper_module_imports.get(name)
                {
                    log::debug!("Rewriting name '{name}' to '{wrapper_module}.{imported_name}'");

                    // Create wrapper_module.imported_name attribute access
                    // Create wrapper_module.imported_name attribute access
                    let mut new_expr =
                        expressions::name_attribute(wrapper_module, imported_name, name_expr.ctx);
                    // Preserve the original range
                    if let Expr::Attribute(attr) = &mut new_expr {
                        attr.range = name_expr.range;
                    }
                    *expr = new_expr;
                }
            }
            // Constants, etc. don't need transformation
            _ => {}
        }
    }

    /// Collect the full dotted attribute path from a potentially nested attribute expression
    /// Returns (`base_name`, [attr1, attr2, ...])
    /// For example: greetings.greeting.message returns (Some("greetings"), ["greeting", "message"])
    fn collect_attribute_path(&self, expr: &Expr) -> (Option<String>, Vec<String>) {
        let mut attrs = Vec::new();
        let mut current = expr;

        loop {
            match current {
                Expr::Attribute(attr) => {
                    attrs.push(attr.attr.as_str().to_string());
                    current = &attr.value;
                }
                Expr::Name(name) => {
                    attrs.reverse();
                    return (Some(name.id.as_str().to_string()), attrs);
                }
                _ => {
                    attrs.reverse();
                    return (None, attrs);
                }
            }
        }
    }

    /// Find the actual module name for a given alias
    fn find_module_for_alias(&self, alias: &str) -> Option<String> {
        log::debug!(
            "find_module_for_alias: alias={}, is_entry_module={}, local_vars={:?}",
            alias,
            self.is_entry_module(),
            self.local_variables.contains(alias)
        );

        // Don't treat local variables as module aliases
        if self.local_variables.contains(alias) {
            return None;
        }

        // First check our tracked import aliases
        if let Some(module_name) = self.import_aliases.get(alias) {
            return Some(module_name.clone());
        }

        // Then check if the alias directly matches a module name
        // But not in the entry module - in the entry module, direct module names
        // are namespace objects, not aliases
        if !self.is_entry_module()
            && self
                .bundler
                .get_module_id(alias)
                .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
        {
            Some(alias.to_string())
        } else {
            None
        }
    }

    /// Create module access expression
    pub fn create_module_access_expr(&self, module_name: &str) -> Expr {
        // Check if this is a wrapper module
        if let Some(synthetic_name) = self
            .bundler
            .get_module_id(module_name)
            .and_then(|id| self.bundler.module_synthetic_names.get(&id))
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
            .bundler
            .get_module_id(module_name)
            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
        {
            // This is an inlined module - create namespace object
            let module_renames = self
                .bundler
                .get_module_id(module_name)
                .and_then(|id| self.symbol_renames.get(&id));
            self.create_namespace_call_for_inlined_module(module_name, module_renames)
        } else {
            // This module wasn't bundled - shouldn't happen for static imports
            log::warn!("Module '{module_name}' referenced in static import but not bundled");
            expressions::none_literal()
        }
    }

    /// Create a namespace call expression for an inlined module
    fn create_namespace_call_for_inlined_module(
        &self,
        module_name: &str,
        module_renames: Option<&FxIndexMap<String, String>>,
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
                let module_id = self
                    .bundler
                    .get_module_id(module_name)
                    .expect("Module should exist");
                if !self
                    .bundler
                    .is_symbol_kept_by_tree_shaking(module_id, original_name)
                {
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
        if let Some(module_id) = self.bundler.get_module_id(module_name)
            && let Some(exports) = self.bundler.module_exports.get(&module_id)
            && let Some(export_list) = exports
        {
            for export in export_list {
                // Check if this export was already added as a renamed symbol
                let was_renamed =
                    module_renames.is_some_and(|renames| renames.contains_key(export));
                if !was_renamed && !seen_args.contains(export) {
                    // Check if this symbol survived tree-shaking
                    if !self
                        .bundler
                        .is_symbol_kept_by_tree_shaking(module_id, export)
                    {
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
            bundler.resolver.resolve_relative_to_absolute_module_name(
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

    let Some(module_name) = resolved_module_name else {
        // If we can't resolve the module, return the original import
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
            return handle_imports_from_inlined_module_with_context(
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
            // This is a wrapper module, we need to transform it
            let context = crate::code_generator::bundler::BundledImportContext {
                inside_wrapper_init,
                at_module_level,
                current_module: Some(current_module),
            };
            return bundler.transform_bundled_import_from_multiple_with_current_module(
                &import_from,
                &module_name,
                context,
                symbol_renames,
            );
        }

        // No bundled submodules, keep original import
        // For relative imports from non-bundled modules, convert to absolute import
        if import_from.level > 0 {
            let mut absolute_import = import_from.clone();
            absolute_import.level = 0;
            absolute_import.module = Some(Identifier::new(&module_name, TextRange::default()));
            return vec![Stmt::ImportFrom(absolute_import)];
        }
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
            // Convert relative import to absolute
            absolute_import.level = 0;
            absolute_import.module = Some(Identifier::new(&module_name, TextRange::default()));
        }
        let context = crate::code_generator::bundler::BundledImportContext {
            inside_wrapper_init,
            at_module_level,
            current_module: Some(current_module),
        };
        bundler.transform_bundled_import_from_multiple_with_current_module(
            &absolute_import,
            &module_name,
            context,
            symbol_renames,
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

/// Handle imports from inlined modules
///
/// This function handles import statements that import from modules that have been inlined
/// into the bundle. It generates appropriate assignment statements to make the inlined
/// symbols available under their expected names.
///
/// # Parameters
/// - `bundler`: The bundler context
/// - `import_from`: The import statement being processed
/// - `source_module_id`: The ID of the module being imported FROM (the inlined module)
/// - `symbol_renames`: Map of symbol renames for all modules
/// - `is_wrapper_init`: Whether we're inside a wrapper module's init function
/// - `importing_module_id`: The ID of the module doing the importing (containing the import
///   statement)
pub(super) fn handle_imports_from_inlined_module_with_context(
    bundler: &Bundler,
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
        "handle_imports_from_inlined_module_with_context: source_module={}, available_renames={:?}",
        module_name,
        symbol_renames.get(&source_module_id)
    );
    let mut result_stmts = Vec::new();

    // Check if this is a wildcard import
    if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
        // Handle wildcard import from inlined module
        log::debug!("Handling wildcard import from inlined module '{module_name}'");

        // Get the module's exports (either from __all__ or all non-private symbols)
        let module_exports =
            if let Some(Some(export_list)) = bundler.module_exports.get(&source_module_id) {
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
                    "Skipping wildcard import for tree-shaken symbol '{symbol_name}' from module \
                     '{module_name}'"
                );
                continue;
            }

            // Get the renamed symbol name if it was renamed
            let renamed_symbol = if let Some(renames) = module_renames {
                renames
                    .get(symbol_name)
                    .cloned()
                    .unwrap_or_else(|| symbol_name.clone())
            } else {
                symbol_name.clone()
            };

            // For wildcard imports, we always need to create assignments for renamed symbols
            // For non-renamed symbols, we only skip assignment if they're actually available
            // in the current scope (i.e., they are in the module_exports list which respects
            // __all__)
            if renamed_symbol == *symbol_name {
                // Symbol wasn't renamed - it's already accessible in scope for symbols
                // that are in module_exports (which respects __all__)
                log::debug!("Symbol '{symbol_name}' is accessible directly from inlined module");
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
                    "Skipping submodule import '{imported_name}' from '{module_name}' - wrapper \
                     module import should be handled elsewhere"
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
                "Resolved re-exported symbol via inlined submodule: {module_name}.{imported_name} \
                 -> {global_name}"
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
                imported_name.to_string()
            } else {
                symbol_renames
                    .get(&source_module_id)
                    .and_then(|renames| renames.get(imported_name))
                    .cloned()
                    .unwrap_or_else(|| imported_name.to_string())
            }
        };

        log::debug!(
            "Processing import: module={}, imported_name={}, local_name={}, renamed_symbol={}, \
             available_renames={:?}",
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
                "Skipping import assignment for tree-shaken symbol '{imported_name}' from module \
                 '{module_name}' (non-wrapper context)"
            );
            continue;
        }

        // Handle wrapper init functions specially
        if is_wrapper_init {
            // When importing from an inlined module, we need to create the local alias FIRST
            // before setting the module attribute, because the module attribute assignment
            // uses the local name which won't exist until we create the alias
            let is_from_inlined = bundler.inlined_modules.contains(&source_module_id);

            // Create a local alias when:
            // 1. The names are different (aliased import), OR
            // 2. We're importing from an inlined module (need to access through namespace)
            if local_name != renamed_symbol || is_from_inlined {
                // When importing from an inlined module inside a wrapper init,
                // prefer qualifying with the module's namespace when the names are identical
                // to avoid creating a self-referential assignment like `x = x`.
                let source_expr = if is_from_inlined {
                    if local_name == renamed_symbol {
                        let module_namespace =
                            crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                                &module_name,
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
                            "Creating local alias from global symbol: {local_name} = \
                             {renamed_symbol} (imported from inlined module {module_name})"
                        );
                        expressions::name(&renamed_symbol, ExprContext::Load)
                    }
                } else {
                    log::debug!("Creating local alias: {local_name} = {renamed_symbol}");
                    expressions::name(&renamed_symbol, ExprContext::Load)
                };
                result_stmts.push(statements::simple_assign(local_name, source_expr));
            }

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
                // When importing from an inlined module, use the local name we just created
                // Otherwise use the renamed symbol directly
                let attr_value = if is_from_inlined {
                    local_name
                } else {
                    &renamed_symbol
                };
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

                // Also expose on the namespace (self.<name> = <name>) so that
                // dir(__cribo_init_result) copies include it. Skip for imports coming from
                // inlined modules to avoid redundant assignments inside inlined init functions.
                if !is_from_inlined {
                    result_stmts.push(statements::assign_attribute(
                        "self",
                        local_name,
                        expressions::name(local_name, ExprContext::Load),
                    ));
                }
            } else {
                log::warn!(
                    "is_wrapper_init is true but current_module is None, skipping module \
                     attribute assignment"
                );
            }
        } else if local_name != renamed_symbol {
            // For non-wrapper contexts, only create assignment if names differ
            // For inlined modules, reference the namespace attribute instead of the renamed symbol
            // directly This avoids ordering issues where the renamed symbol might not
            // be defined yet
            let module_namespace =
                crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                    &module_name,
                );
            log::debug!("Creating assignment: {local_name} = {module_namespace}.{imported_name}");
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
                crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                    &module_name,
                );
            log::debug!("Creating assignment: {local_name} = {module_namespace}.{imported_name}");
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

/// Check if a symbol is likely a re-export from a package __init__.py
fn is_package_init_reexport(bundler: &Bundler, module_name: &str) -> bool {
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
