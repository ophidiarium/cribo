// File previously allowed clippy::excessive_nesting. Refactor reduced nesting instead.

mod imports;
mod symbols;
mod transforms;

use std::path::PathBuf;

use ruff_python_ast::{AtomicNodeIndex, Expr, ModModule, Stmt, StmtAssign, StmtImportFrom};

use crate::{
    analyzers::ImportAnalyzer,
    code_generator::{
        circular_deps::SymbolDependencyGraph,
        context::{BundleParams, SemanticContext},
        expression_handlers, import_deduplicator,
        module_registry::is_init_function,
    },
    dependency_graph::DependencyGraph,
    resolver::{ModuleId, ModuleResolver},
    transformation_context::TransformationContext,
    types::{FxIndexMap, FxIndexSet},
};

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

// Core implementation: construction, utilities, initialization, and module preparation.
impl<'a> Bundler<'a> {
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
            return self.entry_module_name.split('.').next().map(str::to_owned);
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
                    // Symbols defined in this module don't need identity mappings —
                    // namespace_manager resolves them via semantic_exports.
                    // (In contrast, __all__ re-exports DO need identity mappings below
                    // because they're the only signal that these foreign symbols should
                    // be included in namespace population.)
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

                            // Identity mapping: the re-exported symbol keeps its original name,
                            // but MUST appear in the rename map so that namespace_manager
                            // recognises it as inlined (it checks `renames.contains_key`).
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
            let Stmt::ImportFrom(import_from) = stmt else {
                continue;
            };

            // Resolve the source module, handling relative imports
            let source_module = if import_from.level > 0 {
                let from_mod = import_from
                    .module
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str);
                self.resolve_from_import_target(
                    module_name,
                    from_mod.unwrap_or(""),
                    import_from.level,
                )
            } else {
                let Some(module) = &import_from.module else {
                    continue;
                };
                module.as_str().to_owned()
            };

            // Only track imports from first-party modules that were inlined
            if self.get_module_id(&source_module).is_some_and(|id| {
                self.inlined_modules.contains(&id) || self.bundled_modules.contains(&id)
            }) {
                for alias in &import_from.names {
                    let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                    import_sources.insert(local_name.to_owned(), source_module.clone());

                    log::debug!(
                        "Module '{module_name}': Symbol '{local_name}' imported from \
                         '{source_module}'"
                    );
                }
            }
        }

        import_sources
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
            self.entry_path = Some(entry_path.to_string_lossy().into_owned());
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
        self.identify_circular_modules(params.circular_dep_analysis);
        let mut modules = self.build_and_trim_modules(params);
        self.index_module_asts(&mut modules);
        self.module_asts = Some(modules.clone());
        self.populate_symbol_dep_graph(&modules);
        self.track_module_relationships(&modules, params);
        modules
    }

    /// Identify all modules that are part of circular dependencies.
    /// Must be done before trimming imports.
    fn identify_circular_modules(
        &mut self,
        circular_dep_analysis: Option<&crate::analyzers::types::CircularDependencyAnalysis>,
    ) {
        let Some(analysis) = circular_dep_analysis else {
            return;
        };
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

    /// Build the module map from params and trim unused imports.
    fn build_and_trim_modules(
        &self,
        params: &BundleParams<'a>,
    ) -> FxIndexMap<ModuleId, (ModModule, PathBuf, String)> {
        // Build IndexMap directly from params.modules (AST clone is required since
        // params.modules is a borrowed slice, but we skip the intermediate Vec).
        let mut modules_map: FxIndexMap<ModuleId, (ModModule, PathBuf, String)> =
            FxIndexMap::with_capacity_and_hasher(
                params.modules.len(),
                std::hash::BuildHasherDefault::default(),
            );
        for (id, ast, hash) in params.modules {
            let path = params.resolver.get_module_path(*id).unwrap_or_else(|| {
                let name = params
                    .resolver
                    .get_module_name(*id)
                    .unwrap_or_else(|| format!("module_{}", id.as_u32()));
                PathBuf::from(&name)
            });
            modules_map.insert(*id, (ast.clone(), path, hash.clone()));
        }

        // Trim unused imports from all modules
        // Note: stdlib import normalization now happens in the orchestrator
        // before dependency graph building, so imports are already normalized
        import_deduplicator::trim_unused_imports_from_modules(
            &modules_map,
            params.graph,
            params.tree_shaker,
            params.python_version,
            &self.circular_modules,
        )
    }

    /// Index all module ASTs to assign node indices and initialize transformation context.
    fn index_module_asts(
        &mut self,
        modules: &mut FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
    ) {
        log::debug!("Indexing {} modules", modules.len());
        let mut total_nodes = 0_u32;
        let mut module_id_counter = 0_u32;
        let mut module_id_map = FxIndexMap::default();

        for (module_id, (ast, _, _content_hash)) in modules.iter_mut() {
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

        // Initialize transformation context — start new node indices after all module ranges
        self.transformation_context = TransformationContext::new();
        let starting_index = module_id_counter * crate::ast_indexer::MODULE_INDEX_RANGE;
        self.transformation_context.skip_to_index(starting_index);
        log::debug!(
            "Transformation context initialized. Module count: {module_id_counter}, Total nodes: \
             {total_nodes}, New nodes start at: {starting_index}"
        );
    }

    /// Populate symbol dependency graph for circular modules so that
    /// `reorder_statements_for_circular_module` can topologically sort symbols.
    ///
    /// NOTE: This data is currently unused at runtime — see the reachability
    /// comment on `reorder_statements_for_circular_module` in `symbols.rs`.
    /// The graph is populated proactively so the infrastructure is ready when
    /// the classifier is refined to allow inlining certain circular modules.
    fn populate_symbol_dep_graph(
        &mut self,
        modules: &FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
    ) {
        for module_id in &self.circular_modules {
            if let Some((ast, _, _)) = modules.get(module_id) {
                let module_name = self
                    .resolver
                    .get_module_name(*module_id)
                    .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
                self.symbol_dep_graph.populate_from_ast(&module_name, ast);
            }
        }
    }

    /// Track bundled modules, find import relationships, and clean up circular module entries.
    fn track_module_relationships(
        &mut self,
        modules: &FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
        params: &BundleParams<'a>,
    ) {
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
            ImportAnalyzer::find_directly_imported_modules(modules, self.resolver);
        log::debug!("Directly imported modules: {directly_imported_modules:?}");

        // Find modules that are imported as namespaces (e.g., from models import base)
        self.find_namespace_imported_modules(modules);

        // If entry module is __init__.py, remove the entry package from circular modules
        // (e.g., "yaml.__init__" and "yaml" are the same file yaml/__init__.py)
        if params.circular_dep_analysis.is_some()
            && self.entry_is_package_init_or_main
            && let Some(entry_pkg) = self.infer_entry_root_package()
        {
            if let Some(id) = self.get_module_id(&entry_pkg)
                && self.circular_modules.contains(&id)
            {
                log::debug!(
                    "Removing package '{entry_pkg}' from circular modules as it's the same as \
                     entry module '__init__.py'"
                );
                self.circular_modules.swap_remove(&id);
            }
        }
    }

    /// Get the rewritten path for a stdlib module (e.g., "json" -> "_cribo.json")
    pub(crate) fn get_rewritten_stdlib_path(module_name: &str) -> String {
        format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX)
    }

    /// Find modules that are imported as namespaces
    pub(crate) fn find_namespace_imported_modules(
        &mut self,
        modules: &FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
    ) {
        self.namespace_imported_modules =
            ImportAnalyzer::find_namespace_imported_modules(modules, self.resolver);

        log::debug!(
            "Found {} namespace imported modules: {:?}",
            self.namespace_imported_modules.len(),
            self.namespace_imported_modules
        );
    }
}
