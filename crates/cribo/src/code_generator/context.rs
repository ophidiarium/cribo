use std::path::{Path, PathBuf};

use indexmap::{IndexMap as FxIndexMap, IndexSet as FxIndexSet};
use ruff_python_ast::{ModModule, Stmt};

use crate::{
    cribo_graph::CriboGraph as DependencyGraph,
    semantic_bundler::{ModuleGlobalInfo, SemanticBundler, SymbolRegistry},
};

/// Represents a hard dependency between classes across modules
#[derive(Debug, Clone)]
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
    pub global_info: Option<ModuleGlobalInfo>,
    pub semantic_bundler: Option<&'a SemanticBundler>,
}

/// Context for inlining modules
pub struct InlineContext<'a> {
    pub module_exports_map: &'a FxIndexMap<String, Option<Vec<String>>>,
    pub global_symbols: &'a mut FxIndexSet<String>,
    pub module_renames: &'a mut FxIndexMap<String, FxIndexMap<String, String>>,
    pub inlined_stmts: &'a mut Vec<Stmt>,
    /// Import aliases in the current module being inlined (alias -> actual_name)
    pub import_aliases: FxIndexMap<String, String>,
    /// Deferred import assignments that need to be placed after all modules are inlined
    pub deferred_imports: &'a mut Vec<Stmt>,
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

/// Context for handling direct imports
#[derive(Debug)]
pub struct DirectImportContext<'a> {
    pub current_module: &'a str,
    pub module_path: &'a Path,
    pub modules: &'a [(String, ModModule, PathBuf, String)],
}

/// Parameters for transforming functions with globals
#[derive(Debug)]
pub struct TransformFunctionParams<'a> {
    pub lifted_names: &'a FxIndexMap<String, String>,
    pub global_info: &'a ModuleGlobalInfo,
    pub function_globals: &'a FxIndexSet<String>,
}

/// Parameters for bundle_modules function
pub struct BundleParams<'a> {
    pub modules: Vec<(String, ModModule, PathBuf, String)>, // (name, ast, path, content_hash)
    pub sorted_modules: &'a [(String, PathBuf, Vec<String>)], // Module data from CriboGraph
    pub entry_module_name: &'a str,
    pub graph: &'a DependencyGraph, // Dependency graph for unused import detection
    pub semantic_bundler: &'a SemanticBundler, // Semantic analysis results
    pub circular_dep_analysis: Option<&'a crate::cribo_graph::CircularDependencyAnalysis>, /* Circular dependency analysis */
    pub tree_shaker: Option<&'a crate::tree_shaking::TreeShaker>, // Tree shaking analysis
}
