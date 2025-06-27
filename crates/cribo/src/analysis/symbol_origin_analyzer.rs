//! Symbol origin analysis for tracking symbol identity across re-exports
//!
//! This module implements the SymbolOriginAnalysis pass that traces every
//! imported or re-exported symbol back to its original definition, creating
//! a map of equivalences that enables proper symbol conflict detection and
//! renaming across module boundaries.

use anyhow::Result;
use log::{debug, trace};
use ruff_python_semantic::{BindingKind, FromImport, Import, SemanticModel};
use rustc_hash::FxHashMap;

use crate::{
    cribo_graph::{CriboGraph, ModuleId},
    module_registry::ModuleRegistry,
    semantic_model_provider::{GlobalBindingId, SemanticModelProvider},
};

/// Analyzes symbol origins to track re-exports and aliases
pub struct SymbolOriginAnalyzer<'a> {
    graph: &'a CriboGraph,
    registry: &'a ModuleRegistry,
    semantic_provider: &'a SemanticModelProvider<'a>,
}

impl<'a> SymbolOriginAnalyzer<'a> {
    /// Create a new symbol origin analyzer
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

    /// Analyze all symbols and build the origin map
    pub fn analyze_origins(&self) -> Result<FxHashMap<GlobalBindingId, GlobalBindingId>> {
        debug!("Starting symbol origin analysis");

        let mut symbol_origins = FxHashMap::default();

        // Process each module
        for module_id in self.graph.modules.keys() {
            if let Some(Ok(semantic_model)) = self.semantic_provider.get_model(*module_id) {
                self.process_module_imports(*module_id, &semantic_model, &mut symbol_origins)?;
            }
        }

        debug!(
            "Symbol origin analysis complete: {} mappings found",
            symbol_origins.len()
        );

        Ok(symbol_origins)
    }

    /// Process all imports in a module to find their origins
    fn process_module_imports(
        &self,
        module_id: ModuleId,
        semantic_model: &SemanticModel,
        symbol_origins: &mut FxHashMap<GlobalBindingId, GlobalBindingId>,
    ) -> Result<()> {
        let module_info = self
            .registry
            .get_module_by_id(module_id)
            .ok_or_else(|| anyhow::anyhow!("Module {:?} not found", module_id))?;

        debug!(
            "Processing imports in module: {}",
            module_info.canonical_name
        );

        // Get all bindings in the global scope
        let global_scope = semantic_model.global_scope();

        for (name, binding_id) in global_scope.all_bindings() {
            let binding = semantic_model.binding(binding_id);

            // We're only interested in import bindings
            match &binding.kind {
                BindingKind::Import(import) => {
                    self.process_import_binding(
                        module_id,
                        binding_id,
                        name,
                        import,
                        symbol_origins,
                    )?;
                }
                BindingKind::FromImport(from_import) => {
                    self.process_from_import_binding(
                        module_id,
                        binding_id,
                        name,
                        from_import,
                        symbol_origins,
                    )?;
                }
                _ => {
                    // Skip non-import bindings
                }
            }
        }

        Ok(())
    }

    /// Process a regular import binding (import module as alias)
    fn process_import_binding(
        &self,
        module_id: ModuleId,
        binding_id: ruff_python_semantic::BindingId,
        name: &str,
        import: &Import,
        symbol_origins: &mut FxHashMap<GlobalBindingId, GlobalBindingId>,
    ) -> Result<()> {
        // For "import foo.bar as baz", import.qualified_name is a QualifiedName
        let imported_module = import.qualified_name.to_string();

        trace!("Processing import: {imported_module} (imported as {name})");

        // Find the module being imported
        if let Some(source_module_id) = self.registry.get_id_by_name(&imported_module) {
            let current_binding = GlobalBindingId {
                module_id,
                binding_id,
            };

            // For module imports, we need to create a special "module binding" that represents
            // the module itself. This allows us to track attribute access on the module.
            // We use a special binding ID that represents the module as a whole.
            if let Some(_source_module) = self.registry.get_module_by_id(source_module_id) {
                // Find the implicit module binding (usually the first binding in __init__.py)
                // For now, we'll use a placeholder approach
                // In a full implementation, we'd need to handle module-level bindings properly
                let source_binding = GlobalBindingId {
                    module_id: source_module_id,
                    binding_id: ruff_python_semantic::BindingId::from(0u32), // Module itself
                };

                // Track that this binding points to the imported module
                symbol_origins.insert(current_binding, source_binding);

                trace!(
                    "Import {name} in module {module_id:?} imports module {source_module_id:?} \
                     (tracked as {current_binding:?} -> {source_binding:?})"
                );
            }
        }

        Ok(())
    }

    /// Process a from import binding (from module import symbol)
    fn process_from_import_binding(
        &self,
        module_id: ModuleId,
        binding_id: ruff_python_semantic::BindingId,
        name: &str,
        from_import: &FromImport,
        symbol_origins: &mut FxHashMap<GlobalBindingId, GlobalBindingId>,
    ) -> Result<()> {
        // For "from foo.bar import baz", from_import.qualified_name would be "foo.bar.baz"
        // We need to split this into module path and imported name
        let qualified_name = from_import.qualified_name.to_string();
        let parts: Vec<&str> = qualified_name.split('.').collect();

        if parts.is_empty() {
            return Ok(());
        }

        let (imported_name, module_parts) = parts.split_last().unwrap();
        let source_module_name = module_parts.join(".");

        trace!(
            "Processing from import: from {source_module_name} import {imported_name} (as {name})"
        );

        // Find the source module
        if let Some(source_module_id) = self.registry.get_id_by_name(&source_module_name) {
            // Get the semantic model for the source module
            if let Some(Ok(source_semantic)) = self.semantic_provider.get_model(source_module_id) {
                // Find the binding in the source module
                if let Some(source_binding_id) =
                    self.find_binding_in_module(&source_semantic, imported_name)
                {
                    let source_global_id = GlobalBindingId {
                        module_id: source_module_id,
                        binding_id: source_binding_id,
                    };

                    let current_global_id = GlobalBindingId {
                        module_id,
                        binding_id,
                    };

                    // Check if the source binding is itself an import
                    let source_binding = source_semantic.binding(source_binding_id);
                    match &source_binding.kind {
                        BindingKind::Import(_) | BindingKind::FromImport(_) => {
                            // The source is also an import - we need to trace further
                            // Check if we already know the origin of the source
                            if let Some(&origin) = symbol_origins.get(&source_global_id) {
                                // We already traced this symbol - use its origin
                                symbol_origins.insert(current_global_id, origin);
                                trace!(
                                    "Symbol {name} in module {module_id:?} traces to known origin \
                                     {origin:?}"
                                );
                            } else {
                                // We haven't traced the source yet - for now, point to it
                                // A full implementation would need multiple passes or recursion
                                symbol_origins.insert(current_global_id, source_global_id);
                                trace!(
                                    "Symbol {name} in module {module_id:?} traces to import \
                                     {source_global_id:?}"
                                );
                            }
                        }
                        _ => {
                            // The source is a definition - this is the origin
                            symbol_origins.insert(current_global_id, source_global_id);
                            trace!(
                                "Symbol {name} in module {module_id:?} originates from \
                                 {source_global_id:?}"
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Find a binding by name in a module's global scope
    fn find_binding_in_module(
        &self,
        semantic_model: &SemanticModel,
        name: &str,
    ) -> Option<ruff_python_semantic::BindingId> {
        let global_scope = semantic_model.global_scope();

        for (binding_name, binding_id) in global_scope.all_bindings() {
            if binding_name == name {
                return Some(binding_id);
            }
        }

        None
    }
}

/// Result of symbol origin analysis
#[derive(Debug, Clone, Default)]
pub struct SymbolOriginResults {
    /// Maps imported/re-exported symbols to their original definitions
    pub symbol_origins: FxHashMap<GlobalBindingId, GlobalBindingId>,
}
