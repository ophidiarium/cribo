use std::path::Path;

use ruff_python_ast::{ModModule, Stmt};

use crate::{
    dependency_graph::DependencyGraph,
    symbol_conflict_resolver::{SymbolConflictResolver, SymbolRegistry},
    types::{FxIndexMap, FxIndexSet},
};

/// Context for transforming a module
#[derive(Debug)]
pub(crate) struct ModuleTransformContext<'a> {
    pub module_name: &'a str,
    pub module_path: &'a Path,
    pub global_info: Option<crate::symbol_conflict_resolver::ModuleGlobalInfo>,
    pub conflict_resolver: Option<&'a SymbolConflictResolver>,
    pub python_version: u8,
    /// Whether this module is being transformed as a wrapper function body
    pub is_wrapper_body: bool,
    /// Whether this module is in a circular dependency chain
    pub is_in_circular_deps: bool,
}

/// Context for inlining modules
#[derive(Debug)]
pub(crate) struct InlineContext<'a> {
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
pub(crate) struct SemanticContext<'a> {
    pub graph: &'a DependencyGraph,
    pub symbol_registry: &'a SymbolRegistry,
    pub conflict_resolver: &'a SymbolConflictResolver,
}

/// Parameters for the bundling process
///
/// Used by `PhaseOrchestrator::bundle()` to orchestrate all bundling phases.
#[derive(Debug)]
pub(crate) struct BundleParams<'a> {
    pub modules: &'a [(crate::resolver::ModuleId, ModModule, String)], // (id, ast, content_hash)
    pub sorted_module_ids: &'a [crate::resolver::ModuleId],            /* Just IDs in dependency
                                                                        * order */
    pub resolver: &'a crate::resolver::ModuleResolver, // To query module info
    pub graph: &'a DependencyGraph,                    /* Dependency graph for unused import
                                                        * detection */
    pub conflict_resolver: &'a SymbolConflictResolver, // Symbol conflict resolution
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
pub(crate) struct InitializationResult {
    /// Future imports collected from all modules
    pub future_imports: FxIndexSet<String>,
}

/// Result from the post-processing phase
#[derive(Debug, Clone)]
pub(crate) struct PostProcessingResult {
    /// Proxy statements for stdlib access
    pub proxy_statements: Vec<Stmt>,
    /// Package child alias statements
    pub alias_statements: Vec<Stmt>,
    /// Namespace attachment statements for entry module
    pub namespace_attachments: Vec<Stmt>,
}
