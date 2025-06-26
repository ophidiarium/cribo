//! Bundle plan module containing all bundling decisions
//!
//! The BundlePlan consolidates all bundling decisions from various analysis phases
//! into a single, declarative data structure that drives code generation.

use indexmap::IndexMap;
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap;

use crate::{
    analysis::{AnalysisResults, ResolutionStrategy},
    cribo_graph::{CriboGraph, ItemId, ItemType, ModuleId},
    orchestrator::ModuleRegistry,
    semantic_model_provider::GlobalBindingId,
};

pub mod builder;
pub mod final_layout;

pub use final_layout::{
    FinalBundleLayout, FinalLayoutBuilder, HoistedImportType, NamespaceCreation,
    NamespacePopulationStep,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod symbol_origin_tests;

/// The central plan that consolidates all bundling decisions
#[derive(Debug, Clone, Default)]
pub struct BundlePlan {
    /// Primary driver for the executor - granular execution steps
    /// NOTE: This will be deprecated in favor of final_layout
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

    /// Maps a name within a module to the ModuleId it aliases.
    /// Key: (ModuleId where alias is defined, alias name)
    /// Value: ModuleId of the module being aliased
    /// Example: In greeting.py, "from . import config" creates (greeting_id, "config") ->
    /// config_id
    pub module_aliases: FxHashMap<(ModuleId, String), ModuleId>,

    /// Stdlib imports to hoist to top (populated in Phase 2)
    pub hoisted_imports: Vec<HoistedImport>,

    /// Module-level metadata
    pub module_metadata: FxHashMap<ModuleId, ModuleMetadata>,

    /// Import rewrites for circular dependencies (Phase 1 focus)
    pub import_rewrites: Vec<ImportRewrite>,

    /// Declarative import structure for code generation
    pub final_imports: IndexMap<ModuleId, ModuleFinalImports>,

    /// NEW: Declarative final bundle layout (replaces ExecutionStep approach)
    pub final_layout: FinalBundleLayout,

    /// Rich import classification data from analysis phase
    /// Maps (ModuleId, ItemId) to the classification of that import statement
    pub classified_imports: FxHashMap<(ModuleId, ItemId), ImportClassification>,
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

    /// Create a namespace object for a bundled module
    /// Generates: `<target_name> = SimpleNamespace()`
    CreateModuleNamespace { target_name: String },

    /// Copy a statement from a source module and assign it as an attribute
    /// on a namespace object.
    /// Generates: `<target_object>.<target_attribute> = <copied_statement_rhs>`
    CopyStatementToNamespace {
        from_module: ModuleId,
        item_id: ItemId,
        target_object: String,
        target_attribute: String,
    },

    /// Add a pre-determined stdlib/third-party import statement
    AddImport {
        module_name: String,
        alias: Option<String>,
    },

    /// Add a pre-determined stdlib/third-party from-import statement
    AddFromImport {
        module_name: String,
        symbols: Vec<(String, Option<String>)>, // (name, alias)
        level: u32,                             // 0 for absolute imports
    },

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

/// Classification of an import statement for bundling decisions
#[derive(Debug, Clone)]
pub enum ImportClassification {
    /// An import of a module to be bundled as a namespace object
    /// e.g., `import other` or `import other as o`
    BundleAsNamespace {
        module_id: ModuleId,
        /// The name it will have in the importing module, e.g., "o"
        alias: String,
    },
    /// An import of specific symbols from a module to be bundled
    /// e.g., `from other import x, y as z`
    BundleFromImport {
        module_id: ModuleId,
        symbols: Vec<SymbolImport>,
    },
    /// An import that should be hoisted to the top of the bundle
    /// e.g., `import os` or `from json import loads`
    Hoist { import_type: HoistType },
}

/// Type of import to hoist
#[derive(Debug, Clone)]
pub enum HoistType {
    /// Direct import (import module)
    Direct {
        module_name: String,
        alias: Option<String>,
    },
    /// From import (from module import ...)
    From {
        module_name: String,
        symbols: Vec<(String, Option<String>)>,
        level: u32,
    },
}

/// Symbol import info for BundleFromImport
#[derive(Debug, Clone)]
pub struct SymbolImport {
    pub source_name: String,
    pub target_name: String,
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
        registry: &ModuleRegistry,
        entry_module_name: &str,
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

        // Populate module aliases from import statements
        plan.populate_module_aliases(graph, registry);

        // Classify imports based on graph information
        plan.classify_imports(graph, registry);

        // Classify modules and set metadata
        plan.classify_modules(graph);

        // Get entry module ID
        log::debug!("Looking for entry module '{entry_module_name}' in registry");
        if let Some(entry_module_id) = registry.get_id_by_name(entry_module_name) {
            log::debug!("Found entry module ID: {entry_module_id:?}");
            // Build execution plan from all the decisions
            if let Err(e) = plan.build_execution_plan_v2(graph, entry_module_id) {
                log::error!("Failed to build execution plan v2: {e}");
                // Fallback to old execution plan
                plan.build_execution_plan();
            } else {
                log::info!("Successfully built execution plan v2");
            }
        } else {
            log::error!("Entry module '{entry_module_name}' not found in registry");
            // Fallback: use the old execution plan if entry module not found
            plan.build_execution_plan();
        }

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

    /// Populate module aliases from import statements in the graph
    /// This identifies when a name in a module is an alias for another entire module
    fn populate_module_aliases(&mut self, graph: &CriboGraph, registry: &ModuleRegistry) {
        log::debug!("Populating module aliases from imports");

        // Iterate through all modules
        for (module_id, module_graph) in &graph.modules {
            // Check each item in the module
            for item_data in module_graph.items.values() {
                match &item_data.item_type {
                    // Handle regular imports: import config
                    ItemType::Import { module, alias } => {
                        // Check if this is a first-party module
                        if let Some(target_module_id) = registry.get_id_by_name(module) {
                            // Use alias if present, otherwise use module name
                            let local_name = alias.as_ref().unwrap_or(module);
                            self.module_aliases
                                .insert((*module_id, local_name.clone()), target_module_id);
                            log::trace!(
                                "Added module alias: ({module_id:?}, '{local_name}') -> \
                                 {target_module_id:?}"
                            );
                        }
                    }
                    // Handle from imports: from . import config or from package import submodule
                    ItemType::FromImport {
                        module,
                        names,
                        level,
                        ..
                    } => {
                        // For each imported name, check if it's actually a submodule
                        for (name, alias) in names {
                            let local_name = alias.as_ref().unwrap_or(name);

                            // Construct the full module path
                            let full_module_path = if *level > 0 {
                                // For relative imports, we need to resolve based on current module
                                // This is a simplified version - full implementation would need
                                // to properly resolve relative imports
                                if module.is_empty() {
                                    // from . import config case
                                    let current_module_name = &module_graph.module_name;
                                    log::debug!(
                                        "Processing relative import 'from . import {name}' in \
                                         module '{current_module_name}'"
                                    );
                                    if let Some(parent) = current_module_name.rsplit_once('.') {
                                        let result = format!("{}.{}", parent.0, name);
                                        log::debug!("Resolved to full path: {result}");
                                        result
                                    } else {
                                        log::debug!("No parent found, using name as-is: {name}");
                                        name.clone()
                                    }
                                } else {
                                    // from .submodule import something case
                                    format!("{module}.{name}")
                                }
                            } else {
                                // Absolute import
                                format!("{module}.{name}")
                            };

                            // Check if this full path is a module
                            if let Some(target_module_id) =
                                registry.get_id_by_name(&full_module_path)
                            {
                                self.module_aliases
                                    .insert((*module_id, local_name.clone()), target_module_id);
                                log::debug!(
                                    "Added module alias from 'from' import: ({module_id:?}, \
                                     '{local_name}') -> {target_module_id:?} (full_path: \
                                     {full_module_path})"
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        log::debug!("Populated {} module aliases", self.module_aliases.len());
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

        // TODO: This is a transitional implementation
        // We need to properly classify imports and generate appropriate ExecutionSteps
        // For now, we'll use the existing approach but add a warning

        log::warn!(
            "Using transitional build_execution_plan - import classification not yet integrated"
        );

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

        log::debug!(
            "Built execution plan with {} steps",
            self.execution_plan.len()
        );
    }

    /// Build a proper execution plan using classified imports
    /// This will replace the transitional build_execution_plan above
    pub fn build_execution_plan_v2(
        &mut self,
        graph: &CriboGraph,
        entry_module_id: ModuleId,
    ) -> anyhow::Result<()> {
        use anyhow::anyhow;

        log::debug!("Building execution plan v2 with entry module {entry_module_id:?}");

        // Clear any existing plan
        self.execution_plan.clear();

        // Track which modules need namespace objects
        let mut namespace_modules: FxHashMap<ModuleId, String> = FxHashMap::default();

        // First pass: Scan all imports to determine which modules need namespaces
        log::debug!(
            "Scanning {} modules for namespace requirements",
            self.live_items.len()
        );
        log::debug!(
            "Classified imports: {} total",
            self.classified_imports.len()
        );

        for (module_id, items) in &self.live_items {
            let module_graph = graph
                .modules
                .get(module_id)
                .ok_or_else(|| anyhow!("Module not found in graph: {:?}", module_id))?;

            for item_id in items {
                let _item_data = module_graph
                    .items
                    .get(item_id)
                    .ok_or_else(|| anyhow!("Item not found: {:?}", item_id))?;

                // Check if this is an import that's been classified
                if let Some(classification) = self.classified_imports.get(&(*module_id, *item_id)) {
                    log::trace!(
                        "Found classification for ({module_id:?}, {item_id:?}): {classification:?}"
                    );
                    match classification {
                        ImportClassification::BundleAsNamespace {
                            module_id: imported_module_id,
                            alias,
                        } => {
                            // Record that this module needs a namespace
                            namespace_modules.insert(*imported_module_id, alias.clone());
                        }
                        ImportClassification::BundleFromImport { .. } => {
                            // These will be handled as regular inlined statements
                        }
                        ImportClassification::Hoist { import_type } => {
                            // Generate hoist steps
                            match import_type {
                                HoistType::Direct { module_name, alias } => {
                                    self.execution_plan.push(ExecutionStep::AddImport {
                                        module_name: module_name.clone(),
                                        alias: alias.clone(),
                                    });
                                }
                                HoistType::From {
                                    module_name,
                                    symbols,
                                    level,
                                } => {
                                    self.execution_plan.push(ExecutionStep::AddFromImport {
                                        module_name: module_name.clone(),
                                        symbols: symbols.clone(),
                                        level: *level,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // Second pass: Generate namespace creation steps
        for namespace_name in namespace_modules.values() {
            self.execution_plan
                .push(ExecutionStep::CreateModuleNamespace {
                    target_name: namespace_name.clone(),
                });
        }

        // Third pass: Process all statements
        // TODO: This needs to be in topological order
        for (module_id, items) in &self.live_items {
            let is_entry = *module_id == entry_module_id;
            let is_namespace = namespace_modules.contains_key(module_id);

            for item_id in items {
                // Skip import statements that have been classified
                if self
                    .classified_imports
                    .contains_key(&(*module_id, *item_id))
                {
                    continue;
                }

                if is_entry && !is_namespace {
                    // Entry module statements go to top level
                    self.execution_plan.push(ExecutionStep::InlineStatement {
                        module_id: *module_id,
                        item_id: *item_id,
                    });
                } else if is_namespace {
                    // Namespace module statements need to be assigned as attributes
                    // TODO: We need to extract the symbol name from the item
                    // For now, we'll use InlineStatement and fix this later
                    self.execution_plan.push(ExecutionStep::InlineStatement {
                        module_id: *module_id,
                        item_id: *item_id,
                    });
                }
            }
        }

        log::debug!(
            "Built execution plan v2 with {} steps",
            self.execution_plan.len()
        );

        // Log first few steps for debugging
        for (i, step) in self.execution_plan.iter().take(5).enumerate() {
            log::debug!("  Step {i}: {step:?}");
        }

        Ok(())
    }

    /// Classify imports based on their type and how they should be bundled
    fn classify_imports(&mut self, graph: &CriboGraph, registry: &ModuleRegistry) {
        use crate::resolver::{ImportType, ModuleResolver};

        // Create a temporary resolver for classification
        // TODO: This should be passed from the orchestrator
        let mut resolver = match ModuleResolver::new(crate::config::Config::default()) {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to create resolver for import classification: {e}");
                return;
            }
        };

        // Iterate through all live items to find imports
        for (module_id, items) in &self.live_items {
            let module_graph = match graph.modules.get(module_id) {
                Some(mg) => mg,
                None => continue,
            };

            for item_id in items {
                let item_data = match module_graph.items.get(item_id) {
                    Some(data) => data,
                    None => continue,
                };

                match &item_data.item_type {
                    ItemType::Import { module, alias } => {
                        // Classify the import
                        let import_type = resolver.classify_import(module);

                        let classification = match import_type {
                            ImportType::FirstParty => {
                                // Check if this first-party module exists in our graph
                                if let Some(imported_module_id) = registry.get_id_by_name(module) {
                                    ImportClassification::BundleAsNamespace {
                                        module_id: imported_module_id,
                                        alias: alias.clone().unwrap_or_else(|| module.clone()),
                                    }
                                } else {
                                    // First-party but not discovered - treat as hoist
                                    ImportClassification::Hoist {
                                        import_type: HoistType::Direct {
                                            module_name: module.clone(),
                                            alias: alias.clone(),
                                        },
                                    }
                                }
                            }
                            ImportType::StandardLibrary | ImportType::ThirdParty => {
                                ImportClassification::Hoist {
                                    import_type: HoistType::Direct {
                                        module_name: module.clone(),
                                        alias: alias.clone(),
                                    },
                                }
                            }
                        };

                        self.classified_imports
                            .insert((*module_id, *item_id), classification);
                    }
                    ItemType::FromImport {
                        module,
                        names,
                        level,
                        is_star,
                    } => {
                        // Skip __future__ imports - they're handled specially
                        if module == "__future__" {
                            continue;
                        }

                        // Handle relative imports
                        let effective_module = if *level > 0 {
                            // TODO: Resolve relative imports properly
                            module.clone()
                        } else {
                            module.clone()
                        };

                        if *is_star {
                            // Star imports are always hoisted for now
                            let classification = ImportClassification::Hoist {
                                import_type: HoistType::From {
                                    module_name: effective_module,
                                    symbols: vec![("*".to_string(), None)],
                                    level: *level,
                                },
                            };
                            self.classified_imports
                                .insert((*module_id, *item_id), classification);
                            continue;
                        }

                        // Classify the import
                        let import_type = resolver.classify_import(&effective_module);

                        let classification = match import_type {
                            ImportType::FirstParty => {
                                // Check if we're importing the module itself or symbols from it
                                if let Some(imported_module_id) =
                                    registry.get_id_by_name(&effective_module)
                                {
                                    // Check if all names are submodules
                                    let mut all_submodules = true;
                                    for (name, _) in names {
                                        let full_name = format!("{effective_module}.{name}");
                                        if registry.get_id_by_name(&full_name).is_none() {
                                            all_submodules = false;
                                            break;
                                        }
                                    }

                                    if all_submodules {
                                        // These are submodule imports, handle specially
                                        // For now, treat as symbol imports
                                        ImportClassification::BundleFromImport {
                                            module_id: imported_module_id,
                                            symbols: names
                                                .iter()
                                                .map(|(name, alias)| SymbolImport {
                                                    source_name: name.clone(),
                                                    target_name: alias
                                                        .clone()
                                                        .unwrap_or_else(|| name.clone()),
                                                })
                                                .collect(),
                                        }
                                    } else {
                                        // Regular symbol imports from first-party module
                                        ImportClassification::BundleFromImport {
                                            module_id: imported_module_id,
                                            symbols: names
                                                .iter()
                                                .map(|(name, alias)| SymbolImport {
                                                    source_name: name.clone(),
                                                    target_name: alias
                                                        .clone()
                                                        .unwrap_or_else(|| name.clone()),
                                                })
                                                .collect(),
                                        }
                                    }
                                } else {
                                    // First-party but not discovered - treat as hoist
                                    ImportClassification::Hoist {
                                        import_type: HoistType::From {
                                            module_name: effective_module,
                                            symbols: names.clone(),
                                            level: *level,
                                        },
                                    }
                                }
                            }
                            ImportType::StandardLibrary | ImportType::ThirdParty => {
                                ImportClassification::Hoist {
                                    import_type: HoistType::From {
                                        module_name: effective_module,
                                        symbols: names.clone(),
                                        level: *level,
                                    },
                                }
                            }
                        };

                        self.classified_imports
                            .insert((*module_id, *item_id), classification);
                    }
                    _ => {
                        // Not an import
                    }
                }
            }
        }

        log::debug!("Classified {} imports", self.classified_imports.len());
    }
}
