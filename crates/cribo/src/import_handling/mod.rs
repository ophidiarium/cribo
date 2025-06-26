//! Import handling and classification for Python bundling
//!
//! This module provides the core types and logic for handling Python imports
//! during the bundling process, including classification, transformation, and
//! action determination.

use rustc_hash::FxHashMap;

use crate::cribo_graph::{ItemId, ModuleId};

/// Action-oriented classification of what to do with an import
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportAction {
    /// Remove the import (first-party module that will be inlined)
    Remove,

    /// Preserve the import as-is (stdlib or third-party)
    Preserve,

    /// Transform the import (e.g., for namespace creation)
    Transform(ImportTransformation),

    /// Defer the import (circular dependency resolution)
    Defer(DeferStrategy),
}

/// Specific transformation to apply to an import
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportTransformation {
    /// Convert direct import to namespace object
    /// e.g., `import mymodule` → `mymodule = SimpleNamespace(...)`
    CreateNamespace {
        module_name: String,
        exports: Vec<String>,
    },

    /// Rewrite import path (e.g., for package restructuring)
    RewritePath { new_module: String },

    /// Convert to lazy import pattern
    LazyImport { lazy_var_name: String },
}

/// Strategy for deferring an import
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeferStrategy {
    /// Move import into a function
    MoveToFunction {
        function_item_id: ItemId,
        function_name: String,
    },

    /// Defer until after module initialization
    DeferInit,
}

/// Context for import transformation decisions
#[derive(Debug)]
pub struct ImportContext {
    /// Maps original module names to their bundled representation
    pub module_name_map: FxHashMap<String, String>,

    /// Tracks which symbols are available at module level
    pub available_symbols: FxHashMap<ModuleId, Vec<String>>,

    /// Namespace objects for direct module imports
    pub namespace_exports: FxHashMap<String, Vec<String>>,

    /// Import ordering dependencies
    pub import_order: Vec<(ModuleId, ItemId)>,
}

impl ImportContext {
    /// Create a new empty import context
    pub fn new() -> Self {
        Self {
            module_name_map: FxHashMap::default(),
            available_symbols: FxHashMap::default(),
            namespace_exports: FxHashMap::default(),
            import_order: Vec::new(),
        }
    }

    /// Add a module name mapping
    pub fn add_module_mapping(&mut self, original: String, bundled: String) {
        self.module_name_map.insert(original, bundled);
    }

    /// Add available symbols for a module
    pub fn add_available_symbols(&mut self, module_id: ModuleId, symbols: Vec<String>) {
        self.available_symbols.insert(module_id, symbols);
    }

    /// Add namespace exports for a module
    pub fn add_namespace_exports(&mut self, module_name: String, exports: Vec<String>) {
        self.namespace_exports.insert(module_name, exports);
    }

    /// Get the bundled name for a module
    pub fn get_bundled_name(&self, original: &str) -> Option<&String> {
        self.module_name_map.get(original)
    }

    /// Check if a symbol is available from a module
    pub fn is_symbol_available(&self, module_id: ModuleId, symbol: &str) -> bool {
        self.available_symbols
            .get(&module_id)
            .map(|symbols| symbols.iter().any(|s| s == symbol))
            .unwrap_or(false)
    }
}

impl Default for ImportContext {
    fn default() -> Self {
        Self::new()
    }
}
