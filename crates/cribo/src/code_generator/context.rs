use std::path::{Path, PathBuf};

use ruff_python_ast::{ModModule, Stmt};

use crate::{
    cribo_graph::CriboGraph as DependencyGraph,
    semantic_bundler::{SemanticBundler, SymbolRegistry},
    types::{FxIndexMap, FxIndexSet},
};

/// Represents a hard dependency between classes across modules
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HardDependency {
    /// The module where the class is defined
    pub module_name: String,
    /// The name of the class
    pub class_name: String,
    /// The imported base class (module.attribute format)
    pub base_class: String,
    /// The source module of the base class
    pub source_module: String,
    /// The attribute being imported
    pub imported_attr: String,
    /// The alias used for the import (if any)
    pub alias: Option<String>,
    /// Whether the alias is mandatory to avoid name conflicts
    pub alias_is_mandatory: bool,
}

/// Context for transforming a module
#[derive(Debug)]
pub struct ModuleTransformContext<'a> {
    pub module_name: &'a str,
    pub synthetic_name: &'a str,
    pub module_path: &'a Path,
    pub global_info: Option<crate::semantic_bundler::ModuleGlobalInfo>,
    pub semantic_bundler: Option<&'a SemanticBundler>,
    pub python_version: u8,
}

/// Context for inlining modules
#[derive(Debug)]
pub struct InlineContext<'a> {
    pub module_exports_map: &'a FxIndexMap<String, Option<Vec<String>>>,
    pub global_symbols: &'a mut FxIndexSet<String>,
    pub module_renames: &'a mut FxIndexMap<String, FxIndexMap<String, String>>,
    pub inlined_stmts: &'a mut Vec<Stmt>,
    /// Import aliases in the current module being inlined (alias -> `actual_name`)
    pub import_aliases: FxIndexMap<String, String>,
    /// Deferred import assignments that need to be placed after all modules are inlined
    pub deferred_imports: &'a mut Vec<Stmt>,
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

/// Parameters for processing module globals
#[derive(Debug)]
pub struct ProcessGlobalsParams<'a> {
    pub module_name: &'a str,
    pub ast: &'a ModModule,
    pub semantic_ctx: &'a SemanticContext<'a>,
}

/// Parameters for `bundle_modules` function
#[derive(Debug)]
pub struct BundleParams<'a> {
    pub modules: &'a [(String, ModModule, PathBuf, String)], // (name, ast, path, content_hash)
    pub sorted_modules: &'a [(String, PathBuf, Vec<String>)], // Module data from CriboGraph
    pub entry_module_name: &'a str,
    pub graph: &'a DependencyGraph, // Dependency graph for unused import detection
    pub semantic_bundler: &'a SemanticBundler, // Semantic analysis results
    pub circular_dep_analysis: Option<&'a crate::analyzers::types::CircularDependencyAnalysis>, /* Circular dependency analysis */
    pub tree_shaker: Option<&'a crate::tree_shaking::TreeShaker>, // Tree shaking analysis
    pub python_version: u8,                                       /* Target Python version for
                                                                   * builtin checks */
}
