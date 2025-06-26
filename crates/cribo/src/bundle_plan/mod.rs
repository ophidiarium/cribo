//! Bundle plan module containing all bundling decisions
//!
//! The BundlePlan consolidates all bundling decisions from various analysis phases
//! into a single, declarative data structure that drives code generation.

use indexmap::IndexMap;
use rustc_hash::FxHashMap;

use crate::{
    analysis::{AnalysisResults, ResolutionStrategy},
    cribo_graph::{CriboGraph, ItemId, ModuleId},
    orchestrator::ModuleRegistry,
};

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

    /// Build a BundlePlan from analysis results
    ///
    /// This is the main assembly method that converts all analysis results
    /// into a consolidated plan for code generation.
    pub fn from_analysis_results(
        graph: &CriboGraph,
        results: &AnalysisResults,
        _registry: &ModuleRegistry,
    ) -> Self {
        let mut plan = Self::new();

        // Convert circular dependency analysis to import rewrites
        if let Some(circular_deps) = &results.circular_deps {
            plan.add_circular_dep_rewrites(graph, circular_deps);
        }

        // Convert symbol conflicts to rename decisions
        plan.add_symbol_renames(&results.symbol_conflicts);

        // Convert tree-shaking results to live items
        if let Some(tree_shake) = &results.tree_shake_results {
            plan.add_tree_shake_decisions(tree_shake);
        }

        // Classify modules and set metadata
        plan.classify_modules(graph);

        plan
    }

    /// Add import rewrites based on circular dependency analysis
    fn add_circular_dep_rewrites(
        &mut self,
        graph: &CriboGraph,
        circular_deps: &crate::analysis::CircularDependencyAnalysis,
    ) {
        for cycle in &circular_deps.resolvable_cycles {
            match &cycle.suggested_resolution {
                ResolutionStrategy::FunctionScopedImport {
                    import_to_function,
                    descriptions,
                } => {
                    // Convert the resolution strategy to import rewrites
                    for (import_item_id, function_item_id) in import_to_function {
                        // Find the module containing this import
                        if let Some(module_id) =
                            self.find_module_containing_item(graph, *import_item_id)
                        {
                            let function_name = descriptions
                                .iter()
                                .find(|desc| desc.contains(&format!("{function_item_id:?}")))
                                .cloned()
                                .unwrap_or_else(|| format!("function_{function_item_id:?}"));

                            self.add_import_rewrite(ImportRewrite {
                                module_id,
                                import_item_id: *import_item_id,
                                action: ImportRewriteAction::MoveToFunction {
                                    function_item_id: *function_item_id,
                                    function_name,
                                },
                            });
                        }
                    }
                }
                ResolutionStrategy::LazyImport {
                    module_ids,
                    lazy_var_names,
                } => {
                    // Handle lazy import pattern
                    for (module_id, var_name) in module_ids.iter().zip(lazy_var_names.values()) {
                        // Find imports in this module that need to be made lazy
                        if let Some(module_graph) = graph.modules.get(module_id) {
                            for (item_id, item_data) in &module_graph.items {
                                if matches!(
                                    item_data.item_type,
                                    crate::cribo_graph::ItemType::Import { .. }
                                        | crate::cribo_graph::ItemType::FromImport { .. }
                                ) {
                                    self.add_import_rewrite(ImportRewrite {
                                        module_id: *module_id,
                                        import_item_id: *item_id,
                                        action: ImportRewriteAction::LazyImport {
                                            lazy_var_name: var_name.clone(),
                                        },
                                    });
                                }
                            }
                        }
                    }
                }
                ResolutionStrategy::ModuleSplit { .. } => {
                    // Module splitting is a more complex refactoring
                    // For now, we'll skip this strategy
                    log::debug!("Module split strategy not yet implemented");
                }
                ResolutionStrategy::Unresolvable { .. } => {
                    // Nothing to do for unresolvable cycles
                }
            }
        }
    }

    /// Find which module contains a given item
    fn find_module_containing_item(&self, graph: &CriboGraph, item_id: ItemId) -> Option<ModuleId> {
        for (module_id, module_graph) in &graph.modules {
            if module_graph.items.contains_key(&item_id) {
                return Some(*module_id);
            }
        }
        None
    }

    /// Add symbol renames based on conflict analysis
    fn add_symbol_renames(&mut self, symbol_conflicts: &[crate::analysis::SymbolConflict]) {
        for conflict in symbol_conflicts {
            // For now, use a simple numbering strategy
            // In the future, this could be more sophisticated
            for (idx, module_name) in conflict.defining_modules.iter().enumerate().skip(1) {
                // Keep the first module's symbol name unchanged
                // Rename subsequent ones with a suffix
                let new_name = format!("{}_{}", conflict.symbol_name, idx);

                // TODO: Convert module name to ModuleId
                // For now, we'll need to enhance this when we have access to the mapping
                log::debug!(
                    "Would rename symbol '{}' in module '{}' to '{}'",
                    conflict.symbol_name,
                    module_name,
                    new_name
                );
            }
        }
    }

    /// Add tree-shaking decisions
    fn add_tree_shake_decisions(&mut self, tree_shake: &crate::analysis::TreeShakeResults) {
        // Group included items by module
        for (module_id, item_id) in &tree_shake.included_items {
            self.live_items
                .entry(*module_id)
                .or_default()
                .push(*item_id);
        }

        // Note: removed_items and removed_modules are informational
        // The absence of items in live_items implies they should be removed
    }

    /// Classify modules and set their metadata
    fn classify_modules(&mut self, graph: &CriboGraph) {
        for (module_id, module_graph) in &graph.modules {
            let has_side_effects = module_graph
                .items
                .values()
                .any(|item| item.has_side_effects);

            let has_conditional_logic = module_graph.items.values().any(|item| {
                matches!(
                    item.item_type,
                    crate::cribo_graph::ItemType::If { .. } | crate::cribo_graph::ItemType::Try
                )
            });

            let bundle_type = if has_conditional_logic {
                ModuleBundleType::Conditional
            } else if has_side_effects {
                ModuleBundleType::Wrapper
            } else {
                ModuleBundleType::Inlinable
            };

            self.set_module_metadata(
                *module_id,
                ModuleMetadata {
                    bundle_type,
                    has_side_effects,
                    synthetic_namespace: None, // Will be set during code generation if needed
                },
            );
        }
    }
}
