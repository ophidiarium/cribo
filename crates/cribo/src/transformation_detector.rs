//! Transformation detection during analysis phase
//!
//! This module is responsible for identifying all AST transformations needed
//! during the analysis phase and populating the transformations map in AnalysisResults.

use anyhow::Result;
use log::debug;
use rustc_hash::FxHashMap;

use crate::{
    analysis::TreeShakeResults,
    cribo_graph::{CriboGraph, ItemId, ItemType, ModuleId},
    module_registry::ModuleRegistry,
    semantic_model_provider::SemanticModelProvider,
    stdlib_detection::{is_stdlib_module, is_stdlib_without_side_effects},
    transformations::{RemovalReason, TransformationMetadata},
};

/// Detects and collects all transformations needed for the bundle
pub struct TransformationDetector<'a> {
    graph: &'a CriboGraph,
    _registry: &'a ModuleRegistry,
    _semantic_provider: &'a SemanticModelProvider<'a>,
    tree_shake_results: Option<&'a TreeShakeResults>,
    python_version: u8,
}

impl<'a> TransformationDetector<'a> {
    pub fn new(
        graph: &'a CriboGraph,
        registry: &'a ModuleRegistry,
        semantic_provider: &'a SemanticModelProvider<'a>,
        tree_shake_results: Option<&'a TreeShakeResults>,
        python_version: u8,
    ) -> Self {
        Self {
            graph,
            _registry: registry,
            _semantic_provider: semantic_provider,
            tree_shake_results,
            python_version,
        }
    }

    /// Detect all transformations needed for the bundle
    pub fn detect_transformations(
        &self,
    ) -> Result<FxHashMap<(ModuleId, ItemId), Vec<TransformationMetadata>>> {
        let mut transformations = FxHashMap::default();

        // Process each module in the graph
        for (&module_id, module_data) in &self.graph.modules {
            debug!(
                "Detecting transformations for module: {}",
                module_data.module_name
            );

            // Process each item in the module
            for (&item_id, item_data) in &module_data.items {
                let mut item_transformations = Vec::new();

                // Check import-related transformations
                match &item_data.item_type {
                    ItemType::Import { module, alias } => {
                        self.detect_import_transformations(
                            module_id,
                            item_id,
                            module,
                            alias.as_deref(),
                            &mut item_transformations,
                        )?;
                    }
                    ItemType::FromImport {
                        module,
                        names,
                        level,
                        ..
                    } => {
                        debug!(
                            "Checking from-import in module {:?} item {:?}: {} with names {:?} \
                             (stmt_index={:?})",
                            module_id, item_id, module, names, item_data.statement_index
                        );
                        if *level == 0 {
                            // Only process absolute imports for now
                            self.detect_from_import_transformations(
                                module_id,
                                item_id,
                                module,
                                names,
                                &mut item_transformations,
                            )?;
                        }
                    }
                    _ => {
                        // Check for symbol rewrites in other statement types
                        self.detect_symbol_rewrites(module_id, item_id, &mut item_transformations)?;
                    }
                }

                // Store transformations if any were detected
                if !item_transformations.is_empty() {
                    transformations.insert((module_id, item_id), item_transformations);
                }
            }
        }

        // Detect circular dependency import moves
        self.detect_circular_dep_moves(&mut transformations)?;

        // Detect namespace collisions for inlining
        self.detect_inlining_collisions(&mut transformations)?;

        Ok(transformations)
    }

    /// Detect transformations for regular import statements
    fn detect_import_transformations(
        &self,
        module_id: ModuleId,
        item_id: ItemId,
        module_name: &str,
        alias: Option<&str>,
        transformations: &mut Vec<TransformationMetadata>,
    ) -> Result<()> {
        // Determine module classification
        let module_kind = self.get_module_kind(module_name);

        // Check if import is unused (tree-shaking)
        if let Some(tree_shake) = self.tree_shake_results
            && !tree_shake.included_items.contains(&(module_id, item_id))
        {
            // Only remove if it's NOT a first-party import
            // First-party imports are handled by BundleCompiler's ImportClassification
            if module_kind != Some(crate::types::ModuleKind::FirstParty) {
                debug!("Removing unused {module_kind:?} import: {module_name}");
                transformations.push(TransformationMetadata::RemoveImport {
                    reason: RemovalReason::Unused,
                });
                return Ok(());
            }
        }

        // Check if it's a stdlib import that needs normalization
        if module_kind == Some(crate::types::ModuleKind::StandardLibrary) && alias.is_some() {
            // Stdlib imports with aliases should be normalized
            if is_stdlib_without_side_effects(module_name, self.python_version) {
                transformations.push(TransformationMetadata::StdlibImportRewrite {
                    canonical_module: module_name.to_string(),
                    symbols: vec![], // Direct import, no symbols
                });

                // TODO: Also need to add SymbolRewrite transformations for usage sites
                // This requires analyzing where the alias is used in the module
            }
        }

        // Check if it's a first-party import that will be bundled
        if module_kind == Some(crate::types::ModuleKind::FirstParty) {
            transformations.push(TransformationMetadata::RemoveImport {
                reason: RemovalReason::Bundled,
            });
        }

        Ok(())
    }

    /// Detect transformations for from-import statements
    fn detect_from_import_transformations(
        &self,
        module_id: ModuleId,
        item_id: ItemId,
        module_name: &str,
        names: &[(String, Option<String>)],
        transformations: &mut Vec<TransformationMetadata>,
    ) -> Result<()> {
        // Determine module classification
        let module_kind = self.get_module_kind(module_name);

        // Check if entire import is unused
        if let Some(tree_shake) = self.tree_shake_results {
            let is_included = tree_shake.included_items.contains(&(module_id, item_id));

            // Debug: show what's in included_items for this module
            if module_id.as_u32() == 4 {
                let module_items: Vec<_> = tree_shake
                    .included_items
                    .iter()
                    .filter(|(mid, _)| *mid == module_id)
                    .collect();
                debug!("Module {module_id:?} included items: {module_items:?}");
            }

            debug!(
                "From-import {module_name} in module {module_id:?} item {item_id:?}: \
                 included={is_included}, module_kind={module_kind:?}, names={names:?}"
            );

            if !is_included {
                // Only remove if it's NOT a first-party import
                // First-party imports are handled by BundleCompiler's ImportClassification
                if module_kind != Some(crate::types::ModuleKind::FirstParty) {
                    debug!("Removing unused {module_kind:?} from-import: {module_name}");
                    transformations.push(TransformationMetadata::RemoveImport {
                        reason: RemovalReason::Unused,
                    });
                    return Ok(());
                }
            }
        }

        // Check for stdlib normalization
        if module_kind == Some(crate::types::ModuleKind::StandardLibrary) {
            if is_stdlib_without_side_effects(module_name, self.python_version) {
                // Collect symbol mappings
                let symbols: Vec<(String, String)> = names
                    .iter()
                    .map(|(name, _alias)| {
                        let canonical = format!("{module_name}.{name}");
                        (name.clone(), canonical)
                    })
                    .collect();

                transformations.push(TransformationMetadata::StdlibImportRewrite {
                    canonical_module: module_name.to_string(),
                    symbols,
                });

                // TODO: Add SymbolRewrite transformations for usage sites
            }
        } else {
            // Check for partial import removal (some symbols unused)
            // TODO: Implement per-symbol usage tracking
            // For now, we'll skip this optimization
        }

        // Check if it's a first-party import that will be bundled
        if module_kind == Some(crate::types::ModuleKind::FirstParty) {
            transformations.push(TransformationMetadata::RemoveImport {
                reason: RemovalReason::Bundled,
            });
        }

        Ok(())
    }

    /// Detect symbol rewrites needed in non-import statements
    fn detect_symbol_rewrites(
        &self,
        _module_id: ModuleId,
        _item_id: ItemId,
        _transformations: &mut Vec<TransformationMetadata>,
    ) -> Result<()> {
        // TODO: Implement detection of symbol usage that needs rewriting
        // This requires:
        // 1. Tracking which symbols have been normalized (from StdlibImportRewrite)
        // 2. Finding usage sites of those symbols
        // 3. Creating SymbolRewrite transformations for each usage
        Ok(())
    }

    /// Detect import moves needed for circular dependency resolution
    fn detect_circular_dep_moves(
        &self,
        _transformations: &mut FxHashMap<(ModuleId, ItemId), Vec<TransformationMetadata>>,
    ) -> Result<()> {
        // TODO: Implement based on CircularDependencyAnalysis results
        // This requires:
        // 1. Getting circular dependency groups from analysis
        // 2. Determining which imports need to be moved
        // 3. Creating CircularDepImportMove transformations
        Ok(())
    }

    /// Detect namespace collisions that need preemptive renames
    fn detect_inlining_collisions(
        &self,
        _transformations: &mut FxHashMap<(ModuleId, ItemId), Vec<TransformationMetadata>>,
    ) -> Result<()> {
        // TODO: Implement collision detection for module inlining
        // This requires:
        // 1. Determining which modules will be inlined
        // 2. Checking for symbol name collisions
        // 3. Creating SymbolRewrite transformations for renames
        Ok(())
    }

    /// Get the module kind for a given module name
    fn get_module_kind(&self, module_name: &str) -> Option<crate::types::ModuleKind> {
        // First check if the module exists in our graph
        if let Some(module) = self
            .graph
            .modules
            .values()
            .find(|m| m.module_name == module_name)
        {
            return Some(module.kind);
        }

        // If not in graph, check if it's a stdlib module
        if is_stdlib_module(module_name, self.python_version) {
            return Some(crate::types::ModuleKind::StandardLibrary);
        }

        // Otherwise, it's a third-party module
        Some(crate::types::ModuleKind::ThirdParty)
    }
}
