#![allow(clippy::excessive_nesting)]

use std::path::{Path, PathBuf};

use anyhow::Result;
use log::debug;
use ruff_python_ast::{
    Alias, AtomicNodeIndex, Decorator, ExceptHandler, Expr, ExprAttribute, ExprContext, ExprName,
    Identifier, ModModule, Stmt, StmtAssign, StmtClassDef, StmtFunctionDef, StmtImport,
    StmtImportFrom, visitor::source_order::SourceOrderVisitor,
};
use ruff_text_size::TextRange;

use crate::{
    analyzers::{
        ImportAnalyzer, SymbolAnalyzer, dependency_analyzer::DependencyAnalyzer,
        namespace_analyzer::NamespaceAnalyzer,
    },
    ast_builder::{expressions, other, statements},
    code_generator::{
        circular_deps::SymbolDependencyGraph,
        context::{
            BundleParams, HardDependency, InlineContext, ModuleTransformContext,
            ProcessGlobalsParams, SemanticContext,
        },
        expression_handlers, import_deduplicator,
        import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
        module_registry::{
            INIT_RESULT_VAR, generate_unique_name, sanitize_module_name_for_identifier,
        },
        namespace_manager,
    },
    cribo_graph::CriboGraph as DependencyGraph,
    resolver::ModuleResolver,
    side_effects::{is_safe_stdlib_module, module_has_side_effects},
    transformation_context::TransformationContext,
    types::{FxIndexMap, FxIndexSet},
    visitors::ExportCollector,
};

/// Parameters for transforming functions with lifted globals
struct TransformFunctionParams<'a> {
    lifted_names: &'a FxIndexMap<String, String>,
    global_info: &'a crate::semantic_bundler::ModuleGlobalInfo,
    function_globals: &'a FxIndexSet<String>,
}

/// A class definition with its immediately following attributes
#[derive(Debug, Clone)]
struct ClassBlock {
    class_stmt: Stmt,
    attributes: Vec<Stmt>,
    class_name: String,
}

/// This approach avoids forward reference issues while maintaining Python module semantics
pub struct Bundler<'a> {
    /// Track if importlib was fully transformed and should be removed
    pub(crate) importlib_fully_transformed: bool,
    /// Map from original module name to synthetic module name
    pub(crate) module_registry: FxIndexMap<String, String>,
    /// Map from synthetic module name to init function name
    pub(crate) init_functions: FxIndexMap<String, String>,
    /// Collected future imports
    pub(crate) future_imports: FxIndexSet<String>,
    /// Collected stdlib imports that are safe to hoist
    /// Maps module name to map of imported names to their aliases (None if no alias)
    pub(crate) stdlib_import_from_map: FxIndexMap<String, FxIndexMap<String, Option<String>>>,
    /// Regular import statements (import module)
    pub(crate) stdlib_import_statements: Vec<Stmt>,
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
    /// Lifted global declarations to add at module top level
    pub(crate) lifted_global_declarations: Vec<Stmt>,
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
    pub(crate) circular_predeclarations: FxIndexMap<String, FxIndexMap<String, String>>,
    /// Hard dependencies that need to be hoisted
    pub(crate) hard_dependencies: Vec<HardDependency>,
    /// Symbol dependency graph for circular modules
    pub(crate) symbol_dep_graph: SymbolDependencyGraph,
    /// Module ASTs for resolving re-exports
    pub(crate) module_asts: Option<Vec<(String, ModModule, PathBuf, String)>>,
    /// Global registry of deferred imports to prevent duplication
    /// Maps (module_name, symbol_name) to the source module that deferred it
    pub(crate) global_deferred_imports: FxIndexMap<(String, String), String>,
    /// Track all namespaces that need to be created before module initialization
    /// This ensures parent namespaces exist before any submodule assignments
    pub(crate) required_namespaces: FxIndexSet<String>,
    /// Runtime tracking of all created namespaces to prevent duplicates
    /// This includes both pre-identified and dynamically created namespaces
    pub(crate) created_namespaces: FxIndexSet<String>,
    /// Modules that have explicit __all__ defined
    pub(crate) modules_with_explicit_all: FxIndexSet<String>,
    /// Transformation context for tracking node mappings
    pub(crate) transformation_context: TransformationContext,
    /// Module/symbol pairs that should be kept after tree shaking
    /// Maps module name to set of symbols to keep in that module
    pub(crate) tree_shaking_keep_symbols: Option<FxIndexMap<String, FxIndexSet<String>>>,
    /// Whether to use the module cache model for circular dependencies
    pub(crate) use_module_cache: bool,
    /// Track namespaces that were created with initial symbols
    /// These don't need symbol population via populate_namespace_with_module_symbols_with_renames
    pub(crate) namespaces_with_initial_symbols: FxIndexSet<String>,
    /// Track namespace assignments that have already been made to avoid duplicates
    /// Format: (namespace_name, attribute_name)
    pub(crate) namespace_assignments_made: FxIndexSet<(String, String)>,
    /// Track which namespace symbols have been populated after deferred imports
    /// Format: (module_name, symbol_name)
    pub(crate) symbols_populated_after_deferred: FxIndexSet<(String, String)>,
}

impl<'a> std::fmt::Debug for Bundler<'a> {
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

    /// Create a new bundler instance
    pub fn new(
        module_info_registry: Option<&'a crate::orchestrator::ModuleRegistry>,
        resolver: &'a ModuleResolver,
    ) -> Self {
        Self {
            importlib_fully_transformed: false,
            module_registry: FxIndexMap::default(),
            init_functions: FxIndexMap::default(),
            future_imports: FxIndexSet::default(),
            stdlib_import_from_map: FxIndexMap::default(),
            stdlib_import_statements: Vec::new(),
            bundled_modules: FxIndexSet::default(),
            inlined_modules: FxIndexSet::default(),
            entry_path: None,
            entry_module_name: String::new(),
            entry_is_package_init_or_main: false,
            module_exports: FxIndexMap::default(),
            lifted_global_declarations: Vec::new(),
            namespace_imported_modules: FxIndexMap::default(),
            module_info_registry,
            resolver,
            circular_modules: FxIndexSet::default(),
            circular_predeclarations: FxIndexMap::default(),
            hard_dependencies: Vec::new(),
            symbol_dep_graph: SymbolDependencyGraph::default(),
            module_asts: None,
            global_deferred_imports: FxIndexMap::default(),
            required_namespaces: FxIndexSet::default(),
            created_namespaces: FxIndexSet::default(),
            modules_with_explicit_all: FxIndexSet::default(),
            transformation_context: TransformationContext::new(),
            tree_shaking_keep_symbols: None,
            use_module_cache: true, /* Enable module cache by default for circular
                                     * dependencies */
            namespaces_with_initial_symbols: FxIndexSet::default(),
            namespace_assignments_made: FxIndexSet::default(),
            symbols_populated_after_deferred: FxIndexSet::default(),
        }
    }

    /// Create a new node with a proper index from the transformation context
    fn create_node_index(&mut self) -> AtomicNodeIndex {
        self.transformation_context.create_node_index()
    }

    /// Create a new node and record it as a transformation
    fn create_transformed_node(&mut self, reason: String) -> AtomicNodeIndex {
        self.transformation_context.create_new_node(reason)
    }

    /// Post-process AST to assign proper node indices to any nodes created with dummy indices
    fn assign_node_indices_to_ast(&mut self, module: &mut ModModule) {
        struct NodeIndexAssigner<'b, 'a> {
            bundler: &'b mut Bundler<'a>,
        }

        impl<'b, 'a> SourceOrderVisitor<'_> for NodeIndexAssigner<'b, 'a> {
            fn visit_stmt(&mut self, stmt: &Stmt) {
                // Check if this node has a dummy index (value 0)
                let node_index = match stmt {
                    Stmt::FunctionDef(s) => &s.node_index,
                    Stmt::ClassDef(s) => &s.node_index,
                    Stmt::Import(s) => &s.node_index,
                    Stmt::ImportFrom(s) => &s.node_index,
                    Stmt::Assign(s) => &s.node_index,
                    Stmt::Return(s) => &s.node_index,
                    Stmt::Delete(s) => &s.node_index,
                    Stmt::AugAssign(s) => &s.node_index,
                    Stmt::AnnAssign(s) => &s.node_index,
                    Stmt::TypeAlias(s) => &s.node_index,
                    Stmt::For(s) => &s.node_index,
                    Stmt::While(s) => &s.node_index,
                    Stmt::If(s) => &s.node_index,
                    Stmt::With(s) => &s.node_index,
                    Stmt::Match(s) => &s.node_index,
                    Stmt::Raise(s) => &s.node_index,
                    Stmt::Try(s) => &s.node_index,
                    Stmt::Assert(s) => &s.node_index,
                    Stmt::Global(s) => &s.node_index,
                    Stmt::Nonlocal(s) => &s.node_index,
                    Stmt::Expr(s) => &s.node_index,
                    Stmt::Pass(s) => &s.node_index,
                    Stmt::Break(s) => &s.node_index,
                    Stmt::Continue(s) => &s.node_index,
                    Stmt::IpyEscapeCommand(s) => &s.node_index,
                };

                // If it's a dummy index (0), assign a new one
                if node_index.load().as_usize() == 0 {
                    let new_index = self.bundler.create_node_index();
                    node_index.set(new_index.load().as_usize() as u32);
                }

                // Continue walking
                ruff_python_ast::visitor::source_order::walk_stmt(self, stmt);
            }

            fn visit_expr(&mut self, expr: &Expr) {
                // Similar logic for expressions
                let node_index = match expr {
                    Expr::BoolOp(e) => &e.node_index,
                    Expr::BinOp(e) => &e.node_index,
                    Expr::UnaryOp(e) => &e.node_index,
                    Expr::Lambda(e) => &e.node_index,
                    Expr::If(e) => &e.node_index,
                    Expr::Dict(e) => &e.node_index,
                    Expr::Set(e) => &e.node_index,
                    Expr::ListComp(e) => &e.node_index,
                    Expr::SetComp(e) => &e.node_index,
                    Expr::DictComp(e) => &e.node_index,
                    Expr::Generator(e) => &e.node_index,
                    Expr::Await(e) => &e.node_index,
                    Expr::Yield(e) => &e.node_index,
                    Expr::YieldFrom(e) => &e.node_index,
                    Expr::Compare(e) => &e.node_index,
                    Expr::Call(e) => &e.node_index,
                    Expr::NumberLiteral(e) => &e.node_index,
                    Expr::StringLiteral(e) => &e.node_index,
                    Expr::FString(e) => &e.node_index,
                    Expr::BytesLiteral(e) => &e.node_index,
                    Expr::BooleanLiteral(e) => &e.node_index,
                    Expr::NoneLiteral(e) => &e.node_index,
                    Expr::EllipsisLiteral(e) => &e.node_index,
                    Expr::Attribute(e) => &e.node_index,
                    Expr::Subscript(e) => &e.node_index,
                    Expr::Starred(e) => &e.node_index,
                    Expr::Name(e) => &e.node_index,
                    Expr::List(e) => &e.node_index,
                    Expr::Tuple(e) => &e.node_index,
                    Expr::Slice(e) => &e.node_index,
                    Expr::IpyEscapeCommand(e) => &e.node_index,
                    Expr::Named(e) => &e.node_index,
                    Expr::TString(e) => &e.node_index,
                };

                if node_index.load().as_usize() == 0 {
                    let new_index = self.bundler.create_node_index();
                    node_index.set(new_index.load().as_usize() as u32);
                }

                ruff_python_ast::visitor::source_order::walk_expr(self, expr);
            }
        }

        let mut assigner = NodeIndexAssigner { bundler: self };
        assigner.visit_mod(&ruff_python_ast::Mod::Module(module.clone()));
    }

    /// Helper function to filter out invalid submodule assignments.
    ///
    /// This filters statements where we're trying to assign `module.attr = attr`
    /// where `attr` is a submodule that uses an init function and doesn't exist
    /// as a local variable.
    ///
    /// # Arguments
    /// * `stmts` - The statements to filter
    /// * `local_variables` - Optional set of local variables to check against
    fn filter_invalid_submodule_assignments(
        &self,
        stmts: &mut Vec<Stmt>,
        local_variables: Option<&FxIndexSet<String>>,
    ) {
        stmts.retain(|stmt| {
            if let Stmt::Assign(assign) = stmt
                && let [Expr::Attribute(attr)] = assign.targets.as_slice()
                && let Expr::Name(base) = attr.value.as_ref()
                && let Expr::Name(value) = assign.value.as_ref()
            {
                let full_path = format!("{}.{}", base.id.as_str(), attr.attr.as_str());
                let is_bundled_submodule = self.bundled_modules.contains(&full_path);
                let is_submodule_with_init = self.module_registry.contains_key(&full_path);
                let value_is_same_as_attr = value.id.as_str() == attr.attr.as_str();

                log::debug!(
                    "Checking assignment: {}.{} = {} | bundled={} | has_init={} | same_name={}",
                    base.id.as_str(),
                    attr.attr.as_str(),
                    value.id.as_str(),
                    is_bundled_submodule,
                    is_submodule_with_init,
                    value_is_same_as_attr
                );

                if is_submodule_with_init && value_is_same_as_attr {
                    // Always filter out assignments to submodules with init functions
                    log::debug!(
                        "Filtering out invalid assignment: {}.{} = {} (submodule with init \
                         function)",
                        base.id.as_str(),
                        attr.attr.as_str(),
                        value.id.as_str()
                    );
                    return false;
                }

                // Filter out assignments where we're assigning an inlined submodule to itself
                // BUT only if there's no local variable with that name
                // For example: pkg.compat = compat where 'pkg.compat' is an inlined module
                // and 'compat' is not a local variable (just the namespace we're trying to create)
                if is_bundled_submodule
                    && value_is_same_as_attr
                    && self.inlined_modules.contains(&full_path)
                {
                    // If local_variables is provided, check if the value exists as a local variable
                    if let Some(local_vars) = local_variables
                        && !local_vars.contains(value.id.as_str())
                    {
                        log::debug!(
                            "Filtering out invalid assignment: {}.{} = {} (inlined submodule, no \
                             local var)",
                            base.id.as_str(),
                            attr.attr.as_str(),
                            value.id.as_str()
                        );
                        return false;
                    }
                }

                if let Some(local_vars) = local_variables {
                    // Additional filtering when local variables are provided
                    if is_bundled_submodule && value_is_same_as_attr {
                        let is_inlined = self.inlined_modules.contains(&full_path);

                        // If the submodule is NOT inlined AND there's no local variable, it's
                        // invalid
                        if !is_inlined && !local_vars.contains(value.id.as_str()) {
                            log::debug!(
                                "Filtering out invalid assignment: {}.{} = {} (no local variable)",
                                base.id.as_str(),
                                attr.attr.as_str(),
                                value.id.as_str()
                            );
                            return false;
                        }
                    }
                }
            }
            true
        });
    }

    /// Transform bundled import from statement with context and current module
    pub(super) fn transform_bundled_import_from_multiple_with_current_module(
        &self,
        import_from: StmtImportFrom,
        module_name: &str,
        inside_wrapper_init: bool,
        current_module: Option<&str>,
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
                let is_submodule_of_target = current_module
                    .map(|curr| curr.starts_with(&format!("{module_name}.")))
                    .unwrap_or(false);

                // Check if parent module should be initialized
                let should_initialize_parent = self.module_registry.contains_key(module_name)
                    && !locally_initialized.contains(module_name)
                    && current_module != Some(module_name) // Prevent self-initialization
                    && !is_submodule_of_target; // Prevent parent initialization from submodule

                if should_initialize_parent {
                    // Initialize parent module
                    assignments.extend(self.create_module_initialization_for_import(module_name));
                    locally_initialized.insert(module_name.to_string());
                }

                // Check if submodule should be initialized
                if self.module_registry.contains_key(&full_module_path)
                    && !locally_initialized.contains(&full_module_path)
                {
                    // Check if we already have this module initialization in assignments
                    let already_initialized = assignments.iter().any(|stmt| {
                        if let Stmt::Assign(assign) = stmt
                            && assign.targets.len() == 1
                            && let Expr::Attribute(attr) = &assign.targets[0]
                            && let Expr::Call(call) = &assign.value.as_ref()
                            && let Expr::Name(func_name) = &call.func.as_ref()
                            && crate::code_generator::module_registry::is_init_function(
                                func_name.id.as_str(),
                            )
                        {
                            let attr_path = expression_handlers::extract_attribute_path(attr);
                            attr_path == full_module_path
                        } else {
                            false
                        }
                    });

                    if !already_initialized {
                        assignments.extend(
                            self.create_module_initialization_for_import(&full_module_path),
                        );
                    }
                    locally_initialized.insert(full_module_path.clone());
                    initialized_modules.insert(full_module_path.clone());
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
                        // We should treat it like a submodule import, not an attribute import
                        log::debug!(
                            "Importing wrapper submodule '{imported_name}' from inlined module \
                             '{module_name}'"
                        );

                        // Initialize the submodule if needed
                        if !locally_initialized.contains(&full_submodule_path) {
                            assignments.extend(
                                self.create_module_initialization_for_import(&full_submodule_path),
                            );
                            locally_initialized.insert(full_submodule_path.clone());
                        }

                        // Create direct assignment to the submodule namespace
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
                if self.module_registry.contains_key(module_name)
                    && !locally_initialized.contains(module_name)
                    && current_module != Some(module_name)
                // Prevent self-initialization
                {
                    // Check if this module is already initialized in any deferred imports
                    let module_init_exists = assignments.iter().any(|stmt| {
                        if let Stmt::Assign(assign) = stmt
                            && assign.targets.len() == 1
                            && let Expr::Call(call) = &assign.value.as_ref()
                            && let Expr::Name(func_name) = &call.func.as_ref()
                            && crate::code_generator::module_registry::is_init_function(
                                func_name.id.as_str(),
                            )
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
                        assignments
                            .extend(self.create_module_initialization_for_import(module_name));
                    }
                    locally_initialized.insert(module_name.to_string());
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

    /// Check if a string is a valid Python identifier
    fn is_valid_python_identifier(name: &str) -> bool {
        // Use ruff's identifier validation which handles Unicode and keywords
        ruff_python_stdlib::identifiers::is_identifier(name)
    }

    /// Check if a module accesses attributes on imported modules at module level
    /// where those imported modules are part of the same circular dependency
    fn module_accesses_imported_attributes(&self, ast: &ModModule, module_name: &str) -> bool {
        use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};

        // First, collect all module-level imports and their names
        let mut imported_module_names = FxIndexSet::default();

        for stmt in &ast.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let imported_as = alias.asname.as_ref().unwrap_or(&alias.name);
                        let imported_module = &alias.name;
                        // Check if this imported module is in the circular dependency
                        if self.circular_modules.contains(imported_module.as_str()) {
                            imported_module_names.insert(imported_as.to_string());
                        }
                    }
                }
                Stmt::ImportFrom(import_from) => {
                    // Handle relative imports within the same package
                    let resolved_module = if import_from.level > 0 {
                        // This is a relative import - resolve it based on the current module
                        if let Some(parent_idx) = module_name.rfind('.') {
                            let parent = &module_name[..parent_idx];
                            if let Some(module) = &import_from.module {
                                // from .submodule import something
                                format!("{parent}.{module}")
                            } else {
                                // from . import something
                                parent.to_string()
                            }
                        } else {
                            continue; // Can't resolve relative import
                        }
                    } else if let Some(module) = &import_from.module {
                        module.to_string()
                    } else {
                        continue; // Invalid import
                    };

                    // Check if we're importing the module itself (from x import y where y is a
                    // module)
                    for alias in &import_from.names {
                        let name = alias.name.as_str();
                        let imported_as = alias.asname.as_ref().unwrap_or(&alias.name);
                        // Check if this could be a module import
                        let potential_module = format!("{resolved_module}.{name}");
                        if self.circular_modules.contains(&potential_module) {
                            imported_module_names.insert(imported_as.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        // If no circular modules are imported, no need to check further
        if imported_module_names.is_empty() {
            return false;
        }

        // Now check if we access attributes on any of these imported circular modules
        struct AttributeAccessChecker<'a> {
            has_circular_attribute_access: bool,
            imported_circular_modules: &'a FxIndexSet<String>,
        }

        impl<'a> Visitor<'a> for AttributeAccessChecker<'a> {
            fn visit_stmt(&mut self, stmt: &'a Stmt) {
                match stmt {
                    // Skip function and class bodies - we only care about module-level code
                    Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                        // Don't recurse into function or class bodies
                    }
                    _ => {
                        // Continue visiting for other statements
                        walk_stmt(self, stmt);
                    }
                }
            }

            fn visit_expr(&mut self, expr: &'a Expr) {
                if self.has_circular_attribute_access {
                    return; // Already found one
                }

                // Check for attribute access on names (e.g., mod_c.C_CONSTANT)
                if let Expr::Attribute(attr) = expr
                    && let Expr::Name(name_expr) = &*attr.value
                {
                    // Check if this name is one of our imported circular modules
                    if self
                        .imported_circular_modules
                        .contains(name_expr.id.as_str())
                    {
                        self.has_circular_attribute_access = true;
                        return;
                    }
                }

                // Continue walking
                walk_expr(self, expr);
            }
        }

        let mut checker = AttributeAccessChecker {
            has_circular_attribute_access: false,
            imported_circular_modules: &imported_module_names,
        };

        checker.visit_body(&ast.body);
        checker.has_circular_attribute_access
    }

    /// Collect future imports from an AST
    fn collect_future_imports_from_ast(&mut self, ast: &ModModule) {
        for stmt in &ast.body {
            let Stmt::ImportFrom(import_from) = stmt else {
                continue;
            };

            let Some(ref module) = import_from.module else {
                continue;
            };

            if module.as_str() == "__future__" {
                for alias in &import_from.names {
                    self.future_imports.insert(alias.name.to_string());
                }
            }
        }
    }

    /// Collect module renames from semantic analysis
    fn collect_module_renames(
        &self,
        module_name: &str,
        semantic_ctx: &SemanticContext,
        symbol_renames: &mut FxIndexMap<String, FxIndexMap<String, String>>,
    ) {
        log::debug!("collect_module_renames: Processing module '{module_name}'");

        // Find the module ID for this module name
        let module_id = match semantic_ctx.graph.get_module_by_name(module_name) {
            Some(module) => module.module_id,
            None => {
                log::warn!("Module '{module_name}' not found in graph");
                return;
            }
        };

        log::debug!("Module '{module_name}' has ID: {module_id:?}");

        // Get all renames for this module from semantic analysis
        let mut module_renames = FxIndexMap::default();

        // Use ModuleSemanticInfo to get ALL exported symbols from the module
        if let Some(module_info) = semantic_ctx.semantic_bundler.get_module_info(&module_id) {
            log::debug!(
                "Module '{}' exports {} symbols: {:?}",
                module_name,
                module_info.exported_symbols.len(),
                module_info.exported_symbols.iter().collect::<Vec<_>>()
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

                if let Some(new_name) = semantic_ctx.symbol_registry.get_rename(&module_id, symbol)
                {
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

    /// Sort wrapper modules by dependencies
    fn sort_wrapper_modules_by_dependencies(
        &self,
        wrapper_modules: &[(String, ModModule, PathBuf, String)],
        graph: &DependencyGraph,
    ) -> Result<Vec<(String, ModModule, PathBuf, String)>> {
        use crate::analyzers::dependency_analyzer::DependencyAnalyzer;

        // Extract module names
        let wrapper_names: Vec<String> = wrapper_modules
            .iter()
            .map(|(name, _, _, _)| name.clone())
            .collect();

        // Get all modules for the analyzer (wrapper modules are a subset)
        let all_modules = wrapper_modules;

        // Use DependencyAnalyzer to sort
        let sorted_names = DependencyAnalyzer::sort_wrapper_modules_by_dependencies(
            wrapper_names,
            all_modules,
            graph,
        );

        // Create a map for quick lookup
        let module_map: FxIndexMap<String, (String, ModModule, PathBuf, String)> = wrapper_modules
            .iter()
            .map(|m| (m.0.clone(), m.clone()))
            .collect();

        // Map sorted names back to full modules
        let sorted_modules = sorted_names
            .into_iter()
            .filter_map(|module_name| module_map.get(&module_name).cloned())
            .collect();

        Ok(sorted_modules)
    }

    /// Transform module to cache init function
    fn transform_module_to_cache_init_function(
        &mut self,
        ctx: ModuleTransformContext,
        ast: ModModule,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Result<Stmt> {
        // Call the regular transform_module_to_init_function to get the function
        let stmt = crate::code_generator::module_transformer::transform_module_to_init_function(
            self,
            ctx,
            ast,
            symbol_renames,
        )?;

        // Add the @functools.cache decorator
        if let Stmt::FunctionDef(mut func_def) = stmt {
            func_def.decorator_list = vec![Decorator {
                range: TextRange::default(),
                node_index: AtomicNodeIndex::dummy(),
                expression: expressions::attribute(
                    expressions::name("functools", ExprContext::Load),
                    "cache",
                    ExprContext::Load,
                ),
            }];
            return Ok(Stmt::FunctionDef(func_def));
        }

        // Should not happen
        unreachable!("transform_module_to_init_function should return a FunctionDef")
    }

    /// Build a map of imported symbols to their source modules by analyzing import statements
    fn build_import_source_map(
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

    /// Inline a module
    pub fn inline_module(
        &mut self,
        module_name: &str,
        mut ast: ModModule,
        module_path: &Path,
        ctx: &mut InlineContext,
    ) -> Result<Vec<Stmt>> {
        let mut module_renames = FxIndexMap::default();

        // Apply hard dependency rewriting BEFORE import transformation
        if !self.hard_dependencies.is_empty() && self.circular_modules.contains(module_name) {
            self.rewrite_hard_dependencies_in_module(&mut ast, module_name);
        }

        // Then apply recursive import transformation to the module
        let mut transformer = RecursiveImportTransformer::new(RecursiveImportTransformerParams {
            bundler: self,
            module_name,
            module_path: Some(module_path),
            symbol_renames: ctx.module_renames,
            deferred_imports: ctx.deferred_imports,
            is_entry_module: false, // This is not the entry module
            is_wrapper_init: false, // Not a wrapper init
            global_deferred_imports: Some(&self.global_deferred_imports), // Pass global registry
        });
        transformer.transform_module(&mut ast);

        // Copy import aliases from the transformer to the inline context
        ctx.import_aliases = transformer.import_aliases;

        // Reorder statements to ensure proper declaration order
        let statements = if self.circular_modules.contains(module_name) {
            self.reorder_statements_for_circular_module(module_name, ast.body, ctx.python_version)
        } else {
            // Even for non-circular modules, ensure module-level variables are declared
            // before functions that might use them
            self.reorder_statements_for_proper_declaration_order(ast.body)
        };

        // Build a map of imported symbols to their source modules
        ctx.import_sources = self.build_import_source_map(&statements, module_name);

        // Process each statement in the module
        for stmt in statements {
            match &stmt {
                Stmt::Import(import_stmt) => {
                    // Imports have already been transformed by RecursiveImportTransformer
                    // Include them in the inlined output
                    if !import_deduplicator::is_hoisted_import(self, &stmt) {
                        log::debug!(
                            "Including non-hoisted import in inlined module '{}': {:?}",
                            module_name,
                            import_stmt
                                .names
                                .iter()
                                .map(|a| (a.name.as_str(), a.asname.as_ref().map(|n| n.as_str())))
                                .collect::<Vec<_>>()
                        );
                        ctx.inlined_stmts.push(stmt.clone());
                    }
                }
                Stmt::ImportFrom(_) => {
                    // Imports have already been transformed by RecursiveImportTransformer
                    // Include them in the inlined output
                    if !import_deduplicator::is_hoisted_import(self, &stmt) {
                        ctx.inlined_stmts.push(stmt.clone());
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    let func_name = func_def.name.to_string();
                    if !self.should_inline_symbol(&func_name, module_name, ctx.module_exports_map) {
                        continue;
                    }

                    // Check if this symbol was renamed by semantic analysis
                    let renamed_name = if let Some(module_rename_map) =
                        ctx.module_renames.get(module_name)
                    {
                        if let Some(new_name) = module_rename_map.get(&func_name) {
                            // Only use semantic rename if it's actually different
                            if new_name != &func_name {
                                log::debug!(
                                    "Using semantic rename for '{func_name}' to '{new_name}' in \
                                     module '{module_name}'"
                                );
                                new_name.clone()
                            } else {
                                // Semantic rename is same as original, check if there's a conflict
                                if ctx.global_symbols.contains(&func_name) {
                                    // There's a conflict, apply module suffix pattern
                                    let base_name = self.get_unique_name_with_module_suffix(
                                        &func_name,
                                        module_name,
                                    );
                                    generate_unique_name(&base_name, ctx.global_symbols)
                                } else {
                                    // No conflict, use original name
                                    func_name.clone()
                                }
                            }
                        } else {
                            // No semantic rename, check if there's a conflict
                            if ctx.global_symbols.contains(&func_name) {
                                // There's a conflict, apply module suffix pattern
                                let base_name = self
                                    .get_unique_name_with_module_suffix(&func_name, module_name);
                                generate_unique_name(&base_name, ctx.global_symbols)
                            } else {
                                // No conflict, use original name
                                func_name.clone()
                            }
                        }
                    } else {
                        // No semantic rename, check if there's a conflict
                        if ctx.global_symbols.contains(&func_name) {
                            // There's a conflict, apply module suffix pattern
                            let base_name =
                                self.get_unique_name_with_module_suffix(&func_name, module_name);
                            generate_unique_name(&base_name, ctx.global_symbols)
                        } else {
                            // No conflict, use original name
                            func_name.clone()
                        }
                    };

                    // Always track the symbol mapping, even if not renamed
                    module_renames.insert(func_name.clone(), renamed_name.clone());
                    ctx.global_symbols.insert(renamed_name.clone());

                    // Clone and rename the function
                    let mut func_def_clone = func_def.clone();
                    func_def_clone.name = Identifier::new(renamed_name, TextRange::default());

                    // Apply renames to function annotations (parameters and return type)
                    if let Some(ref mut returns) = func_def_clone.returns {
                        expression_handlers::resolve_import_aliases_in_expr(
                            returns,
                            &ctx.import_aliases,
                        );
                        expression_handlers::rewrite_aliases_in_expr(returns, &module_renames);
                    }

                    // Apply renames to parameter annotations
                    for param in &mut func_def_clone.parameters.args {
                        if let Some(ref mut annotation) = param.parameter.annotation {
                            expression_handlers::resolve_import_aliases_in_expr(
                                annotation,
                                &ctx.import_aliases,
                            );
                            expression_handlers::rewrite_aliases_in_expr(
                                annotation,
                                &module_renames,
                            );
                        }
                    }

                    // First resolve import aliases in function body
                    for body_stmt in &mut func_def_clone.body {
                        Self::resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
                    }

                    // Create a temporary statement to rewrite the entire function properly
                    let mut temp_stmt = Stmt::FunctionDef(func_def_clone);

                    // Apply renames to the entire function (this will handle global statements
                    // correctly)
                    expression_handlers::rewrite_aliases_in_stmt(&mut temp_stmt, &module_renames);

                    // Also apply semantic renames from context
                    if let Some(semantic_renames) = ctx.module_renames.get(module_name) {
                        expression_handlers::rewrite_aliases_in_stmt(
                            &mut temp_stmt,
                            semantic_renames,
                        );
                    }

                    ctx.inlined_stmts.push(temp_stmt);
                }
                Stmt::ClassDef(class_def) => {
                    self.inline_class(class_def, module_name, &mut module_renames, ctx);
                }
                Stmt::Assign(assign) => {
                    self.inline_assignment(assign, module_name, &mut module_renames, ctx);
                }
                Stmt::AnnAssign(ann_assign) => {
                    self.inline_ann_assignment(ann_assign, module_name, &mut module_renames, ctx);
                }
                // TypeAlias statements are safe metadata definitions
                Stmt::TypeAlias(_) => {
                    // Type aliases don't need renaming in Python, they're just metadata
                    ctx.inlined_stmts.push(stmt);
                }
                // Pass statements are no-ops and safe
                Stmt::Pass(_) => {
                    // Pass statements can be included as-is
                    ctx.inlined_stmts.push(stmt);
                }
                // Expression statements that are string literals are docstrings
                Stmt::Expr(expr_stmt) => {
                    if matches!(expr_stmt.value.as_ref(), Expr::StringLiteral(_)) {
                        // This is a docstring - safe to include
                        ctx.inlined_stmts.push(stmt);
                    } else {
                        // Other expression statements shouldn't exist in side-effect-free modules
                        log::warn!(
                            "Unexpected expression statement in side-effect-free module \
                             '{module_name}': {stmt:?}"
                        );
                    }
                }
                _ => {
                    // Any other statement type that we haven't explicitly handled
                    log::warn!(
                        "Unexpected statement type in side-effect-free module '{module_name}': \
                         {stmt:?}"
                    );
                }
            }
        }

        // Store the renames for this module
        if !module_renames.is_empty() {
            ctx.module_renames
                .insert(module_name.to_string(), module_renames);
        }

        Ok(Vec::new()) // Statements are accumulated in ctx.inlined_stmts
    }

    /// Get imports from entry module
    fn get_entry_module_imports(
        &self,
        modules: &[(String, ModModule, PathBuf, String)],
        entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut imported_modules = FxIndexSet::default();

        // Find the entry module
        for (module_name, ast, _, _) in modules {
            if module_name == entry_module_name {
                // Check all import statements
                for stmt in &ast.body {
                    if let Stmt::Import(import_stmt) = stmt {
                        for alias in &import_stmt.names {
                            let module_name = alias.name.as_str();
                            // Track both dotted and non-dotted wrapper modules
                            if self.module_registry.contains_key(module_name) {
                                log::debug!("Entry module imports wrapper module: {module_name}");
                                imported_modules.insert(module_name.to_string());
                            }
                        }
                    }
                }
                break;
            }
        }

        log::debug!("Entry module imported modules: {imported_modules:?}");
        imported_modules
    }

    /// Sort deferred imports to ensure dependencies are satisfied
    /// This ensures namespace creations come before assignments that use those namespaces
    /// Uses a simple categorization approach to group statements by type
    fn sort_deferred_imports_for_dependencies(&self, imports: &mut Vec<Stmt>) {
        // This is a simplified implementation that addresses the specific issue
        // of forward references in namespace attribute accesses

        let n = imports.len();
        if n <= 1 {
            return; // No need to sort if 0 or 1 items
        }

        // Separate statements into categories for proper ordering
        let mut namespace_creations = Vec::new();
        let mut namespace_populations = Vec::new();
        let mut attribute_accesses = Vec::new();
        let mut other_statements = Vec::new();

        for stmt in imports.drain(..) {
            if let Stmt::Assign(assign) = &stmt {
                // Check if this creates a namespace
                if assign.targets.len() == 1 {
                    if let Expr::Name(target) = &assign.targets[0]
                        && let Expr::Call(call) = assign.value.as_ref()
                        && let Expr::Attribute(attr) = call.func.as_ref()
                        && let Expr::Name(base) = attr.value.as_ref()
                        && base.id.as_str() == "types"
                        && attr.attr.as_str() == "SimpleNamespace"
                    {
                        log::debug!("Found namespace creation: {}", target.id);
                        namespace_creations.push(stmt);
                        continue;
                    }

                    // Check if this populates a namespace (e.g., namespace.attr = value)
                    if let Expr::Attribute(target_attr) = &assign.targets[0]
                        && let Expr::Name(_) = target_attr.value.as_ref()
                    {
                        // Special case: if the value is a simple name (e.g., pkg.compat = compat)
                        // this needs the name to be defined first, so treat it as an attribute
                        // access
                        if let Expr::Name(value_name) = assign.value.as_ref() {
                            log::debug!(
                                "Found namespace assignment depending on name: {}.{} = {}",
                                target_attr
                                    .value
                                    .as_ref()
                                    .as_name_expr()
                                    .expect(
                                        "target_attr.value should be Expr::Name as checked by \
                                         outer if let"
                                    )
                                    .id
                                    .as_str(),
                                target_attr.attr,
                                value_name.id
                            );
                            attribute_accesses.push(stmt);
                            continue;
                        }

                        log::debug!(
                            "Found namespace population: {}.{}",
                            target_attr
                                .value
                                .as_ref()
                                .as_name_expr()
                                .expect(
                                    "target_attr.value should be Expr::Name as checked by outer \
                                     if let"
                                )
                                .id
                                .as_str(),
                            target_attr.attr
                        );
                        namespace_populations.push(stmt);
                        continue;
                    }
                }

                // Check if this accesses namespace attributes (e.g., var = namespace.attr)
                if let Expr::Attribute(attr) = assign.value.as_ref()
                    && let Expr::Name(_) = attr.value.as_ref()
                {
                    log::debug!(
                        "Found attribute access: {} = {}.{}",
                        if let Expr::Name(target) = &assign.targets[0] {
                            target.id.as_str()
                        } else {
                            "?"
                        },
                        if let Expr::Name(base) = attr.value.as_ref() {
                            base.id.as_str()
                        } else {
                            "?"
                        },
                        attr.attr
                    );
                    attribute_accesses.push(stmt);
                    continue;
                }
            }

            other_statements.push(stmt);
        }

        // Rebuild in proper order:
        // 1. Namespace creations first
        // 2. Other statements (general assignments)
        // 3. Namespace populations
        // 4. Attribute accesses last
        imports.extend(namespace_creations);
        imports.extend(other_statements);
        imports.extend(namespace_populations);
        imports.extend(attribute_accesses);

        if !imports.is_empty() {
            log::debug!(
                "Reordered {} deferred imports to prevent forward references",
                imports.len()
            );
        }
    }

    /// Collect all Name expressions that a given expression depends on
    fn collect_name_dependencies(expr: &Expr, deps: &mut FxIndexSet<String>) {
        match expr {
            Expr::Name(name) => {
                deps.insert(name.id.to_string());
            }
            Expr::Attribute(attr) => {
                Self::collect_name_dependencies(&attr.value, deps);
            }
            Expr::Call(call) => {
                Self::collect_name_dependencies(&call.func, deps);
                for arg in &call.arguments.args {
                    Self::collect_name_dependencies(arg, deps);
                }
            }
            Expr::BinOp(binop) => {
                Self::collect_name_dependencies(&binop.left, deps);
                Self::collect_name_dependencies(&binop.right, deps);
            }
            Expr::UnaryOp(unaryop) => {
                Self::collect_name_dependencies(&unaryop.operand, deps);
            }
            Expr::List(list) => {
                for item in &list.elts {
                    Self::collect_name_dependencies(item, deps);
                }
            }
            Expr::Tuple(tuple) => {
                for item in &tuple.elts {
                    Self::collect_name_dependencies(item, deps);
                }
            }
            Expr::Dict(dict) => {
                for item in &dict.items {
                    if let Some(key) = &item.key {
                        Self::collect_name_dependencies(key, deps);
                    }
                    Self::collect_name_dependencies(&item.value, deps);
                }
            }
            _ => {
                // For other expression types, we don't need to track dependencies
            }
        }
    }

    /// Check if module has forward references that would cause NameError
    pub(crate) fn check_module_has_forward_references(
        &self,
        module_name: &str,
        _module_renames: &FxIndexMap<String, String>,
    ) -> bool {
        // Always create empty namespaces for modules that are part of a package hierarchy
        // to avoid forward reference issues. The symbols will be added later.

        // For modules that are part of packages (contain dots), or are packages themselves
        // we should create empty namespaces initially
        if module_name.contains('.') || self.is_package_namespace(module_name) {
            log::debug!(
                "Module '{module_name}' is part of a package hierarchy, creating empty namespace"
            );
            return true;
        }

        false
    }

    /// Check if a module is a package namespace
    fn is_package_namespace(&self, module_name: &str) -> bool {
        let package_prefix = format!("{module_name}.");
        self.bundled_modules
            .iter()
            .any(|bundled| bundled.starts_with(&package_prefix))
    }

    /// Extract attribute path from expression
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

    /// Bundle multiple modules using the hybrid approach
    pub fn bundle_modules(&mut self, params: BundleParams<'_>) -> Result<ModModule> {
        let mut final_body = Vec::new();

        // Extract the Python version from params
        let python_version = params.python_version;

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
                    .map(|s| s.len())
                    .unwrap_or(0)
            );
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

        // Check if entry is __init__.py or __main__.py
        self.entry_is_package_init_or_main = if let Some((_, _, path, _)) = params
            .modules
            .iter()
            .find(|(name, _, _, _)| name == params.entry_module_name)
        {
            let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            file_name == "__init__.py" || file_name == "__main__.py"
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
            self.collect_future_imports_from_ast(ast);
        }

        // Check if entry module has direct imports or dotted imports that might create namespace
        // objects - but only for first-party modules that we're actually bundling
        let needs_types_for_entry_imports = {
            // Find the entry module AST from the pre-parsed modules
            if let Some((_, ast, _, _)) = params
                .modules
                .iter()
                .find(|(name, _, _, _)| name == params.entry_module_name)
            {
                ast.body.iter().any(|stmt| {
                    if let Stmt::Import(import_stmt) = stmt {
                        import_stmt.names.iter().any(|alias| {
                            let module_name = alias.name.as_str();
                            // Check for dotted imports - but only first-party ones
                            if module_name.contains('.') {
                                // Check if this dotted import refers to a first-party module
                                // by checking if any bundled module matches this dotted path
                                let is_first_party_dotted =
                                    params.modules.iter().any(|(name, _, _, _)| {
                                        name == module_name
                                            || module_name.starts_with(&format!("{name}."))
                                    });
                                if is_first_party_dotted {
                                    log::debug!(
                                        "Found first-party dotted import '{module_name}' that \
                                         requires namespace"
                                    );
                                    return true;
                                }
                            }
                            // NOTE: We can't check for direct imports of inlined modules here
                            // because self.inlined_modules isn't populated yet. That check
                            // happens later when we actually determine which modules to inline.
                            false
                        })
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        };

        // Trim unused imports from all modules
        // Note: stdlib import normalization now happens in the orchestrator
        // before dependency graph building, so imports are already normalized
        let mut modules = import_deduplicator::trim_unused_imports_from_modules(
            params.modules,
            params.graph,
            params.tree_shaker,
        )?;

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

        // Store entry path for relative path calculation
        if let Some((_, entry_path, _)) = params.sorted_modules.last() {
            self.entry_path = Some(entry_path.to_string_lossy().to_string());
        }

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
        // We need to include the entry module in this analysis since it might contain namespace
        // imports
        let mut all_modules_for_namespace_check = modules.clone();

        // Find the entry module from the topologically sorted modules
        for (module_name, ast, module_path, content_hash) in &modules {
            if module_name == params.entry_module_name {
                all_modules_for_namespace_check.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
                break;
            }
        }

        self.find_namespace_imported_modules(&all_modules_for_namespace_check);

        // Identify all modules that are part of circular dependencies
        let mut has_circular_dependencies = false;
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
            has_circular_dependencies = !self.circular_modules.is_empty();
            log::debug!("Circular modules: {:?}", self.circular_modules);
        } else {
            log::debug!("No circular dependency analysis provided");
        }

        // Separate modules into inlinable and wrapper modules
        // Note: modules are already normalized before unused import trimming
        let mut inlinable_modules = Vec::new();
        let mut wrapper_modules = Vec::new();
        let mut module_exports_map = FxIndexMap::default();

        for (module_name, ast, module_path, content_hash) in &modules {
            log::debug!("Processing module: '{module_name}'");
            if module_name == params.entry_module_name {
                continue;
            }

            // Extract __all__ exports from the module using ExportCollector
            let export_info = ExportCollector::analyze(ast);
            let has_explicit_all = export_info.exported_names.is_some();
            if has_explicit_all {
                self.modules_with_explicit_all.insert(module_name.clone());
            }

            // Convert export info to the format expected by the bundler
            let module_exports = if let Some(exported_names) = export_info.exported_names {
                Some(exported_names)
            } else {
                // If no __all__, collect all top-level symbols using SymbolCollector
                let collected = crate::visitors::symbol_collector::SymbolCollector::analyze(ast);
                let mut symbols: Vec<_> = collected
                    .global_symbols
                    .values()
                    .filter(|s| {
                        // Include all public symbols (not starting with underscore)
                        // except __all__ itself
                        // Dunder names (e.g., __version__, __author__, __doc__) are conventionally
                        // public
                        s.name != "__all__"
                            && (!s.name.starts_with('_')
                                || (s.name.starts_with("__") && s.name.ends_with("__")))
                    })
                    .map(|s| s.name.clone())
                    .collect();

                if symbols.is_empty() {
                    None
                } else {
                    // Sort symbols for deterministic output
                    symbols.sort();
                    Some(symbols)
                }
            };

            module_exports_map.insert(module_name.clone(), module_exports.clone());

            // Check if module is imported as a namespace
            let is_namespace_imported = self.namespace_imported_modules.contains_key(module_name);

            if is_namespace_imported {
                log::debug!(
                    "Module '{}' is imported as namespace by: {:?}",
                    module_name,
                    self.namespace_imported_modules.get(module_name)
                );
            }

            // With full static bundling, we only need to wrap modules with side effects
            // All imports are rewritten at bundle time, so namespace imports, direct imports,
            // and circular dependencies can all be handled through static transformation
            let has_side_effects = module_has_side_effects(ast);

            // Check if this module is in a circular dependency and accesses imported module
            // attributes
            let needs_wrapping_for_circular = self.circular_modules.contains(module_name)
                && self.module_accesses_imported_attributes(ast, module_name);

            // Check if this module has an invalid identifier (can't be imported normally)
            // These modules are likely imported via importlib and need to be wrapped
            // Note: Module names with dots are valid (e.g., "core.utils.helpers"), so we only
            // check if the module name itself (without dots) is invalid
            let module_base_name = module_name.split('.').next_back().unwrap_or(module_name);
            let has_invalid_identifier = !Self::is_valid_python_identifier(module_base_name);

            if has_side_effects || has_invalid_identifier || needs_wrapping_for_circular {
                if has_invalid_identifier {
                    log::debug!(
                        "Module '{module_name}' has invalid Python identifier - using wrapper \
                         approach"
                    );
                } else if needs_wrapping_for_circular {
                    log::debug!(
                        "Module '{module_name}' is in circular dependency and accesses imported \
                         attributes - using wrapper approach"
                    );
                } else {
                    log::debug!("Module '{module_name}' has side effects - using wrapper approach");
                }

                wrapper_modules.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            } else {
                log::debug!("Module '{module_name}' has no side effects - can be inlined");
                inlinable_modules.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            }
        }

        // Track which modules will be inlined (before wrapper module generation)
        for (module_name, _, _, _) in &inlinable_modules {
            self.inlined_modules.insert(module_name.clone());
            // Also store module exports for inlined modules
            self.module_exports.insert(
                module_name.clone(),
                module_exports_map.get(module_name).cloned().flatten(),
            );
        }

        // Identify required namespaces BEFORE inlining any modules
        // This is crucial for cases like 'requests' where the entry module has submodules
        let all_modules_for_namespace_detection = modules
            .iter()
            .map(|(name, ast, path, hash)| (name.clone(), ast.clone(), path.clone(), hash.clone()))
            .collect::<Vec<_>>();
        self.required_namespaces =
            NamespaceAnalyzer::identify_required_namespaces(&all_modules_for_namespace_detection);

        // If we need to create namespace statements, ensure types import is available
        if !self.required_namespaces.is_empty() {
            log::debug!(
                "Need to create {} namespace statements - adding types import",
                self.required_namespaces.len()
            );
            import_deduplicator::add_stdlib_import(self, "types");

            // Create namespace statements immediately after identifying them
            // This ensures namespaces exist before any module code that might reference them
            log::debug!(
                "Creating {} namespace statements before module inlining",
                self.required_namespaces.len()
            );
            let namespace_statements = namespace_manager::create_namespace_statements(self);
            final_body.extend(namespace_statements);

            // For wrapper modules that are submodules (e.g., requests.compat),
            // we need to create placeholder attributes on their parent namespaces
            // so that inlined code can reference them before they're initialized
            for (module_name, _, _, _) in &modules {
                if module_name.contains('.') && module_name != "__init__" {
                    // Check if this is a wrapper module
                    let is_wrapper = modules.iter().any(|(name, ast, _, _)| {
                        name == module_name && module_has_side_effects(ast)
                    });

                    if is_wrapper {
                        // Create a placeholder namespace attribute for this wrapper module
                        let parts: Vec<&str> = module_name.split('.').collect();
                        if parts.len() == 2 {
                            // Simple case like "requests.compat"
                            let parent = parts[0];
                            let child = parts[1];

                            // Check if the full namespace was already created
                            if !self.required_namespaces.contains(module_name) {
                                log::debug!(
                                    "Creating placeholder namespace attribute {parent}.{child} \
                                     for wrapper module"
                                );
                                let placeholder_stmt =
                                    namespace_manager::create_namespace_attribute(
                                        self, parent, child,
                                    );
                                final_body.push(placeholder_stmt);
                            } else {
                                log::debug!(
                                    "Skipping placeholder namespace attribute {parent}.{child} - \
                                     already created as full namespace"
                                );
                            }
                        }
                    }
                }
            }
        }

        // Now check if entry module has direct imports of inlined modules that have exports
        let needs_types_for_inlined_imports = if let Some((_, ast, _, _)) = modules
            .iter()
            .find(|(name, _, _, _)| name == params.entry_module_name)
        {
            ast.body.iter().any(|stmt| {
                if let Stmt::Import(import_stmt) = stmt {
                    import_stmt.names.iter().any(|alias| {
                        let module_name = alias.name.as_str();
                        // Check for direct imports of inlined modules that have exports
                        if self.inlined_modules.contains(module_name) {
                            // Check if the module has exports
                            if let Some(Some(exports)) = self.module_exports.get(module_name) {
                                let has_exports = !exports.is_empty();
                                if has_exports {
                                    log::debug!(
                                        "Direct import of inlined module '{module_name}' with \
                                         exports: {exports:?}"
                                    );
                                }
                                return has_exports;
                            }
                        }
                        false
                    })
                } else {
                    false
                }
            })
        } else {
            false
        };

        if needs_types_for_inlined_imports {
            log::debug!("Adding types import for inlined module imports in entry module");
            import_deduplicator::add_stdlib_import(self, "types");
        }

        // Collect imports from ALL modules (after normalization) for hoisting
        // This must be done on the normalized modules to capture stdlib imports
        // that were converted from "from X import Y" to "import X" format
        for (module_name, ast, _, _) in &modules {
            import_deduplicator::collect_imports_from_module(self, ast, module_name);
        }

        // If we have wrapper modules, inject types as stdlib dependency
        // functools will be added later only if we use module cache
        if !wrapper_modules.is_empty() {
            log::debug!("Adding types import for wrapper modules");
            import_deduplicator::add_stdlib_import(self, "types");
        }

        // If we have namespace imports, inject types as stdlib dependency
        if !self.namespace_imported_modules.is_empty() {
            log::debug!("Adding types import for namespace imports");
            import_deduplicator::add_stdlib_import(self, "types");
        }

        // If entry module has direct imports or dotted imports that need namespace objects
        if needs_types_for_entry_imports {
            log::debug!("Adding types import for namespace objects in entry module");
            import_deduplicator::add_stdlib_import(self, "types");
        }

        // We'll add types import later if we actually create namespace objects for importlib

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

        // Note: We'll add hoisted imports later after all transformations are done
        // to ensure we capture all needed imports (like types for namespace objects)

        // Check if we have wrapper modules
        let has_wrapper_modules = !wrapper_modules.is_empty();

        // Check if we need types import (for namespace imports)
        let _need_types_import = !self.namespace_imported_modules.is_empty();

        // Get symbol renames from semantic analysis
        let symbol_registry = params.semantic_bundler.symbol_registry();
        let mut symbol_renames = FxIndexMap::default();

        // Create semantic context
        let semantic_ctx = SemanticContext {
            graph: params.graph,
            symbol_registry,
            semantic_bundler: params.semantic_bundler,
        };

        // Convert ModuleId-based renames to module name-based renames
        for (module_name, _, _, _) in &modules {
            self.collect_module_renames(module_name, &semantic_ctx, &mut symbol_renames);
        }

        // Collect global symbols from the entry module first (for compatibility)
        let mut global_symbols =
            SymbolAnalyzer::collect_global_symbols(&modules, params.entry_module_name);

        // Save wrapper modules for later processing
        let wrapper_modules_saved = wrapper_modules;

        // Sort wrapper modules by their dependencies
        let sorted_wrapper_modules =
            self.sort_wrapper_modules_by_dependencies(&wrapper_modules_saved, params.graph)?;

        // Build symbol-level dependency graph for circular modules if needed
        if !self.circular_modules.is_empty() {
            log::debug!("Building symbol dependency graph for circular modules");

            // Convert modules to the format expected by build_symbol_dependency_graph
            let modules_for_graph: Vec<(String, ModModule, PathBuf, String)> = modules
                .iter()
                .map(|(name, ast, path, hash)| {
                    (name.clone(), ast.clone(), path.clone(), hash.clone())
                })
                .collect();

            self.symbol_dep_graph = SymbolAnalyzer::build_symbol_dependency_graph(
                &modules_for_graph,
                params.graph,
                &self.circular_modules,
            );

            // Get ordered symbols for circular modules
            match self
                .symbol_dep_graph
                .topological_sort_symbols(&self.circular_modules)
            {
                Ok(()) => {
                    log::debug!(
                        "Symbol ordering for circular modules: {:?}",
                        self.symbol_dep_graph.sorted_symbols
                    );
                }
                Err(e) => {
                    log::warn!("Failed to order symbols in circular modules: {e}");
                    // Continue with default ordering
                }
            }
        }

        // Generate pre-declarations only for symbols that actually need them
        let mut circular_predeclarations = Vec::new();
        if !self.circular_modules.is_empty() {
            log::debug!("Analyzing circular modules for necessary pre-declarations");

            // Collect all symbols that need pre-declaration based on actual forward references
            let mut symbols_needing_predeclaration = FxIndexSet::default();

            // First pass: Build a map of where each symbol will be defined in the final output
            let mut symbol_definition_order = FxIndexMap::default();
            let mut order_index = 0;

            for (module_name, _, _, _) in &inlinable_modules {
                if let Some(module_renames) = symbol_renames.get(module_name) {
                    for (original_name, _) in module_renames {
                        symbol_definition_order
                            .insert((module_name.clone(), original_name.clone()), order_index);
                        order_index += 1;
                    }
                }
            }

            // Second pass: Find actual forward references using module-level dependencies
            for ((module, symbol), module_level_deps) in
                &self.symbol_dep_graph.module_level_dependencies
            {
                if self.circular_modules.contains(module) && !module_level_deps.is_empty() {
                    // Check each module-level dependency
                    for (dep_module, dep_symbol) in module_level_deps {
                        if self.circular_modules.contains(dep_module) {
                            // Get the order indices
                            let symbol_order =
                                symbol_definition_order.get(&(module.clone(), symbol.clone()));
                            let dep_order = symbol_definition_order
                                .get(&(dep_module.clone(), dep_symbol.clone()));

                            if let (Some(&sym_idx), Some(&dep_idx)) = (symbol_order, dep_order) {
                                // Check if this creates a forward reference
                                if dep_idx > sym_idx {
                                    log::debug!(
                                        "Found forward reference: {module}.{symbol} (order \
                                         {sym_idx}) uses {dep_module}.{dep_symbol} (order \
                                         {dep_idx}) at module level"
                                    );
                                    symbols_needing_predeclaration
                                        .insert((dep_module.clone(), dep_symbol.clone()));
                                }
                            }
                        }
                    }
                }
            }

            // Now generate pre-declarations only for symbols that actually need them
            log::debug!("Symbols needing pre-declaration: {symbols_needing_predeclaration:?}");
            for (module_name, symbol_name) in symbols_needing_predeclaration {
                if let Some(module_renames) = symbol_renames.get(&module_name)
                    && let Some(renamed_name) = module_renames.get(&symbol_name)
                {
                    // Skip pre-declarations for built-in types
                    // Built-in types are always available and pre-declaring them as None causes
                    // issues
                    if ruff_python_stdlib::builtins::is_python_builtin(
                        renamed_name,
                        python_version,
                        false,
                    ) {
                        log::debug!(
                            "Skipping pre-declaration for built-in type: {renamed_name} (from \
                             {module_name}.{symbol_name})"
                        );
                        continue;
                    }

                    log::debug!(
                        "Pre-declaring {renamed_name} (from {module_name}.{symbol_name}) due to \
                         forward reference"
                    );
                    let mut stmt =
                        statements::simple_assign(renamed_name, expressions::none_literal());

                    // Set custom node index for tracking
                    if let Stmt::Assign(assign) = &mut stmt {
                        assign.node_index = self.create_transformed_node(format!(
                            "Pre-declaration for circular dependency: {renamed_name}"
                        ));
                    }

                    circular_predeclarations.push(stmt);

                    // Track the pre-declaration
                    self.circular_predeclarations
                        .entry(module_name.clone())
                        .or_default()
                        .insert(symbol_name.clone(), renamed_name.clone());
                }
            }
        }

        // Add pre-declarations at the very beginning
        final_body.extend(circular_predeclarations);

        // Decide early if we need module cache for circular dependencies
        let use_module_cache_for_wrappers =
            has_wrapper_modules && has_circular_dependencies && self.use_module_cache;
        if use_module_cache_for_wrappers {
            log::info!(
                "Detected circular dependencies in wrapper modules - will use module cache \
                 approach"
            );
        }

        // Add functools import for module cache decorators when we have wrapper modules to
        // transform
        if has_wrapper_modules {
            log::debug!("Adding functools import for module cache decorators");
            import_deduplicator::add_stdlib_import(self, "functools");
        }

        if use_module_cache_for_wrappers {
            // Detect hard dependencies in circular modules
            log::debug!("Scanning for hard dependencies in circular modules");

            // Need to scan ALL modules, not just wrapper modules
            let all_modules: Vec<(&String, &ModModule, &PathBuf, &String)> = inlinable_modules
                .iter()
                .map(|(name, ast, path, hash)| (name, ast, path, hash))
                .chain(
                    sorted_wrapper_modules
                        .iter()
                        .map(|(name, ast, path, hash)| (name, ast, path, hash)),
                )
                .collect();

            for (module_name, ast, module_path, _) in &all_modules {
                if self.circular_modules.contains(module_name.as_str()) {
                    // Build import map for this module
                    let mut import_map = FxIndexMap::default();

                    // Scan imports in the module
                    for stmt in &ast.body {
                        match stmt {
                            Stmt::Import(import_stmt) => {
                                for alias in &import_stmt.names {
                                    let imported_name = alias.name.as_str();
                                    let local_name = alias
                                        .asname
                                        .as_ref()
                                        .map(|n| n.as_str())
                                        .unwrap_or(imported_name);
                                    import_map.insert(
                                        local_name.to_string(),
                                        (
                                            imported_name.to_string(),
                                            alias.asname.as_ref().map(|n| n.as_str().to_string()),
                                        ),
                                    );
                                }
                            }
                            Stmt::ImportFrom(import_from) => {
                                // Handle relative imports
                                let resolved_module = if import_from.level > 0 {
                                    // Resolve relative import to absolute
                                    self.resolver.resolve_relative_to_absolute_module_name(
                                        import_from.level,
                                        import_from.module.as_ref().map(|m| m.as_str()),
                                        module_path,
                                    )
                                } else {
                                    import_from.module.as_ref().map(|m| m.as_str().to_string())
                                };

                                if let Some(module_str) = resolved_module {
                                    for alias in &import_from.names {
                                        let imported_name = alias.name.as_str();
                                        let local_name = alias
                                            .asname
                                            .as_ref()
                                            .map(|n| n.as_str())
                                            .unwrap_or(imported_name);

                                        // For "from X import Y", track the mapping
                                        let (actual_source, actual_import) =
                                            (module_str.clone(), Some(imported_name.to_string()));

                                        // Handle the alias if present
                                        import_map.insert(
                                            local_name.to_string(),
                                            (actual_source, actual_import),
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // Detect hard dependencies
                    let hard_deps =
                        SymbolAnalyzer::detect_hard_dependencies(module_name, ast, &import_map);
                    if !hard_deps.is_empty() {
                        log::info!(
                            "Found {} hard dependencies in module {}",
                            hard_deps.len(),
                            module_name
                        );
                        self.hard_dependencies.extend(hard_deps);
                    }
                }
            }

            if !self.hard_dependencies.is_empty() {
                log::info!(
                    "Total hard dependencies found: {}",
                    self.hard_dependencies.len()
                );
                for dep in &self.hard_dependencies {
                    log::debug!(
                        "  - Class {} in {} inherits from {} (source: {})",
                        dep.class_name,
                        dep.module_name,
                        dep.base_class,
                        dep.source_module
                    );
                }
            }
        }

        // Before inlining modules, check which wrapper modules they depend on
        let mut wrapper_modules_needed_by_inlined = FxIndexSet::default();
        for (module_name, ast, module_path, _) in &inlinable_modules {
            // Check imports in the module
            for stmt in &ast.body {
                if let Stmt::ImportFrom(import_from) = stmt {
                    // Resolve relative imports to absolute module names
                    let resolved_module = if import_from.level > 0 {
                        // This is a relative import, resolve it
                        self.resolver.resolve_relative_to_absolute_module_name(
                            import_from.level,
                            import_from.module.as_ref().map(|m| m.as_str()),
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

        // Process normalized imports from inlined modules to ensure they are hoisted
        for (_module_name, ast, _, _) in &inlinable_modules {
            // Scan for import statements and add normalized stdlib imports to our hoisted list
            for stmt in &ast.body {
                if let Stmt::Import(import_stmt) = stmt {
                    for alias in &import_stmt.names {
                        let module_name = alias.name.as_str();
                        if is_safe_stdlib_module(module_name) && alias.asname.is_none() {
                            // This is a normalized stdlib import (no alias), ensure it's
                            // hoisted
                            import_deduplicator::add_stdlib_import(self, module_name);
                        }
                    }
                }
            }
        }

        // If we're using module cache, add the infrastructure early
        if use_module_cache_for_wrappers {
            // Note: Module cache infrastructure removed - we don't use sys.modules anymore
        }

        // If there are wrapper modules needed by inlined modules, we need to define their
        // init functions BEFORE inlining the modules that use them
        if !wrapper_modules_needed_by_inlined.is_empty() && has_wrapper_modules {
            log::debug!(
                "Need to define wrapper module init functions early for: \
                 {wrapper_modules_needed_by_inlined:?}"
            );

            // Collect lifted declarations for needed wrapper modules
            // Process globals for the needed wrapper modules
            let mut module_globals = FxIndexMap::default();
            let mut lifted_declarations = Vec::new();

            for (module_name, ast, _, _) in &sorted_wrapper_modules {
                if wrapper_modules_needed_by_inlined.contains(module_name) {
                    let params = ProcessGlobalsParams {
                        module_name,
                        ast,
                        semantic_ctx: &semantic_ctx,
                    };
                    crate::code_generator::globals::process_wrapper_module_globals(
                        &params,
                        &mut module_globals,
                        &mut lifted_declarations,
                    );
                }
            }

            // Add lifted declarations if any
            if !lifted_declarations.is_empty() {
                debug!(
                    "Adding {} lifted global declarations for early wrapper modules",
                    lifted_declarations.len()
                );
                final_body.extend(lifted_declarations.clone());
                self.lifted_global_declarations.extend(lifted_declarations);
            }

            // Define the init functions for wrapper modules needed by inlined modules
            for (module_name, ast, module_path, _) in &sorted_wrapper_modules {
                if wrapper_modules_needed_by_inlined.contains(module_name) {
                    let synthetic_name = self.module_registry[module_name].clone();
                    let global_info = module_globals.get(module_name).cloned();
                    let ctx = ModuleTransformContext {
                        module_name,
                        synthetic_name: &synthetic_name,
                        module_path,
                        global_info,
                        semantic_bundler: Some(semantic_ctx.semantic_bundler),
                        python_version,
                    };
                    // Generate init function with empty symbol_renames for now
                    let empty_renames = FxIndexMap::default();
                    // Always use cached init functions to ensure modules are only initialized once
                    let init_function = self.transform_module_to_cache_init_function(
                        ctx,
                        ast.clone(),
                        &empty_renames,
                    )?;
                    final_body.push(init_function);

                    // Initialize the wrapper module immediately after defining it
                    // ONLY for non-module-cache mode
                    if !use_module_cache_for_wrappers {
                        let init_stmts =
                            crate::code_generator::module_registry::generate_module_init_call(
                                &synthetic_name,
                                module_name,
                                self.init_functions.get(&synthetic_name).map(|s| s.as_str()),
                                &self.module_registry,
                                |statements, module_name, init_result_var| {
                                    self.generate_merge_module_attributes(
                                        statements,
                                        module_name,
                                        init_result_var,
                                    );
                                },
                            );
                        final_body.extend(init_stmts);
                    } else {
                        // For module cache mode, initialization happens later in dependency order
                        // But if this wrapper module is a source of hard dependencies, we need to
                        // handle it specially to avoid forward reference
                        // errors
                        let is_hard_dep_source = self
                            .hard_dependencies
                            .iter()
                            .any(|dep| dep.source_module == *module_name);
                        if is_hard_dep_source {
                            // Don't initialize here - this would cause a forward reference
                            // Instead, we'll handle hard dependencies after all init functions are
                            // defined
                            log::debug!(
                                "Module {module_name} is a hard dependency source, deferring \
                                 initialization"
                            );
                        }
                    }
                    // Module is now initialized and assignments made
                }
            }
        }

        // Now that wrapper modules needed by inlined modules are initialized,
        // we can hoist hard dependencies
        if !self.hard_dependencies.is_empty() {
            log::info!("Hoisting hard dependencies after wrapper module initialization");

            // Group hard dependencies by source module
            let mut deps_by_source: FxIndexMap<String, Vec<&HardDependency>> =
                FxIndexMap::default();
            for dep in &self.hard_dependencies {
                deps_by_source
                    .entry(dep.source_module.clone())
                    .or_default()
                    .push(dep);
            }

            // Collect import statements to generate (to avoid borrow checker issues)
            let mut imports_to_generate: Vec<(String, Vec<(String, Option<String>)>, bool)> =
                Vec::new();

            // Analyze dependencies and determine what imports to generate
            for (source_module, deps) in deps_by_source {
                // Check if we need to import the whole module or specific attributes
                let first_dep = deps.first().expect("hard_deps should not be empty");

                if source_module == "http.cookiejar" && first_dep.imported_attr == "cookielib" {
                    // Special case: import http.cookiejar as cookielib
                    imports_to_generate.push((
                        source_module,
                        vec![("http.cookiejar".to_string(), Some("cookielib".to_string()))],
                        true,
                    ));
                } else {
                    // Check if the source module is a bundled wrapper module
                    let is_bundled_wrapper = wrapper_modules_saved
                        .iter()
                        .any(|(name, _, _, _)| name == &source_module);

                    if is_bundled_wrapper && use_module_cache_for_wrappers {
                        // The source module is a bundled wrapper module that will be initialized
                        // later We can't use a regular import, so we'll
                        // defer this until after module initialization
                        log::debug!(
                            "Deferring hard dependency imports from bundled wrapper module \
                             {source_module}"
                        );
                        // We'll handle these later after all modules are initialized
                    } else {
                        // Regular external module - collect unique imports with their aliases
                        let mut imports_to_make: FxIndexMap<String, Option<String>> =
                            FxIndexMap::default();
                        for dep in deps {
                            // If this dependency has a mandatory alias, use it
                            if dep.alias_is_mandatory && dep.alias.is_some() {
                                imports_to_make
                                    .insert(dep.imported_attr.clone(), dep.alias.clone());
                            } else {
                                // Only insert if we haven't already added this import
                                imports_to_make
                                    .entry(dep.imported_attr.clone())
                                    .or_insert(None);
                            }
                        }

                        if !imports_to_make.is_empty() {
                            let import_list: Vec<(String, Option<String>)> =
                                imports_to_make.into_iter().collect();
                            imports_to_generate.push((source_module, import_list, false));
                        }
                    }
                }
            }

            // Now generate the actual import statements
            for (source_module, imports, is_special_case) in imports_to_generate {
                if is_special_case {
                    // Special case: import http.cookiejar as cookielib
                    let import_stmt = StmtImport {
                        node_index: self.create_node_index(),
                        names: vec![ruff_python_ast::Alias {
                            node_index: self.create_node_index(),
                            name: Identifier::new("http.cookiejar", TextRange::default()),
                            asname: Some(Identifier::new("cookielib", TextRange::default())),
                            range: TextRange::default(),
                        }],
                        range: TextRange::default(),
                    };
                    final_body.push(Stmt::Import(import_stmt));
                    log::debug!("Hoisted import http.cookiejar as cookielib");
                } else {
                    // Generate: from source_module import attr1, attr2 as alias2, ...
                    let names: Vec<Alias> = imports
                        .into_iter()
                        .map(|(import_name, alias)| other::alias(&import_name, alias.as_deref()))
                        .collect();

                    let import_from = statements::import_from(Some(&source_module), names, 0);
                    final_body.push(import_from);
                    log::debug!("Hoisted imports from {source_module} for hard dependencies");
                }
            }
        }

        // Inline the inlinable modules FIRST to populate symbol_renames
        // This ensures we know what symbols have been renamed before processing wrapper modules and
        // namespace hybrids
        let mut all_deferred_imports = Vec::new();
        let mut all_inlined_stmts = Vec::new(); // Collect all inlined statements first

        for (module_name, ast, _module_path, _content_hash) in &inlinable_modules {
            log::debug!("Inlining module '{module_name}'");

            let mut inlined_stmts = Vec::new();
            let mut deferred_imports = Vec::new();
            let mut inline_ctx = InlineContext {
                module_exports_map: &module_exports_map,
                global_symbols: &mut global_symbols,
                module_renames: &mut symbol_renames,
                inlined_stmts: &mut inlined_stmts,
                import_aliases: FxIndexMap::default(),
                deferred_imports: &mut deferred_imports,
                import_sources: FxIndexMap::default(),
                python_version,
            };
            self.inline_module(module_name, ast.clone(), _module_path, &mut inline_ctx)?;
            log::debug!(
                "Inlined {} statements from module '{}'",
                inlined_stmts.len(),
                module_name
            );
            all_inlined_stmts.extend(inlined_stmts); // Collect instead of adding to final_body
            // Note: We don't track deferred imports globally anymore since we don't use sys.modules

            // Filter deferred imports to avoid conflicts
            // If an inlined module imports a symbol but doesn't export it,
            // and that symbol would conflict with other imports, skip it
            for stmt in deferred_imports {
                let should_include = if let Stmt::Assign(assign) = &stmt {
                    if let [Expr::Name(target)] = assign.targets.as_slice()
                        && let Expr::Name(_value) = &*assign.value
                    {
                        let symbol_name = target.id.as_str();

                        // Check if this module exports the symbol
                        let exports_symbol =
                            if let Some(Some(exports)) = module_exports_map.get(module_name) {
                                exports.contains(&symbol_name.to_string())
                            } else {
                                // No explicit __all__, check if it's a module-level definition
                                // For now, assume it's not exported if there's no __all__
                                false
                            };

                        if !exports_symbol {
                            // Check if this would conflict with existing deferred imports
                            let has_conflict = all_deferred_imports.iter().any(|existing| {
                                if let Stmt::Assign(existing_assign) = existing
                                    && let [Expr::Name(existing_target)] =
                                        existing_assign.targets.as_slice()
                                {
                                    existing_target.id.as_str() == symbol_name
                                } else {
                                    false
                                }
                            });

                            if has_conflict {
                                log::debug!(
                                    "Skipping deferred import '{symbol_name}' from module \
                                     '{module_name}' due to conflict"
                                );
                                false
                            } else {
                                true
                            }
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                } else {
                    true
                };

                if should_include {
                    // Check if this deferred import already exists in all_deferred_imports
                    let is_duplicate = if let Stmt::Assign(assign) = &stmt {
                        if let Expr::Name(target) = &assign.targets[0] {
                            let target_name = target.id.as_str();

                            // Check against existing deferred imports
                            all_deferred_imports.iter().any(|existing| {
                                if let Stmt::Assign(existing_assign) = existing
                                    && let [Expr::Name(existing_target)] =
                                        existing_assign.targets.as_slice()
                                    && existing_target.id.as_str() == target_name
                                {
                                    // Check if the values are the same
                                    return expression_handlers::expr_equals(
                                        &existing_assign.value,
                                        &assign.value,
                                    );
                                }
                                false
                            })
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !is_duplicate {
                        // Log what we're adding to deferred imports
                        if let Stmt::Assign(assign) = &stmt
                            && let Expr::Name(target) = &assign.targets[0]
                        {
                            if let Expr::Attribute(attr) = &assign.value.as_ref() {
                                let attr_path = expression_handlers::extract_attribute_path(attr);
                                log::debug!(
                                    "Adding to all_deferred_imports: {} = {} (from inlined module \
                                     '{}')",
                                    target.id.as_str(),
                                    attr_path,
                                    module_name
                                );
                            } else if let Expr::Name(value) = &assign.value.as_ref() {
                                log::debug!(
                                    "Adding to all_deferred_imports: {} = {} (from inlined module \
                                     '{}')",
                                    target.id.as_str(),
                                    value.id.as_str(),
                                    module_name
                                );
                            }
                        }
                        all_deferred_imports.push(stmt);
                    } else {
                        log::debug!(
                            "Skipping duplicate deferred import from module '{module_name}': \
                             {stmt:?}"
                        );
                    }
                }
            }
        }

        // Now reorder all collected inlined statements to ensure proper declaration order
        // This handles cross-module dependencies like classes inheriting from symbols defined in
        // other modules
        let mut reordered_inlined_stmts =
            self.reorder_cross_module_statements(all_inlined_stmts, python_version);

        // Filter out invalid assignments where we're trying to assign a module that uses an init
        // function For example, `mypkg.compat = compat` when `compat` is wrapped in an init
        // function
        self.filter_invalid_submodule_assignments(&mut reordered_inlined_stmts, None);

        final_body.extend(reordered_inlined_stmts);

        // Create namespace objects for inlined modules that are imported as namespaces
        log::debug!(
            "Checking namespace imports for {} inlinable modules",
            inlinable_modules.len()
        );
        log::debug!(
            "namespace_imported_modules: {:?}",
            self.namespace_imported_modules
        );

        // Also need to handle direct imports (like `import mypkg`) for modules with re-exports
        let directly_imported_modules =
            self.find_directly_imported_modules(params.modules, params.entry_module_name);
        log::debug!("directly_imported_modules: {directly_imported_modules:?}");

        for (module_name, _, _, _) in &inlinable_modules {
            log::debug!("Checking if module '{module_name}' needs namespace object");
            log::debug!(
                "  module_exports contains '{}': {}",
                module_name,
                self.module_exports.contains_key(module_name)
            );
            if let Some(exports) = self.module_exports.get(module_name) {
                log::debug!("  module '{module_name}' exports: {exports:?}");
            }

            // Skip the entry module - it doesn't need namespace assignments
            if module_name == params.entry_module_name {
                log::debug!("Skipping namespace creation for entry module '{module_name}'");
                continue;
            }

            // Check if module has exports
            let has_exports = self
                .module_exports
                .get(module_name)
                .is_some_and(|exports| exports.is_some());

            // Check if this is a submodule that needs a namespace
            let needs_namespace_for_submodule = self.submodule_needs_namespace(module_name);

            // Check if module needs a namespace object:
            // 1. It's imported as a namespace (import module)
            // 2. It's directly imported and has exports
            // 3. It's a submodule that's imported by its parent module via from . import
            let needs_namespace = self.namespace_imported_modules.contains_key(module_name)
                || (directly_imported_modules.contains(module_name) && has_exports)
                || needs_namespace_for_submodule;

            if needs_namespace {
                // Check if this namespace was already created
                let namespace_var = sanitize_module_name_for_identifier(module_name);
                let namespace_already_exists = self.created_namespaces.contains(&namespace_var);

                log::debug!(
                    "Namespace for inlined module '{module_name}' already exists: \
                     {namespace_already_exists}"
                );

                // Get the symbols that were inlined from this module
                if let Some(module_rename_map) = symbol_renames.get(module_name) {
                    log::debug!(
                        "Module '{}' has {} symbols in rename map: {:?}",
                        module_name,
                        module_rename_map.len(),
                        module_rename_map.keys().collect::<Vec<_>>()
                    );
                    if !namespace_already_exists {
                        // Check if this module should have an empty namespace due to forward
                        // references
                        let has_forward_references = self
                            .check_module_has_forward_references(module_name, module_rename_map);

                        // Create a SimpleNamespace for this module only if it doesn't exist
                        if let Some(namespace_stmt) =
                            namespace_manager::create_namespace_for_inlined_module_static(
                                self,
                                module_name,
                                module_rename_map,
                            )
                        {
                            final_body.push(namespace_stmt);
                        }

                        // Parent-child namespace assignments will be handled later by
                        // generate_submodule_attributes_with_exclusions, which runs after
                        // all namespaces have been created

                        // Only track as having initial symbols if we didn't create it empty
                        if !has_forward_references {
                            self.namespaces_with_initial_symbols
                                .insert(module_name.to_string());
                        } else {
                            // We created an empty namespace, need to populate it later
                            log::debug!(
                                "Created empty namespace for '{module_name}', will populate with \
                                 symbols later"
                            );
                        }
                    } else {
                        // Namespace already exists, we need to add symbols to it instead
                        log::debug!(
                            "Namespace '{namespace_var}' already exists, adding symbols to it"
                        );

                        // Add all renamed symbols as attributes to the existing namespace
                        for (original_name, renamed_name) in module_rename_map {
                            // Check if this symbol survived tree-shaking
                            if !self.is_symbol_kept_by_tree_shaking(module_name, original_name) {
                                log::debug!(
                                    "Skipping tree-shaken symbol '{original_name}' from namespace \
                                     for module '{module_name}'"
                                );
                                continue;
                            }

                            // Check if this namespace assignment has already been made
                            let assignment_key = (namespace_var.clone(), original_name.clone());
                            if self.namespace_assignments_made.contains(&assignment_key) {
                                log::debug!(
                                    "Skipping duplicate namespace assignment: \
                                     {namespace_var}.{original_name} = {renamed_name} (already \
                                     assigned)"
                                );
                                continue;
                            }

                            // Skip symbols that are re-exported from child modules
                            // These will be handled later by
                            // populate_namespace_with_module_symbols_with_renames
                            // Check if this symbol is in the exports list - if so, it's likely a
                            // re-export
                            let is_reexport = if module_name.contains('.') {
                                // For sub-packages, symbols are likely defined locally
                                false
                            } else if let Some(exports) = self.module_exports.get(module_name)
                                && let Some(export_list) = exports
                                && export_list.contains(original_name)
                            {
                                log::debug!(
                                    "Checking if '{original_name}' in module '{module_name}' is a \
                                     re-export from child modules"
                                );

                                // Check if symbol is actually defined in a child module
                                // by examining ASTs of child modules
                                let result = if let Some(module_asts) = &self.module_asts {
                                    module_asts.iter().any(|(inlined_module_name, ast, _, _)| {
                                        let is_child = inlined_module_name != module_name
                                            && inlined_module_name
                                                .starts_with(&format!("{module_name}."));
                                        if is_child {
                                            // Check if this module defines the symbol (as a class,
                                            // function, or variable)
                                            let defines_symbol =
                                                ast.body.iter().any(|stmt| match stmt {
                                                    Stmt::ClassDef(class_def) => {
                                                        class_def.name.id.as_str() == original_name
                                                    }
                                                    Stmt::FunctionDef(func_def) => {
                                                        func_def.name.id.as_str() == original_name
                                                    }
                                                    Stmt::Assign(assign) => {
                                                        assign.targets.iter().any(|target| {
                                                            if let Expr::Name(name) = target {
                                                                name.id.as_str() == original_name
                                                            } else {
                                                                false
                                                            }
                                                        })
                                                    }
                                                    _ => false,
                                                });
                                            if defines_symbol {
                                                log::debug!(
                                                    "  Child module '{inlined_module_name}' \
                                                     defines symbol '{original_name}' directly"
                                                );
                                            }
                                            defines_symbol
                                        } else {
                                            false
                                        }
                                    })
                                } else {
                                    // Fallback to checking rename maps if ASTs not available
                                    inlinable_modules.iter().any(
                                        |(inlined_module_name, _, _, _)| {
                                            let is_child = inlined_module_name != module_name
                                                && inlined_module_name
                                                    .starts_with(&format!("{module_name}."));
                                            if is_child {
                                                let has_symbol = symbol_renames
                                                    .get(inlined_module_name)
                                                    .is_some_and(|child_renames| {
                                                        child_renames.contains_key(original_name)
                                                    });
                                                log::debug!(
                                                    "  Child module '{inlined_module_name}' has \
                                                     symbol '{original_name}' in rename map: \
                                                     {has_symbol}"
                                                );
                                                has_symbol
                                            } else {
                                                false
                                            }
                                        },
                                    )
                                };
                                log::debug!("  Result: is_reexport = {result}");
                                result
                            } else {
                                false
                            };

                            if is_reexport {
                                log::debug!(
                                    "Skipping namespace assignment for re-exported symbol \
                                     {namespace_var}.{original_name} = {renamed_name} - will be \
                                     handled by \
                                     populate_namespace_with_module_symbols_with_renames"
                                );
                                continue;
                            }

                            // Create assignment: namespace.original_name = renamed_name
                            let assign_stmt = statements::assign(
                                vec![expressions::attribute(
                                    expressions::name(&namespace_var, ExprContext::Load),
                                    original_name,
                                    ExprContext::Store,
                                )],
                                expressions::name(renamed_name, ExprContext::Load),
                            );
                            final_body.push(assign_stmt);

                            // Track that we've made this assignment
                            self.namespace_assignments_made.insert(assignment_key);

                            // Track that this symbol was added when namespace already existed
                            self.symbols_populated_after_deferred
                                .insert((module_name.to_string(), original_name.clone()));
                        }

                        // Also check for module-level variables that weren't renamed
                        // Skip this for the entry module to avoid duplicate assignments
                        if module_name != params.entry_module_name
                            && let Some(exports) = self.module_exports.get(module_name).cloned()
                            && let Some(export_list) = exports
                        {
                            log::debug!(
                                "Module '{module_name}' has __all__ exports: {export_list:?}"
                            );
                            log::debug!(
                                "Module rename map keys: {:?}",
                                module_rename_map.keys().collect::<Vec<_>>()
                            );

                            for export in export_list {
                                // Check if this export was already added as a renamed symbol
                                if !module_rename_map.contains_key(&export) {
                                    log::debug!(
                                        "Export '{export}' not in module_rename_map, will add to \
                                         namespace"
                                    );
                                    // Check if this symbol survived tree-shaking
                                    if !self.is_symbol_kept_by_tree_shaking(module_name, &export) {
                                        log::debug!(
                                            "Skipping tree-shaken export '{export}' from \
                                             namespace for module '{module_name}'"
                                        );
                                        continue;
                                    }

                                    // Check if this namespace assignment has already been made
                                    let assignment_key = (namespace_var.clone(), export.clone());
                                    if self.namespace_assignments_made.contains(&assignment_key) {
                                        log::debug!(
                                            "Skipping duplicate namespace assignment: \
                                             {namespace_var}.{export} = {export} (already \
                                             assigned)"
                                        );
                                        continue;
                                    }

                                    // Also check if this assignment already exists in final_body
                                    // This handles cases where the assignment was created elsewhere
                                    let assignment_exists_in_body = final_body.iter().any(|stmt| {
                                        if let Stmt::Assign(assign) = stmt
                                            && assign.targets.len() == 1
                                            && let Expr::Attribute(attr) = &assign.targets[0]
                                            && let Expr::Name(base) = attr.value.as_ref()
                                        {
                                            return base.id.as_str() == namespace_var
                                                && attr.attr.as_str() == export;
                                        }
                                        false
                                    });

                                    if assignment_exists_in_body {
                                        log::debug!(
                                            "Skipping namespace assignment \
                                             {namespace_var}.{export} = {export} - already exists \
                                             in final_body"
                                        );
                                        // Track it so we don't create it again elsewhere
                                        self.namespace_assignments_made.insert(assignment_key);
                                        continue;
                                    }

                                    // Check if this export is a submodule
                                    // Only skip if it's actually a module (not just a symbol that
                                    // happens to match a module path)
                                    let full_submodule_path = format!("{module_name}.{export}");

                                    if self.bundled_modules.contains(&full_submodule_path) {
                                        log::debug!(
                                            "Export '{export}' is a bundled submodule: \
                                             {full_submodule_path}"
                                        );
                                        // Check if it's inlined or uses an init function
                                        let is_inlined =
                                            self.inlined_modules.contains(&full_submodule_path);
                                        // Check if this module has an init function (meaning it's
                                        // wrapped, not inlined)
                                        let uses_init_function = self
                                            .module_registry
                                            .get(&full_submodule_path)
                                            .and_then(|synthetic_name| {
                                                self.init_functions.get(synthetic_name)
                                            })
                                            .is_some();

                                        if uses_init_function {
                                            // This is a submodule that uses an init function
                                            // The assignment will be handled by init function call
                                            log::debug!(
                                                "Skipping namespace assignment for \
                                                 '{namespace_var}.{export}' - it uses an init \
                                                 function"
                                            );
                                            // Track that we've handled this to prevent duplicate
                                            // assignments
                                            self.namespace_assignments_made.insert(assignment_key);
                                            continue;
                                        } else if is_inlined {
                                            // This is an inlined submodule
                                            // When a submodule is inlined, it creates a local
                                            // variable with the submodule name
                                            // We need to create the assignment: parent.submodule =
                                            // submodule
                                            log::debug!(
                                                "Export '{export}' in module '{module_name}' is \
                                                 an inlined submodule - will create assignment"
                                            );

                                            // Create the assignment but add it to
                                            // all_deferred_imports instead
                                            let assign_stmt = statements::assign(
                                                vec![expressions::attribute(
                                                    expressions::name(
                                                        &namespace_var,
                                                        ExprContext::Load,
                                                    ),
                                                    &export,
                                                    ExprContext::Store,
                                                )],
                                                expressions::name(&export, ExprContext::Load),
                                            );
                                            all_deferred_imports.push(assign_stmt);

                                            // Track that we've made this assignment
                                            self.namespace_assignments_made.insert(assignment_key);
                                            continue;
                                        } else {
                                            // This is a submodule but neither inlined nor using
                                            // init function
                                            // This shouldn't happen for bundled modules
                                            log::debug!(
                                                "Unexpected state: submodule \
                                                 '{full_submodule_path}' is bundled but neither \
                                                 inlined nor using init function"
                                            );
                                            continue;
                                        }
                                    }

                                    // This export wasn't renamed, add it directly
                                    log::debug!(
                                        "Creating namespace assignment for unrenamed export: \
                                         {namespace_var}.{export} = {export}"
                                    );
                                    log::debug!(
                                        "  DEBUG: module_name='{module_name}', \
                                         namespace_var='{namespace_var}', export='{export}'"
                                    );

                                    // Double-check if this is actually a bundled module
                                    let actual_full_path = format!("{module_name}.{export}");

                                    // Final check: make sure this is not a module at all
                                    let is_any_kind_of_module =
                                        self.bundled_modules.contains(&actual_full_path)
                                            || self.module_registry.contains_key(&actual_full_path)
                                            || self.inlined_modules.contains(&actual_full_path);

                                    if is_any_kind_of_module {
                                        log::debug!(
                                            "Skipping assignment for {namespace_var}.{export} - \
                                             it's a module"
                                        );
                                        self.namespace_assignments_made.insert(assignment_key);
                                        continue;
                                    }

                                    log::debug!(
                                        "Creating unrenamed export assignment: \
                                         {namespace_var}.{export} = {export} for module \
                                         {module_name}"
                                    );
                                    let assign_stmt = statements::assign(
                                        vec![expressions::attribute(
                                            expressions::name(&namespace_var, ExprContext::Load),
                                            &export,
                                            ExprContext::Store,
                                        )],
                                        expressions::name(&export, ExprContext::Load),
                                    );
                                    final_body.push(assign_stmt);

                                    // Track that we've made this assignment
                                    self.namespace_assignments_made.insert(assignment_key);
                                }
                            }
                        }
                    }

                    // Track the created namespace to prevent duplicate creation later
                    let namespace_var = sanitize_module_name_for_identifier(module_name);
                    self.created_namespaces.insert(namespace_var);
                } else if self.inlined_modules.contains(module_name)
                    && !self.module_registry.contains_key(module_name)
                {
                    // Module has no symbols in symbol_renames but is still an inlined module
                    // We need to create an empty namespace for it
                    log::debug!(
                        "Module '{module_name}' has no symbols but needs a namespace object"
                    );

                    let namespace_var = sanitize_module_name_for_identifier(module_name);
                    let namespace_already_exists = self.created_namespaces.contains(&namespace_var);

                    if !namespace_already_exists {
                        // Create empty namespace = types.SimpleNamespace()
                        let namespace_stmt = statements::simple_assign(
                            &namespace_var,
                            expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
                        );
                        final_body.push(namespace_stmt);

                        // Track the created namespace
                        self.created_namespaces.insert(namespace_var);
                    }
                }
            }
        }

        // NOTE: Namespace population moved to after deferred imports are added to avoid forward
        // reference errors

        // Module cache infrastructure was already added earlier if needed

        // Now transform wrapper modules into init functions AFTER inlining
        // This way we have access to symbol_renames for proper import resolution
        if has_wrapper_modules {
            // Process all wrapper modules for globals
            let mut module_globals = FxIndexMap::default();
            let mut all_lifted_declarations = Vec::new();
            for (module_name, ast, _, _) in &sorted_wrapper_modules {
                let params = ProcessGlobalsParams {
                    module_name,
                    ast,
                    semantic_ctx: &semantic_ctx,
                };
                crate::code_generator::globals::process_wrapper_module_globals(
                    &params,
                    &mut module_globals,
                    &mut all_lifted_declarations,
                );
            }

            // Store all lifted declarations
            debug!(
                "Collected {} total lifted declarations",
                all_lifted_declarations.len()
            );
            self.lifted_global_declarations = all_lifted_declarations.clone();

            // Add lifted global declarations to final body before init functions
            if !all_lifted_declarations.is_empty() {
                debug!(
                    "Adding {} lifted global declarations to final body",
                    all_lifted_declarations.len()
                );
                final_body.extend(all_lifted_declarations);
            }

            // Second pass: transform modules with global info
            for (module_name, ast, module_path, _content_hash) in &sorted_wrapper_modules {
                // Skip modules that were already defined early for inlined module dependencies
                if wrapper_modules_needed_by_inlined.contains(module_name) {
                    log::debug!("Skipping wrapper module '{module_name}' - already defined early");
                    continue;
                }

                let synthetic_name = self.module_registry[module_name].clone();
                let global_info = module_globals.get(module_name).cloned();
                let ctx = ModuleTransformContext {
                    module_name,
                    synthetic_name: &synthetic_name,
                    module_path,
                    global_info,
                    semantic_bundler: Some(semantic_ctx.semantic_bundler),
                    python_version,
                };
                // Always use cached init functions to ensure modules are only initialized once
                let init_function = self.transform_module_to_cache_init_function(
                    ctx,
                    ast.clone(),
                    &symbol_renames,
                )?;
                final_body.push(init_function);
            }

            // Now add the registries after init functions are defined
            final_body
                .extend(crate::code_generator::module_registry::generate_registries_and_hook());
        }

        // Initialize wrapper modules in dependency order AFTER inlined modules are defined
        if has_wrapper_modules {
            debug!("Creating parent namespaces before module initialization");

            // Note: Namespace identification and creation already happened before module inlining
            // to prevent forward reference errors

            debug!("Initializing modules in order:");

            // First, collect all wrapped modules that need initialization
            let mut wrapped_modules_to_init = Vec::new();
            for (module_name, _, _) in params.sorted_modules {
                if module_name == params.entry_module_name {
                    continue;
                }
                if self.module_registry.contains_key(module_name) {
                    wrapped_modules_to_init.push(module_name.clone());
                }
            }

            // Sort wrapped modules by their dependencies to ensure correct initialization order
            // This is critical for namespace imports in circular dependencies
            debug!("Wrapped modules before sorting: {wrapped_modules_to_init:?}");
            let sorted_wrapped = DependencyAnalyzer::sort_wrapped_modules_by_dependencies(
                wrapped_modules_to_init,
                params.graph,
            );
            debug!("Wrapped modules after sorting: {sorted_wrapped:?}");

            // When using module cache, we must initialize all modules immediately
            // to populate their namespaces
            if use_module_cache_for_wrappers {
                log::info!("Using module cache - initializing all modules immediately");

                // Call all init functions in sorted order
                for module_name in &sorted_wrapped {
                    if let Some(synthetic_name) = self.module_registry.get(module_name) {
                        let init_func_name = &self.init_functions[synthetic_name];

                        // Generate init call and assignment
                        let init_call = expressions::call(
                            expressions::name(init_func_name, ExprContext::Load),
                            vec![],
                            vec![],
                        );

                        // Generate the appropriate assignment based on module type
                        let init_stmts =
                            self.generate_module_assignment_from_init(module_name, init_call);
                        final_body.extend(init_stmts);

                        // Extract hard dependencies from this module immediately after
                        // initialization This is critical for modules that
                        // are sources of hard dependencies
                        if self
                            .hard_dependencies
                            .iter()
                            .any(|dep| dep.source_module == *module_name)
                        {
                            log::debug!(
                                "Module {module_name} is a hard dependency source, extracting \
                                 dependencies immediately"
                            );

                            for dep in &self.hard_dependencies {
                                if dep.source_module == *module_name {
                                    let target_name =
                                        if dep.alias_is_mandatory && dep.alias.is_some() {
                                            dep.alias.as_ref().expect(
                                                "Alias should be present when alias_is_mandatory \
                                                 is true",
                                            )
                                        } else {
                                            &dep.imported_attr
                                        };

                                    // Generate: target_name = module_name.imported_attr
                                    let module_parts: Vec<&str> = module_name.split('.').collect();
                                    let module_expr =
                                        expressions::dotted_name(&module_parts, ExprContext::Load);
                                    let assign_stmt = statements::simple_assign(
                                        target_name,
                                        expressions::attribute(
                                            module_expr,
                                            &dep.imported_attr,
                                            ExprContext::Load,
                                        ),
                                    );

                                    final_body.push(assign_stmt);
                                    log::debug!(
                                        "Generated immediate assignment: {} = {}.{}",
                                        target_name,
                                        module_name,
                                        dep.imported_attr
                                    );
                                }
                            }
                        }
                    }
                }
            } else {
                // DO NOT initialize modules here - they should be initialized when imported
                // This preserves Python's lazy import semantics
                debug!("Skipping eager module initialization - modules will initialize on import");
            }

            // After all modules are initialized, assign temporary variables to their namespace
            // locations For parent modules that are also wrapper modules, we need to
            // merge their attributes
            for module_name in &sorted_wrapped {
                // Direct module name instead of temp variable
                // No longer need to track sanitized names since we use direct assignment

                // Check if this module has submodules (is a parent module)
                let is_parent_module = sorted_wrapped
                    .iter()
                    .any(|m| m != module_name && m.starts_with(&format!("{module_name}.")));

                if module_name.contains('.') {
                    // For dotted modules, check if they have their own submodules
                    // If they do, we need to merge attributes instead of overwriting
                    if is_parent_module {
                        debug!(
                            "Dotted module '{module_name}' is both a wrapper module and parent \
                             namespace"
                        );
                        // We need to merge the wrapper module's attributes into the existing
                        // namespace Get the parts to construct the
                        // namespace path
                        let parts: Vec<&str> = module_name.split('.').collect();
                        let mut namespace_path = String::new();
                        for (i, part) in parts.iter().enumerate() {
                            if i > 0 {
                                namespace_path.push('.');
                            }
                            namespace_path.push_str(part);
                        }

                        // For dotted parent modules, they were already handled during init
                        debug!(
                            "Dotted parent module '{module_name}' already had attributes merged \
                             during init"
                        );
                    } else {
                        // Dotted modules are already assigned during init via attribute expressions
                        debug!("Dotted module '{module_name}' already assigned during init");
                    }
                } else {
                    // For simple module names that are parent modules, we need to merge attributes
                    if is_parent_module {
                        debug!(
                            "Module '{module_name}' is both a wrapper module and parent namespace"
                        );
                        // Parent modules were already handled during init with merge logic
                        debug!(
                            "Parent module '{module_name}' already had attributes merged during \
                             init"
                        );
                    } else {
                        // Simple modules are already assigned during init via direct assignment
                        debug!("Module '{module_name}' already assigned during init");
                    }
                }
            }

            // Track which hard dependencies we've already processed
            let mut processed_hard_deps: FxIndexSet<(String, String)> = FxIndexSet::default();

            // Mark hard dependencies that were processed during module initialization
            if use_module_cache_for_wrappers {
                let sorted_wrapped_set: crate::types::FxIndexSet<_> =
                    sorted_wrapped.iter().cloned().collect();
                for dep in &self.hard_dependencies {
                    if sorted_wrapped_set.contains(&dep.source_module) {
                        let target_name = if dep.alias_is_mandatory && dep.alias.is_some() {
                            dep.alias
                                .as_ref()
                                .expect("Alias should be present when alias_is_mandatory is true")
                        } else {
                            &dep.imported_attr
                        };
                        processed_hard_deps
                            .insert((dep.source_module.clone(), target_name.clone()));
                    }
                }
            }

            // Mark the ones we processed earlier as already handled
            for module_name in &wrapper_modules_needed_by_inlined {
                if self
                    .hard_dependencies
                    .iter()
                    .any(|dep| dep.source_module == *module_name)
                {
                    for dep in &self.hard_dependencies {
                        if dep.source_module == *module_name {
                            let target_name = if dep.alias_is_mandatory && dep.alias.is_some() {
                                dep.alias.as_ref().expect(
                                    "Alias should be present when alias_is_mandatory is true",
                                )
                            } else {
                                &dep.imported_attr
                            };
                            processed_hard_deps
                                .insert((dep.source_module.clone(), target_name.clone()));
                        }
                    }
                }
            }

            // Now handle deferred hard dependencies from bundled wrapper modules
            if !self.hard_dependencies.is_empty() && use_module_cache_for_wrappers {
                log::debug!("Processing deferred hard dependencies from bundled wrapper modules");

                // Group hard dependencies by source module again
                let mut deps_by_source: FxIndexMap<String, Vec<&HardDependency>> =
                    FxIndexMap::default();
                for dep in &self.hard_dependencies {
                    // Only process dependencies from bundled wrapper modules
                    if wrapper_modules_saved
                        .iter()
                        .any(|(name, _, _, _)| name == &dep.source_module)
                    {
                        let target_name = if dep.alias_is_mandatory && dep.alias.is_some() {
                            dep.alias
                                .as_ref()
                                .expect("Alias should be present when alias_is_mandatory is true")
                        } else {
                            &dep.imported_attr
                        };

                        // Skip if we already processed this dependency
                        if processed_hard_deps
                            .contains(&(dep.source_module.clone(), target_name.clone()))
                        {
                            log::debug!(
                                "Skipping already processed hard dependency: {} from {}",
                                target_name,
                                dep.source_module
                            );
                            continue;
                        }

                        deps_by_source
                            .entry(dep.source_module.clone())
                            .or_default()
                            .push(dep);
                    }
                }

                // Generate attribute assignments for bundled wrapper module dependencies
                for (source_module, deps) in deps_by_source {
                    log::debug!(
                        "Generating assignments for hard dependencies from bundled module \
                         {source_module}"
                    );

                    for dep in deps {
                        // Use the same logic as hard dependency rewriting
                        let target_name = if dep.alias_is_mandatory && dep.alias.is_some() {
                            dep.alias
                                .as_ref()
                                .expect("Alias should be present when alias_is_mandatory is true")
                        } else {
                            &dep.imported_attr
                        };

                        // Generate: target_name = source_module.imported_attr
                        let module_parts: Vec<&str> = source_module.split('.').collect();
                        let module_expr =
                            expressions::dotted_name(&module_parts, ExprContext::Load);
                        let assign_stmt = statements::simple_assign(
                            target_name,
                            expressions::attribute(
                                module_expr,
                                &dep.imported_attr,
                                ExprContext::Load,
                            ),
                        );

                        final_body.push(assign_stmt);
                        log::debug!(
                            "Generated assignment: {} = {}.{}",
                            target_name,
                            source_module,
                            dep.imported_attr
                        );
                    }
                }
            }
        }

        // After all modules are initialized, ensure sub-modules are attached to parent modules
        // This is necessary for relative imports like "from . import messages" to work
        // correctly, and also for inlined submodules to be attached to their parent namespaces
        // Check what modules are imported in the entry module to avoid duplicates
        // Recreate all_modules for this check
        let all_modules = inlinable_modules
            .iter()
            .chain(sorted_wrapper_modules.iter())
            .cloned()
            .collect::<Vec<_>>();
        let entry_imported_modules =
            self.get_entry_module_imports(&all_modules, params.entry_module_name);

        debug!(
            "About to generate submodule attributes, current body length: {}",
            final_body.len()
        );
        namespace_manager::generate_submodule_attributes_with_exclusions(
            self,
            params.sorted_modules,
            &mut final_body,
            &entry_imported_modules,
        );
        debug!(
            "After generate_submodule_attributes, body length: {}",
            final_body.len()
        );

        // Add deferred imports from inlined modules before entry module code
        // This ensures they're available when the entry module code runs
        if !all_deferred_imports.is_empty() {
            log::debug!(
                "Adding {} deferred imports from inlined modules before entry module",
                all_deferred_imports.len()
            );

            // Log what deferred imports we have
            for (i, stmt) in all_deferred_imports.iter().enumerate() {
                if let Stmt::Assign(assign) = stmt
                    && let Expr::Name(target) = &assign.targets[0]
                {
                    log::debug!("  Deferred import {}: {} = ...", i, target.id.as_str());
                }
            }

            // Filter out init calls - they should already be added when wrapper modules were
            // initialized
            let imports_without_init_calls: Vec<Stmt> = all_deferred_imports
                .iter()
                .filter(|stmt| {
                    // Skip init calls
                    if let Stmt::Expr(expr_stmt) = stmt
                        && let Expr::Call(call) = &expr_stmt.value.as_ref()
                        && let Expr::Name(name) = &call.func.as_ref()
                    {
                        return !crate::code_generator::module_registry::is_init_function(
                            name.id.as_str(),
                        );
                    }
                    true
                })
                .cloned()
                .collect();

            // Then add the deferred imports (without init calls)
            // Pass the current final_body so we can check for existing assignments
            let num_imports_before = imports_without_init_calls.len();
            log::debug!(
                "About to deduplicate {} deferred imports against {} existing statements",
                num_imports_before,
                final_body.len()
            );

            let mut deduped_imports =
                import_deduplicator::deduplicate_deferred_imports_with_existing(
                    imports_without_init_calls,
                    &final_body,
                );
            log::debug!(
                "After deduplication: {} imports remain from {} original",
                deduped_imports.len(),
                num_imports_before
            );

            // Filter out invalid assignments where the RHS references a module that uses an init
            // function For example, `mypkg.compat = compat` when `compat` is wrapped in
            // an init function
            self.filter_invalid_submodule_assignments(&mut deduped_imports, None);

            // Sort deferred imports to ensure namespace creations come before assignments that use
            // them This prevents forward reference errors like "NameError: name
            // 'compat' is not defined"
            self.sort_deferred_imports_for_dependencies(&mut deduped_imports);

            final_body.extend(deduped_imports);

            // Clear the collection so we don't add them again later
            all_deferred_imports.clear();
        }

        // After processing all inlined modules and deferred imports, populate empty namespaces with
        // their symbols This must happen AFTER deferred imports are added to avoid forward
        // reference errors
        for (module_name, _, _, _) in &inlinable_modules {
            // Skip if this module was created with initial symbols
            if self.namespaces_with_initial_symbols.contains(module_name) {
                continue;
            }

            // Check if this module has a namespace that needs population
            let namespace_var = sanitize_module_name_for_identifier(module_name);
            if self.created_namespaces.contains(&namespace_var) {
                log::debug!("Populating empty namespace '{namespace_var}' with symbols");

                // Don't mark the module as fully populated yet, we'll track individual symbols

                // Get the symbols that were inlined from this module
                if let Some(module_rename_map) = symbol_renames.get(module_name) {
                    // Add all renamed symbols as attributes to the namespace
                    for (original_name, renamed_name) in module_rename_map {
                        // Check if this symbol survived tree-shaking
                        if !self.is_symbol_kept_by_tree_shaking(module_name, original_name) {
                            log::debug!(
                                "Skipping tree-shaken symbol '{original_name}' from namespace for \
                                 module '{module_name}'"
                            );
                            continue;
                        }

                        // Skip symbols that are re-exported from child modules
                        // These will be handled later by
                        // populate_namespace_with_module_symbols_with_renames
                        // Check if this symbol is in the exports list - if so, it's likely a
                        // re-export
                        let is_reexport = if module_name.contains('.') {
                            // For sub-packages, symbols are likely defined locally
                            false
                        } else if let Some(exports) = self.module_exports.get(module_name)
                            && let Some(export_list) = exports
                            && export_list.contains(original_name)
                        {
                            log::debug!(
                                "Checking if '{original_name}' in module '{module_name}' is a \
                                 re-export from child modules"
                            );
                            // Check if symbol is actually defined in a child module
                            // by examining ASTs of child modules
                            let result = if let Some(module_asts) = &self.module_asts {
                                module_asts.iter().any(|(inlined_module_name, ast, _, _)| {
                                    let is_child = inlined_module_name != module_name
                                        && inlined_module_name
                                            .starts_with(&format!("{module_name}."));
                                    if is_child {
                                        // Check if this module defines the symbol (as a class,
                                        // function, or variable)
                                        let defines_symbol =
                                            ast.body.iter().any(|stmt| match stmt {
                                                Stmt::ClassDef(class_def) => {
                                                    class_def.name.id.as_str() == original_name
                                                }
                                                Stmt::FunctionDef(func_def) => {
                                                    func_def.name.id.as_str() == original_name
                                                }
                                                Stmt::Assign(assign) => {
                                                    assign.targets.iter().any(|target| {
                                                        if let Expr::Name(name) = target {
                                                            name.id.as_str() == original_name
                                                        } else {
                                                            false
                                                        }
                                                    })
                                                }
                                                _ => false,
                                            });
                                        if defines_symbol {
                                            log::debug!(
                                                "  Child module '{inlined_module_name}' defines \
                                                 symbol '{original_name}' directly"
                                            );
                                        }
                                        defines_symbol
                                    } else {
                                        false
                                    }
                                })
                            } else {
                                // Fallback to checking rename maps if ASTs not available
                                inlinable_modules
                                    .iter()
                                    .any(|(inlined_module_name, _, _, _)| {
                                        let is_child = inlined_module_name != module_name
                                            && inlined_module_name
                                                .starts_with(&format!("{module_name}."));
                                        if is_child {
                                            let has_symbol = symbol_renames
                                                .get(inlined_module_name)
                                                .is_some_and(|renames| {
                                                    renames.contains_key(original_name)
                                                });
                                            if has_symbol {
                                                log::debug!(
                                                    "  Child module '{inlined_module_name}' has \
                                                     symbol '{original_name}' in rename map"
                                                );
                                            }
                                            has_symbol
                                        } else {
                                            false
                                        }
                                    })
                            };
                            log::debug!(
                                "  Symbol '{original_name}' is re-export from child modules: \
                                 {result}"
                            );
                            result
                        } else {
                            false
                        };

                        if is_reexport {
                            log::debug!(
                                "Skipping namespace assignment for re-exported symbol \
                                 {namespace_var}.{original_name} = {renamed_name} - will be \
                                 handled by populate_namespace_with_module_symbols_with_renames"
                            );
                            continue;
                        }

                        // Check if this namespace assignment has already been made
                        let assignment_key = (namespace_var.clone(), original_name.clone());
                        if self.namespace_assignments_made.contains(&assignment_key) {
                            log::debug!(
                                "Skipping duplicate namespace assignment: \
                                 {namespace_var}.{original_name} = {renamed_name} (already \
                                 assigned)"
                            );
                            continue;
                        }

                        // Also check if this assignment already exists in final_body (may have been
                        // added by populate_namespace_with_module_symbols_with_renames)
                        let assignment_exists = final_body.iter().any(|stmt| {
                            if let Stmt::Assign(assign) = stmt
                                && assign.targets.len() == 1
                                && let Expr::Attribute(attr) = &assign.targets[0]
                                && let Expr::Name(base) = attr.value.as_ref()
                                && let Expr::Name(value) = assign.value.as_ref()
                            {
                                return base.id.as_str() == namespace_var
                                    && attr.attr.as_str() == original_name
                                    && value.id.as_str() == renamed_name;
                            }
                            false
                        });

                        if assignment_exists {
                            log::debug!(
                                "Skipping duplicate namespace assignment: \
                                 {namespace_var}.{original_name} = {renamed_name} (already exists \
                                 in final_body)"
                            );
                            continue;
                        }

                        // Check if this symbol is actually a submodule
                        let full_submodule_path = format!("{module_name}.{original_name}");
                        if self.bundled_modules.contains(&full_submodule_path) {
                            log::debug!(
                                "Skipping namespace assignment for '{original_name}' in module \
                                 '{module_name}' - it's a submodule, not a symbol"
                            );
                            continue;
                        }

                        // Create assignment: namespace.original_name = renamed_name
                        log::debug!(
                            "Creating namespace assignment in empty namespace population: \
                             {namespace_var}.{original_name} = {renamed_name}"
                        );
                        let assign_stmt = statements::assign(
                            vec![expressions::attribute(
                                expressions::name(&namespace_var, ExprContext::Load),
                                original_name,
                                ExprContext::Store,
                            )],
                            expressions::name(renamed_name, ExprContext::Load),
                        );

                        final_body.push(assign_stmt);

                        // Track that we've made this assignment
                        self.namespace_assignments_made
                            .insert(assignment_key.clone());

                        // Track that this symbol was populated after deferred imports
                        self.symbols_populated_after_deferred
                            .insert((module_name.to_string(), original_name.clone()));
                    }
                }
            }
        }

        // Finally, add entry module code (it's always last in topological order)
        // Find the entry module in our modules list
        let entry_module = modules
            .into_iter()
            .find(|(name, _, _, _)| name == params.entry_module_name);

        if let Some((module_name, mut ast, module_path, _)) = entry_module {
            log::debug!("Processing entry module: '{module_name}'");
            log::debug!("Entry module has {} statements", ast.body.len());

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
            let (importlib_was_transformed, created_namespace_objects) = {
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
                    },
                );

                // Pre-populate hoisted importlib aliases for the entry module
                if let Some(importlib_imports) = self.stdlib_import_from_map.get("importlib") {
                    for (name, alias_opt) in importlib_imports {
                        if name == "import_module"
                            && let Some(alias) = alias_opt
                        {
                            log::debug!(
                                "Pre-populating importlib.import_module alias for entry module: \
                                 {alias} -> importlib.import_module"
                            );
                            transformer
                                .import_aliases
                                .insert(alias.clone(), "importlib.import_module".to_string());
                        }
                    }
                }
                log::debug!(
                    "Transforming entry module '{module_name}' with RecursiveImportTransformer"
                );
                transformer.transform_module(&mut ast);
                log::debug!("Finished transforming entry module '{module_name}'");

                (
                    transformer.importlib_transformed,
                    transformer.created_namespace_objects,
                )
            };

            // Track if namespace objects were created
            if created_namespace_objects {
                log::debug!("Namespace objects were created, adding types import");
                import_deduplicator::add_stdlib_import(self, "types");
            }

            // If importlib was transformed, remove importlib import
            if importlib_was_transformed {
                log::debug!("importlib was transformed, removing import if present");
                self.importlib_fully_transformed = true;
                ast.body.retain(|stmt| {
                    match stmt {
                        Stmt::Import(import_stmt) => {
                            // Check if this is import importlib
                            !import_stmt
                                .names
                                .iter()
                                .any(|alias| alias.name.as_str() == "importlib")
                        }
                        Stmt::ImportFrom(import_from) => {
                            // Check if this is from importlib import ...
                            import_from
                                .module
                                .as_ref()
                                .is_none_or(|m| m.as_str() != "importlib")
                        }
                        _ => true,
                    }
                });
            }

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
                        );

                        if !duplicate {
                            // Imports have already been transformed by RecursiveImportTransformer
                            final_body.push(stmt.clone());
                        } else {
                            log::debug!(
                                "Skipping duplicate import in entry module: {:?}",
                                import_from.module
                            );
                        }
                    }
                    Stmt::Import(import_stmt) => {
                        let duplicate = import_deduplicator::is_duplicate_import(
                            self,
                            import_stmt,
                            &final_body,
                        );

                        if !duplicate {
                            // Imports have already been transformed by RecursiveImportTransformer
                            final_body.push(stmt.clone());
                        } else {
                            log::debug!("Skipping duplicate import in entry module");
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
                                        && crate::code_generator::module_registry::is_init_function(
                                            func_name.id.as_str(),
                                        )
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
                                                && crate::code_generator::module_registry::is_init_function(
                                                    existing_func.id.as_str()
                                                )
                                            {
                                                let existing_path =
                                                    expression_handlers::extract_attribute_path(existing_attr);
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
            // Add entry module's deferred imports with deduplication
            for stmt in entry_deferred_imports {
                let is_duplicate = if let Stmt::Assign(assign) = &stmt {
                    match &assign.targets[0] {
                        Expr::Name(target) => {
                            let target_name = target.id.as_str();

                            // Check against existing deferred imports
                            all_deferred_imports.iter().any(|existing| {
                                if let Stmt::Assign(existing_assign) = existing
                                    && let [Expr::Name(existing_target)] =
                                        existing_assign.targets.as_slice()
                                    && existing_target.id.as_str() == target_name
                                {
                                    // Check if the values are the same
                                    return expression_handlers::expr_equals(
                                        &existing_assign.value,
                                        &assign.value,
                                    );
                                }
                                false
                            })
                        }
                        Expr::Attribute(target_attr) => {
                            // For attribute assignments like schemas.user = ...
                            let target_path =
                                expression_handlers::extract_attribute_path(target_attr);

                            // Check if this is a module init assignment
                            if let Expr::Call(call) = &assign.value.as_ref()
                                && let Expr::Name(func_name) = &call.func.as_ref()
                                && crate::code_generator::module_registry::is_init_function(
                                    func_name.id.as_str(),
                                )
                            {
                                // Check against existing deferred imports for same module init
                                all_deferred_imports.iter().any(|existing| {
                                    if let Stmt::Assign(existing_assign) = existing
                                        && existing_assign.targets.len() == 1
                                        && let Expr::Attribute(existing_attr) =
                                            &existing_assign.targets[0]
                                        && let Expr::Call(existing_call) =
                                            &existing_assign.value.as_ref()
                                        && let Expr::Name(existing_func) =
                                            &existing_call.func.as_ref()
                                        && crate::code_generator::module_registry::is_init_function(
                                            existing_func.id.as_str(),
                                        )
                                    {
                                        let existing_path =
                                            expression_handlers::extract_attribute_path(
                                                existing_attr,
                                            );
                                        if existing_path == target_path {
                                            log::debug!(
                                                "Found duplicate module init in entry deferred \
                                                 imports: {} = {}",
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

                if !is_duplicate {
                    all_deferred_imports.push(stmt);
                } else {
                    log::debug!("Skipping duplicate deferred import from entry module");
                }
            }
        }

        // Add any remaining deferred imports from the entry module
        // (The inlined module imports were already added before the entry module code)
        if !all_deferred_imports.is_empty() {
            log::debug!(
                "Adding {} remaining deferred imports from entry module",
                all_deferred_imports.len()
            );

            // First, ensure we have init calls for all wrapper modules that need them
            let mut needed_init_calls = FxIndexSet::default();
            for stmt in &all_deferred_imports {
                if let Stmt::Assign(assign) = stmt
                    && let Expr::Attribute(attr) = &assign.value.as_ref()
                    && let Expr::Subscript(subscript) = &attr.value.as_ref()
                    && let Expr::Attribute(sys_attr) = &subscript.value.as_ref()
                    && let Expr::Name(sys_name) = &sys_attr.value.as_ref()
                    && sys_name.id.as_str() == "sys"
                    && sys_attr.attr.as_str() == "modules"
                    && let Expr::StringLiteral(lit) = &subscript.slice.as_ref()
                {
                    let module_name = lit.value.to_str();
                    if let Some(synthetic_name) = self.module_registry.get(module_name) {
                        needed_init_calls.insert(synthetic_name.clone());
                    }
                }
            }

            // Add init calls first
            for synthetic_name in needed_init_calls {
                // Note: This is in a context where we can't mutate self, so we'll rely on
                // the namespaces being pre-created by identify_required_namespaces
                // Get the original module name for this synthetic name
                let module_name = self
                    .module_registry
                    .iter()
                    .find(|(_, syn_name)| *syn_name == &synthetic_name)
                    .map(|(orig_name, _)| orig_name.to_string())
                    .unwrap_or_else(|| synthetic_name.clone());

                let init_stmts = crate::code_generator::module_registry::generate_module_init_call(
                    &synthetic_name,
                    &module_name,
                    self.init_functions.get(&synthetic_name).map(|s| s.as_str()),
                    &self.module_registry,
                    |statements, module_name, init_result_var| {
                        self.generate_merge_module_attributes(
                            statements,
                            module_name,
                            init_result_var,
                        );
                    },
                );
                final_body.extend(init_stmts);
            }

            // Then deduplicate and add the actual imports (without init calls)
            let imports_without_init_calls: Vec<Stmt> = all_deferred_imports
                .into_iter()
                .filter(|stmt| {
                    // Skip init calls - we've already added them above
                    if let Stmt::Expr(expr_stmt) = stmt
                        && let Expr::Call(call) = &expr_stmt.value.as_ref()
                        && let Expr::Name(name) = &call.func.as_ref()
                    {
                        return !crate::code_generator::module_registry::is_init_function(
                            name.id.as_str(),
                        );
                    }
                    true
                })
                .collect();

            let mut deduped_imports =
                import_deduplicator::deduplicate_deferred_imports_with_existing(
                    imports_without_init_calls,
                    &final_body,
                );

            // Filter out invalid assignments where the RHS references a module that uses an init
            // function For example, `mypkg.compat = compat` when `compat` is wrapped in
            // an init function
            self.filter_invalid_submodule_assignments(&mut deduped_imports, None);

            log::debug!(
                "Total deferred imports after deduplication: {}",
                deduped_imports.len()
            );
            final_body.extend(deduped_imports);
        }

        // Add hoisted imports at the beginning of final_body
        // This is done here after all transformations to ensure we capture all needed imports
        let mut hoisted_imports = Vec::new();
        import_deduplicator::add_hoisted_imports(self, &mut hoisted_imports);

        // Note: Namespace statements are now created earlier, before module inlining
        // to ensure they exist when module code references them

        hoisted_imports.extend(final_body);
        final_body = hoisted_imports;

        // Post-process: Fix forward reference issues in cross-module inheritance
        // Only apply reordering if we detect actual inheritance-based forward references
        if self.has_cross_module_inheritance_forward_refs(&final_body) {
            final_body = self.fix_forward_references_in_statements(final_body);
        }

        // Deduplicate namespace assignments that may have been created multiple times
        // This happens when processing imports like `import mypackage` and `import mypackage.utils`
        final_body = self.deduplicate_namespace_assignments(final_body);

        // Final filter: Remove any invalid assignments where module.attr = attr and attr is a
        // submodule that doesn't exist as a local variable
        // This catches any assignments that slipped through earlier filters

        // First collect all local variable names to avoid borrow checker issues
        let local_variables: FxIndexSet<String> = final_body
            .iter()
            .filter_map(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && let [Expr::Name(name)] = assign.targets.as_slice()
                {
                    return Some(name.id.to_string());
                }
                None
            })
            .collect();

        self.filter_invalid_submodule_assignments(&mut final_body, Some(&local_variables));

        // Also deduplicate function definitions that may have been duplicated by forward reference
        // fixes
        final_body = self.deduplicate_function_definitions(final_body);

        let mut result = ModModule {
            node_index: self.create_transformed_node("Bundled module root".to_string()),
            range: TextRange::default(),
            body: final_body,
        };

        // Assign proper node indices to all nodes in the final AST
        self.assign_node_indices_to_ast(&mut result);

        // Post-processing: Remove importlib import if it's unused
        // This happens when all importlib.import_module() calls were transformed
        import_deduplicator::remove_unused_importlib(&mut result);

        // Log transformation statistics
        let stats = self.transformation_context.get_stats();
        log::info!("Transformation statistics:");
        log::info!("  Total transformations: {}", stats.total_transformations);
        log::info!("  New nodes created: {}", stats.new_nodes);

        Ok(result)
    }

    /// Find modules that are imported directly
    fn find_directly_imported_modules(
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
            // Module defines __all__, only export symbols listed there
            let result = exports.contains(&symbol_name.to_string());
            log::debug!(
                "Module '{module_name}' has explicit __all__ exports: {exports:?}, symbol \
                 '{symbol_name}' included: {result}"
            );
            result
        } else {
            // No __all__ defined, use default Python visibility rules
            // Export all symbols that don't start with underscore
            let result = !symbol_name.starts_with('_');
            log::debug!(
                "Module '{module_name}' has no explicit __all__, symbol '{symbol_name}' should \
                 export: {result}"
            );
            result
        }
    }

    /// Extract simple assignment target name
    pub fn extract_simple_assign_target(&self, assign: &StmtAssign) -> Option<String> {
        if assign.targets.len() == 1
            && let Expr::Name(name) = &assign.targets[0]
        {
            return Some(name.id.to_string());
        }
        None
    }

    /// Check if an assignment is self-referential (e.g., x = x)
    pub fn is_self_referential_assignment(&self, assign: &StmtAssign, python_version: u8) -> bool {
        // Check if this is a simple assignment with a single target and value
        if assign.targets.len() == 1
            && let (Expr::Name(target), Expr::Name(value)) =
                (&assign.targets[0], assign.value.as_ref())
        {
            // Check if target and value have the same name
            if target.id == value.id {
                // Special case: Built-in types like `bytes = bytes`, `str = str` are NOT
                // self-referential They're re-exporting Python's built-in types
                // through the module's namespace
                let name = target.id.as_str();
                if ruff_python_stdlib::builtins::is_python_builtin(name, python_version, false) {
                    log::debug!(
                        "Assignment '{}' = '{}' is a built-in type re-export, not self-referential",
                        target.id,
                        value.id
                    );
                    return false;
                }

                log::debug!(
                    "Found self-referential assignment: {} = {}",
                    target.id,
                    value.id
                );
                return true;
            }
        }
        false
    }

    /// Check if an assignment references a module that will be created as a namespace
    fn assignment_references_namespace_module(
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

    /// Create a dotted attribute assignment
    pub(super) fn create_dotted_attribute_assignment(
        &self,
        parent_module: &str,
        attr_name: &str,
        full_module_name: &str,
    ) -> Stmt {
        // Create the value expression - handle dotted names properly
        let value_expr = if full_module_name.contains('.') {
            // For dotted names like "myrequests.compat", create a proper dotted expression
            let parts: Vec<&str> = full_module_name.split('.').collect();
            expressions::dotted_name(&parts, ExprContext::Load)
        } else {
            // Simple name
            expressions::name(full_module_name, ExprContext::Load)
        };

        statements::assign(
            vec![expressions::attribute(
                expressions::name(parent_module, ExprContext::Load),
                attr_name,
                ExprContext::Store,
            )],
            value_expr,
        )
    }

    /// Create a namespace module using types.SimpleNamespace
    pub(super) fn create_namespace_module(&self, module_name: &str) -> Vec<Stmt> {
        // Create: module_name = types.SimpleNamespace()
        // Note: This should only be called with simple (non-dotted) module names
        debug_assert!(
            !module_name.contains('.'),
            "create_namespace_module called with dotted name: {module_name}"
        );

        // This method is called by create_namespace_statements which already
        // filters based on required_namespaces, so we don't need to check again

        // Create the namespace
        let mut statements = vec![statements::simple_assign(
            module_name,
            expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
        )];

        // Set the __name__ attribute to match real module behavior
        statements.push(statements::assign(
            vec![expressions::attribute(
                expressions::name(module_name, ExprContext::Load),
                "__name__",
                ExprContext::Store,
            )],
            expressions::string_literal(module_name),
        ));

        statements
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
            func_def.name = Identifier::new(new_name, TextRange::default());
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
            class_def.name = Identifier::new(new_name, TextRange::default());
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

    /// Process module body recursively to handle conditional imports
    pub fn process_body_recursive(
        &self,
        body: Vec<Stmt>,
        module_name: &str,
        module_scope_symbols: Option<&rustc_hash::FxHashSet<String>>,
    ) -> Vec<Stmt> {
        self.process_body_recursive_impl(body, module_name, module_scope_symbols, false)
    }

    /// Implementation of process_body_recursive with conditional context tracking
    fn process_body_recursive_impl(
        &self,
        body: Vec<Stmt>,
        module_name: &str,
        module_scope_symbols: Option<&rustc_hash::FxHashSet<String>>,
        in_conditional_context: bool,
    ) -> Vec<Stmt> {
        let mut result = Vec::new();

        for stmt in body {
            match &stmt {
                Stmt::If(if_stmt) => {
                    // Process if body recursively (inside conditional context)
                    let processed_body = self.process_body_recursive_impl(
                        if_stmt.body.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Process elif/else clauses
                    let processed_elif_else = if_stmt
                        .elif_else_clauses
                        .iter()
                        .map(|clause| {
                            let processed_clause_body = self.process_body_recursive_impl(
                                clause.body.clone(),
                                module_name,
                                module_scope_symbols,
                                true,
                            );
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
                    if import_from.module.as_ref().map(|m| m.as_str()) != Some("__future__") {
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
                                if !local_name.starts_with('_') {
                                    log::debug!(
                                        "Adding module.{local_name} = {local_name} after \
                                         conditional import (bypassing __all__ restrictions)"
                                    );
                                    result.push(
                                        crate::code_generator::module_registry::create_module_attr_assignment("module", local_name),
                                    );
                                } else {
                                    log::debug!(
                                        "NOT exporting conditional ImportFrom symbol \
                                         '{local_name}' in module '{module_name}' (private symbol)"
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
                                        result.push(
                                            crate::code_generator::module_registry::create_module_attr_assignment("module", local_name),
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
                                .map(|n| n.as_str())
                                .unwrap_or(imported_name);

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
                                result.push(
                                    crate::code_generator::module_registry::create_module_attr_assignment(
                                        "module",
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
                        && let Some(name) = self.extract_simple_assign_target(assign)
                    {
                        // For conditional assignments, always add module attributes for non-private
                        // symbols regardless of __all__ restrictions, since
                        // they can be defined at runtime
                        if !name.starts_with('_') {
                            log::debug!(
                                "Adding module.{name} = {name} after conditional assignment \
                                 (bypassing __all__ restrictions)"
                            );
                            result.push(
                                crate::code_generator::module_registry::create_module_attr_assignment(
                                    "module",
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

    /// Transform nested functions to use module attributes for module-level variables
    pub fn transform_nested_function_for_module_vars(
        &self,
        func_def: &mut StmtFunctionDef,
        module_level_vars: &rustc_hash::FxHashSet<String>,
    ) {
        // Collect local variables defined in this function
        let mut local_vars = rustc_hash::FxHashSet::default();

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
        collect_local_vars(&func_def.body, &mut local_vars);

        // Transform the function body, excluding local variables
        for stmt in &mut func_def.body {
            self.transform_stmt_for_module_vars_with_locals(stmt, module_level_vars, &local_vars);
        }
    }

    /// Transform a statement with awareness of local variables
    fn transform_stmt_for_module_vars_with_locals(
        &self,
        stmt: &mut Stmt,
        module_level_vars: &rustc_hash::FxHashSet<String>,
        local_vars: &rustc_hash::FxHashSet<String>,
    ) {
        match stmt {
            Stmt::FunctionDef(nested_func) => {
                // Recursively transform nested functions
                self.transform_nested_function_for_module_vars(nested_func, module_level_vars);
            }
            Stmt::Assign(assign) => {
                // Transform assignment targets and values
                for target in &mut assign.targets {
                    Self::transform_expr_for_module_vars_with_locals(
                        target,
                        module_level_vars,
                        local_vars,
                    );
                }
                Self::transform_expr_for_module_vars_with_locals(
                    &mut assign.value,
                    module_level_vars,
                    local_vars,
                );
            }
            Stmt::Expr(expr_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut expr_stmt.value,
                    module_level_vars,
                    local_vars,
                );
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    Self::transform_expr_for_module_vars_with_locals(
                        value,
                        module_level_vars,
                        local_vars,
                    );
                }
            }
            Stmt::If(if_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut if_stmt.test,
                    module_level_vars,
                    local_vars,
                );
                for stmt in &mut if_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                    );
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(condition) = &mut clause.test {
                        Self::transform_expr_for_module_vars_with_locals(
                            condition,
                            module_level_vars,
                            local_vars,
                        );
                    }
                    for stmt in &mut clause.body {
                        self.transform_stmt_for_module_vars_with_locals(
                            stmt,
                            module_level_vars,
                            local_vars,
                        );
                    }
                }
            }
            Stmt::For(for_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut for_stmt.target,
                    module_level_vars,
                    local_vars,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut for_stmt.iter,
                    module_level_vars,
                    local_vars,
                );
                for stmt in &mut for_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                    );
                }
            }
            Stmt::While(while_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut while_stmt.test,
                    module_level_vars,
                    local_vars,
                );
                for stmt in &mut while_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                    );
                }
                for stmt in &mut while_stmt.orelse {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                    );
                }
            }
            Stmt::Try(try_stmt) => {
                for stmt in &mut try_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                    );
                }
                for handler in &mut try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    for stmt in &mut eh.body {
                        self.transform_stmt_for_module_vars_with_locals(
                            stmt,
                            module_level_vars,
                            local_vars,
                        );
                    }
                }
                for stmt in &mut try_stmt.orelse {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                    );
                }
                for stmt in &mut try_stmt.finalbody {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
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
        module_level_vars: &rustc_hash::FxHashSet<String>,
        local_vars: &rustc_hash::FxHashSet<String>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                let name_str = name_expr.id.as_str();

                // Special case: transform __name__ to module.__name__
                if name_str == "__name__" && matches!(name_expr.ctx, ExprContext::Load) {
                    // Transform __name__ -> module.__name__
                    *expr = expressions::attribute(
                        expressions::name("module", ExprContext::Load),
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
                        expressions::name("module", ExprContext::Load),
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
                );
                for arg in &mut call.arguments.args {
                    Self::transform_expr_for_module_vars_with_locals(
                        arg,
                        module_level_vars,
                        local_vars,
                    );
                }
                for keyword in &mut call.arguments.keywords {
                    Self::transform_expr_for_module_vars_with_locals(
                        &mut keyword.value,
                        module_level_vars,
                        local_vars,
                    );
                }
            }
            Expr::BinOp(binop) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut binop.left,
                    module_level_vars,
                    local_vars,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut binop.right,
                    module_level_vars,
                    local_vars,
                );
            }
            Expr::Dict(dict) => {
                for item in &mut dict.items {
                    if let Some(key) = &mut item.key {
                        Self::transform_expr_for_module_vars_with_locals(
                            key,
                            module_level_vars,
                            local_vars,
                        );
                    }
                    Self::transform_expr_for_module_vars_with_locals(
                        &mut item.value,
                        module_level_vars,
                        local_vars,
                    );
                }
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    Self::transform_expr_for_module_vars_with_locals(
                        elem,
                        module_level_vars,
                        local_vars,
                    );
                }
            }
            Expr::Attribute(attr) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut attr.value,
                    module_level_vars,
                    local_vars,
                );
            }
            Expr::Subscript(subscript) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut subscript.value,
                    module_level_vars,
                    local_vars,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut subscript.slice,
                    module_level_vars,
                    local_vars,
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

                    // Create initialization statements for lifted globals
                    let init_stmts =
                        self.create_global_init_statements(&function_globals, lifted_names);

                    // Transform the function body
                    let params = TransformFunctionParams {
                        lifted_names,
                        global_info,
                        function_globals: &function_globals,
                    };
                    self.transform_function_body_for_lifted_globals(func_def, &params, init_stmts);
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
        if !self.is_symbol_kept_by_tree_shaking(module_name, symbol_name) {
            log::trace!(
                "Tree shaking: removing unused symbol '{symbol_name}' from module '{module_name}'"
            );
            return false;
        }

        let exports = module_exports_map.get(module_name).and_then(|e| e.as_ref());

        if let Some(export_list) = exports {
            // Module has exports (either explicit __all__ or extracted symbols)
            // Only inline if the symbol is in the export list
            export_list.contains(&symbol_name.to_string())
        } else {
            // No exports at all, don't inline anything
            false
        }
    }

    /// Get a unique name for a symbol, using the module suffix pattern
    fn get_unique_name_with_module_suffix(&self, base_name: &str, module_name: &str) -> String {
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

            Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: hard_dep.source_module.clone().into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(&hard_dep.imported_attr, TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })
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
    fn rewrite_hard_dependencies_in_module(&self, ast: &mut ModModule, module_name: &str) {
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
                                            *arg = Expr::Attribute(ExprAttribute {
                                                node_index: AtomicNodeIndex::dummy(),
                                                value: Box::new(Expr::Name(ExprName {
                                                    node_index: AtomicNodeIndex::dummy(),
                                                    id: name_to_use.clone().into(),
                                                    ctx: ExprContext::Load,
                                                    range: TextRange::default(),
                                                })),
                                                attr: Identifier::new(
                                                    parts[1],
                                                    TextRange::default(),
                                                ),
                                                ctx: ExprContext::Load,
                                                range: TextRange::default(),
                                            });
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
    fn reorder_statements_for_circular_module(
        &self,
        module_name: &str,
        statements: Vec<Stmt>,
        python_version: u8,
    ) -> Vec<Stmt> {
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
                    if let Some(name) = self.extract_simple_assign_target(assign) {
                        // Skip self-referential assignments - they'll be handled later
                        if self.is_self_referential_assignment(assign, python_version) {
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

    /// Reorder statements from multiple modules to ensure proper declaration order
    /// This handles cross-module dependencies like classes inheriting from symbols defined in other
    /// modules
    fn reorder_cross_module_statements(
        &self,
        statements: Vec<Stmt>,
        python_version: u8,
    ) -> Vec<Stmt> {
        let mut imports: Vec<Stmt> = Vec::new();
        let mut classes: Vec<Stmt> = Vec::new();
        let mut functions: Vec<Stmt> = Vec::new();
        let mut other_stmts: Vec<Stmt> = Vec::new();

        // First pass: identify all symbols used as base classes
        let mut base_class_symbols = FxIndexSet::default();
        for stmt in &statements {
            if let Stmt::ClassDef(class_def) = stmt
                && let Some(arguments) = &class_def.arguments
            {
                for base_expr in &arguments.args {
                    if let Expr::Name(name_expr) = base_expr {
                        base_class_symbols.insert(name_expr.id.to_string());
                    }
                }
            }
        }

        // Separate assignments that define base classes from other assignments
        let mut base_class_assignments: Vec<Stmt> = Vec::new();
        let mut regular_assignments: Vec<Stmt> = Vec::new();
        let mut builtin_restorations: Vec<Stmt> = Vec::new();
        let mut namespace_builtin_assignments: Vec<Stmt> = Vec::new();

        // Categorize statements
        for stmt in statements {
            match &stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    imports.push(stmt);
                }
                Stmt::Assign(assign) => {
                    // Check if this is an attribute assignment
                    let is_attribute_assignment = if assign.targets.len() == 1 {
                        matches!(&assign.targets[0], Expr::Attribute(_))
                    } else {
                        false
                    };

                    if is_attribute_assignment {
                        debug!("Found attribute assignment: {stmt:?}");

                        // Check if this is a namespace attribute assignment of a built-in type
                        // e.g., compat.bytes = bytes
                        let is_namespace_builtin_assignment =
                            if let (Expr::Attribute(_attr), Expr::Name(value_name)) =
                                (&assign.targets[0], assign.value.as_ref())
                            {
                                // Check if the value is a built-in type
                                ruff_python_stdlib::builtins::is_python_builtin(
                                    value_name.id.as_str(),
                                    python_version,
                                    false,
                                )
                            } else {
                                false
                            };

                        if is_namespace_builtin_assignment {
                            log::debug!("Found namespace builtin assignment: {stmt:?}");
                            namespace_builtin_assignments.push(stmt);
                            continue;
                        }

                        // Check if this is a module namespace assignment (e.g., parent.child =
                        // child_namespace) These need to be ordered with
                        // regular assignments, not deferred
                        let is_module_namespace_assignment =
                            if let Expr::Attribute(attr) = &assign.targets[0] {
                                // Check if the right-hand side references a module or namespace
                                if let Expr::Name(name) = &attr.value.as_ref() {
                                    // Check if this looks like a parent-child module relationship
                                    let parent_name = name.id.as_str();
                                    let child_name = attr.attr.as_str();

                                    // Check if the value being assigned matches the child name
                                    if let Expr::Name(value_name) = assign.value.as_ref() {
                                        value_name.id.as_str() == child_name
                                            || value_name.id.as_str()
                                                == format!("{parent_name}_{child_name}")
                                            || value_name
                                                .id
                                                .as_str()
                                                .starts_with(&format!("{child_name}_"))
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                        if is_module_namespace_assignment {
                            // Module namespace assignments should be ordered with regular
                            // assignments
                            regular_assignments.push(stmt);
                        } else {
                            // Other attribute assignments (like class attributes) come after class
                            // definitions
                            debug!("Adding attribute assignment to other_stmts: {stmt:?}");
                            other_stmts.push(stmt);
                        }
                    } else {
                        // Check if this is a built-in type restoration (e.g., bytes = bytes)
                        let is_builtin_restoration =
                            if let ([Expr::Name(target)], Expr::Name(value)) =
                                (assign.targets.as_slice(), assign.value.as_ref())
                            {
                                // Check if it's a self-assignment of a built-in type
                                target.id == value.id
                                    && ruff_python_stdlib::builtins::is_python_builtin(
                                        target.id.as_str(),
                                        python_version,
                                        false,
                                    )
                            } else {
                                false
                            };

                        if is_builtin_restoration {
                            builtin_restorations.push(stmt);
                        } else {
                            // Check if this assignment defines a base class symbol
                            let defines_base_class = if assign.targets.len() == 1 {
                                if let Expr::Name(target) = &assign.targets[0] {
                                    // Only consider it a base class assignment if:
                                    // 1. The target is used as a base class
                                    // 2. The value is an attribute access (e.g.,
                                    //    json.JSONDecodeError)
                                    if base_class_symbols.contains(target.id.as_str()) {
                                        match assign.value.as_ref() {
                                            Expr::Attribute(_) => true, /* e.g., json. */
                                            // JSONDecodeError
                                            _ => false,
                                        }
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            if defines_base_class {
                                base_class_assignments.push(stmt);
                            } else {
                                regular_assignments.push(stmt);
                            }
                        }
                    }
                }
                Stmt::ClassDef(_) => {
                    classes.push(stmt);
                }
                Stmt::FunctionDef(_) => {
                    functions.push(stmt);
                }
                _ => {
                    other_stmts.push(stmt);
                }
            }
        }

        // Build the reordered list:
        // 1. Imports first
        // 2. Built-in type restorations (must come very early to restore types)
        // 3. Namespace built-in assignments (e.g., compat.bytes = bytes)
        // 4. Base class assignments (must come before class definitions)
        // 5. Regular assignments
        // 6. Classes (must come before functions that might use them)
        // 7. Functions (may depend on classes)
        // 8. Other statements (including class attribute assignments)
        let mut reordered = Vec::new();
        reordered.extend(imports);
        reordered.extend(builtin_restorations);
        reordered.extend(namespace_builtin_assignments);
        reordered.extend(base_class_assignments);
        reordered.extend(regular_assignments);
        reordered.extend(classes);
        reordered.extend(functions);
        reordered.extend(other_stmts);

        reordered
    }

    /// Reorder statements to ensure proper declaration order
    fn reorder_statements_for_proper_declaration_order(&self, statements: Vec<Stmt>) -> Vec<Stmt> {
        log::debug!("Reordering {} statements", statements.len());
        let mut imports = Vec::new();
        let mut self_assignments = Vec::new();
        let mut functions_and_classes = Vec::new();
        let mut other_stmts = Vec::new();

        // First pass: identify all symbols used as base classes
        let mut base_class_symbols = FxIndexSet::default();
        for stmt in &statements {
            if let Stmt::ClassDef(class_def) = stmt {
                // Collect all base class names
                if let Some(arguments) = &class_def.arguments {
                    for base_expr in &arguments.args {
                        if let Expr::Name(name_expr) = base_expr {
                            base_class_symbols.insert(name_expr.id.to_string());
                        }
                    }
                }
            }
        }

        // Separate assignments that define base classes from other assignments
        let mut base_class_assignments = Vec::new();
        let mut regular_assignments = Vec::new();

        // Categorize statements
        for stmt in statements {
            match &stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    imports.push(stmt);
                }
                Stmt::Assign(assign) => {
                    // Check if this is a class attribute assignment (e.g., MyClass.__module__ =
                    // 'foo')
                    let is_class_attribute = if assign.targets.len() == 1 {
                        if let Expr::Attribute(attr) = &assign.targets[0] {
                            if let Expr::Name(_) = attr.value.as_ref() {
                                // This is an attribute assignment like MyClass.__module__
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if is_class_attribute {
                        // Class attribute assignments should stay after their class definitions
                        other_stmts.push(stmt);
                    } else {
                        // Check if this assignment defines a base class symbol
                        let defines_base_class = if assign.targets.len() == 1 {
                            if let Expr::Name(target) = &assign.targets[0] {
                                // Only consider it a base class assignment if:
                                // 1. The target is used as a base class
                                // 2. The value looks like it could be a class (attribute access)
                                if base_class_symbols.contains(target.id.as_str()) {
                                    // Check if the value is an attribute access (e.g.,
                                    // json.JSONDecodeError)
                                    // or a simple name that could be a class
                                    match assign.value.as_ref() {
                                        Expr::Attribute(_) => true, // e.g., json.JSONDecodeError
                                        Expr::Name(name) => {
                                            // Check if it looks like a class name (starts with
                                            // uppercase)
                                            name.id.chars().next().is_some_and(|c| c.is_uppercase())
                                        }
                                        _ => false,
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        // Check if this is a self-assignment (e.g., validate = validate)
                        let is_self_assignment = if assign.targets.len() == 1 {
                            if let (Expr::Name(target), Expr::Name(value)) =
                                (&assign.targets[0], assign.value.as_ref())
                            {
                                target.id == value.id
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if is_self_assignment {
                            // Self-assignments should come after function definitions
                            self_assignments.push(stmt);
                        } else if defines_base_class {
                            // Assignments that define base classes must come before class
                            // definitions
                            base_class_assignments.push(stmt);
                        } else {
                            // Regular assignments
                            regular_assignments.push(stmt);
                        }
                    }
                }
                Stmt::AnnAssign(ann_assign) => {
                    // Check if this annotated assignment defines a base class symbol
                    let defines_base_class = if let Expr::Name(target) = ann_assign.target.as_ref()
                    {
                        base_class_symbols.contains(target.id.as_str())
                    } else {
                        false
                    };

                    if defines_base_class {
                        base_class_assignments.push(stmt);
                    } else {
                        regular_assignments.push(stmt);
                    }
                }
                Stmt::FunctionDef(_) => {
                    // Functions need to come after classes they might reference
                    // We'll sort these later
                    functions_and_classes.push(stmt);
                }
                Stmt::ClassDef(_) => {
                    // Classes can have forward references in type annotations
                    // so they can go first among functions/classes
                    functions_and_classes.push(stmt);
                }
                _ => {
                    other_stmts.push(stmt);
                }
            }
        }

        // Separate functions and classes, then order them: classes first, functions second
        // This ensures functions that depend on classes are defined after those classes
        let mut classes = Vec::new();
        let mut functions = Vec::new();

        for stmt in functions_and_classes {
            match &stmt {
                Stmt::ClassDef(_) => classes.push(stmt),
                Stmt::FunctionDef(_) => functions.push(stmt),
                _ => unreachable!("Only functions and classes should be in this list"),
            }
        }

        // Combine: classes first, then functions
        let mut ordered_functions_and_classes = Vec::new();
        ordered_functions_and_classes.extend(classes);
        ordered_functions_and_classes.extend(functions);

        log::debug!(
            "Reordered: {} imports, {} base class assignments, {} regular assignments, {} \
             classes, {} functions, {} self assignments, {} other statements",
            imports.len(),
            base_class_assignments.len(),
            regular_assignments.len(),
            ordered_functions_and_classes
                .iter()
                .filter(|s| matches!(s, Stmt::ClassDef(_)))
                .count(),
            ordered_functions_and_classes
                .iter()
                .filter(|s| matches!(s, Stmt::FunctionDef(_)))
                .count(),
            self_assignments.len(),
            other_stmts.len()
        );

        // Build the reordered list:
        // 1. Imports first
        // 2. Base class assignments (must come before class definitions)
        // 3. Other module-level assignments (variables) - but not self-assignments
        // 4. Functions and classes (ordered by inheritance)
        // 5. Self-assignments (after functions are defined)
        // 6. Other statements
        let mut reordered = Vec::new();
        reordered.extend(imports);
        reordered.extend(base_class_assignments);
        reordered.extend(regular_assignments);
        reordered.extend(ordered_functions_and_classes);
        reordered.extend(self_assignments);
        reordered.extend(other_stmts);

        reordered
    }

    /// Resolve import aliases in a statement
    fn resolve_import_aliases_in_stmt(
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

    /// Inline a class definition
    fn inline_class(
        &mut self,
        class_def: &StmtClassDef,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let class_name = class_def.name.to_string();
        if !self.should_inline_symbol(&class_name, module_name, ctx.module_exports_map) {
            return;
        }

        // Check if this symbol was renamed by semantic analysis
        let renamed_name = if let Some(module_rename_map) = ctx.module_renames.get(module_name) {
            if let Some(new_name) = module_rename_map.get(&class_name) {
                // Only use semantic rename if it's actually different
                if new_name != &class_name {
                    log::debug!(
                        "Using semantic rename for class '{class_name}' to '{new_name}' in module \
                         '{module_name}'"
                    );
                    new_name.clone()
                } else {
                    // Semantic rename is same as original, check if there's a conflict
                    if ctx.global_symbols.contains(&class_name) {
                        // There's a conflict, apply module suffix pattern
                        let base_name =
                            self.get_unique_name_with_module_suffix(&class_name, module_name);
                        generate_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        // No conflict, use original name
                        class_name.clone()
                    }
                }
            } else {
                // No semantic rename, check if there's a conflict
                if ctx.global_symbols.contains(&class_name) {
                    // There's a conflict, apply module suffix pattern
                    let base_name =
                        self.get_unique_name_with_module_suffix(&class_name, module_name);
                    generate_unique_name(&base_name, ctx.global_symbols)
                } else {
                    // No conflict, use original name
                    class_name.clone()
                }
            }
        } else {
            // No semantic rename, check if there's a conflict
            if ctx.global_symbols.contains(&class_name) {
                // There's a conflict, apply module suffix pattern
                let base_name = self.get_unique_name_with_module_suffix(&class_name, module_name);
                generate_unique_name(&base_name, ctx.global_symbols)
            } else {
                // No conflict, use original name
                class_name.clone()
            }
        };

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(class_name.clone(), renamed_name.clone());
        ctx.global_symbols.insert(renamed_name.clone());

        // Clone and rename the class
        let mut class_def_clone = class_def.clone();
        class_def_clone.name = Identifier::new(renamed_name.clone(), TextRange::default());

        // Apply renames to base classes
        // CRITICAL: For cross-module inheritance, we need to apply renames from the
        // source module of each base class, not just from the current module.
        if let Some(ref mut arguments) = class_def_clone.arguments {
            for arg in &mut arguments.args {
                // Try to determine the source module for base class names
                if let Expr::Name(name_expr) = arg {
                    let base_class_name = name_expr.id.as_str();

                    // Check if this base class was imported from another module
                    if let Some(source_module) = ctx.import_sources.get(base_class_name) {
                        // This base class was imported from another module
                        // Use that module's renames instead of the current module's
                        if let Some(source_renames) = ctx.module_renames.get(source_module)
                            && let Some(renamed) = source_renames.get(base_class_name)
                        {
                            log::debug!(
                                "Applying cross-module rename for base class '{base_class_name}' \
                                 from module '{source_module}': '{base_class_name}' -> '{renamed}'"
                            );
                            name_expr.id = renamed.clone().into();
                            continue;
                        }
                    }

                    // Not imported or no rename found in source module, apply local renames
                    if let Some(renamed) = module_renames.get(base_class_name) {
                        name_expr.id = renamed.clone().into();
                    }
                } else {
                    // Complex base class expression, use standard rewriting
                    expression_handlers::rewrite_aliases_in_expr(arg, module_renames);
                }
            }
        }

        // Apply renames and resolve import aliases in class body
        for body_stmt in &mut class_def_clone.body {
            Self::resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
            expression_handlers::rewrite_aliases_in_stmt(body_stmt, module_renames);
            // Also apply semantic renames from context
            if let Some(semantic_renames) = ctx.module_renames.get(module_name) {
                expression_handlers::rewrite_aliases_in_stmt(body_stmt, semantic_renames);
            }
        }

        ctx.inlined_stmts.push(Stmt::ClassDef(class_def_clone));

        // Set the __module__ attribute to preserve the original module name
        ctx.inlined_stmts.push(statements::set_string_attribute(
            &renamed_name,
            "__module__",
            module_name,
        ));

        // If the class was renamed, also set __name__ to preserve the original class name
        if renamed_name != class_name {
            ctx.inlined_stmts.push(statements::set_string_attribute(
                &renamed_name,
                "__name__",
                &class_name,
            ));

            // Set __qualname__ to match __name__ for proper repr()
            ctx.inlined_stmts.push(statements::set_string_attribute(
                &renamed_name,
                "__qualname__",
                &class_name,
            ));
        }
    }

    /// Inline an assignment statement
    fn inline_assignment(
        &mut self,
        assign: &StmtAssign,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let Some(name) = self.extract_simple_assign_target(assign) else {
            return;
        };

        // Special handling for circular modules: include private module-level variables
        // that may be used by public functions
        let is_circular_module = self.circular_modules.contains(module_name);
        let is_single_underscore_private = name.starts_with('_') && !name.starts_with("__");

        // For circular modules, we need special handling of private variables
        if is_circular_module && is_single_underscore_private {
            // For circular modules, we always include single-underscore private module-level
            // variables because they might be used by functions that are part of the
            // circular dependency
            log::debug!("Including private variable '{name}' from circular module '{module_name}'");
        } else if !self.should_inline_symbol(&name, module_name, ctx.module_exports_map) {
            // For all other cases, use the standard inlining check
            return;
        }

        // Clone the assignment first
        let mut assign_clone = assign.clone();

        // Check if this is a self-referential assignment
        let is_self_referential = self.is_self_referential_assignment(assign, ctx.python_version);

        // Skip self-referential assignments entirely - they're meaningless
        if is_self_referential {
            log::debug!("Skipping self-referential assignment '{name}' in module '{module_name}'");
            // Still need to track the rename for the symbol so namespace creation works
            // But we should check if there's already a rename for this symbol
            // (e.g., from a function or class definition)
            if !module_renames.contains_key(&name) {
                // Only create a rename if we haven't seen this symbol yet
                let renamed_name = if let Some(module_rename_map) =
                    ctx.module_renames.get(module_name)
                {
                    if let Some(new_name) = module_rename_map.get(&name) {
                        new_name.clone()
                    } else if ctx.global_symbols.contains(&name) {
                        let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                        generate_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        name.clone()
                    }
                } else if ctx.global_symbols.contains(&name) {
                    let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                    generate_unique_name(&base_name, ctx.global_symbols)
                } else {
                    name.clone()
                };
                module_renames.insert(name.clone(), renamed_name.clone());
                ctx.global_symbols.insert(renamed_name);
            }
            return;
        }

        // Apply existing renames to the RHS value BEFORE creating new rename for LHS
        expression_handlers::resolve_import_aliases_in_expr(
            &mut assign_clone.value,
            &ctx.import_aliases,
        );
        expression_handlers::rewrite_aliases_in_expr(&mut assign_clone.value, module_renames);

        // Now create a new rename for the LHS
        // Check if this symbol was renamed by semantic analysis
        let renamed_name = if let Some(module_rename_map) = ctx.module_renames.get(module_name) {
            if let Some(new_name) = module_rename_map.get(&name) {
                // Only use semantic rename if it's actually different
                if new_name != &name {
                    log::debug!(
                        "Using semantic rename for variable '{name}' to '{new_name}' in module \
                         '{module_name}'"
                    );
                    new_name.clone()
                } else {
                    // Semantic rename is same as original, check if there's a conflict
                    if ctx.global_symbols.contains(&name) {
                        // There's a conflict, apply module suffix pattern
                        let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                        generate_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        // No conflict, use original name
                        name.clone()
                    }
                }
            } else {
                // No semantic rename, check if there's a conflict
                if ctx.global_symbols.contains(&name) {
                    // There's a conflict, apply module suffix pattern
                    let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                    generate_unique_name(&base_name, ctx.global_symbols)
                } else {
                    // No conflict, use original name
                    name.clone()
                }
            }
        } else {
            // No semantic rename, check if there's a conflict
            if ctx.global_symbols.contains(&name) {
                // There's a conflict, apply module suffix pattern
                let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                generate_unique_name(&base_name, ctx.global_symbols)
            } else {
                // No conflict, use original name
                name.clone()
            }
        };

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(name.clone(), renamed_name.clone());
        ctx.global_symbols.insert(renamed_name.clone());

        // Apply the rename to the LHS
        if let Expr::Name(name_expr) = &mut assign_clone.targets[0] {
            name_expr.id = renamed_name.clone().into();
        }

        // Check if this assignment references a module that will be created as a namespace
        // If it does, we need to defer it until after namespace creation
        if self.assignment_references_namespace_module(&assign_clone, module_name, ctx) {
            log::debug!(
                "Deferring assignment '{name}' in module '{module_name}' as it references a \
                 namespace module"
            );
            ctx.deferred_imports.push(Stmt::Assign(assign_clone));
        } else {
            ctx.inlined_stmts.push(Stmt::Assign(assign_clone));
        }
    }

    /// Inline an annotated assignment statement
    fn inline_ann_assignment(
        &mut self,
        ann_assign: &ruff_python_ast::StmtAnnAssign,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let Expr::Name(name) = ann_assign.target.as_ref() else {
            return;
        };

        let var_name = name.id.to_string();
        if !self.should_inline_symbol(&var_name, module_name, ctx.module_exports_map) {
            return;
        }

        // Check if this symbol was renamed by semantic analysis
        let renamed_name = if let Some(module_rename_map) = ctx.module_renames.get(module_name) {
            if let Some(new_name) = module_rename_map.get(&var_name) {
                // Only use semantic rename if it's actually different
                if new_name != &var_name {
                    log::debug!(
                        "Using semantic rename for annotated variable '{var_name}' to \
                         '{new_name}' in module '{module_name}'"
                    );
                    new_name.clone()
                } else {
                    // Semantic rename is same as original, check if there's a conflict
                    if ctx.global_symbols.contains(&var_name) {
                        // There's a conflict, apply module suffix pattern
                        let base_name =
                            self.get_unique_name_with_module_suffix(&var_name, module_name);
                        generate_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        // No conflict, use original name
                        var_name.clone()
                    }
                }
            } else {
                // No semantic rename, check if there's a conflict
                if ctx.global_symbols.contains(&var_name) {
                    // There's a conflict, apply module suffix pattern
                    let base_name = self.get_unique_name_with_module_suffix(&var_name, module_name);
                    generate_unique_name(&base_name, ctx.global_symbols)
                } else {
                    // No conflict, use original name
                    var_name.clone()
                }
            }
        } else {
            // No semantic rename, check if there's a conflict
            if ctx.global_symbols.contains(&var_name) {
                // There's a conflict, apply module suffix pattern
                let base_name = self.get_unique_name_with_module_suffix(&var_name, module_name);
                generate_unique_name(&base_name, ctx.global_symbols)
            } else {
                // No conflict, use original name
                var_name.clone()
            }
        };

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(var_name.clone(), renamed_name.clone());
        if renamed_name != var_name {
            log::debug!(
                "Renaming annotated variable '{var_name}' to '{renamed_name}' in module \
                 '{module_name}'"
            );
        }
        ctx.global_symbols.insert(renamed_name.clone());

        // Clone and rename the annotated assignment
        let mut ann_assign_clone = ann_assign.clone();
        if let Expr::Name(name_expr) = ann_assign_clone.target.as_mut() {
            name_expr.id = renamed_name.into();
        }
        ctx.inlined_stmts.push(Stmt::AnnAssign(ann_assign_clone));
    }
}

/// Collect local variables from statements
fn collect_local_vars(stmts: &[Stmt], local_vars: &mut rustc_hash::FxHashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Assign(assign) => {
                // Collect assignment targets as local variables
                for target in &assign.targets {
                    if let Expr::Name(name) = target {
                        local_vars.insert(name.id.to_string());
                    }
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Collect annotated assignment targets
                if let Expr::Name(name) = ann_assign.target.as_ref() {
                    local_vars.insert(name.id.to_string());
                }
            }
            Stmt::For(for_stmt) => {
                // Collect for loop targets
                if let Expr::Name(name) = for_stmt.target.as_ref() {
                    local_vars.insert(name.id.to_string());
                }
                // Recursively collect from body
                collect_local_vars(&for_stmt.body, local_vars);
                collect_local_vars(&for_stmt.orelse, local_vars);
            }
            Stmt::If(if_stmt) => {
                // Recursively collect from branches
                collect_local_vars(&if_stmt.body, local_vars);
                for clause in &if_stmt.elif_else_clauses {
                    collect_local_vars(&clause.body, local_vars);
                }
            }
            Stmt::While(while_stmt) => {
                collect_local_vars(&while_stmt.body, local_vars);
                collect_local_vars(&while_stmt.orelse, local_vars);
            }
            Stmt::With(with_stmt) => {
                // Collect with statement targets
                for item in &with_stmt.items {
                    if let Some(ref optional_vars) = item.optional_vars
                        && let Expr::Name(name) = optional_vars.as_ref()
                    {
                        local_vars.insert(name.id.to_string());
                    }
                }
                collect_local_vars(&with_stmt.body, local_vars);
            }
            Stmt::Try(try_stmt) => {
                collect_local_vars(&try_stmt.body, local_vars);
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    // Collect exception name if present
                    if let Some(ref name) = eh.name {
                        local_vars.insert(name.to_string());
                    }
                    collect_local_vars(&eh.body, local_vars);
                }
                collect_local_vars(&try_stmt.orelse, local_vars);
                collect_local_vars(&try_stmt.finalbody, local_vars);
            }
            Stmt::FunctionDef(func_def) => {
                // Function definitions create local names
                local_vars.insert(func_def.name.to_string());
            }
            Stmt::ClassDef(class_def) => {
                // Class definitions create local names
                local_vars.insert(class_def.name.to_string());
            }
            _ => {}
        }
    }
}

// Helper methods for import rewriting
impl<'a> Bundler<'a> {
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
        let mut stmts = Vec::new();

        // Check if this is a wrapper module that needs initialization
        if let Some(synthetic_name) = self.module_registry.get(module_name) {
            // Generate the init call
            let init_func_name =
                crate::code_generator::module_registry::get_init_function_name(synthetic_name);

            // Call the init function and get the result
            let init_call = expressions::call(
                expressions::name(&init_func_name, ExprContext::Load),
                vec![],
                vec![],
            );

            // Generate the appropriate assignment based on module type
            stmts.extend(self.generate_module_assignment_from_init(module_name, init_call));

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
                // Check if we haven't already created this namespace globally or locally
                let already_created = self.created_namespaces.contains(&parent_path)
                    || self.is_namespace_already_created(&parent_path, result_stmts);

                if !already_created {
                    // Parent is not a wrapper module and not an inlined module, create a simple
                    // namespace
                    result_stmts.extend(self.create_namespace_module(&parent_path));
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

    /// Create dotted attribute assignments for imports
    pub(super) fn create_dotted_assignments(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
        // For import a.b.c.d, we need:
        // a.b = <module a.b>
        // a.b.c = <module a.b.c>
        // a.b.c.d = <module a.b.c.d>
        for i in 2..=parts.len() {
            let parent = parts[..i - 1].join(".");
            let attr = parts[i - 1];
            let full_path = parts[..i].join(".");

            // Check if this would be a redundant self-assignment
            let full_target = format!("{parent}.{attr}");
            if full_target == full_path {
                debug!(
                    "Skipping redundant self-assignment in create_dotted_assignments: \
                     {parent}.{attr} = {full_path}"
                );
            } else {
                result_stmts
                    .push(self.create_dotted_attribute_assignment(&parent, attr, &full_path));
            }
        }
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

    /// Populate a namespace with symbols from an inlined module using a specific target name
    pub(super) fn populate_namespace_with_module_symbols_with_renames(
        &self,
        target_name: &str,
        module_name: &str,
        result_stmts: &mut Vec<Stmt>,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) {
        // Get the module's exports
        if let Some(exports) = self
            .module_exports
            .get(module_name)
            .and_then(|e| e.as_ref())
        {
            // Build the namespace access expression for the target
            let parts: Vec<&str> = target_name.split('.').collect();

            // First, add __all__ attribute to the namespace
            // Create the target expression for __all__
            let all_target = expressions::dotted_name(&parts, ExprContext::Load);

            // Filter exports to only include symbols that survived tree-shaking
            let filtered_exports = SymbolAnalyzer::filter_exports_by_tree_shaking(
                exports,
                module_name,
                self.tree_shaking_keep_symbols.as_ref(),
                true,
            );

            // Check if __all__ assignment already exists for this namespace
            let all_assignment_exists = result_stmts.iter().any(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && let [Expr::Attribute(attr)] = assign.targets.as_slice()
                    && let Expr::Name(base) = attr.value.as_ref()
                {
                    return base.id.as_str() == target_name && attr.attr.as_str() == "__all__";
                }
                false
            });

            if all_assignment_exists {
                log::debug!("Skipping duplicate __all__ assignment for namespace '{target_name}'");
            } else {
                // Create __all__ = [...] assignment with filtered exports
                let all_list = expressions::list(
                    filtered_exports
                        .iter()
                        .map(|name| expressions::string_literal(name.as_str()))
                        .collect(),
                    ExprContext::Load,
                );

                result_stmts.push(statements::assign(
                    vec![expressions::attribute(
                        all_target,
                        "__all__",
                        ExprContext::Store,
                    )],
                    all_list,
                ));

                log::info!(
                    "Created __all__ assignment for namespace '{target_name}' with exports: \
                     {filtered_exports:?}"
                );
            }

            // Skip individual symbol assignments if this namespace was already created with initial
            // symbols
            if self.namespaces_with_initial_symbols.contains(module_name) {
                log::debug!(
                    "Skipping individual symbol assignments for '{module_name}' - namespace \
                     created with initial symbols"
                );
                return;
            }

            // For each exported symbol that survived tree-shaking, add it to the namespace
            'symbol_loop: for symbol in &filtered_exports {
                // For re-exported symbols, check if the original symbol is kept by tree-shaking
                let should_include = if self.tree_shaking_keep_symbols.is_some() {
                    // First check if this symbol is directly defined in this module
                    if self.is_symbol_kept_by_tree_shaking(module_name, symbol) {
                        true
                    } else {
                        // If not, check if this is a re-exported symbol from another module
                        // For modules with __all__, we always include symbols that are re-exported
                        // even if they're not directly defined in the module
                        let module_has_all_export = self
                            .module_exports
                            .get(module_name)
                            .and_then(|exports| exports.as_ref())
                            .map(|exports| exports.contains(symbol))
                            .unwrap_or(false);

                        if module_has_all_export {
                            log::debug!(
                                "Including re-exported symbol {symbol} from module {module_name} \
                                 (in __all__)"
                            );
                            true
                        } else {
                            false
                        }
                    }
                } else {
                    // No tree-shaking, include everything
                    true
                };

                if !should_include {
                    log::debug!(
                        "Skipping namespace assignment for {module_name}.{symbol} - removed by \
                         tree-shaking"
                    );
                    continue;
                }

                // Check if this symbol is actually a submodule
                let full_submodule_path = format!("{module_name}.{symbol}");
                let is_bundled_submodule = self.bundled_modules.contains(&full_submodule_path);
                let is_inlined = self.inlined_modules.contains(&full_submodule_path);
                let uses_init_function = self.module_registry.contains_key(&full_submodule_path);

                if is_bundled_submodule {
                    log::debug!(
                        "Symbol '{symbol}' in module '{module_name}' is a submodule (bundled: \
                         {is_bundled_submodule}, inlined: {is_inlined}, uses_init: \
                         {uses_init_function})"
                    );

                    // For inlined submodules, check if the parent module re-exports a symbol
                    // with the same name as the submodule (e.g., __version__ from __version__
                    // module)
                    if is_inlined {
                        // Check if the submodule has a symbol with the same name as itself
                        if let Some(submodule_exports) = self
                            .module_exports
                            .get(&full_submodule_path)
                            .and_then(|e| e.as_ref())
                            && submodule_exports.contains(symbol)
                        {
                            // The submodule exports a symbol with the same name as itself
                            // Check if the parent module re-exports this symbol
                            log::debug!(
                                "Submodule '{full_submodule_path}' exports symbol '{symbol}' with \
                                 same name"
                            );

                            // Get the renamed symbol from the submodule
                            if let Some(submodule_renames) =
                                symbol_renames.get(&full_submodule_path)
                                && let Some(renamed) = submodule_renames.get(*symbol)
                            {
                                log::info!(
                                    "Creating namespace assignment: {target_name}.{symbol} = \
                                     {renamed} (re-exported from submodule)"
                                );

                                // Create the assignment
                                let target = expressions::dotted_name(&parts, ExprContext::Load);
                                result_stmts.push(statements::assign(
                                    vec![expressions::attribute(
                                        target,
                                        symbol,
                                        ExprContext::Store,
                                    )],
                                    expressions::name(renamed, ExprContext::Load),
                                ));
                                continue 'symbol_loop;
                            }
                        }
                    }

                    // Skip other submodules - they are handled separately
                    // This prevents creating invalid assignments like `mypkg.compat = compat`
                    // when `compat` is a submodule, not a local variable
                    continue;
                }

                // Get the renamed symbol if it exists
                let actual_symbol_name =
                    if let Some(module_renames) = symbol_renames.get(module_name) {
                        module_renames
                            .get(*symbol)
                            .cloned()
                            .unwrap_or_else(|| (*symbol).clone())
                    } else {
                        (*symbol).clone()
                    };

                // Create the target expression
                // For simple modules, this will be the module name directly
                // For dotted modules (e.g., greetings.greeting), build the chain
                let target = expressions::dotted_name(&parts, ExprContext::Load);

                // Check if this specific symbol was already populated after deferred imports
                // This happens for modules that had forward references and were populated later
                if self
                    .symbols_populated_after_deferred
                    .contains(&(module_name.to_string(), (*symbol).clone()))
                    && target_name == sanitize_module_name_for_identifier(module_name).as_str()
                {
                    log::debug!(
                        "Skipping symbol assignment {target_name}.{symbol} = {actual_symbol_name} \
                         - this specific symbol was already populated after deferred imports"
                    );
                    continue;
                }

                // Check if this assignment already exists in result_stmts
                let assignment_exists = result_stmts.iter().any(|stmt| {
                    if let Stmt::Assign(assign) = stmt
                        && assign.targets.len() == 1
                        && let Expr::Attribute(attr) = &assign.targets[0]
                    {
                        // Check if this is the same assignment target
                        if let Expr::Name(base) = attr.value.as_ref() {
                            return base.id.as_str() == target_name
                                && attr.attr.as_str() == *symbol;
                        }
                    }
                    false
                });

                if assignment_exists {
                    log::debug!(
                        "Skipping duplicate namespace assignment: {target_name}.{symbol} = \
                         {actual_symbol_name}"
                    );
                    continue;
                }

                // Also check if this is a parent module assignment that might already exist
                // For example, if we're processing mypkg.exceptions and the symbol CustomJSONError
                // is in mypkg's __all__, check if mypkg.CustomJSONError = CustomJSONError already
                // exists
                if module_name.contains('.') {
                    let parent_module = module_name
                        .rsplit_once('.')
                        .map(|(parent, _)| parent)
                        .unwrap_or("");
                    if !parent_module.is_empty()
                        && let Some(Some(parent_exports)) = self.module_exports.get(parent_module)
                        && parent_exports.contains(symbol)
                    {
                        // This symbol is re-exported by the parent module
                        // Check if the parent assignment already exists
                        let parent_assignment_exists = result_stmts.iter().any(|stmt| {
                            if let Stmt::Assign(assign) = stmt
                                && assign.targets.len() == 1
                                && let Expr::Attribute(attr) = &assign.targets[0]
                                && let Expr::Name(base) = attr.value.as_ref()
                            {
                                return base.id.as_str() == parent_module
                                    && attr.attr.as_str() == *symbol;
                            }
                            false
                        });

                        if parent_assignment_exists {
                            log::debug!(
                                "Skipping namespace assignment for '{symbol}' - parent module \
                                 '{parent_module}' already has assignment for re-exported symbol"
                            );
                            continue;
                        }
                    }
                }

                // Also check if this assignment was already made by deferred imports
                // This handles the case where imports create namespace assignments that
                // would be duplicated by __all__ processing
                if !self.global_deferred_imports.is_empty() {
                    // Check if this symbol was deferred by the same module (intra-module imports)
                    let key = (module_name.to_string(), symbol.to_string());
                    if self.global_deferred_imports.contains_key(&key) {
                        log::debug!(
                            "Skipping namespace assignment for '{symbol}' - already created by \
                             deferred import from module '{module_name}'"
                        );
                        continue;
                    }
                }

                // For wrapper modules, check if the symbol is imported from an inlined submodule
                // These symbols are already added via module attribute assignments
                if self.module_registry.contains_key(module_name) {
                    log::debug!(
                        "Module '{module_name}' is a wrapper module, checking if symbol \
                         '{symbol}' is imported from inlined submodule"
                    );
                    // This is a wrapper module - check if symbol is re-exported from inlined
                    // submodule
                    if let Some(module_asts) = self.module_asts.as_ref() {
                        // Find the module's AST to check its imports
                        if let Some((_, ast, module_path, _)) = module_asts
                            .iter()
                            .find(|(name, _, _, _)| name == module_name)
                        {
                            // Check if this symbol is imported from an inlined submodule
                            for stmt in &ast.body {
                                if let Stmt::ImportFrom(import_from) = stmt {
                                    let resolved_module = if import_from.level > 0 {
                                        self.resolver.resolve_relative_to_absolute_module_name(
                                            import_from.level,
                                            import_from.module.as_ref().map(|m| m.as_str()),
                                            module_path,
                                        )
                                    } else {
                                        import_from.module.as_ref().map(|m| m.as_str().to_string())
                                    };
                                    if let Some(ref resolved) = resolved_module {
                                        // Check if the resolved module is inlined
                                        if self.inlined_modules.contains(resolved) {
                                            // Check if our symbol is in this import
                                            for alias in &import_from.names {
                                                if alias.name.as_str() == *symbol {
                                                    log::debug!(
                                                        "Skipping namespace assignment for \
                                                         '{symbol}' - already imported from \
                                                         inlined module '{resolved}' and added as \
                                                         module attribute"
                                                    );
                                                    // Skip this symbol - it's already added via
                                                    // module attributes
                                                    continue 'symbol_loop;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Check if we're about to create a parent module assignment when processing a child
                // module For example, when processing mypkg.exceptions, we
                // shouldn't create mypkg.CustomJSONError if it's already been
                // created elsewhere
                let should_skip_parent_assignment =
                    if module_name != target_name && module_name.contains('.') {
                        // We're processing a child module but creating an assignment on a different
                        // namespace This happens when the target_name is
                        // the parent module
                        let parent_module = module_name
                            .rsplit_once('.')
                            .map(|(parent, _)| parent)
                            .unwrap_or("");
                        if parent_module == target_name {
                            // We're creating an assignment on the parent module from a child module
                            // Check if this assignment already exists in result_stmts
                            let parent_assignment_exists = result_stmts.iter().any(|stmt| {
                                if let Stmt::Assign(assign) = stmt
                                    && assign.targets.len() == 1
                                    && let Expr::Attribute(attr) = &assign.targets[0]
                                    && let Expr::Name(base) = attr.value.as_ref()
                                {
                                    return base.id.as_str() == target_name
                                        && attr.attr.as_str() == *symbol;
                                }
                                false
                            });

                            if parent_assignment_exists {
                                log::debug!(
                                    "Skipping parent module assignment {target_name}.{symbol} = \
                                     {actual_symbol_name} - already exists in result_stmts"
                                );
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                if !should_skip_parent_assignment {
                    // Check if this assignment already exists in result_stmts to avoid duplicates
                    let assignment_exists = result_stmts.iter().any(|stmt| {
                        if let Stmt::Assign(assign) = stmt
                            && assign.targets.len() == 1
                            && let Expr::Attribute(attr) = &assign.targets[0]
                            && let Expr::Name(base) = attr.value.as_ref()
                            && let Expr::Name(value) = assign.value.as_ref()
                        {
                            return base.id.as_str() == target_name
                                && attr.attr.as_str() == *symbol
                                && value.id.as_str() == actual_symbol_name;
                        }
                        false
                    });

                    if assignment_exists {
                        log::debug!(
                            "Skipping duplicate namespace assignment: {target_name}.{symbol} = \
                             {actual_symbol_name} (already exists in result_stmts) - in \
                             populate_namespace_with_module_symbols_with_renames"
                        );
                        continue;
                    }

                    // Check if this is a submodule that uses an init function
                    let full_submodule_path = format!("{module_name}.{symbol}");
                    let uses_init_function = self
                        .module_registry
                        .get(&full_submodule_path)
                        .and_then(|synthetic_name| self.init_functions.get(synthetic_name))
                        .is_some();

                    if uses_init_function {
                        // This is a submodule that uses an init function
                        // The assignment will be handled by the init function call
                        log::debug!(
                            "Skipping namespace assignment for '{target_name}.{symbol}' - it uses \
                             an init function"
                        );
                        continue;
                    }

                    // Check if this is an inlined submodule (no local variable exists)
                    let is_inlined_submodule = self.inlined_modules.contains(&full_submodule_path);
                    if is_inlined_submodule {
                        log::debug!(
                            "Skipping namespace assignment for '{target_name}.{symbol}' - it's an \
                             inlined submodule"
                        );
                        continue;
                    }

                    // Check if this is a submodule at all (vs a symbol defined in the module)
                    let is_bundled_submodule = self.bundled_modules.contains(&full_submodule_path);
                    if is_bundled_submodule {
                        // This is a submodule that's bundled but neither inlined nor uses init
                        // function This can happen when the submodule is
                        // handled differently (e.g., by deferred imports)
                        log::debug!(
                            "Skipping namespace assignment for '{target_name}.{symbol}' - it's a \
                             bundled submodule"
                        );
                        continue;
                    }

                    // Log when creating namespace assignments
                    log::info!(
                        "Creating namespace assignment: {target_name}.{symbol} = \
                         {actual_symbol_name} (in \
                         populate_namespace_with_module_symbols_with_renames)"
                    );

                    // Now add the symbol as an attribute (e.g., greetings.greeting.get_greeting =
                    // get_greeting)
                    let attr_assignment = statements::assign(
                        vec![expressions::attribute(target, symbol, ExprContext::Store)],
                        expressions::name(&actual_symbol_name, ExprContext::Load),
                    );

                    result_stmts.push(attr_assignment);
                }
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

    /// Create initialization statements for lifted globals
    fn create_global_init_statements(
        &self,
        _function_globals: &FxIndexSet<String>,
        _lifted_names: &FxIndexMap<String, String>,
    ) -> Vec<Stmt> {
        // No initialization statements needed - global declarations mean
        // we use the lifted names directly, not through local variables
        Vec::new()
    }

    /// Transform function body for lifted globals
    fn transform_function_body_for_lifted_globals(
        &self,
        func_def: &mut StmtFunctionDef,
        params: &TransformFunctionParams,
        init_stmts: Vec<Stmt>,
    ) {
        let mut new_body = Vec::new();
        let mut added_init = false;

        for body_stmt in &mut func_def.body {
            match body_stmt {
                Stmt::Global(global_stmt) => {
                    // Rewrite global statement to use lifted names
                    for name in &mut global_stmt.names {
                        if let Some(lifted_name) = params.lifted_names.get(name.as_str()) {
                            *name = Identifier::new(lifted_name, TextRange::default());
                        }
                    }
                    new_body.push(body_stmt.clone());

                    // Add initialization statements after global declarations
                    if !added_init && !init_stmts.is_empty() {
                        new_body.extend(init_stmts.clone());
                        added_init = true;
                    }
                }
                _ => {
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
                                expressions::name("module", ExprContext::Load),
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
                                expressions::name("module", ExprContext::Load),
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

    /// Check if there are cross-module inheritance forward references
    fn has_cross_module_inheritance_forward_refs(&self, statements: &[Stmt]) -> bool {
        // Look for classes that inherit from base classes that are defined later
        // This can happen when symbol renaming creates forward references

        // First, collect all class positions and assignment positions
        let mut class_positions = FxIndexMap::default();
        let mut assignment_positions = FxIndexMap::default();
        let mut namespace_init_positions = FxIndexMap::default();

        for (idx, stmt) in statements.iter().enumerate() {
            match stmt {
                Stmt::ClassDef(class_def) => {
                    class_positions.insert(class_def.name.to_string(), idx);
                }
                Stmt::Assign(assign) => {
                    // Check if this is a simple assignment like HTTPBasicAuth = HTTPBasicAuth_2
                    if assign.targets.len() == 1
                        && let Expr::Name(target) = &assign.targets[0]
                    {
                        assignment_positions.insert(target.id.to_string(), idx);
                    }
                    // Also check for namespace init assignments like:
                    // mypkg.compat = __cribo_init_...()
                    if assign.targets.len() == 1
                        && let Expr::Attribute(attr) = &assign.targets[0]
                        && let Expr::Call(call) = assign.value.as_ref()
                        && let Expr::Name(func_name) = call.func.as_ref()
                        && func_name.id.starts_with("__cribo_init_")
                    {
                        // Extract the namespace path (e.g., "mypkg.compat")
                        let namespace_path = expr_to_dotted_name(&Expr::Attribute(attr.clone()));
                        namespace_init_positions.insert(namespace_path, idx);
                    }
                }
                _ => {}
            }
        }

        // Now check for forward references
        for (idx, stmt) in statements.iter().enumerate() {
            if let Stmt::ClassDef(class_def) = stmt
                && let Some(arguments) = &class_def.arguments
            {
                let class_name = class_def.name.as_str();
                let class_pos = idx;

                for base in &arguments.args {
                    // Check simple name references
                    if let Expr::Name(name_expr) = base {
                        let base_name = name_expr.id.as_str();

                        // Check if the base class is defined via assignment later
                        if let Some(&assign_pos) = assignment_positions.get(base_name)
                            && assign_pos > class_pos
                        {
                            return true;
                        }

                        // Also check if base class is a renamed class (ends with _<number>)
                        // and is defined later
                        if base_name.chars().any(|c| c == '_')
                            && let Some(last_part) = base_name.split('_').next_back()
                            && last_part.chars().all(|c| c.is_ascii_digit())
                            && let Some(&base_pos) = class_positions.get(base_name)
                            && base_pos > class_pos
                        {
                            return true;
                        }
                    }
                    // Check attribute references (e.g., mypkg.compat.JSONDecodeError)
                    else if let Expr::Attribute(attr_expr) = base {
                        // Extract the base module path (e.g., "mypkg.compat" from
                        // "mypkg.compat.JSONDecodeError")
                        let base_path = expr_to_dotted_name(&attr_expr.value);
                        // Check if this namespace is initialized later
                        if let Some(&init_pos) = namespace_init_positions.get(&base_path)
                            && init_pos > class_pos
                        {
                            log::debug!(
                                "Class '{}' inherits from {}.{} but namespace '{}' is initialized \
                                 later at position {} (class at {})",
                                class_name,
                                base_path,
                                attr_expr.attr,
                                base_path,
                                init_pos,
                                class_pos
                            );
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Deduplicate namespace assignments (e.g., mypackage.__all__ = [...])
    /// This is needed when the same namespace is populated multiple times
    /// (e.g., when both `import mypackage` and `import mypackage.utils` are processed)
    fn deduplicate_namespace_assignments(&self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        let mut seen_assignments: indexmap::IndexSet<(String, String)> = indexmap::IndexSet::new();
        let mut result = Vec::new();

        for stmt in stmts {
            let should_keep = match &stmt {
                Stmt::Assign(assign) => {
                    if let [Expr::Attribute(attr)] = assign.targets.as_slice() {
                        if let Expr::Name(base) = attr.value.as_ref() {
                            // Deduplicate namespace init assignments
                            if let Expr::Call(call) = assign.value.as_ref() {
                                if let Expr::Name(func_name) = call.func.as_ref() {
                                    if func_name.id.starts_with("__cribo_init_") {
                                        // Only keep if we haven't seen this assignment before
                                        seen_assignments
                                            .insert((base.id.to_string(), attr.attr.to_string()))
                                    } else {
                                        true
                                    }
                                } else {
                                    true
                                }
                            } else if attr.attr.as_str().starts_with("__")
                                && attr.attr.as_str().ends_with("__")
                            {
                                // Only keep if we haven't seen this assignment before
                                seen_assignments
                                    .insert((base.id.to_string(), attr.attr.to_string()))
                            } else {
                                // Don't deduplicate regular attributes
                                true
                            }
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                }
                _ => true,
            };

            if should_keep {
                result.push(stmt);
            } else {
                log::debug!("Deduplicating duplicate namespace assignment");
            }
        }

        result
    }

    /// Deduplicate function definitions that may have been created multiple times
    fn deduplicate_function_definitions(&self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        let mut seen_functions: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        let mut result = Vec::new();

        for stmt in stmts {
            let should_keep = match &stmt {
                Stmt::FunctionDef(func_def) => {
                    // Only keep if we haven't seen this function before
                    seen_functions.insert(func_def.name.to_string())
                }
                _ => true,
            };

            if should_keep {
                result.push(stmt);
            } else {
                log::debug!("Deduplicating duplicate function definition");
            }
        }

        result
    }

    /// Fix forward reference issues by reordering statements
    fn fix_forward_references_in_statements(&self, statements: Vec<Stmt>) -> Vec<Stmt> {
        // Quick check: if there are no classes, no need to reorder
        let has_classes = statements.iter().any(|s| matches!(s, Stmt::ClassDef(_)));
        if !has_classes {
            return statements;
        }

        // Use the same detection logic as has_cross_module_inheritance_forward_refs
        // to ensure consistency
        if !self.has_cross_module_inheritance_forward_refs(&statements) {
            return statements;
        }

        log::debug!("Fixing forward references in statements");

        // First, identify namespace initialization statements and their dependencies
        let mut namespace_inits = FxIndexMap::default();
        let mut namespace_functions = FxIndexMap::default();

        for (idx, stmt) in statements.iter().enumerate() {
            // Track namespace init function definitions
            if let Stmt::FunctionDef(func_def) = stmt
                && func_def.name.starts_with("__cribo_init_")
            {
                namespace_functions.insert(func_def.name.to_string(), idx);
            }
            // Track namespace init assignments
            if let Stmt::Assign(assign) = stmt
                && assign.targets.len() == 1
                && let Expr::Call(call) = assign.value.as_ref()
                && let Expr::Name(func_name) = call.func.as_ref()
                && func_name.id.starts_with("__cribo_init_")
            {
                if let Expr::Attribute(attr) = &assign.targets[0] {
                    let namespace_path = expr_to_dotted_name(&Expr::Attribute(attr.clone()));
                    namespace_inits.insert(namespace_path, (idx, func_name.id.to_string()));
                } else if let Expr::Name(name) = &assign.targets[0] {
                    namespace_inits.insert(name.id.to_string(), (idx, func_name.id.to_string()));
                }
            }
        }

        // Find classes that need namespace inits to be moved earlier
        let mut required_namespace_moves = FxIndexSet::default();

        for (idx, stmt) in statements.iter().enumerate() {
            if let Stmt::ClassDef(class_def) = stmt
                && let Some(arguments) = &class_def.arguments
            {
                for base in &arguments.args {
                    if let Expr::Attribute(attr_expr) = base {
                        let base_path = expr_to_dotted_name(&attr_expr.value);
                        if let Some(&(init_pos, ref _func_name)) = namespace_inits.get(&base_path)
                            && init_pos > idx
                        {
                            log::debug!(
                                "Class '{}' at position {} needs namespace '{}' (init at {}) to \
                                 be moved earlier",
                                class_def.name,
                                idx,
                                base_path,
                                init_pos
                            );
                            required_namespace_moves.insert(base_path.clone());
                        }
                    }
                }
            }
        }

        // If no namespace moves are required, use the original ordering logic
        if required_namespace_moves.is_empty() {
            return self.fix_forward_references_classes_only(statements);
        }

        // Reorder statements to move required namespace inits before class definitions
        let mut result = Vec::new();
        let mut moved_indices = FxIndexSet::default();
        let mut moved_func_indices = FxIndexSet::default();

        // Clone statements for indexing
        let statements_copy = statements.clone();

        // First, collect the indices of statements that need to be moved
        for namespace in &required_namespace_moves {
            if let Some(&(init_idx, ref func_name)) = namespace_inits.get(namespace) {
                moved_indices.insert(init_idx);
                // Also move the function definition if it exists
                if let Some(&func_idx) = namespace_functions.get(func_name) {
                    moved_func_indices.insert(func_idx);
                }
            }
        }

        // Process statements in order, moving namespace inits when needed
        for (idx, stmt) in statements.into_iter().enumerate() {
            // Skip statements that will be moved
            if moved_indices.contains(&idx) || moved_func_indices.contains(&idx) {
                continue;
            }

            // Before adding a class, check if it needs any namespace inits
            if let Stmt::ClassDef(ref class_def) = stmt
                && let Some(arguments) = &class_def.arguments
            {
                // Add required namespace init functions and calls before this class
                for base in &arguments.args {
                    if let Expr::Attribute(attr_expr) = base {
                        let base_path = expr_to_dotted_name(&attr_expr.value);
                        if required_namespace_moves.contains(&base_path)
                            && let Some((_, func_name)) = namespace_inits.get(&base_path)
                        {
                            // Add the function definition first if it hasn't been added
                            if let Some(&func_idx) = namespace_functions.get(func_name)
                                && moved_func_indices.contains(&func_idx)
                            {
                                // Clone the function from the original statements
                                if let Some(orig_stmt) = statements_copy.get(func_idx) {
                                    result.push(orig_stmt.clone());
                                    moved_func_indices.swap_remove(&func_idx);
                                }
                            }
                            // Add the init call
                            if let Some(&(init_idx, _)) = namespace_inits.get(&base_path)
                                && moved_indices.contains(&init_idx)
                            {
                                // Clone the init statement from the original statements
                                if let Some(orig_stmt) = statements_copy.get(init_idx) {
                                    result.push(orig_stmt.clone());
                                    moved_indices.swap_remove(&init_idx);
                                    // Note: Can't mutate required_namespace_moves here
                                    // since it's borrowed
                                }
                            }
                        }
                    }
                }
            }

            result.push(stmt);
        }

        result
    }

    /// Original class-only forward reference fixing logic
    fn fix_forward_references_classes_only(&self, statements: Vec<Stmt>) -> Vec<Stmt> {
        // First pass: find where the first class appears
        let first_class_position = statements
            .iter()
            .position(|s| matches!(s, Stmt::ClassDef(_)));

        let mut class_blocks = Vec::new();
        let mut other_statements = Vec::new();
        let mut pre_class_statements = Vec::new();
        let mut current_class: Option<ClassBlock> = None;
        let mut seen_first_class = false;
        let mut class_names = FxIndexSet::default();

        for (idx, stmt) in statements.into_iter().enumerate() {
            if let Some(first_pos) = first_class_position
                && idx < first_pos
                && !seen_first_class
            {
                pre_class_statements.push(stmt);
                continue;
            }

            match stmt {
                Stmt::ClassDef(class_def) => {
                    seen_first_class = true;
                    // If we had a previous class, save it
                    if let Some(block) = current_class.take() {
                        class_blocks.push(block);
                    }
                    // Start a new class block
                    let class_name = class_def.name.to_string();
                    class_names.insert(class_name.clone());
                    current_class = Some(ClassBlock {
                        class_stmt: Stmt::ClassDef(class_def),
                        attributes: Vec::new(),
                        class_name,
                    });
                }
                Stmt::Assign(assign) if current_class.is_some() => {
                    // Check if this is a class attribute assignment (e.g., __module__)
                    let is_class_attr = if assign.targets.len() == 1 {
                        if let Expr::Attribute(attr) = &assign.targets[0] {
                            if let Expr::Name(name) = attr.value.as_ref() {
                                if let Some(ref block) = current_class {
                                    name.id.as_str() == block.class_name
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if is_class_attr {
                        // This is an attribute of the current class
                        if let Some(ref mut block) = current_class {
                            block.attributes.push(Stmt::Assign(assign));
                        }
                    } else {
                        // Not a class attribute, save current class and add to other statements
                        if let Some(block) = current_class.take() {
                            class_blocks.push(block);
                        }
                        other_statements.push(Stmt::Assign(assign));
                    }
                }
                _ => {
                    // Any other statement ends the current class block
                    if let Some(block) = current_class.take() {
                        class_blocks.push(block);
                    }
                    other_statements.push(stmt);
                }
            }
        }

        // Don't forget the last class if there is one
        if let Some(block) = current_class {
            class_blocks.push(block);
        }

        // Now order the class blocks by inheritance
        let ordered_blocks = self.order_class_blocks_by_inheritance(class_blocks);

        // Rebuild the statement list
        let mut result = Vec::new();

        // Add all pre-class statements
        result.extend(pre_class_statements);

        // Collect assignments that create aliases for classes
        let mut class_assignments = FxIndexMap::default();
        let mut other_statements_filtered = Vec::new();

        for stmt in other_statements {
            if let Stmt::Assign(assign) = &stmt {
                // Check if this is an assignment that aliases a class
                if assign.targets.len() == 1
                    && let (Expr::Name(_), Expr::Name(value)) =
                        (&assign.targets[0], assign.value.as_ref())
                {
                    let value_name = value.id.to_string();

                    // Check if the value is a known class
                    if class_names.contains(&value_name) {
                        class_assignments.insert(value_name, stmt);
                        continue;
                    }
                }
            }
            other_statements_filtered.push(stmt);
        }

        // Add all the ordered class blocks with their assignments
        for block in ordered_blocks {
            result.push(block.class_stmt.clone());
            result.extend(block.attributes);

            // Add the assignment for this class if it exists
            if let Some(assignment) = class_assignments.shift_remove(&block.class_name) {
                result.push(assignment);
            }
        }

        // Add any remaining statements
        result.extend(other_statements_filtered);

        result
    }

    /// Order class blocks based on their inheritance dependencies
    fn order_class_blocks_by_inheritance(&self, class_blocks: Vec<ClassBlock>) -> Vec<ClassBlock> {
        use petgraph::{algo::toposort, graph::DiGraph};

        // Build a graph of class dependencies
        let mut graph = DiGraph::new();
        let mut block_indices = FxIndexMap::default();
        let mut blocks_by_name = FxIndexMap::default();

        // First pass: Create nodes for each class block
        for (idx, block) in class_blocks.iter().enumerate() {
            let node_idx = graph.add_node(idx);
            block_indices.insert(block.class_name.clone(), node_idx);
            blocks_by_name.insert(block.class_name.clone(), block);
        }

        // Second pass: Add edges based on inheritance
        for block in &class_blocks {
            if let Stmt::ClassDef(class_def) = &block.class_stmt {
                let class_node = block_indices[&block.class_name];

                // Check each base class
                if let Some(arguments) = &class_def.arguments {
                    for base in &arguments.args {
                        if let Expr::Name(name_expr) = base {
                            let base_name = name_expr.id.to_string();

                            // Only add edge if the base class is defined in this module
                            if let Some(&base_node) = block_indices.get(&base_name) {
                                // Add edge from base to derived (base must come before derived)
                                graph.add_edge(base_node, class_node, ());
                                log::debug!(
                                    "Added inheritance edge: {} -> {}",
                                    base_name,
                                    block.class_name
                                );
                            }
                        }
                    }
                }
            }
        }

        // Perform topological sort
        match toposort(&graph, None) {
            Ok(sorted_nodes) => {
                // Convert back to class blocks in sorted order
                let mut ordered = Vec::new();
                for node in sorted_nodes {
                    let idx = graph[node];
                    ordered.push(class_blocks[idx].clone());
                }
                ordered
            }
            Err(_) => {
                // Circular inheritance detected, return as-is
                log::warn!("Circular inheritance detected, returning classes in original order");
                class_blocks
            }
        }
    }

    /// Check if a submodule needs a namespace object.
    ///
    /// A submodule needs a namespace if:
    /// 1. Its parent module is inlined
    /// 2. The submodule has exports (meaning it's not just internal)
    fn submodule_needs_namespace(&self, module_name: &str) -> bool {
        if let Some(parent_module) = module_name.rsplit_once('.').map(|(parent, _)| parent) {
            if self.inlined_modules.contains(parent_module)
                && self
                    .module_exports
                    .get(module_name)
                    .is_some_and(|exports| exports.is_some())
            {
                log::debug!(
                    "Submodule '{module_name}' needs namespace because parent '{parent_module}' \
                     is inlined and submodule has exports"
                );
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}

/// Convert an expression to a dotted name string
fn expr_to_dotted_name(expr: &Expr) -> String {
    match expr {
        Expr::Name(name) => name.id.as_str().to_string(),
        Expr::Attribute(attr) => {
            let base = expr_to_dotted_name(&attr.value);
            format!("{}.{}", base, attr.attr.as_str())
        }
        _ => String::new(),
    }
}
