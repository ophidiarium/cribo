use std::path::{Path, PathBuf};

use anyhow::Result;
use indexmap::{IndexMap as FxIndexMap, IndexSet as FxIndexSet};
use log::debug;
use ruff_python_ast::{
    Alias, Arguments, AtomicNodeIndex, Decorator, ExceptHandler, Expr, ExprAttribute, ExprCall,
    ExprContext, ExprName, ExprStringLiteral, Identifier, ModModule, Stmt, StmtAssign,
    StmtClassDef, StmtFunctionDef, StmtImport, StmtImportFrom, StringLiteral, StringLiteralFlags,
    StringLiteralValue, visitor::source_order::SourceOrderVisitor,
};
use ruff_text_size::TextRange;

use crate::{
    code_generator::{
        circular_deps::SymbolDependencyGraph,
        context::{
            BundleParams, HardDependency, InlineContext, ModuleGlobalInfo, ModuleTransformContext,
            ProcessGlobalsParams, SemanticContext,
        },
        import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
    },
    cribo_graph::CriboGraph as DependencyGraph,
    transformation_context::TransformationContext,
};

/// Direct import collection context
struct DirectImportContext<'a> {
    current_module: &'a str,
    module_path: &'a Path,
    modules: &'a [(String, ModModule, PathBuf, String)],
}

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
    fn assign_node_indices_to_ast(&mut self, module: &mut ModModule) {
        struct NodeIndexAssigner<'b, 'a> {
            bundler: &'b mut HybridStaticBundler<'a>,
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
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        self.handle_imports_from_inlined_module_with_context(
            import_from,
            module_name,
            symbol_renames,
            None,
        )
    }

    /// Handle imports from inlined modules with optional module context
    fn handle_imports_from_inlined_module_with_context(
        &self,
        _import_from: &StmtImportFrom,
        _module_name: &str,
        _symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        _module_context: Option<&str>,
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

    /// Rewrite import from statement with proper handling for bundled modules
    pub fn rewrite_import_from(
        &self,
        import_from: StmtImportFrom,
        current_module: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        inside_wrapper_init: bool,
    ) -> Vec<Stmt> {
        // Resolve relative imports to absolute module names
        log::debug!(
            "rewrite_import_from: Processing import {:?} in module '{}'",
            import_from.module.as_ref().map(|m| m.as_str()),
            current_module
        );
        let resolved_module_name = self.resolve_relative_import(&import_from, current_module);

        let Some(module_name) = resolved_module_name else {
            // If we can't resolve the module, return the original import
            log::warn!(
                "Could not resolve module name for import {:?}, keeping original import",
                import_from.module.as_ref().map(|m| m.as_str())
            );
            return vec![Stmt::ImportFrom(import_from)];
        };

        if !self.bundled_modules.contains(&module_name) {
            log::debug!(
                "Module '{module_name}' not found in bundled modules, checking if inlined or \
                 importing submodules"
            );

            // Check if this module is inlined
            if self.inlined_modules.contains(&module_name) {
                log::debug!(
                    "Module '{module_name}' is an inlined module, \
                     inside_wrapper_init={inside_wrapper_init}"
                );
                // Handle imports from inlined modules
                return self.handle_imports_from_inlined_module(
                    &import_from,
                    &module_name,
                    symbol_renames,
                );
            }

            // Check if we're importing submodules from a namespace package
            // e.g., from greetings import greeting where greeting is actually greetings.greeting
            let mut has_bundled_submodules = false;
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let full_module_path = format!("{module_name}.{imported_name}");
                if self.bundled_modules.contains(&full_module_path) {
                    has_bundled_submodules = true;
                    break;
                }
            }

            if !has_bundled_submodules {
                log::debug!(
                    "No bundled submodules found for module '{module_name}', checking if it's a \
                     wrapper module"
                );

                // Check if this module is in the module_registry (wrapper module)
                if self.module_registry.contains_key(&module_name) {
                    log::debug!("Module '{module_name}' is a wrapper module in module_registry");
                    // This is a wrapper module, we need to transform it
                    return self.transform_bundled_import_from_multiple_with_context(
                        import_from,
                        &module_name,
                        inside_wrapper_init,
                    );
                }

                // No bundled submodules, keep original import
                // For relative imports from non-bundled modules, convert to absolute import
                if import_from.level > 0 {
                    let mut absolute_import = import_from.clone();
                    absolute_import.level = 0;
                    absolute_import.module =
                        Some(Identifier::new(&module_name, TextRange::default()));
                    return vec![Stmt::ImportFrom(absolute_import)];
                }
                return vec![Stmt::ImportFrom(import_from)];
            }

            // We have bundled submodules, need to transform them
            log::debug!("Module '{module_name}' has bundled submodules, transforming imports");
            // Transform each submodule import
            return self.transform_namespace_package_imports(
                import_from,
                &module_name,
                symbol_renames,
            );
        }

        log::debug!(
            "Transforming bundled import from module: {module_name}, is wrapper: {}",
            self.module_registry.contains_key(&module_name)
        );

        // Check if this module is in the registry (wrapper approach)
        // or if it was inlined
        if self.module_registry.contains_key(&module_name) {
            // Module uses wrapper approach - transform to sys.modules access
            // For relative imports, we need to create an absolute import
            let mut absolute_import = import_from.clone();
            if import_from.level > 0 {
                // Convert relative import to absolute
                absolute_import.level = 0;
                absolute_import.module = Some(Identifier::new(&module_name, TextRange::default()));
            }
            self.transform_bundled_import_from_multiple_with_context(
                absolute_import,
                &module_name,
                inside_wrapper_init,
            )
        } else {
            // Module was inlined - create assignments for imported symbols
            log::debug!(
                "Module '{module_name}' was inlined, creating assignments for imported symbols"
            );
            self.create_assignments_for_inlined_imports(import_from, &module_name, symbol_renames)
        }
    }

    /// Transform bundled import from statement with context
    fn transform_bundled_import_from_multiple_with_context(
        &self,
        _import_from: StmtImportFrom,
        _module_name: &str,
        _inside_wrapper_init: bool,
    ) -> Vec<Stmt> {
        // TODO: Implementation from original file
        vec![]
    }

    /// Create assignments for symbols imported from inlined modules
    fn create_assignments_for_inlined_imports(
        &self,
        _import_from: StmtImportFrom,
        _module_name: &str,
        _symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        // TODO: Implementation from original file
        vec![]
    }

    /// Transform imports from namespace packages
    fn transform_namespace_package_imports(
        &self,
        _import_from: StmtImportFrom,
        _module_name: &str,
        _symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        // TODO: Implementation from original file
        vec![]
    }

    /// Get synthetic module name using content hash
    fn get_synthetic_module_name(&self, module_name: &str, content_hash: &str) -> String {
        // Use only the first 8 characters of the hash for readability
        let hash_prefix = &content_hash[..8.min(content_hash.len())];
        let safe_name = module_name.replace(['.', '-'], "_");
        format!("{safe_name}_{hash_prefix}")
    }

    /// Check if a string is a valid Python identifier
    fn is_valid_python_identifier(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let first_char = s.chars().next().unwrap();
        if !first_char.is_alphabetic() && first_char != '_' {
            return false;
        }
        s.chars().all(|c| c.is_alphanumeric() || c == '_')
    }

    /// Check if a module has side effects
    fn has_side_effects(ast: &ModModule) -> bool {
        ast.body.iter().any(|stmt| match stmt {
            // Imports don't count as side effects for our purposes
            Stmt::Import(_) | Stmt::ImportFrom(_) => false,
            // Definitions don't count as side effects
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => false,
            // Annotations and type aliases don't count
            Stmt::AnnAssign(ann) if ann.value.is_none() => false,
            Stmt::TypeAlias(_) => false,
            // __all__ assignment doesn't count
            Stmt::Assign(assign) => {
                if assign.targets.len() == 1 {
                    if let Expr::Name(name) = &assign.targets[0] {
                        name.id.as_str() != "__all__"
                    } else {
                        true
                    }
                } else {
                    true
                }
            }
            // Everything else is a potential side effect
            _ => true,
        })
    }

    /// Extract __all__ exports from a module
    fn extract_all_exports(&self, ast: &ModModule) -> (bool, Option<Vec<String>>) {
        for stmt in &ast.body {
            if let Stmt::Assign(assign) = stmt
                && assign.targets.len() == 1
                && let Expr::Name(name) = &assign.targets[0]
                && name.id.as_str() == "__all__"
            {
                // Try to extract the list of exports
                if let Expr::List(list_expr) = &assign.value.as_ref() {
                    let mut exports = Vec::new();
                    for elt in &list_expr.elts {
                        if let Expr::StringLiteral(s) = elt {
                            exports.push(s.value.to_str().to_string());
                        }
                    }
                    return (true, Some(exports));
                }
            }
        }
        (false, None)
    }

    /// Add a stdlib import to be hoisted
    fn add_stdlib_import(&mut self, module_name: &str) {
        self.stdlib_import_from_map
            .entry(module_name.to_string())
            .or_default();
    }

    /// Check if a module is a safe stdlib module
    fn is_safe_stdlib_module(&self, module_name: &str) -> bool {
        crate::side_effects::is_safe_stdlib_module(module_name)
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

    /// Identify required namespaces from module names
    fn identify_required_namespaces(&mut self, modules: &[(String, ModModule, PathBuf, String)]) {
        log::debug!(
            "Identifying required namespaces from {} modules",
            modules.len()
        );

        // Collect all module names first
        let all_module_names: FxIndexSet<String> =
            modules.iter().map(|(name, _, _, _)| name.clone()).collect();

        // For each module, check if it has parent namespaces
        for module_name in &all_module_names {
            if module_name.contains('.') {
                let parts: Vec<&str> = module_name.split('.').collect();

                // Create all parent namespaces
                for i in 1..parts.len() {
                    let namespace = parts[..i].join(".");
                    // Only add as required namespace if it's not an actual module
                    if !all_module_names.contains(&namespace) {
                        log::debug!("Module '{module_name}' requires namespace '{namespace}'");
                        self.required_namespaces.insert(namespace);
                    }
                }
            }
        }

        log::debug!(
            "Identified {} required namespaces: {:?}",
            self.required_namespaces.len(),
            self.required_namespaces
        );
    }

    /// Create namespace statements for required namespaces
    fn create_namespace_statements(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();

        // Sort namespaces by depth to create parent namespaces first
        let mut sorted_namespaces: Vec<String> = self.required_namespaces.iter().cloned().collect();
        sorted_namespaces.sort_by_key(|ns| ns.matches('.').count());

        for namespace in sorted_namespaces {
            if !self.created_namespaces.contains(&namespace) {
                let stmt = self.create_namespace_object(&namespace);
                statements.push(stmt);
                self.created_namespaces.insert(namespace);
            }
        }

        statements
    }

    /// Create a namespace object statement
    fn create_namespace_object(&mut self, namespace: &str) -> Stmt {
        // Create: namespace_name = types.SimpleNamespace()
        Stmt::Assign(StmtAssign {
            node_index: self
                .create_transformed_node(format!("Create namespace object for {namespace}")),
            targets: vec![Expr::Name(ExprName {
                node_index: self.create_node_index(),
                id: namespace.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                node_index: self.create_node_index(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: self.create_node_index(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: self.create_node_index(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: self.create_node_index(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create namespace attribute assignment
    fn create_namespace_attribute(&mut self, parent: &str, child: &str) -> Stmt {
        // Create: parent.child = types.SimpleNamespace()
        Stmt::Assign(StmtAssign {
            node_index: self
                .create_transformed_node(format!("Create namespace attribute {parent}.{child}")),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: self.create_node_index(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: self.create_node_index(),
                    id: parent.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(child, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                node_index: self.create_node_index(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: self.create_node_index(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: self.create_node_index(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: self.create_node_index(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Collect imports from a module for hoisting
    fn collect_imports_from_module(
        &mut self,
        ast: &ModModule,
        module_name: &str,
        _module_path: &Path,
    ) {
        log::debug!("Collecting imports from module: {module_name}");

        for stmt in &ast.body {
            match stmt {
                Stmt::ImportFrom(import_from) => {
                    if let Some(ref module) = import_from.module {
                        let module_str = module.as_str();

                        // Skip __future__ imports (handled separately)
                        if module_str == "__future__" {
                            continue;
                        }

                        // Check if this is a safe stdlib module
                        if self.is_safe_stdlib_module(module_str) {
                            let import_map = self
                                .stdlib_import_from_map
                                .entry(module_str.to_string())
                                .or_default();

                            for alias in &import_from.names {
                                let name = alias.name.as_str();
                                let alias_name =
                                    alias.asname.as_ref().map(|a| a.as_str().to_string());
                                import_map.insert(name.to_string(), alias_name);
                            }
                        }
                    }
                }
                Stmt::Import(import_stmt) => {
                    // Track regular import statements for stdlib modules
                    for alias in &import_stmt.names {
                        let module_name = alias.name.as_str();
                        if self.is_safe_stdlib_module(module_name) && alias.asname.is_none() {
                            self.stdlib_import_statements.push(stmt.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Collect module renames from semantic analysis
    fn collect_module_renames(
        &self,
        module_name: &str,
        _semantic_ctx: &SemanticContext,
        symbol_renames: &mut FxIndexMap<String, FxIndexMap<String, String>>,
    ) {
        // TODO: Implement based on semantic analysis
        // For now, just ensure the module has an entry
        symbol_renames.entry(module_name.to_string()).or_default();
    }

    /// Collect global symbols from modules
    fn collect_global_symbols(
        &self,
        modules: &[(String, ModModule, PathBuf, String)],
        entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut global_symbols = FxIndexSet::default();

        // Find entry module and collect its top-level symbols
        if let Some((_, ast, _, _)) = modules
            .iter()
            .find(|(name, _, _, _)| name == entry_module_name)
        {
            for stmt in &ast.body {
                match stmt {
                    Stmt::FunctionDef(func_def) => {
                        global_symbols.insert(func_def.name.to_string());
                    }
                    Stmt::ClassDef(class_def) => {
                        global_symbols.insert(class_def.name.to_string());
                    }
                    Stmt::Assign(assign) => {
                        for target in &assign.targets {
                            if let Expr::Name(name) = target {
                                global_symbols.insert(name.id.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        global_symbols
    }

    /// Sort wrapper modules by dependencies
    fn sort_wrapper_modules_by_dependencies(
        &self,
        wrapper_modules: &[(String, ModModule, PathBuf, String)],
        _graph: &DependencyGraph,
    ) -> Result<Vec<(String, ModModule, PathBuf, String)>> {
        // TODO: Implement proper dependency sorting
        // For now, just return in original order
        Ok(wrapper_modules.to_vec())
    }

    /// Build symbol dependency graph for circular modules
    fn build_symbol_dependency_graph(
        &mut self,
        modules: &[(String, ModModule, PathBuf, String)],
        graph: &DependencyGraph,
        _semantic_ctx: &SemanticContext,
    ) {
        // Collect dependencies for each circular module
        for (module_name, ast, _path, _source) in modules {
            self.symbol_dep_graph.collect_dependencies(
                module_name,
                ast,
                graph,
                &self.circular_modules,
            );
        }

        // Only perform topological sort if we have symbols in circular modules
        if self
            .symbol_dep_graph
            .should_sort_symbols(&self.circular_modules)
            && let Err(e) = self
                .symbol_dep_graph
                .topological_sort_symbols(&self.circular_modules)
        {
            // The error is already logged inside topological_sort_symbols
            log::error!("Failed to sort symbols: {e}");
        }
    }

    /// Detect hard dependencies in a module
    fn detect_hard_dependencies(
        &self,
        module_name: &str,
        ast: &ModModule,
        import_map: &FxIndexMap<String, (String, Option<String>)>,
    ) -> Vec<HardDependency> {
        let mut hard_deps = Vec::new();

        // Scan for class definitions
        for stmt in &ast.body {
            if let Stmt::ClassDef(class_def) = stmt {
                // Check if any base class is an imported symbol
                if let Some(arguments) = &class_def.arguments {
                    for arg in &arguments.args {
                        // Check if this is an attribute access (e.g.,
                        // requests.compat.MutableMapping)
                        if let Expr::Attribute(attr_expr) = arg {
                            if let Expr::Attribute(inner_attr) = &*attr_expr.value {
                                if let Expr::Name(name_expr) = &*inner_attr.value {
                                    let base_module = name_expr.id.as_str();
                                    let sub_module = inner_attr.attr.as_str();
                                    let attr_name = attr_expr.attr.as_str();

                                    // Check if this module.submodule is in our import map
                                    let full_module = format!("{base_module}.{sub_module}");
                                    if let Some((source_module, _alias)) =
                                        import_map.get(&full_module)
                                    {
                                        debug!(
                                            "Found hard dependency: class {} in module {} \
                                             inherits from {}.{}.{}",
                                            class_def.name.as_str(),
                                            module_name,
                                            base_module,
                                            sub_module,
                                            attr_name
                                        );

                                        hard_deps.push(HardDependency {
                                            module_name: module_name.to_string(),
                                            class_name: class_def.name.as_str().to_string(),
                                            base_class: format!(
                                                "{base_module}.{sub_module}.{attr_name}"
                                            ),
                                            source_module: source_module.clone(),
                                            imported_attr: attr_name.to_string(),
                                            alias: None, // No alias for multi-level imports
                                            alias_is_mandatory: false,
                                        });
                                    }
                                }
                            } else if let Expr::Name(name_expr) = &*attr_expr.value {
                                let module = name_expr.id.as_str();
                                let attr_name = attr_expr.attr.as_str();

                                // Check if this module is in our import map
                                if let Some((source_module, _import_info)) = import_map.get(module)
                                {
                                    debug!(
                                        "Found hard dependency: class {} in module {} inherits \
                                         from {}.{}",
                                        class_def.name.as_str(),
                                        module_name,
                                        module,
                                        attr_name
                                    );

                                    // For module.attr, we need to import the module itself
                                    hard_deps.push(HardDependency {
                                        module_name: module_name.to_string(),
                                        class_name: class_def.name.as_str().to_string(),
                                        base_class: format!("{module}.{attr_name}"),
                                        source_module: source_module.clone(),
                                        imported_attr: module.to_string(), /* Import the module,
                                                                            * not the attr */
                                        alias: None, // No alias for module.attr imports
                                        alias_is_mandatory: false,
                                    });
                                }
                            }
                        } else if let Expr::Name(name_expr) = arg {
                            // Direct name reference (e.g., MutableMapping)
                            let base_name = name_expr.id.as_str();

                            // Check if this name is in our import map
                            if let Some((source_module, original_name)) = import_map.get(base_name)
                            {
                                debug!(
                                    "Found hard dependency: class {} in module {} inherits from \
                                     {} (original: {:?})",
                                    class_def.name.as_str(),
                                    module_name,
                                    base_name,
                                    original_name
                                );

                                // Use the original imported name if available (for aliased imports)
                                let import_attr = original_name
                                    .clone()
                                    .unwrap_or_else(|| base_name.to_string());

                                // Check if this base_name is used as an alias
                                // If base_name != import_attr, then base_name is an alias
                                let has_alias = base_name != import_attr;

                                // Check if the alias is mandatory (i.e., the original name
                                // conflicts with a local definition)
                                let alias_is_mandatory = if has_alias {
                                    // Check if there's a local class with the same name as
                                    // import_attr
                                    self.check_local_name_conflict(ast, &import_attr)
                                } else {
                                    false
                                };

                                hard_deps.push(HardDependency {
                                    module_name: module_name.to_string(),
                                    class_name: class_def.name.as_str().to_string(),
                                    base_class: base_name.to_string(),
                                    source_module: source_module.clone(),
                                    imported_attr: import_attr,
                                    alias: if has_alias {
                                        Some(base_name.to_string())
                                    } else {
                                        None
                                    },
                                    alias_is_mandatory,
                                });
                            }
                        }
                    }
                }
            }
        }

        hard_deps
    }

    /// Generate module cache initialization
    fn generate_module_cache_init(&mut self) -> Stmt {
        // TODO: Implement module cache initialization
        Stmt::Pass(ruff_python_ast::StmtPass {
            node_index: self.create_node_index(),
            range: TextRange::default(),
        })
    }

    /// Generate module cache population
    fn generate_module_cache_population(
        &mut self,
        _modules: &[(String, ModModule, PathBuf, String)],
    ) -> Vec<Stmt> {
        // TODO: Implement module cache population
        Vec::new()
    }

    /// Generate sys.modules sync
    fn generate_sys_modules_sync(&mut self) -> Vec<Stmt> {
        // TODO: Implement sys.modules synchronization
        Vec::new()
    }

    /// Process wrapper module globals
    fn process_wrapper_module_globals(
        &mut self,
        _params: &ProcessGlobalsParams,
        _module_globals: &mut FxIndexMap<String, ModuleGlobalInfo>,
        _lifted_declarations: &mut Vec<Stmt>,
    ) {
        // TODO: Implement globals processing
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
                expression: Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "functools".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("cache", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                }),
            }];
            return Ok(Stmt::FunctionDef(func_def));
        }

        // Should not happen
        unreachable!("transform_module_to_init_function should return a FunctionDef")
    }

    /// Generate module init call
    fn generate_module_init_call(
        &mut self,
        _synthetic_name: &str,
        _temp_names: &mut FxIndexSet<String>,
    ) -> Vec<Stmt> {
        // TODO: Implement module init call generation
        Vec::new()
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

        // Reorder statements to ensure proper declaration order
        let statements = if self.circular_modules.contains(module_name) {
            self.reorder_statements_for_circular_module(module_name, ast.body)
        } else {
            // Even for non-circular modules, ensure module-level variables are declared
            // before functions that might use them
            self.reorder_statements_for_proper_declaration_order(ast.body)
        };

        // Process each statement in the module
        for stmt in statements {
            match &stmt {
                Stmt::Import(import_stmt) => {
                    // Imports have already been transformed by RecursiveImportTransformer
                    // Include them in the inlined output
                    if !self.is_hoisted_import(&stmt) {
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
                    if !self.is_hoisted_import(&stmt) {
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
                                    self.get_unique_name(&base_name, ctx.global_symbols)
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
                                self.get_unique_name(&base_name, ctx.global_symbols)
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
                            self.get_unique_name(&base_name, ctx.global_symbols)
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
                        Self::resolve_import_aliases_in_expr(returns, &ctx.import_aliases);
                        self.rewrite_aliases_in_expr(returns, &module_renames);
                    }

                    // Apply renames to parameter annotations
                    for param in &mut func_def_clone.parameters.args {
                        if let Some(ref mut annotation) = param.parameter.annotation {
                            Self::resolve_import_aliases_in_expr(annotation, &ctx.import_aliases);
                            self.rewrite_aliases_in_expr(annotation, &module_renames);
                        }
                    }

                    // Apply renames and resolve import aliases in function body
                    for body_stmt in &mut func_def_clone.body {
                        Self::resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
                        self.rewrite_aliases_in_stmt(body_stmt, &module_renames);
                        // Also apply semantic renames from context
                        if let Some(semantic_renames) = ctx.module_renames.get(module_name) {
                            self.rewrite_aliases_in_stmt(body_stmt, semantic_renames);
                        }
                    }

                    ctx.inlined_stmts.push(Stmt::FunctionDef(func_def_clone));
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

    /// Create namespace for inlined module
    fn create_namespace_for_inlined_module_static(
        &mut self,
        _module_name: &str,
        _module_renames: &FxIndexMap<String, String>,
    ) -> Stmt {
        // TODO: Implement namespace creation for inlined modules
        Stmt::Pass(ruff_python_ast::StmtPass {
            node_index: self.create_node_index(),
            range: TextRange::default(),
        })
    }

    /// Generate registries and hook
    fn generate_registries_and_hook(&mut self) -> Vec<Stmt> {
        // No longer needed - we don't use sys.modules or import hooks
        Vec::new()
    }

    /// Sort wrapped modules by dependencies
    fn sort_wrapped_modules_by_dependencies(
        &self,
        _wrapped_modules: &[String],
        _sorted_modules: &[(String, String)],
    ) -> Vec<String> {
        // TODO: Implement proper dependency sorting
        _wrapped_modules.to_vec()
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

    /// Generate submodule attributes with exclusions
    fn generate_submodule_attributes_with_exclusions(
        &mut self,
        _sorted_modules: &[(String, String)],
        _final_body: &mut Vec<Stmt>,
        _exclusions: &FxIndexSet<String>,
    ) {
        // TODO: Implement submodule attribute generation
    }

    /// Deduplicate deferred imports
    fn deduplicate_deferred_imports_with_existing(
        &self,
        _imports: Vec<Stmt>,
        _existing: &[Stmt],
    ) -> Vec<Stmt> {
        // TODO: Implement deduplication
        _imports
    }

    /// Check if import from is duplicate
    fn is_duplicate_import_from(
        &self,
        import_from: &StmtImportFrom,
        existing_body: &[Stmt],
    ) -> bool {
        if let Some(ref module) = import_from.module {
            let module_name = module.as_str();
            // For third-party imports, check if they're already in the body
            if !self.is_safe_stdlib_module(module_name)
                && !self.is_bundled_module_or_package(module_name)
            {
                return existing_body.iter().any(|existing| {
                    if let Stmt::ImportFrom(existing_import) = existing {
                        existing_import.module.as_ref().map(|m| m.as_str()) == Some(module_name)
                            && Self::import_names_match(&import_from.names, &existing_import.names)
                    } else {
                        false
                    }
                });
            }
        }
        false
    }

    /// Check if import is duplicate
    fn is_duplicate_import(&self, import_stmt: &StmtImport, existing_body: &[Stmt]) -> bool {
        import_stmt.names.iter().any(|alias| {
            let module_name = alias.name.as_str();
            // For third-party imports, check if they're already in the body
            if !self.is_safe_stdlib_module(module_name)
                && !self.is_bundled_module_or_package(module_name)
            {
                existing_body.iter().any(|existing| {
                    if let Stmt::Import(existing_import) = existing {
                        existing_import.names.iter().any(|existing_alias| {
                            existing_alias.name == alias.name
                                && existing_alias.asname == alias.asname
                        })
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        })
    }

    /// Check if two import name lists match
    fn import_names_match(names1: &[Alias], names2: &[Alias]) -> bool {
        if names1.len() != names2.len() {
            return false;
        }

        // Check if all names match (order doesn't matter)
        names1.iter().all(|n1| {
            names2
                .iter()
                .any(|n2| n1.name == n2.name && n1.asname == n2.asname)
        })
    }

    /// Check if a module is bundled or is a package containing bundled modules
    fn is_bundled_module_or_package(&self, module_name: &str) -> bool {
        // Direct check
        if self.bundled_modules.contains(module_name) {
            return true;
        }

        // Check if it's a package containing bundled modules
        // e.g., if "greetings.greeting" is bundled, then "greetings" is a package
        let package_prefix = format!("{module_name}.");
        self.bundled_modules
            .iter()
            .any(|bundled| bundled.starts_with(&package_prefix))
    }

    /// Extract attribute path from expression
    fn extract_attribute_path(&self, attr: &ExprAttribute) -> String {
        let mut parts = Vec::new();
        let mut current = attr;

        loop {
            parts.push(current.attr.as_str());
            match &*current.value {
                Expr::Attribute(parent_attr) => current = parent_attr,
                Expr::Name(name) => {
                    parts.push(name.id.as_str());
                    break;
                }
                _ => break,
            }
        }

        parts.reverse();
        parts.join(".")
    }

    /// Check if two expressions are equal
    fn expr_equals(expr1: &Expr, expr2: &Expr) -> bool {
        match (expr1, expr2) {
            (Expr::Name(n1), Expr::Name(n2)) => n1.id == n2.id,
            (Expr::Attribute(a1), Expr::Attribute(a2)) => {
                a1.attr == a2.attr && Self::expr_equals(&a1.value, &a2.value)
            }
            _ => false,
        }
    }

    /// Process entry module statement
    fn process_entry_module_statement(
        &mut self,
        stmt: &mut Stmt,
        _renames: &FxIndexMap<String, String>,
        final_body: &mut Vec<Stmt>,
    ) {
        // TODO: Implement entry module statement processing
        final_body.push(stmt.clone());
    }

    /// Bundle multiple modules using the hybrid approach
    pub fn bundle_modules(&mut self, params: BundleParams<'_>) -> Result<ModModule> {
        let mut final_body = Vec::new();

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
        let mut modules = self.trim_unused_imports_from_modules(
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
        let mut module_id_map: FxIndexMap<String, u32> = FxIndexMap::default();

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
        let mut _has_circular_dependencies = false;
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
            _has_circular_dependencies = !self.circular_modules.is_empty();
            log::debug!("Circular modules: {:?}", self.circular_modules);
        } else {
            log::debug!("No circular dependency analysis provided");
        }

        // Separate modules into inlinable and wrapper modules
        // Note: modules are already normalized before unused import trimming
        let mut inlinable_modules = Vec::new();
        let mut wrapper_modules = Vec::new();
        let mut module_exports_map: FxIndexMap<String, Option<Vec<String>>> = FxIndexMap::default();

        for (module_name, ast, module_path, content_hash) in &modules {
            log::debug!("Processing module: '{module_name}'");
            if module_name == params.entry_module_name {
                continue;
            }

            // Extract __all__ exports from the module
            let (has_explicit_all, module_exports) = self.extract_all_exports(ast);
            if has_explicit_all {
                self.modules_with_explicit_all.insert(module_name.clone());
            }
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
            let has_side_effects = Self::has_side_effects(ast);

            // Check if this module has an invalid identifier (can't be imported normally)
            // These modules are likely imported via importlib and need to be wrapped
            // Note: Module names with dots are valid (e.g., "core.utils.helpers"), so we only
            // check if the module name itself (without dots) is invalid
            let module_base_name = module_name.split('.').next_back().unwrap_or(module_name);
            let has_invalid_identifier = !Self::is_valid_python_identifier(module_base_name);

            if has_side_effects || has_invalid_identifier {
                if has_invalid_identifier {
                    log::debug!(
                        "Module '{module_name}' has invalid Python identifier - using wrapper \
                         approach"
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
        self.identify_required_namespaces(&all_modules_for_namespace_detection);

        // If we need to create namespace statements, ensure types import is available
        if !self.required_namespaces.is_empty() {
            log::debug!(
                "Need to create {} namespace statements - adding types import",
                self.required_namespaces.len()
            );
            self.add_stdlib_import("types");

            // Create namespace statements immediately after identifying them
            // This ensures namespaces exist before any module code that might reference them
            log::debug!(
                "Creating {} namespace statements before module inlining",
                self.required_namespaces.len()
            );
            let namespace_statements = self.create_namespace_statements();
            final_body.extend(namespace_statements);

            // For wrapper modules that are submodules (e.g., requests.compat),
            // we need to create placeholder attributes on their parent namespaces
            // so that inlined code can reference them before they're initialized
            for (module_name, _, _, _) in &modules {
                if module_name.contains('.') && module_name != "__init__" {
                    // Check if this is a wrapper module
                    let is_wrapper = modules.iter().any(|(name, ast, _, _)| {
                        name == module_name && Self::has_side_effects(ast)
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
                                    self.create_namespace_attribute(parent, child);
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
            self.add_stdlib_import("types");
        }

        // Collect imports from ALL modules (after normalization) for hoisting
        // This must be done on the normalized modules to capture stdlib imports
        // that were converted from "from X import Y" to "import X" format
        for (module_name, ast, module_path, _) in &modules {
            log::debug!("Collecting imports from module: {module_name}");
            self.collect_imports_from_module(ast, module_name, module_path);
        }

        // If we have wrapper modules, inject types as stdlib dependency
        // functools will be added later only if we use module cache
        if !wrapper_modules.is_empty() {
            log::debug!("Adding types import for wrapper modules");
            self.add_stdlib_import("types");
        }

        // If we have namespace imports, inject types as stdlib dependency
        if !self.namespace_imported_modules.is_empty() {
            log::debug!("Adding types import for namespace imports");
            self.add_stdlib_import("types");
        }

        // If entry module has direct imports or dotted imports that need namespace objects
        if _needs_types_for_entry_imports {
            log::debug!("Adding types import for namespace objects in entry module");
            self.add_stdlib_import("types");
        }

        // Register wrapper modules
        for (module_name, _ast, _module_path, content_hash) in &wrapper_modules {
            self.module_exports.insert(
                module_name.clone(),
                module_exports_map.get(module_name).cloned().flatten(),
            );

            // Register module with synthetic name using content hash
            let synthetic_name = self.get_synthetic_module_name(module_name, content_hash);
            self.module_registry
                .insert(module_name.clone(), synthetic_name.clone());

            // Register init function
            let init_func_name = format!("__cribo_init_{synthetic_name}");
            self.init_functions.insert(synthetic_name, init_func_name);
        }

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
        let mut global_symbols = self.collect_global_symbols(&modules, params.entry_module_name);

        // Save wrapper modules for later processing
        let wrapper_modules_saved = wrapper_modules;

        // Sort wrapper modules by their dependencies
        let _sorted_wrapper_modules =
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

            self.build_symbol_dependency_graph(&modules_for_graph, params.graph, &semantic_ctx);

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

        // Inline the inlinable modules FIRST to populate symbol_renames
        // This ensures we know what symbols have been renamed before processing wrapper modules and
        // namespace hybrids
        let mut all_deferred_imports = Vec::new();
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
            };
            self.inline_module(module_name, ast.clone(), _module_path, &mut inline_ctx)?;
            log::debug!(
                "Inlined {} statements from module '{}'",
                inlined_stmts.len(),
                module_name
            );
            final_body.extend(inlined_stmts);
            // Track deferred imports globally
            for stmt in &deferred_imports {
                if let Stmt::Assign(assign) = stmt {
                    // Check for pattern: symbol = sys.modules['module'].symbol
                    if let Expr::Attribute(attr) = &assign.value.as_ref()
                        && let Expr::Subscript(subscript) = &attr.value.as_ref()
                        && let Expr::Attribute(sys_attr) = &subscript.value.as_ref()
                        && let Expr::Name(sys_name) = &sys_attr.value.as_ref()
                        && sys_name.id.as_str() == "sys"
                        && sys_attr.attr.as_str() == "modules"
                        && let Expr::StringLiteral(lit) = &subscript.slice.as_ref()
                    {
                        let import_module = lit.value.to_str();
                        let attr_name = attr.attr.as_str();
                        log::debug!(
                            "Registering deferred import: {attr_name} from {import_module} \
                             (deferred by {module_name})"
                        );
                        self.global_deferred_imports.insert(
                            (import_module.to_string(), attr_name.to_string()),
                            module_name.to_string(),
                        );
                    }
                }
            }

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
                                    return Self::expr_equals(
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
                                let attr_path = self.extract_attribute_path(attr);
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

        // Create namespace objects for inlined modules that are imported as namespaces
        for (module_name, _, _, _) in &inlinable_modules {
            if self.namespace_imported_modules.contains_key(module_name) {
                log::debug!("Creating namespace for inlined module '{module_name}'");

                // Get the symbols that were inlined from this module
                if let Some(module_rename_map) = symbol_renames.get(module_name) {
                    // Create a SimpleNamespace for this module
                    let namespace_stmt = self
                        .create_namespace_for_inlined_module_static(module_name, module_rename_map);
                    final_body.push(namespace_stmt);
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
            let mut locally_defined_symbols: FxIndexSet<String> = FxIndexSet::default();
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
                self.add_stdlib_import("types");
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
                let is_hoisted = self.is_hoisted_import(stmt);
                if is_hoisted {
                    continue;
                }

                match stmt {
                    Stmt::ImportFrom(import_from) => {
                        let is_duplicate = self.is_duplicate_import_from(import_from, &final_body);

                        if !is_duplicate {
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
                        let is_duplicate = self.is_duplicate_import(import_stmt, &final_body);

                        if !is_duplicate {
                            // Imports have already been transformed by RecursiveImportTransformer
                            final_body.push(stmt.clone());
                        } else {
                            log::debug!("Skipping duplicate import in entry module");
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
        }

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
        modules: &[(String, ModModule, PathBuf, String)],
        _entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut directly_imported = FxIndexSet::default();

        // Check all modules for direct imports (both module-level and function-scoped)
        for (module_name, ast, module_path, _) in modules {
            log::debug!("Checking module '{module_name}' for direct imports");
            let ctx = DirectImportContext {
                current_module: module_name,
                module_path,
                modules,
            };
            for stmt in &ast.body {
                self.collect_direct_imports(stmt, &ctx, &mut directly_imported);
                // Also check for imports inside functions, classes, etc.
                self.collect_direct_imports_recursive(stmt, &ctx, &mut directly_imported);
            }
        }

        log::debug!(
            "Found {} directly imported modules: {:?}",
            directly_imported.len(),
            directly_imported
        );
        directly_imported
    }

    /// Helper to collect direct imports from a statement
    fn collect_direct_imports(
        &self,
        stmt: &Stmt,
        ctx: &DirectImportContext<'_>,
        directly_imported: &mut FxIndexSet<String>,
    ) {
        match stmt {
            Stmt::Import(import_stmt) => {
                for alias in &import_stmt.names {
                    let imported_module = alias.name.as_str();
                    // Check if this is a bundled module
                    if ctx
                        .modules
                        .iter()
                        .any(|(name, _, _, _)| name == imported_module)
                    {
                        directly_imported.insert(imported_module.to_string());

                        // When importing a submodule, Python implicitly imports parent packages
                        // For example, importing 'greetings.irrelevant' also imports 'greetings'
                        if imported_module.contains('.') {
                            self.mark_parent_packages_as_imported(
                                imported_module,
                                ctx.modules,
                                directly_imported,
                            );
                        }
                    }
                }
            }
            Stmt::ImportFrom(import_from) => {
                if let Some(module) = &import_from.module {
                    let module_name = module.as_str();
                    // Check if any imported name is actually a submodule
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        let full_module_path = format!("{module_name}.{imported_name}");

                        log::debug!(
                            "Checking if '{full_module_path}' (from {module_name} import \
                             {imported_name}) is a bundled module"
                        );

                        // Check if this full path matches a bundled module
                        if ctx
                            .modules
                            .iter()
                            .any(|(name, _, _, _)| name == &full_module_path)
                        {
                            // This is importing a submodule directly
                            log::debug!(
                                "Found submodule import: from {module_name} import \
                                 {imported_name} -> {full_module_path}"
                            );
                            // Note: We don't mark this as directly imported anymore
                            // `from models import base` should allow `models.base` to be inlined
                            // if it has no side effects. Only `import models.base` should
                            // force wrapping.
                        }
                    }
                } else if import_from.level > 0 {
                    // Handle relative imports (e.g., from . import greeting)
                    self.collect_direct_relative_imports(import_from, ctx, directly_imported);
                }
            }
            _ => {}
        }
    }

    /// Recursively collect direct imports from nested statements
    fn collect_direct_imports_recursive(
        &self,
        stmt: &Stmt,
        ctx: &DirectImportContext<'_>,
        directly_imported: &mut FxIndexSet<String>,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                for body_stmt in &func_def.body {
                    self.collect_direct_imports(body_stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(body_stmt, ctx, directly_imported);
                }
            }
            Stmt::ClassDef(class_def) => {
                for body_stmt in &class_def.body {
                    self.collect_direct_imports(body_stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(body_stmt, ctx, directly_imported);
                }
            }
            Stmt::If(if_stmt) => {
                for body_stmt in &if_stmt.body {
                    self.collect_direct_imports(body_stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(body_stmt, ctx, directly_imported);
                }
                for clause in &if_stmt.elif_else_clauses {
                    for body_stmt in &clause.body {
                        self.collect_direct_imports(body_stmt, ctx, directly_imported);
                        self.collect_direct_imports_recursive(body_stmt, ctx, directly_imported);
                    }
                }
            }
            // Add other compound statements as needed...
            _ => {}
        }
    }

    /// Mark parent packages as imported
    fn mark_parent_packages_as_imported(
        &self,
        module_path: &str,
        modules: &[(String, ModModule, PathBuf, String)],
        directly_imported: &mut FxIndexSet<String>,
    ) {
        let parts: Vec<&str> = module_path.split('.').collect();
        for i in 1..parts.len() {
            let parent_path = parts[0..i].join(".");
            if modules.iter().any(|(name, _, _, _)| name == &parent_path) {
                directly_imported.insert(parent_path);
            }
        }
    }

    /// Collect direct relative imports
    fn collect_direct_relative_imports(
        &self,
        import_from: &StmtImportFrom,
        ctx: &DirectImportContext<'_>,
        directly_imported: &mut FxIndexSet<String>,
    ) {
        let resolved_module = self.resolve_relative_import_with_context(
            import_from,
            ctx.current_module,
            Some(ctx.module_path),
        );

        let Some(base_module) = resolved_module else {
            return;
        };

        // Check if any imported name is actually a submodule
        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let full_module_path = format!("{base_module}.{imported_name}");

            log::debug!(
                "Checking if '{full_module_path}' (from . import {imported_name}) is a bundled \
                 module"
            );

            // Check if this full path matches a bundled module
            let is_bundled_module = ctx
                .modules
                .iter()
                .any(|(name, _, _, _)| name == &full_module_path);

            if is_bundled_module {
                // This is importing a submodule directly
                log::debug!(
                    "Found direct submodule import via relative import: from . import \
                     {imported_name} -> {full_module_path}"
                );
                directly_imported.insert(full_module_path);
            }
        }
    }

    /// Find modules that are imported as namespaces
    fn find_namespace_imported_modules(
        &mut self,
        modules: &[(String, ModModule, PathBuf, String)],
    ) {
        log::debug!(
            "find_namespace_imported_modules: Checking {} modules",
            modules.len()
        );

        // Check all modules for namespace imports
        for (importing_module, ast, _, _) in modules {
            log::debug!("Checking module '{importing_module}' for namespace imports");
            for stmt in &ast.body {
                self.collect_namespace_imports(stmt, modules, importing_module);
            }
        }

        log::debug!(
            "Found {} namespace imported modules: {:?}",
            self.namespace_imported_modules.len(),
            self.namespace_imported_modules
        );
    }

    /// Helper to collect namespace imports from a statement
    fn collect_namespace_imports(
        &mut self,
        stmt: &Stmt,
        modules: &[(String, ModModule, PathBuf, String)],
        importing_module: &str,
    ) {
        match stmt {
            Stmt::ImportFrom(import_from) if import_from.module.is_some() => {
                let module = import_from
                    .module
                    .as_ref()
                    .expect("module was checked to be Some");
                let module_name = module.as_str();
                log::debug!(
                    "  Found import from '{}' with names: {:?} in '{}'",
                    module_name,
                    import_from
                        .names
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>(),
                    importing_module
                );

                // Check if any imported name is actually a submodule
                for alias in &import_from.names {
                    let imported_name = alias.name.as_str();
                    let full_module_path = format!("{module_name}.{imported_name}");

                    log::debug!(
                        "  Checking if '{full_module_path}' (from {module_name} import \
                         {imported_name}) is a bundled module in namespace import check"
                    );

                    // Check if this full path matches a bundled module
                    let is_namespace_import = modules.iter().any(|(name, _, _, _)| {
                        name == &full_module_path || name.ends_with(&full_module_path)
                    });

                    if is_namespace_import {
                        // Find the actual module name that matched
                        let actual_module_name =
                            Self::find_matching_module_name_namespace(modules, &full_module_path);

                        // This is importing a submodule as a namespace
                        log::debug!(
                            "  Found namespace import: from {module_name} import {imported_name} \
                             -> {full_module_path} (actual: {actual_module_name}) in module \
                             {importing_module}"
                        );
                        self.namespace_imported_modules
                            .entry(actual_module_name)
                            .or_default()
                            .insert(importing_module.to_string());
                    }
                }
            }
            // Recursively check function bodies
            Stmt::FunctionDef(func_def) => {
                for body_stmt in &func_def.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            // Recursively check class bodies and methods
            Stmt::ClassDef(class_def) => {
                for body_stmt in &class_def.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            // Recursively check other compound statements
            Stmt::If(if_stmt) => {
                for body_stmt in &if_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for clause in &if_stmt.elif_else_clauses {
                    for body_stmt in &clause.body {
                        self.collect_namespace_imports(body_stmt, modules, importing_module);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                for body_stmt in &while_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for body_stmt in &while_stmt.orelse {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            Stmt::For(for_stmt) => {
                for body_stmt in &for_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for body_stmt in &for_stmt.orelse {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            Stmt::With(with_stmt) => {
                for body_stmt in &with_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            Stmt::Try(try_stmt) => {
                for body_stmt in &try_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    for body_stmt in &eh.body {
                        self.collect_namespace_imports(body_stmt, modules, importing_module);
                    }
                }
                for body_stmt in &try_stmt.orelse {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for body_stmt in &try_stmt.finalbody {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            _ => {}
        }
    }

    /// Find matching module name for namespace imports
    fn find_matching_module_name_namespace(
        modules: &[(String, ModModule, PathBuf, String)],
        full_module_path: &str,
    ) -> String {
        // Find the actual module name that matched
        modules
            .iter()
            .find_map(|(name, _, _, _)| {
                if name == full_module_path || name.ends_with(full_module_path) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| full_module_path.to_string())
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

    /// Create module object statements (types.SimpleNamespace)
    pub fn create_module_object_stmt(&self, module_name: &str, _module_path: &Path) -> Vec<Stmt> {
        let module_call = Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::dummy(),
            func: Box::new(Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "types".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("SimpleNamespace", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                node_index: AtomicNodeIndex::dummy(),
                args: Box::from([]),
                keywords: Box::from([]),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        vec![
            // module = types.SimpleNamespace()
            Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "module".into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(module_call),
                range: TextRange::default(),
            }),
            // module.__name__ = "module_name"
            Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "module".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("__name__", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: StringLiteralValue::single(StringLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        range: TextRange::default(),
                        value: module_name.to_string().into_boxed_str(),
                        flags: StringLiteralFlags::empty(),
                    }),
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }),
        ]
    }

    /// Create module attribute assignment
    pub fn create_module_attr_assignment(&self, module_var: &str, attr_name: &str) -> Stmt {
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: module_var.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(attr_name, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: attr_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
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
            exports.contains(&symbol_name.to_string())
        } else {
            // No __all__ defined, use default Python visibility rules
            // Export all symbols that don't start with underscore
            !symbol_name.starts_with('_')
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
    pub fn is_self_referential_assignment(&self, assign: &StmtAssign) -> bool {
        // Check if this is a simple assignment with a single target and value
        if assign.targets.len() == 1
            && let (Expr::Name(target), Expr::Name(value)) =
                (&assign.targets[0], assign.value.as_ref())
        {
            // It's self-referential if target and value have the same name
            let is_self_ref = target.id == value.id;
            if is_self_ref {
                log::debug!(
                    "Found self-referential assignment: {} = {}",
                    target.id,
                    value.id
                );
            }
            return is_self_ref;
        }
        false
    }

    /// Check if an assignment references a module that will be created as a namespace
    fn assignment_references_namespace_module(
        &self,
        assign: &StmtAssign,
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
        false
    }

    /// Create a reassignment statement (original_name = renamed_name)
    fn create_reassignment(&self, original_name: &str, renamed_name: &str) -> Stmt {
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: original_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: renamed_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Generate module namespace class definition
    fn generate_module_namespace_class(&mut self) -> Stmt {
        // class _ModuleNamespace:
        //     pass
        let class_def = StmtClassDef {
            node_index: self.create_node_index(),
            decorator_list: vec![],
            name: Identifier::new("_ModuleNamespace", TextRange::default()),
            type_params: None,
            arguments: None,
            body: vec![Stmt::Pass(ruff_python_ast::StmtPass {
                node_index: self.create_node_index(),
                range: TextRange::default(),
            })],
            range: TextRange::default(),
        };

        Stmt::ClassDef(class_def)
    }

    /// Check if a name conflicts with any local definition in the module
    fn check_local_name_conflict(&self, ast: &ModModule, name: &str) -> bool {
        for stmt in &ast.body {
            match stmt {
                Stmt::ClassDef(class_def) => {
                    if class_def.name.as_str() == name {
                        return true;
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    if func_def.name.as_str() == name {
                        return true;
                    }
                }
                Stmt::Assign(assign_stmt) => {
                    for target in &assign_stmt.targets {
                        if let Expr::Name(name_expr) = target
                            && name_expr.id.as_str() == name
                        {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Create a string literal expression
    fn create_string_literal(&self, value: &str) -> Expr {
        Expr::StringLiteral(ExprStringLiteral {
            node_index: AtomicNodeIndex::dummy(),
            value: StringLiteralValue::single(StringLiteral {
                node_index: AtomicNodeIndex::dummy(),
                value: value.to_string().into(),
                flags: StringLiteralFlags::empty(),
                range: TextRange::default(),
            }),
            range: TextRange::default(),
        })
    }

    /// Check if a specific module is in our hoisted stdlib imports
    fn is_import_in_hoisted_stdlib(&self, module_name: &str) -> bool {
        // Check if module is in our from imports map
        if self.stdlib_import_from_map.contains_key(module_name) {
            return true;
        }

        // Check if module is in our regular import statements
        self.stdlib_import_statements.iter().any(|hoisted| {
            matches!(hoisted, Stmt::Import(hoisted_import)
                if hoisted_import.names.iter().any(|alias| alias.name.as_str() == module_name))
        })
    }

    /// Generate a unique symbol name to avoid conflicts
    fn generate_unique_name(
        &self,
        base_name: &str,
        existing_symbols: &FxIndexSet<String>,
    ) -> String {
        if !existing_symbols.contains(base_name) {
            return base_name.to_string();
        }

        // Try adding numeric suffixes
        for i in 1..1000 {
            let candidate = format!("{base_name}_{i}");
            if !existing_symbols.contains(&candidate) {
                return candidate;
            }
        }

        // Fallback with module prefix
        format!("__cribo_renamed_{base_name}")
    }

    /// Sanitize a module name for use in a Python identifier
    /// This is a simple character replacement - collision handling should be done by the caller
    fn sanitize_module_name_for_identifier(name: &str) -> String {
        name.chars()
            .map(|c| match c {
                // Replace common invalid characters with descriptive names
                '-' => '_',
                '.' => '_',
                ' ' => '_',
                // For other non-alphanumeric characters, replace with underscore
                c if c.is_alphanumeric() || c == '_' => c,
                _ => '_',
            })
            .collect::<String>()
    }

    /// Collect unique imports from an import statement
    fn collect_unique_imports(
        &self,
        import_stmt: &StmtImport,
        seen_modules: &mut FxIndexSet<String>,
        unique_imports: &mut Vec<(String, Stmt)>,
    ) {
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            if seen_modules.contains(module_name) {
                continue;
            }
            seen_modules.insert(module_name.to_string());
            // Create import statement preserving the original alias
            unique_imports.push((
                module_name.to_string(),
                Stmt::Import(StmtImport {
                    node_index: AtomicNodeIndex::dummy(),
                    names: vec![Alias {
                        node_index: AtomicNodeIndex::dummy(),
                        name: Identifier::new(module_name, TextRange::default()),
                        asname: alias.asname.clone(),
                        range: TextRange::default(),
                    }],
                    range: TextRange::default(),
                }),
            ));
        }
    }

    /// Ensure a namespace exists, creating it and any parent namespaces if needed
    /// Returns statements to create any missing namespaces
    fn ensure_namespace_exists(&mut self, namespace_path: &str) -> Vec<Stmt> {
        let mut statements = Vec::new();

        // For dotted names like "models.user", we need to ensure "models" exists first
        if namespace_path.contains('.') {
            let parts: Vec<&str> = namespace_path.split('.').collect();

            // Create all parent namespaces
            for i in 1..=parts.len() {
                let namespace = parts[..i].join(".");

                if !self.created_namespaces.contains(&namespace) {
                    debug!("Creating namespace dynamically: {namespace}");

                    if i == 1 {
                        // Top-level namespace
                        statements.extend(self.create_namespace_module(&namespace));
                    } else {
                        // Nested namespace - create as attribute
                        let parent = parts[..i - 1].join(".");
                        let child = parts[i - 1];
                        statements.push(self.create_namespace_attribute(&parent, child));
                    }

                    self.created_namespaces.insert(namespace);
                }
            }
        } else {
            // Simple namespace without dots
            if !self.created_namespaces.contains(namespace_path) {
                debug!("Creating simple namespace dynamically: {namespace_path}");
                statements.extend(self.create_namespace_module(namespace_path));
                self.created_namespaces.insert(namespace_path.to_string());
            }
        }

        statements
    }

    /// Create a namespace module using types.SimpleNamespace
    fn create_namespace_module(&self, namespace_name: &str) -> Vec<Stmt> {
        vec![
            // namespace = types.SimpleNamespace()
            Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: namespace_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Call(ExprCall {
                    node_index: AtomicNodeIndex::dummy(),
                    func: Box::new(Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: "types".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("SimpleNamespace", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    arguments: Arguments {
                        node_index: AtomicNodeIndex::dummy(),
                        args: Box::from([]),
                        keywords: Box::from([]),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }),
        ]
    }

    /// Process a function definition in the entry module
    fn process_entry_module_function(
        &self,
        func_def: &mut ruff_python_ast::StmtFunctionDef,
        entry_module_renames: &FxIndexMap<String, String>,
    ) -> Option<(String, String)> {
        let func_name = func_def.name.to_string();
        let needs_reassignment = if let Some(new_name) = entry_module_renames.get(&func_name) {
            debug!("Renaming function '{func_name}' to '{new_name}' in entry module");
            func_def.name = Identifier::new(new_name, TextRange::default());
            true
        } else {
            false
        };

        // TODO: Add special handling for global statements in function bodies
        if needs_reassignment {
            Some((func_name, func_def.name.to_string()))
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
        if let Some(new_name) = entry_module_renames.get(&class_name) {
            debug!("Renaming class '{class_name}' to '{new_name}' in entry module");
            class_def.name = Identifier::new(new_name, TextRange::default());
            Some((class_name, new_name.clone()))
        } else {
            None
        }
    }

    /// Rewrite aliases in a statement based on renames
    fn rewrite_aliases_in_stmt(
        &self,
        stmt: &mut Stmt,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::Expr(expr_stmt) => {
                self.rewrite_aliases_in_expr(&mut expr_stmt.value, alias_to_canonical);
            }
            Stmt::Assign(assign) => {
                self.rewrite_aliases_in_expr(&mut assign.value, alias_to_canonical);
                // Don't transform targets - we only rewrite aliases in expressions
            }
            Stmt::Return(ret_stmt) => {
                if let Some(value) = &mut ret_stmt.value {
                    self.rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Stmt::If(if_stmt) => {
                self.rewrite_aliases_in_expr(&mut if_stmt.test, alias_to_canonical);
                for body_stmt in &mut if_stmt.body {
                    self.rewrite_aliases_in_stmt(body_stmt, alias_to_canonical);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(test) = &mut clause.test {
                        self.rewrite_aliases_in_expr(test, alias_to_canonical);
                    }
                    for body_stmt in &mut clause.body {
                        self.rewrite_aliases_in_stmt(body_stmt, alias_to_canonical);
                    }
                }
            }
            Stmt::For(for_stmt) => {
                self.rewrite_aliases_in_expr(&mut for_stmt.iter, alias_to_canonical);
                for body_stmt in &mut for_stmt.body {
                    self.rewrite_aliases_in_stmt(body_stmt, alias_to_canonical);
                }
                for orelse_stmt in &mut for_stmt.orelse {
                    self.rewrite_aliases_in_stmt(orelse_stmt, alias_to_canonical);
                }
            }
            Stmt::While(while_stmt) => {
                self.rewrite_aliases_in_expr(&mut while_stmt.test, alias_to_canonical);
                for body_stmt in &mut while_stmt.body {
                    self.rewrite_aliases_in_stmt(body_stmt, alias_to_canonical);
                }
                for orelse_stmt in &mut while_stmt.orelse {
                    self.rewrite_aliases_in_stmt(orelse_stmt, alias_to_canonical);
                }
            }
            _ => {}
        }
    }

    /// Check if an assignment statement needs a reassignment due to renaming
    fn check_renamed_assignment(
        &self,
        assign: &StmtAssign,
        renames: &FxIndexMap<String, String>,
    ) -> Option<(String, String)> {
        // Check if any target was renamed
        for target in &assign.targets {
            if let Expr::Name(name) = target {
                let original_name = name.id.as_str();
                if let Some(renamed) = renames.get(original_name) {
                    return Some((original_name.to_string(), renamed.clone()));
                }
            }
        }
        None
    }

    /// Transform a module into an initialization function
    /// This wraps the module body in a function that creates and returns a module object
    pub fn transform_module_to_init_function(
        &self,
        ctx: ModuleTransformContext,
        ast: ModModule,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Result<Stmt> {
        // Delegate to the module_transformer module to keep bundler.rs manageable
        crate::code_generator::module_transformer::transform_module_to_init_function(
            self,
            ctx,
            ast,
            symbol_renames,
        )
    }

    /// Add module attribute assignment if the symbol should be exported
    pub fn add_module_attr_if_exported(
        &self,
        assign: &StmtAssign,
        module_name: &str,
        body: &mut Vec<Stmt>,
    ) {
        if let Some(name) = self.extract_simple_assign_target(assign)
            && self.should_export_symbol(&name, module_name)
        {
            body.push(self.create_module_attr_assignment("module", &name));
        }
    }

    /// Collect variables referenced in statements
    pub fn collect_referenced_vars(&self, stmts: &[Stmt], vars: &mut FxIndexSet<String>) {
        for stmt in stmts {
            self.collect_vars_in_stmt(stmt, vars);
        }
    }

    /// Collect variable names referenced in a statement
    fn collect_vars_in_stmt(&self, stmt: &Stmt, vars: &mut FxIndexSet<String>) {
        match stmt {
            Stmt::Expr(expr_stmt) => Self::collect_vars_in_expr(&expr_stmt.value, vars),
            Stmt::Return(ret) => {
                if let Some(value) = &ret.value {
                    Self::collect_vars_in_expr(value, vars);
                }
            }
            Stmt::Assign(assign) => {
                Self::collect_vars_in_expr(&assign.value, vars);
            }
            Stmt::If(if_stmt) => {
                Self::collect_vars_in_expr(&if_stmt.test, vars);
                self.collect_referenced_vars(&if_stmt.body, vars);
                for clause in &if_stmt.elif_else_clauses {
                    if let Some(condition) = &clause.test {
                        Self::collect_vars_in_expr(condition, vars);
                    }
                    self.collect_referenced_vars(&clause.body, vars);
                }
            }
            Stmt::For(for_stmt) => {
                Self::collect_vars_in_expr(&for_stmt.iter, vars);
                self.collect_referenced_vars(&for_stmt.body, vars);
                self.collect_referenced_vars(&for_stmt.orelse, vars);
            }
            Stmt::While(while_stmt) => {
                Self::collect_vars_in_expr(&while_stmt.test, vars);
                self.collect_referenced_vars(&while_stmt.body, vars);
                self.collect_referenced_vars(&while_stmt.orelse, vars);
            }
            Stmt::Try(try_stmt) => {
                self.collect_referenced_vars(&try_stmt.body, vars);
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    self.collect_referenced_vars(&eh.body, vars);
                }
                self.collect_referenced_vars(&try_stmt.orelse, vars);
                self.collect_referenced_vars(&try_stmt.finalbody, vars);
            }
            Stmt::With(with_stmt) => {
                for item in &with_stmt.items {
                    Self::collect_vars_in_expr(&item.context_expr, vars);
                }
                self.collect_referenced_vars(&with_stmt.body, vars);
            }
            _ => {}
        }
    }

    /// Process module body recursively to handle conditional imports
    pub fn process_body_recursive(
        &self,
        body: Vec<Stmt>,
        module_name: &str,
        module_scope_symbols: Option<&FxIndexSet<String>>,
    ) -> Vec<Stmt> {
        let mut result = Vec::new();

        for stmt in body {
            match &stmt {
                Stmt::If(if_stmt) => {
                    // Process if body recursively
                    let processed_body = self.process_body_recursive(
                        if_stmt.body.clone(),
                        module_name,
                        module_scope_symbols,
                    );

                    // Process elif/else clauses
                    let processed_elif_else = if_stmt
                        .elif_else_clauses
                        .iter()
                        .map(|clause| {
                            let processed_clause_body = self.process_body_recursive(
                                clause.body.clone(),
                                module_name,
                                module_scope_symbols,
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
                    // Process try body recursively
                    let processed_body = self.process_body_recursive(
                        try_stmt.body.clone(),
                        module_name,
                        module_scope_symbols,
                    );

                    // Process handlers
                    let processed_handlers = try_stmt
                        .handlers
                        .iter()
                        .map(|handler| {
                            let ExceptHandler::ExceptHandler(handler) = handler;
                            let processed_handler_body = self.process_body_recursive(
                                handler.body.clone(),
                                module_name,
                                module_scope_symbols,
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

                    // Process orelse
                    let processed_orelse = self.process_body_recursive(
                        try_stmt.orelse.clone(),
                        module_name,
                        module_scope_symbols,
                    );

                    // Process finalbody
                    let processed_finalbody = self.process_body_recursive(
                        try_stmt.finalbody.clone(),
                        module_name,
                        module_scope_symbols,
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

                        // Add module attribute assignments for imported symbols
                        if let Some(symbols) = module_scope_symbols {
                            for alias in &import_from.names {
                                let local_name =
                                    alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                                if symbols.contains(local_name)
                                    && self.should_export_symbol(local_name, module_name)
                                {
                                    log::debug!(
                                        "Adding module.{local_name} = {local_name} after \
                                         conditional import"
                                    );
                                    result.push(
                                        self.create_module_attr_assignment("module", local_name),
                                    );
                                }
                            }
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

    /// Find module ID in semantic bundler using the module registry
    pub fn find_module_id_in_semantic_bundler(
        &self,
        module_name: &str,
        _semantic_bundler: &crate::semantic_bundler::SemanticBundler,
    ) -> Option<crate::cribo_graph::ModuleId> {
        // Use the central module registry for fast, reliable lookup
        if let Some(registry) = self.module_info_registry {
            let module_id = registry.get_id_by_name(module_name);
            if module_id.is_some() {
                log::debug!("Found module ID for '{module_name}' using module registry");
            } else {
                log::debug!("Module '{module_name}' not found in module registry");
            }
            module_id
        } else {
            log::warn!("No module registry available for module ID lookup");
            None
        }
    }

    /// Collect variable names referenced in an expression
    fn collect_vars_in_expr(expr: &Expr, vars: &mut FxIndexSet<String>) {
        match expr {
            Expr::Name(name) => {
                vars.insert(name.id.to_string());
            }
            Expr::Call(call) => {
                Self::collect_vars_in_expr(&call.func, vars);
                for arg in call.arguments.args.iter() {
                    Self::collect_vars_in_expr(arg, vars);
                }
                for keyword in call.arguments.keywords.iter() {
                    Self::collect_vars_in_expr(&keyword.value, vars);
                }
            }
            Expr::Attribute(attr) => {
                Self::collect_vars_in_expr(&attr.value, vars);
            }
            Expr::BinOp(binop) => {
                Self::collect_vars_in_expr(&binop.left, vars);
                Self::collect_vars_in_expr(&binop.right, vars);
            }
            Expr::UnaryOp(unaryop) => {
                Self::collect_vars_in_expr(&unaryop.operand, vars);
            }
            Expr::BoolOp(boolop) => {
                for value in boolop.values.iter() {
                    Self::collect_vars_in_expr(value, vars);
                }
            }
            Expr::Compare(compare) => {
                Self::collect_vars_in_expr(&compare.left, vars);
                for comparator in compare.comparators.iter() {
                    Self::collect_vars_in_expr(comparator, vars);
                }
            }
            Expr::List(list) => {
                for elt in list.elts.iter() {
                    Self::collect_vars_in_expr(elt, vars);
                }
            }
            Expr::Tuple(tuple) => {
                for elt in tuple.elts.iter() {
                    Self::collect_vars_in_expr(elt, vars);
                }
            }
            Expr::Dict(dict) => {
                for item in dict.items.iter() {
                    if let Some(key) = &item.key {
                        Self::collect_vars_in_expr(key, vars);
                    }
                    Self::collect_vars_in_expr(&item.value, vars);
                }
            }
            Expr::Subscript(sub) => {
                Self::collect_vars_in_expr(&sub.value, vars);
                Self::collect_vars_in_expr(&sub.slice, vars);
            }
            Expr::If(if_expr) => {
                Self::collect_vars_in_expr(&if_expr.test, vars);
                Self::collect_vars_in_expr(&if_expr.body, vars);
                Self::collect_vars_in_expr(&if_expr.orelse, vars);
            }
            _ => {}
        }
    }

    /// Transform nested functions to use module attributes for module-level variables
    pub fn transform_nested_function_for_module_vars(
        &self,
        func_def: &mut StmtFunctionDef,
        module_level_vars: &FxIndexSet<String>,
    ) {
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
        module_level_vars: &FxIndexSet<String>,
        local_vars: &FxIndexSet<String>,
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
        module_level_vars: &FxIndexSet<String>,
        local_vars: &FxIndexSet<String>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                let name_str = name_expr.id.as_str();

                // Special case: transform __name__ to module.__name__
                if name_str == "__name__" && matches!(name_expr.ctx, ExprContext::Load) {
                    // Transform __name__ -> module.__name__
                    *expr = Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: "module".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("__name__", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
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
                    *expr = Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: "module".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new(name_str, TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
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

    /// Create namespace for inlined submodule
    pub fn create_namespace_for_inlined_submodule(
        &self,
        full_module_name: &str,
        attr_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        let mut stmts = Vec::new();

        // Create a types.SimpleNamespace() for the inlined module
        stmts.push(Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: attr_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ruff_python_ast::ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        }));

        // Get the module exports for this inlined module
        let exported_symbols = self.module_exports.get(full_module_name).cloned().flatten();

        // Add all exported symbols from the inlined module to the namespace
        if let Some(exports) = exported_symbols {
            for symbol in exports {
                // For re-exported symbols, check if the original symbol is kept by tree-shaking
                let should_include = if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                {
                    // First check if this symbol is directly defined in this module
                    if kept_symbols.contains(&(full_module_name.to_string(), symbol.clone())) {
                        true
                    } else {
                        // If not, check if this is a re-exported symbol from another module
                        // For modules with __all__, we always include symbols that are re-exported
                        // even if they're not directly defined in the module
                        let module_has_all_export = self
                            .module_exports
                            .get(full_module_name)
                            .and_then(|exports| exports.as_ref())
                            .map(|exports| exports.contains(&symbol))
                            .unwrap_or(false);

                        if module_has_all_export {
                            log::debug!(
                                "Including re-exported symbol {symbol} from module \
                                 {full_module_name} (in __all__)"
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
                        "Skipping namespace assignment for {full_module_name}.{symbol} - removed \
                         by tree-shaking"
                    );
                    continue;
                }

                // Get the renamed version of this symbol
                let renamed_symbol =
                    if let Some(module_renames) = symbol_renames.get(full_module_name) {
                        module_renames
                            .get(&symbol)
                            .cloned()
                            .unwrap_or_else(|| symbol.clone())
                    } else {
                        symbol.clone()
                    };

                // Before creating the assignment, check if the renamed symbol exists after
                // tree-shaking
                if !self.renamed_symbol_exists(&renamed_symbol, symbol_renames) {
                    log::warn!(
                        "Skipping namespace assignment {attr_name}.{symbol} = {renamed_symbol} - \
                         renamed symbol doesn't exist after tree-shaking"
                    );
                    continue;
                }

                // attr_name.symbol = renamed_symbol
                log::debug!(
                    "Creating namespace assignment: {attr_name}.{symbol} = {renamed_symbol}"
                );
                stmts.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: attr_name.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new(&symbol, TextRange::default()),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: renamed_symbol.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));
            }
        } else {
            // If no explicit exports, we still need to check if this module defines symbols
            // This is a fallback for modules that don't have __all__ defined
            // For now, log a warning since we can't determine exports without module analysis
            log::warn!(
                "Inlined module '{full_module_name}' has no explicit exports (__all__). Namespace \
                 will be empty unless symbols are added elsewhere."
            );
        }

        stmts
    }

    /// Check if a renamed symbol exists after tree-shaking
    fn renamed_symbol_exists(
        &self,
        renamed_symbol: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> bool {
        // If not using tree-shaking, all symbols exist
        let Some(ref kept_symbols) = self.tree_shaking_keep_symbols else {
            return true;
        };

        // Check all modules to see if any have this renamed symbol
        for (module, renames) in symbol_renames {
            for (orig_name, renamed) in renames {
                if renamed == renamed_symbol
                    && kept_symbols.contains(&(module.clone(), orig_name.clone()))
                {
                    return true;
                }
            }
        }

        false
    }

    /// Transform AST to use lifted globals
    pub fn transform_ast_with_lifted_globals(
        &self,
        ast: &mut ModModule,
        lifted_names: &FxIndexMap<String, String>,
        _global_info: &crate::code_generator::context::ModuleGlobalInfo,
    ) {
        // For now, we'll use a simplified transformation that just renames global variables
        // This is a placeholder implementation that should be expanded based on the full
        // requirements of global lifting

        // Transform all statements in the module
        for stmt in &mut ast.body {
            self.transform_stmt_for_lifted_globals(stmt, lifted_names);
        }
    }

    /// Transform a statement to use lifted globals
    fn transform_stmt_for_lifted_globals(
        &self,
        stmt: &mut Stmt,
        lifted_names: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Transform function body
                for stmt in &mut func_def.body {
                    self.transform_stmt_for_lifted_globals(stmt, lifted_names);
                }
            }
            Stmt::Assign(assign) => {
                // Transform assignment targets and values
                for target in &mut assign.targets {
                    self.transform_expr_for_lifted_globals(target, lifted_names);
                }
                self.transform_expr_for_lifted_globals(&mut assign.value, lifted_names);
            }
            Stmt::Expr(expr_stmt) => {
                self.transform_expr_for_lifted_globals(&mut expr_stmt.value, lifted_names);
            }
            Stmt::If(if_stmt) => {
                self.transform_expr_for_lifted_globals(&mut if_stmt.test, lifted_names);
                for stmt in &mut if_stmt.body {
                    self.transform_stmt_for_lifted_globals(stmt, lifted_names);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(test_expr) = &mut clause.test {
                        self.transform_expr_for_lifted_globals(test_expr, lifted_names);
                    }
                    for stmt in &mut clause.body {
                        self.transform_stmt_for_lifted_globals(stmt, lifted_names);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                self.transform_expr_for_lifted_globals(&mut while_stmt.test, lifted_names);
                for stmt in &mut while_stmt.body {
                    self.transform_stmt_for_lifted_globals(stmt, lifted_names);
                }
            }
            Stmt::For(for_stmt) => {
                self.transform_expr_for_lifted_globals(&mut for_stmt.target, lifted_names);
                self.transform_expr_for_lifted_globals(&mut for_stmt.iter, lifted_names);
                for stmt in &mut for_stmt.body {
                    self.transform_stmt_for_lifted_globals(stmt, lifted_names);
                }
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    self.transform_expr_for_lifted_globals(value, lifted_names);
                }
            }
            Stmt::ClassDef(class_def) => {
                // Transform methods in the class
                for stmt in &mut class_def.body {
                    self.transform_stmt_for_lifted_globals(stmt, lifted_names);
                }
            }
            _ => {
                // Other statement types handled as needed
            }
        }
    }

    /// Transform an expression to use lifted globals
    fn transform_expr_for_lifted_globals(
        &self,
        expr: &mut Expr,
        lifted_names: &FxIndexMap<String, String>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                // If this name has a lifted version, use it
                if let Some(lifted_name) = lifted_names.get(name_expr.id.as_str()) {
                    name_expr.id = lifted_name.clone().into();
                }
            }
            Expr::Call(call) => {
                self.transform_expr_for_lifted_globals(&mut call.func, lifted_names);
                for arg in &mut call.arguments.args {
                    self.transform_expr_for_lifted_globals(arg, lifted_names);
                }
                for keyword in &mut call.arguments.keywords {
                    self.transform_expr_for_lifted_globals(&mut keyword.value, lifted_names);
                }
            }
            Expr::Attribute(attr) => {
                self.transform_expr_for_lifted_globals(&mut attr.value, lifted_names);
            }
            Expr::BinOp(binop) => {
                self.transform_expr_for_lifted_globals(&mut binop.left, lifted_names);
                self.transform_expr_for_lifted_globals(&mut binop.right, lifted_names);
            }
            Expr::UnaryOp(unaryop) => {
                self.transform_expr_for_lifted_globals(&mut unaryop.operand, lifted_names);
            }
            Expr::List(list) => {
                for elem in &mut list.elts {
                    self.transform_expr_for_lifted_globals(elem, lifted_names);
                }
            }
            Expr::Tuple(tuple) => {
                for elem in &mut tuple.elts {
                    self.transform_expr_for_lifted_globals(elem, lifted_names);
                }
            }
            Expr::Dict(dict) => {
                for item in &mut dict.items {
                    if let Some(key) = &mut item.key {
                        self.transform_expr_for_lifted_globals(key, lifted_names);
                    }
                    self.transform_expr_for_lifted_globals(&mut item.value, lifted_names);
                }
            }
            Expr::Subscript(sub) => {
                self.transform_expr_for_lifted_globals(&mut sub.value, lifted_names);
                self.transform_expr_for_lifted_globals(&mut sub.slice, lifted_names);
            }
            _ => {}
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
        if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols {
            let symbol_key = (module_name.to_string(), symbol_name.to_string());
            if !kept_symbols.contains(&symbol_key) {
                log::trace!(
                    "Tree shaking: removing unused symbol '{symbol_name}' from module \
                     '{module_name}'"
                );
                return false;
            }
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
    pub fn get_unique_name_with_module_suffix(&self, base_name: &str, module_name: &str) -> String {
        let module_suffix = module_name.replace('.', "_");
        format!("{base_name}_{module_suffix}")
    }

    /// Get a unique name for a symbol
    pub fn get_unique_name(
        &self,
        base_name: &str,
        existing_symbols: &FxIndexSet<String>,
    ) -> String {
        if !existing_symbols.contains(base_name) {
            return base_name.to_string();
        }

        // Find a unique name by appending numbers
        let mut counter = 2;
        loop {
            let new_name = format!("{base_name}_{counter}");
            if !existing_symbols.contains(&new_name) {
                return new_name;
            }
            counter += 1;
        }
    }

    /// Rewrite hard dependencies in a module's AST
    fn rewrite_hard_dependencies_in_module(&self, _ast: &mut ModModule, _module_name: &str) {
        // TODO: Implementation from original file
        // This handles rewriting of class base classes for circular dependencies
    }

    /// Reorder statements in a module based on symbol dependencies for circular modules
    fn reorder_statements_for_circular_module(
        &self,
        _module_name: &str,
        statements: Vec<Stmt>,
    ) -> Vec<Stmt> {
        // TODO: Implementation from original file
        // For now, return statements as-is
        statements
    }

    /// Reorder statements to ensure proper declaration order
    fn reorder_statements_for_proper_declaration_order(&self, statements: Vec<Stmt>) -> Vec<Stmt> {
        // TODO: Implementation from original file
        // For now, return statements as-is
        statements
    }

    /// Resolve import aliases in an expression
    fn resolve_import_aliases_in_expr(
        expr: &mut Expr,
        import_aliases: &FxIndexMap<String, String>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                let name_str = name_expr.id.as_str();
                if let Some(actual_name) = import_aliases.get(name_str) {
                    name_expr.id = actual_name.clone().into();
                }
            }
            Expr::Attribute(attr) => {
                Self::resolve_import_aliases_in_expr(&mut attr.value, import_aliases);
            }
            Expr::Call(call) => {
                Self::resolve_import_aliases_in_expr(&mut call.func, import_aliases);
                for arg in &mut call.arguments.args {
                    Self::resolve_import_aliases_in_expr(arg, import_aliases);
                }
                for keyword in &mut call.arguments.keywords {
                    Self::resolve_import_aliases_in_expr(&mut keyword.value, import_aliases);
                }
            }
            _ => {}
        }
    }

    /// Rewrite aliases in an expression
    fn rewrite_aliases_in_expr(
        &self,
        expr: &mut Expr,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                let name_str = name_expr.id.as_str();
                if let Some(canonical) = alias_to_canonical.get(name_str) {
                    log::debug!("Rewriting alias '{name_str}' to canonical '{canonical}'");
                    name_expr.id = canonical.clone().into();
                }
            }
            Expr::Attribute(attr) => {
                self.rewrite_aliases_in_expr(&mut attr.value, alias_to_canonical);
            }
            Expr::Call(call) => {
                self.rewrite_aliases_in_expr(&mut call.func, alias_to_canonical);
                for arg in &mut call.arguments.args {
                    self.rewrite_aliases_in_expr(arg, alias_to_canonical);
                }
                for keyword in &mut call.arguments.keywords {
                    self.rewrite_aliases_in_expr(&mut keyword.value, alias_to_canonical);
                }
            }
            _ => {}
        }
    }

    /// Resolve import aliases in a statement
    fn resolve_import_aliases_in_stmt(
        stmt: &mut Stmt,
        import_aliases: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::Expr(expr_stmt) => {
                Self::resolve_import_aliases_in_expr(&mut expr_stmt.value, import_aliases);
            }
            Stmt::Assign(assign) => {
                Self::resolve_import_aliases_in_expr(&mut assign.value, import_aliases);
                // Don't transform targets - we only resolve aliases in expressions
            }
            Stmt::Return(ret_stmt) => {
                if let Some(value) = &mut ret_stmt.value {
                    Self::resolve_import_aliases_in_expr(value, import_aliases);
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
                        self.get_unique_name(&base_name, ctx.global_symbols)
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
                    self.get_unique_name(&base_name, ctx.global_symbols)
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
                self.get_unique_name(&base_name, ctx.global_symbols)
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
        // Apply renames and resolve import aliases in class body
        for body_stmt in &mut class_def_clone.body {
            Self::resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);

            // Build a combined rename map that includes renames from all modules
            // This is needed because global variables from other modules might be renamed
            let mut combined_renames = module_renames.clone();

            // Add renames from all modules to handle cross-module global variable renames
            for (_other_module, other_renames) in ctx.module_renames.iter() {
                for (original_name, renamed_name) in other_renames {
                    // Only add if not already present (local module renames take precedence)
                    if !combined_renames.contains_key(original_name) {
                        combined_renames.insert(original_name.clone(), renamed_name.clone());
                    }
                }
            }

            self.rewrite_aliases_in_stmt(body_stmt, &combined_renames);
        }

        ctx.inlined_stmts.push(Stmt::ClassDef(class_def_clone));

        // Set the __module__ attribute to preserve the original module name
        let module_attr_stmt = Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: renamed_name.clone().into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("__module__", TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                node_index: AtomicNodeIndex::dummy(),
                value: StringLiteralValue::single(StringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: module_name.to_string().into(),
                    range: TextRange::default(),
                    flags: StringLiteralFlags::empty(),
                }),
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        });
        ctx.inlined_stmts.push(module_attr_stmt);

        // If the class was renamed, also set __name__ to preserve the original class name
        if renamed_name != class_name {
            let name_attr_stmt = Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: renamed_name.clone().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("__name__", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: StringLiteralValue::single(StringLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        value: class_name.to_string().into(),
                        range: TextRange::default(),
                        flags: StringLiteralFlags::empty(),
                    }),
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            });
            ctx.inlined_stmts.push(name_attr_stmt);
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
        if !self.should_inline_symbol(&name, module_name, ctx.module_exports_map) {
            return;
        }

        // Clone the assignment first
        let mut assign_clone = assign.clone();

        // Check if this is a self-referential assignment
        let is_self_referential = self.is_self_referential_assignment(assign);

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
                        self.get_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        name.clone()
                    }
                } else if ctx.global_symbols.contains(&name) {
                    let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                    self.get_unique_name(&base_name, ctx.global_symbols)
                } else {
                    name.clone()
                };
                module_renames.insert(name.clone(), renamed_name.clone());
                ctx.global_symbols.insert(renamed_name);
            }
            return;
        }

        // Apply existing renames to the RHS value BEFORE creating new rename for LHS
        Self::resolve_import_aliases_in_expr(&mut assign_clone.value, &ctx.import_aliases);
        self.rewrite_aliases_in_expr(&mut assign_clone.value, module_renames);

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
                        self.get_unique_name(&base_name, ctx.global_symbols)
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
                    self.get_unique_name(&base_name, ctx.global_symbols)
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
                self.get_unique_name(&base_name, ctx.global_symbols)
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
        if self.assignment_references_namespace_module(&assign_clone, ctx) {
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
                        self.get_unique_name(&base_name, ctx.global_symbols)
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
                    self.get_unique_name(&base_name, ctx.global_symbols)
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
                self.get_unique_name(&base_name, ctx.global_symbols)
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
fn collect_local_vars(stmts: &[Stmt], local_vars: &mut FxIndexSet<String>) {
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

/// Main entry point for bundling modules
pub fn bundle_modules(params: BundleParams) -> Result<ModModule> {
    let mut bundler = HybridStaticBundler::new(None);
    bundler.bundle_modules(params)
}
