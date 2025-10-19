use std::path::Path;

use ruff_python_ast::{ModModule, Stmt};

use crate::{
    cribo_graph::CriboGraph as DependencyGraph,
    semantic_bundler::{SemanticBundler, SymbolRegistry},
    types::{FxIndexMap, FxIndexSet},
};

/// Context for transforming a module
#[derive(Debug)]
pub struct ModuleTransformContext<'a> {
    pub module_name: &'a str,
    pub module_path: &'a Path,
    pub global_info: Option<crate::semantic_bundler::ModuleGlobalInfo>,
    pub semantic_bundler: Option<&'a SemanticBundler>,
    pub python_version: u8,
    /// Whether this module is being transformed as a wrapper function body
    pub is_wrapper_body: bool,
    /// Whether this module is in a circular dependency chain
    pub is_in_circular_deps: bool,
}

/// Context for inlining modules
#[derive(Debug)]
pub struct InlineContext<'a> {
    pub module_exports_map: &'a FxIndexMap<crate::resolver::ModuleId, Option<Vec<String>>>,
    pub global_symbols: &'a mut FxIndexSet<String>,
    pub module_renames: &'a mut FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    pub inlined_stmts: &'a mut Vec<Stmt>,
    /// Import aliases in the current module being inlined (alias -> `actual_name`)
    pub import_aliases: FxIndexMap<String, String>,
    /// Maps imported symbols to their source modules (`local_name` -> `source_module`)
    pub import_sources: FxIndexMap<String, String>,
    /// Python version for compatibility checks
    pub python_version: u8,
}

/// Context for semantic analysis
#[derive(Debug)]
pub struct SemanticContext<'a> {
    pub graph: &'a DependencyGraph,
    pub symbol_registry: &'a SymbolRegistry,
    pub semantic_bundler: &'a SemanticBundler,
}

/// Parameters for the bundling process
///
/// Used by `PhaseOrchestrator::bundle()` to orchestrate all bundling phases.
#[derive(Debug)]
pub struct BundleParams<'a> {
    pub modules: &'a [(crate::resolver::ModuleId, ModModule, String)], // (id, ast, content_hash)
    pub sorted_module_ids: &'a [crate::resolver::ModuleId],            /* Just IDs in dependency
                                                                        * order */
    pub resolver: &'a crate::resolver::ModuleResolver, // To query module info
    pub graph: &'a DependencyGraph,                    /* Dependency graph for unused import
                                                        * detection */
    pub semantic_bundler: &'a SemanticBundler, // Semantic analysis results
    pub circular_dep_analysis: Option<&'a crate::analyzers::types::CircularDependencyAnalysis>, /* Circular dependency analysis */
    pub tree_shaker: Option<&'a crate::tree_shaking::TreeShaker<'a>>, // Tree shaking analysis
    pub python_version: u8,                                           /* Target Python version
                                                                       * for
                                                                       * builtin checks */
}

// ==================== Phase Result Types ====================
// These types represent the outputs of individual bundling phases.
// They define the data contracts between phases in the PhaseOrchestrator.

/// Result from the initialization phase
#[derive(Debug, Clone)]
pub struct InitializationResult {
    /// Future imports collected from all modules
    pub future_imports: FxIndexSet<String>,
}
