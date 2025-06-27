//! Symbol conflict detection for the analysis pipeline
//!
//! This module detects symbol conflicts across modules by analyzing
//! the CriboGraph and semantic models.

use anyhow::Result;
use log::{debug, trace};
use ruff_python_semantic::{BindingKind, SemanticModel};
use rustc_hash::FxHashMap;

use crate::{
    analysis::{ConflictInstance, SymbolConflict, SymbolType},
    cribo_graph::{CriboGraph, ModuleId},
    module_registry::ModuleRegistry,
    semantic_model_provider::{GlobalBindingId, SemanticModelProvider},
};

/// Detects symbol conflicts across modules
pub struct SymbolConflictDetector<'a> {
    graph: &'a CriboGraph,
    registry: &'a ModuleRegistry,
    semantic_provider: &'a SemanticModelProvider<'a>,
}

impl<'a> SymbolConflictDetector<'a> {
    /// Create a new conflict detector
    pub fn new(
        graph: &'a CriboGraph,
        registry: &'a ModuleRegistry,
        semantic_provider: &'a SemanticModelProvider<'a>,
    ) -> Self {
        Self {
            graph,
            registry,
            semantic_provider,
        }
    }

    /// Detect all symbol conflicts across modules
    pub fn detect_conflicts(
        &self,
        symbol_origins: &FxHashMap<GlobalBindingId, GlobalBindingId>,
    ) -> Result<Vec<SymbolConflict>> {
        debug!("Starting symbol conflict detection");

        // Map to collect all exported symbols by name
        let mut symbol_map: FxHashMap<String, Vec<ConflictInstance>> = FxHashMap::default();

        // Analyze each module
        for module_id in self.graph.modules.keys() {
            if let Some(Ok(semantic_model)) = self.semantic_provider.get_model(*module_id) {
                self.collect_module_symbols(
                    *module_id,
                    &semantic_model,
                    &mut symbol_map,
                    symbol_origins,
                )?;
            }
        }

        // Build conflict list from symbols that appear in multiple modules
        let mut conflicts = Vec::new();
        for (symbol_name, instances) in symbol_map {
            if instances.len() > 1 {
                // Filter out re-exports of the same symbol
                let unique_instances = self.filter_duplicate_origins(&instances, symbol_origins);

                if unique_instances.len() > 1 {
                    trace!(
                        "Found conflict for symbol '{}' across {} modules",
                        symbol_name,
                        unique_instances.len()
                    );
                    conflicts.push(SymbolConflict {
                        symbol_name,
                        conflicts: unique_instances,
                    });
                }
            }
        }

        debug!("Detected {} symbol conflicts", conflicts.len());
        Ok(conflicts)
    }

    /// Collect exported symbols from a module
    fn collect_module_symbols(
        &self,
        module_id: ModuleId,
        semantic_model: &SemanticModel,
        symbol_map: &mut FxHashMap<String, Vec<ConflictInstance>>,
        _symbol_origins: &FxHashMap<GlobalBindingId, GlobalBindingId>,
    ) -> Result<()> {
        let module_info = self
            .registry
            .get_module_by_id(module_id)
            .ok_or_else(|| anyhow::anyhow!("Module {:?} not found in registry", module_id))?;

        let module_name = &module_info.canonical_name;

        // Get the global scope
        let global_scope = semantic_model.global_scope();

        // Iterate through all bindings in the global scope
        for (name, binding_id) in global_scope.all_bindings() {
            // Skip private symbols (starting with _)
            if name.starts_with('_') && !name.starts_with("__") {
                continue;
            }

            // Get binding information
            let binding = semantic_model.binding(binding_id);

            // Skip built-in symbols
            if matches!(binding.kind, BindingKind::Builtin) {
                continue;
            }

            // Determine symbol type from binding kind
            let symbol_type = match binding.kind {
                BindingKind::ClassDefinition(_) => SymbolType::Class,
                BindingKind::FunctionDefinition(_) => SymbolType::Function,
                BindingKind::Assignment => SymbolType::Variable,
                BindingKind::Import(_) => SymbolType::Import,
                BindingKind::FromImport(_) => SymbolType::Import,
                _ => SymbolType::Other,
            };

            // Create conflict instance
            let instance = ConflictInstance {
                global_id: GlobalBindingId {
                    module_id,
                    binding_id,
                },
                module_name: module_name.clone(),
                symbol_type,
                definition_range: binding.range,
            };

            // Add to symbol map
            symbol_map
                .entry(name.to_string())
                .or_default()
                .push(instance);
        }

        Ok(())
    }

    /// Filter out instances that are re-exports of the same original symbol
    /// but keep them if they would still conflict within their own module
    fn filter_duplicate_origins(
        &self,
        instances: &[ConflictInstance],
        _symbol_origins: &FxHashMap<GlobalBindingId, GlobalBindingId>,
    ) -> Vec<ConflictInstance> {
        // For now, don't filter anything - we need to handle re-exports properly
        // This is a temporary solution until we implement proper re-export handling
        instances.to_vec()
    }
}
