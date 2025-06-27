//! Bundle plan module containing all bundling decisions
//!
//! The BundlePlan consolidates all bundling decisions from various analysis phases
//! into a single, declarative data structure that drives code generation.

use indexmap::IndexMap;
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap;

use crate::{
    analysis::{AnalysisResults, ResolutionStrategy},
    ast_builder,
    cribo_graph::{CriboGraph, ItemId, ItemType, ModuleId},
    module_registry::ModuleRegistry,
    semantic_model_provider::GlobalBindingId,
};

pub mod builder;
pub mod compiler;
pub mod final_layout;

pub use compiler::{BundleCompiler, BundleProgram};
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
    pub symbols: Vec<(String, Option<String>)>,
    pub level: u32,
}

/// An import rewrite for circular dependency resolution
#[derive(Debug, Clone)]
pub struct ImportRewrite {
    pub module_id: ModuleId,
    pub import_item_id: ItemId,
    pub action: ImportRewriteAction,
}

/// Action to take for an import rewrite
#[derive(Debug, Clone)]
pub enum ImportRewriteAction {
    /// Move import to function scope
    MoveToFunction {
        function_item_id: ItemId,
        function_name: String,
    },
    /// Convert to lazy import pattern
    LazyImport { lazy_var_name: String },
}

/// Module-level metadata for bundling decisions
#[derive(Debug, Clone, Default)]
pub struct ModuleMetadata {
    /// Whether this module needs to be wrapped in an init function
    pub needs_init_wrapper: bool,
    /// Resolution strategy for this module
    pub resolution_strategy: Option<ResolutionStrategy>,
    /// Whether this module has side effects
    pub has_side_effects: bool,
    /// Whether this module has circular dependencies
    pub has_circular_deps: bool,
    /// Has conditional logic
    pub has_conditional: bool,
}

/// Module status for dependency resolution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleStatus {
    /// Needs init wrapper function
    NeedsInit,
    /// Direct inline possible
    DirectInline,
    /// Has circular dependencies
    CircularDep,
    /// Has conditional logic
    Conditional,
}

/// Minimal, orthogonal execution steps for the dumb executor
#[derive(Debug, Clone)]
pub enum ExecutionStep {
    /// Insert a pre-built AST statement at the current position
    InsertStatement { stmt: ruff_python_ast::Stmt },

    /// Copy a statement from source, applying AST renames
    CopyStatement {
        source_module: ModuleId,
        item_id: ItemId,
    },
}

/// Classification of an import statement for bundling decisions
#[derive(Debug, Clone)]
pub enum ImportClassification {
    /// Hoist the import to the top of the bundle (ONLY safe stdlib imports)
    /// Third-party imports are NEVER hoisted due to potential side effects
    /// e.g., `import os` or `from json import loads`
    Hoist { import_type: HoistType },

    /// Inline the imported symbols directly into the bundle scope
    /// e.g., `from .utils import helper` results in `def helper(): ...` in the bundle
    Inline {
        module_id: ModuleId,
        symbols: Vec<SymbolImport>,
    },

    /// Emulate the imported module as a namespace object
    /// e.g., `import .utils` results in `utils = SimpleNamespace()` and `utils.helper = helper`
    EmulateAsNamespace {
        module_id: ModuleId,
        /// The name it will have in the importing module, e.g., "utils"
        alias: String,
    },
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

/// Symbol import info for Inline imports
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
}

impl BundlePlan {
    /// Create a new empty bundle plan
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an import rewrite for circular dependency resolution
    pub fn add_import_rewrite(&mut self, rewrite: ImportRewrite) {
        self.import_rewrites.push(rewrite);
    }

    /// Set symbol renames from conflict analysis
    pub fn set_symbol_renames(&mut self, renames: IndexMap<GlobalBindingId, String>) {
        self.symbol_renames = renames;
    }

    /// Add module metadata
    pub fn set_module_metadata(&mut self, module_id: ModuleId, metadata: ModuleMetadata) {
        self.module_metadata.insert(module_id, metadata);
    }

    /// Add a hoisted import
    pub fn add_hoisted_import(&mut self, import: HoistedImport) {
        self.hoisted_imports.push(import);
    }

    /// Add tree-shaking decisions
    fn add_tree_shake_decisions(&mut self, tree_shake: &crate::analysis::TreeShakeResults) {
        log::debug!(
            "Adding tree-shake decisions: {} live items total",
            tree_shake.included_items.len()
        );

        // Convert Vec<(ModuleId, ItemId)> to HashMap<ModuleId, Vec<ItemId>>
        self.live_items.clear();
        for (module_id, item_id) in &tree_shake.included_items {
            self.live_items
                .entry(*module_id)
                .or_default()
                .push(*item_id);
        }
    }

    /// Classify modules based on their dependencies and characteristics
    fn classify_modules(&mut self, graph: &CriboGraph) {
        log::debug!("Classifying {} modules", graph.modules.len());
        for (module_id, module_graph) in &graph.modules {
            let mut metadata = ModuleMetadata::default();

            // Check for circular dependencies
            if self
                .import_rewrites
                .iter()
                .any(|r| r.module_id == *module_id)
            {
                metadata.has_circular_deps = true;
            }

            // Check for side effects (would need side effect analysis)
            // For now, assume modules with non-import/assignment statements have side effects
            for item in module_graph.items.values() {
                match &item.item_type {
                    ItemType::Import { .. }
                    | ItemType::FromImport { .. }
                    | ItemType::Assignment { .. }
                    | ItemType::FunctionDef { .. }
                    | ItemType::ClassDef { .. } => {}
                    _ => {
                        metadata.has_side_effects = true;
                    }
                }
            }

            self.module_metadata.insert(*module_id, metadata);
        }
    }

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
            if let Err(e) = plan.build_execution_plan(graph, registry, entry_module_id) {
                log::error!("Failed to build execution plan: {e}");
            } else {
                log::info!("Successfully built execution plan");
            }
        } else {
            log::error!("Entry module '{entry_module_name}' not found in registry");
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

                self.symbol_renames.insert(instance.global_id, new_name);
            }
        }

        log::debug!(
            "Added {} symbol renames from {} conflicts",
            self.symbol_renames.len(),
            symbol_conflicts.len()
        );
    }

    /// Populate module aliases from import statements
    fn populate_module_aliases(&mut self, graph: &CriboGraph, registry: &ModuleRegistry) {
        log::debug!("Populating module aliases from import statements");

        for (module_id, module_graph) in &graph.modules {
            for item_data in module_graph.items.values() {
                match &item_data.item_type {
                    ItemType::Import { module, alias } => {
                        // import foo -> creates alias "foo" -> module foo
                        // import foo.bar -> creates alias "foo" -> module foo (not foo.bar!)
                        // import foo as f -> creates alias "f" -> module foo
                        if let Some(imported_module_id) = registry.get_id_by_name(module) {
                            let alias_name = alias.as_ref().unwrap_or(module);
                            self.module_aliases
                                .insert((*module_id, alias_name.clone()), imported_module_id);
                            log::trace!(
                                "Module alias: ({module_id:?}, '{alias_name}') -> \
                                 {imported_module_id:?}"
                            );
                        }
                    }
                    ItemType::FromImport {
                        module,
                        names,
                        level,
                        ..
                    } => {
                        // from foo import bar -> creates alias "bar" -> item bar in module foo
                        // from . import foo -> creates alias "foo" -> module foo
                        // Resolve the full module path considering relative imports
                        let current_module_name = registry
                            .get_name_by_id(*module_id)
                            .expect("Module must have a name");

                        let full_module_path = if *level > 0 {
                            // Relative import - resolve based on current module
                            let parts: Vec<_> = current_module_name.split('.').collect();
                            if *level as usize <= parts.len() {
                                let parent_parts = &parts[..parts.len() - *level as usize];
                                if module.is_empty() {
                                    parent_parts.join(".")
                                } else {
                                    format!("{}.{}", parent_parts.join("."), module)
                                }
                            } else {
                                log::warn!(
                                    "Relative import level {level} exceeds module depth for \
                                     {current_module_name}"
                                );
                                continue;
                            }
                        } else {
                            // Absolute import
                            module.clone()
                        };

                        // Check if any imported symbol is actually a submodule
                        for (symbol_name, symbol_alias) in names {
                            let potential_module_name = format!("{full_module_path}.{symbol_name}");
                            if let Some(submodule_id) =
                                registry.get_id_by_name(&potential_module_name)
                            {
                                let alias_name = symbol_alias.as_ref().unwrap_or(symbol_name);
                                self.module_aliases
                                    .insert((*module_id, alias_name.clone()), submodule_id);
                                log::trace!(
                                    "Module alias from 'from' import: ({module_id:?}, \
                                     '{alias_name}') -> {submodule_id:?} (module \
                                     {potential_module_name})"
                                );
                            } else if let Some(direct_module_id) =
                                registry.get_id_by_name(symbol_name)
                            {
                                // Handle "from . import module_name" case
                                if *level > 0 && module.is_empty() {
                                    let alias_name = symbol_alias.as_ref().unwrap_or(symbol_name);
                                    self.module_aliases
                                        .insert((*module_id, alias_name.clone()), direct_module_id);
                                    log::trace!(
                                        "Module alias from relative import: ({module_id:?}, \
                                         '{alias_name}') -> {direct_module_id:?}"
                                    );
                                }
                            } else {
                                log::trace!(
                                    "Import '{symbol_name}' from '{full_module_path}' is not a \
                                     module (likely a symbol)"
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

    /// Build the execution plan using classified imports - acts as a compiler
    pub fn build_execution_plan(
        &mut self,
        graph: &CriboGraph,
        registry: &ModuleRegistry,
        entry_module_id: ModuleId,
    ) -> anyhow::Result<()> {
        use anyhow::anyhow;

        log::debug!("Building execution plan with entry module {entry_module_id:?}");

        // Clear any existing plan
        self.execution_plan.clear();

        // Collect all necessary imports by category
        let mut future_imports = Vec::new();
        let mut stdlib_imports = Vec::new();
        let mut third_party_imports = Vec::new();
        let mut namespace_modules: FxHashMap<ModuleId, String> = FxHashMap::default();

        // Track imported modules to avoid duplicates
        let mut imported_modules: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Process all classified imports
        log::debug!(
            "Processing {} classified imports",
            self.classified_imports.len()
        );
        for ((_module_id, _item_id), classification) in &self.classified_imports {
            match classification {
                ImportClassification::EmulateAsNamespace {
                    module_id: imported_module_id,
                    alias,
                } => {
                    // Record that this module needs a namespace
                    namespace_modules.insert(*imported_module_id, alias.clone());
                }
                ImportClassification::Inline {
                    module_id,
                    symbols: _,
                } => {
                    // For from imports, we need to bundle the module and create namespace objects
                    // Use sanitized module name for namespace (e.g., mymodule.utils ->
                    // mymodule_utils)
                    if let Some(module_name) = registry.get_name_by_id(*module_id) {
                        let namespace_name =
                            ModuleRegistry::sanitize_module_name_for_identifier(module_name);
                        namespace_modules.insert(*module_id, namespace_name);
                    }
                }
                ImportClassification::Hoist { import_type } => {
                    // Build the import AST and categorize it
                    match import_type {
                        HoistType::Direct { module_name, alias } => {
                            // Track the module to avoid duplicates
                            let import_key = if let Some(alias) = alias {
                                format!("{module_name} as {alias}")
                            } else {
                                module_name.clone()
                            };

                            if imported_modules.insert(import_key) {
                                // Not a duplicate, create the import
                                let stmt = if let Some(alias) = alias {
                                    ast_builder::import_as(module_name, alias)
                                } else {
                                    ast_builder::import(module_name)
                                };

                                // Categorize the import
                                if module_name == "__future__" {
                                    future_imports.push(stmt);
                                } else if is_stdlib_module(module_name) {
                                    stdlib_imports.push(stmt);
                                } else {
                                    third_party_imports.push(stmt);
                                }
                            }
                        }
                        HoistType::From {
                            module_name,
                            symbols,
                            level,
                        } => {
                            // Create a unique key for this from import
                            let symbols_str = symbols
                                .iter()
                                .map(|(n, a)| {
                                    if let Some(alias) = a {
                                        format!("{n} as {alias}")
                                    } else {
                                        n.clone()
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(", ");

                            let import_key = if *level > 0 {
                                format!(
                                    "from {}{} import {}",
                                    "..".repeat(*level as usize),
                                    module_name,
                                    symbols_str
                                )
                            } else {
                                format!("from {module_name} import {symbols_str}")
                            };

                            if imported_modules.insert(import_key) {
                                // Not a duplicate, create the import
                                let stmt = if *level > 0 {
                                    // Relative import
                                    let names: Vec<&str> =
                                        symbols.iter().map(|(n, _)| n.as_str()).collect();
                                    ast_builder::relative_from_import(
                                        if module_name.is_empty() {
                                            None
                                        } else {
                                            Some(module_name)
                                        },
                                        *level,
                                        &names,
                                    )
                                } else {
                                    // Absolute import
                                    let symbols_refs: Vec<(&str, Option<&str>)> = symbols
                                        .iter()
                                        .map(|(name, alias)| (name.as_str(), alias.as_deref()))
                                        .collect();
                                    ast_builder::from_import_with_aliases(
                                        module_name,
                                        &symbols_refs,
                                    )
                                };

                                // Categorize the import
                                if module_name == "__future__" {
                                    future_imports.push(stmt);
                                } else if is_stdlib_module(module_name) {
                                    stdlib_imports.push(stmt);
                                } else {
                                    third_party_imports.push(stmt);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Sort imports within each category for determinism
        sort_import_statements(&mut stdlib_imports);
        sort_import_statements(&mut third_party_imports);

        // Build the execution plan in the correct order

        // 1. Future imports first
        for stmt in future_imports {
            self.execution_plan
                .push(ExecutionStep::InsertStatement { stmt });
        }

        // 2. Add types import if we have namespace modules
        if !namespace_modules.is_empty() {
            // Check if types was already imported
            if imported_modules.insert("types".to_string()) {
                let types_import = ast_builder::import("types");
                self.execution_plan
                    .push(ExecutionStep::InsertStatement { stmt: types_import });
            }
        }

        // 3. Other stdlib imports
        for stmt in stdlib_imports {
            self.execution_plan
                .push(ExecutionStep::InsertStatement { stmt });
        }

        // 4. Third-party imports
        for stmt in third_party_imports {
            self.execution_plan
                .push(ExecutionStep::InsertStatement { stmt });
        }

        // 5. Process namespace modules' content
        log::debug!("Processing {} namespace modules", namespace_modules.len());
        for (module_id, namespace_name) in &namespace_modules {
            log::debug!("Processing namespace module {module_id:?} as '{namespace_name}'");
            if let Some(items) = self.live_items.get(module_id) {
                log::debug!(
                    "  Found {} live items for module {:?}",
                    items.len(),
                    module_id
                );
                for item_id in items {
                    log::debug!("    Processing item {item_id:?}");
                    let module_graph = graph
                        .modules
                        .get(module_id)
                        .ok_or_else(|| anyhow!("Module not found: {:?}", module_id))?;

                    let item_data = module_graph
                        .items
                        .get(item_id)
                        .ok_or_else(|| anyhow!("Item not found: {:?}", item_id))?;

                    // Skip import statements
                    if matches!(
                        item_data.item_type,
                        ItemType::Import { .. } | ItemType::FromImport { .. }
                    ) {
                        continue;
                    }

                    // Copy the statement
                    self.execution_plan.push(ExecutionStep::CopyStatement {
                        source_module: *module_id,
                        item_id: *item_id,
                    });
                }
            } else {
                log::warn!("No live items found for namespace module {module_id:?}");
            }
        }

        // 6. Create namespace objects and populate them
        for (module_id, namespace_name) in &namespace_modules {
            // Create the namespace object
            let create_stmt = ast_builder::assign(
                namespace_name,
                ast_builder::call(ast_builder::attribute("types", "SimpleNamespace")),
            );
            self.execution_plan
                .push(ExecutionStep::InsertStatement { stmt: create_stmt });

            // Populate the namespace
            if let Some(items) = self.live_items.get(module_id) {
                let module_graph = graph
                    .modules
                    .get(module_id)
                    .ok_or_else(|| anyhow!("Module not found: {:?}", module_id))?;

                for item_id in items {
                    let item_data = module_graph
                        .items
                        .get(item_id)
                        .ok_or_else(|| anyhow!("Item not found: {:?}", item_id))?;

                    // Skip imports and private symbols
                    if matches!(
                        item_data.item_type,
                        ItemType::Import { .. } | ItemType::FromImport { .. }
                    ) {
                        continue;
                    }

                    // Extract symbol names based on item type
                    let symbols = match &item_data.item_type {
                        ItemType::Assignment { targets } => targets.clone(),
                        ItemType::FunctionDef { name } => vec![name.clone()],
                        ItemType::ClassDef { name } => vec![name.clone()],
                        _ => continue,
                    };

                    // Generate namespace assignment for each symbol
                    for symbol in symbols {
                        // Skip private symbols
                        if symbol.starts_with('_') {
                            continue;
                        }

                        let assign_stmt = ast_builder::assign_attribute(
                            namespace_name,
                            &symbol,
                            ast_builder::name(&symbol),
                        );
                        self.execution_plan
                            .push(ExecutionStep::InsertStatement { stmt: assign_stmt });
                    }
                }
            }
        }

        // 7. Handle symbol assignments for Inline imports
        // After creating all namespace objects, assign the imported symbols
        for ((module_id, _item_id), classification) in &self.classified_imports {
            if let ImportClassification::Inline {
                module_id: imported_module_id,
                symbols,
            } = classification
            {
                // Skip if not from entry module
                if module_id != &entry_module_id {
                    continue;
                }

                // Get the namespace name for the imported module
                if let Some(namespace_name) = namespace_modules.get(imported_module_id) {
                    // Generate assignment for each imported symbol
                    // e.g., from mymodule import utils -> utils = mymodule_utils
                    for symbol in symbols {
                        let assign_stmt = ast_builder::assign(
                            &symbol.target_name,
                            ast_builder::name(namespace_name),
                        );
                        self.execution_plan
                            .push(ExecutionStep::InsertStatement { stmt: assign_stmt });
                    }
                }
            }
        }

        // 8. Process entry module statements
        log::debug!("Processing entry module {entry_module_id:?} statements");
        if let Some(items) = self.live_items.get(&entry_module_id) {
            log::debug!("  Entry module has {} live items", items.len());

            // Sort items by their statement index to preserve source order
            let module_graph = graph
                .modules
                .get(&entry_module_id)
                .ok_or_else(|| anyhow!("Entry module not found in graph"))?;

            let mut sorted_items: Vec<_> = items
                .iter()
                .filter_map(|item_id| {
                    module_graph
                        .items
                        .get(item_id)
                        .and_then(|item_data| item_data.statement_index.map(|idx| (*item_id, idx)))
                })
                .collect();
            sorted_items.sort_by_key(|(_, idx)| *idx);

            for (item_id, _) in sorted_items {
                // Skip import statements that have been classified
                if self
                    .classified_imports
                    .contains_key(&(entry_module_id, item_id))
                {
                    continue;
                }

                self.execution_plan.push(ExecutionStep::CopyStatement {
                    source_module: entry_module_id,
                    item_id,
                });
            }
        }

        log::debug!(
            "Built execution plan with {} steps",
            self.execution_plan.len()
        );

        // Log all steps for debugging
        for (i, step) in self.execution_plan.iter().enumerate() {
            log::debug!("  Step {i}: {step:?}");
        }

        Ok(())
    }

    /// Classify imports based on their type and how they should be bundled
    fn classify_imports(&mut self, graph: &CriboGraph, registry: &ModuleRegistry) {
        use crate::resolver::{ImportType, ModuleResolver};

        log::debug!("Starting import classification");
        log::debug!("Registry has {} modules", registry.len());
        for module_name in registry.module_names() {
            log::debug!("  Module in registry: '{module_name}'");
        }

        // Create a temporary resolver for classification
        // TODO: This should be passed from the orchestrator
        let mut resolver = match ModuleResolver::new(crate::config::Config::default()) {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to create resolver for import classification: {e}");
                return;
            }
        };

        // Iterate through ALL modules and items to find imports
        // We need to classify all imports, not just those in live_items
        log::debug!(
            "Classifying imports from {} modules in graph",
            graph.modules.len()
        );
        for (module_id, module_graph) in &graph.modules {
            log::debug!(
                "  Module {:?} has {} items",
                module_id,
                module_graph.items.len()
            );

            let module_name = match registry.get_name_by_id(*module_id) {
                Some(name) => name,
                None => {
                    log::warn!("Module {module_id:?} not found in registry");
                    continue;
                }
            };

            for (item_id, item_data) in &module_graph.items {
                log::debug!("    Item {:?} type: {:?}", item_id, item_data.item_type);

                match &item_data.item_type {
                    ItemType::Import { module, alias } => {
                        // Handle direct imports: import foo, import foo as bar
                        let classification = if registry.has_module(module) {
                            // This is a first-party module that will be bundled
                            let imported_module_id =
                                registry.get_id_by_name(module).expect("Module must exist");

                            ImportClassification::EmulateAsNamespace {
                                module_id: imported_module_id,
                                alias: alias.clone().unwrap_or_else(|| module.clone()),
                            }
                        } else {
                            // This is stdlib or third-party
                            ImportClassification::Hoist {
                                import_type: HoistType::Direct {
                                    module_name: module.clone(),
                                    alias: alias.clone(),
                                },
                            }
                        };

                        log::debug!(
                            "Classified import at {module_id:?},{item_id:?} as: {classification:?}"
                        );
                        self.classified_imports
                            .insert((*module_id, *item_id), classification);
                    }
                    ItemType::FromImport {
                        module,
                        names,
                        level,
                        ..
                    } => {
                        // Handle from imports
                        // For relative imports, we need to construct the module name with dots
                        let module_to_resolve = if *level > 0 {
                            // Relative import
                            let dots = ".".repeat(*level as usize);
                            if module.is_empty() {
                                dots
                            } else {
                                format!("{dots}.{module}")
                            }
                        } else {
                            module.clone()
                        };

                        // Get the current module's path for context
                        let current_module_path = registry
                            .get_by_id(module_id)
                            .map(|info| info.resolved_path.as_path());

                        // Resolve the import path
                        let resolved_path = resolver.resolve_module_path_with_context(
                            &module_to_resolve,
                            current_module_path,
                        );

                        log::debug!("Resolved path for '{module_to_resolve}': {resolved_path:?}");

                        // Classify the import
                        // First check if the module exists in our registry
                        let import_type = if registry.has_module(&module_to_resolve) {
                            ImportType::FirstParty
                        } else {
                            match resolved_path {
                                Ok(Some(_)) => resolver.classify_import(&module_to_resolve),
                                _ => ImportType::StandardLibrary, /* Default to stdlib if we
                                                                   * can't resolve */
                            }
                        };

                        log::debug!(
                            "Import type for '{module_to_resolve}' from '{module_name}': \
                             {import_type:?}"
                        );

                        let classification = match import_type {
                            ImportType::FirstParty => {
                                // For from imports, we need to check if the imported symbols are
                                // actually submodules e.g., from
                                // mymodule import utils -> utils might be mymodule.utils
                                let mut submodule_imports = Vec::new();
                                let mut regular_symbol_imports = Vec::new();

                                for (name, alias) in names {
                                    // Check if this is a submodule import
                                    let potential_module_name = format!("{module}.{name}");
                                    log::debug!(
                                        "Checking if '{potential_module_name}' is a module in \
                                         registry"
                                    );
                                    if let Some(submodule_id) =
                                        registry.get_id_by_name(&potential_module_name)
                                    {
                                        log::debug!(
                                            "Found submodule '{potential_module_name}' with id \
                                             {submodule_id:?}"
                                        );
                                        // This is a submodule import - we need to bundle it as a
                                        // namespace
                                        submodule_imports.push((
                                            submodule_id,
                                            SymbolImport {
                                                source_name: name.clone(),
                                                target_name: alias
                                                    .clone()
                                                    .unwrap_or_else(|| name.clone()),
                                            },
                                        ));
                                    } else {
                                        // Regular symbol import
                                        regular_symbol_imports.push(SymbolImport {
                                            source_name: name.clone(),
                                            target_name: alias
                                                .clone()
                                                .unwrap_or_else(|| name.clone()),
                                        });
                                    }
                                }

                                // For now, if we have any submodule imports, treat each as a
                                // separate Inline import. This is
                                // a simplification - in the future we might want to handle mixed
                                // imports differently
                                if !submodule_imports.is_empty() {
                                    // Process the first submodule import (simplified for now)
                                    let (submodule_id, symbol_import) = &submodule_imports[0];
                                    ImportClassification::Inline {
                                        module_id: *submodule_id,
                                        symbols: vec![symbol_import.clone()],
                                    }
                                } else if let Some(imported_module_id) =
                                    registry.get_id_by_name(module)
                                {
                                    // Regular from import with symbols
                                    ImportClassification::Inline {
                                        module_id: imported_module_id,
                                        symbols: regular_symbol_imports,
                                    }
                                } else {
                                    // First-party but not in registry - hoist it
                                    ImportClassification::Hoist {
                                        import_type: HoistType::From {
                                            module_name: module.clone(),
                                            symbols: names.clone(),
                                            level: *level,
                                        },
                                    }
                                }
                            }
                            _ => {
                                // Stdlib or third-party - hoist it
                                ImportClassification::Hoist {
                                    import_type: HoistType::From {
                                        module_name: module.clone(),
                                        symbols: names.clone(),
                                        level: *level,
                                    },
                                }
                            }
                        };

                        log::debug!(
                            "Classified from import at {module_id:?},{item_id:?} as: \
                             {classification:?}"
                        );
                        self.classified_imports
                            .insert((*module_id, *item_id), classification);
                    }
                    _ => {}
                }
            }
        }

        log::debug!("Classified {} imports", self.classified_imports.len());
    }
}

/// Check if a module is from the standard library
fn is_stdlib_module(module_name: &str) -> bool {
    // This is a simplified check - in reality we'd use a comprehensive list
    matches!(
        module_name,
        "os" | "sys"
            | "types"
            | "json"
            | "re"
            | "math"
            | "random"
            | "datetime"
            | "collections"
            | "itertools"
            | "functools"
            | "pathlib"
            | "typing"
            | "io"
            | "subprocess"
            | "threading"
            | "multiprocessing"
            | "asyncio"
            | "contextlib"
    )
}

/// Sort import statements alphabetically for determinism
fn sort_import_statements(imports: &mut Vec<ruff_python_ast::Stmt>) {
    imports.sort_by(|a, b| {
        let name_a = match a {
            ruff_python_ast::Stmt::Import(imp) => imp.names[0].name.as_str(),
            ruff_python_ast::Stmt::ImportFrom(imp) => {
                imp.module.as_ref().map_or("", |m| m.as_str())
            }
            _ => "",
        };
        let name_b = match b {
            ruff_python_ast::Stmt::Import(imp) => imp.names[0].name.as_str(),
            ruff_python_ast::Stmt::ImportFrom(imp) => {
                imp.module.as_ref().map_or("", |m| m.as_str())
            }
            _ => "",
        };
        name_a.cmp(name_b)
    });
}
