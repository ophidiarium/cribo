//! Semantic model provider that manages pre-built semantic models
//!
//! This module provides read-only access to semantic models that are built
//! upfront in the orchestrator, following the provider pattern.

use std::sync::Arc;

use anyhow::Result;
use ruff_python_ast::ModModule;
use ruff_python_semantic::{BindingId, SemanticModel};
use rustc_hash::FxHashMap;

use crate::{cribo_graph::ModuleId, semantic_bundler::SemanticModelBuilder};

/// Storage for a semantic model with its source
pub struct StoredSemanticModel {
    /// The source code (using Arc for shared ownership)
    pub source: Arc<String>,
    /// The parsed AST
    pub ast: ModModule,
    /// The file path
    pub path: std::path::PathBuf,
}

/// Registry of pre-built semantic models
pub struct SemanticModelRegistry {
    /// Map from ModuleId to stored semantic model data
    models: FxHashMap<ModuleId, StoredSemanticModel>,
}

impl Default for SemanticModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticModelRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            models: FxHashMap::default(),
        }
    }

    /// Add a semantic model to the registry
    pub fn add_model(
        &mut self,
        module_id: ModuleId,
        source: Arc<String>,
        ast: ModModule,
        path: std::path::PathBuf,
    ) {
        self.models
            .insert(module_id, StoredSemanticModel { source, ast, path });
    }

    /// Get a semantic model by module ID
    ///
    /// Note: This returns a new SemanticModel each time because SemanticModel
    /// has lifetime parameters tied to the source string.
    pub fn get_model(&self, module_id: ModuleId) -> Option<Result<SemanticModel<'_>>> {
        self.models.get(&module_id).map(|stored| {
            SemanticModelBuilder::build_semantic_model(&stored.source, &stored.path, &stored.ast)
                .map(|(model, _)| model)
        })
    }

    /// Check if a module exists in the registry
    pub fn contains(&self, module_id: ModuleId) -> bool {
        self.models.contains_key(&module_id)
    }
}

/// Read-only provider for semantic models
pub struct SemanticModelProvider<'a> {
    registry: &'a SemanticModelRegistry,
}

impl<'a> SemanticModelProvider<'a> {
    /// Create a new provider from a registry
    pub fn new(registry: &'a SemanticModelRegistry) -> Self {
        Self { registry }
    }

    /// Get a semantic model for a module
    pub fn get_model(&self, module_id: ModuleId) -> Option<Result<SemanticModel<'_>>> {
        self.registry.get_model(module_id)
    }

    /// Check if a module has a semantic model
    pub fn has_model(&self, module_id: ModuleId) -> bool {
        self.registry.contains(module_id)
    }
}

/// Global identifier for a binding across modules
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalBindingId {
    pub module_id: ModuleId,
    pub binding_id: BindingId,
}
