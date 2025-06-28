use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use indexmap::{IndexMap as FxIndexMap, IndexSet as FxIndexSet};
use log::{debug, trace, warn};
use ruff_python_ast::{
    self as ast, Arguments, CmpOp, Expr, ExprAttribute, ExprCall, ExprCompare, ExprContext,
    ExprDict, ExprList, ExprName, ExprStringLiteral, ExprSubscript, ExprTuple, Identifier, Int,
    Keyword, ModModule, Stmt, StmtAnnAssign, StmtAssign, StmtClassDef, StmtExpr, StmtFunctionDef,
    StmtIf, StmtImport, StmtImportFrom, StmtReturn, StmtWith, WithItem, visitor::Visitor,
};
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    code_generator::{
        circular_deps::SymbolDependencyGraph,
        context::{
            BundleParams, DirectImportContext, HardDependency, InlineContext,
            ModuleTransformContext, ProcessGlobalsParams, SemanticContext, TransformFunctionParams,
        },
        globals::{GlobalsLifter, transform_globals_in_expr, transform_globals_in_stmt},
        import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
    },
    cribo_graph::CriboGraph as DependencyGraph,
    semantic_bundler::{ModuleGlobalInfo, SemanticBundler, SymbolRegistry},
    transformation_context::TransformationContext,
    visitors::{ImportDiscoveryVisitor, NoOpsRemovalTransformer},
};

/// This approach avoids forward reference issues while maintaining Python module semantics
#[derive(Debug)]
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

impl<'a> Default for HybridStaticBundler<'a> {
    fn default() -> Self {
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
            module_info_registry: None,
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
            use_module_cache_model: true,
        }
    }
}

// Main implementation
impl<'a> HybridStaticBundler<'a> {
    /// Create a new bundler instance
    pub fn new() -> Self {
        Self::default()
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
        current_module: &str,
        module_path: &Path,
        level: u32,
        module: Option<&str>,
    ) -> Option<String> {
        // Implementation from original file
        // This is a placeholder - the actual implementation is quite complex
        None
    }

    /// Filter exports based on tree shaking
    pub fn filter_exports_by_tree_shaking(
        &self,
        keep_symbols: Option<&indexmap::IndexSet<(String, String)>>,
        module_name: &str,
        exports: &[String],
    ) -> Vec<String> {
        if let Some(keep_symbols) = keep_symbols {
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

    // More methods to be moved from the original implementation...
}

/// Main entry point for bundling modules
pub fn bundle_modules(params: BundleParams) -> Result<ModModule> {
    // This function implementation is very large and should be moved from the original file
    // For now, creating a placeholder
    let mut bundler = HybridStaticBundler::new();

    // The actual implementation involves:
    // 1. Processing modules and collecting information
    // 2. Handling circular dependencies
    // 3. Generating the bundled output
    // 4. Managing imports and exports

    // Placeholder return
    Ok(ModModule {
        range: TextRange::default(),
        body: vec![],
        node_index: Default::default(),
    })
}

// Additional implementation blocks and helper functions should be moved here
// from the original code_generator.rs file
