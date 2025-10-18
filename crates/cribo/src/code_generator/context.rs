use std::path::{Path, PathBuf};

use ruff_python_ast::{ModModule, Stmt};

use crate::{
    cribo_graph::CriboGraph as DependencyGraph,
    resolver::ModuleId,
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

/// Parameters for `bundle_modules` function
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
// These types represent the outputs of individual bundling phases
// to support decomposing the monolithic bundle_modules function.
//
// Note: These types are currently unused but will be used as the refactoring progresses.
// They are defined here first to establish the architecture.

/// Result from the initialization phase
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InitializationResult {
    /// Future imports collected from all modules
    pub future_imports: FxIndexSet<String>,
    /// Circular modules identified
    pub circular_modules: FxIndexSet<ModuleId>,
    /// Modules imported as namespaces (module -> set of imported modules)
    pub namespace_imported_modules: FxIndexMap<ModuleId, FxIndexSet<ModuleId>>,
}

/// Result from the module preparation phase
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PreparationResult {
    /// Prepared modules: `ModuleId` -> (AST, Path, `ContentHash`)
    pub modules: FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
}

/// Result from the symbol rename collection phase
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SymbolRenameResult {
    /// Symbol renames per module: `ModuleId` -> (`OriginalName` -> `RenamedName`)
    pub symbol_renames: FxIndexMap<ModuleId, FxIndexMap<String, String>>,
}

/// Result from the global symbol collection phase
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct GlobalSymbolResult {
    /// Global symbols: `ModuleId` -> Set of global symbol names
    pub global_symbols: FxIndexMap<ModuleId, FxIndexSet<String>>,
}

/// Result from the circular dependency analysis phase
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CircularDependencyResult {
    /// Cycle groups (SCCs)
    pub cycle_groups: Vec<Vec<ModuleId>>,
    /// Mapping from module to its cycle group index
    pub member_to_group: FxIndexMap<ModuleId, usize>,
    /// Wrapper modules needed by inlined modules (with transitive deps)
    pub all_needed_wrappers: FxIndexSet<ModuleId>,
    /// Whether any wrapper module participates in circular dependencies
    pub has_circular_wrapped_modules: bool,
}

/// Result from the main processing phase
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ProcessingResult {
    /// All inlined statements from modules (excluding entry module)
    pub inlined_statements: Vec<Stmt>,
    /// Modules that were processed
    pub processed_modules: FxIndexSet<ModuleId>,
}

/// Result from the entry module processing phase
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EntryModuleResult {
    /// Entry module statements (transformed and deduplicated)
    pub entry_statements: Vec<Stmt>,
    /// Locally defined symbols in entry module
    pub entry_module_symbols: FxIndexSet<String>,
    /// Entry module renames
    pub entry_module_renames: FxIndexMap<String, String>,
}

/// Result from the post-processing phase
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PostProcessingResult {
    /// Proxy statements for stdlib access
    pub proxy_statements: Vec<Stmt>,
    /// Package child alias statements
    pub alias_statements: Vec<Stmt>,
}
