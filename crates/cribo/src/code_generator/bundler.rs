#![allow(clippy::excessive_nesting)]

use std::path::PathBuf;

use ruff_python_ast::{
    AtomicNodeIndex, ExceptHandler, Expr, ExprContext, ExprName, Identifier, Keyword, ModModule,
    Stmt, StmtAssign, StmtClassDef, StmtFunctionDef, StmtImportFrom,
};
use ruff_text_size::TextRange;

// Temporary: MODULE_VAR is being phased out but still used in some transformation functions
// TODO: Refactor to pass module variable name through all transformation functions
// Note: This is only used for lifted globals sync in nested functions
// For init functions, we use SELF_PARAM from module_transformer
const MODULE_VAR: &str = "self";

use crate::{
    analyzers::{ImportAnalyzer, SymbolAnalyzer},
    ast_builder::{expressions, expressions::expr_to_dotted_name, other, statements},
    code_generator::{
        circular_deps::SymbolDependencyGraph,
        context::{
            BundleParams, HardDependency, InlineContext, ModuleTransformContext, SemanticContext,
        },
        expression_handlers, import_deduplicator,
        import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
        module_registry::{INIT_RESULT_VAR, is_init_function, sanitize_module_name_for_identifier},
        module_transformer, namespace_manager,
        namespace_manager::NamespaceInfo,
    },
    cribo_graph::CriboGraph,
    resolver::ModuleResolver,
    transformation_context::TransformationContext,
    types::{FxIndexMap, FxIndexSet},
    visitors::LocalVarCollector,
};

/// Parameters for transforming functions with lifted globals
struct TransformFunctionParams<'a> {
    lifted_names: &'a FxIndexMap<String, String>,
    global_info: &'a crate::semantic_bundler::ModuleGlobalInfo,
    function_globals: &'a FxIndexSet<String>,
}

/// Bundler orchestrates the code generation phase of bundling
pub struct Bundler<'a> {
    /// Map from original module name to synthetic module name
    pub(crate) module_registry: FxIndexMap<String, String>,
    /// Map from synthetic module name to init function name
    pub(crate) init_functions: FxIndexMap<String, String>,
    /// Collected future imports
    pub(crate) future_imports: FxIndexSet<String>,
    /// Track which modules have been bundled
    pub(crate) bundled_modules: FxIndexSet<String>,
    /// Modules that were inlined (not wrapper modules)
    pub(crate) inlined_modules: FxIndexSet<String>,
    /// Entry point path for calculating relative paths
    pub(crate) entry_path: Option<String>,
    /// Entry module name
    pub(crate) entry_module_name: String,
    /// Whether the entry is __init__.py or __main__.py
    pub(crate) entry_is_package_init_or_main: bool,
    /// Module export information (for __all__ handling)
    pub(crate) module_exports: FxIndexMap<String, Option<Vec<String>>>,
    /// Semantic export information (includes re-exports from child modules)
    pub(crate) semantic_exports: FxIndexMap<String, FxIndexSet<String>>,
    /// Lifted global declarations to add at module top level
    /// Modules that are imported as namespaces (e.g., from package import module)
    /// Maps module name to set of importing modules
    pub(crate) namespace_imported_modules: FxIndexMap<String, FxIndexSet<String>>,
    /// Reference to the central module registry
    pub(crate) module_info_registry: Option<&'a crate::orchestrator::ModuleRegistry>,
    /// Reference to the module resolver
    pub(crate) resolver: &'a ModuleResolver,
    /// Modules that are part of circular dependencies
    pub(crate) circular_modules: FxIndexSet<String>,
    /// Pre-declared symbols for circular modules (module -> symbol -> renamed)
    /// Hard dependencies that need to be hoisted
    pub(crate) hard_dependencies: Vec<HardDependency>,
    /// Symbol dependency graph for circular modules
    pub(crate) symbol_dep_graph: SymbolDependencyGraph,
    /// Module ASTs for resolving re-exports
    pub(crate) module_asts: Option<Vec<(String, ModModule, PathBuf, String)>>,
    /// Global registry of deferred imports to prevent duplication
    /// Maps (`module_name`, `symbol_name`) to the source module that deferred it
    pub(crate) global_deferred_imports: FxIndexMap<(String, String), String>,
    /// Track all namespaces that need to be created before module initialization
    /// Central registry of all namespaces that need to be created
    /// Maps sanitized name to `NamespaceInfo`
    pub(crate) namespace_registry: FxIndexMap<String, NamespaceInfo>,
    /// Reverse lookup: Maps ORIGINAL path to SANITIZED name
    pub(crate) path_to_sanitized_name: FxIndexMap<String, String>,
    /// Runtime tracking of all created namespaces to prevent duplicates
    pub(crate) created_namespaces: FxIndexSet<String>,
    /// Reference to the dependency graph for module relationship queries
    pub(crate) graph: Option<&'a CriboGraph>,
    /// Modules that have explicit __all__ defined
    pub(crate) modules_with_explicit_all: FxIndexSet<String>,
    /// Transformation context for tracking node mappings
    pub(crate) transformation_context: TransformationContext,
    /// Module/symbol pairs that should be kept after tree shaking
    /// Maps module name to set of symbols to keep in that module
    pub(crate) tree_shaking_keep_symbols: Option<FxIndexMap<String, FxIndexSet<String>>>,
    /// Track namespaces that were created with initial symbols
    /// These don't need symbol population via
    /// `populate_namespace_with_module_symbols_with_renames`
    pub(crate) namespaces_with_initial_symbols: FxIndexSet<String>,
    /// Track which namespace symbols have been populated after deferred imports
    /// Format: (`module_name`, `symbol_name`)
    pub(crate) symbols_populated_after_deferred: FxIndexSet<(String, String)>,
    /// Track modules whose __all__ attribute is accessed in the code
    /// Set of (`accessing_module`, `accessed_alias`) pairs to handle alias collisions
    /// Only these modules need their __all__ emitted in the bundle
    pub(crate) modules_with_accessed_all: FxIndexSet<(String, String)>,
    /// Global cache of all kept symbols for O(1) lookup
    /// Populated from `tree_shaking_keep_symbols` for efficient symbol existence checks
    pub(crate) kept_symbols_global: Option<FxIndexSet<String>>,
    /// Reference to the semantic bundler for semantic analysis
    /// This is set during `bundle_modules` and used by import transformers
    pub(crate) semantic_bundler: Option<&'a crate::semantic_bundler::SemanticBundler>,
}

impl std::fmt::Debug for Bundler<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundler")
            .field("module_registry", &self.module_registry)
            .field("entry_module_name", &self.entry_module_name)
            .field("bundled_modules", &self.bundled_modules)
            .field("inlined_modules", &self.inlined_modules)
            .finish()
    }
}

// Main implementation
impl<'a> Bundler<'a> {
    /// Check if a symbol is kept by tree shaking
    pub(crate) fn is_symbol_kept_by_tree_shaking(
        &self,
        module_name: &str,
        symbol_name: &str,
    ) -> bool {
        match &self.tree_shaking_keep_symbols {
            Some(kept_symbols) => kept_symbols
                .get(module_name)
                .is_some_and(|symbols| symbols.contains(symbol_name)),
            None => true, // No tree shaking, all symbols are kept
        }
    }

    /// Get the entry package name when entry is a package __init__.py
    /// Returns None if entry is not a package __init__.py
    #[inline]
    fn entry_package_name(&self) -> Option<&str> {
        if crate::util::is_init_module(&self.entry_module_name) {
            // Strip the .__init__ suffix if present, otherwise return None
            // Note: if entry is bare "__init__", we don't have the package name
            self.entry_module_name.strip_suffix(".__init__")
        } else {
            None
        }
    }

    /// Create a new bundler instance
    pub fn new(
        module_info_registry: Option<&'a crate::orchestrator::ModuleRegistry>,
        resolver: &'a ModuleResolver,
    ) -> Self {
        Self {
            module_registry: FxIndexMap::default(),
            init_functions: FxIndexMap::default(),
            future_imports: FxIndexSet::default(),
            bundled_modules: FxIndexSet::default(),
            inlined_modules: FxIndexSet::default(),
            entry_path: None,
            entry_module_name: String::new(),
            entry_is_package_init_or_main: false,
            module_exports: FxIndexMap::default(),
            semantic_exports: FxIndexMap::default(),
            namespace_imported_modules: FxIndexMap::default(),
            module_info_registry,
            resolver,
            circular_modules: FxIndexSet::default(),
            hard_dependencies: Vec::new(),
            symbol_dep_graph: SymbolDependencyGraph::default(),
            module_asts: None,
            global_deferred_imports: FxIndexMap::default(),
            namespace_registry: FxIndexMap::default(),
            path_to_sanitized_name: FxIndexMap::default(),
            created_namespaces: FxIndexSet::default(),
            graph: None,
            modules_with_explicit_all: FxIndexSet::default(),
            transformation_context: TransformationContext::new(),
            tree_shaking_keep_symbols: None,
            namespaces_with_initial_symbols: FxIndexSet::default(),
            symbols_populated_after_deferred: FxIndexSet::default(),
            modules_with_accessed_all: FxIndexSet::default(),
            kept_symbols_global: None,
            semantic_bundler: None,
        }
    }

    /// Create a new node with a proper index from the transformation context
    fn create_node_index(&mut self) -> AtomicNodeIndex {
        self.transformation_context.create_node_index()
    }

    /// Create a new node and record it as a transformation
    pub(super) fn create_transformed_node(&mut self, reason: String) -> AtomicNodeIndex {
        self.transformation_context.create_new_node(reason)
    }

    /// Transform bundled import from statement with context and current module
    pub(super) fn transform_bundled_import_from_multiple_with_current_module(
        &self,
        import_from: &StmtImportFrom,
        module_name: &str,
        inside_wrapper_init: bool,
        current_module: Option<&str>,
        _symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
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
        let mut assignments = Vec::new();
        let mut initialized_modules = FxIndexSet::default();

        // Track which modules we've already initialized in this import context
        // to avoid duplicate initialization calls
        let mut locally_initialized = FxIndexSet::default();

        // Check if this is a wildcard import
        if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
            // Handle wildcard import specially
            log::debug!("Handling wildcard import from wrapper module '{module_name}'");

            // Ensure the module is initialized
            if self.module_registry.contains_key(module_name)
                && !locally_initialized.contains(module_name)
            {
                assignments.extend(
                    self.create_module_initialization_for_import_with_current_module(
                        module_name,
                        current_module,
                    ),
                );
                locally_initialized.insert(module_name.to_string());
            }

            // For wildcard imports, we need to handle both wrapper modules and potential symbol
            // renames.
            // Instead of dynamic copying, we'll generate static assignments for
            // all known exports

            // Get the module's exports (either from __all__ or all non-private symbols)
            let module_exports =
                if let Some(Some(export_list)) = self.module_exports.get(module_name) {
                    // Module has __all__ defined, use it
                    export_list.clone()
                } else if let Some(semantic_exports) = self.semantic_exports.get(module_name) {
                    // Use semantic exports from analysis
                    semantic_exports.iter().cloned().collect()
                } else {
                    // Fall back to dynamic copying if we don't have static information
                    log::debug!(
                        "No static export information for module '{module_name}', using dynamic \
                         copying"
                    );

                    let module_expr = expressions::module_reference(module_name, ExprContext::Load);

                    // Create: for __cribo_attr in dir(module):
                    //             if not __cribo_attr.startswith('_'):
                    //                 globals()[__cribo_attr] = getattr(module, __cribo_attr)
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
                                    vec![
                                        module_expr.clone(),
                                        expressions::name(attr_var, ExprContext::Load),
                                    ],
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

            // Generate static assignments for each exported symbol
            log::debug!(
                "Generating static wildcard import assignments for {} symbols from '{}'",
                module_exports.len(),
                module_name
            );

            let module_expr = if module_name.contains('.') {
                let parts: Vec<&str> = module_name.split('.').collect();
                expressions::dotted_name(&parts, ExprContext::Load)
            } else {
                expressions::name(module_name, ExprContext::Load)
            };

            // Cache explicit __all__ (if any) to avoid repeated lookups
            let explicit_all = self
                .module_exports
                .get(module_name)
                .and_then(|exports| exports.as_ref());

            for symbol_name in &module_exports {
                // Skip private symbols unless explicitly in __all__
                if symbol_name.starts_with('_')
                    && !explicit_all.is_some_and(|all| all.contains(symbol_name))
                {
                    continue;
                }

                // For wrapper modules, symbols are always accessed as attributes on the module
                // object. Renaming for conflict resolution applies to inlined
                // modules, not wrapper modules.
                assignments.push(statements::simple_assign(
                    symbol_name,
                    expressions::attribute(module_expr.clone(), symbol_name, ExprContext::Load),
                ));
                log::debug!(
                    "Created wildcard import assignment: {symbol_name} = \
                     {module_name}.{symbol_name}"
                );

                // If we're inside a wrapper init, also add the module namespace assignment
                // This is critical for wildcard imports to work with module attributes
                if inside_wrapper_init && let Some(current_mod) = current_module {
                    let module_var = sanitize_module_name_for_identifier(current_mod);
                    log::debug!(
                        "Creating module attribute assignment in wrapper init for wildcard import: \
                             {module_var}.{symbol_name} = {symbol_name}"
                    );
                    assignments.push(
                            crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                                &module_var,
                                symbol_name,
                                symbol_name,
                            ),
                        );
                }
            }

            return assignments;
        }

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
            let parent_is_wrapper = self.module_registry.contains_key(module_name);
            let submodule_exists = self.bundled_modules.contains(&full_module_path)
                && (self.module_registry.contains_key(&full_module_path)
                    || self.inlined_modules.contains(&full_module_path));

            // If both the parent is a wrapper and a submodule exists, we need to decide
            // In Python, attributes from __init__.py take precedence over submodules
            // So we should prefer the attribute unless we have evidence it's not re-exported
            let importing_submodule = if parent_is_wrapper && submodule_exists {
                // Check if the parent module explicitly exports this name
                if let Some(Some(export_list)) = self.module_exports.get(module_name) {
                    // If __all__ is defined and doesn't include this name, it's the submodule
                    !export_list.contains(&imported_name.to_string())
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
                let is_submodule_of_target =
                    current_module.is_some_and(|curr| curr.starts_with(&format!("{module_name}.")));

                // Check if parent module should be initialized
                let should_initialize_parent = self.module_registry.contains_key(module_name)
                    && !locally_initialized.contains(module_name)
                    && current_module != Some(module_name) // Prevent self-initialization
                    && !is_submodule_of_target; // Prevent parent initialization from submodule

                // Check if submodule should be initialized
                let should_initialize_submodule =
                    self.module_registry.contains_key(&full_module_path)
                        && !locally_initialized.contains(&full_module_path);

                // Check if parent imports from this submodule (indicating dependency)
                // This determines initialization order to avoid forward references
                let parent_imports_submodule = should_initialize_parent
                    && should_initialize_submodule
                    && self.module_registry.contains_key(module_name)
                    && self.module_registry.contains_key(&full_module_path)
                    && self.graph.is_some_and(|graph| {
                        let parent_module = graph.get_module_by_name(module_name);
                        let child_module = graph.get_module_by_name(&full_module_path);
                        if let (Some(parent), Some(child)) = (parent_module, child_module) {
                            // Check if parent has child as a dependency
                            let parent_deps = graph.get_dependencies(parent.module_id);
                            parent_deps.contains(&child.module_id)
                        } else {
                            false
                        }
                    });

                // Initialize modules in the correct order based on dependencies
                // If parent imports submodule, initialize submodule first to avoid forward
                // references Otherwise, use normal order (parent first)
                if parent_imports_submodule {
                    // Initialize submodule first since parent depends on it
                    if should_initialize_submodule {
                        crate::code_generator::module_registry::initialize_submodule_if_needed(
                            &full_module_path,
                            &self.module_registry,
                            &mut assignments,
                            &mut locally_initialized,
                            &mut initialized_modules,
                        );
                    }

                    // Now initialize parent module after submodule
                    if should_initialize_parent {
                        assignments.extend(
                            self.create_module_initialization_for_import_with_current_module(
                                module_name,
                                current_module,
                            ),
                        );
                        locally_initialized.insert(module_name.to_string());
                    }
                } else {
                    // Normal order: parent first, then submodule
                    if should_initialize_parent {
                        // Initialize parent module first
                        assignments.extend(
                            self.create_module_initialization_for_import_with_current_module(
                                module_name,
                                current_module,
                            ),
                        );
                        locally_initialized.insert(module_name.to_string());
                    }

                    if should_initialize_submodule {
                        crate::code_generator::module_registry::initialize_submodule_if_needed(
                            &full_module_path,
                            &self.module_registry,
                            &mut assignments,
                            &mut locally_initialized,
                            &mut initialized_modules,
                        );
                    }
                }

                // Build the direct namespace reference
                log::debug!(
                    "Building namespace reference for '{}' (is_inlined: {}, has_dot: {})",
                    full_module_path,
                    self.inlined_modules.contains(&full_module_path),
                    full_module_path.contains('.')
                );
                let namespace_expr = if self.inlined_modules.contains(&full_module_path) {
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
                if inside_wrapper_init && current_module == Some(module_name) {
                    // Check if this is actually a submodule
                    let full_submodule_path = format!("{module_name}.{imported_name}");
                    if self.bundled_modules.contains(&full_submodule_path)
                        && self.module_registry.contains_key(&full_submodule_path)
                    {
                        // This is a submodule that needs initialization
                        log::debug!(
                            "Special case: module '{module_name}' importing its own submodule \
                             '{imported_name}' - initializing submodule first"
                        );

                        // Initialize the submodule
                        assignments.extend(
                            self.create_module_initialization_for_import(&full_submodule_path),
                        );
                        locally_initialized.insert(full_submodule_path.clone());

                        // Now create the assignment from the parent namespace
                        let module_expr = expressions::name(module_name, ExprContext::Load);
                        let assignment = statements::simple_assign(
                            target_name.as_str(),
                            expressions::attribute(module_expr, imported_name, ExprContext::Load),
                        );
                        assignments.push(assignment);
                        continue; // Skip the rest of the regular attribute handling
                    }
                }

                // Check if we're importing from an inlined module and the target is a wrapper
                // submodule This happens when mypkg is inlined and does `from .
                // import compat` where compat uses init function
                if self.inlined_modules.contains(module_name) && !inside_wrapper_init {
                    let full_submodule_path = format!("{module_name}.{imported_name}");
                    if self.module_registry.contains_key(&full_submodule_path) {
                        // This is importing a wrapper submodule from an inlined parent module
                        // This case should have been handled by the import transformer during inlining
                        // and deferred. If we get here, something went wrong.
                        log::warn!(
                            "Unexpected: importing wrapper submodule '{imported_name}' from inlined module \
                             '{module_name}' in transform_bundled_import_from_multiple - should have been deferred"
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

                // Ensure the module is initialized first if it's a wrapper module
                // Only initialize if we're inside a wrapper init OR if the module's init
                // function has already been defined (to avoid forward references)
                if self.module_registry.contains_key(module_name)
                    && !locally_initialized.contains(module_name)
                    && current_module != Some(module_name)  // Prevent self-initialization
                    && (inside_wrapper_init || self.init_functions.contains_key(&self.module_registry[module_name]))
                {
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
                        assignments.extend(
                            self.create_module_initialization_for_import_with_current_module(
                                module_name,
                                current_module,
                            ),
                        );
                    }
                    locally_initialized.insert(module_name.to_string());
                }

                // Check if this symbol is re-exported from an inlined submodule.
                // If it is, use the globally inlined symbol (respecting semantic renames)
                // instead of wrapper attribute access.
                if self.module_registry.contains_key(module_name) {
                    // Keep current semantics: we don't attempt to detect "directly defined in wrapper" here.
                    let is_defined_in_wrapper = false;

                    if !is_defined_in_wrapper
                        && let Some((source_module, source_symbol)) =
                            self.is_symbol_from_inlined_submodule(module_name, target_name.as_str())
                    {
                        // Map to the effective global name considering semantic renames of the source module.
                        let global_name = _symbol_renames
                            .get(&source_module)
                            .and_then(|m| m.get(&source_symbol))
                            .cloned()
                            .unwrap_or_else(|| source_symbol.clone());

                        log::debug!(
                            "Using global symbol '{}' from inlined submodule '{}' for re-exported symbol '{}' in wrapper '{}'",
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
                        continue; // Skip the normal attribute assignment
                    }
                }

                // Create: target = module.imported_name
                let module_expr = if module_name.contains('.') {
                    // For nested modules like models.user, create models.user expression
                    let parts: Vec<&str> = module_name.split('.').collect();
                    expressions::dotted_name(&parts, ExprContext::Load)
                } else {
                    // Top-level module
                    expressions::name(module_name, ExprContext::Load)
                };

                let assignment = statements::simple_assign(
                    target_name.as_str(),
                    expressions::attribute(module_expr, imported_name, ExprContext::Load),
                );

                log::debug!(
                    "Generating attribute assignment: {} = {}.{} (inside_wrapper_init: {})",
                    target_name.as_str(),
                    module_name,
                    imported_name,
                    inside_wrapper_init
                );

                assignments.push(assignment);
            }
        }

        assignments
    }

    /// Check if a symbol is re-exported from an inlined submodule
    pub(crate) fn is_symbol_from_inlined_submodule(
        &self,
        module_name: &str,
        local_name: &str,
    ) -> Option<(String, String)> {
        // We need to check if this symbol is imported from a submodule and re-exported
        // Use the graph to check if the symbol is locally defined or imported

        if let Some(graph) = self.graph
            && let Some(module) = graph.get_module_by_name(module_name)
        {
            // Look through the module's items to find imports
            for item_data in module.items.values() {
                if let crate::cribo_graph::ItemType::FromImport {
                    module: from_module,
                    names,
                    level,
                    ..
                } = &item_data.item_type
                {
                    // Check if this is importing from a relative submodule
                    let resolved_module = if *level > 0 {
                        // Relative import - resolve it properly using the resolver
                        // Find the module's path from module_asts
                        let module_path = self.module_asts.as_ref().and_then(|asts| {
                            asts.iter()
                                .find(|(name, _, _, _)| name == module_name)
                                .map(|(_, _, path, _)| path.clone())
                        });

                        // Define fallback logic once
                        let fallback = || {
                            let clean_module = from_module.trim_start_matches('.');
                            format!("{module_name}.{clean_module}")
                        };

                        if let Some(path) = module_path {
                            // Use the resolver to correctly resolve the relative import
                            // The from_module contains dots like ".submodule", we need to strip them
                            let clean_module = from_module.trim_start_matches('.');
                            let module_str = if clean_module.is_empty() {
                                None
                            } else {
                                Some(clean_module)
                            };
                            self.resolver
                                .resolve_relative_to_absolute_module_name(*level, module_str, &path)
                                .unwrap_or_else(fallback)
                        } else {
                            // Fallback if we can't find the module path
                            fallback()
                        }
                    } else {
                        from_module.clone()
                    };

                    // Check if this resolved module is an inlined submodule
                    if resolved_module.starts_with(&format!("{module_name}."))
                        && self.inlined_modules.contains(&resolved_module)
                    {
                        // Check if this import includes our symbol
                        for (imported_name, alias) in names {
                            let local = alias.as_ref().unwrap_or(imported_name);
                            if local == local_name {
                                log::debug!(
                                    "Symbol '{local_name}' in module '{module_name}' is re-exported from inlined submodule '{resolved_module}' (original name: '{imported_name}')"
                                );
                                // Return source module and original symbol name so caller can resolve renames
                                return Some((resolved_module, imported_name.to_string()));
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Collect module renames from semantic analysis
    fn collect_module_renames(
        &mut self,
        module_name: &str,
        semantic_ctx: &SemanticContext,
        symbol_renames: &mut FxIndexMap<String, FxIndexMap<String, String>>,
    ) {
        log::debug!("collect_module_renames: Processing module '{module_name}'");

        // Find the module ID for this module name
        let module_id = if let Some(module) = semantic_ctx.graph.get_module_by_name(module_name) {
            module.module_id
        } else {
            log::warn!("Module '{module_name}' not found in graph");
            return;
        };

        log::debug!("Module '{module_name}' has ID: {module_id:?}");

        // Get all renames for this module from semantic analysis
        let mut module_renames = FxIndexMap::default();

        // Use ModuleSemanticInfo to get ALL exported symbols from the module
        if let Some(module_info) = semantic_ctx.semantic_bundler.get_module_info(module_id) {
            log::debug!(
                "Module '{}' exports {} symbols: {:?}",
                module_name,
                module_info.exported_symbols.len(),
                module_info.exported_symbols.iter().collect::<Vec<_>>()
            );

            // Store semantic exports for later use
            self.semantic_exports.insert(
                module_name.to_string(),
                module_info.exported_symbols.clone(),
            );

            // Process all exported symbols from the module
            for symbol in &module_info.exported_symbols {
                // Check if this symbol is actually a submodule
                let full_submodule_path = format!("{module_name}.{symbol}");
                if self.bundled_modules.contains(&full_submodule_path) {
                    // This is a submodule - but we still need it in the rename map for namespace
                    // population Mark it specially so we know it's a submodule
                    log::debug!(
                        "Symbol '{symbol}' in module '{module_name}' is a submodule - will need \
                         special handling"
                    );
                }

                if let Some(new_name) = semantic_ctx.symbol_registry.get_rename(module_id, symbol) {
                    module_renames.insert(symbol.to_string(), new_name.to_string());
                    log::debug!(
                        "Module '{module_name}': symbol '{symbol}' renamed to '{new_name}'"
                    );
                } else {
                    // Include non-renamed symbols too - they still need to be in the namespace
                    module_renames.insert(symbol.to_string(), symbol.to_string());
                    log::debug!(
                        "Module '{module_name}': symbol '{symbol}' has no rename, using original \
                         name"
                    );
                }
            }
        } else {
            log::warn!("No semantic info found for module '{module_name}' with ID {module_id:?}");
        }

        // For inlined modules with __all__, we need to also include symbols from __all__
        // even if they're not defined in this module (they might be re-exports)
        if self.inlined_modules.contains(module_name) {
            log::debug!("Module '{module_name}' is inlined, checking for __all__ exports");
            if let Some(export_info) = self.module_exports.get(module_name) {
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
                            if self.bundled_modules.contains(&full_submodule_path) {
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
        symbol_renames.insert(module_name.to_string(), module_renames);
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
                if self.inlined_modules.contains(source_module)
                    || self.bundled_modules.contains(source_module)
                {
                    for alias in &import_from.names {
                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                        // Map the local name to its source module
                        import_sources.insert(local_name.to_string(), source_module.to_string());

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
    fn process_entry_module_statement(
        &mut self,
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

    /// Check if a file is __init__.py or __main__.py
    fn is_package_init_or_main(path: &std::path::Path) -> bool {
        path.file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|name| name == "__init__.py" || name == "__main__.py")
    }

    /// Initialize the bundler with parameters and basic settings
    fn initialize_bundler(&mut self, params: &BundleParams<'a>) {
        // Store tree shaking decisions if provided
        if let Some(shaker) = params.tree_shaker {
            // Extract all kept symbols from the tree shaker
            let mut kept_symbols: FxIndexMap<String, FxIndexSet<String>> = FxIndexMap::default();
            for (module_name, _, _, _) in params.modules {
                let module_symbols = shaker.get_used_symbols_for_module(module_name);
                if !module_symbols.is_empty() {
                    kept_symbols.insert(module_name.clone(), module_symbols);
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
                log::debug!(
                    "Populated global kept symbols cache with {} unique symbols",
                    all_kept.len()
                );
                self.kept_symbols_global = Some(all_kept);
            }
        }

        // Extract modules that access __all__ from the pre-computed graph data
        // Store (accessing_module, accessed_alias) pairs to handle alias collisions
        for (alias_name, accessing_modules) in params.graph.get_modules_accessing_all() {
            for accessing_module in accessing_modules {
                self.modules_with_accessed_all
                    .insert((accessing_module.clone(), alias_name.clone()));
                log::debug!("Module '{accessing_module}' accesses {alias_name}.__all__");
            }
        }

        log::debug!("Entry module name: {}", params.entry_module_name);
        log::debug!(
            "Module names in modules vector: {:?}",
            params
                .modules
                .iter()
                .map(|(name, _, _, _)| name)
                .collect::<Vec<_>>()
        );

        // Store entry module information
        self.entry_module_name = params.entry_module_name.to_string();

        // Check if entry is __init__.py or __main__.py from params.modules
        self.entry_is_package_init_or_main = if let Some((_, _, path, _)) = params
            .modules
            .iter()
            .find(|(name, _, _, _)| name == params.entry_module_name)
        {
            Self::is_package_init_or_main(path)
        } else if let Some((_, path, _)) = params
            .sorted_modules
            .iter()
            .find(|(name, _, _)| name == params.entry_module_name)
        {
            // Fallback to sorted_modules if not found in modules
            Self::is_package_init_or_main(path)
        } else {
            false
        };

        log::debug!(
            "Entry is package init or main: {}",
            self.entry_is_package_init_or_main
        );

        // First pass: collect future imports from ALL modules before trimming
        // This ensures future imports are hoisted even if they appear late in the file
        for (_module_name, ast, _, _) in params.modules {
            let future_imports = crate::analyzers::ImportAnalyzer::collect_future_imports(ast);
            self.future_imports.extend(future_imports);
        }

        // Store entry path for relative path calculation
        if let Some((_, entry_path, _)) = params.sorted_modules.last() {
            self.entry_path = Some(entry_path.to_string_lossy().to_string());
        }
    }

    /// Collect symbol renames from semantic analysis
    fn collect_symbol_renames(
        &mut self,
        modules: &[(String, ModModule, PathBuf, String)],
        semantic_ctx: &SemanticContext,
    ) -> FxIndexMap<String, FxIndexMap<String, String>> {
        let mut symbol_renames = FxIndexMap::default();

        // Convert ModuleId-based renames to module name-based renames
        for (module_name, _, _, _) in modules {
            self.collect_module_renames(module_name, semantic_ctx, &mut symbol_renames);
        }

        symbol_renames
    }

    /// Prepare modules by trimming imports, indexing ASTs, and detecting circular dependencies
    fn prepare_modules(
        &mut self,
        params: &BundleParams<'a>,
    ) -> Vec<(String, ModModule, PathBuf, String)> {
        // Trim unused imports from all modules
        // Note: stdlib import normalization now happens in the orchestrator
        // before dependency graph building, so imports are already normalized
        let mut modules = import_deduplicator::trim_unused_imports_from_modules(
            params.modules,
            params.graph,
            params.tree_shaker,
            params.python_version,
        );

        // Index all module ASTs to assign node indices and initialize transformation context
        log::debug!("Indexing {} modules", modules.len());
        let mut module_indices = Vec::new();
        let mut total_nodes = 0u32;
        let mut module_id = 0u32;

        // Create a mapping from module name to module ID for debugging
        let mut module_id_map = FxIndexMap::default();

        for (module_name, ast, path, _content_hash) in &mut modules {
            let indexed = crate::ast_indexer::index_module_with_id(ast, module_id);
            let node_count = indexed.node_count;
            log::debug!(
                "Module {} (ID: {}) indexed with {} nodes (indices {}-{})",
                module_name,
                module_id,
                node_count,
                module_id * crate::ast_indexer::MODULE_INDEX_RANGE,
                module_id * crate::ast_indexer::MODULE_INDEX_RANGE + node_count - 1
            );
            module_id_map.insert(module_name.clone(), module_id);
            module_indices.push((module_name.clone(), path.clone(), indexed));
            total_nodes += node_count;
            module_id += 1;
        }

        // Initialize transformation context
        // Start new node indices after all module ranges
        self.transformation_context = TransformationContext::new();
        let starting_index = module_id * crate::ast_indexer::MODULE_INDEX_RANGE;
        for _ in 0..starting_index {
            self.transformation_context.next_node_index();
        }
        log::debug!(
            "Transformation context initialized. Module count: {module_id}, Total nodes: \
             {total_nodes}, New nodes start at: {starting_index}"
        );

        // Store module ASTs for re-export resolution
        self.module_asts = Some(modules.clone());

        // Track bundled modules
        for (module_name, _, _, _) in &modules {
            self.bundled_modules.insert(module_name.clone());
            log::debug!("Tracking bundled module: '{module_name}'");
        }

        // Check which modules are imported directly (e.g., import module_name)
        let directly_imported_modules =
            self.find_directly_imported_modules(&modules, params.entry_module_name);
        log::debug!("Directly imported modules: {directly_imported_modules:?}");

        // Find modules that are imported as namespaces (e.g., from models import base)
        // The modules vector already contains all modules including the entry module
        self.find_namespace_imported_modules(&modules);

        // Identify all modules that are part of circular dependencies
        if let Some(analysis) = params.circular_dep_analysis {
            log::debug!("CircularDependencyAnalysis received:");
            log::debug!("  Resolvable cycles: {:?}", analysis.resolvable_cycles);
            log::debug!("  Unresolvable cycles: {:?}", analysis.unresolvable_cycles);
            for group in &analysis.resolvable_cycles {
                for module in &group.modules {
                    self.circular_modules.insert(module.clone());
                }
            }
            for group in &analysis.unresolvable_cycles {
                for module in &group.modules {
                    self.circular_modules.insert(module.clone());
                }
            }
            log::debug!("Circular modules: {:?}", self.circular_modules);

            // If entry module is __init__.py, also remove the entry package from circular modules
            // For example, if entry is "yaml.__init__" and "yaml" is in circular modules, remove "yaml"
            // as they're the same file (yaml/__init__.py)
            if self.entry_is_package_init_or_main
                && let Some(entry_pkg) = self.entry_package_name()
            {
                let entry_pkg = entry_pkg.to_string(); // Convert to owned string to avoid borrow issues
                // Remove the specific entry package from circular modules
                if self.circular_modules.contains(&entry_pkg) {
                    log::debug!(
                        "Removing package '{entry_pkg}' from circular modules as it's the same as entry module '__init__.py'"
                    );
                    self.circular_modules.swap_remove(&entry_pkg);
                }
            }
        } else {
            log::debug!("No circular dependency analysis provided");
        }

        modules
    }

    /// Bundle multiple modules using the hybrid approach
    pub fn bundle_modules(&mut self, params: &BundleParams<'a>) -> ModModule {
        let mut final_body = Vec::new();

        // Extract the Python version from params
        let python_version = params.python_version;

        // Store the graph reference for use in transformation methods
        self.graph = Some(params.graph);

        // Store the semantic bundler reference for use in transformations
        self.semantic_bundler = Some(params.semantic_bundler);

        // Initialize bundler settings and collect preliminary data
        self.initialize_bundler(params);

        // Prepare modules: trim imports, index ASTs, detect circular dependencies
        let modules = self.prepare_modules(params);

        // Classify modules into inlinable and wrapper modules
        let classifier = crate::analyzers::ModuleClassifier::new(
            self.resolver,
            self.entry_module_name.clone(),
            self.entry_is_package_init_or_main,
            self.namespace_imported_modules.clone(),
            self.circular_modules.clone(),
        );
        let classification = classifier.classify_modules(&modules, python_version);
        self.modules_with_explicit_all = classification.modules_with_explicit_all.clone();
        let inlinable_modules = classification.inlinable_modules;
        let wrapper_modules = classification.wrapper_modules;
        let module_exports_map = classification.module_exports_map;

        // Track which modules will be inlined (before wrapper module generation)
        for (module_name, _, _, _) in &inlinable_modules {
            self.inlined_modules.insert(module_name.clone());
            // Also store module exports for inlined modules
            self.module_exports.insert(
                module_name.clone(),
                module_exports_map.get(module_name).cloned().flatten(),
            );
        }

        // Register wrapper modules
        for (module_name, _ast, _module_path, content_hash) in &wrapper_modules {
            self.module_exports.insert(
                module_name.clone(),
                module_exports_map.get(module_name).cloned().flatten(),
            );

            // Register module with synthetic name and init function
            crate::code_generator::module_registry::register_module(
                module_name,
                content_hash,
                &mut self.module_registry,
                &mut self.init_functions,
            );

            // Remove from inlined_modules since it's now a wrapper module
            self.inlined_modules.shift_remove(module_name);
        }

        // Check if we have wrapper modules
        let _has_wrapper_modules = !wrapper_modules.is_empty();

        // Create semantic context
        let semantic_ctx = SemanticContext {
            graph: params.graph,
            symbol_registry: params.semantic_bundler.symbol_registry(),
            semantic_bundler: params.semantic_bundler,
        };

        // Get symbol renames from semantic analysis
        let mut symbol_renames = self.collect_symbol_renames(&modules, &semantic_ctx);

        // Pre-detect namespace requirements from imports of inlined submodules
        // This must be done after we know which modules are inlined but before transformation
        // begins
        namespace_manager::detect_namespace_requirements_from_imports(self, &modules);

        // Collect global symbols from the entry module first (for compatibility)
        let mut global_symbols =
            SymbolAnalyzer::collect_global_symbols(&modules, params.entry_module_name);

        // Save wrapper modules for later processing
        let wrapper_modules_saved = wrapper_modules;

        // The dependency graph already provides the correct order
        let sorted_wrapper_modules = wrapper_modules_saved.clone();

        // Check if at least one wrapper module participates in a circular dependency
        // This affects initialization order and hard dependency handling
        let has_circular_wrapped_modules = sorted_wrapper_modules
            .iter()
            .any(|(name, _, _, _)| self.circular_modules.contains(name.as_str()));
        if has_circular_wrapped_modules {
            log::info!(
                "Detected circular dependencies in modules with side effects - special handling \
                 required"
            );
        }

        // Before inlining modules, check which wrapper modules they depend on
        // We only track direct dependencies from inlined modules to wrapper modules
        // Wrapper-to-wrapper dependencies will be handled through normal init ordering
        let mut wrapper_modules_needed_by_inlined = FxIndexSet::default();
        for (module_name, ast, module_path, _) in &inlinable_modules {
            // Check imports in the module
            for stmt in &ast.body {
                if let Stmt::ImportFrom(import_from) = stmt {
                    // Handle "from . import X" pattern where X might be a wrapper module
                    if import_from.level > 0 && import_from.module.is_none() {
                        // This is "from . import X" pattern
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str();
                            // Resolve the parent module
                            let parent_module =
                                self.resolver.resolve_relative_to_absolute_module_name(
                                    import_from.level,
                                    None, // No module name, just the parent
                                    module_path,
                                );
                            if let Some(parent) = parent_module {
                                let potential_module = format!("{parent}.{imported_name}");
                                // Check if this will be a wrapper module (check in wrapper_modules_saved list)
                                if wrapper_modules_saved
                                    .iter()
                                    .any(|(name, _, _, _)| name == &potential_module)
                                {
                                    wrapper_modules_needed_by_inlined
                                        .insert(potential_module.clone());
                                    log::debug!(
                                        "Inlined module '{module_name}' imports wrapper module '{potential_module}' via 'from . import'"
                                    );
                                }
                            }
                        }
                    }

                    // Resolve relative imports to absolute module names
                    let resolved_module = if import_from.level > 0 {
                        // This is a relative import, resolve it
                        self.resolver.resolve_relative_to_absolute_module_name(
                            import_from.level,
                            import_from
                                .module
                                .as_ref()
                                .map(ruff_python_ast::Identifier::as_str),
                            module_path,
                        )
                    } else {
                        // Absolute import
                        import_from.module.as_ref().map(|m| m.as_str().to_string())
                    };

                    if let Some(ref resolved) = resolved_module {
                        // Check if this is a wrapper module
                        if self.module_registry.contains_key(resolved.as_str()) {
                            wrapper_modules_needed_by_inlined.insert(resolved.to_string());
                            log::debug!(
                                "Inlined module '{module_name}' imports from wrapper module \
                                 '{resolved}'"
                            );
                        }
                    }
                }
            }
        }

        // Now we need to find transitive dependencies for wrapper modules needed by inlined modules
        // If an inlined module needs wrapper A, and wrapper A needs wrapper B, then B must be
        // initialized before A. We need to build the full dependency chain.
        let mut wrapper_to_wrapper_deps: FxIndexMap<String, FxIndexSet<String>> =
            FxIndexMap::default();

        // Collect wrapper-to-wrapper dependencies
        for (module_name, ast, module_path, _) in &wrapper_modules_saved {
            for stmt in &ast.body {
                if let Stmt::ImportFrom(import_from) = stmt {
                    // Handle "from . import X" pattern
                    if import_from.level > 0 && import_from.module.is_none() {
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str();
                            let parent_module =
                                self.resolver.resolve_relative_to_absolute_module_name(
                                    import_from.level,
                                    None,
                                    module_path,
                                );
                            if let Some(parent) = parent_module {
                                let potential_module = format!("{parent}.{imported_name}");
                                if wrapper_modules_saved
                                    .iter()
                                    .any(|(name, _, _, _)| name == &potential_module)
                                {
                                    wrapper_to_wrapper_deps
                                        .entry(module_name.clone())
                                        .or_default()
                                        .insert(potential_module);
                                }
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
                        import_from.module.as_ref().map(|m| m.as_str().to_string())
                    };

                    if let Some(ref resolved) = resolved_module
                        && wrapper_modules_saved
                            .iter()
                            .any(|(name, _, _, _)| name == resolved)
                    {
                        wrapper_to_wrapper_deps
                            .entry(module_name.clone())
                            .or_default()
                            .insert(resolved.clone());
                    }
                }
            }
        }

        // Add transitive dependencies
        let mut all_needed = wrapper_modules_needed_by_inlined.clone();
        let mut to_process = wrapper_modules_needed_by_inlined.clone();

        while !to_process.is_empty() {
            let mut next_to_process = FxIndexSet::default();
            for module in &to_process {
                if let Some(deps) = wrapper_to_wrapper_deps.get(module) {
                    for dep in deps {
                        if !all_needed.contains(dep) {
                            all_needed.insert(dep.clone());
                            next_to_process.insert(dep.clone());
                            log::debug!("Adding transitive dependency: {dep} (needed by {module})");
                        }
                    }
                }
            }
            to_process = next_to_process;
        }

        // Create classification lookups
        let inlinable_set: FxIndexSet<String> = inlinable_modules
            .iter()
            .map(|(name, _, _, _)| name.clone())
            .collect();
        let wrapper_set: FxIndexSet<String> = wrapper_modules_saved
            .iter()
            .map(|(name, _, _, _)| name.clone())
            .collect();

        // Create module map for quick lookup
        let mut module_map: FxIndexMap<String, (ModModule, PathBuf, String)> =
            FxIndexMap::default();
        for (name, ast, path, hash) in &modules {
            module_map.insert(name.clone(), (ast.clone(), path.clone(), hash.clone()));
        }

        let mut all_inlined_stmts = Vec::new();
        let mut processed_modules = FxIndexSet::default();
        let mut pending_import_assignments: Vec<(String, Stmt)> = Vec::new(); // (required_symbol, assignment)

        // Log the dependency order from the graph
        log::info!("Module processing order from dependency graph:");
        for (i, (module_name, module_path, deps)) in params.sorted_modules.iter().enumerate() {
            log::info!(
                "  {}. {} (path: {:?}, deps: {:?})",
                i + 1,
                module_name,
                module_path,
                deps
            );
        }

        // Process each module in dependency order
        for (module_name, _module_path, _deps) in params.sorted_modules {
            // Skip if not in our module set (e.g., stdlib modules)
            if !module_map.contains_key(module_name) {
                log::debug!("  Skipping {module_name} - not in module map (likely stdlib)");
                continue;
            }

            log::info!(
                "Processing module: {} (inlinable: {}, wrapper: {})",
                module_name,
                inlinable_modules
                    .iter()
                    .any(|(n, _, _, _)| n == module_name),
                wrapper_modules_saved
                    .iter()
                    .any(|(n, _, _, _)| n == module_name)
            );

            let (ast, path, _hash) = module_map
                .get(module_name)
                .expect("Module should exist in module_map after topological sorting")
                .clone();

            if inlinable_set.contains(module_name) {
                // Process as inlinable module
                log::debug!("Inlining module: {module_name}");

                // Create namespace for inlinable modules (simple namespace without flags)
                if module_name != params.entry_module_name {
                    let namespace_var = sanitize_module_name_for_identifier(module_name);
                    if !self.created_namespaces.contains(&namespace_var) {
                        log::debug!("Creating namespace for inlinable module '{module_name}'");
                        let namespace_stmt = statements::simple_assign(
                            &namespace_var,
                            expressions::call(
                                expressions::simple_namespace_ctor(),
                                vec![],
                                vec![Keyword {
                                    node_index: self.create_node_index(),
                                    arg: Some(Identifier::new("__name__", TextRange::default())),
                                    value: expressions::string_literal(module_name),
                                    range: TextRange::default(),
                                }],
                            ),
                        );
                        all_inlined_stmts.push(namespace_stmt);
                        self.created_namespaces.insert(namespace_var.clone());

                        // Also handle parent namespaces if this is a submodule
                        if let Some((parent, child)) = module_name.rsplit_once('.') {
                            let parent_var = sanitize_module_name_for_identifier(parent);
                            if !self.created_namespaces.contains(&parent_var) {
                                let parent_stmt = statements::simple_assign(
                                    &parent_var,
                                    expressions::call(
                                        expressions::simple_namespace_ctor(),
                                        vec![],
                                        vec![Keyword {
                                            node_index: self.create_node_index(),
                                            arg: Some(Identifier::new(
                                                "__name__",
                                                TextRange::default(),
                                            )),
                                            value: expressions::string_literal(parent),
                                            range: TextRange::default(),
                                        }],
                                    ),
                                );
                                all_inlined_stmts.push(parent_stmt);
                                self.created_namespaces.insert(parent_var.clone());
                            }

                            // Add parent.child = child assignment
                            let parent_child_assign = statements::assign(
                                vec![expressions::attribute(
                                    expressions::name(&parent_var, ExprContext::Load),
                                    child,
                                    ExprContext::Store,
                                )],
                                expressions::name(&namespace_var, ExprContext::Load),
                            );
                            all_inlined_stmts.push(parent_child_assign);
                        }
                    }
                }

                // Create a temporary vector for import assignments
                let mut import_assignments = Vec::new();

                // Create inline context for this specific module
                let mut inline_ctx = InlineContext {
                    module_exports_map: &module_exports_map,
                    global_symbols: &mut global_symbols,
                    module_renames: &mut symbol_renames,
                    inlined_stmts: &mut all_inlined_stmts,
                    import_aliases: FxIndexMap::default(),
                    deferred_imports: &mut import_assignments,
                    import_sources: FxIndexMap::default(),
                    python_version,
                };

                // Inline just this module
                self.inline_module(module_name, ast, &path, &mut inline_ctx);

                // Check which import assignments can be added now
                for stmt in import_assignments {
                    // Try to extract the symbol being assigned from
                    if let Stmt::Assign(ref assign) = stmt {
                        if let Some(value_name) = assign.value.as_name_expr() {
                            let required_symbol = value_name.id.as_str();

                            // Check if the required symbol is already defined
                            if global_symbols.contains(required_symbol) {
                                // Symbol is available, add the assignment immediately
                                log::debug!(
                                    "Adding immediate import assignment for {required_symbol}"
                                );
                                all_inlined_stmts.push(stmt);
                            } else {
                                // Symbol not yet available, defer the assignment
                                log::debug!(
                                    "Deferring import assignment for {required_symbol} (not yet defined)"
                                );
                                pending_import_assignments
                                    .push((required_symbol.to_string(), stmt));
                            }
                        } else {
                            // Complex assignment, add it immediately
                            all_inlined_stmts.push(stmt);
                        }
                    } else {
                        // Not an assignment, add it immediately
                        all_inlined_stmts.push(stmt);
                    }
                }

                // Mark this module as processed
                processed_modules.insert(module_name.to_string());

                // Check if any pending assignments can now be resolved
                let mut still_pending = Vec::new();
                for (required_symbol, stmt) in pending_import_assignments.drain(..) {
                    if global_symbols.contains(&required_symbol) {
                        log::debug!("Resolving deferred import assignment for {required_symbol}");
                        all_inlined_stmts.push(stmt);
                    } else {
                        still_pending.push((required_symbol, stmt));
                    }
                }
                pending_import_assignments = still_pending;
            } else if wrapper_set.contains(module_name) {
                // Process wrapper module immediately in dependency order
                log::debug!("Processing wrapper module: {module_name}");

                // Get the content hash for this module
                let content_hash = module_map
                    .get(module_name)
                    .map_or_else(|| "000000".to_string(), |(_, _, hash)| hash.clone());

                // Generate the init function for this wrapper module
                let synthetic_name = self
                    .module_registry
                    .entry(module_name.to_string())
                    .or_insert_with(|| {
                        let name =
                            crate::code_generator::module_registry::get_synthetic_module_name(
                                module_name,
                                &content_hash,
                            );
                        log::debug!(
                            "Registered wrapper module '{module_name}' with synthetic name '{name}'"
                        );
                        name
                    })
                    .clone();

                // Create the module transform context
                let transform_ctx = ModuleTransformContext {
                    module_name,
                    synthetic_name: &synthetic_name,
                    module_path: &path,
                    global_info: None,
                    semantic_bundler: self.semantic_bundler,
                    python_version,
                    is_wrapper_body: true,
                };

                // Transform the module into an init function
                // For wrapper modules processed in dependency order, we don't have module_renames yet
                let empty_renames = FxIndexMap::default();
                let init_function = module_transformer::transform_module_to_init_function(
                    self,
                    &transform_ctx,
                    ast.clone(),
                    &empty_renames,
                );

                // Check if this is a package (ends with __init__.py)
                let is_package = path.ends_with("__init__.py");

                // Use the new create_wrapper_module function to output everything together
                let wrapper_stmts = crate::ast_builder::module_wrapper::create_wrapper_module(
                    module_name,
                    &synthetic_name,
                    init_function,
                    is_package,
                );

                // Add all the wrapper module statements (namespace, init function, __init__ assignment)
                all_inlined_stmts.extend(wrapper_stmts);

                // Mark the namespace as created
                let module_var = sanitize_module_name_for_identifier(module_name);
                self.created_namespaces.insert(module_var.clone());

                // Also handle parent namespaces if this is a submodule
                if let Some((parent, child)) = module_name.rsplit_once('.') {
                    let parent_var = sanitize_module_name_for_identifier(parent);
                    if !self.created_namespaces.contains(&parent_var) {
                        let parent_stmt = statements::simple_assign(
                            &parent_var,
                            expressions::call(
                                expressions::simple_namespace_ctor(),
                                vec![],
                                vec![Keyword {
                                    node_index: self.create_node_index(),
                                    arg: Some(Identifier::new("__name__", TextRange::default())),
                                    value: expressions::string_literal(parent),
                                    range: TextRange::default(),
                                }],
                            ),
                        );
                        all_inlined_stmts.push(parent_stmt);
                        self.created_namespaces.insert(parent_var.clone());
                    }

                    // Add parent.child = child assignment
                    let parent_child_assign = statements::assign(
                        vec![expressions::attribute(
                            expressions::name(&parent_var, ExprContext::Load),
                            child,
                            ExprContext::Store,
                        )],
                        expressions::name(&module_var, ExprContext::Load),
                    );
                    all_inlined_stmts.push(parent_child_assign);
                }

                // Mark this module as processed
                processed_modules.insert(module_name.to_string());
            }
        }

        // Generate module-level exports for all submodules of the entry module
        // This ensures that imports like `requests.exceptions` work in the bundled module
        // For package bundles, the entry is typically "__init__" but we want to export
        // submodules of the package itself (e.g., "requests.exceptions" not "__init__.exceptions")
        // So we need to find the package name from the modules
        let package_name = if params.entry_module_name == "__init__" {
            // Find the package name from any submodule
            self.inlined_modules
                .iter()
                .chain(self.module_registry.keys())
                .find_map(|name| {
                    if name.contains('.') {
                        name.split('.').next()
                    } else if name != "__init__" {
                        Some(name.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("__init__")
        } else {
            params.entry_module_name
        };

        // Collect already-defined symbols to avoid overwriting them
        // We scan the final_body for assignments to detect what symbols exist
        let mut already_defined_symbols = FxIndexSet::default();
        for stmt in &final_body {
            if let Stmt::Assign(assign) = stmt {
                for target in &assign.targets {
                    if let Expr::Name(name) = target {
                        already_defined_symbols.insert(name.id.to_string());
                    }
                }
            }
        }

        if package_name != "__init__" {
            log::debug!("Generating module-level exports for package '{package_name}'");

            // Find all direct submodules of the package
            let mut submodule_exports = Vec::new();

            // Check both inlined and wrapper modules
            for module_name in self
                .inlined_modules
                .iter()
                .chain(self.module_registry.keys())
            {
                // Check if this is a direct submodule of the package
                if module_name.starts_with(&format!("{package_name}.")) {
                    // Extract the first component after the package name
                    let relative_path = &module_name[package_name.len() + 1..];
                    if let Some(first_component) = relative_path.split('.').next() {
                        // Generate: first_component = package.first_component
                        // e.g., exceptions = requests.exceptions
                        let export_name = first_component.to_string();
                        let full_path = format!("{package_name}.{first_component}");

                        // Only add if we haven't already exported this
                        // AND if this name doesn't already exist as a symbol in the entry module
                        // This prevents overwriting values like __version__ = "2.32.4" with namespace objects
                        if !submodule_exports.contains(&export_name) {
                            // Check if this symbol already exists in the entry module's symbols
                            // The entry module is always the first to be processed and its symbols
                            // are the ones that should be preserved at the module level
                            let symbol_already_exists =
                                already_defined_symbols.contains(&export_name);

                            if symbol_already_exists {
                                log::debug!(
                                    "  Skipping module-level export for '{export_name}' - already exists as a symbol in entry module"
                                );
                            } else {
                                log::debug!(
                                    "  Adding module-level export: {export_name} = {full_path}"
                                );
                                final_body.push(statements::simple_assign(
                                    &export_name,
                                    expressions::dotted_name(
                                        &full_path.split('.').collect::<Vec<_>>(),
                                        ExprContext::Load,
                                    ),
                                ));
                                submodule_exports.push(export_name);
                            }
                        }
                    }
                }
            }

            log::debug!(
                "Added {} module-level exports for entry module submodules",
                submodule_exports.len()
            );
        }

        // Add all inlined and wrapper module statements to final_body
        final_body.extend(all_inlined_stmts);

        // Finally, add entry module code (it's always last in topological order)
        // Find the entry module in our modules list
        let entry_module = modules
            .into_iter()
            .find(|(name, _, _, _)| name == params.entry_module_name);

        if let Some((module_name, mut ast, module_path, _)) = entry_module {
            log::debug!("Processing entry module: '{module_name}'");
            log::debug!("Entry module has {} statements", ast.body.len());

            // If the entry module is part of circular dependencies, reorder its statements
            // The entry module might be named "__init__" while the circular module is tracked as "yaml" (or similar package name)
            let mut reorder = false;
            let mut lookup_name = module_name.as_str();

            if crate::util::is_init_module(&module_name) {
                // For __init__ modules, we need to find the corresponding package name
                // in the circular modules list
                if let Some(package_name) = self
                    .circular_modules
                    .iter()
                    .find(|m| !m.contains('.') && !crate::util::is_init_module(m))
                {
                    reorder = true;
                    lookup_name = package_name.as_str();
                }
            } else if self.circular_modules.contains(&module_name) {
                reorder = true;
            }

            if reorder {
                log::debug!(
                    "Entry module '{module_name}' is part of circular dependencies, reordering statements"
                );
                ast.body = self.reorder_statements_for_circular_module(
                    lookup_name,
                    ast.body,
                    python_version,
                );
            }

            // Entry module - add its code directly at the end
            // The entry module needs special handling for symbol conflicts
            let entry_module_renames = symbol_renames
                .get(&module_name)
                .cloned()
                .unwrap_or_default();

            log::debug!("Entry module '{module_name}' renames: {entry_module_renames:?}");

            // First pass: collect locally defined symbols in the entry module
            let mut locally_defined_symbols = FxIndexSet::default();
            for stmt in &ast.body {
                match stmt {
                    Stmt::FunctionDef(func_def) => {
                        locally_defined_symbols.insert(func_def.name.to_string());
                    }
                    Stmt::ClassDef(class_def) => {
                        locally_defined_symbols.insert(class_def.name.to_string());
                    }
                    _ => {}
                }
            }
            log::debug!("Entry module locally defined symbols: {locally_defined_symbols:?}");

            // Apply recursive import transformation to the entry module
            log::debug!("Creating RecursiveImportTransformer for entry module '{module_name}'");
            let mut entry_deferred_imports = Vec::new();

            // Check if importlib has been fully transformed
            let (_importlib_was_transformed, _created_namespace_objects) = {
                let mut transformer = RecursiveImportTransformer::new(
                    RecursiveImportTransformerParams {
                        bundler: self,
                        module_name: &module_name,
                        module_path: Some(&module_path),
                        symbol_renames: &symbol_renames,
                        deferred_imports: &mut entry_deferred_imports,
                        is_entry_module: true,  // This is the entry module
                        is_wrapper_init: false, // Not a wrapper init
                        global_deferred_imports: Some(&self.global_deferred_imports), /* Pass global deferred imports for checking */
                        python_version,
                    },
                );

                // Pre-populate stdlib aliases that are defined in the ENTRY module only
                // to avoid leaking aliases from other modules.
                let mut entry_stdlib_aliases: FxIndexMap<String, String> = FxIndexMap::default();
                for stmt in &ast.body {
                    match stmt {
                        Stmt::Import(import_stmt) => {
                            for alias in &import_stmt.names {
                                let imported = alias.name.as_str();
                                let root = imported.split('.').next().unwrap_or(imported);
                                if ruff_python_stdlib::sys::is_known_standard_library(
                                    python_version,
                                    root,
                                ) {
                                    let local = alias
                                        .asname
                                        .as_ref()
                                        .map_or(imported, ruff_python_ast::Identifier::as_str);
                                    entry_stdlib_aliases
                                        .insert(local.to_string(), imported.to_string());
                                }
                            }
                        }
                        Stmt::ImportFrom(import_from) => {
                            if import_from.level == 0
                                && let Some(module) = &import_from.module
                            {
                                let module_str = module.as_str();
                                if module_str != "__future__" {
                                    let root = module_str.split('.').next().unwrap_or(module_str);
                                    if ruff_python_stdlib::sys::is_known_standard_library(
                                        python_version,
                                        root,
                                    ) {
                                        for alias in &import_from.names {
                                            if let Some(asname) = &alias.asname {
                                                entry_stdlib_aliases.insert(
                                                    asname.as_str().to_string(),
                                                    module_str.to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                for (alias_name, module_name) in entry_stdlib_aliases {
                    let rewritten_path = Self::get_rewritten_stdlib_path(&module_name);
                    log::debug!("Entry stdlib alias: {alias_name} -> {rewritten_path}");
                    transformer
                        .import_aliases
                        .insert(alias_name, rewritten_path);
                }

                transformer.transform_module(&mut ast);
                log::debug!("Finished transforming entry module '{module_name}'");

                (
                    transformer.importlib_transformed,
                    transformer.created_namespace_objects,
                )
            };

            // Process statements in order
            for stmt in &ast.body {
                let is_hoisted = import_deduplicator::is_hoisted_import(self, stmt);
                if is_hoisted {
                    continue;
                }

                match stmt {
                    Stmt::ImportFrom(import_from) => {
                        let duplicate = import_deduplicator::is_duplicate_import_from(
                            self,
                            import_from,
                            &final_body,
                            python_version,
                        );

                        if duplicate {
                            log::debug!(
                                "Skipping duplicate import in entry module: {:?}",
                                import_from.module
                            );
                        } else {
                            // Imports have already been transformed by RecursiveImportTransformer
                            final_body.push(stmt.clone());
                        }
                    }
                    Stmt::Import(import_stmt) => {
                        let duplicate = import_deduplicator::is_duplicate_import(
                            self,
                            import_stmt,
                            &final_body,
                        );

                        if duplicate {
                            log::debug!("Skipping duplicate import in entry module");
                        } else {
                            // Imports have already been transformed by RecursiveImportTransformer
                            final_body.push(stmt.clone());
                        }
                    }
                    Stmt::Assign(assign) => {
                        // Check if this is an import assignment for a locally defined symbol
                        let is_import_for_local_symbol = if assign.targets.len() == 1 {
                            if let Expr::Name(target) = &assign.targets[0] {
                                locally_defined_symbols.contains(target.id.as_str())
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if is_import_for_local_symbol {
                            log::debug!(
                                "Skipping import assignment for locally defined symbol in entry \
                                 module"
                            );
                            continue;
                        }

                        // Check if this assignment already exists in final_body to avoid duplicates
                        let is_duplicate = if assign.targets.len() == 1 {
                            match &assign.targets[0] {
                                // Check name assignments
                                Expr::Name(target) => {
                                    // Look for exact duplicate in final_body
                                    final_body.iter().any(|stmt| {
                                        if let Stmt::Assign(existing) = stmt {
                                            if existing.targets.len() == 1 {
                                                if let Expr::Name(existing_target) =
                                                    &existing.targets[0]
                                                {
                                                    // Check if it's the same assignment
                                                    existing_target.id == target.id
                                                        && expression_handlers::expr_equals(
                                                            &existing.value,
                                                            &assign.value,
                                                        )
                                                } else {
                                                    false
                                                }
                                            } else {
                                                false
                                            }
                                        } else {
                                            false
                                        }
                                    })
                                }
                                // Check attribute assignments like schemas.user = ...
                                Expr::Attribute(target_attr) => {
                                    let target_path =
                                        expression_handlers::extract_attribute_path(target_attr);

                                    // Check if this is a module init assignment
                                    if let Expr::Call(call) = &assign.value.as_ref()
                                        && let Expr::Name(func_name) = &call.func.as_ref()
                                        && is_init_function(func_name.id.as_str())
                                    {
                                        // Check in final_body for same module init
                                        final_body.iter().any(|stmt| {
                                            if let Stmt::Assign(existing) = stmt
                                                && existing.targets.len() == 1
                                                && let Expr::Attribute(existing_attr) =
                                                    &existing.targets[0]
                                                && let Expr::Call(existing_call) =
                                                    &existing.value.as_ref()
                                                && let Expr::Name(existing_func) =
                                                    &existing_call.func.as_ref()
                                                && is_init_function(existing_func.id.as_str())
                                            {
                                                let existing_path =
                                                    expression_handlers::extract_attribute_path(
                                                        existing_attr,
                                                    );
                                                if existing_path == target_path {
                                                    log::debug!(
                                                        "Found duplicate module init in \
                                                         final_body: {} = {}",
                                                        target_path,
                                                        func_name.id.as_str()
                                                    );
                                                    return true;
                                                }
                                            }
                                            false
                                        })
                                    } else {
                                        false
                                    }
                                }
                                _ => false,
                            }
                        } else {
                            false
                        };

                        if is_duplicate {
                            log::debug!("Skipping duplicate assignment in entry module");
                        } else {
                            let mut stmt_clone = stmt.clone();
                            self.process_entry_module_statement(
                                &mut stmt_clone,
                                &entry_module_renames,
                                &mut final_body,
                            );
                        }
                    }
                    _ => {
                        let mut stmt_clone = stmt.clone();
                        self.process_entry_module_statement(
                            &mut stmt_clone,
                            &entry_module_renames,
                            &mut final_body,
                        );
                    }
                }
            }

            // CRITICAL FIX: Expose child modules at module level for entry module
            // When the entry module is a package that has child modules (like requests.exceptions),
            // those child modules need to be exposed at the module level so they can be accessed
            // when the module is imported via importlib.
            // For example, after `import requests`, you should be able to access `requests.exceptions`.
            // This adds statements like: exceptions = requests.exceptions
            if module_name == params.entry_module_name {
                log::debug!(
                    "Adding module-level exposure for child modules of entry module {module_name}"
                );

                // For __init__ modules, we need to find the actual package name
                // The package name is the wrapper module without __init__
                let package_name = if crate::util::is_init_module(&module_name) {
                    // Find the wrapper module that represents the package
                    self.module_registry
                        .keys()
                        .find(|m| !m.contains('.') && self.module_registry.contains_key(*m))
                        .cloned()
                        .unwrap_or_else(|| module_name.to_string())
                } else {
                    module_name.to_string()
                };

                log::debug!("Package name for exposure: {package_name}");

                // Find all child modules of the entry module's package
                let entry_child_modules: Vec<String> = self
                    .bundled_modules
                    .iter()
                    .filter(|m| m.starts_with(&format!("{package_name}.")) && m.contains('.'))
                    .cloned()
                    .collect();

                // First, collect all existing variable names to avoid conflicts
                let existing_variables: FxIndexSet<String> = final_body
                    .iter()
                    .filter_map(|stmt| {
                        if let Stmt::Assign(assign) = stmt
                            && let [Expr::Name(name)] = assign.targets.as_slice()
                        {
                            Some(name.id.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                for child_module in entry_child_modules {
                    // Get the child module's local name (e.g., "exceptions" from "requests.exceptions")
                    if let Some(local_name) = child_module.strip_prefix(&format!("{package_name}."))
                    {
                        // Only add top-level children, not nested ones
                        if !local_name.contains('.') {
                            // CRITICAL: Don't expose child modules that would overwrite existing variables
                            // For example, don't overwrite __version__ = "2.32.4" with __version__ = requests.__version__
                            if existing_variables.contains(local_name) {
                                log::debug!(
                                    "Skipping exposure of child module {child_module} as {local_name} - would overwrite existing variable"
                                );
                                continue;
                            }

                            log::debug!("Exposing child module {child_module} as {local_name}");

                            // Generate: local_name = package_name.local_name
                            // e.g., exceptions = requests.exceptions
                            let expose_stmt = statements::simple_assign(
                                local_name,
                                expressions::attribute(
                                    expressions::name(&package_name, ExprContext::Load),
                                    local_name,
                                    ExprContext::Load,
                                ),
                            );
                            final_body.push(expose_stmt);
                        }
                    }
                }
            }

            // Add deferred imports from the entry module after all its statements
            // But first update the global registry to prevent future duplicates
            for stmt in &entry_deferred_imports {
                if let Stmt::Assign(assign) = stmt
                    && let Expr::Attribute(attr) = &assign.value.as_ref()
                    && let Expr::Subscript(subscript) = &attr.value.as_ref()
                    && let Expr::Attribute(sys_attr) = &subscript.value.as_ref()
                    && let Expr::Name(sys_name) = &sys_attr.value.as_ref()
                    && sys_name.id.as_str() == "sys"
                    && sys_attr.attr.as_str() == "modules"
                    && let Expr::StringLiteral(lit) = &subscript.slice.as_ref()
                {
                    let import_module = lit.value.to_str();
                    let attr_name = attr.attr.as_str();
                    if let Expr::Name(target) = &assign.targets[0] {
                        let _symbol_name = target.id.as_str();
                        self.global_deferred_imports.insert(
                            (import_module.to_string(), attr_name.to_string()),
                            module_name.to_string(),
                        );
                    }
                }
            }
            // Add entry module's deferred imports to the collection
            log::debug!(
                "Adding {} deferred imports from entry module",
                entry_deferred_imports.len()
            );
            for stmt in &entry_deferred_imports {
                if let Stmt::Assign(assign) = stmt
                    && let Expr::Name(target) = &assign.targets[0]
                    && let Expr::Attribute(attr) = &assign.value.as_ref()
                {
                    let attr_path = expression_handlers::extract_attribute_path(attr);
                    log::debug!(
                        "Entry module deferred import: {} = {}",
                        target.id.as_str(),
                        attr_path
                    );
                }
            }
        }

        // Generate _cribo proxy for stdlib access (always included)
        // IMPORTANT: This must be inserted after any __future__ imports but before any other code
        // We insert it here at the very end to ensure it's not affected by any reordering
        log::debug!("Inserting _cribo proxy after __future__ imports");

        // Find the position after __future__ imports
        let mut insert_position = 0;
        for (i, stmt) in final_body.iter().enumerate() {
            if let Stmt::ImportFrom(import_from) = stmt
                && let Some(module) = &import_from.module
                && module.as_str() == "__future__"
            {
                insert_position = i + 1;
                continue;
            }
            // Stop after we've passed all __future__ imports
            break;
        }

        let proxy_statements = crate::ast_builder::proxy_generator::generate_cribo_proxy();
        // Insert proxy statements after __future__ imports
        for (i, stmt) in proxy_statements.into_iter().enumerate() {
            final_body.insert(insert_position + i, stmt);
        }

        log::debug!(
            "Creating final ModModule with {} statements",
            final_body.len()
        );
        for (i, stmt) in final_body.iter().take(3).enumerate() {
            log::debug!("Statement {}: type = {:?}", i, std::mem::discriminant(stmt));
        }
        let result = ModModule {
            node_index: self.create_transformed_node("Bundled module root".to_string()),
            range: TextRange::default(),
            body: final_body,
        };

        // Log transformation statistics
        let stats = self.transformation_context.get_stats();
        log::info!("Transformation statistics:");
        log::info!("  Total transformations: {}", stats.total_transformations);
        log::info!("  New nodes created: {}", stats.new_nodes);

        result
    }

    /// Check if a namespace is already registered
    pub fn is_namespace_registered(&self, sanitized_name: &str) -> bool {
        self.namespace_registry.contains_key(sanitized_name)
    }

    /// Get the rewritten path for a stdlib module (e.g., "json" -> "_cribo.json")
    pub fn get_rewritten_stdlib_path(module_name: &str) -> String {
        format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX)
    }

    /// Find modules that are imported directly
    pub(super) fn find_directly_imported_modules(
        &self,
        modules: &[(String, ModModule, PathBuf, String)],
        entry_module_name: &str,
    ) -> FxIndexSet<String> {
        // Use ImportAnalyzer to find directly imported modules
        ImportAnalyzer::find_directly_imported_modules(modules, entry_module_name)
    }

    /// Find modules that are imported as namespaces
    fn find_namespace_imported_modules(
        &mut self,
        modules: &[(String, ModModule, PathBuf, String)],
    ) {
        // Use ImportAnalyzer to find namespace imported modules
        self.namespace_imported_modules = ImportAnalyzer::find_namespace_imported_modules(modules);

        log::debug!(
            "Found {} namespace imported modules: {:?}",
            self.namespace_imported_modules.len(),
            self.namespace_imported_modules
        );
    }

    /// Check if a symbol should be exported from a module
    pub fn should_export_symbol(&self, symbol_name: &str, module_name: &str) -> bool {
        // Don't export __all__ itself as a module attribute
        if symbol_name == "__all__" {
            return false;
        }

        // Check if the module has explicit __all__ exports
        if let Some(Some(exports)) = self.module_exports.get(module_name) {
            // Module defines __all__, check if symbol is listed there
            if exports.iter().any(|s| s == symbol_name) {
                // Symbol is in __all__. For re-exported symbols, check if the symbol exists anywhere in the bundle.
                let should_export = match &self.kept_symbols_global {
                    Some(kept) => kept.contains(symbol_name),
                    None => true, // No tree-shaking, export everything in __all__
                };

                if should_export {
                    log::debug!(
                        "Symbol '{symbol_name}' is in module '{module_name}' __all__ list, exporting"
                    );
                } else {
                    log::debug!(
                        "Symbol '{symbol_name}' is in __all__ but was completely removed by tree-shaking, not exporting"
                    );
                }
                return should_export;
            }
        }

        // For symbols not in __all__ (or if no __all__ is defined), check tree-shaking
        let is_kept_by_tree_shaking = self.is_symbol_kept_by_tree_shaking(module_name, symbol_name);
        if !is_kept_by_tree_shaking {
            log::debug!(
                "Symbol '{symbol_name}' from module '{module_name}' was removed by tree-shaking; not exporting"
            );
            return false;
        }

        // When tree-shaking is enabled, if a symbol is kept it means it's imported/used somewhere
        // For private symbols (starting with _), we should export them if tree-shaking kept them
        // This handles the case where a private symbol is imported by another module
        if self.tree_shaking_keep_symbols.is_some() {
            // Tree-shaking is enabled and the symbol was kept, so export it
            log::debug!(
                "Symbol '{symbol_name}' from module '{module_name}' kept by tree-shaking, exporting despite visibility"
            );
            return true;
        }

        // Special case: if a symbol is imported by another module in the bundle, export it
        // even if it starts with underscore. This is necessary for symbols like _is_single_cell_widths
        // in rich.cells that are imported by rich.segment
        if symbol_name.starts_with('_') {
            log::debug!(
                "Checking if private symbol '{symbol_name}' from module '{module_name}' is imported by other modules"
            );
            if let Some(module_asts) = &self.module_asts
                && crate::analyzers::ImportAnalyzer::is_symbol_imported_by_other_modules(
                    module_asts,
                    module_name,
                    symbol_name,
                    Some(&self.module_exports),
                )
            {
                log::debug!(
                    "Private symbol '{symbol_name}' from module '{module_name}' is imported by other modules, exporting"
                );
                return true;
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
        _ctx: &InlineContext,
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
            for bundled_module in &self.bundled_modules {
                if bundled_module.ends_with(&format!(".{base_name}")) {
                    // Check if this is an inlined module (will be a namespace)
                    if self.inlined_modules.contains(bundled_module) {
                        log::debug!(
                            "Assignment references namespace module: {bundled_module} (via name \
                             {base_name})"
                        );
                        return true;
                    }
                }
            }

            // Also check if the base name itself is an inlined module
            if self.inlined_modules.contains(base_name) {
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
                if self.inlined_modules.contains(&potential_sibling) {
                    log::debug!(
                        "Assignment references sibling namespace module: {potential_sibling} (via \
                         name {name_str})"
                    );
                    return true;
                }
            }

            // Also check if the name itself is an inlined module
            if self.inlined_modules.contains(name_str) {
                log::debug!("Assignment references namespace module directly: {name_str}");
                return true;
            }
        }

        false
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
    pub fn process_body_recursive(
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
                            // For non-conditional imports, use the original logic with
                            // module_scope_symbols
                            if let Some(symbols) = module_scope_symbols {
                                for alias in &import_from.names {
                                    let local_name =
                                        alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                                    if symbols.contains(local_name)
                                        && self.should_export_symbol(local_name, module_name)
                                    {
                                        log::debug!(
                                            "Adding module.{local_name} = {local_name} after \
                                             non-conditional import"
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
                            }
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
    pub fn transform_nested_function_for_module_vars_with_global_info(
        &self,
        func_def: &mut StmtFunctionDef,
        module_level_vars: &FxIndexSet<String>,
        global_declarations: &FxIndexMap<String, Vec<ruff_text_size::TextRange>>,
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
    pub fn transform_nested_function_for_module_vars(
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
    pub fn transform_ast_with_lifted_globals(
        &self,
        ast: &mut ModModule,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::semantic_bundler::ModuleGlobalInfo,
    ) {
        // Transform all statements that use global declarations
        for stmt in &mut ast.body {
            self.transform_stmt_for_lifted_globals(stmt, lifted_names, global_info, None);
        }
    }

    /// Transform a statement to use lifted globals
    fn transform_stmt_for_lifted_globals(
        &self,
        stmt: &mut Stmt,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::semantic_bundler::ModuleGlobalInfo,
        current_function_globals: Option<&FxIndexSet<String>>,
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
                    );
                }

                // Transform finally block
                for stmt in &mut try_stmt.finalbody {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
            }
            _ => {
                // Other statement types handled as needed
            }
        }
    }

    /// Check if a symbol should be inlined based on export rules
    pub fn should_inline_symbol(
        &self,
        symbol_name: &str,
        module_name: &str,
        module_exports_map: &FxIndexMap<String, Option<Vec<String>>>,
    ) -> bool {
        // First check tree-shaking decisions if available
        let kept_by_tree_shaking = self.is_symbol_kept_by_tree_shaking(module_name, symbol_name);
        if !kept_by_tree_shaking {
            log::trace!(
                "Tree shaking: removing unused symbol '{symbol_name}' from module '{module_name}'"
            );
            return false;
        }

        // If tree-shaking kept the symbol, check if it's in the export list
        let exports = module_exports_map.get(module_name).and_then(|e| e.as_ref());

        if let Some(export_list) = exports {
            // Module has exports (either explicit __all__ or extracted symbols)
            // Check if the symbol is in the export list
            if export_list.contains(&symbol_name.to_string()) {
                return true;
            }

            // Special case for circular modules: If tree-shaking kept a private symbol
            // (starts with underscore but not dunder) in a circular module,
            // it means it's explicitly imported by another module and should be included
            // even if it's not in the regular export list
            if self.circular_modules.contains(module_name)
                && symbol_name.starts_with('_')
                && !symbol_name.starts_with("__")
            {
                log::debug!(
                    "Including private symbol '{symbol_name}' from circular module '{module_name}' because it's kept by tree-shaking"
                );
                return true;
            }

            false
        } else {
            // No exports at all, don't inline anything
            false
        }
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

    /// Create a rewritten base class expression for hard dependencies
    fn create_rewritten_base_expr(&self, hard_dep: &HardDependency, class_name: &str) -> Expr {
        // Check if the source module is a wrapper module
        let source_is_wrapper = self.module_registry.contains_key(&hard_dep.source_module);

        if source_is_wrapper && !hard_dep.base_class.contains('.') {
            // For imports from wrapper modules, we need to use module.attr pattern
            log::info!(
                "Rewrote base class {} to {}.{} for class {} in inlined module (source is wrapper)",
                hard_dep.base_class,
                hard_dep.source_module,
                hard_dep.imported_attr,
                class_name
            );

            expressions::name_attribute(
                &hard_dep.source_module,
                &hard_dep.imported_attr,
                ExprContext::Load,
            )
        } else {
            // Use the alias if it's mandatory, otherwise use the imported attr
            let name_to_use = if hard_dep.alias_is_mandatory && hard_dep.alias.is_some() {
                hard_dep
                    .alias
                    .as_ref()
                    .expect(
                        "alias should exist when alias_is_mandatory is true and alias.is_some() \
                         is true",
                    )
                    .clone()
            } else {
                hard_dep.imported_attr.clone()
            };

            log::info!(
                "Rewrote base class {} to {} for class {} in inlined module",
                hard_dep.base_class,
                name_to_use,
                class_name
            );

            Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: name_to_use.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })
        }
    }

    /// Rewrite hard dependencies in a module's AST
    pub(crate) fn rewrite_hard_dependencies_in_module(
        &self,
        ast: &mut ModModule,
        module_name: &str,
    ) {
        log::debug!("Rewriting hard dependencies in module {module_name}");

        for stmt in &mut ast.body {
            if let Stmt::ClassDef(class_def) = stmt {
                let class_name = class_def.name.as_str();
                log::debug!("  Checking class {class_name} in module {module_name}");

                // Check if this class has hard dependencies
                if let Some(arguments) = &mut class_def.arguments {
                    for arg in &mut arguments.args {
                        let base_str = expr_to_dotted_name(arg);
                        log::debug!("    Base class: {base_str}");

                        // Check against all hard dependencies for this class
                        for hard_dep in &self.hard_dependencies {
                            if hard_dep.module_name == module_name
                                && hard_dep.class_name == class_name
                            {
                                log::debug!(
                                    "      Checking against hard dep: {} -> {}",
                                    hard_dep.base_class,
                                    hard_dep.imported_attr
                                );
                                if base_str == hard_dep.base_class {
                                    // Rewrite to use the hoisted import
                                    // If the base class is module.attr pattern and we're importing
                                    // just the module,
                                    // we need to preserve the attribute access
                                    if hard_dep.base_class.contains('.')
                                        && !hard_dep.imported_attr.contains('.')
                                    {
                                        // The base class is like "cookielib.CookieJar" but we're
                                        // importing "cookielib"
                                        // So we need to preserve the attribute access pattern
                                        let parts: Vec<&str> =
                                            hard_dep.base_class.split('.').collect();
                                        if parts.len() == 2 && parts[0] == hard_dep.imported_attr {
                                            // Replace just the module part, keep the attribute
                                            let name_to_use = if hard_dep.alias_is_mandatory
                                                && hard_dep.alias.is_some()
                                            {
                                                hard_dep
                                                    .alias
                                                    .as_ref()
                                                    .expect(
                                                        "alias should exist when \
                                                         alias_is_mandatory is true and \
                                                         alias.is_some() is true",
                                                    )
                                                    .clone()
                                            } else {
                                                hard_dep.imported_attr.clone()
                                            };

                                            // Create module.attr expression
                                            *arg = expressions::name_attribute(
                                                &name_to_use,
                                                parts[1],
                                                ExprContext::Load,
                                            );
                                            log::info!(
                                                "Rewrote base class {} to {}.{} for class {} in \
                                                 inlined module",
                                                hard_dep.base_class,
                                                name_to_use,
                                                parts[1],
                                                class_name
                                            );
                                        } else {
                                            // Fall back to helper function
                                            *arg = self
                                                .create_rewritten_base_expr(hard_dep, class_name);
                                        }
                                    } else {
                                        // Use helper function for non-dotted base classes
                                        *arg =
                                            self.create_rewritten_base_expr(hard_dep, class_name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Reorder statements in a module based on symbol dependencies for circular modules
    pub(crate) fn reorder_statements_for_circular_module(
        &self,
        module_name: &str,
        statements: Vec<Stmt>,
        python_version: u8,
    ) -> Vec<Stmt> {
        log::debug!(
            "reorder_statements_for_circular_module called for module: '{}' \
             (entry_module_name: '{}', entry_is_package_init_or_main: {})",
            module_name,
            self.entry_module_name,
            self.entry_is_package_init_or_main
        );

        // Check if this is the entry module - entry modules should not have their
        // statements reordered even if they're part of circular dependencies
        let is_entry_module = if self.entry_is_package_init_or_main {
            // If entry is __init__.py or __main__.py, the module might be identified
            // by its package name (e.g., 'yaml' instead of '__init__')
            if let Some(entry_pkg) = self.entry_package_name() {
                // Check if this module is the entry package
                module_name == entry_pkg
            } else {
                // Direct comparison when we don't have package context
                module_name == self.entry_module_name
            }
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

    /// Create module initialization statements for wrapper modules when they are imported
    pub(super) fn create_module_initialization_for_import(&self, module_name: &str) -> Vec<Stmt> {
        let mut locally_initialized = FxIndexSet::default();
        self.create_module_initialization_for_import_with_tracking(
            module_name,
            &mut locally_initialized,
            None, // No current module context
        )
    }

    /// Create module initialization statements with current module context
    pub(super) fn create_module_initialization_for_import_with_current_module(
        &self,
        module_name: &str,
        current_module: Option<&str>,
    ) -> Vec<Stmt> {
        let mut locally_initialized = FxIndexSet::default();
        self.create_module_initialization_for_import_with_tracking(
            module_name,
            &mut locally_initialized,
            current_module,
        )
    }

    /// Create module initialization statements with tracking to avoid duplicates
    fn create_module_initialization_for_import_with_tracking(
        &self,
        module_name: &str,
        locally_initialized: &mut FxIndexSet<String>,
        current_module: Option<&str>,
    ) -> Vec<Stmt> {
        let mut stmts = Vec::new();

        // Skip if already initialized in this context
        if locally_initialized.contains(module_name) {
            return stmts;
        }

        // Skip if we're trying to initialize the current module
        // (we're already inside its init function)
        if let Some(current) = current_module
            && module_name == current
        {
            log::debug!(
                "Skipping initialization of module '{module_name}' - already inside its init function"
            );
            return stmts;
        }

        // If this is a child module (contains '.'), ensure parent is initialized first
        if module_name.contains('.')
            && let Some((parent_name, _)) = module_name.rsplit_once('.')
        {
            // Check if parent is also a wrapper module
            if let Some(parent_synthetic) = self.module_registry.get(parent_name) {
                // Check if parent has an init function
                if self.init_functions.contains_key(parent_synthetic) {
                    log::debug!(
                        "Ensuring parent '{parent_name}' is initialized before child '{module_name}'"
                    );

                    // Recursively ensure parent is initialized
                    // This will handle multi-level packages like foo.bar.baz
                    stmts.extend(self.create_module_initialization_for_import_with_tracking(
                        parent_name,
                        locally_initialized,
                        current_module,
                    ));
                }
            }
        }

        // Check if this is a wrapper module that needs initialization
        if let Some(synthetic_name) = self.module_registry.get(module_name) {
            // Check if the init function has been defined yet
            // (wrapper modules are processed in dependency order, so it might not exist yet)
            log::debug!(
                "Checking if wrapper module '{}' has been processed (has init function: {})",
                module_name,
                self.init_functions.contains_key(synthetic_name)
            );

            // Generate the init call
            let init_func_name =
                crate::code_generator::module_registry::get_init_function_name(synthetic_name);

            // Call the init function with the module as the self argument
            let module_var = sanitize_module_name_for_identifier(module_name);
            let init_call = expressions::call(
                expressions::name(&init_func_name, ExprContext::Load),
                vec![expressions::name(&module_var, ExprContext::Load)],
                vec![],
            );

            // Generate the appropriate assignment based on module type
            stmts.extend(self.generate_module_assignment_from_init(module_name, init_call));

            // Mark as initialized to avoid duplicates
            locally_initialized.insert(module_name.to_string());

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
        module_name: &str,
        init_call: Expr,
    ) -> Vec<Stmt> {
        let mut stmts = Vec::new();

        // Check if this module is a parent namespace that already exists
        let is_parent_namespace = self
            .module_registry
            .iter()
            .any(|(name, _)| name != module_name && name.starts_with(&format!("{module_name}.")));

        if is_parent_namespace {
            // Use temp variable and merge attributes for parent namespaces
            // Store init result in temp variable
            stmts.push(statements::simple_assign(INIT_RESULT_VAR, init_call));

            // Merge attributes from init result into existing namespace
            self.generate_merge_module_attributes(&mut stmts, module_name, INIT_RESULT_VAR);
        } else {
            // Direct assignment for simple and dotted modules
            let target_expr = if module_name.contains('.') {
                // Create attribute expression for dotted modules
                let parts: Vec<&str> = module_name.split('.').collect();
                expressions::dotted_name(&parts, ExprContext::Store)
            } else {
                // Simple name expression
                expressions::name(module_name, ExprContext::Store)
            };

            stmts.push(statements::assign(vec![target_expr], init_call));
        }

        stmts
    }

    /// Create parent namespaces for dotted imports
    pub(super) fn create_parent_namespaces(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
        for i in 1..parts.len() {
            let parent_path = parts[..i].join(".");

            if self.module_registry.contains_key(&parent_path) {
                // Parent is a wrapper module, create reference to it
                result_stmts
                    .push(self.create_module_reference_assignment(&parent_path, &parent_path));
            } else if !self.bundled_modules.contains(&parent_path) {
                // Check if this namespace is registered in the centralized system
                let sanitized = sanitize_module_name_for_identifier(&parent_path);
                let registered_in_namespace_system =
                    self.namespace_registry.contains_key(&sanitized);

                // Check if we haven't already created this namespace globally or locally
                let already_created = self.created_namespaces.contains(&parent_path)
                    || self.is_namespace_already_created(&parent_path, result_stmts)
                    || registered_in_namespace_system;

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
                        node_index: AtomicNodeIndex::dummy(),
                        arg: Some(other::identifier("__name__")),
                        value: expressions::string_literal(&parent_path),
                        range: TextRange::default(),
                    }];
                    result_stmts.push(statements::simple_assign(
                        &parent_path,
                        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords),
                    ));
                } else if registered_in_namespace_system
                    && !self.created_namespaces.contains(&parent_path)
                {
                    // The namespace is registered but hasn't been created yet
                    // This shouldn't happen if generate_required_namespaces() was called before
                    // transformation
                    log::debug!(
                        "Warning: Namespace '{parent_path}' is registered but not yet created \
                         during import transformation"
                    );
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

            // Skip if this module is already a wrapper module
            if self.module_registry.contains_key(&partial_module) {
                continue;
            }

            // Skip if this namespace was already created globally
            if self.created_namespaces.contains(&partial_module) {
                log::debug!(
                    "Skipping namespace creation for '{partial_module}' - already created globally"
                );
                continue;
            }

            // Check if we should use a flattened namespace instead of creating an empty one
            let flattened_name = sanitize_module_name_for_identifier(&partial_module);
            let should_use_flattened = self.inlined_modules.contains(&partial_module)
                && self
                    .namespaces_with_initial_symbols
                    .contains(&partial_module);

            let namespace_expr = if should_use_flattened {
                // Use the flattened namespace variable
                expressions::name(&flattened_name, ExprContext::Load)
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
        // Check if we should use a flattened namespace instead of creating an empty one
        let flattened_name = sanitize_module_name_for_identifier(module_name);
        let should_use_flattened = self.inlined_modules.contains(module_name)
            && self.namespaces_with_initial_symbols.contains(module_name);

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

        // We'll create a compound statement that does both
        let _stmts: Vec<Stmt> = Vec::new();

        // First, create the empty namespace
        let namespace_expr =
            expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]);

        // Create assignment for the namespace

        // For now, return just the namespace creation
        // The actual symbol population needs to happen after all symbols are available
        statements::simple_assign(target_name, namespace_expr)
    }

    /// Generate code to merge module attributes from the initialization result into a namespace
    fn generate_merge_module_attributes(
        &self,
        statements: &mut Vec<Stmt>,
        namespace_name: &str,
        source_module_name: &str,
    ) {
        // Generate code like:
        // for attr in dir(source_module):
        //     if not attr.startswith('_'):
        //         setattr(namespace, attr, getattr(source_module, attr))

        let attr_var = "attr";
        let loop_target = expressions::name(attr_var, ExprContext::Store);

        // dir(source_module)
        let dir_call = expressions::call(
            expressions::name("dir", ExprContext::Load),
            vec![expressions::name(source_module_name, ExprContext::Load)],
            vec![],
        );

        // not attr.startswith('_')
        let condition = expressions::unary_op(
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
        );

        // getattr(source_module, attr)
        let getattr_call = expressions::call(
            expressions::name("getattr", ExprContext::Load),
            vec![
                expressions::name(source_module_name, ExprContext::Load),
                expressions::name(attr_var, ExprContext::Load),
            ],
            vec![],
        );

        // setattr(namespace, attr, getattr(...))
        let setattr_call = statements::expr(expressions::call(
            expressions::name("setattr", ExprContext::Load),
            vec![
                expressions::name(namespace_name, ExprContext::Load),
                expressions::name(attr_var, ExprContext::Load),
                getattr_call,
            ],
            vec![],
        ));

        // if not attr.startswith('_'): setattr(...)
        let if_stmt = Stmt::If(ruff_python_ast::StmtIf {
            node_index: AtomicNodeIndex::dummy(),
            test: Box::new(condition),
            body: vec![setattr_call],
            elif_else_clauses: vec![],
            range: TextRange::default(),
        });

        // for attr in dir(...): if ...
        let for_loop = Stmt::For(ruff_python_ast::StmtFor {
            node_index: AtomicNodeIndex::dummy(),
            target: Box::new(loop_target),
            iter: Box::new(dir_call),
            body: vec![if_stmt],
            orelse: vec![],
            is_async: false,
            range: TextRange::default(),
        });

        statements.push(for_loop);
    }

    /// Transform function body for lifted globals
    fn transform_function_body_for_lifted_globals(
        &self,
        func_def: &mut StmtFunctionDef,
        params: &TransformFunctionParams,
    ) {
        let mut new_body = Vec::new();

        for body_stmt in &mut func_def.body {
            if let Stmt::Global(global_stmt) = body_stmt {
                // Rewrite global statement to use lifted names
                for name in &mut global_stmt.names {
                    if let Some(lifted_name) = params.lifted_names.get(name.as_str()) {
                        *name = crate::ast_builder::other::identifier(lifted_name);
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
                );
                new_body.push(body_stmt.clone());

                // After transforming, check if we need to add synchronization
                self.add_global_sync_if_needed(
                    body_stmt,
                    params.function_globals,
                    params.lifted_names,
                    &mut new_body,
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
    ) {
        match stmt {
            Stmt::Assign(assign) => {
                // Check if this is an assignment to a global variable
                if let [Expr::Name(name)] = &assign.targets[..] {
                    let var_name = name.id.as_str();

                    // The variable name might already be transformed to the lifted name,
                    // so we need to check if it's a lifted variable
                    if let Some(original_name) = lifted_names
                        .iter()
                        .find(|(orig, lifted)| {
                            lifted.as_str() == var_name && function_globals.contains(orig.as_str())
                        })
                        .map(|(orig, _)| orig)
                    {
                        log::debug!(
                            "Adding sync for assignment to global {var_name}: {var_name} -> \
                             module.{original_name}"
                        );
                        // Add: module.<original_name> = <lifted_name>
                        new_body.push(statements::assign(
                            vec![expressions::attribute(
                                expressions::name(MODULE_VAR, ExprContext::Load),
                                original_name,
                                ExprContext::Store,
                            )],
                            expressions::name(var_name, ExprContext::Load),
                        ));
                    }
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Check if this is an augmented assignment to a global variable
                if let Expr::Name(name) = aug_assign.target.as_ref() {
                    let var_name = name.id.as_str();

                    // Similar check for augmented assignments
                    if let Some(original_name) = lifted_names
                        .iter()
                        .find(|(orig, lifted)| {
                            lifted.as_str() == var_name && function_globals.contains(orig.as_str())
                        })
                        .map(|(orig, _)| orig)
                    {
                        log::debug!(
                            "Adding sync for augmented assignment to global {var_name}: \
                             {var_name} -> module.{original_name}"
                        );
                        // Add: module.<original_name> = <lifted_name>
                        new_body.push(statements::assign(
                            vec![expressions::attribute(
                                expressions::name(MODULE_VAR, ExprContext::Load),
                                original_name,
                                ExprContext::Store,
                            )],
                            expressions::name(var_name, ExprContext::Load),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}
