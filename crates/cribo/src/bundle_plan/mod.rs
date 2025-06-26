//! Bundle plan module containing all bundling decisions
//!
//! The BundlePlan consolidates all bundling decisions from various analysis phases
//! into a single, declarative data structure that drives code generation.

use indexmap::IndexMap;
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap;

use crate::{
    analysis::{AnalysisResults, ResolutionStrategy},
    cribo_graph::{CriboGraph, ItemId, ModuleId},
    orchestrator::ModuleRegistry,
    semantic_model_provider::GlobalBindingId,
};

pub mod builder;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod symbol_origin_tests;

/// The central plan that consolidates all bundling decisions
#[derive(Debug, Clone, Default)]
pub struct BundlePlan {
    /// Primary driver for the executor - granular execution steps
    pub execution_plan: Vec<ExecutionStep>,

    /// Statement ordering for final bundle (populated in Phase 2)
    pub final_statement_order: Vec<(ModuleId, ItemId)>,

    /// Live code tracking for tree-shaking (populated in Phase 2)
    pub live_items: FxHashMap<ModuleId, Vec<ItemId>>,

    /// Symbol renaming decisions (populated in Phase 2)
    pub symbol_renames: IndexMap<GlobalBindingId, String>,

    /// AST node renaming map for code generator (populated from symbol_renames)
    /// Maps (ModuleId, TextRange) to the new name for that AST node
    pub ast_node_renames: FxHashMap<(ModuleId, TextRange), String>,

    /// Maps the GlobalBindingId of an imported/re-exported symbol to the
    /// GlobalBindingId of its original definition. This is the key to
    /// tracking symbol identity across modules.
    pub symbol_origins: FxHashMap<GlobalBindingId, GlobalBindingId>,

    /// Stdlib imports to hoist to top (populated in Phase 2)
    pub hoisted_imports: Vec<HoistedImport>,

    /// Module-level metadata
    pub module_metadata: FxHashMap<ModuleId, ModuleMetadata>,

    /// Import rewrites for circular dependencies (Phase 1 focus)
    pub import_rewrites: Vec<ImportRewrite>,

    /// Declarative import structure for code generation
    pub final_imports: IndexMap<ModuleId, ModuleFinalImports>,
}

/// Final import structure for a module after all transformations
#[derive(Debug, Clone, Default)]
pub struct ModuleFinalImports {
    /// Direct imports (import module)
    pub direct_imports: Vec<DirectImport>,
    /// From imports (from module import ...)
    pub from_imports: Vec<FromImport>,
}

/// A direct import after transformations
#[derive(Debug, Clone)]
pub struct DirectImport {
    pub module: String,
    pub alias: Option<String>,
}

/// A from import after transformations
#[derive(Debug, Clone)]
pub struct FromImport {
    pub module: String,
    pub symbols: IndexMap<String, Option<String>>, // symbol -> alias
    pub level: u32,                                // relative import level
}

/// How a module should be instantiated in the bundle
#[derive(Debug, Clone, Default)]
pub enum ModuleInstantiation {
    /// Default: statements inserted directly into bundle
    #[default]
    Inline,
    /// Module wrapped in init function with exports
    Wrap {
        init_function_name: String,
        exports: Vec<String>, // Pre-computed by analysis
    },
}

/// Metadata about how a module should be bundled
#[derive(Debug, Clone)]
pub struct ModuleMetadata {
    pub instantiation: ModuleInstantiation,
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

/// Granular execution steps for the dumb executor
#[derive(Debug, Clone)]
pub enum ExecutionStep {
    /// Hoist a `from __future__ import ...` statement
    HoistFutureImport { name: String },

    /// Hoist a standard library import
    HoistStdlibImport { name: String },

    /// Define the init function for a wrapped module
    DefineInitFunction { module_id: ModuleId },

    /// Create the module object by calling its init function
    CallInitFunction {
        module_id: ModuleId,
        target_variable: String,
    },

    /// Directly inline a statement from a source module
    InlineStatement {
        module_id: ModuleId,
        item_id: ItemId,
    },
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

        // Add symbol origin mappings from analysis
        plan.symbol_origins = results.symbol_origins.symbol_origins.clone();

        // Convert symbol conflicts to rename decisions
        plan.add_symbol_renames(&results.symbol_conflicts);

        // Convert tree-shaking results to live items
        if let Some(tree_shake) = &results.tree_shake_results {
            plan.add_tree_shake_decisions(tree_shake);
        } else {
            // Fallback: If no tree-shaking results, include all items from all modules
            log::debug!("No tree-shaking results, including all items");
            for (module_id, module_graph) in &graph.modules {
                let items: Vec<_> = module_graph.items.keys().cloned().collect();
                plan.live_items.insert(*module_id, items);
            }
        }

        // Classify modules and set metadata
        plan.classify_modules(graph);

        // Build execution plan from all the decisions
        plan.build_execution_plan();

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
            // Skip the first instance - it keeps the original name
            for (_idx, instance) in conflict.conflicts.iter().enumerate().skip(1) {
                // Generate rename using module suffix
                let module_suffix = instance.module_name.replace(['.', '-'], "_");
                let new_name = format!("{}_{}", conflict.symbol_name, module_suffix);

                // Add rename decision
                self.symbol_renames
                    .insert(instance.global_id, new_name.clone());

                log::debug!(
                    "Renaming symbol '{}' in module '{}' to '{}'",
                    conflict.symbol_name,
                    instance.module_name,
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

            // Determine instantiation based on bundle type
            let instantiation = match bundle_type {
                ModuleBundleType::Wrapper | ModuleBundleType::Conditional => {
                    // TODO: Generate proper init function name and exports list
                    ModuleInstantiation::Wrap {
                        init_function_name: format!("__cribo_init_{module_id:?}"),
                        exports: vec![], // Will be populated by analysis
                    }
                }
                ModuleBundleType::Inlinable => ModuleInstantiation::Inline,
            };

            self.set_module_metadata(
                *module_id,
                ModuleMetadata {
                    instantiation,
                    bundle_type,
                    has_side_effects,
                    synthetic_namespace: None, // Will be set during code generation if needed
                },
            );
        }
    }

    /// Populate the ast_node_renames map from symbol_renames using semantic models
    /// This must be called after symbol_renames is populated
    pub fn populate_ast_node_renames(
        &mut self,
        semantic_provider: &crate::semantic_model_provider::SemanticModelProvider,
    ) {
        // Clear any existing entries
        self.ast_node_renames.clear();

        // Iterate through all symbol rename decisions
        for (global_binding_id, new_name) in &self.symbol_renames {
            let module_id = global_binding_id.module_id;
            let binding_id = global_binding_id.binding_id;

            // Get the semantic model for this module
            if let Some(Ok(semantic_model)) = semantic_provider.get_model(module_id) {
                // Get the binding information
                let binding = semantic_model.binding(binding_id);

                // 1. Add the definition itself to the rename map
                self.ast_node_renames
                    .insert((module_id, binding.range), new_name.clone());

                // 2. Add all references to the rename map
                for reference_id in &binding.references {
                    let reference = semantic_model.reference(*reference_id);
                    self.ast_node_renames
                        .insert((module_id, reference.range()), new_name.clone());
                }

                log::trace!(
                    "Added {} AST node renames for symbol '{}' in module {:?}",
                    binding.references.len() + 1,
                    new_name,
                    module_id
                );
            }
        }

        log::debug!(
            "Populated {} AST node renames from {} symbol renames",
            self.ast_node_renames.len(),
            self.symbol_renames.len()
        );
    }

    /// Build the execution plan from all the collected decisions
    fn build_execution_plan(&mut self) {
        // Clear any existing plan
        self.execution_plan.clear();

        // TODO: Add hoisted future imports
        // For now, we don't have a way to detect them yet

        // TODO: Add hoisted stdlib imports
        // For now, we don't have a way to detect them yet

        // Add statements from live_items if final_statement_order is empty
        if self.final_statement_order.is_empty() && !self.live_items.is_empty() {
            // Build statement order from live_items
            // Sort items by their statement index to preserve original order
            let mut all_items = Vec::new();
            for (module_id, items) in &self.live_items {
                for item_id in items {
                    all_items.push((*module_id, *item_id));
                }
            }

            // Sort by statement index (for now, use ItemId as proxy for order)
            // TODO: This should use actual statement indices once we have proper graph access
            all_items.sort_by_key(|(_, item_id)| item_id.as_u32());

            for (module_id, item_id) in all_items {
                self.execution_plan
                    .push(ExecutionStep::InlineStatement { module_id, item_id });
            }
        } else {
            // Use final_statement_order if available
            let statement_order: Vec<_> = self.final_statement_order.clone();
            for (module_id, item_id) in statement_order {
                self.execution_plan
                    .push(ExecutionStep::InlineStatement { module_id, item_id });
            }
        }

        // TODO: Add wrapped module support
        // This will involve:
        // 1. DefineInitFunction for each wrapped module
        // 2. CallInitFunction in the correct order

        log::debug!(
            "Built execution plan with {} steps",
            self.execution_plan.len()
        );
    }
}
