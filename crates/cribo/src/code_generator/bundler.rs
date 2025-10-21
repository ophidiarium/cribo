// File previously allowed clippy::excessive_nesting. Refactor reduced nesting instead.

use std::path::PathBuf;

use ruff_python_ast::{
    AtomicNodeIndex, ExceptHandler, Expr, ExprContext, Keyword, ModModule, Stmt, StmtAssign,
    StmtClassDef, StmtFunctionDef, StmtImportFrom,
};
use ruff_text_size::TextRange;

use crate::{
    analyzers::ImportAnalyzer,
    ast_builder::{expressions, other, statements},
    code_generator::{
        circular_deps::SymbolDependencyGraph,
        context::{BundleParams, InlineContext, SemanticContext},
        expression_handlers, import_deduplicator,
        module_registry::{INIT_RESULT_VAR, is_init_function, sanitize_module_name_for_identifier},
    },
    dependency_graph::DependencyGraph,
    resolver::{ModuleId, ModuleResolver},
    transformation_context::TransformationContext,
    types::{FxIndexMap, FxIndexSet},
    visitors::LocalVarCollector,
};

/// Parameters for transforming functions with lifted globals
struct TransformFunctionParams<'a> {
    lifted_names: &'a FxIndexMap<String, String>,
    global_info: &'a crate::symbol_conflict_resolver::ModuleGlobalInfo,
    function_globals: &'a FxIndexSet<String>,
    module_name: Option<&'a str>,
}

/// Context for transforming bundled imports
pub(super) struct BundledImportContext<'a> {
    pub inside_wrapper_init: bool,
    pub at_module_level: bool,
    pub current_module: Option<&'a str>,
    /// Cached set of symbols used in the current function scope (if available)
    pub current_function_used_symbols: Option<&'a FxIndexSet<String>>,
}

/// Bundler orchestrates the code generation phase of bundling
pub(crate) struct Bundler<'a> {
    /// Map from module ID to synthetic name for wrapper modules
    pub(crate) module_synthetic_names: FxIndexMap<ModuleId, String>,
    /// Map from module ID to init function name (for wrapper modules)
    pub(crate) module_init_functions: FxIndexMap<ModuleId, String>,
    /// Collected future imports
    pub(crate) future_imports: FxIndexSet<String>,
    /// Track which modules have been bundled
    pub(crate) bundled_modules: FxIndexSet<ModuleId>,
    /// Modules that were inlined (not wrapper modules)
    pub(crate) inlined_modules: FxIndexSet<ModuleId>,
    /// Modules that use wrapper functions (side effects or circular deps)
    pub(crate) wrapper_modules: FxIndexSet<ModuleId>,
    /// Entry point path for calculating relative paths
    pub(crate) entry_path: Option<String>,
    /// Entry module name
    pub(crate) entry_module_name: String,
    /// Whether the entry is __init__.py or __main__.py
    pub(crate) entry_is_package_init_or_main: bool,
    /// Module export information (for __all__ handling)
    pub(crate) module_exports: FxIndexMap<ModuleId, Option<Vec<String>>>,
    /// Semantic export information (includes re-exports from child modules)
    pub(crate) semantic_exports: FxIndexMap<ModuleId, FxIndexSet<String>>,
    /// Lifted global declarations to add at module top level
    /// Modules that are imported as namespaces (e.g., from package import module)
    /// Maps module ID to set of importing module IDs
    pub(crate) namespace_imported_modules: FxIndexMap<ModuleId, FxIndexSet<ModuleId>>,
    /// Reference to the central module registry
    pub(crate) module_info_registry: Option<&'a crate::orchestrator::ModuleRegistry>,
    /// Reference to the module resolver
    pub(crate) resolver: &'a ModuleResolver,
    /// Modules that are part of circular dependencies (may be pruned for entry package)
    pub(crate) circular_modules: FxIndexSet<ModuleId>,
    /// All modules that are part of circular dependencies (unpruned, for accurate checks)
    all_circular_modules: FxIndexSet<ModuleId>,
    /// Pre-declared symbols for circular modules (module -> symbol -> renamed)
    /// Symbol dependency graph for circular modules
    pub(crate) symbol_dep_graph: SymbolDependencyGraph,
    /// Module ASTs for resolving re-exports
    pub(crate) module_asts: Option<FxIndexMap<ModuleId, (ModModule, PathBuf, String)>>,
    /// Track all namespaces that need to be created before module initialization
    /// Runtime tracking of all created namespaces to prevent duplicates
    pub(crate) created_namespaces: FxIndexSet<String>,
    /// Track parent-child assignments that have been made to prevent duplicates
    /// Format: (parent, child) where both are module names
    pub(crate) parent_child_assignments_made: FxIndexSet<(String, String)>,
    /// Track modules that have had their symbols populated to their namespace
    /// This prevents duplicate population when modules are imported multiple times
    pub(crate) modules_with_populated_symbols: FxIndexSet<ModuleId>,
    /// Reference to the dependency graph for module relationship queries
    pub(crate) graph: Option<&'a DependencyGraph>,
    /// Modules that have explicit __all__ defined
    pub(crate) modules_with_explicit_all: FxIndexSet<ModuleId>,
    /// Transformation context for tracking node mappings
    pub(crate) transformation_context: TransformationContext,
    /// Module/symbol pairs that should be kept after tree shaking
    /// Maps module ID to set of symbols to keep in that module
    pub(crate) tree_shaking_keep_symbols: Option<FxIndexMap<ModuleId, FxIndexSet<String>>>,
    /// Track modules whose __all__ attribute is accessed in the code
    /// Set of (`accessing_module_id`, `accessed_alias`) pairs to handle alias collisions
    /// Only these modules need their __all__ emitted in the bundle
    pub(crate) modules_with_accessed_all: FxIndexSet<(ModuleId, String)>,
    /// Global cache of all kept symbols for O(1) lookup
    /// Populated from `tree_shaking_keep_symbols` for efficient symbol existence checks
    pub(crate) kept_symbols_global: Option<FxIndexSet<String>>,
    /// Reference to the symbol conflict resolver for detecting and resolving name conflicts
    /// This is set during bundling and provides access to symbol renames and conflict information
    pub(crate) conflict_resolver:
        Option<&'a crate::symbol_conflict_resolver::SymbolConflictResolver>,
    /// Track which wrapper modules have had their init function emitted (definition + assignment)
    pub(crate) emitted_wrapper_inits: FxIndexSet<ModuleId>,
}

impl std::fmt::Debug for Bundler<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundler")
            .field("module_synthetic_names", &self.module_synthetic_names)
            .field("entry_module_name", &self.entry_module_name)
            .field("bundled_modules", &self.bundled_modules)
            .field("inlined_modules", &self.inlined_modules)
            .finish()
    }
}

/// Parameters for resolving import value expressions
pub(in crate::code_generator) struct ImportResolveParams<'a> {
    pub(in crate::code_generator) module_expr: Expr,
    pub(in crate::code_generator) module_name: &'a str,
    pub(in crate::code_generator) imported_name: &'a str,
    pub(in crate::code_generator) at_module_level: bool,
    pub(in crate::code_generator) inside_wrapper_init: bool,
    pub(in crate::code_generator) current_module: Option<&'a str>,
    pub(in crate::code_generator) symbol_renames:
        &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
}

// Main implementation
impl<'a> Bundler<'a> {
    /// Helper: resolve a relative import target to an absolute module name
    fn resolve_from_import_target(
        &self,
        module_name: &str,
        from_module: &str,
        level: u32,
    ) -> String {
        if level == 0 {
            return from_module.to_owned();
        }

        // Determine the path of the current module for proper relative resolution
        let module_path = self.get_module_id(module_name).and_then(|id| {
            self.module_asts
                .as_ref()
                .and_then(|asts| asts.get(&id).map(|(_, path, _)| path.clone()))
        });

        let fallback = || {
            let clean = from_module.trim_start_matches('.');
            if clean.is_empty() {
                module_name.to_owned()
            } else {
                format!("{module_name}.{clean}")
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

    // (removed) entry_directly_imports_module, build_namespace_all_assignment: dead code

    /// Helper: collect entry stdlib alias names from a `from` import
    pub(crate) fn collect_aliases_from_stdlib_from_import(
        &self,
        import_from: &StmtImportFrom,
        python_version: u8,
        entry_stdlib_aliases: &mut FxIndexMap<String, String>,
    ) {
        if import_from.level != 0 {
            return;
        }
        let Some(module) = &import_from.module else {
            return;
        };
        let module_str = module.as_str();
        if module_str == "__future__" {
            return;
        }

        let root = module_str.split('.').next().unwrap_or(module_str);
        if !ruff_python_stdlib::sys::is_known_standard_library(python_version, root) {
            return;
        }

        for alias in &import_from.names {
            if let Some(asname) = &alias.asname {
                entry_stdlib_aliases.insert(asname.as_str().to_owned(), module_str.to_owned());
            }
        }
    }

    /// Helper: does this `Assign` target a locally defined symbol (simple name target)?
    pub(crate) fn is_import_for_local_symbol(
        assign: &StmtAssign,
        locals: &FxIndexSet<String>,
    ) -> bool {
        if assign.targets.len() != 1 {
            return false;
        }
        match &assign.targets[0] {
            Expr::Name(target) => locals.contains(target.id.as_str()),
            _ => false,
        }
    }

    /// Helper: check duplicate name assignment exists in final body
    pub(crate) fn is_duplicate_name_assignment(assign: &StmtAssign, final_body: &[Stmt]) -> bool {
        let Expr::Name(target) = &assign.targets[0] else {
            return false;
        };
        final_body.iter().any(|stmt| {
            let Stmt::Assign(existing) = stmt else {
                return false;
            };
            if existing.targets.len() != 1 {
                return false;
            }
            if let Expr::Name(existing_target) = &existing.targets[0] {
                existing_target.id == target.id
                    && expression_handlers::expr_equals(&existing.value, &assign.value)
            } else {
                false
            }
        })
    }

    /// Helper: check duplicate module init attribute assignment exists in final body
    pub(crate) fn is_duplicate_module_init_attr_assignment(
        assign: &StmtAssign,
        final_body: &[Stmt],
    ) -> bool {
        let Expr::Attribute(target_attr) = &assign.targets[0] else {
            return false;
        };
        let Expr::Call(call) = &assign.value.as_ref() else {
            return false;
        };
        let Expr::Name(func_name) = &call.func.as_ref() else {
            return false;
        };
        if !is_init_function(func_name.id.as_str()) {
            return false;
        }

        let target_path = expression_handlers::extract_attribute_path(target_attr);
        final_body.iter().any(|stmt| {
            if let Stmt::Assign(existing) = stmt
                && existing.targets.len() == 1
                && let Expr::Attribute(existing_attr) = &existing.targets[0]
                && let Expr::Call(existing_call) = &existing.value.as_ref()
                && let Expr::Name(existing_func) = &existing_call.func.as_ref()
                && is_init_function(existing_func.id.as_str())
            {
                let existing_path = expression_handlers::extract_attribute_path(existing_attr);
                return existing_path == target_path;
            }
            false
        })
    }

    /// Check if a simple module attribute assignment already exists in the body
    /// Only considers it a duplicate if both the target path AND value are the same
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
    fn handle_nonconditional_from_import_exports(
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

    // (removed) add_all_if_accessed: dead code

    // (moved) handle_wildcard_import_from_multiple -> import_transformer::handlers::wrapper

    /// Helper to get module ID from name during transition
    pub(crate) fn get_module_id(&self, module_name: &str) -> Option<ModuleId> {
        self.resolver.get_module_id_by_name(module_name)
    }

    /// Check if a module has a synthetic name (i.e., is a wrapper module)
    pub(crate) fn has_synthetic_name(&self, module_name: &str) -> bool {
        self.get_module_id(module_name)
            .is_some_and(|id| self.module_synthetic_names.contains_key(&id))
    }

    /// Check if a symbol is kept by tree shaking
    pub(crate) fn is_symbol_kept_by_tree_shaking(
        &self,
        module_id: ModuleId,
        symbol_name: &str,
    ) -> bool {
        self.tree_shaking_keep_symbols
            .as_ref()
            .is_none_or(|kept_symbols| {
                kept_symbols
                    .get(&module_id)
                    .is_some_and(|symbols| symbols.contains(symbol_name))
            })
    }

    /// Get the entry package name when entry is a package __init__.py
    /// Returns None if entry is not a package __init__.py
    #[inline]
    pub(crate) fn entry_package_name(&self) -> Option<&str> {
        if crate::util::is_init_module(&self.entry_module_name) {
            // Strip the .__init__ suffix if present, otherwise return None
            // Note: if entry is bare "__init__", we don't have the package name
            self.entry_module_name
                .strip_suffix(&format!(".{}", crate::python::constants::INIT_STEM))
        } else {
            None
        }
    }

    /// Infer the root package name for the entry when the entry module name alone is insufficient.
    /// This handles the case where the entry module name is just "__init__" and we need to
    /// discover the package root (e.g., "requests") by scanning known modules.
    pub(crate) fn infer_entry_root_package(&self) -> Option<String> {
        // Prefer explicit strip if available
        if let Some(pkg) = self.entry_package_name() {
            return Some(pkg.to_owned());
        }

        // If the entry module name already includes a dot, use its root component
        if self.entry_module_name.contains('.') {
            return self
                .entry_module_name
                .split('.')
                .next()
                .map(ToString::to_string);
        }

        // Fallback discovery: scan known modules for a dotted name and return its root component
        // Check inlined, wrapper (synthetic), and bundled modules for robustness
        for name in self
            .inlined_modules
            .iter()
            .filter_map(|id| self.resolver.get_module_name(*id))
            .chain(
                self.module_synthetic_names
                    .keys()
                    .filter_map(|id| self.resolver.get_module_name(*id)),
            )
            .chain(
                self.bundled_modules
                    .iter()
                    .filter_map(|id| self.resolver.get_module_name(*id)),
            )
        {
            if name.contains('.') {
                if let Some(root) = name.split('.').next()
                    && !root.is_empty()
                    && root != crate::python::constants::INIT_STEM
                {
                    return Some(root.to_owned());
                }
            } else if name != crate::python::constants::INIT_STEM {
                // Single-name module that's not __init__ can serve as the root
                return Some(name);
            }
        }

        None
    }

    /// Create a new bundler instance
    pub(crate) fn new(
        module_info_registry: Option<&'a crate::orchestrator::ModuleRegistry>,
        resolver: &'a ModuleResolver,
    ) -> Self {
        Self {
            module_synthetic_names: FxIndexMap::default(),
            module_init_functions: FxIndexMap::default(),
            future_imports: FxIndexSet::default(),
            bundled_modules: FxIndexSet::default(),
            inlined_modules: FxIndexSet::default(),
            wrapper_modules: FxIndexSet::default(),
            entry_path: None,
            entry_module_name: String::new(),
            entry_is_package_init_or_main: false,
            module_exports: FxIndexMap::default(),
            semantic_exports: FxIndexMap::default(),
            namespace_imported_modules: FxIndexMap::default(),
            module_info_registry,
            resolver,
            circular_modules: FxIndexSet::default(),
            all_circular_modules: FxIndexSet::default(),
            symbol_dep_graph: SymbolDependencyGraph::default(),
            module_asts: None,
            created_namespaces: FxIndexSet::default(),
            parent_child_assignments_made: FxIndexSet::default(),
            modules_with_populated_symbols: FxIndexSet::default(),
            graph: None,
            modules_with_explicit_all: FxIndexSet::default(),
            transformation_context: TransformationContext::new(),
            tree_shaking_keep_symbols: None,
            modules_with_accessed_all: FxIndexSet::default(),
            kept_symbols_global: None,
            conflict_resolver: None,
            emitted_wrapper_inits: FxIndexSet::default(),
        }
    }

    /// Create a new node with a proper index from the transformation context
    pub(crate) fn create_node_index(&self) -> AtomicNodeIndex {
        self.transformation_context.create_node_index()
    }

    /// Create a new node and record it as a transformation
    pub(super) fn create_transformed_node(&mut self, reason: String) -> AtomicNodeIndex {
        self.transformation_context.create_new_node(reason)
    }

    /// Transform bundled import-from with explicit context (wrapper modules)
    ///
    /// Dispatches to wildcard or symbol import handlers while preserving context flags.
    pub(super) fn transform_bundled_import_from_multiple_with_current_module(
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

    // (moved) transform_bundled_import_from_multiple_with_current_module ->
    // crate::code_generator::import_transformer::handlers::wrapper::WrapperHandler::rewrite_from_import_for_wrapper_module_with_context

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

    /// Collect module renames from semantic analysis
    fn collect_module_renames(
        &mut self,
        module_id: ModuleId,
        semantic_ctx: &SemanticContext<'_>,
        symbol_renames: &mut FxIndexMap<ModuleId, FxIndexMap<String, String>>,
    ) {
        let module_name = self
            .resolver
            .get_module_name(module_id)
            .expect("Module name must exist for ModuleId");
        log::debug!("collect_module_renames: Processing module '{module_name}'");

        // Get the module from the dependency graph
        if semantic_ctx.graph.get_module(module_id).is_none() {
            log::warn!("Module '{module_name}' not found in graph");
            return;
        }

        log::debug!("Module '{module_name}' has ID: {module_id:?}");

        // Get all renames for this module from semantic analysis
        let mut module_renames = FxIndexMap::default();

        // Use ModuleSemanticInfo to get ALL exported symbols from the module
        if let Some(module_info) = semantic_ctx.conflict_resolver.get_module_info(module_id) {
            log::debug!(
                "Module '{}' exports {} symbols: {:?}",
                module_name,
                module_info.exported_symbols.len(),
                module_info.exported_symbols.iter().collect::<Vec<_>>()
            );

            // Store semantic exports for later use
            self.semantic_exports
                .insert(module_id, module_info.exported_symbols.clone());

            // Process all exported symbols from the module
            for symbol in &module_info.exported_symbols {
                // Check if this symbol is actually a submodule
                let full_submodule_path = format!("{module_name}.{symbol}");
                if self
                    .get_module_id(&full_submodule_path)
                    .is_some_and(|id| self.bundled_modules.contains(&id))
                {
                    // This is a submodule - but we still need it in the rename map for namespace
                    // population Mark it specially so we know it's a submodule
                    log::debug!(
                        "Symbol '{symbol}' in module '{module_name}' is a submodule - will need \
                         special handling"
                    );
                }

                if let Some(new_name) = semantic_ctx.symbol_registry.get_rename(module_id, symbol) {
                    module_renames.insert(symbol.clone(), new_name.to_owned());
                    log::debug!(
                        "Module '{module_name}': symbol '{symbol}' renamed to '{new_name}'"
                    );
                } else {
                    // Don't add non-renamed symbols to the rename map
                    // They'll be handled differently in namespace population
                    log::debug!(
                        "Module '{module_name}': symbol '{symbol}' has no rename, skipping rename \
                         map"
                    );
                }
            }
        } else {
            log::warn!("No semantic info found for module '{module_name}' with ID {module_id:?}");
        }

        // For inlined modules with __all__, we need to also include symbols from __all__
        // even if they're not defined in this module (they might be re-exports)
        if self
            .get_module_id(&module_name)
            .is_some_and(|id| self.inlined_modules.contains(&id))
        {
            log::debug!("Module '{module_name}' is inlined, checking for __all__ exports");
            if let Some(export_info) = self
                .get_module_id(&module_name)
                .and_then(|id| self.module_exports.get(&id))
            {
                log::debug!("Module '{module_name}' export info: {export_info:?}");
                if let Some(all_exports) = export_info {
                    log::debug!(
                        "Module '{}' has __all__ with {} exports: {:?}",
                        module_name,
                        all_exports.len(),
                        all_exports
                    );

                    // Add any symbols from __all__ that aren't already in module_renames
                    for export in all_exports {
                        if !module_renames.contains_key(export) {
                            // Check if this is actually a submodule
                            let full_submodule_path = format!("{module_name}.{export}");
                            if self
                                .get_module_id(&full_submodule_path)
                                .is_some_and(|id| self.bundled_modules.contains(&id))
                            {
                                log::debug!(
                                    "Module '{module_name}': skipping export '{export}' from \
                                     __all__ - it's a submodule, not a symbol"
                                );
                                continue;
                            }

                            // This is a re-exported symbol - use the original name
                            module_renames.insert(export.clone(), export.clone());
                            log::debug!(
                                "Module '{module_name}': adding re-exported symbol '{export}' \
                                 from __all__"
                            );
                        }
                    }
                }
            }
        }

        // Store the renames for this module
        symbol_renames.insert(module_id, module_renames);
    }

    /// Build a map of imported symbols to their source modules by analyzing import statements
    pub(crate) fn build_import_source_map(
        &self,
        statements: &[Stmt],
        module_name: &str,
    ) -> FxIndexMap<String, String> {
        let mut import_sources = FxIndexMap::default();

        for stmt in statements {
            if let Stmt::ImportFrom(import_from) = stmt
                && let Some(module) = &import_from.module
            {
                let source_module = module.as_str();

                // Only track imports from first-party modules that were inlined
                if self.get_module_id(source_module).is_some_and(|id| {
                    self.inlined_modules.contains(&id) || self.bundled_modules.contains(&id)
                }) {
                    for alias in &import_from.names {
                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                        // Map the local name to its source module
                        import_sources.insert(local_name.to_owned(), source_module.to_owned());

                        log::debug!(
                            "Module '{module_name}': Symbol '{local_name}' imported from \
                             '{source_module}'"
                        );
                    }
                }
            }
        }

        import_sources
    }

    /// Process entry module statement
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

    /// Initialize the bundler with parameters and basic settings
    pub(crate) fn initialize_bundler(&mut self, params: &BundleParams<'a>) {
        // Store tree shaking decisions if provided
        if let Some(shaker) = params.tree_shaker {
            // Extract all kept symbols from the tree shaker
            let mut kept_symbols: FxIndexMap<ModuleId, FxIndexSet<String>> = FxIndexMap::default();
            for (module_id, _, _) in params.modules {
                let module_name = params
                    .resolver
                    .get_module_name(*module_id)
                    .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
                let module_symbols = shaker.get_used_symbols_for_module(&module_name);
                if !module_symbols.is_empty() {
                    kept_symbols.insert(*module_id, module_symbols);
                }
            }
            self.tree_shaking_keep_symbols = Some(kept_symbols);
            log::debug!(
                "Tree shaking enabled, keeping symbols in {} modules",
                self.tree_shaking_keep_symbols
                    .as_ref()
                    .map_or(0, indexmap::IndexMap::len)
            );

            // Populate global cache of all kept symbols for O(1) lookup
            if let Some(ref kept_by_module) = self.tree_shaking_keep_symbols {
                // Pre-reserve capacity to avoid re-allocations
                let estimated_capacity: usize =
                    kept_by_module.values().map(indexmap::IndexSet::len).sum();
                let mut all_kept = FxIndexSet::default();
                all_kept.reserve(estimated_capacity);

                for symbols in kept_by_module.values() {
                    // Strings are already owned; clone to populate the global set
                    all_kept.extend(symbols.iter().cloned());
                }
                // Do not include extra symbols here; __all__ handling occurs elsewhere
                log::debug!(
                    "Populated global kept symbols cache with {} unique symbols",
                    all_kept.len()
                );
                self.kept_symbols_global = Some(all_kept);
            }
        }

        // Extract modules that access __all__ from the pre-computed graph data
        // Store (accessing_module_id, accessed_module_name) pairs to handle alias collisions
        for &(accessing_module_id, accessed_module_id) in params.graph.get_modules_accessing_all() {
            // Get the accessed module's name for the alias tracking
            if let Some(accessed_module_info) = self.resolver.get_module(accessed_module_id) {
                self.modules_with_accessed_all
                    .insert((accessing_module_id, accessed_module_info.name.clone()));
                log::debug!(
                    "Module ID {:?} accesses {}.__all__ (ID {:?})",
                    accessing_module_id,
                    accessed_module_info.name,
                    accessed_module_id
                );
            }
        }

        // Get entry module name from resolver
        let entry_module_name = params
            .resolver
            .get_module_name(ModuleId::ENTRY)
            .unwrap_or_else(|| "main".to_owned());

        log::debug!("Entry module name: {entry_module_name}");
        log::debug!(
            "Module names in modules vector: {:?}",
            params
                .modules
                .iter()
                .map(|(id, _, _)| params
                    .resolver
                    .get_module_name(*id)
                    .unwrap_or_else(|| format!("module_{}", id.as_u32())))
                .collect::<Vec<_>>()
        );

        // Store entry module information
        self.entry_module_name = entry_module_name;

        // Check if entry is a package using resolver
        self.entry_is_package_init_or_main = params.resolver.is_entry_package()
            || params
                .resolver
                .get_module_path(ModuleId::ENTRY)
                .is_some_and(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name == crate::python::constants::MAIN_FILE)
                });

        log::debug!(
            "Entry is package init or main: {}",
            self.entry_is_package_init_or_main
        );

        // First pass: collect future imports from ALL modules before trimming
        // This ensures future imports are hoisted even if they appear late in the file
        for (_module_id, ast, _) in params.modules {
            let future_imports = ImportAnalyzer::collect_future_imports(ast);
            self.future_imports.extend(future_imports);
        }

        // Store entry path for relative path calculation
        if let Some(entry_path) = params.resolver.get_module_path(ModuleId::ENTRY) {
            self.entry_path = Some(entry_path.to_string_lossy().to_string());
        }
    }

    /// Collect symbol renames from semantic analysis
    pub(crate) fn collect_symbol_renames(
        &mut self,
        modules: &FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
        semantic_ctx: &SemanticContext<'_>,
    ) -> FxIndexMap<ModuleId, FxIndexMap<String, String>> {
        let mut symbol_renames = FxIndexMap::default();

        // Collect renames for each module
        for module_id in modules.keys() {
            self.collect_module_renames(*module_id, semantic_ctx, &mut symbol_renames);
        }

        symbol_renames
    }

    /// Prepare modules by trimming imports, indexing ASTs, and detecting circular dependencies
    pub(crate) fn prepare_modules(
        &mut self,
        params: &BundleParams<'a>,
    ) -> FxIndexMap<ModuleId, (ModModule, PathBuf, String)> {
        // Identify all modules that are part of circular dependencies FIRST
        // This must be done before trimming imports
        if let Some(analysis) = params.circular_dep_analysis {
            log::debug!("CircularDependencyAnalysis received:");
            log::debug!("  Resolvable cycles: {:?}", analysis.resolvable_cycles);
            log::debug!("  Unresolvable cycles: {:?}", analysis.unresolvable_cycles);
            for group in &analysis.resolvable_cycles {
                for &module_id in &group.modules {
                    self.circular_modules.insert(module_id);
                    self.all_circular_modules.insert(module_id);
                }
            }
            for group in &analysis.unresolvable_cycles {
                for &module_id in &group.modules {
                    self.circular_modules.insert(module_id);
                    self.all_circular_modules.insert(module_id);
                }
            }
            log::debug!("Circular modules: {:?}", self.circular_modules);
        }

        // Convert modules to the format expected by functions
        let modules_with_paths: Vec<(ModuleId, ModModule, PathBuf, String)> = params
            .modules
            .iter()
            .map(|(id, ast, hash)| {
                let path = params.resolver.get_module_path(*id).unwrap_or_else(|| {
                    let name = params
                        .resolver
                        .get_module_name(*id)
                        .unwrap_or_else(|| format!("module_{}", id.as_u32()));
                    PathBuf::from(&name)
                });
                (*id, ast.clone(), path, hash.clone())
            })
            .collect();

        // Convert to IndexMap first for efficient lookups
        let mut modules_map: FxIndexMap<ModuleId, (ModModule, PathBuf, String)> =
            FxIndexMap::default();
        for (module_id, ast, path, hash) in modules_with_paths {
            modules_map.insert(module_id, (ast, path, hash));
        }

        // Trim unused imports from all modules
        // Note: stdlib import normalization now happens in the orchestrator
        // before dependency graph building, so imports are already normalized
        let mut modules = import_deduplicator::trim_unused_imports_from_modules(
            &modules_map,
            params.graph,
            params.tree_shaker,
            params.python_version,
            &self.circular_modules,
        );

        // Index all module ASTs to assign node indices and initialize transformation context
        log::debug!("Indexing {} modules", modules.len());
        let mut total_nodes = 0_u32;
        let mut module_id_counter = 0_u32;

        // Create a mapping from module ID to counter for debugging
        let mut module_id_map = FxIndexMap::default();

        for (module_id, (ast, _, _content_hash)) in &mut modules {
            let indexed = crate::ast_indexer::index_module_with_id(ast, module_id_counter);
            let node_count = indexed.node_count;
            let module_name = self
                .resolver
                .get_module_name(*module_id)
                .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
            log::debug!(
                "Module {} (ID: {}) indexed with {} nodes (indices {}-{})",
                module_name,
                module_id_counter,
                node_count,
                module_id_counter * crate::ast_indexer::MODULE_INDEX_RANGE,
                module_id_counter * crate::ast_indexer::MODULE_INDEX_RANGE + node_count - 1
            );
            module_id_map.insert(*module_id, module_id_counter);
            total_nodes += node_count;
            module_id_counter += 1;
        }

        // Initialize transformation context
        // Start new node indices after all module ranges
        self.transformation_context = TransformationContext::new();
        let starting_index = module_id_counter * crate::ast_indexer::MODULE_INDEX_RANGE;
        for _ in 0..starting_index {
            self.transformation_context.next_node_index();
        }
        log::debug!(
            "Transformation context initialized. Module count: {module_id_counter}, Total nodes: \
             {total_nodes}, New nodes start at: {starting_index}"
        );

        // Store for re-export resolution (modules already use ModuleId)
        self.module_asts = Some(modules.clone());

        // Track bundled modules
        for module_id in modules.keys() {
            self.bundled_modules.insert(*module_id);
            let module_name = self
                .resolver
                .get_module_name(*module_id)
                .expect("Module name must exist for ModuleId");
            log::debug!("Tracking bundled module: '{module_name}' (ID: {module_id:?})");
        }

        // Check which modules are imported directly (e.g., import module_name)
        let directly_imported_modules =
            self.find_directly_imported_modules(&modules, &self.entry_module_name);
        log::debug!("Directly imported modules: {directly_imported_modules:?}");

        // Find modules that are imported as namespaces (e.g., from models import base)
        // The modules vector already contains all modules including the entry module
        self.find_namespace_imported_modules(&modules);

        // Note: Circular dependencies have already been identified at the beginning of
        // prepare_modules to ensure imports are properly handled before tree-shaking
        if params.circular_dep_analysis.is_some() {
            // If entry module is __init__.py, also remove the entry package from circular modules
            // For example, if entry is "yaml.__init__" and "yaml" is in circular modules, remove
            // "yaml" as they're the same file (yaml/__init__.py)
            if self.entry_is_package_init_or_main
                && let Some(entry_pkg) = self.entry_package_name()
            {
                let entry_pkg = entry_pkg.to_owned(); // Convert to owned string to avoid borrow issues
                // Remove the specific entry package from circular modules
                if self
                    .get_module_id(&entry_pkg)
                    .is_some_and(|id| self.circular_modules.contains(&id))
                {
                    log::debug!(
                        "Removing package '{entry_pkg}' from circular modules as it's the same as \
                         entry module '__init__.py'"
                    );
                    if let Some(id) = self.get_module_id(&entry_pkg) {
                        self.circular_modules.swap_remove(&id);
                    }
                }
            }
        }

        modules
    }

    /// Get the rewritten path for a stdlib module (e.g., "json" -> "_cribo.json")
    pub(crate) fn get_rewritten_stdlib_path(module_name: &str) -> String {
        format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX)
    }

    /// Find modules that are imported directly
    pub(super) fn find_directly_imported_modules(
        &self,
        modules: &FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
        entry_module_name: &str,
    ) -> FxIndexSet<String> {
        // Convert to old format temporarily for ImportAnalyzer
        let modules_with_names: Vec<(String, ModModule, PathBuf, String)> = modules
            .iter()
            .map(|(id, (ast, path, hash))| {
                let name = self
                    .resolver
                    .get_module_name(*id)
                    .expect("Module name must exist for ModuleId");
                (name, ast.clone(), path.clone(), hash.clone())
            })
            .collect();
        // Use ImportAnalyzer to find directly imported modules
        ImportAnalyzer::find_directly_imported_modules(&modules_with_names, entry_module_name)
    }

    /// Find modules that are imported as namespaces
    pub(crate) fn find_namespace_imported_modules(
        &mut self,
        modules: &FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
    ) {
        // Convert to old format temporarily for ImportAnalyzer
        let modules_with_names: Vec<(String, ModModule, PathBuf, String)> = modules
            .iter()
            .map(|(id, (ast, path, hash))| {
                let name = self
                    .resolver
                    .get_module_name(*id)
                    .expect("Module name must exist for ModuleId");
                (name, ast.clone(), path.clone(), hash.clone())
            })
            .collect();

        // Use ImportAnalyzer to find namespace imported modules
        let string_based = ImportAnalyzer::find_namespace_imported_modules(&modules_with_names);

        // Convert String-based map to ModuleId-based
        self.namespace_imported_modules = string_based
            .into_iter()
            .filter_map(|(module_name, imports)| {
                let module_id = self.get_module_id(&module_name)?;
                let import_ids: FxIndexSet<ModuleId> = imports
                    .into_iter()
                    .filter_map(|import_name| self.get_module_id(&import_name))
                    .collect();
                Some((module_id, import_ids))
            })
            .collect();

        log::debug!(
            "Found {} namespace imported modules: {:?}",
            self.namespace_imported_modules.len(),
            self.namespace_imported_modules
        );
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
                // Symbol is in __all__. For re-exported symbols, check if the symbol exists
                // anywhere in the bundle.
                let should_export = self
                    .kept_symbols_global
                    .as_ref()
                    .is_none_or(|kept| kept.contains(symbol_name));

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
        _ctx: &InlineContext<'_>,
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

        // For function bodies, we need special handling:
        // - Global statements must be renamed to match module-level renames
        // - But other references should NOT be renamed (Python resolves at runtime)
        // Note: This functionality was removed as stdlib normalization now happens in the
        // orchestrator

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

    // rewrite_aliases_in_stmt and rewrite_aliases_in_except_handler have been moved to
    // expression_handlers.rs

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
    fn is_type_checking_condition(expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
            Expr::Attribute(attr) => {
                attr.attr.as_str() == "TYPE_CHECKING"
                    && matches!(&*attr.value, Expr::Name(name) if name.id.as_str() == "typing")
            }
            _ => false,
        }
    }

    /// Process module body recursively to handle conditional imports
    pub(crate) fn process_body_recursive(
        &self,
        body: Vec<Stmt>,
        module_name: &str,
        module_scope_symbols: Option<&FxIndexSet<String>>,
    ) -> Vec<Stmt> {
        self.process_body_recursive_impl(body, module_name, module_scope_symbols, false)
    }

    /// Implementation of `process_body_recursive` with conditional context tracking
    fn process_body_recursive_impl(
        &self,
        body: Vec<Stmt>,
        module_name: &str,
        module_scope_symbols: Option<&FxIndexSet<String>>,
        in_conditional_context: bool,
    ) -> Vec<Stmt> {
        let mut result = Vec::new();

        for stmt in body {
            match &stmt {
                Stmt::If(if_stmt) => {
                    // Process if body recursively (inside conditional context)
                    let mut processed_body = self.process_body_recursive_impl(
                        if_stmt.body.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Check if this is a TYPE_CHECKING block and ensure it has a body
                    if processed_body.is_empty() && Self::is_type_checking_condition(&if_stmt.test)
                    {
                        log::debug!("Adding pass statement to empty TYPE_CHECKING block");
                        // Add a pass statement to avoid IndentationError
                        processed_body.push(statements::pass());
                    }

                    // Process elif/else clauses
                    let processed_elif_else = if_stmt
                        .elif_else_clauses
                        .iter()
                        .map(|clause| {
                            let mut processed_clause_body = self.process_body_recursive_impl(
                                clause.body.clone(),
                                module_name,
                                module_scope_symbols,
                                true,
                            );

                            // Ensure non-empty body for elif/else clauses too
                            if processed_clause_body.is_empty() {
                                log::debug!("Adding pass statement to empty elif/else clause");
                                processed_clause_body.push(statements::pass());
                            }

                            ruff_python_ast::ElifElseClause {
                                node_index: clause.node_index.clone(),
                                test: clause.test.clone(),
                                body: processed_clause_body,
                                range: clause.range,
                            }
                        })
                        .collect();

                    // Create new if statement with processed bodies
                    let new_if = ruff_python_ast::StmtIf {
                        node_index: if_stmt.node_index.clone(),
                        test: if_stmt.test.clone(),
                        body: processed_body,
                        elif_else_clauses: processed_elif_else,
                        range: if_stmt.range,
                    };

                    result.push(Stmt::If(new_if));
                }
                Stmt::Try(try_stmt) => {
                    // Process try body recursively (inside conditional context)
                    let processed_body = self.process_body_recursive_impl(
                        try_stmt.body.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Process handlers
                    let processed_handlers = try_stmt
                        .handlers
                        .iter()
                        .map(|handler| {
                            let ExceptHandler::ExceptHandler(handler) = handler;
                            let processed_handler_body = self.process_body_recursive_impl(
                                handler.body.clone(),
                                module_name,
                                module_scope_symbols,
                                true,
                            );
                            ExceptHandler::ExceptHandler(
                                ruff_python_ast::ExceptHandlerExceptHandler {
                                    node_index: handler.node_index.clone(),
                                    type_: handler.type_.clone(),
                                    name: handler.name.clone(),
                                    body: processed_handler_body,
                                    range: handler.range,
                                },
                            )
                        })
                        .collect();

                    // Process orelse (inside conditional context)
                    let processed_orelse = self.process_body_recursive_impl(
                        try_stmt.orelse.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Process finalbody (inside conditional context)
                    let processed_finalbody = self.process_body_recursive_impl(
                        try_stmt.finalbody.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Create new try statement
                    let new_try = ruff_python_ast::StmtTry {
                        node_index: try_stmt.node_index.clone(),
                        body: processed_body,
                        handlers: processed_handlers,
                        orelse: processed_orelse,
                        finalbody: processed_finalbody,
                        is_star: try_stmt.is_star,
                        range: try_stmt.range,
                    };

                    result.push(Stmt::Try(new_try));
                }
                Stmt::ImportFrom(import_from) => {
                    // Skip __future__ imports
                    if import_from
                        .module
                        .as_ref()
                        .map(ruff_python_ast::Identifier::as_str)
                        != Some("__future__")
                    {
                        // Check if this is a relative import that needs special handling
                        // Skip wildcard cases to preserve semantics
                        let has_wildcard = import_from.names.iter().any(|a| a.name.as_str() == "*");
                        let handled = if import_from.level > 0 && !has_wildcard {
                            // For relative imports, transform same-module case to explicit
                            // assignments
                            let from_mod = import_from
                                .module
                                .as_ref()
                                .map_or("", ruff_python_ast::Identifier::as_str);
                            let resolved = self.resolve_from_import_target(
                                module_name,
                                from_mod,
                                import_from.level,
                            );
                            if resolved == module_name {
                                let parent_pkg = self.derive_parent_package_for_relative_import(
                                    module_name,
                                    import_from.level,
                                );
                                crate::code_generator::import_transformer::handlers::relative::transform_relative_import_aliases(
                                    self,
                                    import_from,
                                    &parent_pkg, // correct parent package
                                    module_name, // current module
                                    &mut result,
                                    true,        // add module attributes
                                );
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if handled {
                            // Helper emitted the local bindings and module attrs; skip fall-through
                            // to avoid duplicates
                            continue;
                        }
                        result.push(stmt.clone());

                        // Add module attribute assignments for imported symbols when in conditional
                        // context
                        if in_conditional_context {
                            for alias in &import_from.names {
                                let local_name =
                                    alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                                log::debug!(
                                    "Checking conditional ImportFrom symbol '{local_name}' in \
                                     module '{module_name}' for export"
                                );

                                // For conditional imports, always add module attributes for
                                // non-private symbols regardless of
                                // __all__ restrictions, since they can be defined at runtime
                                if local_name.starts_with('_') {
                                    log::debug!(
                                        "NOT exporting conditional ImportFrom symbol \
                                         '{local_name}' in module '{module_name}' (private symbol)"
                                    );
                                } else {
                                    log::debug!(
                                        "Adding module.{local_name} = {local_name} after \
                                         conditional import (bypassing __all__ restrictions)"
                                    );
                                    let module_var =
                                        sanitize_module_name_for_identifier(module_name);
                                    result.push(
                                        crate::code_generator::module_registry::create_module_attr_assignment(
                                            &module_var,
                                            local_name,
                                        ),
                                    );
                                }
                            }
                        } else {
                            // Non-conditional imports
                            self.handle_nonconditional_from_import_exports(
                                import_from,
                                module_scope_symbols,
                                module_name,
                                &mut result,
                            );
                        }
                    }
                }
                Stmt::Import(import_stmt) => {
                    // Add the import statement itself
                    result.push(stmt.clone());

                    // Add module attribute assignments for imported modules when in conditional
                    // context
                    if in_conditional_context {
                        for alias in &import_stmt.names {
                            let imported_name = alias.name.as_str();
                            let local_name = alias
                                .asname
                                .as_ref()
                                .map_or(imported_name, ruff_python_ast::Identifier::as_str);

                            // For conditional imports, always add module attributes for non-private
                            // symbols regardless of __all__
                            // restrictions, since they can be defined at runtime
                            // Only handle simple (non-dotted) names that can be valid attribute
                            // names
                            if !local_name.starts_with('_')
                                && !local_name.contains('.')
                                && !local_name.is_empty()
                                && !local_name.as_bytes()[0].is_ascii_digit()
                                && local_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                            {
                                log::debug!(
                                    "Adding module.{local_name} = {local_name} after conditional \
                                     import (bypassing __all__ restrictions)"
                                );
                                let module_var = sanitize_module_name_for_identifier(module_name);
                                result.push(
                                    crate::code_generator::module_registry::create_module_attr_assignment(
                                        &module_var,
                                        local_name
                                    ),
                                );
                            } else {
                                log::debug!(
                                    "NOT exporting conditional Import symbol '{local_name}' in \
                                     module '{module_name}' (complex or invalid attribute name)"
                                );
                            }
                        }
                    }
                }
                Stmt::Assign(assign) => {
                    // Add the assignment itself
                    result.push(stmt.clone());

                    // Check if this assignment should create a module attribute when in conditional
                    // context
                    if in_conditional_context
                        && let Some(name) =
                            expression_handlers::extract_simple_assign_target(assign)
                    {
                        // For conditional assignments, always add module attributes for non-private
                        // symbols regardless of __all__ restrictions, since
                        // they can be defined at runtime
                        if !name.starts_with('_') {
                            log::debug!(
                                "Adding module.{name} = {name} after conditional assignment \
                                 (bypassing __all__ restrictions)"
                            );
                            let module_var = sanitize_module_name_for_identifier(module_name);
                            result.push(
                                crate::code_generator::module_registry::create_module_attr_assignment(
                                    &module_var,
                                    &name
                                ),
                            );
                        }
                    }
                }
                _ => {
                    // For other statements, just add them as-is
                    result.push(stmt.clone());
                }
            }
        }

        result
    }

    /// Transform nested functions to use module attributes for module-level variables,
    /// including lifted variables (they access through module attrs unless they declare global)
    pub(crate) fn transform_nested_function_for_module_vars_with_global_info(
        &self,
        func_def: &mut StmtFunctionDef,
        module_level_vars: &FxIndexSet<String>,
        global_declarations: &FxIndexMap<String, Vec<TextRange>>,
        lifted_names: Option<&FxIndexMap<String, String>>,
        module_var_name: &str,
    ) {
        // First, collect all names in this function scope that must NOT be rewritten
        // (globals declared here or nonlocals captured from an outer function)
        let mut global_vars = FxIndexSet::default();

        // Build a reverse map for lifted names to avoid O(n) scans per name
        let lifted_to_original: Option<FxIndexMap<String, String>> = lifted_names.map(|m| {
            m.iter()
                .map(|(orig, lift)| (lift.clone(), orig.clone()))
                .collect()
        });

        for stmt in &func_def.body {
            if let Stmt::Global(global_stmt) = stmt {
                for name in &global_stmt.names {
                    let var_name = name.to_string();

                    // The global statement might have already been rewritten to use lifted names
                    // (e.g., "_cribo_httpx__transports_default_HTTPCORE_EXC_MAP")
                    // We need to check both the lifted name AND the original name

                    // First check if this is directly a global declaration
                    if global_declarations.contains_key(&var_name) {
                        global_vars.insert(var_name.clone());
                    }

                    // Also check if this is a lifted name via reverse lookup
                    if let Some(rev) = &lifted_to_original
                        && let Some(original_name) = rev.get(var_name.as_str())
                    {
                        // Exclude both original and lifted names from transformation
                        global_vars.insert(original_name.clone());
                        global_vars.insert(var_name.clone());
                    }
                }
            } else if let Stmt::Nonlocal(nonlocal_stmt) = stmt {
                // Nonlocals are not module-level; exclude them from module attribute rewrites
                for name in &nonlocal_stmt.names {
                    global_vars.insert(name.to_string());
                }
            }
        }

        // Now transform the function, but skip variables that are declared as global
        // Create a modified set of module_level_vars that excludes the global vars
        let mut filtered_module_vars = module_level_vars.clone();
        for global_var in &global_vars {
            filtered_module_vars.swap_remove(global_var);
        }

        // Transform using the filtered set
        self.transform_nested_function_for_module_vars(
            func_def,
            &filtered_module_vars,
            module_var_name,
        );
    }

    /// Transform nested functions to use module attributes for module-level variables
    pub(crate) fn transform_nested_function_for_module_vars(
        &self,
        func_def: &mut StmtFunctionDef,
        module_level_vars: &FxIndexSet<String>,
        module_var_name: &str,
    ) {
        // First, collect all global declarations in this function
        let mut global_vars = FxIndexSet::default();
        for stmt in &func_def.body {
            if let Stmt::Global(global_stmt) = stmt {
                for name in &global_stmt.names {
                    global_vars.insert(name.to_string());
                }
            }
        }

        // Collect local variables defined in this function
        let mut local_vars = FxIndexSet::default();

        // Add function parameters to local variables
        for param in &func_def.parameters.args {
            local_vars.insert(param.parameter.name.to_string());
        }
        for param in &func_def.parameters.posonlyargs {
            local_vars.insert(param.parameter.name.to_string());
        }
        for param in &func_def.parameters.kwonlyargs {
            local_vars.insert(param.parameter.name.to_string());
        }
        if let Some(ref vararg) = func_def.parameters.vararg {
            local_vars.insert(vararg.name.to_string());
        }
        if let Some(ref kwarg) = func_def.parameters.kwarg {
            local_vars.insert(kwarg.name.to_string());
        }

        // Collect all local variables assigned in the function body
        // Pass global_vars to exclude them from local_vars
        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&func_def.body);

        // Transform the function body, excluding local variables
        for stmt in &mut func_def.body {
            self.transform_stmt_for_module_vars_with_locals(
                stmt,
                module_level_vars,
                &local_vars,
                module_var_name,
            );
        }
    }

    /// Transform a statement with awareness of local variables
    fn transform_stmt_for_module_vars_with_locals(
        &self,
        stmt: &mut Stmt,
        module_level_vars: &FxIndexSet<String>,
        local_vars: &FxIndexSet<String>,
        module_var_name: &str,
    ) {
        match stmt {
            Stmt::FunctionDef(nested_func) => {
                // Recursively transform nested functions
                self.transform_nested_function_for_module_vars(
                    nested_func,
                    module_level_vars,
                    module_var_name,
                );
            }
            Stmt::Assign(assign) => {
                // Transform assignment targets and values
                for target in &mut assign.targets {
                    Self::transform_expr_for_module_vars_with_locals(
                        target,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                Self::transform_expr_for_module_vars_with_locals(
                    &mut assign.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Stmt::Expr(expr_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut expr_stmt.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    Self::transform_expr_for_module_vars_with_locals(
                        value,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::If(if_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut if_stmt.test,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for stmt in &mut if_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(condition) = &mut clause.test {
                        Self::transform_expr_for_module_vars_with_locals(
                            condition,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                    for stmt in &mut clause.body {
                        self.transform_stmt_for_module_vars_with_locals(
                            stmt,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                }
            }
            Stmt::For(for_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut for_stmt.target,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut for_stmt.iter,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for stmt in &mut for_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::While(while_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut while_stmt.test,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for stmt in &mut while_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for stmt in &mut while_stmt.orelse {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::Try(try_stmt) => {
                for stmt in &mut try_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for handler in &mut try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    for stmt in &mut eh.body {
                        self.transform_stmt_for_module_vars_with_locals(
                            stmt,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                }
                for stmt in &mut try_stmt.orelse {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for stmt in &mut try_stmt.finalbody {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            _ => {
                // Handle other statement types as needed
            }
        }
    }

    /// Transform an expression with awareness of local variables
    fn transform_expr_for_module_vars_with_locals(
        expr: &mut Expr,
        module_level_vars: &FxIndexSet<String>,
        local_vars: &FxIndexSet<String>,
        module_var_name: &str,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                let name_str = name_expr.id.as_str();

                // Special case: transform __name__ to module.__name__
                if name_str == "__name__" && matches!(name_expr.ctx, ExprContext::Load) {
                    // Transform __name__ -> module.__name__
                    *expr = expressions::attribute(
                        expressions::name(module_var_name, ExprContext::Load),
                        "__name__",
                        ExprContext::Load,
                    );
                }
                // If this is a module-level variable being read AND NOT a local variable AND NOT a
                // builtin, transform to module.var
                else if module_level_vars.contains(name_str)
                    && !local_vars.contains(name_str)
                    && !ruff_python_stdlib::builtins::python_builtins(u8::MAX, false)
                        .any(|b| b == name_str)
                    && matches!(name_expr.ctx, ExprContext::Load)
                {
                    // Transform foo -> module.foo
                    *expr = expressions::attribute(
                        expressions::name(module_var_name, ExprContext::Load),
                        name_str,
                        ExprContext::Load,
                    );
                }
            }
            Expr::Call(call) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut call.func,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for arg in &mut call.arguments.args {
                    Self::transform_expr_for_module_vars_with_locals(
                        arg,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for keyword in &mut call.arguments.keywords {
                    Self::transform_expr_for_module_vars_with_locals(
                        &mut keyword.value,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::BinOp(binop) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut binop.left,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut binop.right,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Dict(dict) => {
                for item in &mut dict.items {
                    if let Some(key) = &mut item.key {
                        Self::transform_expr_for_module_vars_with_locals(
                            key,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                    Self::transform_expr_for_module_vars_with_locals(
                        &mut item.value,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    Self::transform_expr_for_module_vars_with_locals(
                        elem,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::Attribute(attr) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut attr.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Subscript(subscript) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut subscript.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut subscript.slice,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            _ => {
                // Handle other expression types as needed
            }
        }
    }

    /// Transform AST to use lifted globals
    pub(crate) fn transform_ast_with_lifted_globals(
        &self,
        ast: &mut ModModule,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::symbol_conflict_resolver::ModuleGlobalInfo,
        module_name: Option<&str>,
    ) {
        // Transform all statements that use global declarations
        for stmt in &mut ast.body {
            self.transform_stmt_for_lifted_globals(
                stmt,
                lifted_names,
                global_info,
                None,
                module_name,
            );
        }
    }

    /// Transform a statement to use lifted globals
    fn transform_stmt_for_lifted_globals(
        &self,
        stmt: &mut Stmt,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::symbol_conflict_resolver::ModuleGlobalInfo,
        current_function_globals: Option<&FxIndexSet<String>>,
        module_name: Option<&str>,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Check if this function uses globals
                if global_info
                    .functions_using_globals
                    .contains(&func_def.name.to_string())
                {
                    // Collect globals declared in this function
                    let function_globals =
                        crate::visitors::VariableCollector::collect_function_globals(
                            &func_def.body,
                        );

                    // Transform the function body
                    let params = TransformFunctionParams {
                        lifted_names,
                        global_info,
                        function_globals: &function_globals,
                        module_name,
                    };
                    self.transform_function_body_for_lifted_globals(func_def, &params);
                }
            }
            Stmt::Assign(assign) => {
                // Transform assignments to use lifted names if they're in a function with global
                // declarations
                for target in &mut assign.targets {
                    expression_handlers::transform_expr_for_lifted_globals(
                        self,
                        target,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut assign.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::Expr(expr_stmt) => {
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut expr_stmt.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::If(if_stmt) => {
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut if_stmt.test,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                for stmt in &mut if_stmt.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(test_expr) = &mut clause.test {
                        expression_handlers::transform_expr_for_lifted_globals(
                            self,
                            test_expr,
                            lifted_names,
                            global_info,
                            current_function_globals,
                        );
                    }
                    for stmt in &mut clause.body {
                        self.transform_stmt_for_lifted_globals(
                            stmt,
                            lifted_names,
                            global_info,
                            current_function_globals,
                            module_name,
                        );
                    }
                }
            }
            Stmt::While(while_stmt) => {
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut while_stmt.test,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                for stmt in &mut while_stmt.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }
            }
            Stmt::For(for_stmt) => {
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut for_stmt.target,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut for_stmt.iter,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                for stmt in &mut for_stmt.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    expression_handlers::transform_expr_for_lifted_globals(
                        self,
                        value,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
            }
            Stmt::ClassDef(class_def) => {
                // Transform methods in the class that use globals
                for stmt in &mut class_def.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Transform augmented assignments to use lifted names
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut aug_assign.target,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut aug_assign.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::Try(try_stmt) => {
                // Transform try block body
                for stmt in &mut try_stmt.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }

                // Transform exception handlers
                for handler in &mut try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;

                    // Transform the exception type expression if present
                    if let Some(ref mut type_expr) = eh.type_ {
                        expression_handlers::transform_expr_for_lifted_globals(
                            self,
                            type_expr,
                            lifted_names,
                            global_info,
                            current_function_globals,
                        );
                    }

                    // Transform the handler body
                    for stmt in &mut eh.body {
                        self.transform_stmt_for_lifted_globals(
                            stmt,
                            lifted_names,
                            global_info,
                            current_function_globals,
                            module_name,
                        );
                    }
                }

                // Transform orelse block
                for stmt in &mut try_stmt.orelse {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }

                // Transform finally block
                for stmt in &mut try_stmt.finalbody {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }
            }
            _ => {
                // Other statement types handled as needed
            }
        }
    }

    /// Check if a symbol should be inlined based on export rules
    pub(crate) fn should_inline_symbol(
        &self,
        symbol_name: &str,
        module_id: ModuleId,
        module_exports_map: &FxIndexMap<ModuleId, Option<Vec<String>>>,
    ) -> bool {
        // First check tree-shaking decisions if tree-shaking is enabled
        let kept_by_tree_shaking = self.is_symbol_kept_by_tree_shaking(module_id, symbol_name);

        // Check if module has explicit __all__
        let has_explicit_all = self.modules_with_explicit_all.contains(&module_id);

        // If module has explicit __all__ and symbol is not in it, don't inline it
        // even if tree-shaking kept it (it might be referenced but shouldn't be accessible)
        if has_explicit_all
            && let Some(Some(export_list)) = module_exports_map.get(&module_id)
            && !export_list.contains(&symbol_name.to_owned())
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

        // Special case: Check if this symbol is imported by a wrapper module
        // Wrapper modules need runtime access to symbols even if tree-shaking removed them
        if self.is_symbol_imported_by_wrapper(module_id, symbol_name) {
            return true;
        }

        // Symbol was removed by tree-shaking, but we may still need to keep it if:
        // 1. It's in an explicit __all__ (re-exported but not used internally)
        // 2. It's imported by other modules
        // 3. Tree-shaking is disabled and it's in the export list

        // Check if module has explicit __all__ and symbol is listed there
        if self.modules_with_explicit_all.contains(&module_id) {
            let exports = module_exports_map.get(&module_id).and_then(|e| e.as_ref());
            if let Some(export_list) = exports
                && export_list.contains(&symbol_name.to_owned())
            {
                // Symbol is in explicit __all__, keep it even if tree-shaking removed it
                // This handles the case where a symbol is re-exported but not used internally
                return true;
            }
        }

        // If tree-shaking is disabled, check export list
        if self.tree_shaking_keep_symbols.is_none() {
            let exports = module_exports_map.get(&module_id).and_then(|e| e.as_ref());
            if let Some(export_list) = exports
                && export_list.contains(&symbol_name.to_owned())
            {
                return true;
            }
        }
        if !kept_by_tree_shaking {
            let module_name = self
                .resolver
                .get_module_name(module_id)
                .unwrap_or_else(|| "<unknown>".to_owned());

            // Fallback: keep symbols that are explicitly imported by other modules.
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
            return false;
        }

        // If tree-shaking kept the symbol, check if it's in the export list
        let exports = module_exports_map.get(&module_id).and_then(|e| e.as_ref());
        if let Some(export_list) = exports {
            // Module has exports (either explicit __all__ or extracted symbols)
            // Check if the symbol is in the export list
            if export_list.contains(&symbol_name.to_owned()) {
                return true;
            }

            // Special case for circular modules: If tree-shaking kept a private symbol
            // (starts with underscore but not dunder) in a circular module,
            // it means it's explicitly imported by another module and should be included
            // even if it's not in the regular export list
            if self.is_module_in_circular_deps(module_id)
                && symbol_name.starts_with('_')
                && !symbol_name.starts_with("__")
            {
                let module_name = self
                    .resolver
                    .get_module_name(module_id)
                    .unwrap_or_else(|| "<unknown>".to_owned());
                log::debug!(
                    "Including private symbol '{symbol_name}' from circular module \
                     '{module_name}' because it's kept by tree-shaking"
                );
                return true;
            }
        }

        // No match found - either no exports or symbol not in export list
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
                    symbol_to_stmt.insert(func_def.name.to_string(), stmt);
                }
                Stmt::ClassDef(class_def) => {
                    symbol_to_stmt.insert(class_def.name.to_string(), stmt);
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
    pub(super) fn create_module_reference_assignment(
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
    pub(super) fn create_module_initialization_for_import(&self, module_id: ModuleId) -> Vec<Stmt> {
        let mut locally_initialized = FxIndexSet::default();
        self.create_module_initialization_for_import_with_tracking(
            module_id,
            &mut locally_initialized,
            None, // No current module context
            true, // At module level by default
        )
    }

    /// Create module initialization statements with current module context
    pub(super) fn create_module_initialization_for_import_with_current_module(
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
    pub(super) fn create_parent_namespaces(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
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
                    result_stmts.push(statements::simple_assign(
                        &parent_path,
                        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords),
                    ));
                }
            }
        }
    }

    /// Check if a namespace module was already created
    fn is_namespace_already_created(&self, parent_path: &str, result_stmts: &[Stmt]) -> bool {
        result_stmts.iter().any(|stmt| {
            if let Stmt::Assign(assign) = stmt
                && let Some(Expr::Name(name)) = assign.targets.first()
            {
                return name.id.as_str() == parent_path;
            }
            false
        })
    }

    /// Create all namespace objects including the leaf for a dotted import
    pub(super) fn create_all_namespace_objects(
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
    pub(super) fn create_namespace_object_for_module(
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

    /// Transform function body for lifted globals
    fn transform_function_body_for_lifted_globals(
        &self,
        func_def: &mut StmtFunctionDef,
        params: &TransformFunctionParams<'_>,
    ) {
        let mut new_body = Vec::new();

        for body_stmt in &mut func_def.body {
            if let Stmt::Global(global_stmt) = body_stmt {
                // Rewrite global statement to use lifted names
                for name in &mut global_stmt.names {
                    if let Some(lifted_name) = params.lifted_names.get(name.as_str()) {
                        *name = other::identifier(lifted_name);
                    }
                }
                new_body.push(body_stmt.clone());
            } else {
                // Transform other statements recursively with function context
                self.transform_stmt_for_lifted_globals(
                    body_stmt,
                    params.lifted_names,
                    params.global_info,
                    Some(params.function_globals),
                    params.module_name,
                );
                new_body.push(body_stmt.clone());

                // After transforming, check if we need to add synchronization
                self.add_global_sync_if_needed(
                    body_stmt,
                    params.function_globals,
                    params.lifted_names,
                    &mut new_body,
                    params.module_name,
                );
            }
        }

        // Replace function body with new body
        func_def.body = new_body;
    }

    /// Add synchronization statements for global variable modifications
    fn add_global_sync_if_needed(
        &self,
        stmt: &Stmt,
        function_globals: &FxIndexSet<String>,
        lifted_names: &FxIndexMap<String, String>,
        new_body: &mut Vec<Stmt>,
        module_name: Option<&str>,
    ) {
        match stmt {
            Stmt::Assign(assign) => {
                // Collect all names from all targets (handles simple and unpacking assignments)
                let mut all_names = Vec::new();
                for target in &assign.targets {
                    all_names.extend(
                        crate::visitors::utils::collect_names_from_assignment_target(target),
                    );
                }

                // Process each collected name
                for var_name in all_names {
                    // The variable name might already be transformed to the lifted name,
                    // so we need to check if it's a lifted variable
                    if let Some(original_name) = lifted_names
                        .iter()
                        .find(|(orig, lifted)| {
                            lifted.as_str() == var_name && function_globals.contains(orig.as_str())
                        })
                        .map(|(orig, _)| orig.as_str())
                    {
                        log::debug!(
                            "Adding sync for assignment to global {var_name}: {var_name} -> \
                             module.{original_name}"
                        );
                        // Add: module.<original_name> = <lifted_name>
                        // Use the provided module name if available, otherwise we can't sync
                        if let Some(mod_name) = module_name {
                            let module_var = sanitize_module_name_for_identifier(mod_name);
                            new_body.push(statements::assign(
                                vec![expressions::attribute(
                                    expressions::name(&module_var, ExprContext::Load),
                                    original_name,
                                    ExprContext::Store,
                                )],
                                expressions::name(var_name, ExprContext::Load),
                            ));
                        }
                    }
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Collect names from the target (though augmented assignment typically doesn't use
                // unpacking)
                let target_names = crate::visitors::utils::collect_names_from_assignment_target(
                    &aug_assign.target,
                );

                for var_name in target_names {
                    // Similar check for augmented assignments
                    if let Some(original_name) = lifted_names
                        .iter()
                        .find(|(orig, lifted)| {
                            lifted.as_str() == var_name && function_globals.contains(orig.as_str())
                        })
                        .map(|(orig, _)| orig.as_str())
                    {
                        log::debug!(
                            "Adding sync for augmented assignment to global {var_name}: \
                             {var_name} -> module.{original_name}"
                        );
                        // Add: module.<original_name> = <lifted_name>
                        // Use the provided module name if available, otherwise we can't sync
                        if let Some(mod_name) = module_name {
                            let module_var = sanitize_module_name_for_identifier(mod_name);
                            new_body.push(statements::assign(
                                vec![expressions::attribute(
                                    expressions::name(&module_var, ExprContext::Load),
                                    original_name,
                                    ExprContext::Store,
                                )],
                                expressions::name(var_name, ExprContext::Load),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Derive the parent package for a relative import at the given level.
    fn derive_parent_package_for_relative_import(&self, module_name: &str, level: u32) -> String {
        // First try to resolve using the module's actual path
        if let Some(module_id) = self.get_module_id(module_name)
            && let Some(module_asts) = &self.module_asts
            && let Some((_, path, _)) = module_asts.get(&module_id)
            && let Some(resolved) = self
                .resolver
                .resolve_relative_to_absolute_module_name(level, None, path)
        {
            return resolved;
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
