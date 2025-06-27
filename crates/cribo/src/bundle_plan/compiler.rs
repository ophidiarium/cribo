//! Bundle compiler that transforms analysis results into an executable program
//!
//! The BundleCompiler is responsible for all the "intelligence" in the bundling process.
//! It takes semantic analysis results and compiles them into a simple, linear sequence
//! of instructions (BundleProgram) that can be mechanically executed by the bundle VM.

use anyhow::Result;
use indexmap::IndexMap;
use log::{debug, warn};
use ruff_python_ast::Stmt;
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{ExecutionStep, HoistType, ImportClassification, ModuleMetadata, SymbolImport};
use crate::{
    analysis::{AnalysisResults, TreeShakeResults},
    ast_builder,
    cribo_graph::{CriboGraph, ItemId, ItemType, ModuleId},
    module_registry::ModuleRegistry,
    semantic_model_provider::GlobalBindingId,
};

/// The bundle compiler - a stateful object that orchestrates compilation
pub struct BundleCompiler<'a> {
    /// Analysis results from semantic analysis phase
    analysis_results: &'a AnalysisResults,

    /// The dependency graph
    graph: &'a CriboGraph,

    /// Module registry - single source of truth for module identity
    registry: &'a ModuleRegistry,

    /// Entry module ID
    entry_module_id: ModuleId,

    /// Symbol renames for conflict resolution
    symbol_renames: IndexMap<GlobalBindingId, String>,

    /// Live items from tree-shaking
    live_items: FxHashMap<ModuleId, Vec<ItemId>>,

    /// Classified imports
    classified_imports: FxHashMap<(ModuleId, ItemId), ImportClassification>,

    /// Module metadata
    module_metadata: FxHashMap<ModuleId, ModuleMetadata>,

    /// Module aliases from import statements
    module_aliases: FxHashMap<(ModuleId, String), ModuleId>,

    /// Semantic model provider for AST node resolution
    semantic_provider: Option<&'a crate::semantic_model_provider::SemanticModelProvider<'a>>,
}

/// The final, clean output of compilation - the "bytecode" for the VM
#[derive(Debug, Clone)]
pub struct BundleProgram {
    /// The linear sequence of instructions to execute
    pub steps: Vec<ExecutionStep>,

    /// AST node renames to apply during CopyStatement execution
    /// Maps (ModuleId, TextRange) to the new name for that AST node
    pub ast_node_renames: FxHashMap<(ModuleId, TextRange), String>,
}

impl<'a> BundleCompiler<'a> {
    /// Create a new compiler with all necessary context
    pub fn new(
        analysis_results: &'a AnalysisResults,
        graph: &'a CriboGraph,
        registry: &'a ModuleRegistry,
        entry_module_name: &str,
    ) -> Result<Self> {
        // Get entry module ID
        let entry_module_id = registry
            .get_id_by_name(entry_module_name)
            .ok_or_else(|| anyhow::anyhow!("Entry module '{}' not found", entry_module_name))?;

        let mut compiler = Self {
            analysis_results,
            graph,
            registry,
            entry_module_id,
            symbol_renames: IndexMap::new(),
            live_items: FxHashMap::default(),
            classified_imports: FxHashMap::default(),
            module_metadata: FxHashMap::default(),
            module_aliases: FxHashMap::default(),
            semantic_provider: None,
        };

        // Initialize compiler state from analysis results
        compiler.initialize_from_analysis();

        Ok(compiler)
    }

    /// Set the semantic model provider for AST node rename generation
    pub fn with_semantic_provider(
        mut self,
        provider: &'a crate::semantic_model_provider::SemanticModelProvider<'a>,
    ) -> Self {
        self.semantic_provider = Some(provider);
        self
    }

    /// Initialize compiler state from analysis results
    fn initialize_from_analysis(&mut self) {
        // Extract symbol renames from conflict analysis
        self.add_symbol_renames(&self.analysis_results.symbol_conflicts);

        // Extract live items from tree-shaking
        if let Some(tree_shake) = &self.analysis_results.tree_shake_results {
            self.add_tree_shake_decisions(tree_shake);
        } else {
            // If no tree-shaking, include all items
            for (module_id, module_graph) in &self.graph.modules {
                let items: Vec<_> = module_graph.items.keys().cloned().collect();
                self.live_items.insert(*module_id, items);
            }
        }

        // Populate module aliases
        self.populate_module_aliases();

        // Classify imports
        self.classify_imports();

        // Classify modules
        self.classify_modules();
    }

    /// Main compilation method - transforms all state into a clean program
    pub fn compile(self) -> Result<BundleProgram> {
        let mut steps = Vec::new();

        // Phase 1: Compile hoisted imports (__future__, stdlib, third-party)
        let hoisted_steps = self.compile_hoisted_imports()?;
        steps.extend(hoisted_steps);

        // Phase 2: Compile namespace infrastructure
        let namespace_steps = self.compile_namespace_modules()?;
        steps.extend(namespace_steps);

        // Phase 3: Compile entry module body
        let entry_steps = self.compile_entry_module()?;
        steps.extend(entry_steps);

        // Generate AST node renames from symbol renames
        let ast_node_renames = self.generate_ast_node_renames();

        Ok(BundleProgram {
            steps,
            ast_node_renames,
        })
    }

    /// Compile hoisted imports into execution steps
    /// IMPORTANT: Only __future__ and stdlib imports can be hoisted safely.
    /// Third-party imports may have side effects and must be preserved in their original location.
    fn compile_hoisted_imports(&self) -> Result<Vec<ExecutionStep>> {
        let mut steps = Vec::new();
        let mut future_imports = Vec::new();
        let mut stdlib_imports = Vec::new();

        // Track imported modules to avoid duplicates
        let mut imported_modules = std::collections::HashSet::new();

        // Process only LIVE Hoist classifications (imports that are actually used)
        for ((module_id, item_id), classification) in &self.classified_imports {
            // Skip if this import is not in live items (unused import)
            if let Some(module_live_items) = self.live_items.get(module_id) {
                if !module_live_items.contains(item_id) {
                    debug!("Skipping unused import in module {module_id:?}, item {item_id:?}");
                    continue; // Skip unused imports
                }
            } else {
                debug!("Skipping imports from module {module_id:?} - not in live items");
                continue; // Module not in live items at all
            }

            if let ImportClassification::Hoist { import_type } = classification {
                match import_type {
                    HoistType::Direct { module_name, alias } => {
                        let import_key = if let Some(alias) = alias {
                            format!("{module_name} as {alias}")
                        } else {
                            module_name.clone()
                        };

                        if imported_modules.insert(import_key) {
                            let stmt = if let Some(alias) = alias {
                                ast_builder::import_as(module_name, alias)
                            } else {
                                ast_builder::import(module_name)
                            };

                            // Only hoist __future__ and stdlib imports
                            if module_name == "__future__" {
                                future_imports.push(stmt);
                                debug!("Adding __future__ import: {module_name}");
                            } else if is_stdlib_module(module_name) {
                                stdlib_imports.push(stmt);
                                debug!(
                                    "Adding stdlib import: {module_name} from module {module_id:?}"
                                );
                            } else {
                                debug!("Skipping third-party import: {module_name}");
                            }
                            // Third-party imports are NOT hoisted - they stay in original location
                        }
                    }
                    HoistType::From {
                        module_name,
                        symbols,
                        level,
                    } => {
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
                                ".".repeat(*level as usize),
                                module_name,
                                symbols_str
                            )
                        } else {
                            format!("from {module_name} import {symbols_str}")
                        };

                        if imported_modules.insert(import_key) {
                            let stmt = if *level > 0 {
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
                                let symbols_refs: Vec<(&str, Option<&str>)> = symbols
                                    .iter()
                                    .map(|(name, alias)| (name.as_str(), alias.as_deref()))
                                    .collect();
                                ast_builder::from_import_with_aliases(module_name, &symbols_refs)
                            };

                            // Only hoist __future__ and stdlib imports
                            if module_name == "__future__" {
                                future_imports.push(stmt);
                            } else if is_stdlib_module(module_name) {
                                stdlib_imports.push(stmt);
                            }
                            // Third-party imports are NOT hoisted - they stay in original location
                        }
                    }
                }
            }
        }

        // Sort imports for determinism
        sort_import_statements(&mut stdlib_imports);

        // Build steps in order: future, stdlib only
        // Third-party imports are NOT hoisted due to potential side effects
        for stmt in future_imports {
            steps.push(ExecutionStep::InsertStatement { stmt });
        }

        for stmt in stdlib_imports {
            steps.push(ExecutionStep::InsertStatement { stmt });
        }

        Ok(steps)
    }

    /// Compile namespace modules into execution steps
    fn compile_namespace_modules(&self) -> Result<Vec<ExecutionStep>> {
        let mut steps = Vec::new();
        let mut namespace_modules: FxHashMap<ModuleId, String> = FxHashMap::default();

        // Collect modules that need namespace treatment
        for classification in self.classified_imports.values() {
            match classification {
                ImportClassification::EmulateAsNamespace { module_id, alias } => {
                    namespace_modules.insert(*module_id, alias.clone());
                }
                ImportClassification::Inline { module_id, .. } => {
                    // Inline imports also need namespace objects for their module
                    if let Some(module_name) = self.registry.get_name_by_id(*module_id) {
                        let namespace_name =
                            ModuleRegistry::sanitize_module_name_for_identifier(module_name);
                        namespace_modules.insert(*module_id, namespace_name);
                    }
                }
                _ => {}
            }
        }

        if namespace_modules.is_empty() {
            return Ok(steps);
        }

        // Add types import if needed
        let types_import = ast_builder::import("types");
        steps.push(ExecutionStep::InsertStatement { stmt: types_import });

        // Process each namespace module
        debug!("Processing {} namespace modules", namespace_modules.len());
        for (module_id, namespace_name) in &namespace_modules {
            debug!("Processing namespace module {module_id:?} with name '{namespace_name}'");
            // First, copy all non-import statements from the module
            if let Some(items) = self.live_items.get(module_id) {
                debug!("Module {:?} has {} live items", module_id, items.len());
                let module_graph = self
                    .graph
                    .modules
                    .get(module_id)
                    .ok_or_else(|| anyhow::anyhow!("Module not found: {:?}", module_id))?;

                for item_id in items {
                    let item_data = module_graph
                        .items
                        .get(item_id)
                        .ok_or_else(|| anyhow::anyhow!("Item not found: {:?}", item_id))?;

                    // Skip import statements
                    if matches!(
                        item_data.item_type,
                        ItemType::Import { .. } | ItemType::FromImport { .. }
                    ) {
                        continue;
                    }

                    // Skip function definitions and expressions that are likely inside a class
                    // (they have the same statement_index as the class itself)
                    if matches!(
                        item_data.item_type,
                        ItemType::FunctionDef { .. } | ItemType::Expression
                    ) {
                        // Check if there's a class definition with the same statement index
                        let has_class_with_same_index = items.iter().any(|other_id| {
                            if let Some(other_data) = module_graph.items.get(other_id) {
                                matches!(other_data.item_type, ItemType::ClassDef { .. })
                                    && other_data.statement_index == item_data.statement_index
                            } else {
                                false
                            }
                        });

                        if has_class_with_same_index {
                            debug!("Skipping method {:?} (inside a class)", item_data.item_type);
                            continue;
                        }
                    }

                    debug!(
                        "Adding CopyStatement for item {:?} of type {:?}",
                        item_id, item_data.item_type
                    );
                    steps.push(ExecutionStep::CopyStatement {
                        source_module: *module_id,
                        item_id: *item_id,
                    });
                }
            }

            // Create the namespace object
            let create_stmt = ast_builder::assign(
                namespace_name,
                ast_builder::call(ast_builder::attribute("types", "SimpleNamespace")),
            );
            steps.push(ExecutionStep::InsertStatement { stmt: create_stmt });

            // Populate the namespace with public symbols
            if let Some(items) = self.live_items.get(module_id) {
                let module_graph = self
                    .graph
                    .modules
                    .get(module_id)
                    .ok_or_else(|| anyhow::anyhow!("Module not found: {:?}", module_id))?;

                // First pass: collect all class names to identify methods
                let class_names: FxHashSet<_> = items
                    .iter()
                    .filter_map(|item_id| {
                        module_graph.items.get(item_id).and_then(|item_data| {
                            if let ItemType::ClassDef { name } = &item_data.item_type {
                                Some(name.clone())
                            } else {
                                None
                            }
                        })
                    })
                    .collect();

                for item_id in items {
                    let item_data = module_graph
                        .items
                        .get(item_id)
                        .ok_or_else(|| anyhow::anyhow!("Item not found: {:?}", item_id))?;

                    // Skip imports and private symbols
                    if matches!(
                        item_data.item_type,
                        ItemType::Import { .. } | ItemType::FromImport { .. }
                    ) {
                        continue;
                    }

                    // Skip methods (functions that are inside classes)
                    if let ItemType::FunctionDef { .. } = &item_data.item_type {
                        let has_class_with_same_index = items.iter().any(|other_id| {
                            if let Some(other_data) = module_graph.items.get(other_id) {
                                matches!(other_data.item_type, ItemType::ClassDef { .. })
                                    && other_data.statement_index == item_data.statement_index
                            } else {
                                false
                            }
                        });

                        if has_class_with_same_index {
                            continue; // Skip methods
                        }
                    }

                    // Extract symbol names based on item type
                    let symbols = match &item_data.item_type {
                        ItemType::Assignment { targets } => targets.clone(),
                        ItemType::FunctionDef { name } => vec![name.clone()],
                        ItemType::ClassDef { name } => vec![name.clone()],
                        _ => continue,
                    };

                    // Generate namespace assignment for each public symbol
                    for symbol in symbols {
                        if !symbol.starts_with('_') {
                            // Get the potentially renamed symbol name
                            let renamed_symbol = self.get_renamed_symbol_name(*module_id, &symbol);

                            let assign_stmt = ast_builder::assign_attribute(
                                namespace_name,
                                &symbol,
                                ast_builder::name(&renamed_symbol),
                            );
                            steps.push(ExecutionStep::InsertStatement { stmt: assign_stmt });
                        }
                    }
                }
            }
        }

        // Handle symbol assignments for Inline imports from entry module
        for ((module_id, _), classification) in &self.classified_imports {
            if *module_id != self.entry_module_id {
                continue;
            }

            if let ImportClassification::Inline {
                module_id: imported_module_id,
                symbols,
            } = classification
                && let Some(namespace_name) = namespace_modules.get(imported_module_id)
            {
                for symbol in symbols {
                    // Create assignment: imported_name = namespace.source_name
                    let assign_stmt = ast_builder::assign(
                        &symbol.target_name,
                        ast_builder::attribute(namespace_name, &symbol.source_name),
                    );
                    steps.push(ExecutionStep::InsertStatement { stmt: assign_stmt });
                }
            }
        }

        Ok(steps)
    }

    /// Compile entry module body
    fn compile_entry_module(&self) -> Result<Vec<ExecutionStep>> {
        let mut steps = Vec::new();

        if let Some(items) = self.live_items.get(&self.entry_module_id) {
            let module_graph = self
                .graph
                .modules
                .get(&self.entry_module_id)
                .ok_or_else(|| anyhow::anyhow!("Entry module not found in graph"))?;

            // Sort items by statement index to preserve source order
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
                    .contains_key(&(self.entry_module_id, item_id))
                {
                    continue;
                }

                steps.push(ExecutionStep::CopyStatement {
                    source_module: self.entry_module_id,
                    item_id,
                });
            }
        }

        Ok(steps)
    }

    /// Get the renamed name for a symbol, or return the original if not renamed
    fn get_renamed_symbol_name(&self, module_id: ModuleId, symbol_name: &str) -> String {
        // If we don't have a semantic provider, we can't resolve renames
        let Some(semantic_provider) = self.semantic_provider else {
            return symbol_name.to_string();
        };

        // Get the semantic model for this module
        let Some(result) = semantic_provider.get_model(module_id) else {
            return symbol_name.to_string();
        };

        let Ok(semantic_model) = result else {
            return symbol_name.to_string();
        };

        // Find the binding for this symbol in the global scope
        let global_scope = semantic_model.global_scope();
        let Some(binding_id) = global_scope.get(symbol_name) else {
            return symbol_name.to_string();
        };

        // Create the global binding ID
        let global_binding_id = GlobalBindingId {
            module_id,
            binding_id,
        };

        // Check if this symbol has been renamed
        self.symbol_renames
            .get(&global_binding_id)
            .cloned()
            .unwrap_or_else(|| symbol_name.to_string())
    }

    /// Generate AST node renames from symbol renames
    fn generate_ast_node_renames(&self) -> FxHashMap<(ModuleId, TextRange), String> {
        let mut ast_node_renames = FxHashMap::default();

        // We need the semantic provider to map bindings to AST nodes
        let Some(semantic_provider) = self.semantic_provider else {
            warn!("No semantic provider available for AST node rename generation");
            return ast_node_renames;
        };

        // Iterate through all symbol rename decisions
        for (global_binding_id, new_name) in &self.symbol_renames {
            let module_id = global_binding_id.module_id;
            let binding_id = global_binding_id.binding_id;

            // Get the semantic model for this module
            if let Some(Ok(semantic_model)) = semantic_provider.get_model(module_id) {
                // Get the binding information
                let binding = semantic_model.binding(binding_id);

                // 1. Add the definition itself to the rename map
                ast_node_renames.insert((module_id, binding.range), new_name.clone());

                // 2. Add all references to the rename map
                for reference_id in &binding.references {
                    let reference = semantic_model.reference(*reference_id);
                    ast_node_renames.insert((module_id, reference.range()), new_name.clone());
                }

                debug!(
                    "Added {} AST node renames for symbol '{}' in module {:?}",
                    binding.references.len() + 1,
                    new_name,
                    module_id
                );
            }
        }

        debug!(
            "Generated {} AST node renames from {} symbol renames",
            ast_node_renames.len(),
            self.symbol_renames.len()
        );

        ast_node_renames
    }

    // Helper methods moved from BundlePlan

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

        debug!(
            "Added {} symbol renames from {} conflicts",
            self.symbol_renames.len(),
            symbol_conflicts.len()
        );
    }

    fn add_tree_shake_decisions(&mut self, tree_shake: &TreeShakeResults) {
        debug!(
            "Adding tree-shake decisions: {} live items total",
            tree_shake.included_items.len()
        );

        self.live_items.clear();
        for (module_id, item_id) in &tree_shake.included_items {
            self.live_items
                .entry(*module_id)
                .or_default()
                .push(*item_id);
        }
    }

    fn populate_module_aliases(&mut self) {
        debug!("Populating module aliases from import statements");

        for (module_id, module_graph) in &self.graph.modules {
            for item_data in module_graph.items.values() {
                match &item_data.item_type {
                    ItemType::Import { module, alias } => {
                        if let Some(imported_module_id) = self.registry.get_id_by_name(module) {
                            let alias_name = alias.as_ref().unwrap_or(module);
                            self.module_aliases
                                .insert((*module_id, alias_name.clone()), imported_module_id);
                        }
                    }
                    ItemType::FromImport {
                        module,
                        names,
                        level,
                        ..
                    } => {
                        let current_module_name = self
                            .registry
                            .get_name_by_id(*module_id)
                            .expect("Module must have a name");

                        let full_module_path = if *level > 0 {
                            let parts: Vec<_> = current_module_name.split('.').collect();
                            if *level as usize <= parts.len() {
                                let parent_parts = &parts[..parts.len() - *level as usize];
                                if module.is_empty() {
                                    parent_parts.join(".")
                                } else {
                                    format!("{}.{}", parent_parts.join("."), module)
                                }
                            } else {
                                warn!(
                                    "Relative import level {level} exceeds module depth for \
                                     {current_module_name}"
                                );
                                continue;
                            }
                        } else {
                            module.clone()
                        };

                        // Check if any imported symbol is actually a submodule
                        for (symbol_name, symbol_alias) in names {
                            let potential_module_name = format!("{full_module_path}.{symbol_name}");
                            if let Some(submodule_id) =
                                self.registry.get_id_by_name(&potential_module_name)
                            {
                                let alias_name = symbol_alias.as_ref().unwrap_or(symbol_name);
                                self.module_aliases
                                    .insert((*module_id, alias_name.clone()), submodule_id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        debug!("Populated {} module aliases", self.module_aliases.len());
    }

    fn classify_imports(&mut self) {
        use crate::resolver::{ImportType, ModuleResolver};

        debug!("Starting import classification");

        // Create a temporary resolver for classification
        // TODO: This should be passed from the orchestrator
        let mut resolver = match ModuleResolver::new(crate::config::Config::default()) {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to create resolver for import classification: {e}");
                return;
            }
        };

        // Process all imports in all modules
        for (module_id, module_graph) in &self.graph.modules {
            let _module_name = match self.registry.get_name_by_id(*module_id) {
                Some(name) => name,
                None => {
                    warn!("Module {module_id:?} not found in registry");
                    continue;
                }
            };

            // Get live items for this module
            let module_live_items = self.live_items.get(module_id);

            for (item_id, item_data) in &module_graph.items {
                // Skip if this item is not in live items (was removed by tree-shaking)
                if let Some(live_items) = module_live_items
                    && !live_items.contains(item_id)
                {
                    // This import was removed by tree-shaking, skip classification
                    continue;
                }
                match &item_data.item_type {
                    ItemType::Import { module, alias } => {
                        debug!(
                            "Classifying import '{module}' in module {module_id:?}, item \
                             {item_id:?}"
                        );
                        let classification = if self.registry.has_module(module) {
                            let imported_module_id = self
                                .registry
                                .get_id_by_name(module)
                                .expect("Module must exist");
                            debug!(
                                "Classified '{module}' as EmulateAsNamespace with module_id \
                                 {imported_module_id:?}"
                            );
                            ImportClassification::EmulateAsNamespace {
                                module_id: imported_module_id,
                                alias: alias.clone().unwrap_or_else(|| module.clone()),
                            }
                        } else {
                            debug!("Classified '{module}' as Hoist (not first-party)");
                            ImportClassification::Hoist {
                                import_type: HoistType::Direct {
                                    module_name: module.clone(),
                                    alias: alias.clone(),
                                },
                            }
                        };

                        self.classified_imports
                            .insert((*module_id, *item_id), classification);
                    }
                    ItemType::FromImport {
                        module,
                        names,
                        level,
                        ..
                    } => {
                        // For relative imports, resolve to absolute module name
                        let resolved_module_name = if *level > 0 {
                            let current_module_name = match self.registry.get_name_by_id(*module_id)
                            {
                                Some(name) => name,
                                None => {
                                    warn!("Module {module_id:?} not found in registry");
                                    continue;
                                }
                            };

                            let parts: Vec<_> = current_module_name.split('.').collect();
                            if *level as usize <= parts.len() {
                                let parent_parts = &parts[..parts.len() - *level as usize];
                                if module.is_empty() {
                                    parent_parts.join(".")
                                } else {
                                    format!("{}.{}", parent_parts.join("."), module)
                                }
                            } else {
                                warn!(
                                    "Relative import level {level} exceeds module depth for \
                                     {current_module_name}"
                                );
                                continue;
                            }
                        } else {
                            module.clone()
                        };

                        let import_type = if self.registry.has_module(&resolved_module_name) {
                            ImportType::FirstParty
                        } else {
                            // Use resolver for non-first-party classification
                            let module_to_resolve = if *level > 0 {
                                let dots = ".".repeat(*level as usize);
                                if module.is_empty() {
                                    dots
                                } else {
                                    format!("{dots}.{module}")
                                }
                            } else {
                                module.clone()
                            };

                            let current_module_path = self
                                .registry
                                .get_by_id(module_id)
                                .map(|info| info.resolved_path.as_path());

                            let resolved_path = resolver.resolve_module_path_with_context(
                                &module_to_resolve,
                                current_module_path,
                            );

                            match resolved_path {
                                Ok(Some(_)) => resolver.classify_import(&module_to_resolve),
                                _ => ImportType::StandardLibrary,
                            }
                        };

                        let classification = match import_type {
                            ImportType::FirstParty => {
                                let mut submodule_imports = Vec::new();
                                let mut regular_symbol_imports = Vec::new();

                                for (name, alias) in names {
                                    let potential_module_name =
                                        format!("{resolved_module_name}.{name}");
                                    if let Some(submodule_id) =
                                        self.registry.get_id_by_name(&potential_module_name)
                                    {
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
                                        regular_symbol_imports.push(SymbolImport {
                                            source_name: name.clone(),
                                            target_name: alias
                                                .clone()
                                                .unwrap_or_else(|| name.clone()),
                                        });
                                    }
                                }

                                if !submodule_imports.is_empty() {
                                    let (submodule_id, symbol_import) = &submodule_imports[0];
                                    ImportClassification::Inline {
                                        module_id: *submodule_id,
                                        symbols: vec![symbol_import.clone()],
                                    }
                                } else if let Some(imported_module_id) =
                                    self.registry.get_id_by_name(&resolved_module_name)
                                {
                                    ImportClassification::Inline {
                                        module_id: imported_module_id,
                                        symbols: regular_symbol_imports,
                                    }
                                } else {
                                    // This should not happen for first-party imports
                                    warn!(
                                        "First-party module '{resolved_module_name}' not found in \
                                         registry"
                                    );
                                    ImportClassification::Hoist {
                                        import_type: HoistType::From {
                                            module_name: module.clone(),
                                            symbols: names.clone(),
                                            level: *level,
                                        },
                                    }
                                }
                            }
                            _ => ImportClassification::Hoist {
                                import_type: HoistType::From {
                                    module_name: module.clone(),
                                    symbols: names.clone(),
                                    level: *level,
                                },
                            },
                        };

                        self.classified_imports
                            .insert((*module_id, *item_id), classification);
                    }
                    _ => {}
                }
            }
        }

        debug!("Classified {} imports", self.classified_imports.len());
    }

    fn classify_modules(&mut self) {
        debug!("Classifying {} modules", self.graph.modules.len());

        for (module_id, module_graph) in &self.graph.modules {
            let mut metadata = ModuleMetadata::default();

            // Check for circular dependencies
            if let Some(circular_deps) = &self.analysis_results.circular_deps
                && circular_deps
                    .resolvable_cycles
                    .iter()
                    .any(|cycle| cycle.module_ids.contains(module_id))
            {
                metadata.has_circular_deps = true;
            }

            // Check for side effects
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
}

/// Check if a module is from the standard library
fn is_stdlib_module(module_name: &str) -> bool {
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
fn sort_import_statements(imports: &mut Vec<Stmt>) {
    imports.sort_by(|a, b| {
        let name_a = match a {
            Stmt::Import(imp) => imp.names[0].name.as_str(),
            Stmt::ImportFrom(imp) => imp.module.as_ref().map_or("", |m| m.as_str()),
            _ => "",
        };
        let name_b = match b {
            Stmt::Import(imp) => imp.names[0].name.as_str(),
            Stmt::ImportFrom(imp) => imp.module.as_ref().map_or("", |m| m.as_str()),
            _ => "",
        };
        name_a.cmp(name_b)
    });
}
