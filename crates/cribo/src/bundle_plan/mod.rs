//! Bundle plan module containing all bundling decisions
//!
//! The BundlePlan consolidates all bundling decisions from various analysis phases
//! into a single, declarative data structure that drives code generation.

use indexmap::IndexMap;
use rustc_hash::FxHashMap;

use crate::cribo_graph::{ItemId, ModuleId};

pub mod builder;

#[cfg(test)]
mod tests;

/// The central plan that consolidates all bundling decisions
#[derive(Debug, Clone, Default)]
pub struct BundlePlan {
    /// Statement ordering for final bundle (populated in Phase 2)
    pub final_statement_order: Vec<(ModuleId, ItemId)>,

    /// Live code tracking for tree-shaking (populated in Phase 2)
    pub live_items: FxHashMap<ModuleId, Vec<ItemId>>,

    /// Symbol renaming decisions (populated in Phase 2)
    pub symbol_renames: IndexMap<(ModuleId, String), String>,

    /// Stdlib imports to hoist to top (populated in Phase 2)
    pub hoisted_imports: Vec<HoistedImport>,

    /// Module-level metadata
    pub module_metadata: FxHashMap<ModuleId, ModuleMetadata>,

    /// Import rewrites for circular dependencies (Phase 1 focus)
    pub import_rewrites: Vec<ImportRewrite>,
}

/// Metadata about how a module should be bundled
#[derive(Debug, Clone)]
pub struct ModuleMetadata {
    pub bundle_type: ModuleBundleType,
    pub has_side_effects: bool,
    pub synthetic_namespace: Option<Vec<String>>,
}

/// How a module should be bundled
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleBundleType {
    /// Can merge into global scope
    Inlinable,
    /// Must keep in init function
    Wrapper,
    /// Has conditional logic
    Conditional,
}

/// A stdlib import to hoist
#[derive(Debug, Clone)]
pub struct HoistedImport {
    pub module_name: String,
    pub alias: Option<String>,
    pub symbols: Option<Vec<String>>,
}

/// Instructions for rewriting an import
#[derive(Debug, Clone)]
pub struct ImportRewrite {
    /// The module containing the import
    pub module_id: ModuleId,
    /// The specific import item to rewrite
    pub import_item_id: ItemId,
    /// The rewrite action to take
    pub action: ImportRewriteAction,
}

/// Specific action to take when rewriting an import
#[derive(Debug, Clone)]
pub enum ImportRewriteAction {
    /// Move import into a function
    MoveToFunction {
        /// Target function item ID
        function_item_id: ItemId,
        /// Name of the function (for debugging)
        function_name: String,
    },
    /// Defer import until after module initialization
    DeferInit,
    /// Convert to lazy import pattern
    LazyImport {
        /// Variable name for lazy import
        lazy_var_name: String,
    },
}

impl BundlePlan {
    /// Create a new empty bundle plan
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an import rewrite instruction
    pub fn add_import_rewrite(&mut self, rewrite: ImportRewrite) {
        self.import_rewrites.push(rewrite);
    }

    /// Set module metadata
    pub fn set_module_metadata(&mut self, module_id: ModuleId, metadata: ModuleMetadata) {
        self.module_metadata.insert(module_id, metadata);
    }

    /// Get import rewrites for a specific module
    pub fn get_module_import_rewrites(&self, module_id: ModuleId) -> Vec<&ImportRewrite> {
        self.import_rewrites
            .iter()
            .filter(|r| r.module_id == module_id)
            .collect()
    }
}
