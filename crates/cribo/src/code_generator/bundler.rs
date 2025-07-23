#![allow(clippy::excessive_nesting)]

use std::path::{Path, PathBuf};

use anyhow::Result;
use cow_utils::CowUtils;
use log::debug;
use ruff_python_ast::{
    Alias, Arguments, AtomicNodeIndex, Decorator, ExceptHandler, Expr, ExprAttribute, ExprCall,
    ExprContext, ExprList, ExprName, ExprStringLiteral, Identifier, Keyword, ModModule, Stmt,
    StmtAssign, StmtClassDef, StmtFunctionDef, StmtImport, StmtImportFrom, StringLiteral,
    StringLiteralFlags, StringLiteralValue, visitor::source_order::SourceOrderVisitor,
};
use ruff_text_size::TextRange;

use crate::{
    analyzers::{ImportAnalyzer, SymbolAnalyzer, namespace_analyzer::NamespaceAnalyzer},
    ast_builder::{expressions, other, statements},
    code_generator::{
        circular_deps::SymbolDependencyGraph,
        context::{
            BundleParams, HardDependency, InlineContext, ModuleTransformContext,
            ProcessGlobalsParams, SemanticContext,
        },
        import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
        module_registry::{INIT_RESULT_VAR, generate_unique_name},
        namespace_manager,
    },
    cribo_graph::CriboGraph as DependencyGraph,
    side_effects::{is_safe_stdlib_module, module_has_side_effects},
    transformation_context::TransformationContext,
    types::{FxIndexMap, FxIndexSet},
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
            Stmt::ImportFrom(import_from) => {
                if let Some(ref module) = import_from.module {
                    let module_name = module.as_str();
                    // Check if this is a __future__ import (always hoisted)
                    if module_name == "__future__" {
                        return true;
                    }
                    // Check if this is a stdlib import that we've hoisted
                    if is_safe_stdlib_module(module_name) {
                        // Check if this exact import is in our hoisted stdlib imports
                        return self.is_import_in_hoisted_stdlib(module_name);
                    }
                    // We no longer hoist third-party imports, so they should never be considered
                    // hoisted Only stdlib and __future__ imports are hoisted
                }
                false
            }
            Stmt::Import(import_stmt) => {
                // Check if any of the imported modules are hoisted (stdlib or third-party)
                import_stmt.names.iter().any(|alias| {
                    let module_name = alias.name.as_str();
                    // Check stdlib imports
                    if is_safe_stdlib_module(module_name) {
                        self.stdlib_import_statements.iter().any(|hoisted| {
                            matches!(hoisted, Stmt::Import(hoisted_import)
                                if hoisted_import.names.iter().any(|h| h.name == alias.name))
                        })
                    }
                    // We no longer hoist third-party imports
                    else {
                        false
                    }
                })
            }
            _ => false,
        }
    }

    /// Resolve a relative import with context
    pub fn resolve_relative_import_with_context(
        &self,
        import_from: &StmtImportFrom,
        current_module: &str,
        module_path: Option<&Path>,
    ) -> Option<String> {
        log::debug!(
            "Resolving relative import: level={}, module={:?}, current_module={}",
            import_from.level,
            import_from.module,
            current_module
        );

        if import_from.level > 0 {
            // This is a relative import
            let mut parts: Vec<&str> = current_module.split('.').collect();

            // Special handling for different module types
            if parts.len() == 1 && import_from.level == 1 {
                // For single-component modules with level 1 imports, we need to determine
                // if this is a root-level module or a package __init__ file

                // Check if current module is a package __init__.py file
                let is_package_init = if let Some(path) = module_path {
                    path.file_name()
                        .and_then(|f| f.to_str())
                        .map(|f| f == "__init__.py")
                        .unwrap_or(false)
                } else {
                    false
                };

                // Check if this module is the entry module and is __init__.py
                let is_entry_init = current_module
                    == self
                        .entry_path
                        .as_ref()
                        .and_then(|p| Path::new(p).file_stem())
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                    && is_package_init;

                if is_entry_init {
                    // This is the entry __init__.py - relative imports should resolve within the
                    // package but without the package prefix
                    log::debug!(
                        "Module '{current_module}' is the entry __init__.py, clearing parts for \
                         relative import"
                    );
                    parts.clear();
                } else {
                    // Check if this module is in the inlined_modules or module_registry to
                    // determine if it's a package
                    let is_package = self
                        .bundled_modules
                        .iter()
                        .any(|m| m.starts_with(&format!("{current_module}.")));

                    if is_package {
                        // This is a package __init__ file - level 1 imports stay in the package
                        log::debug!(
                            "Module '{current_module}' is a package, keeping parts for relative \
                             import"
                        );
                        // Keep parts as is
                    } else {
                        // This is a root-level module - level 1 imports are siblings
                        log::debug!(
                            "Module '{current_module}' is root-level, clearing parts for relative \
                             import"
                        );
                        parts.clear();
                    }
                }
            } else {
                // For modules with multiple components (e.g., "greetings.greeting")
                // Special handling: if this module represents a package __init__.py file,
                // the first level doesn't remove anything (stays in the package)
                // Subsequent levels go up the hierarchy

                // Check if current module is a package __init__.py file
                let is_package_init = if let Some(path) = module_path {
                    path.file_name()
                        .and_then(|f| f.to_str())
                        .map(|f| f == "__init__.py")
                        .unwrap_or(false)
                } else {
                    // Fallback: check if module has submodules
                    self.bundled_modules
                        .iter()
                        .any(|m| m.starts_with(&format!("{current_module}.")))
                };

                let levels_to_remove = if is_package_init {
                    // For package __init__.py files, the first dot stays in the package
                    // So we remove (level - 1) parts
                    import_from.level.saturating_sub(1)
                } else {
                    // For regular modules, remove 'level' parts
                    import_from.level
                };

                log::debug!(
                    "Relative import resolution: current_module={}, is_package_init={}, level={}, \
                     levels_to_remove={}, parts={:?}",
                    current_module,
                    is_package_init,
                    import_from.level,
                    levels_to_remove,
                    parts
                );

                for _ in 0..levels_to_remove {
                    if parts.is_empty() {
                        log::debug!("Invalid relative import - ran out of parent levels");
                        return None; // Invalid relative import
                    }
                    parts.pop();
                }
            }

            // Add the module name if specified
            if let Some(ref module) = import_from.module {
                parts.push(module.as_str());
            }

            let resolved = parts.join(".");

            // Handle the case where relative import resolves to empty or just the package itself
            // This happens with "from . import something" in a package __init__.py
            if resolved.is_empty() {
                // For "from . import X" in a package, the resolved module is the current package
                // We need to check if we're in a package __init__.py
                if import_from.level == 1 && import_from.module.is_none() {
                    // This is "from . import X" - we need to determine the parent package
                    // For a module like "requests.utils", the parent is "requests"
                    // For a module like "__init__", it's the current directory
                    if current_module.contains('.') {
                        // Module has a parent package - extract it
                        let parent_parts: Vec<&str> = current_module.split('.').collect();
                        let parent = parent_parts[..parent_parts.len() - 1].join(".");
                        log::debug!(
                            "Relative import 'from . import' in module '{current_module}' - \
                             returning parent package '{parent}'"
                        );
                        return Some(parent);
                    } else if current_module == "__init__" {
                        // This is a package __init__.py doing "from . import X"
                        // The package name should be derived from the directory
                        log::debug!(
                            "Relative import 'from . import' in __init__ module - this case needs \
                             special handling"
                        );
                        // For now, we'll return None and let it be handled elsewhere
                        return None;
                    } else {
                        // Single-level module doing "from . import X" - this is importing from the
                        // same directory We need to return empty string to
                        // indicate current directory
                        log::debug!(
                            "Relative import 'from . import' in root-level module \
                             '{current_module}' - returning empty for current directory"
                        );
                        return Some(String::new());
                    }
                }
                log::debug!("Invalid relative import - resolved to empty module");
                return None;
            }

            // Check for potential circular import
            if resolved == current_module {
                log::warn!("Potential circular import detected: {current_module} importing itself");
            }

            log::debug!("Resolved relative import to: {resolved}");
            Some(resolved)
        } else {
            // Not a relative import
            let resolved = import_from.module.as_ref().map(|m| m.as_str().to_string());
            log::debug!("Not a relative import, resolved to: {resolved:?}");
            resolved
        }
    }

    /// Rewrite import with renames
    pub fn rewrite_import_with_renames(
        &self,
        import_stmt: StmtImport,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        // Check each import individually
        let mut result_stmts = Vec::new();
        let mut handled_all = true;

        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();

            // Check if this is a dotted import (e.g., greetings.greeting)
            if module_name.contains('.') {
                // Handle dotted imports specially
                let parts: Vec<&str> = module_name.split('.').collect();

                // Check if the full module is bundled
                if self.bundled_modules.contains(module_name) {
                    if self.module_registry.contains_key(module_name) {
                        // Create all parent namespaces if needed (e.g., for a.b.c.d, create a, a.b,
                        // a.b.c)
                        self.create_parent_namespaces(&parts, &mut result_stmts);

                        // Initialize the module at import time
                        result_stmts
                            .extend(self.create_module_initialization_for_import(module_name));

                        let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                        // If there's no alias, we need to handle the dotted name specially
                        if alias.asname.is_none() && module_name.contains('.') {
                            // Create assignments for each level of nesting
                            self.create_dotted_assignments(&parts, &mut result_stmts);
                        } else {
                            // For aliased imports or non-dotted imports, just assign to the target
                            // Skip self-assignments - the module is already initialized
                            if target_name.as_str() != module_name {
                                result_stmts.push(self.create_module_reference_assignment(
                                    target_name.as_str(),
                                    module_name,
                                ));
                            }
                        }
                    } else {
                        // Module was inlined - create a namespace object
                        let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                        // For dotted imports, we need to create the parent namespaces
                        if alias.asname.is_none() && module_name.contains('.') {
                            // For non-aliased dotted imports like "import a.b.c"
                            // Create all parent namespace objects AND the leaf namespace
                            self.create_all_namespace_objects(&parts, &mut result_stmts);

                            // Populate ALL namespace levels with their symbols, not just the leaf
                            // For "import greetings.greeting", populate both "greetings" and
                            // "greetings.greeting"
                            for i in 1..=parts.len() {
                                let partial_module = parts[..i].join(".");
                                // Only populate if this module was actually bundled and has exports
                                if self.bundled_modules.contains(&partial_module) {
                                    self.populate_namespace_with_module_symbols_with_renames(
                                        &partial_module,
                                        &partial_module,
                                        &mut result_stmts,
                                        symbol_renames,
                                    );
                                }
                            }
                        } else {
                            // For simple imports or aliased imports, create namespace object with
                            // the module's exports

                            // Check if namespace already exists
                            if !self.created_namespaces.contains(target_name.as_str()) {
                                let namespace_stmt = self.create_namespace_object_for_module(
                                    target_name.as_str(),
                                    module_name,
                                );
                                result_stmts.push(namespace_stmt);
                            } else {
                                log::debug!(
                                    "Skipping namespace creation for '{}' - already created \
                                     globally",
                                    target_name.as_str()
                                );
                            }

                            // Always populate the namespace with symbols
                            self.populate_namespace_with_module_symbols_with_renames(
                                target_name.as_str(),
                                module_name,
                                &mut result_stmts,
                                symbol_renames,
                            );
                        }
                    }
                } else {
                    handled_all = false;
                    continue;
                }
            } else {
                // Non-dotted import - handle as before
                if !self.bundled_modules.contains(module_name) {
                    handled_all = false;
                    continue;
                }

                if self.module_registry.contains_key(module_name) {
                    // Module uses wrapper approach - need to initialize it now
                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                    // First, ensure the module is initialized
                    result_stmts.extend(self.create_module_initialization_for_import(module_name));

                    // Then create assignment if needed (skip self-assignments)
                    if target_name.as_str() != module_name {
                        result_stmts.push(
                            self.create_module_reference_assignment(
                                target_name.as_str(),
                                module_name,
                            ),
                        );
                    }
                } else {
                    // Module was inlined - create a namespace object
                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                    // Create namespace object with the module's exports
                    // Check if namespace already exists
                    if !self.created_namespaces.contains(target_name.as_str()) {
                        let namespace_stmt = self
                            .create_namespace_object_for_module(target_name.as_str(), module_name);
                        result_stmts.push(namespace_stmt);
                    } else {
                        log::debug!(
                            "Skipping namespace creation for '{}' - already created globally",
                            target_name.as_str()
                        );
                    }

                    // Always populate the namespace with symbols
                    self.populate_namespace_with_module_symbols_with_renames(
                        target_name.as_str(),
                        module_name,
                        &mut result_stmts,
                        symbol_renames,
                    );
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

    /// Resolve relative import
    pub fn resolve_relative_import(
        &self,
        import_from: &StmtImportFrom,
        current_module: &str,
    ) -> Option<String> {
        self.resolve_relative_import_with_context(import_from, current_module, None)
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
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        _module_context: Option<&str>,
    ) -> Vec<Stmt> {
        let mut result_stmts = Vec::new();

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Check if this is likely a re-export from a package __init__.py
            let is_package_reexport = self.is_package_init_reexport(module_name, imported_name);

            let renamed_symbol = if is_package_reexport {
                // For package re-exports, use the original symbol name
                // This handles cases like greetings/__init__.py re-exporting from greetings.english
                log::debug!(
                    "Using original name '{imported_name}' for symbol imported from package \
                     '{module_name}'"
                );
                imported_name.to_string()
            } else {
                // Not a re-export, check normal renames
                if let Some(module_renames) = symbol_renames.get(module_name) {
                    module_renames
                        .get(imported_name)
                        .cloned()
                        .unwrap_or_else(|| {
                            // If no rename found, use the default pattern
                            let module_suffix = module_name.cow_replace('.', "_").into_owned();
                            format!("{imported_name}_{module_suffix}")
                        })
                } else {
                    // If no rename map, use the default pattern
                    let module_suffix = module_name.cow_replace('.', "_").into_owned();
                    format!("{imported_name}_{module_suffix}")
                }
            };

            // Only create assignment if the names are different
            if local_name != renamed_symbol {
                result_stmts.push(statements::simple_assign(
                    local_name,
                    expressions::name(&renamed_symbol, ExprContext::Load),
                ));
            }
        }

        result_stmts
    }

    /// Check if a symbol is likely a re-export from a package __init__.py
    fn is_package_init_reexport(&self, module_name: &str, _symbol_name: &str) -> bool {
        // Special handling for package __init__.py files
        // If we're importing from "greetings" and there's a "greetings.X" module
        // that could be the source of the symbol

        // For now, check if this looks like a package (no dots) and if there are
        // any inlined submodules
        if !module_name.contains('.') {
            // Check if any inlined module starts with module_name.
            for inlined in &self.inlined_modules {
                if inlined.starts_with(&format!("{module_name}.")) {
                    log::debug!(
                        "Module '{module_name}' appears to be a package with inlined submodule \
                         '{inlined}'"
                    );
                    // For the specific case of greetings/__init__.py importing from
                    // greetings.english, we assume the symbol should use its
                    // original name
                    return true;
                }
            }
        }
        false
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
            #[allow(clippy::too_many_arguments)]
            crate::code_generator::module_registry::create_assignments_for_inlined_imports(
                import_from,
                &module_name,
                symbol_renames,
                &self.module_registry,
                &self.inlined_modules,
                &self.bundled_modules,
                |local_name, full_module_path| {
                    self.create_namespace_with_name(local_name, full_module_path)
                },
            )
        }
    }

    /// Transform bundled import from statement with context
    fn transform_bundled_import_from_multiple_with_context(
        &self,
        import_from: StmtImportFrom,
        module_name: &str,
        inside_wrapper_init: bool,
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

                // When inside a wrapper init, we need to initialize modules we're importing from
                if inside_wrapper_init {
                    // First, ensure the parent module is initialized if it's a wrapper module
                    if self.module_registry.contains_key(module_name)
                        && !locally_initialized.contains(module_name)
                    {
                        assignments
                            .extend(self.create_module_initialization_for_import(module_name));
                        locally_initialized.insert(module_name.to_string());
                    }
                    // Then ensure the submodule is initialized if it's a wrapper module
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
                                let attr_path = self.extract_attribute_path(attr);
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
                } else {
                    // Not inside wrapper init - normal lazy initialization
                    if self.module_registry.contains_key(module_name)
                        && !locally_initialized.contains(module_name)
                    {
                        // Initialize parent module if needed
                        assignments
                            .extend(self.create_module_initialization_for_import(module_name));
                        locally_initialized.insert(module_name.to_string());
                    }
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
                                let attr_path = self.extract_attribute_path(attr);
                                attr_path == full_module_path
                            } else {
                                false
                            }
                        });

                        if !already_initialized {
                            // Initialize submodule if needed
                            assignments.extend(
                                self.create_module_initialization_for_import(&full_module_path),
                            );
                        }
                        locally_initialized.insert(full_module_path.clone());
                        initialized_modules.insert(full_module_path.clone());
                    }
                }

                // Build the direct namespace reference
                let namespace_expr = if self.inlined_modules.contains(&full_module_path) {
                    // For inlined modules, use the temporary variable directly
                    // Use direct module name for inlined modules
                    let module_var_name = full_module_path.clone();
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: module_var_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
                } else if full_module_path.contains('.') {
                    // For nested modules like models.user, create models.user expression
                    let parts: Vec<&str> = full_module_path.split('.').collect();
                    expressions::dotted_name(&parts, ExprContext::Load)
                } else {
                    // Top-level module
                    expressions::name(&full_module_path, ExprContext::Load)
                };

                assignments.push(statements::simple_assign(
                    target_name.as_str(),
                    namespace_expr,
                ));
            } else {
                // Regular attribute import
                // Ensure the module is initialized first if it's a wrapper module
                if self.module_registry.contains_key(module_name)
                    && !locally_initialized.contains(module_name)
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
                                    let attr_path = self.extract_attribute_path(attr);
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
                    let mut expr = Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
                    for part in &parts[1..] {
                        expr = Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(expr),
                            attr: Identifier::new(*part, TextRange::default()),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        });
                    }
                    expr
                } else {
                    // Top-level module
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: module_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
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

    /// Create a namespace object with __name__ attribute
    fn create_namespace_with_name(&self, var_name: &str, module_path: &str) -> Vec<Stmt> {
        // Create: var_name = types.SimpleNamespace()
        let types_simple_namespace_call = expressions::call(
            expressions::dotted_name(&["types", "SimpleNamespace"], ExprContext::Load),
            vec![],
            vec![],
        );
        let mut statements = vec![statements::simple_assign(
            var_name,
            types_simple_namespace_call,
        )];

        // Set the __name__ attribute
        let target = expressions::attribute(
            expressions::name(var_name, ExprContext::Load),
            "__name__",
            ExprContext::Store,
        );
        let value = expressions::string_literal(module_path);
        statements.push(statements::assign(vec![target], value));

        statements
    }

    /// Transform imports from namespace packages
    fn transform_namespace_package_imports(
        &self,
        import_from: StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        let mut result_stmts = Vec::new();

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            let full_module_path = format!("{module_name}.{imported_name}");

            if self.bundled_modules.contains(&full_module_path) {
                if self.module_registry.contains_key(&full_module_path) {
                    // Wrapper module - create direct module reference
                    result_stmts.push(statements::simple_assign(
                        local_name,
                        expressions::name(&full_module_path, ExprContext::Load),
                    ));
                } else {
                    // Inlined module - create a namespace object for it
                    log::debug!(
                        "Submodule '{imported_name}' from namespace package '{module_name}' was \
                         inlined, creating namespace"
                    );

                    // For namespace hybrid modules, we need to create the namespace object
                    // The inlined module's symbols are already renamed with module prefix
                    // e.g., message -> message_greetings_greeting
                    let _inlined_key = full_module_path.cow_replace('.', "_").into_owned();

                    // Create a SimpleNamespace object manually with all the inlined symbols
                    // Since the module was inlined, we need to map the original names to the
                    // renamed ones
                    result_stmts.push(statements::simple_assign(
                        local_name,
                        expressions::call(
                            expressions::dotted_name(
                                &["types", "SimpleNamespace"],
                                ExprContext::Load,
                            ),
                            vec![],
                            vec![],
                        ),
                    ));

                    // Add all the renamed symbols as attributes to the namespace
                    // Get the symbol renames for this module if available
                    if let Some(module_renames) = symbol_renames.get(&full_module_path) {
                        let module_suffix = full_module_path.cow_replace('.', "_");
                        for (original_name, renamed_name) in module_renames {
                            // Check if this is an identity mapping (no semantic rename)
                            let actual_renamed_name = if renamed_name == original_name {
                                // No semantic rename, apply module suffix pattern

                                self.get_unique_name_with_module_suffix(
                                    original_name,
                                    &module_suffix,
                                )
                            } else {
                                // Use the semantic rename
                                renamed_name.clone()
                            };

                            // base.original_name = actual_renamed_name
                            result_stmts.push(statements::assign(
                                vec![expressions::attribute(
                                    expressions::name(local_name, ExprContext::Load),
                                    original_name,
                                    ExprContext::Store,
                                )],
                                expressions::name(&actual_renamed_name, ExprContext::Load),
                            ));
                        }
                    } else {
                        // Fallback: try to guess the renamed symbols based on module suffix
                        log::warn!(
                            "No symbol renames found for inlined module '{full_module_path}', \
                             namespace will be empty"
                        );
                    }
                }
            } else {
                // Not a bundled submodule, keep as attribute access
                // This might be importing a symbol from the namespace package's __init__.py
                // But since we're here, the namespace package has no __init__.py
                log::warn!(
                    "Import '{imported_name}' from namespace package '{module_name}' is not a \
                     bundled module"
                );
            }
        }

        if result_stmts.is_empty() {
            // If we didn't transform anything, return the original
            vec![Stmt::ImportFrom(import_from)]
        } else {
            result_stmts
        }
    }

    /// Check if a string is a valid Python identifier
    fn is_valid_python_identifier(name: &str) -> bool {
        // Use ruff's identifier validation which handles Unicode and keywords
        ruff_python_stdlib::identifiers::is_identifier(name)
    }

    /// Extract __all__ exports from a module
    /// Returns:
    /// - has_explicit_all: true if __all__ is explicitly defined
    /// - exports: Some(vec) if there are exports, None if no exports
    fn extract_all_exports(&self, ast: &ModModule) -> (bool, Option<Vec<String>>) {
        // First, look for explicit __all__
        for stmt in &ast.body {
            let Stmt::Assign(assign) = stmt else {
                continue;
            };

            // Look for __all__ = [...]
            if assign.targets.len() != 1 {
                continue;
            }

            let Expr::Name(name) = &assign.targets[0] else {
                continue;
            };

            if name.id.as_str() == "__all__" {
                return (true, self.extract_string_list_from_expr(&assign.value));
            }
        }

        // If no __all__, collect all top-level symbols (including private ones for module state)
        let mut symbols = Vec::new();
        for stmt in &ast.body {
            match stmt {
                Stmt::FunctionDef(func) => {
                    symbols.push(func.name.to_string());
                }
                Stmt::ClassDef(class) => {
                    symbols.push(class.name.to_string());
                }
                Stmt::Assign(assign) => {
                    // Include ALL variable assignments (including private ones starting with _)
                    // This ensures module state variables like _config, _logger are available
                    for target in &assign.targets {
                        if let Expr::Name(name) = target
                            && name.id.as_str() != "__all__"
                        {
                            symbols.push(name.id.to_string());
                        }
                    }
                }
                Stmt::AnnAssign(ann_assign) => {
                    // Include ALL annotated assignments (including private ones)
                    if let Expr::Name(name) = ann_assign.target.as_ref() {
                        symbols.push(name.id.to_string());
                    }
                }
                _ => {}
            }
        }

        if symbols.is_empty() {
            (false, None)
        } else {
            // Sort symbols for deterministic output
            symbols.sort();
            (false, Some(symbols))
        }
    }

    fn extract_string_list_from_expr(&self, expr: &Expr) -> Option<Vec<String>> {
        match expr {
            Expr::List(list_expr) => {
                let mut exports = Vec::new();
                for element in &list_expr.elts {
                    if let Expr::StringLiteral(string_lit) = element {
                        let string_value = string_lit.value.to_str();
                        exports.push(string_value.to_string());
                    }
                }
                Some(exports)
            }
            Expr::Tuple(tuple_expr) => {
                let mut exports = Vec::new();
                for element in &tuple_expr.elts {
                    if let Expr::StringLiteral(string_lit) = element {
                        let string_value = string_lit.value.to_str();
                        exports.push(string_value.to_string());
                    }
                }
                Some(exports)
            }
            _ => None, // Other expressions like computed lists are not supported
        }
    }

    /// Add a regular stdlib import (e.g., "sys", "types")
    /// This creates an import statement and adds it to the tracked imports
    fn add_stdlib_import(&mut self, module_name: &str) {
        // Check if we already have this import to avoid duplicates
        let already_imported = self.stdlib_import_statements.iter().any(|stmt| {
            if let Stmt::Import(import_stmt) = stmt {
                import_stmt
                    .names
                    .iter()
                    .any(|alias| alias.name.as_str() == module_name)
            } else {
                false
            }
        });

        if already_imported {
            log::debug!("Stdlib import '{module_name}' already exists, skipping");
            return;
        }

        let import_stmt = statements::import(vec![other::alias(module_name, None)]);
        self.stdlib_import_statements.push(import_stmt);
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

    /// Create namespace statements for required namespaces
    fn create_namespace_statements(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();

        // Sort namespaces for deterministic output
        let mut sorted_namespaces: Vec<String> = self.required_namespaces.iter().cloned().collect();
        sorted_namespaces.sort();

        for namespace in sorted_namespaces {
            debug!("Creating namespace statement for: {namespace}");

            // Use ensure_namespace_exists to handle both simple and dotted namespaces
            let namespace_stmts = self.ensure_namespace_exists(&namespace);
            statements.extend(namespace_stmts);
        }

        statements
    }

    /// Create namespace attribute assignment
    fn create_namespace_attribute(&mut self, parent: &str, child: &str) -> Stmt {
        // Create: parent.child = types.SimpleNamespace()
        let mut stmt = statements::assign(
            vec![expressions::attribute(
                expressions::name(parent, ExprContext::Load),
                child,
                ExprContext::Store,
            )],
            expressions::call(
                expressions::dotted_name(&["types", "SimpleNamespace"], ExprContext::Load),
                vec![],
                vec![],
            ),
        );

        // Update the node index for tracking
        if let Stmt::Assign(assign) = &mut stmt {
            assign.node_index = self
                .create_transformed_node(format!("Create namespace attribute {parent}.{child}"));
        }

        stmt
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
                        if is_safe_stdlib_module(module_str) {
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
                        if is_safe_stdlib_module(module_name) && alias.asname.is_none() {
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

    /// Process wrapper module globals (matching original implementation)
    fn process_wrapper_module_globals(
        &self,
        params: &ProcessGlobalsParams,
        module_globals: &mut FxIndexMap<String, crate::semantic_bundler::ModuleGlobalInfo>,
        all_lifted_declarations: &mut Vec<Stmt>,
    ) {
        // Get module ID from graph
        let module = match params
            .semantic_ctx
            .graph
            .get_module_by_name(params.module_name)
        {
            Some(m) => m,
            None => return,
        };

        let module_id = module.module_id;
        let global_info = params.semantic_ctx.semantic_bundler.analyze_module_globals(
            module_id,
            params.ast,
            params.module_name,
        );

        // Create GlobalsLifter and collect declarations
        if !global_info.global_declarations.is_empty() {
            let globals_lifter = crate::code_generator::globals::GlobalsLifter::new(&global_info);
            all_lifted_declarations.extend(globals_lifter.get_lifted_declarations());
        }

        module_globals.insert(params.module_name.to_string(), global_info);
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
            self.reorder_statements_for_circular_module(module_name, ast.body)
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
        module_name: &str,
        module_renames: &FxIndexMap<String, String>,
    ) -> Stmt {
        // Check if this module has forward references that would cause NameError
        // This happens when the module uses symbols from other modules that haven't been defined
        // yet
        let has_forward_references =
            self.check_module_has_forward_references(module_name, module_renames);

        if has_forward_references {
            log::debug!("Module '{module_name}' has forward references, creating empty namespace");
            // Create the namespace variable name
            let namespace_var = module_name.cow_replace('.', "_").into_owned();

            // Create empty namespace = types.SimpleNamespace() to avoid forward reference errors
            return statements::simple_assign(
                &namespace_var,
                expressions::call(
                    expressions::dotted_name(&["types", "SimpleNamespace"], ExprContext::Load),
                    vec![],
                    vec![],
                ),
            );
        }
        // Create a types.SimpleNamespace with all the module's symbols
        let mut keywords = Vec::new();
        let mut seen_args = FxIndexSet::default();

        // Add all renamed symbols as keyword arguments, avoiding duplicates
        for (original_name, renamed_name) in module_renames {
            // Skip if we've already added this argument name
            if seen_args.contains(original_name) {
                log::debug!(
                    "Skipping duplicate namespace argument '{original_name}' for module \
                     '{module_name}'"
                );
                continue;
            }

            // Check if this symbol survived tree-shaking
            if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                && !kept_symbols.contains(&(module_name.to_string(), original_name.clone()))
            {
                log::debug!(
                    "Skipping tree-shaken symbol '{original_name}' from namespace for module \
                     '{module_name}'"
                );
                continue;
            }

            seen_args.insert(original_name.clone());

            keywords.push(Keyword {
                node_index: AtomicNodeIndex::dummy(),
                arg: Some(Identifier::new(original_name, TextRange::default())),
                value: Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: renamed_name.clone().into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                }),
                range: TextRange::default(),
            });
        }

        // Also check if module has module-level variables that weren't renamed
        if let Some(exports) = self.module_exports.get(module_name)
            && let Some(export_list) = exports
        {
            for export in export_list {
                // Check if this export was already added as a renamed symbol
                if !module_renames.contains_key(export) && !seen_args.contains(export) {
                    // Check if this symbol survived tree-shaking
                    if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                        && !kept_symbols.contains(&(module_name.to_string(), export.clone()))
                    {
                        log::debug!(
                            "Skipping tree-shaken export '{export}' from namespace for module \
                             '{module_name}'"
                        );
                        continue;
                    }

                    // This export wasn't renamed and wasn't already added, add it directly
                    seen_args.insert(export.clone());
                    keywords.push(Keyword {
                        node_index: AtomicNodeIndex::dummy(),
                        arg: Some(Identifier::new(export, TextRange::default())),
                        value: Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: export.clone().into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        }),
                        range: TextRange::default(),
                    });
                }
            }
        }

        // Create the namespace variable name
        let namespace_var = module_name.cow_replace('.', "_").into_owned();

        // Create namespace = types.SimpleNamespace(**kwargs) assignment
        statements::assign(
            vec![expressions::name(&namespace_var, ExprContext::Store)],
            expressions::call(
                expressions::dotted_name(&["types", "SimpleNamespace"], ExprContext::Load),
                vec![],
                keywords,
            ),
        )
    }

    /// Sort wrapped modules by dependencies
    fn sort_wrapped_modules_by_dependencies(
        &self,
        wrapped_modules: &[String],
        _all_modules: &[(String, PathBuf, Vec<String>)],
        graph: &DependencyGraph,
    ) -> Vec<String> {
        use crate::analyzers::dependency_analyzer::DependencyAnalyzer;

        // Convert wrapped_modules slice to Vec for DependencyAnalyzer
        let module_names: Vec<String> = wrapped_modules.to_vec();

        // Use DependencyAnalyzer to sort
        DependencyAnalyzer::sort_wrapped_modules_by_dependencies(module_names, graph)
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

    /// Deduplicate deferred import statements
    /// This prevents duplicate init calls and symbol assignments
    fn deduplicate_deferred_imports_with_existing(
        &self,
        imports: Vec<Stmt>,
        existing_body: &[Stmt],
    ) -> Vec<Stmt> {
        let mut seen_init_calls = FxIndexSet::default();
        let mut seen_assignments = FxIndexSet::default();
        let mut result = Vec::new();

        // First, collect all existing assignments from the body
        for stmt in existing_body {
            if let Stmt::Assign(assign) = stmt
                && assign.targets.len() == 1
            {
                // Handle attribute assignments like schemas.user = ...
                if let Expr::Attribute(target_attr) = &assign.targets[0] {
                    let target_path = self.extract_attribute_path(target_attr);

                    // Handle init function calls
                    if let Expr::Call(call) = &assign.value.as_ref()
                        && let Expr::Name(name) = &call.func.as_ref()
                    {
                        let func_name = name.id.as_str();
                        if crate::code_generator::module_registry::is_init_function(func_name) {
                            // Use just the target path as the key for module init assignments
                            let key = target_path.clone();
                            log::debug!(
                                "Found existing module init assignment: {key} = {func_name}"
                            );
                            seen_assignments.insert(key);
                        }
                    }
                }
                // Handle simple name assignments
                else if let Expr::Name(target) = &assign.targets[0] {
                    let target_str = target.id.as_str();

                    // Handle simple name assignments
                    if let Expr::Name(value) = &assign.value.as_ref() {
                        let key = format!("{} = {}", target_str, value.id.as_str());
                        seen_assignments.insert(key);
                    }
                    // Handle attribute assignments like User = services.auth.manager.User
                    else if let Expr::Attribute(attr) = &assign.value.as_ref() {
                        let attr_path = self.extract_attribute_path(attr);
                        let key = format!("{target_str} = {attr_path}");
                        seen_assignments.insert(key);
                    }
                }
            }
        }

        log::debug!(
            "Found {} existing assignments in body",
            seen_assignments.len()
        );
        log::debug!("Deduplicating {} deferred imports", imports.len());

        // Now process the deferred imports
        for (idx, stmt) in imports.into_iter().enumerate() {
            log::debug!("Processing deferred import {idx}: {stmt:?}");
            match &stmt {
                // Check for init function calls
                Stmt::Expr(expr_stmt) => {
                    if let Expr::Call(call) = &expr_stmt.value.as_ref() {
                        if let Expr::Name(name) = &call.func.as_ref() {
                            let func_name = name.id.as_str();
                            if crate::code_generator::module_registry::is_init_function(func_name) {
                                if seen_init_calls.insert(func_name.to_string()) {
                                    result.push(stmt);
                                } else {
                                    log::debug!("Skipping duplicate init call: {func_name}");
                                }
                            } else {
                                result.push(stmt);
                            }
                        } else {
                            result.push(stmt);
                        }
                    } else {
                        result.push(stmt);
                    }
                }
                // Check for symbol assignments
                Stmt::Assign(assign) => {
                    // First check if this is an attribute assignment with an init function call
                    // like: schemas.user = <cribo_init_prefix>__cribo_f275a8_schemas_user()
                    if assign.targets.len() == 1
                        && let Expr::Attribute(target_attr) = &assign.targets[0]
                    {
                        let target_path = self.extract_attribute_path(target_attr);

                        // Check if value is an init function call
                        if let Expr::Call(call) = &assign.value.as_ref()
                            && let Expr::Name(name) = &call.func.as_ref()
                        {
                            let func_name = name.id.as_str();
                            if crate::code_generator::module_registry::is_init_function(func_name) {
                                // For module init assignments, just check the target path
                                // since the same module should only be initialized once
                                let key = target_path.clone();
                                log::debug!(
                                    "Checking deferred module init assignment: {key} = {func_name}"
                                );
                                if seen_assignments.contains(&key) {
                                    log::debug!(
                                        "Skipping duplicate module init assignment: {key} = \
                                         {func_name}"
                                    );
                                    continue; // Skip this statement entirely
                                } else {
                                    log::debug!(
                                        "Adding new module init assignment: {key} = {func_name}"
                                    );
                                    seen_assignments.insert(key);
                                    result.push(stmt);
                                    continue;
                                }
                            }
                        }
                    }

                    // Check for simple assignments like: Logger = Logger_4
                    if assign.targets.len() == 1 {
                        if let Expr::Name(target) = &assign.targets[0] {
                            if let Expr::Name(value) = &assign.value.as_ref() {
                                // This is a simple name assignment
                                let target_str = target.id.as_str();
                                let value_str = value.id.as_str();
                                let key = format!("{target_str} = {value_str}");

                                // Check for self-assignment
                                if target_str == value_str {
                                    log::warn!("Found self-assignment in deferred imports: {key}");
                                    // Skip self-assignments entirely
                                    log::debug!("Skipping self-assignment: {key}");
                                } else if seen_assignments.insert(key.clone()) {
                                    log::debug!("First occurrence of simple assignment: {key}");
                                    result.push(stmt);
                                } else {
                                    log::debug!("Skipping duplicate simple assignment: {key}");
                                }
                            } else {
                                // Not a simple name assignment, check for duplicates
                                // Handle attribute assignments like User =
                                // services.auth.manager.User
                                let target_str = target.id.as_str();

                                // For attribute assignments, extract the actual attribute path
                                let key = if let Expr::Attribute(attr) = &assign.value.as_ref() {
                                    // Extract the full attribute path (e.g.,
                                    // services.auth.manager.User)
                                    let attr_path = self.extract_attribute_path(attr);
                                    format!("{target_str} = {attr_path}")
                                } else {
                                    // Fallback to debug format for other types
                                    let value_str = format!("{:?}", assign.value);
                                    format!("{target_str} = {value_str}")
                                };

                                if seen_assignments.insert(key.clone()) {
                                    log::debug!("First occurrence of attribute assignment: {key}");
                                    result.push(stmt);
                                } else {
                                    log::debug!("Skipping duplicate attribute assignment: {key}");
                                }
                            }
                        } else {
                            // Target is not a simple name, include it
                            result.push(stmt);
                        }
                    } else {
                        // Multiple targets, include it
                        result.push(stmt);
                    }
                }
                _ => result.push(stmt),
            }
        }

        result
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
            if !is_safe_stdlib_module(module_name)
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
            if !is_safe_stdlib_module(module_name)
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

    /// Check if module has forward references that would cause NameError
    fn check_module_has_forward_references(
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
                    self.rewrite_aliases_in_stmt(stmt, entry_module_renames);

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

        // Store tree shaking decisions if provided
        if let Some(shaker) = params.tree_shaker {
            // Extract all kept symbols from the tree shaker
            let mut kept_symbols = indexmap::IndexSet::new();
            for (module_name, _, _, _) in params.modules {
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
            let has_side_effects = module_has_side_effects(ast);

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
        self.required_namespaces =
            NamespaceAnalyzer::identify_required_namespaces(&all_modules_for_namespace_detection);

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
        if needs_types_for_entry_imports {
            log::debug!("Adding types import for namespace objects in entry module");
            self.add_stdlib_import("types");
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
            self.add_stdlib_import("functools");
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

            for (module_name, ast, _, _) in &all_modules {
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
                                    self.resolve_relative_import(import_from, module_name)
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
                        self.resolve_relative_import_with_context(
                            import_from,
                            module_name,
                            Some(module_path),
                        )
                    } else {
                        // Absolute import
                        import_from.module.as_ref().map(|m| m.as_str().to_string())
                    };

                    if let Some(ref resolved) = resolved_module {
                        // Check if this is a wrapper module
                        if self.module_registry.contains_key(resolved) {
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
                            self.add_stdlib_import(module_name);
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
                    self.process_wrapper_module_globals(
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
                        // But if this wrapper module is a source of hard dependencies, generate
                        // assignments now
                        let is_hard_dep_source = self
                            .hard_dependencies
                            .iter()
                            .any(|dep| dep.source_module == *module_name);
                        if is_hard_dep_source {
                            let hard_deps = self.hard_dependencies.clone();

                            // Generate a call to initialize this module immediately
                            let init_func_name = self.init_functions[&synthetic_name].clone();
                            let init_call = statements::simple_assign(
                                INIT_RESULT_VAR,
                                expressions::call(
                                    expressions::name(&init_func_name, ExprContext::Load),
                                    vec![],
                                    vec![],
                                ),
                            );
                            final_body.push(init_call);

                            // Generate the merge
                            self.generate_merge_module_attributes(
                                &mut final_body,
                                module_name,
                                INIT_RESULT_VAR,
                            );

                            // Now generate assignments for hard dependencies from this module
                            for dep in &hard_deps {
                                if dep.source_module == *module_name {
                                    // Use the same logic as hard dependency rewriting in inlined
                                    // modules This must match
                                    // the logic in rewrite_hard_dependencies_in_inlined_module
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
                                        "Generated early assignment: {} = {}.{}",
                                        target_name,
                                        module_name,
                                        dep.imported_attr
                                    );
                                }
                            }
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

            // Clone hard dependencies to avoid borrowing issues
            let hard_deps = self.hard_dependencies.clone();

            // Group hard dependencies by source module
            let mut deps_by_source: FxIndexMap<String, Vec<&HardDependency>> =
                FxIndexMap::default();
            for dep in &hard_deps {
                deps_by_source
                    .entry(dep.source_module.clone())
                    .or_default()
                    .push(dep);
            }

            // Generate hoisted imports
            for (source_module, deps) in deps_by_source {
                // Check if we need to import the whole module or specific attributes
                let first_dep = deps.first().expect("hard_deps should not be empty");

                if source_module == "http.cookiejar" && first_dep.imported_attr == "cookielib" {
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
                        // Regular external module - generate normal import
                        // Collect unique imports with their aliases
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

                        // Generate: from source_module import attr1, attr2 as alias2, ...
                        let names: Vec<Alias> = imports_to_make
                            .into_iter()
                            .map(|(import_name, alias)| {
                                other::alias(&import_name, alias.as_deref())
                            })
                            .collect();

                        let import_from = statements::import_from(Some(&source_module), names, 0);
                        final_body.push(import_from);
                        log::debug!("Hoisted imports from {source_module} for hard dependencies");
                    }
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

        // Now reorder all collected inlined statements to ensure proper declaration order
        // This handles cross-module dependencies like classes inheriting from symbols defined in
        // other modules
        let reordered_inlined_stmts = self.reorder_cross_module_statements(all_inlined_stmts);
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

            // Skip the entry module - it doesn't need namespace assignments
            if module_name == params.entry_module_name {
                log::debug!("Skipping namespace creation for entry module '{module_name}'");
                continue;
            }

            let needs_namespace = self.namespace_imported_modules.contains_key(module_name)
                || (directly_imported_modules.contains(module_name)
                    && self
                        .module_exports
                        .get(module_name)
                        .is_some_and(|exports| exports.is_some()));

            if needs_namespace {
                // Check if this namespace was already created
                let namespace_var = module_name.cow_replace('.', "_").into_owned();
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
                        let namespace_stmt = self.create_namespace_for_inlined_module_static(
                            module_name,
                            module_rename_map,
                        );
                        final_body.push(namespace_stmt);

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
                            if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                                && !kept_symbols
                                    .contains(&(module_name.to_string(), original_name.clone()))
                            {
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
                                    if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                                        && !kept_symbols
                                            .contains(&(module_name.to_string(), export.clone()))
                                    {
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

                                    // This export wasn't renamed, add it directly
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
                    let namespace_var = module_name.cow_replace('.', "_").into_owned();
                    self.created_namespaces.insert(namespace_var);
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
                self.process_wrapper_module_globals(
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
            let sorted_wrapped = self.sort_wrapped_modules_by_dependencies(
                &wrapped_modules_to_init,
                params.sorted_modules,
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

                        // Generate a call to the init function
                        let init_call = Stmt::Expr(ruff_python_ast::StmtExpr {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(Expr::Call(ExprCall {
                                node_index: AtomicNodeIndex::dummy(),
                                func: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: init_func_name.clone().into(),
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
                        });
                        final_body.push(init_call);
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

                // Clone hard dependencies to avoid borrowing issues
                let hard_deps = self.hard_dependencies.clone();

                // Group hard dependencies by source module again
                let mut deps_by_source: FxIndexMap<String, Vec<&HardDependency>> =
                    FxIndexMap::default();
                for dep in &hard_deps {
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

            // After all modules are initialized, ensure sub-modules are attached to parent modules
            // This is necessary for relative imports like "from . import messages" to work
            // correctly
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
        }

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
            let deduped_imports = self.deduplicate_deferred_imports_with_existing(
                imports_without_init_calls,
                &final_body,
            );
            log::debug!(
                "After deduplication: {} imports remain from {} original",
                deduped_imports.len(),
                num_imports_before
            );
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
            let namespace_var = module_name.cow_replace('.', "_").into_owned();
            if self.created_namespaces.contains(&namespace_var) {
                log::debug!("Populating empty namespace '{namespace_var}' with symbols");

                // Don't mark the module as fully populated yet, we'll track individual symbols

                // Get the symbols that were inlined from this module
                if let Some(module_rename_map) = symbol_renames.get(module_name) {
                    // Add all renamed symbols as attributes to the namespace
                    for (original_name, renamed_name) in module_rename_map {
                        // Check if this symbol survived tree-shaking
                        if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                            && !kept_symbols
                                .contains(&(module_name.to_string(), original_name.clone()))
                        {
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
                                                        && Self::expr_equals(
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
                                    let target_path = self.extract_attribute_path(target_attr);

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
                                                    self.extract_attribute_path(existing_attr);
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
                    let attr_path = self.extract_attribute_path(attr);
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
                                    return Self::expr_equals(
                                        &existing_assign.value,
                                        &assign.value,
                                    );
                                }
                                false
                            })
                        }
                        Expr::Attribute(target_attr) => {
                            // For attribute assignments like schemas.user = ...
                            let target_path = self.extract_attribute_path(target_attr);

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
                                            self.extract_attribute_path(existing_attr);
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

            let deduped_imports = self.deduplicate_deferred_imports_with_existing(
                imports_without_init_calls,
                &final_body,
            );
            log::debug!(
                "Total deferred imports after deduplication: {}",
                deduped_imports.len()
            );
            final_body.extend(deduped_imports);
        }

        // Add hoisted imports at the beginning of final_body
        // This is done here after all transformations to ensure we capture all needed imports
        let mut hoisted_imports = Vec::new();
        self.add_hoisted_imports(&mut hoisted_imports);

        // Note: Namespace statements are now created earlier, before module inlining
        // to ensure they exist when module code references them

        hoisted_imports.extend(final_body);
        final_body = hoisted_imports;

        // Post-process: Fix forward reference issues in cross-module inheritance
        // Only apply reordering if we detect actual inheritance-based forward references
        if self.has_cross_module_inheritance_forward_refs(&final_body) {
            final_body = self.fix_forward_references_in_statements(final_body);
        }

        let mut result = ModModule {
            node_index: self.create_transformed_node("Bundled module root".to_string()),
            range: TextRange::default(),
            body: final_body,
        };

        // Assign proper node indices to all nodes in the final AST
        self.assign_node_indices_to_ast(&mut result);

        // Post-processing: Remove importlib import if it's unused
        // This happens when all importlib.import_module() calls were transformed
        self.remove_unused_importlib(&mut result);

        // Log transformation statistics
        let stats = self.transformation_context.get_stats();
        log::info!("Transformation statistics:");
        log::info!("  Total transformations: {}", stats.total_transformations);
        log::info!("  New nodes created: {}", stats.new_nodes);

        Ok(result)
    }

    /// Add hoisted imports to the final body
    fn add_hoisted_imports(&self, final_body: &mut Vec<Stmt>) {
        // Future imports first - combine all into a single import statement
        if !self.future_imports.is_empty() {
            // Sort future imports for deterministic output
            let mut sorted_imports: Vec<String> = self.future_imports.iter().cloned().collect();
            sorted_imports.sort();

            let aliases: Vec<Alias> = sorted_imports
                .into_iter()
                .map(|import| other::alias(&import, None))
                .collect();

            final_body.push(statements::import_from(Some("__future__"), aliases, 0));
        }

        // Then stdlib from imports - deduplicated and sorted by module name
        let mut sorted_modules: Vec<_> = self.stdlib_import_from_map.iter().collect();
        sorted_modules.sort_by_key(|(module_name, _)| *module_name);

        for (module_name, imported_names) in sorted_modules {
            // Skip importlib if it was fully transformed
            if module_name == "importlib" && self.importlib_fully_transformed {
                log::debug!("Skipping importlib from hoisted imports as it was fully transformed");
                continue;
            }

            // Sort the imported names for deterministic output
            let mut sorted_names: Vec<(String, Option<String>)> = imported_names
                .iter()
                .map(|(name, alias)| (name.clone(), alias.clone()))
                .collect();
            sorted_names.sort_by_key(|(name, _)| name.clone());

            let aliases: Vec<Alias> = sorted_names
                .into_iter()
                .map(|(name, alias_opt)| other::alias(&name, alias_opt.as_deref()))
                .collect();

            final_body.push(statements::import_from(Some(module_name), aliases, 0));
        }

        // IMPORTANT: Only safe stdlib imports are hoisted to the bundle top level.
        // Third-party imports are NEVER hoisted because they may have side effects
        // (e.g., registering plugins, modifying global state, network calls).
        // Third-party imports remain in their original location to preserve execution order.

        // Regular stdlib import statements - deduplicated and sorted by module name
        let mut seen_modules = FxIndexSet::default();
        let mut unique_imports = Vec::new();

        for stmt in &self.stdlib_import_statements {
            if let Stmt::Import(import_stmt) = stmt {
                self.collect_unique_imports(import_stmt, &mut seen_modules, &mut unique_imports);
            }
        }

        // Sort by module name for deterministic output
        unique_imports.sort_by_key(|(module_name, _)| module_name.clone());

        for (_, import_stmt) in unique_imports {
            final_body.push(import_stmt);
        }

        // NOTE: We do NOT hoist third-party regular import statements for the same reason
        // as above - they may have side effects and should remain in their original context.
    }

    /// Remove importlib import if it's unused after transformation
    fn remove_unused_importlib(&self, ast: &mut ModModule) {
        // Check if importlib is actually used in the code
        let mut importlib_used = false;

        // Check all expressions in the AST for importlib usage
        for stmt in &ast.body {
            if Self::stmt_uses_importlib(stmt) {
                importlib_used = true;
                break;
            }
        }

        if !importlib_used {
            log::debug!("importlib is unused after transformation, removing import");
            ast.body.retain(|stmt| {
                if let Stmt::Import(import_stmt) = stmt {
                    // Check if this is import importlib
                    !import_stmt
                        .names
                        .iter()
                        .any(|alias| alias.name.as_str() == "importlib")
                } else {
                    true
                }
            });
        }
    }

    fn trim_unused_imports_from_modules(
        &mut self,
        modules: &[(String, ModModule, PathBuf, String)],
        graph: &DependencyGraph,
        tree_shaker: Option<&crate::tree_shaking::TreeShaker>,
    ) -> Result<Vec<(String, ModModule, PathBuf, String)>> {
        let mut trimmed_modules = Vec::new();

        for (module_name, ast, module_path, content_hash) in modules {
            log::debug!("Trimming unused imports from module: {module_name}");
            let mut ast = ast.clone(); // Clone here to allow mutation

            // Check if this is an __init__.py file
            let is_init_py =
                module_path.file_name().and_then(|name| name.to_str()) == Some("__init__.py");

            // Get unused imports from the graph
            if let Some(module_dep_graph) = graph.get_module_by_name(module_name) {
                let mut unused_imports = crate::analyzers::import_analyzer::ImportAnalyzer::find_unused_imports_in_module(
                    module_dep_graph,
                    is_init_py,
                );

                // If tree shaking is enabled, also check if imported symbols were removed
                // Note: We only apply tree-shaking logic to "from module import symbol" style
                // imports, not to "import module" style imports, since module
                // imports set up namespace objects
                if let Some(shaker) = tree_shaker {
                    // Only apply tree-shaking-aware import removal if tree shaking is actually
                    // enabled Get the symbols that survive tree-shaking for
                    // this module
                    let used_symbols = shaker.get_used_symbols_for_module(module_name);

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
                                        && shaker.module_has_side_effects(module_name)
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
                                        unused_imports.push(
                                            crate::analyzers::types::UnusedImportInfo {
                                                name: local_name.clone(),
                                                module: from_module.clone(),
                                            },
                                        );
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
                                    for item in module_dep_graph.items.values() {
                                        if let crate::cribo_graph::ItemType::Assignment {
                                            targets,
                                        } = &item.item_type
                                        {
                                            // Check if this assignment reads the import
                                            if item.read_vars.contains(import_name) {
                                                // Check if any of the assignment targets are kept
                                                for target in targets {
                                                    if used_symbols.contains(target) {
                                                        log::debug!(
                                                            "Import '{import_name}' is used by \
                                                             surviving assignment to '{target}'"
                                                        );
                                                        used_by_surviving_code = true;
                                                        break;
                                                    }
                                                }
                                                if used_by_surviving_code {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }

                                // Extra check for normalized imports: If this is a normalized
                                // import and no assignments using
                                // it survived, it should be removed
                                if import_item.is_normalized_import {
                                    log::debug!(
                                        "Import '{import_name}' is a normalized import \
                                         (used_by_surviving_code: {used_by_surviving_code})"
                                    );
                                }

                                if !used_by_surviving_code {
                                    log::debug!(
                                        "Import '{import_name}' from module '{module}' is not \
                                         used by surviving code after tree-shaking (item_id: \
                                         {item_id:?})"
                                    );
                                    unused_imports.push(
                                        crate::analyzers::types::UnusedImportInfo {
                                            name: import_name.to_string(),
                                            module: module.clone(),
                                        },
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if !unused_imports.is_empty() {
                    log::debug!(
                        "Found {} unused imports in {}",
                        unused_imports.len(),
                        module_name
                    );
                    // Log unused imports details
                    Self::log_unused_imports_details(&unused_imports);

                    // Filter out unused imports from the AST
                    ast.body
                        .retain(|stmt| !self.should_remove_import_stmt(stmt, &unused_imports));
                }
            }

            trimmed_modules.push((
                module_name.clone(),
                ast,
                module_path.clone(),
                content_hash.clone(),
            ));
        }

        log::debug!(
            "Successfully trimmed unused imports from {} modules",
            trimmed_modules.len()
        );
        Ok(trimmed_modules)
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

    fn log_unused_imports_details(unused_imports: &[crate::analyzers::types::UnusedImportInfo]) {
        if log::log_enabled!(log::Level::Debug) {
            for unused in unused_imports {
                log::debug!("  - {} from {}", unused.name, unused.module);
            }
        }
    }

    /// Check if an import statement should be removed based on unused imports
    fn should_remove_import_stmt(
        &self,
        stmt: &Stmt,
        unused_imports: &[crate::analyzers::types::UnusedImportInfo],
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

    /// Create a dotted attribute assignment
    pub(super) fn create_dotted_attribute_assignment(
        &self,
        parent_module: &str,
        attr_name: &str,
        full_module_name: &str,
    ) -> Stmt {
        statements::assign(
            vec![expressions::attribute(
                expressions::name(parent_module, ExprContext::Load),
                attr_name,
                ExprContext::Store,
            )],
            expressions::name(full_module_name, ExprContext::Load),
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
            expressions::call(
                expressions::dotted_name(&["types", "SimpleNamespace"], ExprContext::Load),
                vec![],
                vec![],
            ),
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
        // We need to create a temporary Stmt to pass to rewrite_aliases_in_stmt
        let mut temp_stmt = Stmt::ClassDef(class_def.clone());
        self.rewrite_aliases_in_stmt(&mut temp_stmt, entry_module_renames);
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

    fn rewrite_aliases_in_stmt(
        &self,
        stmt: &mut Stmt,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Rewrite in parameter annotations and defaults
                let params = &mut func_def.parameters;
                for param in &mut params.args {
                    if let Some(ref mut annotation) = param.parameter.annotation {
                        self.rewrite_aliases_in_expr(annotation, alias_to_canonical);
                    }
                    if let Some(ref mut default) = param.default {
                        self.rewrite_aliases_in_expr(default, alias_to_canonical);
                    }
                }

                // Rewrite return type annotation
                if let Some(ref mut returns) = func_def.returns {
                    self.rewrite_aliases_in_expr(returns, alias_to_canonical);
                }

                // Rewrite in function body
                for stmt in &mut func_def.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::ClassDef(class_def) => {
                // Rewrite in base classes
                if let Some(ref mut arguments) = class_def.arguments {
                    for arg in &mut arguments.args {
                        self.rewrite_aliases_in_expr(arg, alias_to_canonical);
                    }
                }
                // Rewrite in class body
                for stmt in &mut class_def.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::If(if_stmt) => {
                self.rewrite_aliases_in_expr(&mut if_stmt.test, alias_to_canonical);
                for stmt in &mut if_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(ref mut condition) = clause.test {
                        self.rewrite_aliases_in_expr(condition, alias_to_canonical);
                    }
                    for stmt in &mut clause.body {
                        self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                self.rewrite_aliases_in_expr(&mut while_stmt.test, alias_to_canonical);
                for stmt in &mut while_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut while_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::For(for_stmt) => {
                self.rewrite_aliases_in_expr(&mut for_stmt.iter, alias_to_canonical);
                for stmt in &mut for_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut for_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::With(with_stmt) => {
                for item in &mut with_stmt.items {
                    self.rewrite_aliases_in_expr(&mut item.context_expr, alias_to_canonical);
                }
                for stmt in &mut with_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::Try(try_stmt) => {
                for stmt in &mut try_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for handler in &mut try_stmt.handlers {
                    self.rewrite_aliases_in_except_handler(handler, alias_to_canonical);
                }
                for stmt in &mut try_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut try_stmt.finalbody {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::Assign(assign) => {
                // Rewrite in targets
                for target in &mut assign.targets {
                    self.rewrite_aliases_in_expr(target, alias_to_canonical);
                }
                // Rewrite in value
                self.rewrite_aliases_in_expr(&mut assign.value, alias_to_canonical);
            }
            Stmt::AugAssign(aug_assign) => {
                self.rewrite_aliases_in_expr(&mut aug_assign.target, alias_to_canonical);
                self.rewrite_aliases_in_expr(&mut aug_assign.value, alias_to_canonical);
            }
            Stmt::AnnAssign(ann_assign) => {
                self.rewrite_aliases_in_expr(&mut ann_assign.target, alias_to_canonical);
                self.rewrite_aliases_in_expr(&mut ann_assign.annotation, alias_to_canonical);
                if let Some(ref mut value) = ann_assign.value {
                    self.rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Stmt::Expr(expr_stmt) => {
                self.rewrite_aliases_in_expr(&mut expr_stmt.value, alias_to_canonical);
            }
            Stmt::Return(return_stmt) => {
                if let Some(ref mut value) = return_stmt.value {
                    self.rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Stmt::Raise(raise_stmt) => {
                if let Some(ref mut exc) = raise_stmt.exc {
                    self.rewrite_aliases_in_expr(exc, alias_to_canonical);
                }
                if let Some(ref mut cause) = raise_stmt.cause {
                    self.rewrite_aliases_in_expr(cause, alias_to_canonical);
                }
            }
            Stmt::Assert(assert_stmt) => {
                self.rewrite_aliases_in_expr(&mut assert_stmt.test, alias_to_canonical);
                if let Some(ref mut msg) = assert_stmt.msg {
                    self.rewrite_aliases_in_expr(msg, alias_to_canonical);
                }
            }
            Stmt::Delete(delete_stmt) => {
                for target in &mut delete_stmt.targets {
                    self.rewrite_aliases_in_expr(target, alias_to_canonical);
                }
            }
            Stmt::Global(global_stmt) => {
                // Apply renames to global variable names
                for name in &mut global_stmt.names {
                    let name_str = name.as_str();
                    if let Some(new_name) = alias_to_canonical.get(name_str) {
                        log::debug!("Rewriting global variable '{name_str}' to '{new_name}'");
                        *name = Identifier::new(new_name, TextRange::default());
                    }
                }
            }
            Stmt::Nonlocal(_) => {
                // Nonlocal statements don't need rewriting in our use case
            }
            Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => {
                // These don't contain expressions
            }
            Stmt::Import(_) | Stmt::ImportFrom(_) => {
                // Import statements are handled separately and shouldn't be rewritten here
            }
            Stmt::TypeAlias(type_alias) => {
                self.rewrite_aliases_in_expr(&mut type_alias.value, alias_to_canonical);
            }
            Stmt::Match(_) => {
                // Match statements are not handled in the original implementation
            }
            // IPython-specific statements
            Stmt::IpyEscapeCommand(_) => {
                // These don't contain expressions that need rewriting
            }
        }
    }

    /// Helper to rewrite aliases in except handlers to reduce nesting
    fn rewrite_aliases_in_except_handler(
        &self,
        handler: &mut ruff_python_ast::ExceptHandler,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match handler {
            ruff_python_ast::ExceptHandler::ExceptHandler(except_handler) => {
                for stmt in &mut except_handler.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
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
                    self.transform_expr_for_lifted_globals(
                        target,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
                self.transform_expr_for_lifted_globals(
                    &mut assign.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::Expr(expr_stmt) => {
                self.transform_expr_for_lifted_globals(
                    &mut expr_stmt.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::If(if_stmt) => {
                self.transform_expr_for_lifted_globals(
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
                        self.transform_expr_for_lifted_globals(
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
                self.transform_expr_for_lifted_globals(
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
                self.transform_expr_for_lifted_globals(
                    &mut for_stmt.target,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                self.transform_expr_for_lifted_globals(
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
                    self.transform_expr_for_lifted_globals(
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
                self.transform_expr_for_lifted_globals(
                    &mut aug_assign.target,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                self.transform_expr_for_lifted_globals(
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

    /// Transform an expression to use lifted globals
    fn transform_expr_for_lifted_globals(
        &self,
        expr: &mut Expr,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::semantic_bundler::ModuleGlobalInfo,
        in_function_with_globals: Option<&FxIndexSet<String>>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                // Transform if this is a lifted global and we're in a function that declares it
                // global
                if let Some(function_globals) = in_function_with_globals
                    && function_globals.contains(name_expr.id.as_str())
                    && let Some(lifted_name) = lifted_names.get(name_expr.id.as_str())
                {
                    name_expr.id = lifted_name.clone().into();
                }
            }
            Expr::Call(call) => {
                self.transform_expr_for_lifted_globals(
                    &mut call.func,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
                for arg in &mut call.arguments.args {
                    self.transform_expr_for_lifted_globals(
                        arg,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            Expr::Attribute(attr) => {
                self.transform_expr_for_lifted_globals(
                    &mut attr.value,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::FString(_) => {
                self.transform_fstring_for_lifted_globals(
                    expr,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::BinOp(binop) => {
                self.transform_expr_for_lifted_globals(
                    &mut binop.left,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
                self.transform_expr_for_lifted_globals(
                    &mut binop.right,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::UnaryOp(unaryop) => {
                self.transform_expr_for_lifted_globals(
                    &mut unaryop.operand,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::Compare(compare) => {
                self.transform_expr_for_lifted_globals(
                    &mut compare.left,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
                for comparator in &mut compare.comparators {
                    self.transform_expr_for_lifted_globals(
                        comparator,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            Expr::Subscript(subscript) => {
                self.transform_expr_for_lifted_globals(
                    &mut subscript.value,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
                self.transform_expr_for_lifted_globals(
                    &mut subscript.slice,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    self.transform_expr_for_lifted_globals(
                        elem,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            Expr::Tuple(tuple_expr) => {
                for elem in &mut tuple_expr.elts {
                    self.transform_expr_for_lifted_globals(
                        elem,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            Expr::Dict(dict_expr) => {
                for item in &mut dict_expr.items {
                    if let Some(key) = &mut item.key {
                        self.transform_expr_for_lifted_globals(
                            key,
                            lifted_names,
                            global_info,
                            in_function_with_globals,
                        );
                    }
                    self.transform_expr_for_lifted_globals(
                        &mut item.value,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            _ => {
                // Other expressions handled as needed
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
    fn get_unique_name_with_module_suffix(&self, base_name: &str, module_name: &str) -> String {
        let module_suffix = module_name.cow_replace('.', "_").into_owned();
        format!("{base_name}_{module_suffix}")
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
                                    // Use the alias if it's mandatory, otherwise use the imported
                                    // attr
                                    let name_to_use = if hard_dep.alias_is_mandatory
                                        && hard_dep.alias.is_some()
                                    {
                                        hard_dep
                                            .alias
                                            .as_ref()
                                            .expect(
                                                "alias should exist when alias_is_mandatory is \
                                                 true and alias.is_some() is true",
                                            )
                                            .clone()
                                    } else {
                                        hard_dep.imported_attr.clone()
                                    };

                                    *arg = Expr::Name(ExprName {
                                        node_index: AtomicNodeIndex::dummy(),
                                        id: name_to_use.clone().into(),
                                        ctx: ExprContext::Load,
                                        range: TextRange::default(),
                                    });
                                    log::info!(
                                        "Rewrote base class {} to {} for class {} in inlined \
                                         module",
                                        hard_dep.base_class,
                                        name_to_use,
                                        class_name
                                    );
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
                        if self.is_self_referential_assignment(assign) {
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
    fn reorder_cross_module_statements(&self, statements: Vec<Stmt>) -> Vec<Stmt> {
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
                        matches!(&assign.targets[0], Expr::Attribute(_))
                    } else {
                        false
                    };

                    if is_class_attribute {
                        // Class attribute assignments should come after class definitions
                        other_stmts.push(stmt);
                    } else {
                        // Check if this assignment defines a base class symbol
                        let defines_base_class = if assign.targets.len() == 1 {
                            if let Expr::Name(target) = &assign.targets[0] {
                                // Only consider it a base class assignment if:
                                // 1. The target is used as a base class
                                // 2. The value is an attribute access (e.g., json.JSONDecodeError)
                                if base_class_symbols.contains(target.id.as_str()) {
                                    match assign.value.as_ref() {
                                        Expr::Attribute(_) => true, // e.g., json.JSONDecodeError
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
        // 2. Base class assignments (must come before class definitions)
        // 3. Regular assignments
        // 4. Classes (must come before functions that might use them)
        // 5. Functions (may depend on classes)
        // 6. Other statements (including class attribute assignments)
        let mut reordered = Vec::new();
        reordered.extend(imports);
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
        rewrite_aliases_in_expr_impl(expr, alias_to_canonical);
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
                    self.rewrite_aliases_in_expr(arg, module_renames);
                }
            }
        }

        // Apply renames and resolve import aliases in class body
        for body_stmt in &mut class_def_clone.body {
            Self::resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
            self.rewrite_aliases_in_stmt(body_stmt, module_renames);
            // Also apply semantic renames from context
            if let Some(semantic_renames) = ctx.module_renames.get(module_name) {
                self.rewrite_aliases_in_stmt(body_stmt, semantic_renames);
            }
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

            // Set __qualname__ to match __name__ for proper repr()
            let qualname_attr_stmt = Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: renamed_name.clone().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("__qualname__", TextRange::default()),
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
            ctx.inlined_stmts.push(qualname_attr_stmt);
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
impl<'a> HybridStaticBundler<'a> {
    /// Create a module reference assignment
    fn create_module_reference_assignment(&self, target_name: &str, module_name: &str) -> Stmt {
        // Simply assign the module reference: target_name = module_name
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: target_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: module_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create module initialization statements for wrapper modules when they are imported
    fn create_module_initialization_for_import(&self, module_name: &str) -> Vec<Stmt> {
        let mut stmts = Vec::new();

        // Check if this is a wrapper module that needs initialization
        if let Some(synthetic_name) = self.module_registry.get(module_name) {
            // Generate the init call
            let init_func_name =
                crate::code_generator::module_registry::get_init_function_name(synthetic_name);

            // Call the init function and get the result
            let init_call = Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: init_func_name.clone().into(),
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
            stmts.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: INIT_RESULT_VAR.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(init_call),
                range: TextRange::default(),
            }));

            // Merge attributes from init result into existing namespace
            self.generate_merge_module_attributes(&mut stmts, module_name, INIT_RESULT_VAR);
        } else {
            // Direct assignment for simple and dotted modules
            let target_expr = if module_name.contains('.') {
                // Create attribute expression for dotted modules
                let parts: Vec<&str> = module_name.split('.').collect();
                let mut expr = Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: parts[0].into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });

                for part in &parts[1..parts.len() - 1] {
                    expr = Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(expr),
                        attr: Identifier::new(*part, TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
                }

                Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(expr),
                    attr: Identifier::new(parts[parts.len() - 1], TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })
            } else {
                // Simple name expression
                Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: module_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })
            };

            stmts.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![target_expr],
                value: Box::new(init_call),
                range: TextRange::default(),
            }));
        }

        stmts
    }

    /// Create parent namespaces for dotted imports
    fn create_parent_namespaces(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
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
    fn create_dotted_assignments(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
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
    fn create_all_namespace_objects(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
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
            let flattened_name = partial_module.cow_replace('.', "_").into_owned();
            let should_use_flattened = self.inlined_modules.contains(&partial_module)
                && self
                    .namespaces_with_initial_symbols
                    .contains(&partial_module);

            let namespace_expr = if should_use_flattened {
                // Use the flattened namespace variable
                Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: flattened_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })
            } else {
                // Create empty namespace object
                Expr::Call(ExprCall {
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
                        args: vec![].into(),
                        keywords: vec![].into(),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })
            };

            // Assign to the first part of the name
            if i == 1 {
                result_stmts.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(namespace_expr),
                    range: TextRange::default(),
                }));
            } else {
                // For deeper levels, create attribute assignments
                let mut target = Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: parts[0].into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });

                // Build up the chain up to but not including the last part
                for part in &parts[1..(i - 1)] {
                    target = Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(target),
                        attr: Identifier::new(&**part, TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
                }

                result_stmts.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(target),
                        attr: Identifier::new(parts[i - 1], TextRange::default()),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(namespace_expr),
                    range: TextRange::default(),
                }));
            }
        }
    }

    /// Populate a namespace with symbols from an inlined module using a specific target name
    fn populate_namespace_with_module_symbols_with_renames(
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
            let mut all_target = Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: parts[0].into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            });

            for part in &parts[1..] {
                all_target = Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(all_target),
                    attr: Identifier::new(&**part, TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });
            }

            // Filter exports to only include symbols that survived tree-shaking
            let filtered_exports = self.filter_exports_by_tree_shaking_with_logging(
                exports,
                module_name,
                self.tree_shaking_keep_symbols.as_ref(),
            );

            // Create __all__ = [...] assignment with filtered exports
            let all_list = Expr::List(ExprList {
                node_index: AtomicNodeIndex::dummy(),
                elts: filtered_exports
                    .iter()
                    .map(|name| {
                        Expr::StringLiteral(ExprStringLiteral {
                            node_index: AtomicNodeIndex::dummy(),
                            value: StringLiteralValue::single(StringLiteral {
                                node_index: AtomicNodeIndex::dummy(),
                                value: name.as_str().into(),
                                flags: StringLiteralFlags::empty(),
                                range: TextRange::default(),
                            }),
                            range: TextRange::default(),
                        })
                    })
                    .collect(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            });

            result_stmts.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(all_target),
                    attr: Identifier::new("__all__", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(all_list),
                range: TextRange::default(),
            }));

            log::info!(
                "Created __all__ assignment for namespace '{target_name}' with exports: \
                 {filtered_exports:?}"
            );

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
                let should_include = if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                {
                    // First check if this symbol is directly defined in this module
                    if kept_symbols.contains(&(module_name.to_string(), (*symbol).clone())) {
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
                let target = if parts.len() == 1 {
                    // Simple module - use the name directly
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
                } else {
                    // Dotted module - build the attribute chain
                    let mut base = Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });

                    for part in &parts[1..] {
                        base = Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(base),
                            attr: Identifier::new(&**part, TextRange::default()),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        });
                    }
                    base
                };

                // Check if this specific symbol was already populated after deferred imports
                // This happens for modules that had forward references and were populated later
                if self
                    .symbols_populated_after_deferred
                    .contains(&(module_name.to_string(), (*symbol).clone()))
                    && target_name == module_name.cow_replace('.', "_").as_ref()
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
                        if let Some((_, ast, _, _)) = module_asts
                            .iter()
                            .find(|(name, _, _, _)| name == module_name)
                        {
                            // Check if this symbol is imported from an inlined submodule
                            for stmt in &ast.body {
                                if let Stmt::ImportFrom(import_from) = stmt {
                                    let resolved_module =
                                        self.resolve_relative_import(import_from, module_name);
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

                    // Log when creating namespace assignments
                    log::info!(
                        "Creating namespace assignment: {target_name}.{symbol} = \
                         {actual_symbol_name} (in \
                         populate_namespace_with_module_symbols_with_renames)"
                    );

                    // Now add the symbol as an attribute (e.g., greetings.greeting.get_greeting =
                    // get_greeting)
                    let attr_assignment = Stmt::Assign(StmtAssign {
                        node_index: AtomicNodeIndex::dummy(),
                        targets: vec![Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(target),
                            attr: Identifier::new(*symbol, TextRange::default()),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: actual_symbol_name.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    });

                    result_stmts.push(attr_assignment);
                }
            }
        }
    }

    /// Filter exports by tree shaking with logging
    fn filter_exports_by_tree_shaking_with_logging<'b>(
        &self,
        exports: &'b [String],
        module_name: &str,
        kept_symbols: Option<&indexmap::IndexSet<(String, String)>>,
    ) -> Vec<&'b String> {
        if let Some(kept_symbols) = kept_symbols {
            let result: Vec<&String> = exports
                .iter()
                .filter(|symbol| {
                    // Check if this symbol is kept in this module
                    let is_kept =
                        kept_symbols.contains(&(module_name.to_string(), (*symbol).clone()));
                    if !is_kept {
                        log::debug!(
                            "Filtering out symbol '{symbol}' from __all__ of module \
                             '{module_name}' - removed by tree-shaking"
                        );
                    } else {
                        log::debug!(
                            "Keeping symbol '{symbol}' in __all__ of module '{module_name}' - \
                             survived tree-shaking"
                        );
                    }
                    is_kept
                })
                .collect();
            log::debug!(
                "Module '{}' __all__ filtering: {} symbols -> {} symbols",
                module_name,
                exports.len(),
                result.len()
            );
            result
        } else {
            // No tree-shaking, include all exports
            exports.iter().collect()
        }
    }

    /// Create a namespace object for an inlined module
    fn create_namespace_object_for_module(&self, target_name: &str, module_name: &str) -> Stmt {
        // Check if we should use a flattened namespace instead of creating an empty one
        let flattened_name = module_name.cow_replace('.', "_").into_owned();
        let should_use_flattened = self.inlined_modules.contains(module_name)
            && self.namespaces_with_initial_symbols.contains(module_name);

        if should_use_flattened {
            // Create assignment: target_name = flattened_name
            return Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: target_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: flattened_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            });
        }

        // For inlined modules, we need to return a vector of statements:
        // 1. Create the namespace object
        // 2. Add all the module's symbols to it

        // We'll create a compound statement that does both
        let _stmts: Vec<Stmt> = Vec::new();

        // First, create the empty namespace
        let namespace_expr = Expr::Call(ExprCall {
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
                args: vec![].into(),
                keywords: vec![].into(),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        // Create assignment for the namespace

        // For now, return just the namespace creation
        // The actual symbol population needs to happen after all symbols are available
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: target_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(namespace_expr),
            range: TextRange::default(),
        })
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
        let loop_target = Expr::Name(ExprName {
            node_index: AtomicNodeIndex::dummy(),
            id: attr_var.into(),
            ctx: ExprContext::Store,
            range: TextRange::default(),
        });

        // dir(source_module)
        let dir_call = Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::dummy(),
            func: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: "dir".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                node_index: AtomicNodeIndex::dummy(),
                args: Box::from([Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: source_module_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })]),
                keywords: Box::from([]),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        // not attr.startswith('_')
        let condition = Expr::UnaryOp(ruff_python_ast::ExprUnaryOp {
            node_index: AtomicNodeIndex::dummy(),
            op: ruff_python_ast::UnaryOp::Not,
            operand: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: attr_var.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("startswith", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([Expr::StringLiteral(ExprStringLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        value: StringLiteralValue::single(StringLiteral {
                            node_index: AtomicNodeIndex::dummy(),
                            value: "_".into(),
                            range: TextRange::default(),
                            flags: StringLiteralFlags::empty(),
                        }),
                        range: TextRange::default(),
                    })]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        });

        // getattr(source_module, attr)
        let getattr_call = Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::dummy(),
            func: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: "getattr".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                node_index: AtomicNodeIndex::dummy(),
                args: Box::from([
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: source_module_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    }),
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: attr_var.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    }),
                ]),
                keywords: Box::from([]),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        // setattr(namespace, attr, getattr(...))
        let setattr_call = Stmt::Expr(ruff_python_ast::StmtExpr {
            node_index: AtomicNodeIndex::dummy(),
            value: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "setattr".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([
                        Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: namespace_name.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        }),
                        Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: attr_var.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        }),
                        getattr_call,
                    ]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        });

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
                        new_body.push(Stmt::Assign(StmtAssign {
                            node_index: AtomicNodeIndex::dummy(),
                            targets: vec![Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: "module".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new(original_name, TextRange::default()),
                                ctx: ExprContext::Store,
                                range: TextRange::default(),
                            })],
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: var_name.into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
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
                        new_body.push(Stmt::Assign(StmtAssign {
                            node_index: AtomicNodeIndex::dummy(),
                            targets: vec![Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: "module".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new(original_name, TextRange::default()),
                                ctx: ExprContext::Store,
                                range: TextRange::default(),
                            })],
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: var_name.into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
                    }
                }
            }
            _ => {}
        }
    }

    /// Transform f-string expressions for lifted globals
    fn transform_fstring_for_lifted_globals(
        &self,
        expr: &mut Expr,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::semantic_bundler::ModuleGlobalInfo,
        in_function_with_globals: Option<&FxIndexSet<String>>,
    ) {
        if let Expr::FString(fstring) = expr {
            let fstring_range = fstring.range;
            let mut transformed_elements = Vec::new();
            let mut any_transformed = false;

            for element in fstring.value.elements() {
                match element {
                    ruff_python_ast::InterpolatedStringElement::Literal(lit_elem) => {
                        // Literal elements stay the same
                        transformed_elements.push(
                            ruff_python_ast::InterpolatedStringElement::Literal(lit_elem.clone()),
                        );
                    }
                    ruff_python_ast::InterpolatedStringElement::Interpolation(expr_elem) => {
                        let (new_element, was_transformed) = self.transform_fstring_expression(
                            expr_elem,
                            lifted_names,
                            global_info,
                            in_function_with_globals,
                        );
                        transformed_elements.push(
                            ruff_python_ast::InterpolatedStringElement::Interpolation(new_element),
                        );
                        if was_transformed {
                            any_transformed = true;
                        }
                    }
                }
            }

            // If any expressions were transformed, we need to rebuild the f-string
            if any_transformed {
                // Create a new FString with our transformed elements
                let new_fstring = ruff_python_ast::FString {
                    node_index: AtomicNodeIndex::dummy(),
                    elements: ruff_python_ast::InterpolatedStringElements::from(
                        transformed_elements,
                    ),
                    range: TextRange::default(),
                    flags: ruff_python_ast::FStringFlags::empty(),
                };

                // Create a new FStringValue containing our FString
                let new_value = ruff_python_ast::FStringValue::single(new_fstring);

                // Replace the entire expression with the new f-string
                *expr = Expr::FString(ruff_python_ast::ExprFString {
                    node_index: AtomicNodeIndex::dummy(),
                    value: new_value,
                    range: fstring_range,
                });

                log::debug!("Transformed f-string expressions for lifted globals");
            }
        }
    }

    /// Transform a single f-string expression element
    fn transform_fstring_expression(
        &self,
        expr_elem: &ruff_python_ast::InterpolatedElement,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::semantic_bundler::ModuleGlobalInfo,
        in_function_with_globals: Option<&FxIndexSet<String>>,
    ) -> (ruff_python_ast::InterpolatedElement, bool) {
        // Clone and transform the expression
        let mut new_expr = (*expr_elem.expression).clone();
        let old_expr_str = format!("{new_expr:?}");

        self.transform_expr_for_lifted_globals(
            &mut new_expr,
            lifted_names,
            global_info,
            in_function_with_globals,
        );

        let new_expr_str = format!("{new_expr:?}");
        let was_transformed = old_expr_str != new_expr_str;

        // Create a new expression element with the transformed expression
        let new_element = ruff_python_ast::InterpolatedElement {
            node_index: AtomicNodeIndex::dummy(),
            expression: Box::new(new_expr),
            debug_text: expr_elem.debug_text.clone(),
            conversion: expr_elem.conversion,
            format_spec: expr_elem.format_spec.clone(),
            range: expr_elem.range,
        };

        (new_element, was_transformed)
    }

    /// Check if there are cross-module inheritance forward references
    fn has_cross_module_inheritance_forward_refs(&self, statements: &[Stmt]) -> bool {
        // Look for classes that inherit from base classes that are defined later
        // This can happen when symbol renaming creates forward references

        // First, collect all class positions and assignment positions
        let mut class_positions = FxIndexMap::default();
        let mut assignment_positions = FxIndexMap::default();

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
                }
                _ => {}
            }
        }

        // Now check for forward references
        for stmt in statements {
            if let Stmt::ClassDef(class_def) = stmt
                && let Some(arguments) = &class_def.arguments
            {
                let class_name = class_def.name.as_str();
                let class_pos = class_positions
                    .get(class_name)
                    .copied()
                    .unwrap_or(usize::MAX);

                for base in &arguments.args {
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
                }
            }
        }
        false
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
}

/// Helper function to recursively rewrite aliases in an expression
fn rewrite_aliases_in_expr_impl(expr: &mut Expr, alias_to_canonical: &FxIndexMap<String, String>) {
    match expr {
        Expr::Name(name_expr) => {
            let name_str = name_expr.id.as_str();
            if let Some(canonical) = alias_to_canonical.get(name_str) {
                log::debug!("Rewriting alias '{name_str}' to canonical '{canonical}'");
                name_expr.id = canonical.clone().into();
            }
        }
        Expr::Attribute(attr_expr) => {
            // Handle cases like j.dumps -> json.dumps
            // First check if this is a direct attribute on an aliased name
            if let Expr::Name(name_expr) = attr_expr.value.as_ref() {
                let name_str = name_expr.id.as_str();
                if alias_to_canonical.contains_key(name_str) {
                    log::debug!(
                        "Found attribute access on alias: {}.{}",
                        name_str,
                        attr_expr.attr.as_str()
                    );
                }
            }
            rewrite_aliases_in_expr_impl(&mut attr_expr.value, alias_to_canonical);
        }
        Expr::Call(call_expr) => {
            rewrite_aliases_in_expr_impl(&mut call_expr.func, alias_to_canonical);
            for arg in &mut call_expr.arguments.args {
                rewrite_aliases_in_expr_impl(arg, alias_to_canonical);
            }
            // Also process keyword arguments
            for keyword in &mut call_expr.arguments.keywords {
                rewrite_aliases_in_expr_impl(&mut keyword.value, alias_to_canonical);
            }
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                rewrite_aliases_in_expr_impl(elem, alias_to_canonical);
            }
        }
        Expr::Dict(dict_expr) => {
            for item in &mut dict_expr.items {
                if let Some(ref mut key) = item.key {
                    rewrite_aliases_in_expr_impl(key, alias_to_canonical);
                }
                rewrite_aliases_in_expr_impl(&mut item.value, alias_to_canonical);
            }
        }
        Expr::Tuple(tuple_expr) => {
            for elem in &mut tuple_expr.elts {
                rewrite_aliases_in_expr_impl(elem, alias_to_canonical);
            }
        }
        Expr::Set(set_expr) => {
            for elem in &mut set_expr.elts {
                rewrite_aliases_in_expr_impl(elem, alias_to_canonical);
            }
        }
        Expr::BinOp(binop_expr) => {
            rewrite_aliases_in_expr_impl(&mut binop_expr.left, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut binop_expr.right, alias_to_canonical);
        }
        Expr::UnaryOp(unaryop_expr) => {
            rewrite_aliases_in_expr_impl(&mut unaryop_expr.operand, alias_to_canonical);
        }
        Expr::Compare(compare_expr) => {
            rewrite_aliases_in_expr_impl(&mut compare_expr.left, alias_to_canonical);
            for comparator in &mut compare_expr.comparators {
                rewrite_aliases_in_expr_impl(comparator, alias_to_canonical);
            }
        }
        Expr::BoolOp(boolop_expr) => {
            for value in &mut boolop_expr.values {
                rewrite_aliases_in_expr_impl(value, alias_to_canonical);
            }
        }
        Expr::If(if_expr) => {
            rewrite_aliases_in_expr_impl(&mut if_expr.test, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut if_expr.body, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut if_expr.orelse, alias_to_canonical);
        }
        Expr::ListComp(listcomp_expr) => {
            rewrite_aliases_in_expr_impl(&mut listcomp_expr.elt, alias_to_canonical);
            for generator in &mut listcomp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::SetComp(setcomp_expr) => {
            rewrite_aliases_in_expr_impl(&mut setcomp_expr.elt, alias_to_canonical);
            for generator in &mut setcomp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::DictComp(dictcomp_expr) => {
            rewrite_aliases_in_expr_impl(&mut dictcomp_expr.key, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut dictcomp_expr.value, alias_to_canonical);
            for generator in &mut dictcomp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::Subscript(subscript_expr) => {
            // Rewrite the value expression (e.g., the `obj` in `obj[key]`)
            rewrite_aliases_in_expr_impl(&mut subscript_expr.value, alias_to_canonical);
            // Rewrite the slice - this handles type annotations like Dict[str, Any]
            rewrite_aliases_in_expr_impl(&mut subscript_expr.slice, alias_to_canonical);
        }
        Expr::Slice(slice_expr) => {
            if let Some(ref mut lower) = slice_expr.lower {
                rewrite_aliases_in_expr_impl(lower, alias_to_canonical);
            }
            if let Some(ref mut upper) = slice_expr.upper {
                rewrite_aliases_in_expr_impl(upper, alias_to_canonical);
            }
            if let Some(ref mut step) = slice_expr.step {
                rewrite_aliases_in_expr_impl(step, alias_to_canonical);
            }
        }
        Expr::Lambda(lambda_expr) => {
            rewrite_aliases_in_expr_impl(&mut lambda_expr.body, alias_to_canonical);
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                rewrite_aliases_in_expr_impl(value, alias_to_canonical);
            }
        }
        Expr::YieldFrom(yieldfrom_expr) => {
            rewrite_aliases_in_expr_impl(&mut yieldfrom_expr.value, alias_to_canonical);
        }
        Expr::Await(await_expr) => {
            rewrite_aliases_in_expr_impl(&mut await_expr.value, alias_to_canonical);
        }
        Expr::Starred(starred_expr) => {
            rewrite_aliases_in_expr_impl(&mut starred_expr.value, alias_to_canonical);
        }
        Expr::FString(_fstring_expr) => {
            // FString handling is complex due to its structure
            // For now, skip FString rewriting as it's rarely used with module aliases
            log::debug!("FString expression rewriting not yet implemented");
        }
        // Constant values and other literals don't need rewriting
        Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_) => {}
        // Generator expressions
        Expr::Generator(gen_expr) => {
            rewrite_aliases_in_expr_impl(&mut gen_expr.elt, alias_to_canonical);
            for generator in &mut gen_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        // Named expressions (walrus operator)
        Expr::Named(named_expr) => {
            rewrite_aliases_in_expr_impl(&mut named_expr.value, alias_to_canonical);
        }
        _ => {}
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
