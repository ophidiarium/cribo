use std::path::{Path, PathBuf};

use anyhow::Result;
use indexmap::{IndexMap as FxIndexMap, IndexSet as FxIndexSet};
use ruff_python_ast::{
    Alias, AtomicNodeIndex, ExceptHandler, Expr, ExprContext, ExprName, Identifier, ModModule,
    Stmt, StmtImport, StmtImportFrom,
};
use ruff_text_size::TextRange;

use crate::{
    code_generator::{
        circular_deps::SymbolDependencyGraph,
        context::{BundleParams, HardDependency},
        // import_transformer::RecursiveImportTransformerParams, // TODO: Use when implementing
    },
    cribo_graph::CriboGraph as DependencyGraph,
    transformation_context::TransformationContext,
};

/// This approach avoids forward reference issues while maintaining Python module semantics
pub struct HybridStaticBundler<'a> {
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
    pub(crate) tree_shaking_keep_symbols: Option<indexmap::IndexSet<(String, String)>>,
    /// Whether to use the module cache model for circular dependencies
    pub(crate) use_module_cache_model: bool,
}

// Implementation block for importlib detection methods
impl<'a> HybridStaticBundler<'a> {
    /// Check if a statement uses importlib
    fn stmt_uses_importlib(stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Expr(expr_stmt) => Self::expr_uses_importlib(&expr_stmt.value),
            Stmt::Assign(assign) => Self::expr_uses_importlib(&assign.value),
            Stmt::AugAssign(aug_assign) => Self::expr_uses_importlib(&aug_assign.value),
            Stmt::AnnAssign(ann_assign) => ann_assign
                .value
                .as_ref()
                .is_some_and(|v| Self::expr_uses_importlib(v)),
            Stmt::FunctionDef(func_def) => func_def.body.iter().any(Self::stmt_uses_importlib),
            Stmt::ClassDef(class_def) => class_def.body.iter().any(Self::stmt_uses_importlib),
            Stmt::If(if_stmt) => {
                Self::expr_uses_importlib(&if_stmt.test)
                    || if_stmt.body.iter().any(Self::stmt_uses_importlib)
                    || if_stmt.elif_else_clauses.iter().any(|clause| {
                        clause.test.as_ref().is_some_and(Self::expr_uses_importlib)
                            || clause.body.iter().any(Self::stmt_uses_importlib)
                    })
            }
            Stmt::While(while_stmt) => {
                Self::expr_uses_importlib(&while_stmt.test)
                    || while_stmt.body.iter().any(Self::stmt_uses_importlib)
                    || while_stmt.orelse.iter().any(Self::stmt_uses_importlib)
            }
            Stmt::For(for_stmt) => {
                Self::expr_uses_importlib(&for_stmt.iter)
                    || for_stmt.body.iter().any(Self::stmt_uses_importlib)
                    || for_stmt.orelse.iter().any(Self::stmt_uses_importlib)
            }
            Stmt::With(with_stmt) => {
                with_stmt.items.iter().any(|item| {
                    Self::expr_uses_importlib(&item.context_expr)
                        || item
                            .optional_vars
                            .as_ref()
                            .is_some_and(|v| Self::expr_uses_importlib(v))
                }) || with_stmt.body.iter().any(Self::stmt_uses_importlib)
            }
            Stmt::Try(try_stmt) => {
                try_stmt.body.iter().any(Self::stmt_uses_importlib)
                    || try_stmt.handlers.iter().any(|handler| match handler {
                        ExceptHandler::ExceptHandler(eh) => {
                            eh.body.iter().any(Self::stmt_uses_importlib)
                        }
                    })
                    || try_stmt.orelse.iter().any(Self::stmt_uses_importlib)
                    || try_stmt.finalbody.iter().any(Self::stmt_uses_importlib)
            }
            Stmt::Assert(assert_stmt) => {
                Self::expr_uses_importlib(&assert_stmt.test)
                    || assert_stmt
                        .msg
                        .as_ref()
                        .is_some_and(|v| Self::expr_uses_importlib(v))
            }
            Stmt::Return(ret) => ret
                .value
                .as_ref()
                .is_some_and(|v| Self::expr_uses_importlib(v)),
            Stmt::Raise(raise_stmt) => {
                raise_stmt
                    .exc
                    .as_ref()
                    .is_some_and(|v| Self::expr_uses_importlib(v))
                    || raise_stmt
                        .cause
                        .as_ref()
                        .is_some_and(|v| Self::expr_uses_importlib(v))
            }
            Stmt::Delete(del) => del.targets.iter().any(Self::expr_uses_importlib),
            // Statements that don't contain expressions
            Stmt::Import(_) | Stmt::ImportFrom(_) => false, /* Already handled by import */
            // transformation
            Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => false,
            Stmt::Global(_) | Stmt::Nonlocal(_) => false,
            // Match and TypeAlias need special handling
            Stmt::Match(match_stmt) => {
                Self::expr_uses_importlib(&match_stmt.subject)
                    || match_stmt
                        .cases
                        .iter()
                        .any(|case| case.body.iter().any(Self::stmt_uses_importlib))
            }
            Stmt::TypeAlias(type_alias) => Self::expr_uses_importlib(&type_alias.value),
            Stmt::IpyEscapeCommand(_) => false, // IPython specific, unlikely to use importlib
        }
    }

    /// Check if an expression uses importlib
    fn expr_uses_importlib(expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "importlib",
            Expr::Attribute(attr) => Self::expr_uses_importlib(&attr.value),
            Expr::Call(call) => {
                Self::expr_uses_importlib(&call.func)
                    || call.arguments.args.iter().any(Self::expr_uses_importlib)
                    || call
                        .arguments
                        .keywords
                        .iter()
                        .any(|kw| Self::expr_uses_importlib(&kw.value))
            }
            Expr::Subscript(sub) => {
                Self::expr_uses_importlib(&sub.value) || Self::expr_uses_importlib(&sub.slice)
            }
            Expr::Tuple(tuple) => tuple.elts.iter().any(Self::expr_uses_importlib),
            Expr::List(list) => list.elts.iter().any(Self::expr_uses_importlib),
            Expr::Set(set) => set.elts.iter().any(Self::expr_uses_importlib),
            Expr::Dict(dict) => dict.items.iter().any(|item| {
                item.key.as_ref().is_some_and(Self::expr_uses_importlib)
                    || Self::expr_uses_importlib(&item.value)
            }),
            Expr::ListComp(comp) => {
                Self::expr_uses_importlib(&comp.elt)
                    || comp.generators.iter().any(|generator| {
                        Self::expr_uses_importlib(&generator.iter)
                            || generator.ifs.iter().any(Self::expr_uses_importlib)
                    })
            }
            Expr::SetComp(comp) => {
                Self::expr_uses_importlib(&comp.elt)
                    || comp.generators.iter().any(|generator| {
                        Self::expr_uses_importlib(&generator.iter)
                            || generator.ifs.iter().any(Self::expr_uses_importlib)
                    })
            }
            Expr::DictComp(comp) => {
                Self::expr_uses_importlib(&comp.key)
                    || Self::expr_uses_importlib(&comp.value)
                    || comp.generators.iter().any(|generator| {
                        Self::expr_uses_importlib(&generator.iter)
                            || generator.ifs.iter().any(Self::expr_uses_importlib)
                    })
            }
            Expr::Generator(generator_exp) => {
                Self::expr_uses_importlib(&generator_exp.elt)
                    || generator_exp.generators.iter().any(|g| {
                        Self::expr_uses_importlib(&g.iter)
                            || g.ifs.iter().any(Self::expr_uses_importlib)
                    })
            }
            Expr::BoolOp(bool_op) => bool_op.values.iter().any(Self::expr_uses_importlib),
            Expr::UnaryOp(unary) => Self::expr_uses_importlib(&unary.operand),
            Expr::BinOp(bin_op) => {
                Self::expr_uses_importlib(&bin_op.left) || Self::expr_uses_importlib(&bin_op.right)
            }
            Expr::Compare(cmp) => {
                Self::expr_uses_importlib(&cmp.left)
                    || cmp.comparators.iter().any(Self::expr_uses_importlib)
            }
            Expr::If(if_exp) => {
                Self::expr_uses_importlib(&if_exp.test)
                    || Self::expr_uses_importlib(&if_exp.body)
                    || Self::expr_uses_importlib(&if_exp.orelse)
            }
            Expr::Lambda(lambda) => {
                // Check default parameter values
                lambda.parameters.as_ref().is_some_and(|params| {
                    params.args.iter().any(|arg| {
                        arg.default
                            .as_ref()
                            .is_some_and(|d| Self::expr_uses_importlib(d))
                    })
                }) || Self::expr_uses_importlib(&lambda.body)
            }
            Expr::Await(await_expr) => Self::expr_uses_importlib(&await_expr.value),
            Expr::Yield(yield_expr) => yield_expr
                .value
                .as_ref()
                .is_some_and(|v| Self::expr_uses_importlib(v)),
            Expr::YieldFrom(yield_from) => Self::expr_uses_importlib(&yield_from.value),
            Expr::Starred(starred) => Self::expr_uses_importlib(&starred.value),
            Expr::Named(named) => {
                Self::expr_uses_importlib(&named.target) || Self::expr_uses_importlib(&named.value)
            }
            Expr::Slice(slice) => {
                slice
                    .lower
                    .as_ref()
                    .is_some_and(|l| Self::expr_uses_importlib(l))
                    || slice
                        .upper
                        .as_ref()
                        .is_some_and(|u| Self::expr_uses_importlib(u))
                    || slice
                        .step
                        .as_ref()
                        .is_some_and(|s| Self::expr_uses_importlib(s))
            }
            // Literals don't use importlib
            Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_) => false,
            // F-strings and T-strings are unlikely to directly use importlib
            Expr::FString(_) => false,
            Expr::TString(_) => false,
            // IPython specific, unlikely to use importlib
            Expr::IpyEscapeCommand(_) => false,
        }
    }
}

impl<'a> std::fmt::Debug for HybridStaticBundler<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridStaticBundler")
            .field("module_registry", &self.module_registry)
            .field("entry_module_name", &self.entry_module_name)
            .field("bundled_modules", &self.bundled_modules)
            .field("inlined_modules", &self.inlined_modules)
            .finish()
    }
}

impl<'a> Default for HybridStaticBundler<'a> {
    fn default() -> Self {
        Self::new(None)
    }
}

// Main implementation
impl<'a> HybridStaticBundler<'a> {
    /// Create a new bundler instance
    pub fn new(module_info_registry: Option<&'a crate::orchestrator::ModuleRegistry>) -> Self {
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
            use_module_cache_model: true, /* Enable module cache by default for circular
                                           * dependencies */
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
    fn assign_node_indices_to_ast(&mut self, _module: &mut ModModule) {
        // TODO: Implement visitor to assign indices
    }

    /// Check if a statement is a hoisted import
    pub fn is_hoisted_import(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Import(import) => {
                // Check if any alias is in our stdlib imports
                import.names.iter().any(|alias| {
                    self.stdlib_import_from_map
                        .contains_key(&alias.name.to_string())
                })
            }
            Stmt::ImportFrom(from_import) => {
                if let Some(module) = &from_import.module {
                    let module_name = module.to_string();
                    self.stdlib_import_from_map.contains_key(&module_name)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Resolve a relative import with context
    pub fn resolve_relative_import_with_context(
        &self,
        _import_from: &StmtImportFrom,
        _current_module: &str,
        _module_path: Option<&Path>,
    ) -> Option<String> {
        // TODO: Implementation from original file
        // This is a placeholder - the actual implementation is quite complex
        None
    }

    /// Create module access expression
    pub fn create_module_access_expr(
        &self,
        module_name: &str,
        _symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Expr {
        // TODO: Implementation from original file
        // For now, just return a simple name expression
        Expr::Name(ExprName {
            id: Identifier::new(module_name, TextRange::default()).into(),
            ctx: ExprContext::Load,
            range: TextRange::default(),
            node_index: Default::default(),
        })
    }

    /// Rewrite import with renames
    pub fn rewrite_import_with_renames(
        &self,
        import_stmt: StmtImport,
        _symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        // TODO: Implementation from original file
        vec![Stmt::Import(import_stmt)]
    }

    /// Resolve relative import
    pub fn resolve_relative_import(
        &self,
        _import_from: &StmtImportFrom,
        _current_module: &str,
    ) -> Option<String> {
        // TODO: Implementation from original file
        None
    }

    /// Filter exports based on tree shaking
    pub fn filter_exports_by_tree_shaking(
        &self,
        module_name: &str,
        exports: &[String],
    ) -> Vec<String> {
        if let Some(ref keep_symbols) = self.tree_shaking_keep_symbols {
            exports
                .iter()
                .filter(|symbol| {
                    keep_symbols.contains(&(module_name.to_string(), symbol.to_string()))
                })
                .cloned()
                .collect()
        } else {
            exports.to_vec()
        }
    }

    /// Handle imports from inlined module
    pub fn handle_imports_from_inlined_module(
        &self,
        _module_name: &str,
        _names: &[Alias],
        _symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        _deferred_imports: &mut Vec<Stmt>,
        _is_entry_module: bool,
    ) -> Vec<Stmt> {
        // TODO: Implementation from original file
        vec![]
    }

    /// Rewrite import in statement with full context
    pub fn rewrite_import_in_stmt_multiple_with_full_context(
        &self,
        import_stmt: StmtImport,
        _symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        _deferred_imports: &mut Vec<Stmt>,
        _module_name: &str,
        _is_wrapper_init: bool,
        _local_variables: &FxIndexSet<String>,
        _is_entry_module: bool,
        _importlib_inlined_modules: &mut FxIndexMap<String, String>,
        _created_namespace_objects: &mut bool,
        _global_deferred_imports: Option<&FxIndexMap<(String, String), String>>,
    ) -> Vec<Stmt> {
        // TODO: Implementation from original file
        vec![Stmt::Import(import_stmt)]
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

    /// Bundle multiple modules using the hybrid approach
    pub fn bundle_modules(&mut self, params: BundleParams<'_>) -> Result<ModModule> {
        let final_body = Vec::new();

        // Store tree shaking decisions if provided
        if let Some(shaker) = params.tree_shaker {
            // Extract all kept symbols from the tree shaker
            let mut kept_symbols = indexmap::IndexSet::new();
            for (module_name, _, _, _) in &params.modules {
                for symbol in shaker.get_used_symbols_for_module(module_name) {
                    kept_symbols.insert((module_name.clone(), symbol));
                }
            }
            self.tree_shaking_keep_symbols = Some(kept_symbols);
            log::debug!(
                "Tree shaking enabled, keeping {} symbols",
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
        for (_module_name, ast, _, _) in &params.modules {
            self.collect_future_imports_from_ast(ast);
        }

        // Check if entry module has direct imports or dotted imports that might create namespace
        // objects - but only for first-party modules that we're actually bundling
        let _needs_types_for_entry_imports = {
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
        let _modules = self.trim_unused_imports_from_modules(
            params.modules,
            params.graph,
            params.tree_shaker,
        )?;

        // TODO: Implement the rest of the bundling logic - need to port over 1700+ lines

        Ok(ModModule {
            body: final_body,
            range: TextRange::default(),
            node_index: Default::default(),
        })
    }

    /// Trim unused imports from all modules
    fn trim_unused_imports_from_modules(
        &mut self,
        modules: Vec<(String, ModModule, PathBuf, String)>,
        graph: &DependencyGraph,
        tree_shaker: Option<&crate::tree_shaking::TreeShaker>,
    ) -> Result<Vec<(String, ModModule, PathBuf, String)>> {
        let mut trimmed_modules = Vec::new();

        for (module_name, mut ast, module_path, content_hash) in modules {
            log::debug!("Trimming unused imports from module: {module_name}");

            // Check if this is an __init__.py file
            let is_init_py =
                module_path.file_name().and_then(|name| name.to_str()) == Some("__init__.py");

            // Get unused imports from the graph
            if let Some(module_dep_graph) = graph.get_module_by_name(&module_name) {
                let mut unused_imports = module_dep_graph.find_unused_imports(is_init_py);

                // If tree shaking is enabled, also check if imported symbols were removed
                // Note: We only apply tree-shaking logic to "from module import symbol" style
                // imports, not to "import module" style imports, since module
                // imports set up namespace objects
                if let Some(shaker) = tree_shaker {
                    // Only apply tree-shaking-aware import removal if tree shaking is actually
                    // enabled Get the symbols that survive tree-shaking for
                    // this module
                    let used_symbols = shaker.get_used_symbols_for_module(&module_name);

                    // Check each import to see if it's only used by tree-shaken code
                    let import_items = module_dep_graph.get_all_import_items();
                    log::debug!(
                        "Checking {} import items in module '{}' for tree-shaking",
                        import_items.len(),
                        module_name
                    );
                    for (item_id, import_item) in import_items {
                        match &import_item.item_type {
                            crate::cribo_graph::ItemType::FromImport {
                                module: from_module,
                                names,
                                ..
                            } => {
                                // For from imports, check each imported name
                                for (imported_name, alias_opt) in names {
                                    let local_name = alias_opt.as_ref().unwrap_or(imported_name);

                                    // Skip if already marked as unused
                                    if unused_imports.iter().any(|u| u.name == *local_name) {
                                        continue;
                                    }

                                    // Skip if this is a re-export (in __all__ or explicit
                                    // re-export)
                                    if import_item.reexported_names.contains(local_name)
                                        || module_dep_graph.is_in_all_export(local_name)
                                    {
                                        log::debug!(
                                            "Skipping tree-shaking for re-exported import \
                                             '{local_name}' from '{from_module}'"
                                        );
                                        continue;
                                    }

                                    // Check if this import is only used by symbols that were
                                    // tree-shaken
                                    let mut used_by_surviving_code = false;

                                    // First check if any surviving symbol uses this import
                                    for symbol in &used_symbols {
                                        if module_dep_graph
                                            .does_symbol_use_import(symbol, local_name)
                                        {
                                            used_by_surviving_code = true;
                                            break;
                                        }
                                    }

                                    // Also check if the module has side effects and uses this
                                    // import at module level
                                    if !used_by_surviving_code
                                        && shaker.module_has_side_effects(&module_name)
                                    {
                                        // Check if any module-level code uses this import
                                        for item in module_dep_graph.items.values() {
                                            if matches!(
                                                item.item_type,
                                                crate::cribo_graph::ItemType::Expression
                                                    | crate::cribo_graph::ItemType::Assignment { .. }
                                            ) && item.read_vars.contains(local_name)
                                            {
                                                used_by_surviving_code = true;
                                                log::debug!(
                                                    "Import '{local_name}' is used by \
                                                     module-level code in module with side effects"
                                                );
                                                break;
                                            }
                                        }
                                    }

                                    if !used_by_surviving_code {
                                        // This import is not used by any surviving symbol or
                                        // module-level code
                                        log::debug!(
                                            "Import '{local_name}' from '{from_module}' is not \
                                             used by surviving code after tree-shaking"
                                        );
                                        unused_imports.push(crate::cribo_graph::UnusedImportInfo {
                                            item_id,
                                            name: local_name.clone(),
                                            module: from_module.clone(),
                                            is_reexport: import_item
                                                .reexported_names
                                                .contains(local_name),
                                        });
                                    }
                                }
                            }
                            crate::cribo_graph::ItemType::Import { module, .. } => {
                                // For regular imports (import module), check if they're only used
                                // by tree-shaken code
                                let import_name = module.split('.').next_back().unwrap_or(module);

                                log::debug!(
                                    "Checking module import '{import_name}' (full: '{module}') \
                                     for tree-shaking"
                                );

                                // Skip if already marked as unused
                                if unused_imports.iter().any(|u| u.name == *import_name) {
                                    continue;
                                }

                                // Skip if this is a re-export
                                if import_item.reexported_names.contains(import_name)
                                    || module_dep_graph.is_in_all_export(import_name)
                                {
                                    log::debug!(
                                        "Skipping tree-shaking for re-exported import \
                                         '{import_name}'"
                                    );
                                    continue;
                                }

                                // Check if this import is only used by symbols that were
                                // tree-shaken
                                let mut used_by_surviving_code = false;

                                // Check if any surviving symbol uses this import
                                log::debug!(
                                    "Checking if any of {} surviving symbols use import \
                                     '{import_name}'",
                                    used_symbols.len()
                                );
                                for symbol in &used_symbols {
                                    if module_dep_graph.does_symbol_use_import(symbol, import_name)
                                    {
                                        log::debug!(
                                            "Symbol '{symbol}' uses import '{import_name}'"
                                        );
                                        used_by_surviving_code = true;
                                        break;
                                    }
                                }

                                // Also check if any module-level code that has side effects uses it
                                if !used_by_surviving_code {
                                    log::debug!(
                                        "No surviving symbols use '{import_name}', checking \
                                         module-level side effects"
                                    );
                                    for item in module_dep_graph.items.values() {
                                        if item.has_side_effects
                                            && !matches!(
                                                item.item_type,
                                                crate::cribo_graph::ItemType::Import { .. }
                                                    | crate::cribo_graph::ItemType::FromImport { .. }
                                            )
                                            && (item.read_vars.contains(import_name)
                                                || item.eventual_read_vars.contains(import_name))
                                        {
                                            log::debug!(
                                                "Module-level item {:?} with side effects uses \
                                                 '{import_name}'",
                                                item.item_type
                                            );
                                            used_by_surviving_code = true;
                                            break;
                                        }
                                    }
                                }

                                // Special case: Check if this import is only used by assignment
                                // statements that were removed by
                                // tree-shaking (e.g., ABC = abc.ABC after normalizing
                                // from abc import ABC)
                                if !used_by_surviving_code {
                                    // Check if any assignment that uses this import is kept
                                    // TODO: Complete implementation
                                }

                                if !used_by_surviving_code {
                                    unused_imports.push(crate::cribo_graph::UnusedImportInfo {
                                        item_id,
                                        name: import_name.to_string(),
                                        module: module.clone(),
                                        is_reexport: false,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Remove unused imports from the AST
                if !unused_imports.is_empty() {
                    log::debug!(
                        "Removing {} unused imports from module {}",
                        unused_imports.len(),
                        module_name
                    );
                    // Filter out unused imports from the AST
                    ast.body
                        .retain(|stmt| !self.should_remove_import_stmt(stmt, &unused_imports));
                }
            }

            trimmed_modules.push((module_name, ast, module_path, content_hash));
        }

        Ok(trimmed_modules)
    }

    /// Find modules that are imported directly
    fn find_directly_imported_modules(
        &self,
        _modules: &[(String, ModModule, PathBuf, String)],
        _entry_module_name: &str,
    ) -> FxIndexSet<String> {
        // TODO: Implement
        FxIndexSet::default()
    }

    /// Find modules that are imported as namespaces
    fn find_namespace_imported_modules(
        &mut self,
        _modules: &[(String, ModModule, PathBuf, String)],
    ) {
        // TODO: Implement
    }

    /// Check if an import statement should be removed based on unused imports
    fn should_remove_import_stmt(
        &self,
        stmt: &Stmt,
        unused_imports: &[crate::cribo_graph::UnusedImportInfo],
    ) -> bool {
        match stmt {
            Stmt::Import(import_stmt) => {
                // Check if all names in this import are unused
                let should_remove = import_stmt.names.iter().all(|alias| {
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(alias.name.as_str());

                    unused_imports.iter().any(|unused| {
                        log::trace!(
                            "Checking if import '{}' matches unused '{}' from '{}'",
                            local_name,
                            unused.name,
                            unused.module
                        );
                        unused.name == local_name
                    })
                });

                if should_remove {
                    log::debug!(
                        "Removing import statement: {:?}",
                        import_stmt
                            .names
                            .iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>()
                    );
                }
                should_remove
            }
            Stmt::ImportFrom(import_from) => {
                // Skip __future__ imports - they're handled separately
                if import_from.module.as_ref().map(|m| m.as_str()) == Some("__future__") {
                    return false;
                }

                // Check if all names in this from-import are unused
                import_from.names.iter().all(|alias| {
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(alias.name.as_str());

                    unused_imports
                        .iter()
                        .any(|unused| unused.name == local_name)
                })
            }
            _ => false,
        }
    }

    // More methods to be moved from the original implementation...
}

/// Main entry point for bundling modules
pub fn bundle_modules(params: BundleParams) -> Result<ModModule> {
    let mut bundler = HybridStaticBundler::new(None);
    bundler.bundle_modules(params)
}

// Additional implementation blocks and helper functions should be moved here
// from the original code_generator.rs file
