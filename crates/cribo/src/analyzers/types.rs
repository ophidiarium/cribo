//! Common types used across analyzers
//!
//! This module contains shared type definitions for analysis results
//! and intermediate data structures used by various analyzers.

use ruff_text_size::TextRange;

use crate::types::{FxIndexMap, FxIndexSet};

/// Represents a scope path in the AST (e.g., module.function.class)
pub type ScopePath = Vec<String>;

/// Information about a defined symbol in the code
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolInfo {
    /// The name of the symbol
    pub name: String,
    /// The type of symbol (function, class, variable, etc.)
    pub kind: SymbolKind,
    /// The scope where this symbol is defined
    pub scope: ScopePath,
    /// Whether this symbol is exported (in __all__ or public)
    pub is_exported: bool,
    /// Whether this symbol is declared as global
    pub is_global: bool,
    /// The text range where this symbol is defined
    pub definition_range: TextRange,
}

/// Different kinds of symbols that can be defined
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    /// A function definition
    Function {
        /// Decorator names applied to the function
        decorators: Vec<String>,
    },
    /// A class definition
    Class {
        /// Base class names
        bases: Vec<String>,
    },
    /// A variable assignment
    Variable {
        /// Whether this appears to be a constant (UPPER_CASE naming)
        is_constant: bool,
    },
    /// An import statement
    Import {
        /// The module being imported from
        module: String,
    },
}

/// Collection of symbols found in a module
#[derive(Debug, Default)]
pub struct CollectedSymbols {
    /// Global symbols mapped by name
    pub global_symbols: FxIndexMap<String, SymbolInfo>,
    /// Symbols organized by their scope
    pub scoped_symbols: FxIndexMap<ScopePath, Vec<SymbolInfo>>,
    /// Module-level renames from imports (alias -> actual_name)
    pub module_renames: FxIndexMap<String, String>,
}

/// Information about variable usage in the code
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableUsage {
    /// The name of the variable
    pub name: String,
    /// How the variable is being used
    pub usage_type: UsageType,
    /// Where this usage occurs
    pub location: TextRange,
    /// The scope containing this usage (dot-separated path)
    pub scope: String,
}

/// Different ways a variable can be used
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageType {
    /// Reading the value of a variable
    Read,
    /// Assigning a new value to a variable
    Write,
    /// Deleting a variable
    Delete,
    /// Declaring a variable as global
    GlobalDeclaration,
    /// Declaring a variable as nonlocal
    NonlocalDeclaration,
}

/// Collection of variable usage information
#[derive(Debug, Default)]
pub struct CollectedVariables {
    /// All variable usages in the module
    pub usages: Vec<VariableUsage>,
    /// Functions and their global variable declarations
    pub function_globals: FxIndexMap<String, FxIndexSet<String>>,
    /// All variables that are referenced (read) in the module
    pub referenced_vars: FxIndexSet<String>,
}

/// Information about module exports
#[derive(Debug, Clone)]
pub struct ExportInfo {
    /// Explicitly exported names via __all__ (None means export all public symbols)
    pub exported_names: Option<Vec<String>>,
    /// Whether __all__ is modified dynamically
    pub is_dynamic: bool,
    /// Re-exports from other modules
    pub re_exports: Vec<ReExport>,
}

/// Represents a re-export from another module
#[derive(Debug, Clone)]
pub struct ReExport {
    /// The module being imported from
    pub from_module: String,
    /// Names being imported and their aliases (name, alias)
    pub names: Vec<(String, Option<String>)>,
    /// Whether this is a star import
    pub is_star: bool,
}

/// Result of symbol analysis
#[derive(Debug)]
pub struct SymbolAnalysis {
    /// All collected symbols
    pub symbols: CollectedSymbols,
    /// Variable usage information
    pub variables: CollectedVariables,
    /// Export information
    pub exports: Option<ExportInfo>,
    /// Symbol dependency relationships
    pub symbol_dependencies: FxIndexMap<String, FxIndexSet<String>>,
}

/// Represents a dependency between symbols
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolDependency {
    /// The symbol that depends on another
    pub from_symbol: String,
    /// The symbol being depended upon
    pub to_symbol: String,
    /// The module containing the from_symbol
    pub from_module: String,
    /// The module containing the to_symbol
    pub to_module: String,
}

/// Result of dependency analysis
#[derive(Debug)]
pub struct DependencyAnalysis {
    /// Direct module dependencies
    pub module_dependencies: FxIndexMap<String, FxIndexSet<String>>,
    /// Symbol-level dependencies
    pub symbol_dependencies: Vec<SymbolDependency>,
    /// Circular dependency groups
    pub circular_groups: Vec<FxIndexSet<String>>,
    /// Hard dependencies (e.g., base class dependencies)
    pub hard_dependencies: Vec<crate::code_generator::context::HardDependency>,
}

/// Result of import analysis
#[derive(Debug)]
pub struct ImportAnalysis {
    /// Modules directly imported (import module)
    pub directly_imported: FxIndexSet<String>,
    /// Modules imported as namespaces (from package import module)
    pub namespace_imported: FxIndexMap<String, FxIndexSet<String>>,
    /// Import aliases mapping
    pub import_aliases: FxIndexMap<String, String>,
    /// Unused imports
    pub unused_imports: FxIndexSet<(String, String)>,
}

/// Information about an unused import
#[derive(Debug, Clone)]
pub struct UnusedImportInfo {
    /// The imported name that is unused
    pub name: String,
    /// The module it was imported from
    pub module: String,
}

/// Result of namespace analysis
#[derive(Debug)]
pub struct NamespaceAnalysis {
    /// Required namespace modules
    pub required_namespaces: FxIndexSet<String>,
    /// Namespace hierarchy (parent -> children)
    pub namespace_hierarchy: FxIndexMap<String, FxIndexSet<String>>,
    /// Modules that need namespace objects
    pub modules_needing_namespaces: FxIndexSet<String>,
}

/// Type of circular dependency
#[derive(Debug, Clone, PartialEq)]
pub enum CircularDependencyType {
    /// Can be resolved by moving imports inside functions
    FunctionLevel,
    /// May be resolvable depending on usage patterns
    ClassLevel,
    /// Unresolvable - temporal paradox
    ModuleConstants,
    /// Depends on execution order
    ImportTime,
}

/// Resolution strategy for circular dependencies
#[derive(Debug, Clone)]
pub enum ResolutionStrategy {
    LazyImport,
    FunctionScopedImport,
    ModuleSplit,
    Unresolvable { reason: String },
}

/// A group of modules forming a circular dependency
#[derive(Debug, Clone)]
pub struct CircularDependencyGroup {
    pub modules: Vec<String>,
    pub cycle_type: CircularDependencyType,
    pub suggested_resolution: ResolutionStrategy,
}

/// Comprehensive analysis of circular dependencies
#[derive(Debug, Clone)]
pub struct CircularDependencyAnalysis {
    /// Circular dependencies that can be resolved through code transformations
    pub resolvable_cycles: Vec<CircularDependencyGroup>,
    /// Circular dependencies that cannot be resolved
    pub unresolvable_cycles: Vec<CircularDependencyGroup>,
}
